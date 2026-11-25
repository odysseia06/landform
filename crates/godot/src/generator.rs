use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use godot::classes::notify::NodeNotification;
use godot::classes::{INode, Node, ProjectSettings};
use godot::prelude::*;
use landform_core::{
    generate_cancellable, generate_drawn, Drawing, ElevationSource, GenConfig, PackSource, SyntheticSource, World,
    MAX_SIZE_CELLS, MIN_SIZE_CELLS,
};

use crate::config::LandformConfig;
use crate::jobs::{self, Cancel};
use crate::pool;
use crate::world::{is_layer_name, layer_names_text, LandformWorld};

const PACK_EXTENSION: &str = "lfpk";
const STOPPED_UNEXPECTEDLY: &str = "the generator stopped unexpectedly";
const INTERRUPTED_BY_RELOAD: &str =
    "The Landform extension was reloaded during the generation, which discarded it; generate again.";
const NO_DRAWING: &str = "The config's source is the drawing, but it has no drawing.";

/// Starts `job` on a background thread: where its result arrives, and its cancel flag.
pub(crate) fn start(job: Job) -> (Receiver<Result<World, String>>, Cancel) {
    let (sender, receiver) = mpsc::channel();
    let cancel = job.cancel.clone();
    spawn_worker(job, sender);
    (receiver, cancel)
}

/// Runs `job` on a new thread that sends its result to `sender`.
fn spawn_worker(job: Job, sender: Sender<Result<World, String>>) {
    let on_failure = sender.clone();
    let spawned = jobs::spawn("landform-generate", move || {
        // The receiver is gone when the node was freed; the result is discarded.
        let _ = sender.send(job.run());
    });
    if let Err(e) = spawned {
        let _ = on_failure.send(Err(format!("The generator's thread could not start: {e}.")));
    }
}

/// What a job reads its terrain from, resolved on the main thread.
enum Sources {
    Synthetic,
    Packs(Vec<PathBuf>),
    Drawing(Drawing),
}

/// Everything one generation needs, as plain Rust data so it can move to a worker thread.
pub(crate) struct Job {
    seed: u64,
    sources: Sources,
    config: GenConfig,
    /// The layers the world keeps; the core makes all of them, the water only for a drawn world.
    layers: BTreeSet<String>,
    /// This generation's own cancel flag.
    cancel: Cancel,
}

impl Job {
    /// Reads the config on the main thread; fails before any allocation when it cannot work. A
    /// drawing sets the world's size itself, so `map_size` is checked only for the other sources.
    fn plan(seed: i64, config: &LandformConfig) -> Result<Self, String> {
        let size_cells = match config.source {
            // `generate_drawn` reads the size from the drawing, not from the config.
            LandformConfig::SOURCE_DRAWING => GenConfig::default().size_cells,
            _ => map_size_cells(config.map_size)?,
        };
        // Only a world grown from a drawing has water, so no other source reads its rules.
        let settings = config.settings(size_cells, config.source == LandformConfig::SOURCE_DRAWING)?;
        let layers = exported_layers(&config.export_layers)?;
        let sources = match config.source {
            LandformConfig::SOURCE_SYNTHETIC => Sources::Synthetic,
            LandformConfig::SOURCE_PACKS => Sources::Packs(pack_paths(&config.pack_dir)?),
            LandformConfig::SOURCE_DRAWING => match &config.drawing {
                Some(drawing) => Sources::Drawing(drawing.bind().to_core()?),
                None => return Err(NO_DRAWING.to_string()),
            },
            other => {
                return Err(format!(
                    "Unknown source {other}: use SOURCE_SYNTHETIC, SOURCE_PACKS or SOURCE_DRAWING."
                ))
            }
        };
        Ok(Self {
            seed: seed as u64,
            sources,
            config: settings,
            layers,
            cancel: Cancel::new(),
        })
    }

    /// Generates inside the extension's pool; runs on any thread and waits for the result.
    fn run(self) -> Result<World, String> {
        pool::run(move || self.generate()).and_then(|result| result)
    }

    /// Opens the sources or grows the drawing, generates with its own cancel flag and drops the
    /// layers the config did not ask for, so a world never holds a full-size layer nobody uses.
    fn generate(self) -> Result<World, String> {
        let cancel = self.cancel.flag();
        let generated = match &self.sources {
            Sources::Synthetic => {
                let synthetic = SyntheticSource::standard();
                generate_cancellable(self.seed, &[&synthetic], &self.config, cancel)
            }
            Sources::Packs(paths) => {
                let packs: Vec<PackSource> = paths
                    .iter()
                    .map(|path| PackSource::open(path))
                    .collect::<Result<_, _>>()
                    .map_err(|e| e.to_string())?;
                let sources: Vec<&dyn ElevationSource> = packs.iter().map(|p| p as &dyn ElevationSource).collect();
                generate_cancellable(self.seed, &sources, &self.config, cancel)
            }
            Sources::Drawing(drawing) => generate_drawn(self.seed, drawing, &self.config, cancel),
        };
        let mut world = generated.map_err(|e| e.to_string())?;
        world.layers.retain(|name, _| self.layers.contains(name));
        Ok(world)
    }
}

/// `map_size` as a number of cells the core takes, or why it is not one.
fn map_size_cells(size: i64) -> Result<usize, String> {
    usize::try_from(size).ok().filter(|s| (MIN_SIZE_CELLS..=MAX_SIZE_CELLS).contains(s)).ok_or_else(|| {
        format!("map_size must be between {MIN_SIZE_CELLS} and {MAX_SIZE_CELLS} cells, not {size}.")
    })
}

/// The names in `export_layers`, each one of the core's `LAYER_NAMES` or its `WATER_LAYER`, for
/// every source: a world without water simply lacks that layer.
fn exported_layers(names: &PackedStringArray) -> Result<BTreeSet<String>, String> {
    names
        .as_slice()
        .iter()
        .map(|name| {
            let name = name.to_string();
            if is_layer_name(&name) {
                Ok(name)
            } else {
                Err(format!("Unknown layer \"{name}\": the layers are {}.", layer_names_text()))
            }
        })
        .collect()
}

/// Every `*.lfpk` file in `pack_dir`, sorted by file name. Main thread only.
fn pack_paths(pack_dir: &GString) -> Result<Vec<PathBuf>, String> {
    if pack_dir.is_empty() {
        return Err("No pack folder is set: set pack_dir.".to_string());
    }
    let dir = PathBuf::from(ProjectSettings::singleton().globalize_path(pack_dir).to_string());
    if !dir.is_dir() {
        return Err(format!("The pack folder {} does not exist.", dir.display()));
    }
    let entries = fs::read_dir(&dir)
        .map_err(|e| format!("The pack folder {} cannot be read: {e}.", dir.display()))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == PACK_EXTENSION))
        .collect();
    if paths.is_empty() {
        return Err(format!("The pack folder {} holds no .{PACK_EXTENSION} files.", dir.display()));
    }
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    Ok(paths)
}

/// Turns a seed into a `LandformWorld`, blocking with `generate_now` or on a background thread
/// with `generate`. A background result arrives through `poll`, which `_process` calls; `cancel`
/// stops the background generation this generator started, and no other.
#[derive(GodotClass)]
#[class(tool, init, base = Node)]
pub struct LandformGenerator {
    /// What to generate; without one, `generate` and `generate_now` refuse to run.
    #[export]
    config: Option<Gd<LandformConfig>>,
    /// Whether a background job is running, for Godot to carry across an editor reload. A reload
    /// discards the job, so a generator restored with this set reports the job as failed. Not
    /// shown in the inspector; not part of the API.
    #[var(get = _get_job_running, set = _set_job_running, usage_flags = [STORAGE])]
    _job_running: PhantomVar<bool>,
    /// The running job's result channel; `None` when idle.
    job: Option<Receiver<Result<World, String>>>,
    /// The running job's cancel flag, which `cancel` sets; `None` when idle, and when the job was
    /// refused before it ran. Not carried across an editor reload: the reload sets it and discards
    /// the job.
    job_cancel: Option<Cancel>,
    /// `_job_running` was restored as `true`. `EXTENSION_RELOADED` turns it into a failure; `READY`
    /// clears it, because a scene saved mid-job restores the flag on load without any reload.
    interrupted: bool,
    /// Why the last generation failed or was refused; empty after a success.
    last_error: GString,
    base: Base<Node>,
}

#[godot_api]
impl INode for LandformGenerator {
    fn process(&mut self, _delta: f64) {
        self.poll();
    }

    fn on_notification(&mut self, what: NodeNotification) {
        match what {
            NodeNotification::EXTENSION_RELOADED if self.interrupted => {
                self.interrupted = false;
                let (sender, receiver) = mpsc::channel();
                // The receiver is held below, so this send cannot fail.
                let _ = sender.send(Err(INTERRUPTED_BY_RELOAD.to_string()));
                self.job = Some(receiver);
            }
            NodeNotification::READY => self.interrupted = false,
            _ => {}
        }
    }
}

#[godot_api]
impl LandformGenerator {
    #[signal]
    fn generation_finished(world: Gd<LandformWorld>);

    #[signal]
    fn generation_failed(reason: GString);

    /// Starts a background job. Returns `false`, emitting nothing, when a job is already running
    /// or there is no config; any other failure arrives as `generation_failed`.
    #[func]
    fn generate(&mut self, seed: i64) -> bool {
        if self.job.is_some() {
            self.last_error = GString::from("A generation is already running.");
            return false;
        }
        let Some(config) = self.config.clone() else {
            self.last_error = GString::from("The generator has no config.");
            return false;
        };
        let planned = Job::plan(seed, &config.bind());
        let receiver = match planned {
            Ok(job) => {
                let (receiver, cancel) = start(job);
                self.job_cancel = Some(cancel);
                receiver
            }
            Err(reason) => {
                let (sender, receiver) = mpsc::channel();
                // The receiver is held below, so this send cannot fail.
                let _ = sender.send(Err(reason));
                receiver
            }
        };
        self.job = Some(receiver);
        true
    }

    /// Cancels the background generation this generator started: it stops at its next check of
    /// its own flag, at most one stage of the generation or one round of growing land later, and
    /// arrives through `poll` as `generation_failed` with "The generation was cancelled.". Every
    /// other generation, another generator's too, runs on. Does nothing when this generator has
    /// none running; one that has already ended, its result waiting for `poll`, arrives as it
    /// ended. `is_busy()` stays true until the result arrives. `generate_now` holds the calling
    /// thread and is not cancelled by this.
    #[func]
    fn cancel(&mut self) {
        if let Some(cancel) = &self.job_cancel {
            cancel.set();
        }
    }

    /// Generates on the calling thread. Returns `null` on failure, with the reason in
    /// `get_last_error()`; emits no signal.
    #[func]
    fn generate_now(&mut self, seed: i64) -> Option<Gd<LandformWorld>> {
        let Some(config) = self.config.clone() else {
            self.last_error = GString::from("The generator has no config.");
            return None;
        };
        let result = Job::plan(seed, &config.bind()).and_then(Job::run);
        self.finish_now(result)
    }

    /// Delivers a finished background job as `generation_finished` or `generation_failed`.
    #[func]
    fn poll(&mut self) {
        let Some(receiver) = &self.job else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(STOPPED_UNEXPECTEDLY.to_string()),
        };
        self.job = None;
        self.job_cancel = None;
        match result {
            Ok(world) => {
                self.last_error = GString::new();
                let world = LandformWorld::from_world(world);
                self.signals().generation_finished().emit(&world);
            }
            Err(reason) => {
                self.last_error = GString::from(reason.as_str());
                self.signals().generation_failed().emit(reason.as_str());
            }
        }
    }

    /// Records the outcome of a foreground generation and wraps a finished world.
    fn finish_now(&mut self, result: Result<World, String>) -> Option<Gd<LandformWorld>> {
        match result {
            Ok(world) => {
                self.last_error = GString::new();
                Some(LandformWorld::from_world(world))
            }
            Err(reason) => {
                self.last_error = GString::from(reason.as_str());
                None
            }
        }
    }

    #[func]
    fn _get_job_running(&self) -> bool {
        self.job.is_some()
    }

    #[func]
    fn _set_job_running(&mut self, running: bool) {
        self.interrupted = running && self.job.is_none();
    }

    /// Whether a background job is running or has a result waiting for `poll`.
    #[func]
    fn is_busy(&self) -> bool {
        self.job.is_some()
    }

    /// Why the last generation failed or was refused; empty after a success.
    #[func]
    fn get_last_error(&self) -> GString {
        self.last_error.clone()
    }
}

/// A job here runs its parallel stages through the extension's pool (`Job::run`), or is
/// cancelled before its first one, so no test here starts rayon's global pool, which the test in
/// `pool.rs` checks.
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::jobs::join_workers;
    use crate::pool;
    use landform_core::LandformError;
    use std::sync::atomic::Ordering;
    use std::sync::PoisonError;

    /// A job growing the example valley `size_m` across from `seed`, with a new cancel flag.
    pub(crate) fn drawn_job(size_m: u32, seed: u64) -> Job {
        Job { seed, sources: Sources::Drawing(Drawing::example_valley(size_m)), config: GenConfig::default(), layers: BTreeSet::new(), cancel: Cancel::new() }
    }

    /// The error a cancelled generation ends with.
    pub(crate) fn cancelled() -> Option<String> {
        Some(LandformError::Cancelled.to_string())
    }

    /// Each source's generation stops on its own flag and sets no other. It takes turns with the
    /// test below, whose `join_workers` sets every flag of the process.
    #[test]
    fn every_source_stops_on_its_own_cancel_flag_and_no_other() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let drawing = Drawing::example_valley(256);
        for sources in [Sources::Synthetic, Sources::Packs(Vec::new()), Sources::Drawing(drawing)] {
            let job = Job { seed: 1, sources, config: GenConfig::default(), layers: BTreeSet::new(), cancel: Cancel::new() };
            let other = Cancel::new();
            job.cancel.set();
            assert_eq!(job.generate().err(), cancelled());
            assert!(!other.flag().load(Ordering::Relaxed), "cancelling one generation set another's flag");
        }
    }

    /// Two generations started as two generators start theirs (`start`): the first cancelled ends
    /// with `Cancelled`, the second with its world. Then `join_workers`, called as two more start,
    /// sets both their flags and waits for both, which end cancelled whether or not they had begun
    /// to run, and a generation started after it runs to its end. The 1,024 m valley takes far longer to grow
    /// than the flags take to be set, so each is set while its generation runs, or before its
    /// first check. The pool's tests take turns with this one, since it builds the pool and
    /// `join_workers` reaches every job of the process.
    #[test]
    fn a_cancel_stops_its_own_generation_and_join_workers_stops_every_one() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let (first, first_cancel) = start(drawn_job(1024, 1));
        let (second, _second_cancel) = start(drawn_job(256, 2));
        first_cancel.set();
        assert_eq!(first.recv().expect("the first job sends its result").err(), cancelled());
        let world = second.recv().expect("the second job sends its result").expect("the second generation ends with a world");
        assert_eq!((world.heights.width, world.meta.seed), (256, 2));

        let running = [start(drawn_job(1024, 3)), start(drawn_job(1024, 4))];
        join_workers();
        for (result, cancel) in running {
            assert!(cancel.flag().load(Ordering::Relaxed), "join_workers left a flag unset");
            assert_eq!(result.try_recv().expect("join_workers waited for the job's result").err(), cancelled());
        }
        let (after, after_cancel) = start(drawn_job(256, 5));
        assert!(!after_cancel.flag().load(Ordering::Relaxed));
        assert!(after.recv().expect("the job sends its result").is_ok(), "a generation after join_workers was stopped");
        join_workers();
        pool::shut_down();
    }
}
