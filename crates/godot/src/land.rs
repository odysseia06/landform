//! A land kept as a land file (`.lfland`, the core's `Land::write` and `Land::read`), held in Godot
//! whole: the core's land in an `Arc`, its world as a `LandformWorld`, its drawings as
//! `LandformDrawing`s, its seed, its reach, its size and its marks as pictures. It is read with
//! `load_file`, written with `save_file`, and carried across an editor reload as its file's bytes.
//! It is grown and edited off the main thread, one job at a time, each with its own cancel flag
//! (`jobs.rs`): a job reads the land through its `Arc` and changes nothing, and a finished edit
//! arrives through `poll` as a `LandformLandStep` (`step.rs`), what lies between the land and the
//! edited land (the core's `Change`), worked out on the job's thread. A step is swapped in on the
//! main thread (`swap`), and a hold, an ask and a reach are done there and given as steps already
//! done; each moves the land to another state, and what Godot shows of the land is made again only
//! where the step changed it, its plain-Rust part in the extension's pool. The outline of a
//! regrowth toward a drawing, or of an area regrown, is worked out off the main thread beside the
//! job while a person drags (`outline.rs`) and arrives through `poll` too.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use godot::classes::image::Format;
use godot::classes::{Image, ProjectSettings};
use godot::prelude::*;
use landform_core::{
    Change, Drawing, Edit, GenConfig, Held, Lake, Land, LandFile, LandformError, Marks, River, Touched, BAND_M, REACH_MAX_M, REACH_MIN_M,
};

use crate::config::LandformConfig;
use crate::drawing::LandformDrawing;
use crate::jobs::Running;
use crate::outline::{Asked, LandformOutline, Outlines};
use crate::overlay::{paint, Colours, EDGE_CELLS_MOST};
use crate::pool;
use crate::step::{EditSaid, LandformLandStep, LetGo, Said, StepKind, Steps};
use crate::world::{copy_of, fits, image_from, or_report, read, world_parts, LandformWorld, WorldParts};

/// A marked cell's value in the held and hand pictures; an unmarked cell's is 0.
const MARKED: u8 = 255;

/// The kinds of item `get_grown_partner` pairs: a river or a lake of the asked drawing.
const RIVER: &str = "river";
const LAKE: &str = "lake";

/// Why an empty land cannot give what was asked of it.
const EMPTY: &str = "The land is empty: read one with LandformLand.load_file.";

/// Why `load_file` and `save_file` refuse an empty path, before anything is read or written: Godot
/// turns it into no path at all, which the core would read or write as a file named after nothing
/// but its `.part` in the process's working directory.
const NO_PATH: &str = "the path is empty: give the path of a land file.";

/// The keys of `_state`: the land's file bytes, and whether its state was the one last read or
/// written.
const STATE_LAND: &str = "land";
const STATE_SAVED: &str = "saved";

/// The least value of a pixel of an area (`start_regrow_area`, an `Image.FORMAT_L8`) whose cell is
/// in the area, and of a mark's or an area's picture that the overlay draws its cell by
/// (`LandformOverlay.compose`), one value for both so the overlay draws an area as the regrowth reads
/// it (stage 2's ruling R22, N3); a pixel below it leaves its cell out.
pub(crate) const AREA_LEAST: u8 = 128;

/// The longest `_get_state` waits for a land's running job to end, once it has set the job's own
/// flag, before it takes the land's state beside the job if it still runs: four times the stop's
/// budget of 500 ms (the stage 2 plan's **Global Constraints**). It bounds `_get_state` itself,
/// which matters when a script calls it: at a reload the library's deinit then waits for the job
/// anyway, without a bound, through `jobs::join_workers`, so a job that never checked its flag
/// would hold a reload either way. A job ends at its next check of its flag (an edit at 2,048 m
/// within about 140 ms of it, measured through a whole run), and the land does not change while a
/// job runs, so the state is the same whether or not the job has ended; past the bound its bytes
/// are made beside the job, behind its parallel work in the pool.
const RELOAD_WAIT_MOST: Duration = Duration::from_millis(2_000);

/// What a land job's thread is called, and what a start is refused with when its thread cannot
/// start.
const JOB_THREAD: &str = "landform-land";
const JOB_CANNOT_START: &str = "The land's thread could not start";

/// Why `swap` refuses a step, after the method's name: one that holds nothing, one let go by the
/// budget, one let go by a land read, restored or made anew (stage 2's ruling R10), and one that
/// lies between two states the land is not in. The land is as it was after each.
const HOLDS_NOTHING: &str = "the step holds nothing: it was made by LandformLandStep.new(), or Landform was reloaded since it was made, which lets every step go; the land is as it was.";
const LET_GO: &str = "the step was let go to keep the land's steps within their budget (keep_steps_within), so it changes nothing; the land is as it was.";
const LET_GO_NEW_LAND: &str = "the step was let go when the land was read, restored or made anew, which lets go every step of the land it held, so it changes nothing; the land is as it was.";
const OTHER_STATES: &str = "the step lies between two states the land is not in: it is a step of another land, or of this land before it changed without the step; the land is as it was.";

/// Why a land is not changed here when a job that was let go before it ended still reads it: the
/// land is not copied for it (a land of 2,048 m holds about its file's 115 MB), and the job stops
/// at its next check of its flag, which letting it go set.
const STILL_READ: &str = "the land is still read by a job that was let go and has not yet stopped; try again once it has, at its next check of its flag; the land is as it was.";

/// Every part of a land: what a new state of it shows in Godot is made whole.
const EVERY_PART: Touched = Touched { world: true, held: true, by_hand: true, drawings: true, rest: true };

/// The core's sentence for why the last `LandformLand.load_file` failed; empty after a success.
/// Plain Rust, not a Godot type; Godot sets and reads it on the main thread.
static LAST_ERROR: Mutex<String> = Mutex::new(String::new());

fn set_last_error(sentence: &str) {
    let mut last = LAST_ERROR.lock().unwrap_or_else(PoisonError::into_inner);
    sentence.clone_into(&mut last);
}

/// The id the next state of a land takes, 0 until the first is given. Every id is given once in
/// the process: the first one asked for after the library is loaded is the clock's nanoseconds
/// since 1970 at that call, so the ids given after an editor reload of the extension, which starts
/// this again from 0, lie beyond every id given before it (unless more than one state a nanosecond
/// was made).
static NEXT_STATE: AtomicU64 = AtomicU64::new(0);

/// A new id of a state of a land, never given before in the process.
fn new_state_id() -> u64 {
    if NEXT_STATE.load(Ordering::Relaxed) == 0 {
        let start = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |since| since.as_nanos() as u64).max(1);
        // Another thread may have started it first; either start serves.
        let _ = NEXT_STATE.compare_exchange(0, start, Ordering::Relaxed, Ordering::Relaxed);
    }
    NEXT_STATE.fetch_add(1, Ordering::Relaxed)
}

/// What the tool asks of a land often, worked out once for each state of the land in the
/// extension's pool, where the land is read, so that no call on the main thread builds the
/// drawings' bytes (`Land::waiting`) or compares the drawings again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Derived {
    /// `Land::waiting`: the asked drawing differs from the one the land was grown from.
    waiting: bool,
    /// For each river of the asked drawing, in its order, the index of its partner among the grown
    /// drawing's rivers (`partners`), or -1.
    river_partners: Vec<i64>,
    /// The same for the lakes.
    lake_partners: Vec<i64>,
}

/// What `land` gives the tool often (`Derived`). Plain Rust, no thread pool: called in the pool with
/// the read.
fn derive(land: &Land) -> Derived {
    Derived {
        waiting: land.waiting(),
        river_partners: partners(&land.asked.rivers, &land.drawing.rivers, same_river),
        lake_partners: partners(&land.asked.lakes, &land.drawing.lakes, same_lake),
    }
}

/// For each item of `asked`, in its order, the index of the first item of `grown` that `same` takes
/// as equal to it and no earlier item of `asked` took, or -1 when there is none (the stage 2 plan's
/// decision 14).
fn partners<T>(asked: &[T], grown: &[T], same: fn(&T, &T) -> bool) -> Vec<i64> {
    let mut taken = vec![false; grown.len()];
    (asked.iter())
        .map(|item| match (0..grown.len()).find(|&at| !taken[at] && same(item, &grown[at])) {
            Some(at) => {
                taken[at] = true;
                at as i64
            }
            None => -1,
        })
        .collect()
}

/// Whether two rivers are one, bit for bit: every point's east and south and the width by their
/// bits, so 0 and -0 differ, as the land file's bytes of them do. Taken apart field by field, so a
/// field added to a river fails to compile until it is compared.
fn same_river(a: &River, b: &River) -> bool {
    let (River { points, width_m }, other) = (a, b);
    width_m.to_bits() == other.width_m.to_bits()
        && points.len() == other.points.len()
        && points.iter().zip(&other.points).all(|(&(x, z), &(other_x, other_z))| x.to_bits() == other_x.to_bits() && z.to_bits() == other_z.to_bits())
}

/// Whether two lakes are one, bit for bit, as `same_river` compares rivers.
fn same_lake(a: &Lake, b: &Lake) -> bool {
    let (Lake { x_m, z_m, radius_m }, other) = (a, b);
    x_m.to_bits() == other.x_m.to_bits() && z_m.to_bits() == other.z_m.to_bits() && radius_m.to_bits() == other.radius_m.to_bits()
}

/// What Godot shows of a land's state, made of the land as plain Rust off the main thread, each part
/// `None` where it is kept as it was: its world's parts, its marks' pictures' bytes, and what the
/// tool asks of it often. Godot's images are made of them on the main thread (`Kept::new`,
/// `Kept::refresh`).
struct Shown {
    world: Option<WorldParts>,
    held: Option<Vec<u8>>,
    by_hand: Option<Vec<u8>>,
    derived: Option<Derived>,
}

/// What Godot shows of `land` where `touched` names a part, made side by side on the threads of the
/// pool the call runs in (rayon's current pool: called inside `pool::run`, or on a job's thread
/// there). With `EVERY_PART`, every part of `Shown`.
fn shown(land: &Land, touched: Touched) -> Shown {
    let size = land.drawing.size_m as usize;
    let Marks { held, by_hand } = &land.marks;
    let ((world, derived), (held, by_hand)) = rayon::join(
        || rayon::join(|| touched.world.then(|| world_parts(&land.world)), || touched.drawings.then(|| derive(land))),
        || rayon::join(|| touched.held.then(|| marks_bytes(held, size)), || touched.by_hand.then(|| marks_bytes(by_hand, size))),
    );
    Shown { world, held, by_hand, derived }
}

/// The land `read` gives and what Godot shows of it whole, both worked out in the extension's pool
/// while the calling thread waits, so that what the reader does in parallel (each section's
/// checksum) does not start rayon's global pool. The sentence when the pool cannot start or the
/// land is not read.
fn read_in_pool(read: impl FnOnce() -> Result<Land, String> + Send) -> Result<(Land, Shown), String> {
    read_in_pool_or_not(read).and_then(|read| read)
}

/// `read_in_pool`, the two ways it fails apart: the outer sentence when the pool cannot start, the
/// inner one when the land is not read.
fn read_in_pool_or_not(read: impl FnOnce() -> Result<Land, String> + Send) -> Result<Result<(Land, Shown), String>, String> {
    pool::run(|| read().map(|land| {
        let shown = shown(&land, EVERY_PART);
        (land, shown)
    }))
}

/// `land` written to `file`, its folder made when missing, as the command line writes a land: the
/// core's `Land::write`, through a file of the same name with `.part` added that then takes its
/// name. Plain Rust: called in the pool.
fn write_land(land: &Land, file: &Path) -> Result<(), LandformError> {
    if let Some(folder) = file.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        fs::create_dir_all(folder).map_err(|error| LandformError::Io(format!("{}: {error}", folder.display())))?;
    }
    land.write(file)
}

/// `path` as the path on the disk it stands for: a Godot path (`res://`, `user://`) through
/// `ProjectSettings.globalize_path`, an absolute or relative one as it is. Main thread only.
fn disk_path(path: &GString) -> PathBuf {
    PathBuf::from(ProjectSettings::singleton().globalize_path(path).to_string())
}

/// `marks` (one flag a world cell, in the world's cell order; a cell beyond the list unmarked) as an
/// `Image.FORMAT_L8`'s bytes, `size` x `size`, `MARKED` on a marked cell and 0 elsewhere. Plain
/// Rust.
fn marks_bytes(marks: &[bool], size: usize) -> Vec<u8> {
    (0..size * size).map(|cell| if marks.get(cell) == Some(&true) { MARKED } else { 0 }).collect()
}

/// The marks' picture of a land `size` cells a side from its bytes (`marks_bytes`). Main thread
/// only.
fn marks_image(bytes: Vec<u8>, size: usize) -> Gd<Image> {
    image_from(size, size, Format::L8, bytes)
}

/// `area`'s cells, one flag a world cell of a land `size` cells a side, in the world's cell order:
/// in the area where its pixel is `AREA_LEAST` or more. The area must be an `Image.FORMAT_L8` of
/// the land's size without mipmaps; any other is refused with a sentence naming both sizes, after
/// `method`'s name. Main thread only: it reads the image.
fn area_cells(method: &str, area: Option<Gd<Image>>, size: i64) -> Result<Vec<bool>, String> {
    let area = area.ok_or_else(|| format!("{method}: there is no area."))?;
    if !fits(&area, Format::L8, size) {
        return Err(format!(
            "{method}: the area is {} x {} pixels in FORMAT_{:?}{} and the land {size} x {size} cells: give an Image.FORMAT_L8 of the land's size, without mipmaps.",
            area.get_width(),
            area.get_height(),
            area.get_format(),
            if area.has_mipmaps() { " with mipmaps" } else { "" },
        ));
    }
    Ok(area.get_data().as_slice().iter().map(|&value| value >= AREA_LEAST).collect())
}

/// What a land's job does; one runs at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    /// `start_new`: an empty land grows a new one (`Land::grow`).
    New,
    /// `start_regrow`: the edit toward the drawing asked for (`Land::edit`).
    Regrow,
    /// `start_reseed`: the edit toward the drawing asked for from another seed
    /// (`Land::edit_from_seed`).
    Seed,
    /// `start_regrow_area`: the land's own drawing regrown as drawn on an area
    /// (`Land::regrow_area`).
    Area,
}

impl JobKind {
    /// What the job does, in a sentence's words.
    fn what(self) -> &'static str {
        match self {
            Self::New => "the new land's growing",
            Self::Regrow | Self::Seed | Self::Area => "the land's regrowth",
        }
    }

    /// The sentence a job of this kind ends with when its cancel flag stops it.
    fn stopped(self) -> &'static str {
        match self {
            Self::New => "Growing the new land was stopped.",
            Self::Regrow | Self::Seed | Self::Area => "The regrowth was stopped.",
        }
    }

    /// The sentence the land restored after an editor reload gives for a job of this kind that the
    /// reload stopped.
    fn reloaded(self) -> &'static str {
        match self {
            Self::New => "Landform was reloaded, so growing the new land was stopped.",
            Self::Regrow | Self::Seed | Self::Area => "Landform was reloaded, so the regrowth was stopped.",
        }
    }
}

/// What a land job works out on its thread, as plain Rust data taken from Godot's on the main
/// thread when the job starts.
enum Work {
    /// A new land.
    New(Box<NewLand>),
    /// The land edited toward the drawing it asks for.
    Regrow(Arc<Land>),
    /// The same from the seed.
    Seed(Arc<Land>, u64),
    /// The land's own drawing regrown as drawn on the area, one flag a world cell.
    Area(Arc<Land>, Vec<bool>),
}

/// A new land grown from `drawing` and `seed` with `config`.
struct NewLand {
    drawing: Drawing,
    seed: u64,
    config: GenConfig,
}

/// What a land job gives when it ends well.
enum Done {
    /// A new land, with what Godot shows of it, made on the job's thread.
    Made(Box<(Land, Shown)>),
    /// An edit, as the parts of its step.
    Step(Box<StepParts>),
}

impl Work {
    fn kind(&self) -> JobKind {
        match self {
            Self::New(_) => JobKind::New,
            Self::Regrow(_) => JobKind::Regrow,
            Self::Seed(..) => JobKind::Seed,
            Self::Area(..) => JobKind::Area,
        }
    }

    /// Does the work in the extension's pool, on the job's thread, which waits, reading `cancel`
    /// where the core reads it and once more after the core's work and after `Change::between`: so
    /// a job whose flag was set before its last check ends with the kind's sentence
    /// (`JobKind::stopped`), and what it made is let go; a refusal or failure the core returns,
    /// though the flag was set before it returned, arrives as itself, and a job whose flag was set
    /// after its last check ends as it would have. An edit's step is what lies between the land and
    /// the edited land, which is let go once the step is made; the land itself is only read.
    fn run(self, cancel: &AtomicBool) -> Result<Done, String> {
        let kind = self.kind();
        let said = |error: LandformError| match error {
            LandformError::Cancelled => kind.stopped().to_string(),
            other => other.to_string(),
        };
        let unless_stopped = || if cancel.load(Ordering::Relaxed) { Err(kind.stopped().to_string()) } else { Ok(()) };
        pool::run(|| {
            let (step, land, edit) = match self {
                Self::New(new) => {
                    let NewLand { drawing, seed, config } = *new;
                    let land = Land::grow(&drawing, seed, &config, cancel).map_err(said)?;
                    unless_stopped()?;
                    let shown = shown(&land, EVERY_PART);
                    return Ok(Done::Made(Box::new((land, shown))));
                }
                Self::Regrow(land) => {
                    let edit = land.edit(&land.asked, cancel);
                    (StepKind::Regrow, land, edit)
                }
                Self::Seed(land, seed) => {
                    let edit = land.edit_from_seed(&land.asked, seed, cancel);
                    (StepKind::Seed, land, edit)
                }
                Self::Area(land, area) => {
                    let edit = land.regrow_area(&area, cancel);
                    (StepKind::Area, land, edit)
                }
            };
            // Taken apart, so a part added to an edit fails to compile until its step holds it or
            // it is let go here.
            let Edit { land: edited, outline, troughs: _, pits_left: _, water: _, notes, notes_about, timings } = edit.map_err(said)?;
            unless_stopped()?;
            let clock = Instant::now();
            let change = Change::between(&land, &edited).map_err(|error| error.to_string())?;
            let between_ms = clock.elapsed().as_secs_f32() * 1000.0;
            drop(edited);
            unless_stopped()?;
            let (inside_m2, fixed_inside_m2) = (outline.inside_m2, outline.fixed_inside_m2);
            let said = EditSaid { notes, notes_about, timings, between_ms, inside_m2, fixed_inside_m2 };
            Ok(Done::Step(Box::new(StepParts { kind: step, change, said })))
        })
        .and_then(|done| done)
    }
}

/// What a step of an edit holds, made on the job's thread: the change that makes the land the
/// edited land when swapped in (`Change::between`), and what the edit said and did.
struct StepParts {
    kind: StepKind,
    change: Change,
    said: EditSaid,
}

/// A land's running job: where its result arrives, with its own cancel flag, what it does, when it
/// started and the state of the land it read (the land changes nothing while a job runs, so its
/// step lies between this state and the edited land's).
struct LandJob {
    running: Running<Done>,
    kind: JobKind,
    started: Instant,
    from: u64,
}

/// A land as `LandformLand` holds it: the core's land whole, and what is made of it once for its
/// state.
struct Kept {
    /// The land: what it was grown from, the drawing asked for, its settings, the land model's
    /// ground, the first lakes' levels, its world, its marks and its reach. Shared, so a job can read
    /// it off the main thread while the land changes nothing.
    land: Arc<Land>,
    /// The land's world wrapped, the same object on every `get_world()` while the state lasts.
    world: Gd<LandformWorld>,
    /// `Image.FORMAT_L8`, `size` x `size`, cell `(x, z)` at pixel `(x, z)`: `MARKED` on a held cell,
    /// 0 elsewhere.
    held: Gd<Image>,
    /// The same for the cells shaped by hand.
    by_hand: Gd<Image>,
    derived: Derived,
}

impl Kept {
    /// `land` kept whole, with what Godot shows of it, `shown` of every part (`EVERY_PART`), whose
    /// images are made here. Main thread only.
    fn new(land: Land, shown: Shown) -> Kept {
        const WHOLE: &str = "what a new state of a land shows is made of every part";
        let size = land.drawing.size_m as usize;
        let Shown { world, held, by_hand, derived } = shown;
        Kept {
            world: LandformWorld::of_parts(world.expect(WHOLE)),
            held: marks_image(held.expect(WHOLE), size),
            by_hand: marks_image(by_hand.expect(WHOLE), size),
            derived: derived.expect(WHOLE),
            land: Arc::new(land),
        }
    }

    /// Makes again what Godot shows of the land where `shown` holds a part, `shown` of what a step
    /// changed: a new world object, new pictures of the marks, the values worked out from the
    /// drawings. The rest stays as it was, `get_world()` the same object among it. Main thread
    /// only.
    fn refresh(&mut self, shown: Shown) {
        let size = self.land.drawing.size_m as usize;
        let Shown { world, held, by_hand, derived } = shown;
        if let Some(world) = world {
            self.world = LandformWorld::of_parts(world);
        }
        if let Some(held) = held {
            self.held = marks_image(held, size);
        }
        if let Some(by_hand) = by_hand {
            self.by_hand = marks_image(by_hand, size);
        }
        if let Some(derived) = derived {
            self.derived = derived;
        }
    }
}

/// A land read from a land file, held whole: the land as it is (its world, in the land model's
/// frame), what was drawn, the drawing asked for, the seed it was grown from, how far an edit's
/// outline reaches, which cells are held and which shaped by hand, and the settings, the model's
/// ground and the first lakes' levels an edit reads. `LandformLand.load_file(path)` reads one and
/// `save_file(path)` writes it; `LandformLand.new()` is an empty land of size 0, with no world, no
/// drawing and no marks. Each state of a land has an id, a new one whenever the land is read,
/// restored or changed; `is_saved()` says whether the present state is the one last read or
/// written. A land survives an editor reload of the extension through its hidden `_state`
/// property, which holds its file's bytes. A land is held by scripts and never stored in a saved
/// scene or resource, in a node's metadata, or in a dictionary an editor's state saves: Godot
/// writes a plain object inside a stored value with its `_state`, the land's whole file.
///
/// A land grows and is edited off the main thread, one job at a time: `start_new` grows a new land
/// into an empty one, `start_regrow`, `start_reseed` and `start_regrow_area` edit it, each job on a
/// thread of its own with a cancel flag of its own, which `cancel` sets. `poll` returns at once and,
/// once the job has ended, delivers it: `land_made` (the land is now the new land), `job_finished`
/// with a `LandformLandStep` (the land is unchanged until `swap` swaps the step in), or
/// `job_failed` with a sentence. `hold`, `ask` and `set_reach_m` change the land on the calling
/// thread and give their steps already done. A step lies between two states of the land, and
/// `swap` moves the land from either to the other, so undo and redo are one call; the land reaches
/// its steps in the order they were made without keeping them (`keep_steps_within`).
///
/// While a person drags, `start_outline` and `start_area_outline` ask for the outline of regrowing
/// the land toward a drawing or of regrowing an area as drawn, worked out off the main thread, one
/// at a time, beside the job and apart from it; `poll` starts a request and delivers its outline
/// through `outline_ready`. A request replaces any request waiting, so only the latest is worked
/// out next, and an outline is of the land in the state it was asked in: a change of the land lets
/// every request go.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformLand {
    /// The land as its file's bytes, and whether it was saved, as one dictionary, for Godot to carry
    /// across an editor reload. Not shown in the inspector; not part of the API.
    #[var(get = _get_state, set = _set_state, usage_flags = [STORAGE])]
    _state: PhantomVar<VarDictionary>,
    /// Whether a job runs, or has ended and waits for `poll`, for Godot to carry across an editor
    /// reload. A reload stops the job, so a land restored with this set says so at its next `poll`.
    /// Not shown in the inspector; not part of the API.
    #[var(get = _get_job_running, set = _set_job_running, usage_flags = [STORAGE])]
    _job_running: PhantomVar<bool>,
    /// The land; `None` in an empty land.
    kept: Option<Kept>,
    /// The present state's id (`new_state_id`).
    #[init(val = new_state_id())]
    state_id: u64,
    /// The id of the state last read or written; `None` before any.
    saved_id: Option<u64>,
    /// The running job, or the one that has ended and waits for `poll`; `None` when idle. Not
    /// carried across an editor reload, which stops it.
    job: Option<LandJob>,
    /// `_job_running` was restored as `true` with no job here: a reload stopped the job, which the
    /// next `poll` says.
    interrupted: bool,
    /// Why the last start was refused or the last job failed, or the last hold, ask or reach was
    /// refused; empty after a job started, after one that ended well, and after a step done here.
    job_error: String,
    /// The land's steps, in the order they were made, reached without keeping them alive.
    steps: Steps,
    /// The land's outlines: the request waiting, the one worked out on their own thread, the
    /// newest outline not yet delivered, and the land's own start and lakes' zones kept for them.
    /// Not carried across an editor reload, which lets every request go.
    outlines: Outlines,
    /// Why the last `start_outline` or `start_area_outline` was refused, or the last outline
    /// failed; empty after a request taken and after an outline delivered.
    outline_error: String,
    base: Base<RefCounted>,
}

impl LandformLand {
    /// Takes `land` as the land's new state, with what Godot shows of it made whole (`shown` of
    /// `EVERY_PART`); `saved` when the state is the one last read or written. The steps of the land
    /// it held before lie between states it will never be in again, and are let go, and so are its
    /// outlines' requests, with its start and its lakes' zones. Main thread only.
    fn keep(&mut self, land: Land, shown: Shown, saved: bool) {
        self.kept = Some(Kept::new(land, shown));
        self.state_id = new_state_id();
        self.saved_id = saved.then_some(self.state_id);
        self.steps.let_all_go();
        self.outlines.let_go();
        self.outlines.forget(true, true);
    }

    /// Makes the land empty, in a new state that no read or write gave, its steps and its outlines'
    /// requests let go.
    fn clear(&mut self) {
        self.kept = None;
        self.state_id = new_state_id();
        self.saved_id = None;
        self.steps.let_all_go();
        self.outlines.let_go();
        self.outlines.forget(true, true);
    }

    /// The land, or why there is none, naming `method`.
    fn kept(&self, method: &str) -> Result<&Kept, String> {
        self.kept.as_ref().ok_or_else(|| format!("{method}: {EMPTY}"))
    }

    /// Why no job can start now, naming `method`, or nothing: a job runs, or has ended but has not
    /// been polled (each said in its own words, both asking for `poll`), or a reload stopped one that
    /// `poll` has not yet said.
    fn idle(&self, method: &str) -> Result<(), String> {
        match (&self.job, self.interrupted) {
            (Some(job), _) if job.running.wait(Some(Duration::ZERO)) => {
                Err(format!("{method}: {} has ended but has not been polled; poll, which delivers it, first.", job.kind.what()))
            }
            (Some(job), _) => Err(format!("{method}: {} has not ended; stop it, or poll until it ends, first.", job.kind.what())),
            (None, true) => Err(format!("{method}: Landform was reloaded while a job of the land ran; poll, which says so, first.")),
            (None, false) => Ok(()),
        }
    }

    /// The land read for an edit, shared with the job, when no job runs and the land is not empty.
    fn edited(&self, method: &str) -> Result<Arc<Land>, String> {
        self.idle(method)?;
        self.kept(method).map(|kept| Arc::clone(&kept.land))
    }

    /// Starts `work` as the land's job, or says why not: `false`, with the sentence in
    /// `get_job_error()`, when `work` is that sentence or the job's thread cannot start
    /// (`JOB_CANNOT_START`, with the reason); `true` once the job's thread runs.
    fn start(&mut self, work: Result<Work, String>) -> bool {
        let started = work.and_then(|work| {
            let kind = work.kind();
            Running::start(JOB_THREAD, JOB_CANNOT_START, move |cancel| work.run(cancel)).map(|running| (running, kind))
        });
        match started {
            Ok((running, kind)) => {
                self.job = Some(LandJob { running, kind, started: Instant::now(), from: self.state_id });
                self.job_error.clear();
                true
            }
            Err(sentence) => {
                self.job_error = sentence;
                false
            }
        }
    }

    /// Delivers a job that has ended, which read the land in the state `from`: a new land is kept
    /// as the land's new state, unsaved, and `land_made` emitted; an edit's step, which lies between
    /// `from` and a new state (or `from` twice when its change is empty), is emitted with
    /// `job_finished`, the land unchanged; a failure is emitted with `job_failed`. The signal is
    /// emitted last, so a handler may start the next job.
    fn deliver(&mut self, from: u64, result: Result<Done, String>) {
        match result {
            Ok(Done::Made(made)) => {
                let (land, shown) = *made;
                self.keep(land, shown, false);
                self.job_error.clear();
                self.signals().land_made().emit();
            }
            Ok(Done::Step(parts)) => {
                self.job_error.clear();
                let StepParts { kind, change, said } = *parts;
                let to = if change.is_empty() { from } else { new_state_id() };
                let step = LandformLandStep::made(kind, [from, to], change, Said::Edit(Box::new(said)), &mut self.steps);
                self.signals().job_finished().emit(&step);
            }
            Err(sentence) => {
                self.job_error.clone_from(&sentence);
                self.signals().job_failed().emit(sentence.as_str());
            }
        }
    }

    /// The land to change here, on the calling thread (a hold, an ask, a reach, a swap), when no
    /// job runs and the land is not empty, or why not, naming `method`. A job lets the land's `Arc`
    /// go before its result arrives, so with no job the land is its own (`Arc::get_mut`) and is
    /// changed in place, never copied; were a job let go unended still to read it, the call is
    /// refused (`STILL_READ`) rather than the land copied. The outlines' requests are of the land as
    /// it is, so they are let go first, and the calling thread waits for the one being worked out to
    /// end (at most one step of it), which then no longer reads the land.
    fn land_here(&mut self, method: &str) -> Result<&mut Land, String> {
        self.idle(method)?;
        self.kept(method)?;
        self.outlines.let_go_and_wait();
        let kept = self.kept.as_mut().ok_or_else(|| format!("{method}: {EMPTY}"))?;
        Arc::get_mut(&mut kept.land).ok_or_else(|| format!("{method}: {STILL_READ}"))
    }

    /// Finishes a change of the land done here (a hold, an ask, a reach set), which changed only
    /// the parts `before` held: `step_back` makes, in the extension's pool, the change from the land
    /// as it is to the land as it was (the core's `Change::to_parts`), and what Godot shows of the
    /// parts `touched` names is made again there. The land then moves to a new state, unless the
    /// change is empty, and the step is given, already done, lying between the two. When the pool
    /// cannot start, `undo` puts `before` back and the pool's sentence is given, the land as it
    /// was.
    #[allow(clippy::too_many_arguments)]
    fn finish_here<P: Sync>(
        &mut self,
        method: &str,
        kind: StepKind,
        said: Said,
        touched: Touched,
        before: P,
        step_back: impl FnOnce(&Land, &P) -> Change + Send,
        undo: impl FnOnce(&mut Land, P),
    ) -> Result<Gd<LandformLandStep>, String> {
        let kept = self.kept.as_mut().ok_or_else(|| format!("{method}: {EMPTY}"))?;
        let made = {
            let (land, before): (&Land, &P) = (&kept.land, &before);
            pool::run(|| (step_back(land, before), shown(land, touched)))
        };
        let (change, shown) = match made {
            Ok(made) => made,
            Err(sentence) => {
                // The land was its own when it was changed (`land_here`), and nothing has taken it since.
                if let Some(land) = Arc::get_mut(&mut kept.land) {
                    undo(land, before);
                }
                return Err(format!("{method}: {sentence}"));
            }
        };
        kept.refresh(shown);
        let from = self.state_id;
        if !change.is_empty() {
            self.state_id = new_state_id();
        }
        Ok(LandformLandStep::made(kind, [from, self.state_id], change, said, &mut self.steps))
    }

    /// Asks for an outline (`asked`, of the land shared with the request), or says why not: `true`
    /// once the request waits, `false` with the sentence in `get_outline_error()`.
    fn ask_outline(&mut self, asked: Result<(Arc<Land>, Asked), String>) -> bool {
        match asked {
            Ok((land, asked)) => {
                self.outlines.ask(land, asked, self.state_id);
                self.outline_error.clear();
                true
            }
            Err(sentence) => {
                self.outline_error = sentence;
                false
            }
        }
    }

    /// Delivers the land's job once it has ended, as `poll` does.
    fn poll_job(&mut self) {
        if self.interrupted {
            self.interrupted = false;
            // Only an empty land grows a new one, and only a land is edited.
            let kind = if self.kept.is_some() { JobKind::Regrow } else { JobKind::New };
            self.deliver(self.state_id, Err(kind.reloaded().to_string()));
            return;
        }
        let Some((from, result)) = self.job.as_ref().and_then(|job| Some((job.from, job.running.take()?))) else {
            return;
        };
        self.job = None;
        self.deliver(from, result);
    }

    /// Starts the waiting request for an outline when none is worked out, and delivers the newest
    /// outline worked out since, when it is of the land as it is now, through `outline_ready`, or
    /// reports why it failed, as `poll` does.
    fn poll_outline(&mut self) {
        match self.outlines.poll(self.state_id) {
            Some(Ok(parts)) => {
                self.outline_error.clear();
                let outline = LandformOutline::made(parts);
                self.signals().outline_ready().emit(&outline);
            }
            Some(Err(sentence)) => {
                godot_error!("{sentence}");
                self.outline_error = sentence;
            }
            None => {}
        }
    }

    /// The step, or `None` with the sentence in `get_job_error()`.
    fn answer(&mut self, step: Result<Gd<LandformLandStep>, String>) -> Option<Gd<LandformLandStep>> {
        match step {
            Ok(step) => {
                self.job_error.clear();
                Some(step)
            }
            Err(sentence) => {
                self.job_error = sentence;
                None
            }
        }
    }

    /// `hold`'s work: the area held or lifted in place, then its step.
    fn hold_area(&mut self, area: Option<Gd<Image>>, on: bool) -> Result<Gd<LandformLandStep>, String> {
        const METHOD: &str = "hold";
        self.idle(METHOD)?;
        let size = i64::from(self.kept(METHOD)?.land.drawing.size_m);
        let cells = area_cells(METHOD, area, size)?;
        let kind = if on { StepKind::Hold } else { StepKind::Lift };
        let touched = Touched { held: true, ..Touched::default() };
        let step_back = |land: &Land, before: &Vec<bool>| Change::to_parts(land, Some(before), None, None);
        let undo = |land: &mut Land, before: Vec<bool>| land.marks.held = before;
        // A hold that changes no mark changes no part of the land: its step is empty, and the
        // outlines asked for are of the land as it stays (stage 2's ruling R12, N4). The cells the
        // hold marks are worked out once, for both (stage 2's ruling R14, N4): no change of the land
        // comes between, since `land_here` only waits for an outline, which changes nothing.
        let unchanged = self.kept(METHOD)?;
        let reach = unchanged.land.hold_reach(&cells);
        if !unchanged.land.holds_anything_in(&reach, on) {
            let before = unchanged.land.marks.held.clone();
            return self.finish_here(METHOD, kind, Said::Hold(Held { cells: 0, lakes: Vec::new() }), touched, before, step_back, undo);
        }
        let land = self.land_here(METHOD)?;
        let before = land.marks.held.clone();
        let held = land.hold_reached(reach, on);
        self.finish_here(METHOD, kind, Said::Hold(held), touched, before, step_back, undo)
    }

    /// `ask`'s work: the drawing asked for set in place, then its step.
    fn ask_for(&mut self, drawing: Option<Gd<LandformDrawing>>) -> Result<Gd<LandformLandStep>, String> {
        const METHOD: &str = "ask";
        self.idle(METHOD)?;
        self.kept(METHOD)?;
        let drawing = drawing.ok_or_else(|| format!("{METHOD}: there is no drawing."))?;
        let drawing = drawing.bind().to_core().map_err(|reason| format!("{METHOD}: {reason}"))?;
        // Refused before anything is let go, in the words `Land::ask` refuses it with (stage 2's
        // ruling R12, N4).
        self.kept(METHOD)?.land.check_ask(&drawing).map_err(|error| format!("{METHOD}: {error}"))?;
        let land = self.land_here(METHOD)?;
        let before = land.asked.clone();
        land.ask(drawing).map_err(|error| format!("{METHOD}: {error}"))?;
        let touched = Touched { drawings: true, ..Touched::default() };
        let step_back = |land: &Land, before: &Drawing| Change::to_parts(land, None, Some(before), None);
        self.finish_here(METHOD, StepKind::Ask, Said::Nothing, touched, before, step_back, |land, before| land.asked = before)
    }

    /// `set_reach_m`'s work: the reach set in place, held to its range, then its step.
    fn reach_to(&mut self, reach_m: f64) -> Result<Gd<LandformLandStep>, String> {
        const METHOD: &str = "set_reach_m";
        if !reach_m.is_finite() {
            return Err(format!("{METHOD}: the reach is {reach_m}: give it in metres, from {REACH_MIN_M} to {REACH_MAX_M}."));
        }
        let held = reach_m.clamp(f64::from(REACH_MIN_M), f64::from(REACH_MAX_M)) as f32;
        // Nothing Godot shows of a land is made from its reach.
        let step_back = |land: &Land, &before: &f32| Change::to_parts(land, None, None, Some(before));
        let undo = |land: &mut Land, before: f32| land.reach_m = before;
        // The reach the land has already changes nothing: its step is empty, and the outlines asked
        // for are of the land as it stays (stage 2's ruling R12, N4).
        self.idle(METHOD)?;
        let before = self.kept(METHOD)?.land.reach_m;
        if before.to_bits() == held.to_bits() {
            return self.finish_here(METHOD, StepKind::Reach, Said::Nothing, Touched::default(), before, step_back, undo);
        }
        let land = self.land_here(METHOD)?;
        land.reach_m = held;
        self.finish_here(METHOD, StepKind::Reach, Said::Nothing, Touched::default(), before, step_back, undo)
    }

    /// `swap`'s work: the step's change exchanged with the land, the land moved to the step's other
    /// state, and what Godot shows of the parts it changed made again.
    fn swap_in(&mut self, step: Option<Gd<LandformLandStep>>) -> Result<(), String> {
        const METHOD: &str = "swap";
        let step = step.ok_or_else(|| format!("{METHOD}: there is no step."))?;
        self.idle(METHOD)?;
        self.kept(METHOD)?;
        let step = step.bind();
        let contents = step.contents().ok_or_else(|| format!("{METHOD}: {HOLDS_NOTHING}"))?;
        let mut slot = contents.slot();
        let why = match slot.let_go {
            LetGo::Budget => LET_GO,
            LetGo::NewLand => LET_GO_NEW_LAND,
        };
        let change = slot.change.as_mut().ok_or_else(|| format!("{METHOD}: {why}"))?;
        let other = contents.other_state(self.state_id).ok_or_else(|| format!("{METHOD}: {OTHER_STATES}"))?;
        if other == self.state_id {
            // A step whose change is empty lies at one state, and moves the land nowhere.
            return Ok(());
        }
        let touched = change.touched();
        let land = self.land_here(METHOD)?;
        change.swap(land);
        let made = {
            let land: &Land = land;
            pool::run(|| shown(land, touched))
        };
        match made {
            Ok(shown) => {
                self.kept.as_mut().expect("the land was swapped above").refresh(shown);
                self.state_id = other;
                // The land's own start is made of its drawing and seed, and its lakes' zones of its
                // world and land model too; a step of an ask or a reach, which `drawings` and `rest`
                // also name, lets them go as well, and they are made again.
                let (start, zones) = (touched.drawings || touched.rest, touched.world || touched.drawings || touched.rest);
                self.outlines.forget(start, zones);
                Ok(())
            }
            Err(sentence) => {
                change.swap(land);
                Err(format!("{METHOD}: {sentence}"))
            }
        }
    }
}

/// The land's file bytes and whether it was saved, from `_get_state`'s dictionary; `None` when
/// either is missing or of the wrong type.
fn state_parts(state: &VarDictionary) -> Option<(PackedByteArray, bool)> {
    Some((read::<PackedByteArray>(state.get(STATE_LAND))?, read::<bool>(state.get(STATE_SAVED))?))
}

#[godot_api]
impl LandformLand {
    /// The band of an edit's outline in metres, the core's `BAND_M`: the ground within it of the
    /// outline's rim is only partly regrown, and an area regrown as drawn is fully regrown only
    /// farther than it inside its edge. The editor tool shows it; a whole number, as the core's is.
    #[constant]
    pub const BAND_M: i64 = BAND_M as i64;
    /// The least reach `set_reach_m` keeps, in metres, the core's `REACH_MIN_M`; a whole number.
    #[constant]
    pub const REACH_MIN_M: i64 = REACH_MIN_M as i64;
    /// The greatest reach `set_reach_m` keeps, in metres, the core's `REACH_MAX_M`; a whole number.
    #[constant]
    pub const REACH_MAX_M: i64 = REACH_MAX_M as i64;

    /// Reads the land file at `path`: a Godot path (`res://`, `user://`) or an absolute one. A Godot
    /// path is turned into the absolute path on the disk it stands for (`ProjectSettings.
    /// globalize_path`), so a `res://` file is read from the project's folder, as when the editor or
    /// a game runs from it. In an exported game `res://` stands for no folder on the disk:
    /// `globalize_path` turns `res://x.lfland` into the relative path `x.lfland`, which is read from
    /// the process's working directory, as any relative path given here is; an exported game reads
    /// a land from `user://` or an absolute path. The whole file is read and checked in the
    /// extension's pool while the calling thread waits (a land of 2,048 m is a file of 115 MB), with
    /// what the land gives often (`is_waiting()`, `get_grown_partner()`), and the world's and the
    /// marks' images are then made on the calling thread, so this is called from the main thread
    /// only, as every Godot type is touched. The land read is saved (`is_saved()`). Returns `null`
    /// when the file cannot be read or is not a land this Landform reads, after reporting the core's
    /// sentence, which names the file and the fault, as an error; `get_last_error()` holds it until
    /// the next call. An empty path is refused so, with "load_file: the path is empty: give the path
    /// of a land file.", before anything is read.
    #[func]
    fn load_file(path: GString) -> Option<Gd<LandformLand>> {
        if path.is_empty() {
            let sentence = format!("load_file: {NO_PATH}");
            godot_error!("{sentence}");
            set_last_error(&sentence);
            return None;
        }
        let file = disk_path(&path);
        match read_in_pool(|| Land::read(&file).map_err(|error| error.to_string())) {
            Ok((land, shown)) => {
                set_last_error("");
                let mut read = Self::new_gd();
                read.bind_mut().keep(land, shown, true);
                Some(read)
            }
            Err(sentence) => {
                godot_error!("{sentence}");
                set_last_error(&sentence);
                None
            }
        }
    }

    /// Why the last `load_file` failed, the core's sentence naming the file and the fault; empty
    /// after a success and before any call.
    #[func]
    fn get_last_error() -> GString {
        GString::from(LAST_ERROR.lock().unwrap_or_else(PoisonError::into_inner).as_str())
    }

    /// Writes the land to the land file at `path`, a path as `load_file` takes one, every part of it
    /// as it is (the core's `Land::write`), so a land read and saved is its file byte for byte: the
    /// bytes go to a file of the same name with `.part` added, which then takes the file's name, so
    /// a write that fails leaves a file already there as it was and no `.part`. The folder is made
    /// when it is missing, as the command line makes it. The land is written in the extension's pool
    /// while the calling thread waits; main thread only. Returns an empty string on success, after
    /// which the land is saved (`is_saved()`); otherwise the sentence, the core's for a file or a
    /// folder that cannot be written or made, naming it, also reported as an error. An empty path is
    /// refused so, with "save_file: the path is empty: give the path of a land file.", before
    /// anything is written.
    #[func]
    fn save_file(&mut self, path: GString) -> GString {
        let land = match self.kept("save_file") {
            Ok(_) if path.is_empty() => Err(format!("save_file: {NO_PATH}")),
            Ok(kept) => Ok(Arc::clone(&kept.land)),
            Err(sentence) => Err(sentence),
        };
        let land = match land {
            Ok(land) => land,
            Err(sentence) => {
                godot_error!("{sentence}");
                return GString::from(sentence.as_str());
            }
        };
        let file = disk_path(&path);
        match pool::run(|| write_land(&land, &file)).and_then(|written| written.map_err(|error| error.to_string())) {
            Ok(()) => {
                self.saved_id = Some(self.state_id);
                GString::new()
            }
            Err(sentence) => {
                godot_error!("{sentence}");
                GString::from(sentence.as_str())
            }
        }
    }

    /// Whether the present state of the land is the one last read (`load_file`) or written
    /// (`save_file`); a land restored after an editor reload is saved when it was saved before it.
    /// An empty land holds nothing to save, and is saved.
    #[func]
    fn is_saved(&self) -> bool {
        self.kept.is_none() || self.saved_id == Some(self.state_id)
    }

    /// Whether an edit waits to regrow: the drawing asked for (`get_asked_drawing()`) is not the one
    /// the land was grown from (`get_drawing()`), bit for bit (the core's `Land::waiting`). Worked
    /// out once for each state of the land, where it was read, so this costs nothing to call every
    /// frame. False in an empty land.
    #[func]
    fn is_waiting(&self) -> bool {
        self.kept.as_ref().is_some_and(|kept| kept.derived.waiting)
    }

    /// The land as it is: its heights, ground map, layers, rivers, brooks, lakes and notes, in the
    /// land model's frame, as `LandformWorld` says of a land's world (`get_base_elevation()` 0, the
    /// heights as the land keeps them). Its `get_notes()` and `get_note_subjects()` are the land's.
    /// The same object on every call while the land's state lasts: a land read again, or restored
    /// after an editor reload, has a new one. `null` in an empty land.
    #[func]
    fn get_world(&self) -> Option<Gd<LandformWorld>> {
        self.kept.as_ref().map(|kept| kept.world.clone())
    }

    /// What the land was grown from: its size, its picture of high ground (as a `LandformPicture`, held
    /// on the CPU, over an `Image.FORMAT_RF` of the core's values), its rivers, its lakes and its kind
    /// of country, which the world's rivers and lakes are counted by. A new resource on every call, so
    /// changing it leaves the land as it was. Each point and lake centre is the land's own 64-bit float.
    /// `null` in an empty land.
    #[func]
    fn get_drawing(&self) -> Option<Gd<LandformDrawing>> {
        self.kept.as_ref().map(|kept| LandformDrawing::from_core(&kept.land.drawing))
    }

    /// The drawing the person asked for, toward which the next regrowth grows (the core's
    /// `Land::asked`): `get_drawing()`'s equal while nothing waits (`is_waiting()`). A new resource
    /// on every call, as `get_drawing()` gives one. `null` in an empty land.
    #[func]
    fn get_asked_drawing(&self) -> Option<Gd<LandformDrawing>> {
        self.kept.as_ref().map(|kept| LandformDrawing::from_core(&kept.land.asked))
    }

    /// For `kind` `"river"` or `"lake"` and `index` an item of the asked drawing's rivers or lakes
    /// (from 0), the index of the grown drawing's item (`get_drawing()`) equal to it bit for bit, the
    /// first that no earlier item of the asked drawing took, or -1 when none is: an asked item
    /// without a partner waits to regrow, and one with a partner has that item's grown water. Worked
    /// out once for each state of the land, as `is_waiting()` is. Reports an error and returns -1
    /// for another kind, an index out of range or an empty land.
    #[func]
    fn get_grown_partner(&self, kind: GString, index: i64) -> i64 {
        let found = self.kept("get_grown_partner").and_then(|kept| {
            let (partners, item) = match kind.to_string().as_str() {
                RIVER => (&kept.derived.river_partners, RIVER),
                LAKE => (&kept.derived.lake_partners, LAKE),
                other => return Err(format!("get_grown_partner: the kind is \"{RIVER}\" or \"{LAKE}\", not \"{other}\".")),
            };
            let at = usize::try_from(index).ok().and_then(|at| partners.get(at));
            at.copied().ok_or_else(|| format!("get_grown_partner: the asked drawing has no {item} at index {index}; it has {}.", partners.len()))
        });
        or_report(found).unwrap_or(-1)
    }

    /// The seed the land was grown from, as the signed integer GDScript passed in; 0 in an empty
    /// land.
    #[func]
    fn get_seed(&self) -> i64 {
        self.kept.as_ref().map_or(0, |kept| kept.land.seed as i64)
    }

    /// How far an edit's outline reaches beyond the ground it changes, in metres; 0 in an empty land.
    #[func]
    fn get_reach_m(&self) -> f64 {
        self.kept.as_ref().map_or(0.0, |kept| f64::from(kept.land.reach_m))
    }

    /// The land's side in metres, which is its world's side in cells of 1 m; 0 in an empty land.
    #[func]
    fn get_size(&self) -> i64 {
        self.kept.as_ref().map_or(0, |kept| i64::from(kept.land.drawing.size_m))
    }

    /// The held cells, which no edit changes, as an `Image.FORMAT_L8`, `size` x `size`, cell
    /// `(x, z)` at pixel `(x, z)`: 255 on a held cell, 0 elsewhere. A lake is held whole. A new image
    /// on every call; `null` in an empty land.
    #[func]
    fn get_held_image(&self) -> Option<Gd<Image>> {
        self.kept.as_ref().and_then(|kept| copy_of(&kept.held))
    }

    /// The cells shaped by hand, as `get_held_image()` gives the held ones.
    #[func]
    fn get_hand_image(&self) -> Option<Gd<Image>> {
        self.kept.as_ref().and_then(|kept| copy_of(&kept.by_hand))
    }

    /// A job of `start_new` has ended well: the land is the new land, in a new state that is not
    /// saved. Emitted by `poll` (or `stop_and_wait`).
    #[signal]
    fn land_made();

    /// An edit of `start_regrow`, `start_reseed` or `start_regrow_area` has ended well: `step` holds
    /// what lies between the land and the edited land, and the land is unchanged until it is
    /// swapped in (`swap`). Emitted by `poll` (or `stop_and_wait`).
    #[signal]
    fn job_finished(step: Gd<LandformLandStep>);

    /// Exchanges `step` with the land (the core's `Change::swap`), moving the land from one of the two
    /// states the step lies between to the other: a step a job gave is swapped in, and a step swapped
    /// again is undone, so undo and redo are this one call, bit for bit (the land's file byte for
    /// byte). Afterwards `get_world()`, the pictures of the marks and what is worked out from the
    /// drawings are the land's as it now is, each made again only where the step changed it (a hold,
    /// a lift, an ask or a reach keeps `get_world()` the same object), their plain-Rust part in the
    /// extension's pool and Godot's images on the calling thread, which is the main thread. The land
    /// whose state the step moves it back to is saved again when that state was saved
    /// (`is_saved()`). Returns an empty string, or, the land as it was, the sentence why not: there
    /// is no step, a job runs or waits for `poll`, the land is empty, the step holds nothing (made by
    /// `LandformLandStep.new()`, or rebuilt by a reload), it was let go (by `keep_steps_within`, or
    /// by a land read, restored or made anew since, each said in its own words), or it lies between
    /// two states the land is not in (a step of another land, or of this one before it changed
    /// without the step). A step whose change is empty lies at one state, and its swap changes
    /// nothing and returns an empty string.
    #[func]
    fn swap(&mut self, step: Option<Gd<LandformLandStep>>) -> GString {
        match self.swap_in(step) {
            Ok(()) => GString::new(),
            Err(sentence) => GString::from(sentence.as_str()),
        }
    }

    /// Holds (`on`) or lifts the hold on the cells of `area` (the core's `Land::hold`), on the
    /// calling thread, and gives the step, already done: of kind `"hold"` or `"lift"`, saying how
    /// many cells' mark it changed (`get_cells_changed()`) and which lakes it held or lifted whole
    /// (`get_lakes_whole()`): a lake is held whole and lifted whole. `area` is an `Image.FORMAT_L8` of
    /// the land's size without mipmaps, as `start_regrow_area` takes one, a cell in it where its
    /// pixel is 128 or more. Only the marks change: `get_world()` stays the same object, and the held
    /// picture is made again. Returns `null`, with the sentence in `get_job_error()` and the land as
    /// it was, when a job runs, the land is empty or the area is refused.
    #[func]
    fn hold(&mut self, area: Option<Gd<Image>>, on: bool) -> Option<Gd<LandformLandStep>> {
        let step = self.hold_area(area, on);
        self.answer(step)
    }

    /// Sets the drawing the land asks for (the core's `Land::ask`), growing nothing, on the calling
    /// thread, and gives the step, already done, of kind `"ask"`: the next regrowth grows toward it,
    /// and `is_waiting()` and `get_grown_partner()` are worked out again for it, in the extension's
    /// pool. `get_world()` stays the same object. Returns `null`, with the sentence in
    /// `get_job_error()` (the core's for a drawing it refuses, or of another size) and the land as
    /// it was, when a job runs, the land is empty, or the drawing is missing or refused.
    #[func]
    fn ask(&mut self, drawing: Option<Gd<LandformDrawing>>) -> Option<Gd<LandformLandStep>> {
        let step = self.ask_for(drawing);
        self.answer(step)
    }

    /// Sets how far an edit's outline reaches beyond the ground it changes, `reach_m` held to 300 to
    /// 600 m (the core's `REACH_MIN_M` and `REACH_MAX_M`), on the calling thread, and gives the step,
    /// already done, of kind `"reach"`. Returns `null`, with the sentence in `get_job_error()` and the
    /// land as it was, when a job runs, the land is empty, or `reach_m` is not a number.
    #[func]
    fn set_reach_m(&mut self, reach_m: f64) -> Option<Gd<LandformLandStep>> {
        let step = self.reach_to(reach_m);
        self.answer(step)
    }

    /// Lets go the oldest steps of this land kept beyond `bytes` (the stage 2 plan's decision 4):
    /// from the newest step back, the first whose bytes (`LandformLandStep.get_bytes()`), with those
    /// of every step newer than it, come to more than `bytes`, and every step older than it, so the
    /// land is never put together from steps of different ages. A step let go frees what it held,
    /// stays where Godot's history holds it, and changes nothing when swapped (`swap` says so).
    /// Returns how many steps it let go; a `bytes` below 0 is 0. The land reaches its steps without
    /// keeping them: a step Godot lets go is freed then, and is not counted here. An empty step
    /// (`LandformLandStep.is_empty()`, which changes nothing) is neither counted nor let go, so one
    /// held outside every undo action never makes older steps go (stage 2's ruling R22, M1).
    #[func]
    fn keep_steps_within(&mut self, bytes: i64) -> i64 {
        let let_go = self.steps.keep_within(usize::try_from(bytes).unwrap_or(0));
        i64::try_from(let_go).unwrap_or(i64::MAX)
    }

    /// What this land's steps still kept hold in memory, in bytes: the sum of their `get_bytes()`, an
    /// empty step's left out.
    #[func]
    fn get_steps_bytes(&self) -> i64 {
        i64::try_from(self.steps.bytes()).unwrap_or(i64::MAX)
    }

    /// A job has ended without a land or a step: "The regrowth was stopped." (or "Growing the new
    /// land was stopped.") when its cancel flag stopped it, "Landform was reloaded, so the regrowth
    /// was stopped." after an editor reload stopped it, and otherwise the core's sentence (a
    /// refusal as the core words it). The land is as it was. Emitted by `poll` (or
    /// `stop_and_wait`).
    #[signal]
    fn job_failed(sentence: GString);

    /// Grows a new land into this empty one, off the main thread: `Land::grow` of `drawing` from
    /// `seed`, with `config`'s detail, ground rules and water rules (each `null` the core's
    /// default; its source, drawing, size and layers are not read: a land keeps every layer).
    /// Returns `false`, with the sentence in `get_job_error()`, when a job runs, the land is not
    /// empty, the drawing or the config is missing or not valid, or the job's thread cannot start;
    /// `true` once the job started. It ends through `poll` with `land_made`, or `job_failed`.
    /// Making a land is not a step.
    #[func]
    fn start_new(&mut self, drawing: Option<Gd<LandformDrawing>>, seed: i64, config: Option<Gd<LandformConfig>>) -> bool {
        const METHOD: &str = "start_new";
        let work = self.idle(METHOD).and_then(|()| {
            if self.kept.is_some() {
                return Err(format!("{METHOD}: the land is not empty: a new land grows into an empty one, LandformLand.new()."));
            }
            let drawing = drawing.ok_or_else(|| format!("{METHOD}: there is no drawing."))?;
            let config = config.ok_or_else(|| format!("{METHOD}: there is no config."))?;
            let drawing = drawing.bind().to_core().map_err(|reason| format!("{METHOD}: {reason}"))?;
            let config = config.bind().settings(drawing.size_m as usize, true).map_err(|reason| format!("{METHOD}: {reason}"))?;
            // Shown as the signed integer GDScript passed in, as `get_seed()` gives it back.
            Ok(Work::New(Box::new(NewLand { drawing, seed: seed as u64, config })))
        });
        self.start(work)
    }

    /// Regrows the land toward the drawing it asks for (`get_asked_drawing()`), off the main thread:
    /// `Land::edit` of it. Returns `false`, with the sentence in `get_job_error()`, when a job runs
    /// or has ended but has not been polled, the land is empty, or the job's thread cannot start;
    /// `true` once the job started. It ends through `poll` with `job_finished` and a step of kind
    /// `"regrow"`, or `job_failed`.
    #[func]
    fn start_regrow(&mut self) -> bool {
        let work = self.edited("start_regrow").map(Work::Regrow);
        self.start(work)
    }

    /// Regrows the land toward the drawing it asks for from `seed`, off the main thread:
    /// `Land::edit_from_seed`, whose footprint is the whole land when `seed` is not the land's.
    /// Refused and ended as `start_regrow` is; its step's kind is `"seed"`.
    #[func]
    fn start_reseed(&mut self, seed: i64) -> bool {
        // Taken as the signed integer GDScript passed in, as `get_seed()` gives it back.
        let work = self.edited("start_reseed").map(|land| Work::Seed(land, seed as u64));
        self.start(work)
    }

    /// Regrows the land's own drawing as drawn on `area`, off the main thread: `Land::regrow_area`.
    /// `area` is an `Image.FORMAT_L8` of the land's size without mipmaps, cell `(x, z)` at pixel
    /// `(x, z)`, a cell in the area where its pixel is 128 or more. Refused as `start_regrow` is, and
    /// for an area of another size or format, naming both sizes; its step's kind is `"area"`.
    #[func]
    fn start_regrow_area(&mut self, area: Option<Gd<Image>>) -> bool {
        const METHOD: &str = "start_regrow_area";
        let work = self.edited(METHOD).and_then(|land| {
            let cells = area_cells(METHOD, area, i64::from(land.drawing.size_m))?;
            Ok(Work::Area(land, cells))
        });
        self.start(work)
    }

    /// Asks the running job to stop: sets its own cancel flag, and no other job's. It ends at its
    /// next check of the flag and arrives through `poll` as `job_failed` ("The regrowth was
    /// stopped."), the land as it was; a job that had already ended arrives as it ended. Does
    /// nothing when no job runs. `is_busy()` stays true until `poll` delivers it.
    #[func]
    fn cancel(&self) {
        if let Some(job) = &self.job {
            job.running.cancel();
        }
    }

    /// Stops the running job and waits for its end, then delivers it as `poll` does (it is `poll`
    /// once the job has ended, an outline among what it may deliver), so the land is idle
    /// afterwards: the calling thread waits at most one step of the job, the time from one check of
    /// its cancel flag to the next. With no job running it is `poll`. Outlines are not stopped.
    #[func]
    fn stop_and_wait(&mut self) {
        if let Some(job) = &self.job {
            job.running.cancel();
            job.running.wait(None);
        }
        self.poll();
    }

    /// Whether a job runs, or has ended and waits for `poll` to deliver it.
    #[func]
    fn is_busy(&self) -> bool {
        self.job.is_some() || self.interrupted
    }

    /// How long the job has run, in milliseconds, until `poll` delivers it; 0 with no job.
    #[func]
    fn get_busy_ms(&self) -> f64 {
        self.job.as_ref().map_or(0.0, |job| job.started.elapsed().as_secs_f64() * 1000.0)
    }

    /// Delivers the land's job once it has ended (`land_made`, `job_finished` or `job_failed`), then,
    /// with no job running and no outline asked, has the outlines' thread make the land's own start
    /// and its lakes' zones ahead for the outlines to come, starts the waiting request for an
    /// outline when none is worked out and delivers the newest
    /// outline worked out since the last call (`outline_ready`), when it is of the land as it is
    /// after the job's signal; otherwise it returns at once: it never waits for the job or an
    /// outline. A land restored after an editor reload that stopped its job gives `job_failed` with
    /// "Landform was reloaded, so the regrowth was stopped." (or "..., so growing the new land was
    /// stopped." for an empty land) on its first call. Main thread only, as every Godot type is
    /// touched.
    #[func]
    fn poll(&mut self) {
        self.poll_job();
        self.ready_outlines();
        self.poll_outline();
    }

    /// Has the outlines' thread make the land's own start and its lakes' zones ahead while no job
    /// runs and no outline is asked (`Outlines::ready`), so a drag's first outline after the land
    /// was read, made or swapped does not wait for them.
    fn ready_outlines(&mut self) {
        if self.is_busy() {
            return;
        }
        if let Some(kept) = &self.kept {
            self.outlines.ready(Arc::clone(&kept.land), self.state_id);
        }
    }

    /// Asks for the outline of regrowing the land toward `drawing` (a drawing of the land's size, the
    /// asked drawing as a person drags it): the land's own start (`start_of` of its drawing and seed,
    /// kept between requests while its drawing and seed stay as they are) against `drawing`'s, their
    /// footprint and its outline with the land's marks and reach and the zones of the land's own
    /// lakes as an area's request has them (kept likewise), so a lake of the land the outline reaches
    /// is taken whole with its band, as the regrowth takes it (the core's `start_of`, `footprint`,
    /// `lake_zones` and `outline`; stage 2's ruling R21, S3), worked out off the main thread. A lake
    /// the drawing adds or moves has no zone until the regrowth grows the drawing's model, so there
    /// the outline drawn may be smaller than the regrowth's. With the overlay's colours set
    /// (`set_outline_overlay`), the outline also carries the overlay's picture of it over the land's
    /// marks (`LandformOutline.get_overlay_image`, stage 2's ruling R21, N3). While no job runs and
    /// no outline is asked, `poll` has the outlines' thread make the land's start and zones ahead,
    /// so the first outline after the land was read, made or swapped does not wait for them. The
    /// request waits until the outlines' thread takes it, and a request made while another waits
    /// replaces it: `poll` starts the thread when a request waits and the thread does
    /// not run, and the thread takes the latest request waiting each time it is free, ending once
    /// none waits. So of requests made while none is worked out, before the next `poll`, only the
    /// last is worked out, and while a person drags the thread goes from one outline to the latest
    /// asked for. Its outline arrives through `poll` as `outline_ready`, while the land is in the
    /// state it was asked in: a change of the land (a hold, an ask, a reach, a swap, a land read,
    /// restored or made) lets every request go, and none of them arrives, and so does an ask the
    /// land takes even when it changes nothing (an ask of the drawing it asks for already, whose step
    /// is empty: stage 2's ruling R22, N1), while an ask refused, a hold of held ground, a lift of
    /// ground not held and the reach the land has let none go. A job running on the land
    /// neither stops nor is stopped by it. Returns `true` once the request waits; `false`, asking
    /// for nothing, with the sentence in `get_outline_error()`, for an empty land, no drawing, a
    /// drawing that is not valid, or one of another size.
    #[func]
    fn start_outline(&mut self, drawing: Option<Gd<LandformDrawing>>) -> bool {
        const METHOD: &str = "start_outline";
        let asked = self.kept(METHOD).and_then(|kept| {
            let drawing = drawing.ok_or_else(|| format!("{METHOD}: there is no drawing."))?;
            let drawing = drawing.bind().to_core().map_err(|reason| format!("{METHOD}: {reason}"))?;
            let size_m = kept.land.drawing.size_m;
            if drawing.size_m != size_m {
                return Err(format!(
                    "{METHOD}: the drawing is {} m across and the land {size_m} m: an outline is of a drawing of the land's size.",
                    drawing.size_m
                ));
            }
            Ok((Arc::clone(&kept.land), Asked::Drawing(drawing)))
        });
        self.ask_outline(asked)
    }

    /// Asks for the outline of regrowing `area` as drawn (`start_regrow_area`'s area: an
    /// `Image.FORMAT_L8` of the land's size without mipmaps, a cell in it where its pixel is 128 or
    /// more), worked out off the main thread as `start_outline`'s is: the core's `area_outline` of
    /// the area, the marks the regrowth weighs it by (the hand marks cleared on its cells that are not
    /// held) and the zones of the land's own lakes as the land holds them (its water and its kept
    /// model's, kept between requests while its ground, drawing and seed stay as they are), so a lake
    /// the area touches is taken whole with its band before the regrowth runs. The regrowth also
    /// takes the cells where the drawing grown whole holds a lake's water, which only growing it
    /// shows. Waits, replaces, arrives and is let go as `start_outline`'s request does; `false`, with
    /// the sentence in `get_outline_error()`, for an empty land or an area of another size or format.
    #[func]
    fn start_area_outline(&mut self, area: Option<Gd<Image>>) -> bool {
        const METHOD: &str = "start_area_outline";
        let asked = self.kept(METHOD).and_then(|kept| {
            let cells = area_cells(METHOD, area, i64::from(kept.land.drawing.size_m))?;
            Ok((Arc::clone(&kept.land), Asked::Area(cells)))
        });
        self.ask_outline(asked)
    }

    /// Makes the outline of each drawing asked for from now on (`start_outline`) carry the overlay's
    /// picture of it over the land's held and hand-shaped cells (`LandformOutline.get_overlay_image`),
    /// made on the outlines' thread with these colours and `edge_cells` as `LandformOverlay.compose`
    /// makes it, so a view showing it costs the main thread only the image (stage 2's ruling R21,
    /// N3); `on` false, or an `edge_cells` outside 1 to 64, makes none. An area's outline carries
    /// none: the area painted goes on changing while it is worked out. Not kept across a reload of
    /// the extension: the land's view sets it again at each refresh while it edits.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn set_outline_overlay(&mut self, on: bool, rim_colour: Color, band_edge_colour: Color, held_colour: Color, hand_colour: Color, edge_cells: i64) {
        let colours = Colours { hand: paint(hand_colour), held: paint(held_colour), area: [0.0; 4], band_edge: paint(band_edge_colour), rim: paint(rim_colour) };
        let edge = (on && (1..=EDGE_CELLS_MOST).contains(&edge_cells)).then_some(edge_cells as usize);
        self.outlines.set_overlay(edge.map(|edge| (colours, edge)));
    }

    /// Whether an outline was asked for and has not yet arrived: a request waits, is worked out, or
    /// its outline waits for `poll`. A request let go by a change of the land is not counted. The
    /// land's job is not counted either (`is_busy()`).
    #[func]
    fn is_outlining(&self) -> bool {
        self.outlines.is_working()
    }

    /// Why the last `start_outline` or `start_area_outline` was refused, or the last outline failed
    /// (its thread could not start, or its work stopped unexpectedly, each also reported as an
    /// error), as a sentence; empty after a request taken and after an outline delivered.
    #[func]
    fn get_outline_error(&self) -> GString {
        GString::from(self.outline_error.as_str())
    }

    /// The outline asked for by the latest request that was worked out (`start_outline`,
    /// `start_area_outline`), of the land as it is. Emitted by `poll`, after the job's signal.
    #[signal]
    fn outline_ready(outline: Gd<LandformOutline>);

    /// Why the last start was refused, or the last job failed, or the last `hold`, `ask` or
    /// `set_reach_m` was refused, as a sentence; empty after a job started, after one that ended
    /// well, and after a step done.
    #[func]
    fn get_job_error(&self) -> GString {
        GString::from(self.job_error.as_str())
    }

    #[func]
    fn _get_job_running(&self) -> bool {
        self.is_busy()
    }

    #[func]
    fn _set_job_running(&mut self, running: bool) {
        self.interrupted = running && self.job.is_none();
    }

    /// The land as one dictionary, for an editor reload: `land`, its file's bytes, those `save_file`
    /// writes (the core's `Land::to_bytes`, made in the extension's pool: 115 MB for a land of
    /// 2,048 m, so this is for the reload only), and `saved`, whether its state is the one last read
    /// or written; empty for an empty land, and when the pool cannot start (reported). A reload reads
    /// it before it stops the workers (`join_workers`), so a running job is stopped first, by its own
    /// cancel flag, and waited for, at most `RELOAD_WAIT_MOST`: the bytes are then made with the
    /// pool's threads free. The job holds the land and changes nothing, so the bytes are the same
    /// whether or not it has ended.
    #[func]
    fn _get_state(&self) -> VarDictionary {
        if let Some(job) = &self.job {
            job.running.cancel();
            job.running.wait(Some(RELOAD_WAIT_MOST));
        }
        let Some(kept) = &self.kept else {
            return VarDictionary::new();
        };
        let land = &kept.land;
        match pool::run(|| land.to_bytes()) {
            Ok(bytes) => vdict! { STATE_LAND => &PackedByteArray::from(bytes), STATE_SAVED => self.is_saved() },
            Err(sentence) => {
                godot_error!("The land could not be kept for the reload: {sentence}");
                VarDictionary::new()
            }
        }
    }

    /// Restores the land from `_get_state`'s dictionary after an editor reload: the bytes read as a
    /// land file is read (in the extension's pool, while this thread waits), so the land is the one
    /// kept bit for bit, the drawing asked for and so what waits among it; its world is made again
    /// from it, a new object, and its state has a new id, saved when `saved` is true. It restores
    /// whole or not at all: a state missing a part or of the wrong type, or whose bytes are not a
    /// land the reader takes, restores an empty land. No file is read, so a file changed or removed
    /// since the land was read changes nothing. A job running on the land as it was is stopped and
    /// let go, its result never delivered.
    #[func]
    fn _set_state(&mut self, state: VarDictionary) {
        // Letting the job go sets its cancel flag.
        self.job = None;
        let Some((bytes, saved)) = state_parts(&state) else {
            self.clear();
            return;
        };
        // The bytes' own memory, read on the pool's threads while this thread waits and holds them.
        let bytes = bytes.as_slice();
        match read_in_pool_or_not(|| LandFile::from_bytes(bytes).map(|file| file.land)) {
            Ok(Ok((land, shown))) => self.keep(land, shown, saved),
            // A state the reader refuses restores an empty land without a word, as a state missing a
            // part does.
            Ok(Err(_)) => self.clear(),
            Err(sentence) => {
                godot_error!("The land could not be restored after the reload: {sentence}");
                self.clear();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A regrowth of `land` toward the drawing it asks for, started as `start_regrow` starts one,
    /// what it gives let go: whether it ended well, or its sentence.
    pub(crate) fn start_regrow(land: Arc<Land>) -> Running<()> {
        Running::start(JOB_THREAD, JOB_CANNOT_START, move |cancel| Work::Regrow(land).run(cancel).map(drop)).expect("the job's thread starts")
    }

    fn river(points: &[(f64, f64)], width_m: f32) -> River {
        River { points: points.to_vec(), width_m }
    }

    /// Each asked item takes the first equal grown item no earlier one took; an item equal to none,
    /// or whose equals are all taken, has none; equal means bit for bit, so -0 is not 0.
    #[test]
    fn each_asked_item_takes_the_first_equal_grown_item_left() {
        let (a, b) = (river(&[(0.0, 0.0), (10.5, -3.0)], 4.0), river(&[(1.0, 1.0), (2.0, 2.0)], 2.0));
        let grown = [b.clone(), a.clone(), a.clone()];
        let asked = [a.clone(), a.clone(), a.clone(), b.clone(), river(&[(0.0, 0.0), (10.5, -3.0)], 4.5)];
        assert_eq!(partners(&asked, &grown, same_river), [1, 2, -1, 0, -1]);
        let negative_zero = river(&[(-0.0, 0.0), (10.5, -3.0)], 4.0);
        assert_eq!(partners(&[negative_zero], &grown, same_river), [-1]);
        let lake = Lake { x_m: -153.6, z_m: 40.2, radius_m: 60.0 };
        let moved = Lake { x_m: f64::from(-153.6_f32), ..lake };
        assert_eq!(partners(&[lake, moved], &[moved, lake], same_lake), [1, 0]);
        assert_eq!(partners::<Lake>(&[], &[lake], same_lake), Vec::<i64>::new());
    }

    /// The band and the reach's bounds Godot sees are the core's, which are whole metres.
    #[test]
    fn the_band_and_the_reachs_bounds_are_the_cores() {
        assert_eq!(LandformLand::BAND_M as f32, BAND_M);
        assert_eq!(LandformLand::REACH_MIN_M as f32, REACH_MIN_M);
        assert_eq!(LandformLand::REACH_MAX_M as f32, REACH_MAX_M);
    }

    /// Every id is new, so a state never takes another's.
    #[test]
    fn every_state_id_is_new() {
        let ids: Vec<u64> = (0..1000).map(|_| new_state_id()).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "ids not new in order");
        assert!(ids[0] > 0);
    }
}
