use crate::analysis::{analyse, slope_deg, Analysis};
use crate::cover::{cover, layers, Cover, GroundCell};
use crate::detail::{add_detail_faded, cuts_gullies};
use crate::drawing::Drawing;
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{check_model, grow, split_notes, About, Grown, Note, GROW_MARGIN_M};
use crate::resample::resample;
use crate::rng::mix64;
use crate::rules::GroundRules;
use crate::source::{ElevationSource, HeightBlock};
use crate::water::{add_water, gully_factor, Water};
use crate::water_rules::WaterRules;
use crate::window::{pick_window, PickRules, Window};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// The bytes of "detail" and a version. The detail stage's sub-seed is
/// `mix64(seed ^ DETAIL_STAGE_TAG)`, so it shares no draw with the window picker's stream.
const DETAIL_STAGE_TAG: u64 = 0x6465_7461_696C_0001;
/// The bytes of "cover" and a version: the cover stage's sub-seed is `mix64(seed ^ COVER_STAGE_TAG)`.
const COVER_STAGE_TAG: u64 = 0x636F_7665_7200_0001;

/// Smallest and largest world side, in cells, that `generate` accepts.
pub const MIN_SIZE_CELLS: usize = 64;
pub const MAX_SIZE_CELLS: usize = 8192;

/// The `source_id` of a world grown from a drawing.
pub const DRAWN_SOURCE_ID: &str = "drawn";

/// Where a drawn world's heights stand (`finish_drawn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// Lowered so that the lowest cell is exactly 0, the water with them; `meta.base_elevation_m`
    /// is how far they were lowered. A world cut from a source always stands so.
    Lowest,
    /// Where the land model has them: in its frame, its outlets at 0, the water's surfaces and
    /// levels too; `meta.base_elevation_m` is 0.
    Model,
}

/// Size and tuning of a generated world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GenConfig {
    pub size_cells: usize,
    pub cell_size_m: f32,
    /// The amplitude in metres of the low hummocks; 0 switches off all the ground shapes
    /// (hummocks, roughness, ledges, gullies). It does not scale the other three.
    pub detail_amplitude_m: f32,
    pub pick: PickRules,
    /// What lies where; checked before anything is generated.
    pub rules: GroundRules,
    /// The water of a world grown from a drawing; read and checked only by `generate_drawn`.
    pub water: WaterRules,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            size_cells: 2048,
            cell_size_m: 1.0,
            detail_amplitude_m: 0.25,
            pick: PickRules::default(),
            rules: GroundRules::default(),
            water: WaterRules::default(),
        }
    }
}

/// Wall-clock milliseconds per stage; informational only.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Timings {
    /// Choosing the window, including reading the source; 0 for a drawn world.
    pub pick_ms: f32,
    /// Growing the land from the drawing; 0 for a world cut from a source, and for a drawn world
    /// finished from a model grown before (`finish_drawn`).
    pub grow_ms: f32,
    pub resample_ms: f32,
    /// Analysing the resampled heights.
    pub analyse_ms: f32,
    pub detail_ms: f32,
    /// Putting the water into a drawn world: its lakes, beds and wet banks; 0 for a world cut from
    /// a source.
    pub water_ms: f32,
    /// The final slope, the ground map and the layers.
    pub cover_ms: f32,
    pub total_ms: f32,
}

#[derive(Debug, Clone)]
pub struct WorldMeta {
    pub seed: u64,
    pub source_id: String,
    pub window: Window,
    /// Elevation above the source's datum of the world's lowest cell: how far the heights were
    /// lowered to put it at 0. For a drawn world the datum is the land model's frame, its outlets at
    /// 0, and in `Frame::Model`, where nothing is lowered, this is 0. A height plus this is its
    /// height above the datum in either frame.
    pub base_elevation_m: f32,
    /// Unit vector in world `(x, z)` pointing to real north.
    pub north: (f32, f32),
    pub timings: Timings,
}

/// Equal when everything but `timings` is equal: timings vary from run to run.
impl PartialEq for WorldMeta {
    fn eq(&self, other: &Self) -> bool {
        self.seed == other.seed
            && self.source_id == other.source_id
            && self.window == other.window
            && self.base_elevation_m == other.base_elevation_m
            && self.north == other.north
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct World {
    /// Metres above the lowest cell, which is exactly `0.0`; a drawn world in `Frame::Model` keeps
    /// the land model's frame instead, its outlets at 0, where ground may lie below 0.
    pub heights: Grid<f32>,
    /// The layers named in `LAYER_NAMES`, each the size of `heights`: `slope` in degrees, `flow`
    /// as `log10` of the smoothed drained area in square metres, `wetness` and `vigour` from 0 to
    /// 1. `flow` and `wetness` describe the surface before the detail, `slope` the surface after it.
    ///
    /// A drawn world's `wetness` is no less than how wet its water makes the ground, and it has a
    /// fifth layer, `WATER_LAYER`: the water's depth in metres, 0 on land.
    pub layers: BTreeMap<String, Grid<f32>>,
    /// What lies on each cell, the size of `heights`.
    pub ground: Grid<GroundCell>,
    pub meta: WorldMeta,
    /// What the land engine changed in the drawing or could not honour, one sentence each; empty
    /// for a world cut from a source.
    pub notes: Vec<String>,
    /// What each note is about, one for each of `notes` and in its order: a river or lake of the
    /// drawing by its place in it, or the land (`About`); empty for a world cut from a source.
    pub notes_about: Vec<About>,
    /// The rivers, brooks and lakes of a world grown from a drawing, their surfaces and levels in
    /// the frame of `heights`; empty for a world cut from a source.
    pub water: Water,
}

fn elapsed_ms(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1000.0
}

/// `NotFinite` naming `source_id` when any height is not finite.
fn check_finite(heights: &Grid<f32>, source_id: &str) -> Result<(), LandformError> {
    if heights.data.iter().any(|h| !h.is_finite()) {
        return Err(LandformError::NotFinite { source_id: source_id.to_string() });
    }
    Ok(())
}

/// Builds the world for `seed`: check the rules, pick a window, resample it to the world grid,
/// analyse it, add detail, lay the cover on the final surface, and shift heights so the lowest
/// cell is `0.0`. The same seed and sources give bit-identical heights, ground map and layers on
/// any thread count. Every height of a returned world is finite.
pub fn generate(
    seed: u64,
    sources: &[&dyn ElevationSource],
    config: &GenConfig,
) -> Result<World, LandformError> {
    generate_cancellable(seed, sources, config, &AtomicBool::new(false))
}

/// `Cancelled` when `cancel` has been set.
pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), LandformError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(LandformError::Cancelled);
    }
    Ok(())
}

/// `generate`, which stops with `LandformError::Cancelled` when another thread sets `cancel`. The
/// flag is read before the window is picked and after each of the resampling, the analysis, the
/// detail and the cover, so a stage that has begun runs to its end. A flag that is never set
/// gives exactly the world `generate` gives.
pub fn generate_cancellable(
    seed: u64,
    sources: &[&dyn ElevationSource],
    config: &GenConfig,
    cancel: &AtomicBool,
) -> Result<World, LandformError> {
    let size = config.size_cells;
    if !(MIN_SIZE_CELLS..=MAX_SIZE_CELLS).contains(&size) {
        return Err(LandformError::BadConfig(format!(
            "size_cells must be between {MIN_SIZE_CELLS} and {MAX_SIZE_CELLS}, not {size}"
        )));
    }
    check_detail_amplitude(config.detail_amplitude_m)?;
    config.rules.validate()?;
    check_cancel(cancel)?;
    let start = Instant::now();
    let size_m = config.size_cells as f64 * f64::from(config.cell_size_m);
    let (window, block) = pick_window(seed, sources, size_m, &config.pick)?;
    let pick_ms = elapsed_ms(start);

    let stage = Instant::now();
    let heights = resample(&block, &window, config.size_cells, config.cell_size_m);
    // The source block is as large as the world or larger; free it before the later stages.
    drop(block);
    let resample_ms = elapsed_ms(stage);
    check_cancel(cancel)?;
    let source_id = sources[window.source].info().id.clone();
    let timings = Timings { pick_ms, resample_ms, ..Timings::default() };
    let raw = RawHeights { heights, source_id, window, notes: Vec::new(), timings };
    finish_world(seed, raw, config, None, Frame::Lowest, cancel, start)
}

/// A world grown from `drawing`. Its size is the drawing's; `config.size_cells` and `config.pick` are not read.
///
/// The drawing (`Drawing::validate`), the ground rules, the water rules, `detail_amplitude_m` and
/// `cell_size_m` (exactly 1: a drawing's size is in cells of one metre) are checked, in that order,
/// before anything grows. The land is grown (`grow`, with `config.water`), and the world is made
/// from it by the fine steps, as `finish_drawn` runs them in `Frame::Lowest`: the land's heights,
/// on 8 m cells over the world and its margin, are resampled onto the world's `drawing.size_m` x
/// `drawing.size_m` cells of `config.cell_size_m` through a window centred on the world, neither
/// turned nor mirrored, so north is `-z`. From there the world is made by the same steps as one cut from a
/// source, with the detail and the cover taking the same sub-seeds of `seed`, but for the detail's
/// gullies, faded out near the lakes the land holds (`water::gully_factor`), and one more after the
/// detail: the water (`add_water`, with `config.water`), its lakes, beds and wet banks, which the
/// cover reads. The world carries its rivers and lakes (`World::water`) and the `WATER_LAYER`
/// layer of the water's depth besides the four of `LAYER_NAMES`. The meta names `DRAWN_SOURCE_ID`
/// and that window, and the world carries the land engine's notes, then the water's, each with
/// what it is about (`World::notes_about`). Stops with
/// `LandformError::Cancelled` within one round of growing, or after the resampling, the analysis,
/// the detail, the lakes, the beds, the wet banks or the cover, when `cancel` has been set. The same
/// drawing, seed and config give bit-identical heights, ground map, layers and water on any thread
/// count. `timings.grow_ms` is the growing, and `total_ms` counts from its start.
pub fn generate_drawn(
    seed: u64,
    drawing: &Drawing,
    config: &GenConfig,
    cancel: &AtomicBool,
) -> Result<World, LandformError> {
    check_drawn(drawing, config)?;
    let start = Instant::now();
    let grown = grow(drawing, &config.water, seed, cancel)?;
    let grow_ms = elapsed_ms(start);
    fine_steps(seed, WaterStep { grown: &grown, drawing }, config, Frame::Lowest, cancel, start, grow_ms)
}

/// The fine steps of `generate_drawn` from a land model made before: `grown` is a model of
/// `drawing`, the one `grow` gives it from `seed` or one whose heights were made otherwise and its
/// water worked out from them (`model_water`), and the world is the one `generate_drawn` makes from
/// such a model, in `frame`. With
/// `Frame::Lowest`, of the model `grow` gives, it is `generate_drawn`'s world bit for bit, but for
/// its timings; with `Frame::Model` its heights, the water's surfaces and the lakes' levels stay in
/// the model's frame and `meta.base_elevation_m` is 0, and nothing else differs. Its notes are the
/// model's (`Grown::notes`, with `Grown::notes_about`), then the water's. `timings.grow_ms` is 0,
/// and `total_ms` counts from after the checks and the first reading of the flag below, as the fine
/// steps begin.
///
/// Checked first, in this order and with the same errors as `generate_drawn` checks them: the
/// drawing, the ground rules, the water rules, `detail_amplitude_m` and `cell_size_m`; then that
/// `grown` can be `drawing`'s model (`BadConfig`: its three grids on the drawing's model grid, one
/// receiver per cell and among them, its rivers and lakes among the drawing's, one subject for
/// each of its notes, each the land or a river or lake of the drawing). Then the flag is
/// read, and again after the resampling, the analysis, the detail, the lakes, the beds, the wet
/// banks and the cover, as `generate_drawn` reads it. The same inputs give bit-identical heights,
/// ground map, layers and water on any thread count.
pub fn finish_drawn(
    seed: u64,
    drawing: &Drawing,
    config: &GenConfig,
    grown: &Grown,
    frame: Frame,
    cancel: &AtomicBool,
) -> Result<World, LandformError> {
    check_drawn(drawing, config)?;
    check_model(drawing, grown)?;
    check_cancel(cancel)?;
    fine_steps(seed, WaterStep { grown, drawing }, config, frame, cancel, Instant::now(), 0.0)
}

/// `BadConfig` unless `drawing` and `config` can make a drawn world, checked in this order: the
/// drawing (`Drawing::validate`), the ground rules, the water rules, `detail_amplitude_m`, and
/// `cell_size_m`, which must be exactly 1, a drawing's size being in cells of one metre.
pub(crate) fn check_drawn(drawing: &Drawing, config: &GenConfig) -> Result<(), LandformError> {
    drawing.validate()?;
    config.rules.validate()?;
    config.water.validate()?;
    check_detail_amplitude(config.detail_amplitude_m)?;
    if config.cell_size_m != 1.0 {
        return Err(LandformError::BadConfig(format!(
            "cell_size_m must be exactly 1 for a drawn world, whose size is in cells of one metre, not {}",
            config.cell_size_m
        )));
    }
    Ok(())
}

/// The fine steps of a drawn world from its land model, `step.grown`: its heights, on 8 m cells
/// over the world and its margin, resampled onto the world's cells through a window centred on the
/// world, then `finish_world` with the water, in `frame`, the model's notes first. `start` is when
/// the generation began, for `total_ms`, and `grow_ms` how long growing the model took in it.
fn fine_steps(
    seed: u64,
    step: WaterStep,
    config: &GenConfig,
    frame: Frame,
    cancel: &AtomicBool,
    start: Instant,
    grow_ms: f32,
) -> Result<World, LandformError> {
    let WaterStep { grown, drawing } = step;
    let stage = Instant::now();
    let (size_m, margin_m) = (f64::from(drawing.size_m), f64::from(GROW_MARGIN_M));
    // Source metres run from the world's north-west corner, and the grown land begins a margin
    // beyond it on both axes. The water reads the grown land later, so the block takes a copy.
    let block = HeightBlock { origin_x: -margin_m, origin_y: -margin_m, grid: grown.heights.clone() };
    let window =
        Window { source: 0, center_x: size_m / 2.0, center_y: size_m / 2.0, rotation_rad: 0.0, mirrored: false, size_m };
    let heights = resample(&block, &window, drawing.size_m as usize, config.cell_size_m);
    drop(block);
    let resample_ms = elapsed_ms(stage);
    check_cancel(cancel)?;
    let timings = Timings { grow_ms, resample_ms, ..Timings::default() };
    let notes = grown.notes_about.iter().copied().zip(grown.notes.iter().cloned()).collect();
    let raw = RawHeights { heights, source_id: DRAWN_SOURCE_ID.to_string(), window, notes, timings };
    finish_world(seed, raw, config, Some(WaterStep { grown, drawing }), frame, cancel, start)
}

/// `BadConfig` naming `detail_amplitude_m` unless it is a finite number, zero or more.
fn check_detail_amplitude(amplitude: f32) -> Result<(), LandformError> {
    if !(amplitude.is_finite() && amplitude >= 0.0) {
        return Err(LandformError::BadConfig(format!(
            "detail_amplitude_m must be a finite number of metres, zero or more, not {amplitude}"
        )));
    }
    Ok(())
}

/// A world's heights as they first exist, resampled from a source or from grown land, with what
/// the world records of where they came from.
struct RawHeights {
    heights: Grid<f32>,
    source_id: String,
    window: Window,
    /// The notes so far, each with what it is about.
    notes: Vec<Note>,
    /// The stages that made `heights`; `finish_world` fills in the rest.
    timings: Timings,
}

/// What a drawn world's water is made from: the drawing and the land grown from it.
struct WaterStep<'a> {
    grown: &'a Grown,
    drawing: &'a Drawing,
}

/// The steps every world takes once its heights exist: check that they are finite, analyse them,
/// add detail, check again, lay the cover on the final surface and make the layers, and, in
/// `Frame::Lowest`, shift the heights so the lowest cell is `0.0` (in `Frame::Model` they are left
/// where they are and the base elevation is 0). The flag is read after the analysis, the detail
/// and the cover; `start` is when the generation began, for `total_ms`.
///
/// With `water` (a drawn world), the detail's gullies are faded out near the lakes
/// (`add_world_detail`), and the water is put in after the detail and before the slope
/// (`add_water`, under `config.water` and from `seed`, its notes after the others and the flag read
/// once more after it); the cover and the layers read it, and the world's water is moved by the
/// same shift as its heights. Without, the detail is `add_detail`'s, the world has no water and
/// `water_ms` is 0.
fn finish_world(
    seed: u64,
    raw: RawHeights,
    config: &GenConfig,
    water: Option<WaterStep>,
    frame: Frame,
    cancel: &AtomicBool,
    start: Instant,
) -> Result<World, LandformError> {
    let RawHeights { mut heights, source_id, window, mut notes, mut timings } = raw;
    check_finite(&heights, &source_id)?;

    let stage = Instant::now();
    let analysis = analyse(&heights, &config.rules);
    timings.analyse_ms = elapsed_ms(stage);
    check_cancel(cancel)?;

    let stage = Instant::now();
    add_world_detail(&mut heights, &analysis, seed, config, water.as_ref());
    timings.detail_ms = elapsed_ms(stage);
    check_cancel(cancel)?;
    check_finite(&heights, &source_id)?;

    let (mut water, field) = match water {
        Some(WaterStep { grown, drawing }) => {
            let stage = Instant::now();
            let (water, field) = add_water(&mut heights, grown, drawing, &config.water, seed, &mut notes, cancel)?;
            timings.water_ms = elapsed_ms(stage);
            check_cancel(cancel)?;
            (water, Some(field))
        }
        None => (Water::default(), None),
    };

    let stage = Instant::now();
    let slope = slope_deg(&heights);
    let north = window.north();
    let Cover { ground, vigour } =
        cover(&heights, &slope, &analysis, north, mix64(seed ^ COVER_STAGE_TAG), &config.rules, field.as_ref());
    let layers = layers(slope, &analysis, vigour, field);
    timings.cover_ms = elapsed_ms(stage);
    check_cancel(cancel)?;

    let base_elevation_m = match frame {
        Frame::Lowest => {
            let lowest_m = heights.data.iter().copied().fold(f32::INFINITY, f32::min);
            for h in &mut heights.data {
                *h -= lowest_m;
            }
            water.lower(lowest_m);
            lowest_m
        }
        Frame::Model => 0.0,
    };

    timings.total_ms = elapsed_ms(start);
    let meta = WorldMeta { seed, source_id, window, base_elevation_m, north, timings };
    let (notes, notes_about) = split_notes(notes);
    Ok(World { heights, layers, ground, meta, notes, notes_about, water })
}

/// The detail stage of `finish_world`: `add_detail` from the detail's sub-seed of `seed`, under
/// `config`. In a drawn world (`water`), each cell's gully cut is multiplied by
/// `water::gully_factor`, built from the land and the drawing, so no gully runs through a lake's
/// rim; a drawn world none of whose lakes holds water, and a world cut from a source, take
/// `add_detail` as it is. Where the detail cuts no gully (`detail::cuts_gullies`: no detail, or a
/// `gully_depth_m` of 0) the factor would change nothing, and none is built.
fn add_world_detail(heights: &mut Grid<f32>, analysis: &Analysis, seed: u64, config: &GenConfig, water: Option<&WaterStep>) {
    let water = water.filter(|_| cuts_gullies(config.detail_amplitude_m, &config.rules));
    let factor = water.and_then(|step| gully_factor(step.grown, step.drawing, (heights.width, heights.height)));
    let seed = mix64(seed ^ DETAIL_STAGE_TAG);
    add_detail_faded(heights, analysis, seed, config.detail_amplitude_m, &config.rules, factor.as_deref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{Country, COUNTRY_NAMES};
    use crate::cover::{Material, LAYER_NAMES};
    use crate::detail::add_detail;
    use crate::drawing::{Lake, River};
    use crate::grow::GROW_CELL_M;
    use crate::noise::fbm;
    use crate::pack::{write_pack, PackSource};
    use crate::source::{HeightBlock, RectM, SourceInfo};
    use crate::synthetic::SyntheticSource;
    use crate::trial::trial_drawing;
    use crate::water::tests::{at_one_metre, behind_the_head, beside, bounded_water, inside_the_lines_ended_on, kept_by_lines, model_holds_water, near_lines, reach_of_water, rims, Ends, Held, KeptBy, LinePoint};
    use crate::water::{ARRIVAL_ALLOWANCE_M, BORDER_START_M, GULLY_CLEAR_M, GULLY_FADE_M, PASSES_MOST};
    use crate::water::{floor_m, CourseEnd, CoursePoint, LakeWater, RiverCourse, BANK_FOOT_M, BANK_REACH_M, HALF_WIDTH_MIN_M, WATER_LAYER};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    /// A world small enough to generate in a blink.
    fn small_config() -> GenConfig {
        GenConfig { size_cells: 128, ..Default::default() }
    }

    /// A unique path in the temp directory, removed when dropped.
    struct TempPack(PathBuf);

    impl TempPack {
        fn new(name: &str) -> Self {
            Self(std::env::temp_dir().join(format!("landform_world_{}_{name}.lfpk", std::process::id())))
        }
    }

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn coarse_source_cells_give_finite_heights_up_to_the_edges() {
        let mut g = Grid::new(400, 400, 40.0);
        for j in 0..400 {
            for i in 0..400 {
                let (x, y) = (i as f64 * 40.0, j as f64 * 40.0);
                let variation = 5.0 * f64::from(fbm(3, x, y, 4, 2000.0));
                g.set(i, j, (100.0 + 0.05 * x + 0.03 * y + variation) as f32);
            }
        }
        let path = TempPack::new("coarse");
        write_pack(&path.0, "coarse", "Test data", &g).unwrap();
        let s = PackSource::open(&path.0).unwrap();
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        let w = generate(1, &[&s], &cfg).unwrap();
        let bad = w.heights.data.iter().filter(|h| !h.is_finite()).count();
        assert_eq!(bad, 0, "{bad} heights are not finite");
    }

    #[test]
    fn generate_builds_a_shifted_finite_world() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        let w = generate(1, &[&s], &cfg).unwrap();
        assert_eq!((w.heights.width, w.heights.height), (256, 256));
        assert!(w.heights.data.iter().all(|h| h.is_finite()));
        assert_eq!(w.heights.data.iter().cloned().fold(f32::INFINITY, f32::min), 0.0);
        assert!(w.meta.base_elevation_m > 0.0);
        assert!(w.layers.keys().map(String::as_str).eq(LAYER_NAMES));
        assert_eq!(w.meta.source_id, "synthetic");
        assert!(w.notes.is_empty(), "{:?}", w.notes);
        assert_eq!(w.meta.timings.grow_ms, 0.0);
    }

    #[test]
    fn a_world_carries_its_ground_map_and_layers() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        let w = generate(1, &[&s], &cfg).unwrap();
        assert_eq!((w.ground.width, w.ground.height), (256, 256));
        let names: Vec<&str> = w.layers.keys().map(String::as_str).collect();
        assert_eq!(names, LAYER_NAMES);
        for (name, layer) in &w.layers {
            assert_eq!((layer.width, layer.height), (256, 256), "{name}");
            let bad = layer.data.iter().filter(|v| !v.is_finite()).count();
            assert_eq!(bad, 0, "{bad} {name} values are not finite");
        }
        for (name, low, high) in [("wetness", 0.0, 1.0), ("vigour", 0.0, 1.0), ("slope", 0.0, 90.0)] {
            let outside = w.layers[name].data.iter().filter(|v| !(low..=high).contains(*v)).count();
            assert_eq!(outside, 0, "{outside} {name} values lie outside {low}..{high}");
        }
    }

    #[test]
    fn a_world_of_66_cells_generates() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 66, ..Default::default() };
        let w = generate(1, &[&s], &cfg).unwrap();
        assert_eq!((w.heights.width, w.ground.width, w.ground.height), (66, 66, 66));
        assert_eq!(w.layers.len(), LAYER_NAMES.len());
        for (name, layer) in &w.layers {
            assert_eq!((layer.width, layer.height), (66, 66), "{name}");
            let bad = layer.data.iter().filter(|v| !v.is_finite()).count();
            assert_eq!(bad, 0, "{bad} {name} values are not finite");
        }
        for (index, cell) in w.ground.data.iter().enumerate() {
            let at = (index % 66, index / 66);
            assert!(cell.strongest < 6 && cell.second < 6, "{cell:?} at {at:?} holds an index of no material");
            assert!(cell.blend <= 127, "{cell:?} at {at:?} blends past a half");
            assert!(cell.blend == 0 || cell.second != cell.strongest, "{cell:?} at {at:?} blends a material with itself");
        }
    }

    #[test]
    fn invalid_rules_are_refused_before_generating() {
        let s = SyntheticSource::new(1, 50_000.0);
        let rules = GroundRules { wet_start: 0.9, wet_full: 0.8, ..Default::default() };
        let cfg = GenConfig { size_cells: 256, rules, ..Default::default() };
        // With no sources at all, picking would fail with `NoSources`: the rules are checked first.
        let no_sources: [&dyn ElevationSource; 0] = [];
        for sources in [&[&s as &dyn ElevationSource][..], &no_sources[..]] {
            match generate(1, sources, &cfg) {
                Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains("wet_start"), "{e}"),
                other => panic!("{} sources: expected BadConfig, got {:?}", sources.len(), other.map(|w| w.meta)),
            }
        }
    }

    #[test]
    fn upsampled_ground_has_no_facets() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 256, detail_amplitude_m: 0.0, ..Default::default() };
        let w = generate(1, &[&s], &cfg).unwrap();
        let g = &w.heights;
        for j in 0..g.height {
            for i in 1..g.width - 1 {
                let second = g.get(i + 1, j) - 2.0 * g.get(i, j) + g.get(i - 1, j);
                assert!(second.abs() < 0.01, "second difference {second} at ({i}, {j})");
            }
        }
    }

    #[test]
    fn a_detail_amplitude_that_is_not_finite_or_is_negative_is_refused() {
        let s = SyntheticSource::new(1, 50_000.0);
        for amplitude in [f32::NAN, -1.0] {
            let cfg = GenConfig { size_cells: 256, detail_amplitude_m: amplitude, ..Default::default() };
            match generate(1, &[&s], &cfg) {
                Err(e @ LandformError::BadConfig(_)) => {
                    assert!(e.to_string().contains("detail_amplitude_m"), "{e}")
                }
                other => panic!("amplitude {amplitude}: expected BadConfig, got {:?}", other.map(|w| w.meta)),
            }
        }
    }

    /// The synthetic source with the sample at the middle of every block it reads made infinite.
    struct InfiniteMiddle(SyntheticSource);

    impl ElevationSource for InfiniteMiddle {
        fn info(&self) -> &SourceInfo {
            self.0.info()
        }

        fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError> {
            let mut block = self.0.read(rect)?;
            let (i, j) = (block.grid.width / 2, block.grid.height / 2);
            block.grid.set(i, j, f32::INFINITY);
            Ok(block)
        }
    }

    #[test]
    fn a_world_with_a_height_that_is_not_finite_is_an_error() {
        let s = InfiniteMiddle(SyntheticSource::new(1, 50_000.0));
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        match generate(1, &[&s], &cfg) {
            Err(LandformError::NotFinite { source_id }) => assert_eq!(source_id, "synthetic"),
            other => panic!("expected NotFinite, got {:?}", other.map(|w| w.meta)),
        }
    }

    #[test]
    fn detail_uses_its_own_labelled_sub_seed() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        let size_m = cfg.size_cells as f64 * f64::from(cfg.cell_size_m);
        let (window, block) = pick_window(9, &[&s], size_m, &cfg.pick).unwrap();
        let mut heights = resample(&block, &window, cfg.size_cells, cfg.cell_size_m);
        let analysis = analyse(&heights, &cfg.rules);
        add_detail(&mut heights, &analysis, mix64(9 ^ DETAIL_STAGE_TAG), cfg.detail_amplitude_m, &cfg.rules);
        let lowest = heights.data.iter().copied().fold(f32::INFINITY, f32::min);
        let expected: Vec<u32> = heights.data.iter().map(|h| (h - lowest).to_bits()).collect();
        let actual: Vec<u32> = generate(9, &[&s], &cfg).unwrap().heights.data.iter().map(|h| h.to_bits()).collect();
        assert_eq!(actual.len(), expected.len());
        let differing = actual.iter().zip(&expected).filter(|(a, b)| a != b).count();
        assert_eq!(differing, 0, "{differing} of {} heights differ", expected.len());
    }

    #[test]
    fn detail_carves_along_the_water() {
        let s = SyntheticSource::new(1, 50_000.0);
        let carved = GenConfig { size_cells: 512, ..Default::default() };
        let rules = GroundRules { gully_depth_m: 0.0, ..Default::default() };
        let uncarved = GenConfig { rules, ..carved };
        let (a, b) = (generate(1, &[&s], &carved).unwrap(), generate(1, &[&s], &uncarved).unwrap());
        let water: Vec<usize> =
            a.layers["flow"].data.iter().enumerate().filter(|(_, &flow)| flow > 3.6).map(|(index, _)| index).collect();
        assert!(water.len() >= 100, "only {} cells carry a flow above 3.6", water.len());
        // Each world is shifted so its own lowest cell is 0: add the base back to compare them.
        let mean = |w: &World| {
            let total: f64 = water.iter().map(|&index| f64::from(w.heights.data[index])).sum();
            total / water.len() as f64 + f64::from(w.meta.base_elevation_m)
        };
        let (with, without) = (mean(&a), mean(&b));
        assert!(without - with >= 0.05, "the water lines lie at {with} m with gullies and {without} m without");
    }

    #[test]
    fn worlds_from_the_same_seed_are_equal() {
        let s = SyntheticSource::new(1, 50_000.0);
        let cfg = GenConfig { size_cells: 256, ..Default::default() };
        let a = generate(5, &[&s], &cfg).unwrap();
        let b = generate(5, &[&s], &cfg).unwrap();
        let c = generate(6, &[&s], &cfg).unwrap();
        assert!(a == b);
        assert!(a != c);
    }

    #[test]
    fn sizes_outside_the_limits_are_refused_and_the_limits_are_accepted() {
        let s = SyntheticSource::standard();
        for size in [MIN_SIZE_CELLS - 1, MAX_SIZE_CELLS + 1, 0] {
            let cfg = GenConfig { size_cells: size, ..Default::default() };
            match generate(1, &[&s], &cfg) {
                Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains("size_cells"), "{e}"),
                other => panic!("size {size}: expected BadConfig, got {:?}", other.map(|w| w.meta)),
            }
        }
        let cfg = GenConfig { size_cells: MIN_SIZE_CELLS, ..Default::default() };
        assert_eq!(generate(1, &[&s], &cfg).unwrap().heights.width, MIN_SIZE_CELLS);
    }

    #[test]
    fn a_set_cancel_flag_stops_the_generation() {
        let source = SyntheticSource::standard();
        let cancel = AtomicBool::new(true);
        let result = generate_cancellable(1, &[&source], &small_config(), &cancel);
        assert_eq!(result.err(), Some(LandformError::Cancelled));
    }

    #[test]
    fn an_unset_cancel_flag_changes_nothing() {
        let source = SyntheticSource::standard();
        let cancel = AtomicBool::new(false);
        assert_eq!(
            generate_cancellable(1, &[&source], &small_config(), &cancel).unwrap(),
            generate(1, &[&source], &small_config()).unwrap()
        );
    }

    fn drawn(seed: u64, drawing: &Drawing) -> World {
        generate_drawn(seed, drawing, &GenConfig::default(), &AtomicBool::new(false)).expect("the drawing gives a world")
    }

    #[test]
    fn a_drawn_world_has_the_drawings_size() {
        let w = drawn(1, &Drawing::example_valley(256));
        assert_eq!((w.heights.width, w.heights.height), (256, 256));
        assert_eq!((w.ground.width, w.ground.height), (256, 256));
        // The four layers every world has, and the water's, in name order.
        assert!(w.layers.keys().map(String::as_str).eq(["flow", "slope", "vigour", WATER_LAYER, "wetness"]), "{:?}", w.layers.keys());
        for (name, layer) in &w.layers {
            assert_eq!((layer.width, layer.height), (256, 256), "{name}");
        }
        let bad = w.heights.data.iter().filter(|h| !h.is_finite()).count();
        assert_eq!(bad, 0, "{bad} heights are not finite");
        assert_eq!(w.heights.data.iter().copied().fold(f32::INFINITY, f32::min), 0.0);
        assert_eq!(w.meta.source_id, DRAWN_SOURCE_ID);
        assert_eq!(DRAWN_SOURCE_ID, "drawn");
        assert_eq!(w.meta.north, (0.0, -1.0));
        assert_eq!(w.meta.timings.pick_ms, 0.0);
    }

    #[test]
    fn odd_sizes_give_exact_worlds() {
        for size in [64, 100] {
            let w = drawn(1, &Drawing::example_valley(size));
            let side = size as usize;
            assert_eq!((w.heights.width, w.heights.height), (side, side), "{size} m");
            assert_eq!((w.ground.width, w.ground.height), (side, side), "{size} m");
            for (name, layer) in &w.layers {
                assert_eq!((layer.width, layer.height), (side, side), "{name} at {size} m");
            }
            assert!(w.heights.data.iter().all(|h| h.is_finite()), "{size} m");
        }
    }

    #[test]
    fn a_drawn_world_follows_its_grown_land() {
        let drawing = Drawing::example_valley(512);
        let config = GenConfig::default();
        let world = drawn(1, &drawing);
        let land = grow(&drawing, &config.water, 1, &AtomicBool::new(false)).unwrap();
        let (size, cell_m, margin_m) = (drawing.size_m as usize, GROW_CELL_M as usize, GROW_MARGIN_M as usize);
        // Coarse sample `index` lies at world cell `index * 8 - 256` along its axis, if that is in the world.
        let world_cell = |index: usize| (index * cell_m).checked_sub(margin_m).filter(|&cell| cell < size);
        let limit = config.rules.detail_limit_m + 0.5;
        let mut compared = 0;
        for j in 0..land.heights.height {
            for i in 0..land.heights.width {
                let (Some(x), Some(z)) = (world_cell(i), world_cell(j)) else { continue };
                let (in_world, grown) = (world.heights.get(x, z) + world.meta.base_elevation_m, land.heights.get(i, j));
                assert!(
                    (in_world - grown).abs() < limit,
                    "cell ({x}, {z}) lies at {in_world} m in the world and {grown} m in the grown land"
                );
                compared += 1;
            }
        }
        assert_eq!(compared, (size / cell_m).pow(2));
    }

    #[test]
    fn without_detail_a_drawn_world_lies_on_its_grown_land() {
        let drawing = Drawing::example_valley(512);
        // No detail, so no fine shape at all: away from the water, which cuts its beds and banks
        // and grades its lakes' shores, each within 12 m, the world is the grown land, resampled.
        let config = GenConfig { detail_amplitude_m: 0.0, ..GenConfig::default() };
        let world = generate_drawn(1, &drawing, &config, &AtomicBool::new(false)).expect("the drawing gives a world");
        let land = grow(&drawing, &config.water, 1, &AtomicBool::new(false)).unwrap();
        let (size, cell_m, margin_m) = (drawing.size_m as usize, GROW_CELL_M as usize, GROW_MARGIN_M as usize);
        let cut = near_water(&world, &drawing, 13.0, 13.0);
        // Coarse sample `index` lies at world cell `index * 8 - 256` along its axis, if that is in the world.
        let world_cell = |index: usize| (index * cell_m).checked_sub(margin_m).filter(|&cell| cell < size);
        let mut compared = 0;
        for j in 0..land.heights.height {
            for i in 0..land.heights.width {
                let (Some(x), Some(z)) = (world_cell(i), world_cell(j)) else { continue };
                if cut[z * size + x] {
                    continue;
                }
                let (in_world, grown) = (world.heights.get(x, z) + world.meta.base_elevation_m, land.heights.get(i, j));
                assert!(
                    (in_world - grown).abs() < 1e-3,
                    "cell ({x}, {z}) lies at {in_world} m in the world and {grown} m in the grown land"
                );
                compared += 1;
            }
        }
        let samples = (size / cell_m).pow(2);
        assert!(compared * 2 >= samples, "only {compared} of {samples} samples lie away from the water");
    }

    /// No gully through a lake's rim. In the example in hill country from seed 11, on the ground the
    /// detail leaves, before the water is put in: the cells within `GULLY_CLEAR_M` (8 m) of where
    /// the land model holds the lake's water (within 1.5 of its radii of its centre, at its level:
    /// `model_holds_water`) are, bit for bit, those of the same world made with a `gully_depth_m` of
    /// 0, and the cells more than `GULLY_CLEAR_M` and `GULLY_FADE_M` (24 m) from it those of the
    /// same world made without the gully factor, as a world cut from a source takes the detail;
    /// between, the ground lies between the two. Without the factor the outflow's gully lay 0.8 to
    /// 1.2 m deep across the rim: within 8 m of the lake's water it cut at least 0.8 m (1.20 m
    /// measured), so the comparison is not empty.
    #[test]
    fn the_detail_cuts_no_gully_through_a_lakes_rim() {
        let seed = 11;
        let drawing = Drawing { country: Country::named("hills").expect("a known country"), ..Drawing::example_valley(2048) };
        let config = GenConfig::default();
        let grown = grow(&drawing, &config.water, seed, &AtomicBool::new(false)).expect("the drawing grows");
        let before = at_one_metre(&grown, drawing.size_m);
        let step = WaterStep { grown: &grown, drawing: &drawing };
        let detailed = |config: &GenConfig, water: Option<&WaterStep>| {
            let mut heights = before.clone();
            let analysis = analyse(&heights, &config.rules);
            add_world_detail(&mut heights, &analysis, seed, config, water);
            heights
        };
        let ungullied = GenConfig { rules: GroundRules { gully_depth_m: 0.0, ..config.rules }, ..config };
        let (faded, uncut, whole) = (detailed(&config, Some(&step)), detailed(&ungullied, Some(&step)), detailed(&config, None));

        // Where the model holds the lake's water, and each cell's distance to the nearest such cell
        // out to a cell beyond the fade's end (`GULLY_CLEAR_M` and `GULLY_FADE_M`, and 1 m: 25 m):
        // exact, as every such cell beside one that is not marks the cells within reach of it.
        let size = before.width;
        let lake = drawing.lakes[0];
        let level_m = grown.lakes.iter().find(|listed| listed.drawn == 0).expect("the lake is listed").level_m;
        let mut held = vec![false; size * size];
        for cell in cells_within(size, (lake.x_m, lake.z_m), 1.5 * f64::from(lake.radius_m)) {
            held[cell] = model_holds_water(&grown, size, cell, level_m);
        }
        let reach = (GULLY_CLEAR_M + GULLY_FADE_M).ceil() as isize + 1;
        let mut distance_m = vec![f64::INFINITY; size * size];
        for cell in (0..size * size).filter(|&cell| held[cell]) {
            distance_m[cell] = 0.0;
            if beside(cell, size).all(|next| held[next]) {
                continue;
            }
            let (i, j) = ((cell % size) as isize, (cell / size) as isize);
            for (di, dj) in (-reach..=reach).flat_map(|dj| (-reach..=reach).map(move |di| (di, dj))) {
                let (ni, nj) = (i + di, j + dj);
                if (0..size as isize).contains(&ni) && (0..size as isize).contains(&nj) {
                    let next = nj as usize * size + ni as usize;
                    distance_m[next] = distance_m[next].min(((di * di + dj * dj) as f64).sqrt());
                }
            }
        }

        let (mut counts, mut deepest_m, mut failures) = ([0_usize; 3], 0.0_f32, Vec::new());
        for (cell, &apart_m) in distance_m.iter().enumerate() {
            let at = at_cell(size, cell);
            let (faded_m, uncut_m, whole_m) = (faded.data[cell], uncut.data[cell], whole.data[cell]);
            if apart_m <= GULLY_CLEAR_M {
                counts[0] += 1;
                deepest_m = deepest_m.max(uncut_m - whole_m);
                if faded_m.to_bits() != uncut_m.to_bits() {
                    failures.push(format!("{at:?}, {apart_m:.2} m from the lake's water: {faded_m} m, with no gullies {uncut_m} m"));
                }
            } else if apart_m > GULLY_CLEAR_M + GULLY_FADE_M {
                counts[2] += 1;
                if faded_m.to_bits() != whole_m.to_bits() {
                    failures.push(format!("{at:?}, {apart_m:.2} m from the lake's water: {faded_m} m, without the factor {whole_m} m"));
                }
            } else {
                counts[1] += 1;
                if !(whole_m <= faded_m && faded_m <= uncut_m) {
                    failures.push(format!("{at:?}, {apart_m:.2} m from the lake's water: {faded_m} m, not from {whole_m} to {uncut_m} m"));
                }
            }
        }
        assert!(failures.is_empty(), "{} cells: {:#?}", failures.len(), &failures[..failures.len().min(10)]);
        assert!(counts.iter().all(|&count| count > 1000), "cells near, in the fade and beyond it: {counts:?}");
        assert!(deepest_m >= 0.8, "without the factor the gully cut only {deepest_m} m within 8 m of the lake's water");
    }

    #[test]
    fn a_drawn_world_carries_the_engines_notes() {
        let river = River { points: vec![(-500.0, 0.0), (0.0, 0.0)], width_m: 8.0 };
        let drawing =
            Drawing { size_m: 2048, high_ground: None, rivers: vec![river], lakes: Vec::new(), country: Country::hills() };
        let w = drawn(1, &drawing);
        // The land engine's notes, then the water's: the spring's head wanders off its line.
        assert_eq!(w.notes, ["River 1 ended inside the land; it was carried on to the edge.", "River 1 leaves its drawn line by up to 71 m."]);
        assert_eq!(w.notes_about, [About::River(0), About::River(0)]);
        assert!(w.meta.timings.grow_ms > 0.0);
    }

    /// What a note names first, read from its sentence, for the test below: the river or lake of
    /// the first "river N" or "lake N" in it, either case, N counted from 1 as the sentences count;
    /// the land when it names neither.
    fn first_named(note: &str) -> About {
        let lower = note.to_lowercase();
        let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|word| !word.is_empty()).collect();
        for pair in words.windows(2) {
            match (pair[0], pair[1].parse::<usize>()) {
                ("river", Ok(number)) => return About::River(number - 1),
                ("lake", Ok(number)) => return About::Lake(number - 1),
                _ => {}
            }
        }
        About::Land
    }

    /// Every note a drawn world carries says what it is about, one subject for each note and in
    /// its order, given where the note is written; this checks what they say against the
    /// sentences, which are not read to find them. The trial drawings that note something, at
    /// 1,024 m in hill country: `overpeak` and `headjoin` from seed 3 (the head-join says "River 1
    /// was cut 2.1 m deeper where river 3 joins it.", about river 1), `lake3` from seed 1 (it notes
    /// nothing from seed 3) and `wide` from seed 1, whose lake is lowered: each note that names a river or a lake is about the
    /// first it names, and one that names neither is about the land. Over the 17 trial drawings
    /// in the three kinds of country from seeds 1, 2 and 3 (153 worlds) this held for all 320
    /// notes when it was written.
    #[test]
    fn each_note_is_about_the_first_river_or_lake_it_names() {
        let worlds = [("overpeak", 3), ("lake3", 1), ("headjoin", 3), ("wide", 1)];
        let found: Vec<(&str, World)> = std::thread::scope(|scope| {
            let running: Vec<_> = (worlds.iter())
                .map(|&(name, seed)| {
                    scope.spawn(move || {
                        let drawing = trial_drawing(name, 1024, Country::hills()).expect("a trial drawing");
                        (name, drawn(seed, &drawing))
                    })
                })
                .collect();
            running.into_iter().map(|world| world.join().expect("the world grows")).collect()
        });
        let mut subjects = Vec::new();
        for (name, world) in &found {
            assert_eq!(world.notes_about.len(), world.notes.len(), "{name}: {:?} about {:?}", world.notes, world.notes_about);
            assert!(!world.notes.is_empty(), "{name} notes nothing, so the test says nothing of it");
            for (note, about) in world.notes.iter().zip(&world.notes_about) {
                assert_eq!(*about, first_named(note), "{name}: \"{note}\"");
                subjects.push(*about);
            }
        }
        let headjoin = &found[2].1;
        let junction = headjoin.notes.iter().position(|note| note.starts_with("River 1 was cut")).expect("the head-join's note");
        assert_eq!(headjoin.notes_about[junction], About::River(0), "a note that names two rivers is about the first");
        assert!(subjects.iter().any(|about| matches!(about, About::River(_))) && subjects.iter().any(|about| matches!(about, About::Lake(_))), "{subjects:?}");
    }

    /// A note's subject as the front ends write it, and read back.
    #[test]
    fn a_subject_is_written_land_river_n_or_lake_n_and_read_back() {
        let written = [(About::Land, "land"), (About::River(0), "river 0"), (About::River(12), "river 12"), (About::Lake(1), "lake 1")];
        for (about, text) in written {
            assert_eq!(about.to_string(), text);
            assert_eq!(About::parse(text), Some(about), "{text}");
        }
        for text in ["", "Land", "River 1", "river", "river ", "river -1", "river +1", "river  1", " river 1", "river 1 ", "lake 1x", "lake one", "land 0", "brook 1", "river 99999999999999999999999", "river 01", "lake 00", "river 007"] {
            assert_eq!(About::parse(text), None, "\"{text}\"");
        }
    }

    #[test]
    fn a_bad_drawing_is_refused_before_growing() {
        let mut one_point = Drawing::example_valley(256);
        one_point.rivers[0].points.truncate(1);
        let bad_rules = GroundRules { wet_start: 0.9, wet_full: 0.8, ..Default::default() };
        let bad_water = WaterRules { bed_depth_min_m: 3.0, ..Default::default() };
        let bad_amplitude = GenConfig { detail_amplitude_m: -1.0, ..Default::default() };
        // The drawing first, then the ground rules, the water rules, the amplitude and the cell
        // size; all before growing, which would stop at once with `Cancelled` on the set flag.
        let cases = [
            (&one_point, GenConfig::default(), "river 1"),
            (&one_point, GenConfig { rules: bad_rules, water: bad_water, ..bad_amplitude }, "river 1"),
            (&Drawing::example_valley(256), GenConfig { rules: bad_rules, water: bad_water, ..bad_amplitude }, "wet_start"),
            (&Drawing::example_valley(256), GenConfig { water: bad_water, ..bad_amplitude }, "bed_depth_min_m"),
            (&Drawing::example_valley(256), bad_amplitude, "detail_amplitude_m"),
            (&Drawing::example_valley(256), GenConfig { cell_size_m: 2.0, ..Default::default() }, "cell_size_m"),
            (&Drawing::example_valley(256), GenConfig { cell_size_m: 0.5, ..bad_amplitude }, "detail_amplitude_m"),
        ];
        for (drawing, config, name) in cases {
            match generate_drawn(1, drawing, &config, &AtomicBool::new(true)) {
                Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains(name), "{e} should name {name}"),
                other => panic!("expected BadConfig naming {name}, got {:?}", other.map(|w| w.meta)),
            }
        }
    }

    #[test]
    fn a_set_flag_cancels_a_drawn_world() {
        let result = generate_drawn(1, &Drawing::example_valley(256), &GenConfig::default(), &AtomicBool::new(true));
        assert_eq!(result.err(), Some(LandformError::Cancelled));
    }

    /// Where `shifted` is not `world` lowered by `shift_m`: each height, course surface and lake
    /// level of `shifted` must be `world`'s less `shift_m` in `f32`, as `finish_world` and
    /// `Water::lower` lower them, bit for bit, and everything else but the meta's base elevation
    /// and timings must be equal, bit for bit. With a shift of 0 the two worlds are the same, since
    /// any number less 0 is itself, bit for bit. One line a failure.
    fn shift_failures(shifted: &World, world: &World, shift_m: f32) -> Vec<String> {
        let bits = |grid: &Grid<f32>| grid.data.iter().map(|value| value.to_bits()).collect::<Vec<u32>>();
        let lowered = |value: f32| (value - shift_m).to_bits();
        let mut failures = Vec::new();
        let differing = shifted.heights.data.iter().zip(&world.heights.data).filter(|&(a, &b)| a.to_bits() != lowered(b)).count();
        if differing > 0 || (shifted.heights.width, shifted.heights.height) != (world.heights.width, world.heights.height) {
            failures.push(format!("{differing} heights are not lowered by {shift_m} m"));
        }
        if shifted.ground != world.ground {
            failures.push("the ground maps differ".into());
        }
        if !shifted.layers.keys().eq(world.layers.keys()) {
            failures.push(format!("the layers {:?} and {:?}", shifted.layers.keys(), world.layers.keys()));
        }
        for (name, layer) in &world.layers {
            if shifted.layers.get(name).is_none_or(|other| bits(other) != bits(layer) || other.cell_size != layer.cell_size) {
                failures.push(format!("the layer {name} differs"));
            }
        }
        let (courses, other_courses) = (&shifted.water.rivers, &world.water.rivers);
        if courses.len() != other_courses.len() {
            failures.push(format!("{} courses and {}", courses.len(), other_courses.len()));
        }
        for (k, (course, other)) in courses.iter().zip(other_courses).enumerate() {
            let same_points = course.points.len() == other.points.len()
                && course.points.iter().zip(&other.points).all(|(a, b)| {
                    [a.x_m, a.z_m, a.width_m, a.depth_m].map(f32::to_bits) == [b.x_m, b.z_m, b.width_m, b.depth_m].map(f32::to_bits)
                        && a.surface_m.to_bits() == lowered(b.surface_m)
                });
            if !same_points || (course.drawn, course.from_lake, course.end) != (other.drawn, other.from_lake, other.end) {
                failures.push(format!("course {k} is not lowered by {shift_m} m"));
            }
        }
        let (lakes, other_lakes) = (&shifted.water.lakes, &world.water.lakes);
        let same_lake = |a: &LakeWater, b: &LakeWater| {
            a.drawn == b.drawn
                && [a.x_m, a.z_m, a.area_m2].map(f32::to_bits) == [b.x_m, b.z_m, b.area_m2].map(f32::to_bits)
                && a.level_m.to_bits() == lowered(b.level_m)
        };
        if lakes.len() != other_lakes.len() || !lakes.iter().zip(other_lakes).all(|(a, b)| same_lake(a, b)) {
            failures.push(format!("the lakes {lakes:?} are not {other_lakes:?} lowered by {shift_m} m"));
        }
        if (&shifted.notes, &shifted.notes_about) != (&world.notes, &world.notes_about) {
            failures.push(format!("the notes {:?} and {:?}, about {:?} and {:?}", shifted.notes, world.notes, shifted.notes_about, world.notes_about));
        }
        let (a, b) = (&shifted.meta, &world.meta);
        if (a.seed, &a.source_id, a.window, a.north) != (b.seed, &b.source_id, b.window, b.north) {
            failures.push(format!("the metas {a:?} and {b:?}"));
        }
        failures
    }

    #[test]
    fn the_fine_steps_from_a_grown_model_give_the_drawn_world() {
        let (seed, drawing, config, cancel) = (1, Drawing::example_valley(512), GenConfig::default(), AtomicBool::new(false));
        let grown = grow(&drawing, &config.water, seed, &cancel).expect("the drawing grows");
        let finished = finish_drawn(seed, &drawing, &config, &grown, Frame::Lowest, &cancel).expect("the model gives a world");
        let whole = drawn(seed, &drawing);
        let failures = shift_failures(&finished, &whole, 0.0);
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!(finished.meta.base_elevation_m.to_bits(), whole.meta.base_elevation_m.to_bits());
        assert!(finished == whole, "the worlds differ");
        // The world has water, notes are compared and the frame is shifted, so the test says something.
        assert!(!whole.water.rivers.is_empty() && !whole.water.lakes.is_empty(), "{:?}", whole.water.lakes);
        assert!(whole.meta.base_elevation_m != 0.0);
        // The model was grown before the call: the fine steps took all of `total_ms`.
        let (timings, whole_timings) = (finished.meta.timings, whole.meta.timings);
        assert_eq!(timings.grow_ms, 0.0);
        assert!(timings.total_ms > 0.0 && timings.total_ms >= timings.resample_ms + timings.water_ms, "{timings:?}");
        assert!(whole_timings.grow_ms > 0.0 && whole_timings.total_ms > whole_timings.grow_ms + whole_timings.water_ms, "{whole_timings:?}");
    }

    #[test]
    fn in_the_models_frame_a_drawn_world_differs_only_by_the_shift() {
        let (seed, drawing, config, cancel) = (1, Drawing::example_valley(512), GenConfig::default(), AtomicBool::new(false));
        let grown = grow(&drawing, &config.water, seed, &cancel).expect("the drawing grows");
        let lowest = finish_drawn(seed, &drawing, &config, &grown, Frame::Lowest, &cancel).expect("the model gives a world");
        let model = finish_drawn(seed, &drawing, &config, &grown, Frame::Model, &cancel).expect("the model gives a world");
        let base_m = lowest.meta.base_elevation_m;
        assert_eq!(model.meta.base_elevation_m, 0.0);
        assert_eq!(base_m, model.heights.data.iter().copied().fold(f32::INFINITY, f32::min), "the lowest cell in the model's frame");
        assert!(base_m != 0.0, "the shift is 0, so the test says nothing");
        let failures = shift_failures(&lowest, &model, base_m);
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(!model.water.rivers.is_empty() && !model.water.lakes.is_empty(), "{:?}", model.water.lakes);
    }

    #[test]
    fn the_fine_steps_check_what_they_read() {
        let drawing = Drawing::example_valley(256);
        let grown = grow(&drawing, &WaterRules::default(), 1, &AtomicBool::new(false)).expect("the drawing grows");
        let mut one_point = drawing.clone();
        one_point.rivers[0].points.truncate(1);
        let bad_rules = GroundRules { wet_start: 0.9, wet_full: 0.8, ..Default::default() };
        let bad_water = WaterRules { bed_depth_min_m: 3.0, ..Default::default() };
        let bad_amplitude = GenConfig { detail_amplitude_m: -1.0, ..Default::default() };
        let other_size = grow(&Drawing::example_valley(512), &WaterRules::default(), 1, &AtomicBool::new(false)).expect("the drawing grows");
        // A subject too many, and a note about a lake the drawing does not have.
        let mut unsaid = grown.clone();
        unsaid.notes_about.push(About::Land);
        let mut elsewhere = grown.clone();
        elsewhere.notes.push("Lake 9 holds no water; it was left out.".to_string());
        elsewhere.notes_about.push(About::Lake(drawing.lakes.len()));
        // As `generate_drawn` checks them: the drawing first, then the ground rules, the water
        // rules, the amplitude and the cell size; then the model. All before the flag is read,
        // which would stop the steps at once with `Cancelled`.
        let cases = [
            (&one_point, &grown, GenConfig::default(), "river 1"),
            (&one_point, &other_size, GenConfig { rules: bad_rules, water: bad_water, ..bad_amplitude }, "river 1"),
            (&drawing, &other_size, GenConfig { rules: bad_rules, water: bad_water, ..bad_amplitude }, "wet_start"),
            (&drawing, &other_size, GenConfig { water: bad_water, ..bad_amplitude }, "bed_depth_min_m"),
            (&drawing, &other_size, bad_amplitude, "detail_amplitude_m"),
            (&drawing, &other_size, GenConfig { cell_size_m: 2.0, ..Default::default() }, "cell_size_m"),
            (&drawing, &grown, GenConfig { cell_size_m: 0.5, ..bad_amplitude }, "detail_amplitude_m"),
            (&drawing, &other_size, GenConfig::default(), "model's heights"),
            (&without_water(256), &grown, GenConfig::default(), "rivers and lakes"),
            (&drawing, &unsaid, GenConfig::default(), "notes a subject"),
            (&drawing, &elsewhere, GenConfig::default(), "notes must each be about"),
        ];
        for (drawing, grown, config, name) in cases {
            for frame in [Frame::Lowest, Frame::Model] {
                match finish_drawn(1, drawing, &config, grown, frame, &AtomicBool::new(true)) {
                    Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains(name), "{frame:?}: {e} should name {name}"),
                    other => panic!("{frame:?}: expected BadConfig naming {name}, got {:?}", other.map(|w| w.meta)),
                }
            }
        }
        let result = finish_drawn(1, &drawing, &GenConfig::default(), &grown, Frame::Model, &AtomicBool::new(true));
        assert_eq!(result.err(), Some(LandformError::Cancelled));
    }

    /// The example drawing `size_m` across with neither rivers nor lakes.
    fn without_water(size_m: u32) -> Drawing {
        Drawing { rivers: Vec::new(), lakes: Vec::new(), ..Drawing::example_valley(size_m) }
    }

    /// `drawing`'s world from seed 1 under `water`.
    fn drawn_with(drawing: &Drawing, water: WaterRules) -> World {
        let config = GenConfig { water, ..GenConfig::default() };
        generate_drawn(1, drawing, &config, &AtomicBool::new(false)).expect("the drawing gives a world")
    }

    /// The ground of `world` at world `(x, z)`, bilinear between the cells around it, the nearest
    /// edge's beyond them. Cell `(i, j)` lies at world `(i - size / 2, j - size / 2)`.
    fn ground_at(world: &World, (x_m, z_m): (f32, f32)) -> f32 {
        let heights = &world.heights;
        let (width, height) = (heights.width, heights.height);
        let column = (f64::from(x_m) + (width / 2) as f64).clamp(0.0, (width - 1) as f64);
        let row = (f64::from(z_m) + (height / 2) as f64).clamp(0.0, (height - 1) as f64);
        let (i, j) = (column.floor() as usize, row.floor() as usize);
        let (next_i, next_j) = ((i + 1).min(width - 1), (j + 1).min(height - 1));
        let (across, down) = (column - i as f64, row - j as f64);
        let value = |i: usize, j: usize| f64::from(heights.get(i, j));
        let top = value(i, j) + (value(next_i, j) - value(i, j)) * across;
        let bottom = value(i, next_j) + (value(next_i, next_j) - value(i, next_j)) * across;
        (top + (bottom - top) * down) as f32
    }

    /// How far `p` lies from the stretch from `a` to `b`, in metres.
    fn from_stretch_m(a: (f32, f32), b: (f32, f32), p: (f64, f64)) -> f64 {
        let (a, b) = ((f64::from(a.0), f64::from(a.1)), (f64::from(b.0), f64::from(b.1)));
        let along = (b.0 - a.0, b.1 - a.1);
        let squared = along.0 * along.0 + along.1 * along.1;
        let share = if squared > 0.0 { (((p.0 - a.0) * along.0 + (p.1 - a.1) * along.1) / squared).clamp(0.0, 1.0) } else { 0.0 };
        (a.0 + share * along.0 - p.0).hypot(a.1 + share * along.1 - p.1)
    }

    /// The cells of a `size` x `size` world, as indices, whose middle lies within `reach_m` of
    /// world `(x, z)`.
    fn cells_within(size: usize, (x_m, z_m): (f64, f64), reach_m: f64) -> impl Iterator<Item = usize> {
        let half = (size / 2) as f64;
        let span = move |at: f64| (at + half - reach_m).ceil().max(0.0) as usize..=((at + half + reach_m).floor().max(0.0) as usize).min(size - 1);
        span(z_m).flat_map(move |j| span(x_m).map(move |i| (i, j))).filter_map(move |(i, j)| {
            ((i as f64 - half - x_m).hypot(j as f64 - half - z_m) <= reach_m).then_some(j * size + i)
        })
    }

    /// Whether each cell of `world`, grown from `drawing`, lies within `beyond_m` of a course's
    /// water, each stretch between two points taken at the wider of its two widths, or within
    /// `lake_beyond_m` of where a lake's water may lie: 1.5 of its radii from its centre.
    fn near_water(world: &World, drawing: &Drawing, beyond_m: f64, lake_beyond_m: f64) -> Vec<bool> {
        let size = world.heights.width;
        let mut near = vec![false; size * size];
        for course in &world.water.rivers {
            for pair in course.points.windows(2) {
                let (a, b) = ((pair[0].x_m, pair[0].z_m), (pair[1].x_m, pair[1].z_m));
                let reach_m = f64::from(pair[0].width_m.max(pair[1].width_m)) / 2.0 + beyond_m;
                let middle = ((f64::from(a.0) + f64::from(b.0)) / 2.0, (f64::from(a.1) + f64::from(b.1)) / 2.0);
                let half_length_m = f64::from(b.0 - a.0).hypot(f64::from(b.1 - a.1)) / 2.0;
                for cell in cells_within(size, middle, half_length_m + reach_m) {
                    let at = ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
                    if from_stretch_m(a, b, at) <= reach_m {
                        near[cell] = true;
                    }
                }
            }
        }
        for lake in &world.water.lakes {
            let drawn = drawing.lakes[lake.drawn];
            for cell in cells_within(size, (drawn.x_m, drawn.z_m), 1.5 * f64::from(drawn.radius_m) + lake_beyond_m) {
                near[cell] = true;
            }
        }
        near
    }

    #[test]
    fn a_drawn_world_has_its_water() {
        let drawing = Drawing::example_valley(2048);
        let world = drawn(1, &drawing);
        let water = &world.water;
        let drawn_courses = water.rivers.iter().filter(|course| course.drawn.is_some()).count();
        assert_eq!(drawn_courses, 3, "the big river into and out of the lake, and the small river");
        assert!(!water.lakes.is_empty(), "no lake");
        for lake in &water.lakes {
            assert!(lake.drawn < drawing.lakes.len(), "{lake:?} names no drawn lake");
        }
        let half = drawing.size_m as f32 / 2.0;
        let mut failures = Vec::new();
        for (index, course) in water.rivers.iter().enumerate() {
            let name = format!("course {index} ({:?})", course.drawn);
            assert!(course.points.len() >= 2, "{name} has {} points", course.points.len());
            // Inside the world: within the span of its cells, from `-half` to `half - 1` m on both axes.
            let inside = |at_m: f32| (-half..=half - 1.0).contains(&at_m);
            if let Some(point) = course.points.iter().find(|point| !inside(point.x_m) || !inside(point.z_m)) {
                failures.push(format!("{name}: {point:?} lies outside the world"));
            }
            if let Some(k) = (1..course.points.len()).find(|&k| course.points[k].surface_m > course.points[k - 1].surface_m) {
                failures.push(format!("{name}: the surface rises at point {k}"));
            }
            for (k, point) in course.points.iter().enumerate() {
                let ground_m = ground_at(&world, (point.x_m, point.z_m));
                // A spring's head has no depth yet: its water lies on the ground.
                let below = if point.depth_m < 0.05 { ground_m <= point.surface_m + 1e-3 } else { ground_m < point.surface_m };
                if !below {
                    failures.push(format!("{name} point {k}: the ground {ground_m} m, the surface {} m", point.surface_m));
                }
            }
            let in_range = match course.end {
                CourseEnd::Edge => true,
                CourseEnd::River(k) => k < water.rivers.len() && k != index,
                CourseEnd::Lake(k) => k < water.lakes.len(),
            };
            if !in_range || course.from_lake.is_some_and(|lake| lake >= water.lakes.len()) {
                failures.push(format!("{name}: an index out of range, from {:?} to {:?}", course.from_lake, course.end));
            }
        }
        assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
    }

    /// How far world `(x, z)` lies from the nearest point of `course`'s line, and the half width and
    /// surface of its water there, linear along each stretch (`held::nearest_on_course`, the
    /// surface as single precision).
    fn nearest_on_course(course: &RiverCourse, at: (f64, f64)) -> (f64, f64, f32) {
        let (apart_m, half_width_m, surface_m) = crate::held::nearest_on_course(course, at);
        (apart_m, half_width_m, surface_m as f32)
    }

    /// The failures `check` finds in the example in each kind of country, from seeds 1, 2 and 3, at
    /// 2048 m (given its name, its drawing and its world), each world made and checked on a thread
    /// of its own and dropped there.
    fn example_world_failures(check: impl Fn(&str, &Drawing, &World) -> Vec<String> + Sync) -> Vec<String> {
        example_world_failures_under(WaterRules::default(), check)
    }

    /// `example_world_failures` with every world made under the water rules `water`.
    fn example_world_failures_under(water: WaterRules, check: impl Fn(&str, &Drawing, &World) -> Vec<String> + Sync) -> Vec<String> {
        example_failures_from(&[1, 2, 3], water, check)
    }

    /// The seeds of the counting program's example worlds (`examples/held_water.rs`, its `example`
    /// set): the example at 2048 m in each kind of country from these.
    const COUNTED_SEEDS: [u64; 3] = [11, 23, 42];

    /// `example_world_failures` on the example in each kind of country from each of `seeds`, every
    /// world made under the water rules `water`.
    fn example_failures_from(seeds: &[u64], water: WaterRules, check: impl Fn(&str, &Drawing, &World) -> Vec<String> + Sync) -> Vec<String> {
        let check = &check;
        std::thread::scope(|scope| {
            let checking: Vec<_> = (COUNTRY_NAMES.iter().flat_map(|&name| seeds.iter().map(move |&seed| (name, seed))))
                .map(|(name, seed)| {
                    scope.spawn(move || {
                        let drawing = Drawing { country: Country::named(name).expect("a known country"), ..Drawing::example_valley(2048) };
                        let config = GenConfig { water, ..GenConfig::default() };
                        let world = generate_drawn(seed, &drawing, &config, &AtomicBool::new(false)).expect("the drawing gives a world");
                        check(&format!("{name}, seed {seed}"), &drawing, &world)
                    })
                })
                .collect();
            checking.into_iter().flat_map(|failures| failures.join().expect("an example world generates")).collect()
        })
    }

    #[test]
    fn an_outflow_leaves_its_lake_at_its_level_and_no_lake_water_stands_above_it() {
        let failures = example_world_failures(|name, drawing, world| {
            let mut failures = Vec::new();
            let (layer, size) = (&world.layers[WATER_LAYER], world.heights.width);
            let outflows: Vec<(usize, &RiverCourse)> =
                world.water.rivers.iter().enumerate().filter(|(_, course)| course.from_lake.is_some()).collect();
            if outflows.is_empty() {
                failures.push(format!("{name}: no course flows out of a lake"));
            }
            for &(index, course) in &outflows {
                let lake = course.from_lake.expect("an outflow");
                let (first_m, level_m) = (course.points[0].surface_m, world.water.lakes[lake].level_m);
                if (first_m - level_m).abs() > 1e-4 {
                    failures.push(format!("{name}: course {index} leaves lake {lake} at {first_m} m, its level {level_m} m"));
                }
                // Where the lake's water may lie, within 1.5 of its radii of its centre, no cell
                // within reach of the course's banks holds water more than 0.05 m above the course's
                // surface at its line's nearest point, where that lies more than 0.05 m below the
                // lake's level. (Farther out a tributary comes down to meet the course, its water
                // above the course's within the course's banks' reach.)
                let drawn = drawing.lakes[world.water.lakes[lake].drawn];
                let within_m = 1.5 * f64::from(drawn.radius_m);
                for cell in cells_within(size, (drawn.x_m, drawn.z_m), within_m).filter(|&cell| layer.data[cell] > 0.0) {
                    let at = ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
                    let (apart_m, half_width_m, surface_m) = nearest_on_course(course, at);
                    let water_m = world.heights.data[cell] + layer.data[cell];
                    if apart_m - half_width_m <= BANK_REACH_M && surface_m < level_m - 0.05 && water_m > surface_m + 0.05 {
                        failures.push(format!(
                            "{name}: water at {water_m} m at {at:?}, {apart_m:.1} m from course {index}, whose surface there is {surface_m} m (level {level_m} m)"
                        ));
                    }
                }
            }
            failures
        });
        assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
    }

    /// No course stands above its ground: at every course point the water's surface lies no more
    /// than 0.5 m above its floor, the land the model grew with its lakes filled, read at the point
    /// (`water::floor_m`): the valley's floor in a valley, the lake's level in a lake. Worked out
    /// through `generate_drawn`, fine shapes included, with the land grown once more for the floor,
    /// on the example's nine worlds, the example at 4096 m (hills, seed 1) and the example with a
    /// lake 200 m in radius (hills, seeds 1 to 3). The guard of a lake's level, of the grade out of
    /// a lake and of where a channel's end is moved: each, gone wrong, stood water metres above the
    /// land.
    #[test]
    fn no_course_stands_above_its_ground() {
        let above = |name: &str, drawing: &Drawing, world: &World| -> Vec<String> {
            let land = grow(drawing, &WaterRules::default(), world.meta.seed, &AtomicBool::new(false)).expect("the drawing grows");
            let size = (world.heights.width, world.heights.height);
            let (mut checked, mut high, mut worst) = (0, 0, (0.0_f32, String::new()));
            for (index, course) in world.water.rivers.iter().enumerate() {
                for (k, point) in course.points.iter().enumerate() {
                    checked += 1;
                    // The surface back in the land model's frame, less the floor there. A surface
                    // held at 0.5 m above its floor may read a hair more.
                    let at = (f64::from(point.x_m), f64::from(point.z_m));
                    let above_m = point.surface_m + world.meta.base_elevation_m - floor_m(&land, size, at);
                    if above_m > 0.5 + 1e-4 {
                        high += 1;
                        if above_m > worst.0 {
                            worst = (above_m, format!("course {index} ({:?}, {:?}) point {k} at {at:?}", course.drawn, course.end));
                        }
                    }
                }
            }
            match high {
                0 => Vec::new(),
                _ => vec![format!("{name}: {high} of {checked} course points stand more than 0.5 m above their floor, up to {} m at {}", worst.0, worst.1)],
            }
        };
        let mut failures = example_world_failures(above);
        let mut wide: Vec<(String, Drawing, u64)> = (1..=3)
            .map(|seed| {
                let mut drawing = Drawing::example_valley(2048);
                drawing.lakes[0].radius_m = 200.0;
                (format!("hills, lake radius 200 m, seed {seed}"), drawing, seed)
            })
            .collect();
        wide.push(("the example at 4096 m, seed 1".to_string(), Drawing::example_valley(4096), 1));
        failures.extend(std::thread::scope(|scope| {
            let checking: Vec<_> = (wide.iter())
                .map(|(name, drawing, seed)| scope.spawn(move || above(name, drawing, &drawn(*seed, drawing))))
                .collect();
            checking.into_iter().flat_map(|failures| failures.join().expect("a world is checked")).collect::<Vec<String>>()
        }));
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// A 64-bit FNV-1a hash of `world`'s courses' lines and widths: the number of courses, and for
    /// each its number of points and each point's `x_m`, `z_m` and `width_m` as bits.
    fn lines_and_widths_hash(world: &World) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut take = |value: u64| {
            for byte in value.to_le_bytes() {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        take(world.water.rivers.len() as u64);
        for course in &world.water.rivers {
            take(course.points.len() as u64);
            for point in &course.points {
                for value in [point.x_m, point.z_m, point.width_m] {
                    take(u64::from(value.to_bits()));
                }
            }
        }
        hash
    }

    /// Where the courses run and how wide they are is the land model's and the drawing's alone: in
    /// the counting program's nine example worlds (the example at 2048 m in each kind of country,
    /// seeds 11, 23 and 42) the courses' lines and widths hash (`lines_and_widths_hash`) to what was
    /// recorded before the channels were traced ahead of the lakes.
    #[test]
    fn the_courses_run_and_widen_as_before_the_lakes_settled() {
        /// Each world's hash, recorded at `62472ce`, where the lakes were still found before the
        /// channels were traced. The channels read only the land model, the drawing and the seed,
        /// none of which a commit since `master` changes, so these are `master`'s too.
        const RECORDED: [(&str, u64); 9] = [
            ("gentle, seed 11", 0x5692_0943_5f0e_2dbd),
            ("gentle, seed 23", 0x2c66_62bf_f096_3f48),
            ("gentle, seed 42", 0x274c_4df3_c297_1f9c),
            ("hills, seed 11", 0xbba1_b4a7_d957_8ff2),
            ("hills, seed 23", 0xc413_275c_d5cf_1fcf),
            ("hills, seed 42", 0x2a9f_6345_32ff_9d02),
            ("mountains, seed 11", 0xe3c6_e080_9b1b_5a16),
            ("mountains, seed 23", 0x467c_b2c1_c84a_bd6b),
            ("mountains, seed 42", 0x6e55_6e08_69c5_eab7),
        ];
        let failures = example_failures_from(&COUNTED_SEEDS, WaterRules::default(), |name, _, world| {
            let hash = lines_and_widths_hash(world);
            match RECORDED.iter().find(|(recorded, _)| *recorded == name) {
                Some(&(_, recorded)) if recorded == hash => Vec::new(),
                found => vec![format!("{name}: the courses' lines and widths hash to {hash:#018x}, recorded {found:?}")],
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The example's lake keeps the level the land model filled it to, or settles a little below
    /// it, and its ground holds it: in the counting program's nine example worlds (seeds 11, 23 and
    /// 42) every drawn lake holds water, its level, back in the land model's frame, lies no more
    /// than `SETTLED_MOST_M` below the model's (the land grown again), and no dry cell stands more
    /// than 0.3 m below a lake's water beside it (`unheld`'s `dry_lake` is 0).
    #[test]
    fn the_example_lakes_keep_their_level_on_ground_that_holds_them() {
        /// How far below the model's level an example lake may settle, in metres (the prototype
        /// measured 0.10 m in gentle country from seed 23, and none elsewhere).
        const SETTLED_MOST_M: f32 = 0.15;
        let failures = example_failures_from(&COUNTED_SEEDS, WaterRules::default(), |name, drawing, world| {
            let mut failures = Vec::new();
            if world.water.lakes.len() != drawing.lakes.len() {
                failures.push(format!("{name}: {} of {} drawn lakes hold water: {:?}", world.water.lakes.len(), drawing.lakes.len(), world.notes));
            }
            let land = grow(drawing, &WaterRules::default(), world.meta.seed, &AtomicBool::new(false)).expect("the drawing grows");
            for lake in &world.water.lakes {
                let model_m = land.lakes.iter().find(|grown| grown.drawn == lake.drawn).map_or(f32::NAN, |grown| grown.level_m);
                let down_m = model_m - (lake.level_m + world.meta.base_elevation_m);
                println!("{name}: lake {} settled {down_m:.3} m below the model's level, {model_m:.3} m", lake.drawn + 1);
                if !(-1e-4..=SETTLED_MOST_M).contains(&down_m) {
                    failures.push(format!("{name}: lake {} settled {down_m} m below the model's level, {model_m} m", lake.drawn + 1));
                }
            }
            let found = crate::held::unheld(world).expect("a drawn world has the water layer");
            if found.dry_lake != 0 {
                failures.push(format!("{name}: {} dry cells more than 0.3 m below a lake's water beside them, the worst {} m at {:?}", found.dry_lake, found.worst_dry_m, found.worst_dry_at));
            }
            failures
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// No lake's water stands over lower water beside it: in the counting program's nine example
    /// worlds (seeds 11, 23 and 42) `unheld`'s `lake_over_water` is 0, as a course out of a lake
    /// holds the lake's level for as long as the lake lies beside its bed, and the lake keeps every
    /// cell it settled with.
    #[test]
    fn no_lake_water_stands_over_lower_water_in_the_example_worlds() {
        let failures = example_failures_from(&COUNTED_SEEDS, WaterRules::default(), |name, _, world| {
            let found = crate::held::unheld(world).expect("a drawn world has the water layer");
            match found.lake_over_water {
                0 => Vec::new(),
                count => vec![format!("{name}: {count} lake cells stand more than 0.3 m over lower water beside them")],
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Three lakes of radius 40 m drawn in a row, at (-60, 0), (0, 0) and (60, 5), on one river
    /// 8 m wide from (-480, 0) through (-100, 0) and (100, 10) to (480, 40), on the example's
    /// picture at 1024 m in hill country. From seeds 2 and 3 the land model holds the three as one
    /// water body, at one level: each lake's water ends at its reach in the middle of that body,
    /// and beyond it lies a later lake's water at the same level, which is no leak. So all three
    /// are listed, none is lowered or left out, the measure (`unheld`) counts nothing, and their
    /// levels lie within `UNHELD_LIMIT_M` of each other. The bound is the measure's limit, not
    /// nothing: the last lake, which the river leaves, may still settle for the ground beside its
    /// own outflow's bed, as it does drawn alone; from seed 2 lake 3 comes down 0.25 m so, where
    /// the ground beside the bed's end falls below its level and the model holds no water, and the
    /// levels lie 0.2514 m apart. Every other lake (from seed 3 all three) keeps the model's level,
    /// bit for bit.
    #[test]
    fn lakes_on_one_water_body_stand_at_one_level() {
        /// The lakes that settle for the ground beside their own outflow's bed: the seed and the
        /// drawn lake (counting from 0).
        const OWN_OUTFLOW: [(u64, usize); 1] = [(2, 2)];
        let at = |x_m: f64, z_m: f64| Lake { x_m, z_m, radius_m: 40.0 };
        let drawing = Drawing {
            rivers: vec![River { points: vec![(-480.0, 0.0), (-100.0, 0.0), (100.0, 10.0), (480.0, 40.0)], width_m: 8.0 }],
            lakes: vec![at(-60.0, 0.0), at(0.0, 0.0), at(60.0, 5.0)],
            country: Country::named("hills").expect("a known country"),
            ..Drawing::example_valley(1024)
        };
        let failures: Vec<String> = std::thread::scope(|scope| {
            let drawing = &drawing;
            let checking: Vec<_> = [2, 3]
                .into_iter()
                .map(|seed| {
                    scope.spawn(move || {
                        let (world, mut failures) = (drawn(seed, drawing), Vec::new());
                        let levels: Vec<f32> = world.water.lakes.iter().map(|lake| lake.level_m).collect();
                        println!("hills, seed {seed}: the lakes' levels {levels:?}, the notes {:?}", world.notes);
                        if levels.len() != drawing.lakes.len() {
                            failures.push(format!("seed {seed}: {} of {} drawn lakes hold water", levels.len(), drawing.lakes.len()));
                        }
                        let apart_m = levels.iter().copied().fold(f32::NEG_INFINITY, f32::max) - levels.iter().copied().fold(f32::INFINITY, f32::min);
                        if apart_m > crate::held::UNHELD_LIMIT_M {
                            failures.push(format!("seed {seed}: the lakes stand at {levels:?} m, {apart_m} m apart"));
                        }
                        let land = grow(drawing, &WaterRules::default(), seed, &AtomicBool::new(false)).expect("the drawing grows");
                        for lake in world.water.lakes.iter().filter(|lake| !OWN_OUTFLOW.contains(&(seed, lake.drawn))) {
                            let model_m = land.lakes.iter().find(|grown| grown.drawn == lake.drawn).map_or(f32::NAN, |grown| grown.level_m);
                            if lake.level_m.to_bits() != (model_m - world.meta.base_elevation_m).to_bits() {
                                failures.push(format!("seed {seed}: lake {} stands at {} m, the model's level {model_m} m", lake.drawn + 1, lake.level_m + world.meta.base_elevation_m));
                            }
                        }
                        let said: Vec<&String> = world.notes.iter().filter(|note| note.contains("lowered") || note.contains("left out")).collect();
                        if !said.is_empty() {
                            failures.push(format!("seed {seed}: {said:?}"));
                        }
                        let found = crate::held::unheld(&world).expect("a drawn world has the water layer");
                        if found != crate::held::Unheld::default() {
                            failures.push(format!("seed {seed}: the measure counts {found:?}"));
                        }
                        failures
                    })
                })
                .collect();
            checking.into_iter().flat_map(|failures| failures.join().expect("a world is made and counted")).collect()
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The three lakes in a row of `lakes_on_one_water_body_stand_at_one_level`, in hill country
    /// from seeds 1, 4 and 5 and gentle country from seeds 1 and 5. In four of these worlds (hills
    /// seeds 1 and 5, gentle seeds 1 and 5) the land model holds lake 1 on a water body of its own,
    /// 0.53, 0.17, 0.21 and 0.04 m above the body of lakes 2 and 3, and the 1 m ground parts the
    /// two: each lake keeps its own model level, so their levels are bound to lie within
    /// `UNHELD_LIMIT_M` of each other only where the model fills lakes 1 and 2 to within
    /// `AT_LEVEL_M` of each other (hills seed 4, one body; gentle seed 5, two bodies 0.041 m
    /// apart). Every lake holds water at the model's level, bit for bit, but the last lake from
    /// hills seeds 4 and 5, which settles one step of the scan (`SETTLE_STEP_M`) for the ground
    /// beside its own outflow's bed (the only cells that leak at the model's level lie beside that
    /// bed, 55 to 72 m east of its centre, 0.38 and 0.42 m under the level). No note says a lake was
    /// lowered or left out, the measure (`unheld`) counts nothing, and lake 1's water lies in its
    /// own bowl: at least `OWN_BOWL_CELLS` cells within its radius, 40 m, of its centre hold water at
    /// its level. That count reads the water layer, which does not tell one lake's cells from
    /// another's, so it is asserted where lake 1's level lies apart from the others' by more than
    /// the count's tolerance; from hills seed 4 the three stand within 0.002 m, lake 2's water in
    /// lake 1's circle would count too, and the lakes, one water, are bound by the spread instead.
    /// Its water starts from the candidate nearest its drawn centre. Found from the deepest
    /// candidate within its reach, it was a sliver in the edge of lake 2's bowl (43 cells from
    /// hills seed 1), which leaked at every level, and in hills seeds 1 and 5 and gentle seed 1 the
    /// lake drained away; found from the deepest inside its drawn circle, in gentle seed 5 it was
    /// 1,232 cells in the west of lake 2's bowl, which the circle reaches deeper than lake 1's own
    /// floor, and lake 1's own bowl stood dry, 3.9 m deep, with nothing counted.
    #[test]
    fn a_lake_on_a_water_body_of_its_own_beside_others_holds_its_bowl() {
        /// The fewest cells within its radius of its centre that hold lake 1's water.
        const OWN_BOWL_CELLS: usize = 1_500;
        /// How near a water cell's surface lies to lake 1's level to count as its water, in metres.
        const LEVEL_WITHIN_M: f32 = 0.002;
        /// Each world's country and seed, whether the land model fills lakes 1 and 2 to within
        /// `AT_LEVEL_M` of each other, whether lake 1's level lies apart from the others' so that
        /// its own water is counted, and the lake (counting from 0) that settles one step for its
        /// own outflow's bed.
        const WORLDS: [(&str, u64, bool, bool, Option<usize>); 5] = [
            ("hills", 1, false, true, None),
            ("hills", 4, true, false, Some(2)),
            ("hills", 5, false, true, Some(2)),
            ("gentle", 1, false, true, None),
            ("gentle", 5, true, true, None),
        ];
        let at = |x_m: f64, z_m: f64| Lake { x_m, z_m, radius_m: 40.0 };
        let failures: Vec<String> = std::thread::scope(|scope| {
            let checking: Vec<_> = WORLDS
                .into_iter()
                .map(|(country, seed, one_level, counted, own_outflow)| {
                    scope.spawn(move || {
                        let name = format!("{country}, seed {seed}");
                        let drawing = Drawing {
                            rivers: vec![River { points: vec![(-480.0, 0.0), (-100.0, 0.0), (100.0, 10.0), (480.0, 40.0)], width_m: 8.0 }],
                            lakes: vec![at(-60.0, 0.0), at(0.0, 0.0), at(60.0, 5.0)],
                            country: Country::named(country).expect("a known country"),
                            ..Drawing::example_valley(1024)
                        };
                        let (world, mut failures) = (drawn(seed, &drawing), Vec::new());
                        let levels: Vec<f32> = world.water.lakes.iter().map(|lake| lake.level_m).collect();
                        println!("{name}: the lakes' levels {levels:?}, the notes {:?}", world.notes);
                        if levels.len() != drawing.lakes.len() {
                            failures.push(format!("{name}: {} of {} drawn lakes hold water", levels.len(), drawing.lakes.len()));
                        }
                        let land = grow(&drawing, &WaterRules::default(), seed, &AtomicBool::new(false)).expect("the drawing grows");
                        let model_of = |drawn: usize| land.lakes.iter().find(|grown| grown.drawn == drawn).map_or(f32::NAN, |grown| grown.level_m);
                        let apart_m = model_of(0) - model_of(1);
                        if one_level != (f64::from(apart_m).abs() <= crate::water::AT_LEVEL_M) {
                            failures.push(format!("{name}: the model holds lake 1 {apart_m} m above lake 2"));
                        }
                        for lake in &world.water.lakes {
                            let (model_m, level_m) = (model_of(lake.drawn), lake.level_m + world.meta.base_elevation_m);
                            let expected_m = if own_outflow == Some(lake.drawn) { model_m - crate::water::SETTLE_STEP_M } else { model_m };
                            let kept = if own_outflow == Some(lake.drawn) {
                                (level_m - expected_m).abs() < 1e-4
                            } else {
                                lake.level_m.to_bits() == (model_m - world.meta.base_elevation_m).to_bits()
                            };
                            if !kept {
                                failures.push(format!("{name}: lake {} stands at {level_m} m, not {expected_m} m (the model's level {model_m} m)", lake.drawn + 1));
                            }
                        }
                        if one_level {
                            let apart_m = levels.iter().copied().fold(f32::NEG_INFINITY, f32::max) - levels.iter().copied().fold(f32::INFINITY, f32::min);
                            if apart_m > crate::held::UNHELD_LIMIT_M {
                                failures.push(format!("{name}: the lakes stand at {levels:?} m, {apart_m} m apart"));
                            }
                        }
                        let said: Vec<&String> =
                            world.notes.iter().filter(|note| note.contains("lowered") || note.contains("left out") || note.contains("drains away")).collect();
                        if !said.is_empty() {
                            failures.push(format!("{name}: {said:?}"));
                        }
                        let found = crate::held::unheld(&world).expect("a drawn world has the water layer");
                        if found != crate::held::Unheld::default() {
                            failures.push(format!("{name}: the measure counts {found:?}"));
                        }
                        // Lake 1's own bowl: the cells within its radius of its centre whose water
                        // stands at its level, counted where no other lake stands at that level.
                        if let Some(first) = world.water.lakes.iter().find(|lake| lake.drawn == 0) {
                            let apart = world.water.lakes.iter().filter(|lake| lake.drawn != 0).all(|lake| (lake.level_m - first.level_m).abs() >= LEVEL_WITHIN_M);
                            if apart != counted {
                                failures.push(format!("{name}: lake 1's level apart from the others' is {apart}, not {counted}: {levels:?}"));
                            }
                            let (layer, size, drawn_1) = (&world.layers[WATER_LAYER], world.heights.width, drawing.lakes[0]);
                            let half = (size / 2) as f64;
                            let own = (0..size * size)
                                .filter(|&cell| {
                                    let (x, z) = ((cell % size) as f64 - half, (cell / size) as f64 - half);
                                    let surface_m = world.heights.data[cell] + layer.data[cell];
                                    (x - drawn_1.x_m).hypot(z - drawn_1.z_m) <= f64::from(drawn_1.radius_m)
                                        && layer.data[cell] > 0.0
                                        && (surface_m - first.level_m).abs() < LEVEL_WITHIN_M
                                })
                                .count();
                            println!("{name}: {own} cells within lake 1's radius hold water at its level");
                            if counted && own < OWN_BOWL_CELLS {
                                failures.push(format!("{name}: {own} cells within lake 1's radius hold its water, fewer than {OWN_BOWL_CELLS}"));
                            }
                        }
                        failures
                    })
                })
                .collect();
            checking.into_iter().flat_map(|failures| failures.join().expect("a world is made and counted")).collect()
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The example's worlds hold nothing to say: no lake left out, no river off its line or short
    /// of its end, no water standing more than 0.5 m above its floor.
    #[test]
    fn the_example_worlds_carry_no_notes() {
        let failures = example_world_failures(|name, _, world| match world.notes.is_empty() {
            true => Vec::new(),
            false => vec![format!("{name}: {:?}", world.notes)],
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The width the example's river 1 is drawn, in metres.
    const RIVER_1_M: f64 = 16.0;
    /// The rays across a course that one reading of the water's width averages
    /// (`water_width_m`), how far apart they lie along the course, and the steps a ray walks out
    /// from the line in, in metres.
    const WIDTH_RAYS: usize = 8;
    const RAY_SPACING_M: f64 = 0.25;
    const RAY_STEP_M: f64 = 0.05;

    /// How far each point of `course` lies along it from its first, in metres.
    fn along_course_m(course: &RiverCourse) -> Vec<f64> {
        let mut along_m = Vec::with_capacity(course.points.len());
        let mut sum_m = 0.0;
        for (k, point) in course.points.iter().enumerate() {
            if k > 0 {
                let before = course.points[k - 1];
                sum_m += (f64::from(point.x_m) - f64::from(before.x_m)).hypot(f64::from(point.z_m) - f64::from(before.z_m));
            }
            along_m.push(sum_m);
        }
        along_m
    }

    /// The water's width across `course` at its point `k`, read off `world`'s `water` layer: the
    /// mean over `WIDTH_RAYS` rays at right angles to the course there (to its direction from point
    /// `k - 1` to point `k + 1`), spread `RAY_SPACING_M` apart along it round the point, of how far
    /// each ray's water reaches. A ray walks out from the line both ways in steps of `RAY_STEP_M`
    /// until the cell that holds its step is dry, or for `reach_m` at most; its water runs from the
    /// first dry step on one side to the first on the other. Read so, the metre-wide cells'
    /// stairs average out between the rays and along the course.
    fn water_width_m(world: &World, course: &RiverCourse, k: usize, reach_m: f64) -> f64 {
        let layer = &world.layers[WATER_LAYER];
        let (size, half) = (layer.width, (layer.width / 2) as f64);
        let at = |point: &CoursePoint| (f64::from(point.x_m), f64::from(point.z_m));
        let (before, after) = (at(&course.points[k.saturating_sub(1)]), at(&course.points[(k + 1).min(course.points.len() - 1)]));
        let length = (after.0 - before.0).hypot(after.1 - before.1);
        let (dx, dz) = ((after.0 - before.0) / length, (after.1 - before.1) / length);
        let wet = |(x, z): (f64, f64)| {
            let (i, j) = ((x + half).round(), (z + half).round());
            let inside = (0.0..size as f64).contains(&i) && (0.0..size as f64).contains(&j);
            inside && layer.data[j as usize * size + i as usize] > 0.0
        };
        let origin = at(&course.points[k]);
        let mut reached_m = 0.0;
        for ray in 0..WIDTH_RAYS {
            let along_m = (ray as f64 - (WIDTH_RAYS - 1) as f64 / 2.0) * RAY_SPACING_M;
            let from = (origin.0 + along_m * dx, origin.1 + along_m * dz);
            for side in [1.0, -1.0] {
                let mut out_m = 0.0;
                while out_m < reach_m && wet((from.0 + side * out_m * dz, from.1 - side * out_m * dx)) {
                    out_m += RAY_STEP_M;
                }
                reached_m += out_m;
            }
        }
        reached_m / WIDTH_RAYS as f64
    }

    /// A reading of river 1's water stays this far from either end of its course, from every
    /// other course's points and from where a lake's water may lie, in metres: its rays' reach (the
    /// drawn width), the half width of the widest brook that may join it (as wide as the river),
    /// the banks' reach, over which water may lie beyond a waterline, and the 4 m between points.
    /// Nearer, the water the layer holds across the river may be a brook's or a lake's that meets
    /// the river's.
    const CLEAR_OF_OTHER_WATER_M: f64 = RIVER_1_M + RIVER_1_M / 2.0 + BANK_REACH_M + 4.0;

    /// The water's width (`water_width_m`, each ray reaching at most the drawn width) at every point
    /// of the example's river 1, in `world` grown from `drawing`, that lies `CLEAR_OF_OTHER_WATER_M`
    /// (40 m) or more along its course from either end of it, from every point of every other
    /// course and from 1.5 radii of the centre of every lake that holds water: where the water
    /// across the river is the river's alone.
    fn river_1_widths(world: &World, drawing: &Drawing) -> Vec<f64> {
        let clear_m = CLEAR_OF_OTHER_WATER_M;
        let at = |point: &CoursePoint| (f64::from(point.x_m), f64::from(point.z_m));
        let apart_m = |a: (f64, f64), b: (f64, f64)| (a.0 - b.0).hypot(a.1 - b.1);
        let lakes: Vec<((f64, f64), f64)> = (world.water.lakes.iter())
            .map(|lake| drawing.lakes[lake.drawn])
            .map(|lake| ((lake.x_m, lake.z_m), 1.5 * f64::from(lake.radius_m)))
            .collect();
        let mut widths = Vec::new();
        for (index, course) in world.water.rivers.iter().enumerate().filter(|(_, course)| course.drawn == Some(0)) {
            let others: Vec<(f64, f64)> = (world.water.rivers.iter().enumerate())
                .filter(|&(other, _)| other != index)
                .flat_map(|(_, other)| other.points.iter().map(at))
                .collect();
            let along_m = along_course_m(course);
            let length_m = along_m[along_m.len() - 1];
            for (k, point) in course.points.iter().enumerate() {
                let off_the_ends = along_m[k] >= clear_m && length_m - along_m[k] >= clear_m;
                let off_others = others.iter().all(|&other| apart_m(other, at(point)) >= clear_m);
                let off_lakes = lakes.iter().all(|&(centre, reach_m)| apart_m(centre, at(point)) >= reach_m + clear_m);
                if off_the_ends && off_others && off_lakes {
                    widths.push(water_width_m(world, course, k, RIVER_1_M));
                }
            }
        }
        widths
    }

    /// The least, the most and the mean of `values`.
    fn spread(values: &[f64]) -> (f64, f64, f64) {
        let least = values.iter().copied().fold(f64::INFINITY, f64::min);
        let most = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (least, most, values.iter().sum::<f64>() / values.len() as f64)
    }

    /// The spec's first check: the water of the example's river 1, drawn 16 m wide, wanders. Its
    /// width read off the `water` layer (`water_width_m`) at every point of its courses at least
    /// 40 m from either end of its course, and clear of the other water there (`river_1_widths`), is
    /// at most 0.85 of 16 m somewhere and at least 1.15 of it somewhere, and within 10 % of it on
    /// average, in each of the nine example worlds.
    #[test]
    fn a_drawn_rivers_water_wanders() {
        let failures = example_world_failures(|name, drawing, world| {
            let widths = river_1_widths(world, drawing);
            let (least, most, mean) = spread(&widths);
            let wanders = least <= 0.85 * RIVER_1_M && most >= 1.15 * RIVER_1_M && (mean - RIVER_1_M).abs() <= 0.1 * RIVER_1_M;
            match !widths.is_empty() && wanders {
                true => Vec::new(),
                false => vec![format!("{name}: river 1's water is {least:.2} to {most:.2} m wide, {mean:.2} m on average, at {} points", widths.len())],
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Past a spring's first 12 m, where it grows into its bed, every course holds at least
    /// `water_fill` of the shallowest bed's depth of water at its deep line, however wide the water
    /// wanders.
    #[test]
    fn the_bed_stays_under_the_water() {
        let rules = WaterRules::default();
        let least_m = rules.water_fill * rules.bed_depth_min_m;
        let failures = example_world_failures(|name, _, world| {
            let mut failures = Vec::new();
            for (index, course) in world.water.rivers.iter().enumerate() {
                let along_m = along_course_m(course);
                // A point 12 m along may read a hair more or less than 12 m in single precision.
                if let Some(k) = (0..course.points.len()).find(|&k| along_m[k] > 12.01 && course.points[k].depth_m < least_m) {
                    let depth_m = course.points[k].depth_m;
                    failures.push(format!("{name}: course {index} ({:?}) point {k}, {:.1} m along, holds {depth_m} m of water", course.drawn, along_m[k]));
                }
            }
            failures
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Decision 9: the example's river 2, a spring 1.2 to 4 m wide, wanders by about a metre at
    /// most, under what the layer's metre-wide cells can read, so its wander is read off its course
    /// points: somewhere its width falls by at least 5 % from one point to the first point 24 m on,
    /// in each of the nine example worlds, though the spring's own width only grows downstream
    /// (`channel::tests::widths_follow_the_rules`).
    #[test]
    fn a_springs_width_wanders_too() {
        let failures = example_world_failures(|name, _, world| {
            let falls = world.water.rivers.iter().filter(|course| course.drawn == Some(1)).any(|course| {
                let along_m = along_course_m(course);
                (0..course.points.len()).any(|a| {
                    let b = (a..course.points.len()).find(|&b| along_m[b] - along_m[a] >= 24.0);
                    b.is_some_and(|b| course.points[b].width_m <= 0.95 * course.points[a].width_m)
                })
            });
            match falls {
                true => Vec::new(),
                false => vec![format!("{name}: river 2's width never falls by 5 % over 24 m")],
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Review Focus 3: with no wander and no shift of the deep line, the bed under each course (its
    /// surface less the water's depth at the deep line) never rises downstream, in each of the
    /// nine example worlds; and the width of river 1's water, read as `a_drawn_rivers_water_wanders`
    /// reads it, spans less than 2 m, short of the range that test asks for.
    #[test]
    fn without_wander_no_bed_rises_downstream() {
        let still = WaterRules { width_wander: 0.0, bend_shift: 0.0, ..WaterRules::default() };
        let failures = example_world_failures_under(still, |name, drawing, world| {
            let mut failures = Vec::new();
            for (index, course) in world.water.rivers.iter().enumerate() {
                let bed_m = |k: usize| course.points[k].surface_m - course.points[k].depth_m;
                if let Some(k) = (1..course.points.len()).find(|&k| bed_m(k) > bed_m(k - 1)) {
                    failures.push(format!("{name}: course {index} ({:?}): the bed rises from {} to {} m at point {k}", course.drawn, bed_m(k - 1), bed_m(k)));
                }
            }
            let widths = river_1_widths(world, drawing);
            let (least, most, mean) = spread(&widths);
            if widths.is_empty() || most - least >= 2.0 {
                failures.push(format!("{name}: with no wander river 1's water is {least:.2} to {most:.2} m wide, {mean:.2} m on average"));
            }
            failures
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Review Focus 3: the rules at the ends of their ranges, the widest wander over the shortest
    /// wave and the largest shift of the deep line, still give water: hills, seed 1, every number of
    /// the world's water and every height finite, no surface rising downstream, and as many courses
    /// as under the shipped rules.
    #[test]
    fn rules_at_their_ends_still_give_water() {
        let drawing = Drawing::example_valley(2048);
        assert_eq!(drawing.country, Country::hills());
        let ends = WaterRules { width_wander: 0.5, wander_length_widths: 3.0, bend_shift: 0.5, ..WaterRules::default() };
        let (world, shipped) = std::thread::scope(|scope| {
            let world = scope.spawn(|| drawn_with(&drawing, ends));
            let shipped = scope.spawn(|| drawn(1, &drawing));
            (world.join().expect("a world at the ends"), shipped.join().expect("a world"))
        });
        let mut failures = Vec::new();
        for (index, course) in world.water.rivers.iter().enumerate() {
            let numbers = course.points.iter().flat_map(|point| [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m]);
            if numbers.clone().any(|value| !value.is_finite()) {
                failures.push(format!("course {index} holds a number that is not finite"));
            }
            if let Some(k) = (1..course.points.len()).find(|&k| course.points[k].surface_m > course.points[k - 1].surface_m) {
                failures.push(format!("course {index} ({:?}): the surface rises at point {k}", course.drawn));
            }
        }
        let lake_numbers = world.water.lakes.iter().flat_map(|lake| [lake.level_m, lake.x_m, lake.z_m, lake.area_m2]);
        if lake_numbers.into_iter().any(|value| !value.is_finite()) {
            failures.push("a lake holds a number that is not finite".to_string());
        }
        let layer = &world.layers[WATER_LAYER];
        let unfinite = layer.data.iter().chain(&world.heights.data).filter(|value| !value.is_finite()).count();
        if unfinite > 0 {
            failures.push(format!("{unfinite} heights and depths are not finite"));
        }
        assert!(layer.data.iter().any(|&depth| depth > 0.0), "no water on the layer");
        assert_eq!(world.water.rivers.len(), shipped.water.rivers.len(), "the courses at the ends against the shipped rules'");
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// Review Focus 5: the same drawing and seed give the same world, and another seed another
    /// wander: the widths of river 1's courses differ at most of their points. (Without the wander
    /// they are 16 m at every point, whatever the seed.)
    #[test]
    fn another_seed_wanders_differently_on_the_same_drawing() {
        let drawing = &Drawing::example_valley(2048);
        let (first, again, other) = std::thread::scope(|scope| {
            let worlds: Vec<_> = [1, 1, 2].into_iter().map(|seed| scope.spawn(move || drawn(seed, drawing))).collect();
            let mut worlds = worlds.into_iter().map(|world| world.join().expect("a world"));
            (worlds.next().unwrap(), worlds.next().unwrap(), worlds.next().unwrap())
        });
        assert!(first == again, "the same seed gave another world");
        let widths = |world: &World| -> Vec<f32> {
            (world.water.rivers.iter().filter(|course| course.drawn == Some(0)))
                .flat_map(|course| course.points.iter().map(|point| point.width_m))
                .collect()
        };
        let (one, two) = (widths(&first), widths(&other));
        let compared = one.len().min(two.len());
        let same = one.iter().zip(&two).filter(|(a, b)| a == b).count();
        assert!(compared > 100, "only {compared} points of river 1");
        assert!(same < compared / 2, "{same} of {compared} widths of river 1 are the same under another seed");
    }

    /// Inside a river's bed the water is the river's, and the bed is the one that wanders: in each
    /// of the nine example worlds no course stands more than 1e-4 m above the surface of the course
    /// it ends on at any of its points inside that course's bed, within half its width (after the
    /// wander, no less than `HALF_WIDTH_MIN_M`) of its line, the surface and the width taken at its
    /// line's nearest point.
    #[test]
    fn no_tributary_stands_above_its_river_inside_its_bed() {
        let failures = example_world_failures(|name, _, world| {
            let (mut checked, mut worst) = (0, (f32::NEG_INFINITY, String::new()));
            for (index, course) in world.water.rivers.iter().enumerate() {
                let CourseEnd::River(river) = course.end else { continue };
                for (k, point) in course.points.iter().enumerate() {
                    let at = (f64::from(point.x_m), f64::from(point.z_m));
                    let (apart_m, half_width_m, river_m) = nearest_on_course(&world.water.rivers[river], at);
                    if apart_m <= half_width_m.max(HALF_WIDTH_MIN_M) {
                        checked += 1;
                        if point.surface_m - river_m > worst.0 {
                            worst = (point.surface_m - river_m, format!("course {index} ({:?}) point {k} at {at:?}", course.drawn));
                        }
                    }
                }
            }
            match (checked, worst.0 > 1e-4) {
                (0, _) => vec![format!("{name}: no tributary point inside its river's bed was checked")],
                (_, true) => vec![format!("{name}: {} stands {} m above its river, inside its bed", worst.1, worst.0)],
                _ => Vec::new(),
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// The courses of `world` as lines of water, as the tests read them (`reach_of_water`): each
    /// point's half width no less than `HALF_WIDTH_MIN_M`.
    fn course_lines(world: &World) -> Vec<Vec<LinePoint>> {
        (world.water.rivers.iter())
            .map(|course| {
                (course.points.iter())
                    .map(|point| LinePoint {
                        x_m: f64::from(point.x_m),
                        z_m: f64::from(point.z_m),
                        half_width_m: (f64::from(point.width_m) / 2.0).max(HALF_WIDTH_MIN_M),
                        surface_m: f64::from(point.surface_m),
                    })
                    .collect()
            })
            .collect()
    }

    /// For each cell of `world`, grown from `drawing`, the index in `Water::lakes` of the lake whose
    /// water it holds, as narrowly as the tests can know it: a cell that holds water within 1.5 of
    /// the lake's radii of its centre, where its water may lie, whose water stands at the lake's
    /// level, to 1e-3 m. A lake keeps every cell it settled with, and a course out of it stands at
    /// its level along its stretch beside it, so water at the level there is the lake's.
    fn lake_water(world: &World, drawing: &Drawing) -> Vec<Option<usize>> {
        let (layer, size) = (&world.layers[WATER_LAYER], world.heights.width);
        let mut lake_of = vec![None; size * size];
        for (index, lake) in world.water.lakes.iter().enumerate() {
            let drawn = drawing.lakes[lake.drawn];
            for cell in cells_within(size, (drawn.x_m, drawn.z_m), 1.5 * f64::from(drawn.radius_m)) {
                if layer.data[cell] > 0.0 && (world.heights.data[cell] + layer.data[cell] - lake.level_m).abs() <= 1e-3 {
                    lake_of[cell] = Some(index);
                }
            }
        }
        lake_of
    }

    /// Whether `course` of a world `size` metres across begins on dry land, as the tests know it:
    /// out of no lake, its first point more than `BORDER_START_M` inside the world's border.
    fn on_dry_land(course: &RiverCourse, size: usize) -> bool {
        let half = (size / 2) as f64;
        let first = course.points[0];
        course.from_lake.is_none() && (half - f64::from(first.x_m).abs()).min(half - f64::from(first.z_m).abs()) > BORDER_START_M
    }

    /// For each course of `world`, grown from `drawing`, whose lines are `lines` (`course_lines`),
    /// the cells whose water is not its (`Ends`, read with 1e-3 m of slack): inside what it ends on,
    /// the bed of the course it ends on, or, for one that ends in a lake, surely the cells that hold
    /// the lake's water at its level (`lake_water`) and maybe every cell within 1.5 of its radii of
    /// its centre, where its water may lie; and, for one that begins on dry land (`on_dry_land`),
    /// straight beyond its first point (`behind_the_head`).
    fn course_ends(world: &World, drawing: &Drawing, lines: &[Vec<LinePoint>]) -> Vec<Ends> {
        let size = world.heights.width;
        let ends_on: Vec<Option<usize>> = (world.water.rivers.iter())
            .map(|course| match course.end {
                CourseEnd::River(joined) => Some(joined),
                CourseEnd::Lake(_) | CourseEnd::Edge => None,
            })
            .collect();
        let lake_of = lake_water(world, drawing);
        (world.water.rivers.iter().zip(lines).zip(inside_the_lines_ended_on(size, lines, &ends_on, 1e-3)))
            .map(|((course, line), ends)| {
                let ends = match course.end {
                    CourseEnd::Lake(lake) => {
                        let drawn = drawing.lakes[world.water.lakes[lake].drawn];
                        let mut maybe: Vec<usize> = cells_within(size, (drawn.x_m, drawn.z_m), 1.5 * f64::from(drawn.radius_m)).collect();
                        maybe.sort_unstable();
                        let surely = (0..size * size).filter(|&cell| lake_of[cell] == Some(lake)).collect();
                        ends.and(Ends { surely, maybe })
                    }
                    CourseEnd::River(_) | CourseEnd::Edge => ends,
                };
                match on_dry_land(course, size) {
                    true => ends.and(behind_the_head(size, line, BANK_REACH_M, 1e-3)),
                    false => ends,
                }
            })
            .collect()
    }

    /// Water lies only where a course's water reaches and the ground holds it, or where a lake may
    /// lie: outside every lake's 1.5 radii, a cell in a course's bed whose ground lies below its
    /// surface there (`kept_by_lines`: no farther from its line than half its width, at least
    /// `HALF_WIDTH_MIN_M`, at its line's nearest point, none of the cells whose water is not the
    /// course's, `course_ends`) holds water at the highest surface of the courses whose bed holds
    /// it. Beyond the beds, a wet cell's water stands at the lower of the highest level the courses
    /// whose water reaches it reach it at and the rim it would cross to get out, and a dry cell is
    /// one where that lies at or below its ground. A course's water fills its bed and spreads from
    /// it across the cells' sides through the cells within reach of its banks (no more than
    /// `BANK_REACH_M` beyond that half width), each at the lower of the level it came from and its
    /// own surface at its nearest point where its ground lies below that (`reach_of_water`). The rim
    /// is the lowest way out across the eight neighbours, a dry cell at its ground, a bed's or a
    /// lake's water (`lake_water`) at its level, the border below everything, and spread water at
    /// the higher of its ground and its rim (`rims`). All read with 1e-3 m of slack, and the rims as
    /// low and as high as the cells that may or may not lie in a bed, or be a lake's, let them be:
    /// as low with such a cell a way out at its ground, or a lake's at its level, and as high with
    /// it spread water over ground as high as the highest water it may keep. A cell that may or may
    /// not lie in a bed is held to both readings: wet, its water stands no lower than the lowest
    /// surface of the courses that may keep it or the lower of the level it is surely reached at and
    /// its rim, and no higher than the highest of those that may hold it; dry, it is not surely
    /// reached below a rim above its ground. And some cell near every course point holds water.
    #[test]
    fn the_water_layer_matches_the_courses_and_lakes() {
        let drawing = Drawing::example_valley(1024);
        let world = drawn(1, &drawing);
        let layer = &world.layers[WATER_LAYER];
        let size = world.heights.width;
        assert_eq!((layer.width, layer.height), (size, size));
        let mut in_a_lake = vec![false; size * size];
        for lake in &world.water.lakes {
            let drawn = drawing.lakes[lake.drawn];
            for cell in cells_within(size, (drawn.x_m, drawn.z_m), 1.5 * f64::from(drawn.radius_m)) {
                in_a_lake[cell] = true;
            }
        }
        let lines = course_lines(&world);
        let ends = course_ends(&world, &drawing, &lines);
        let reached = reach_of_water(&world.heights, &lines, &ends, 1e-3);
        let kept = kept_by_lines(&world.heights, &lines, &ends, 1e-3);
        let lake_of = lake_water(&world, &drawing);
        let ground_m = |cell: usize| f64::from(world.heights.data[cell]);
        let wet = |cell: usize| layer.data[cell] > 0.0;
        let level_m = |cell: usize| lake_of[cell].map(|lake| f64::from(world.water.lakes[lake].level_m));
        // What each cell is to the rims, as low as they may be and as high.
        let held_low: Vec<Held> = (0..size * size)
            .map(|cell| match (wet(cell), kept[cell], level_m(cell)) {
                (false, _, _) => Held::Dry,
                (true, kept, _) if kept.surely_m > f64::NEG_INFINITY => Held::Kept(kept.surely_m),
                (true, _, Some(level_m)) => Held::Kept(level_m),
                (true, kept, None) if kept.most_m > f64::NEG_INFINITY => Held::Dry,
                (true, _, None) => Held::Spread(ground_m(cell)),
            })
            .collect();
        let held_high: Vec<Held> = (0..size * size)
            .map(|cell| {
                let highest_m = kept[cell].most_m.max(level_m(cell).unwrap_or(f64::NEG_INFINITY));
                match (wet(cell), kept[cell].surely_m > f64::NEG_INFINITY) {
                    (false, _) => Held::Dry,
                    (true, true) => Held::Kept(highest_m),
                    (true, false) => Held::Spread(ground_m(cell).max(highest_m)),
                }
            })
            .collect();
        let (low, high) = (rims(&world.heights, &held_low), rims(&world.heights, &held_high));
        // The rim a cell would have as spread water, as low as it may be.
        let rim_at_least_m = |cell: usize| {
            let (i, j) = (cell % size, cell / size);
            if i == 0 || j == 0 || i + 1 == size || j + 1 == size {
                return ground_m(cell);
            }
            let around = (j - 1..=j + 1).flat_map(|row| (i - 1..=i + 1).map(move |column| row * size + column)).filter(|&next| next != cell);
            let out_m = around.map(|next| match held_low[next] {
                Held::Dry => ground_m(next),
                Held::Kept(level_m) => level_m,
                Held::Spread(next_m) => next_m.max(low[next]),
            });
            ground_m(cell).max(out_m.fold(f64::INFINITY, f64::min))
        };
        let (mut wet_cells, mut in_the_band, mut astray) = (0, (0, 0), Vec::new());
        for cell in (0..size * size).filter(|&cell| !in_a_lake[cell]) {
            let at = at_cell(size, cell);
            let ((lowest_m, highest_m), KeptBy { surely_m, least_m, most_m }, ground_m) = (reached[cell], kept[cell], ground_m(cell));
            if wet(cell) {
                wet_cells += 1;
                let water_m = f64::from(world.heights.data[cell] + layer.data[cell]);
                let (from_m, to_m, kind) = if surely_m > f64::NEG_INFINITY {
                    (surely_m, most_m, "in a course's bed")
                } else if most_m > f64::NEG_INFINITY {
                    in_the_band.0 += 1;
                    (least_m.min(lowest_m.min(rim_at_least_m(cell))), most_m.max(highest_m.min(high[cell])), "maybe in a course's bed")
                } else {
                    (lowest_m.min(low[cell]), highest_m.min(high[cell]), "beyond the beds")
                };
                if water_m < from_m - 1e-3 || water_m > to_m + 1e-3 {
                    astray.push(format!(
                        "water at {water_m} m at {at:?} {kind}, over ground at {ground_m} m, not from {from_m} to {to_m} m (reached at {lowest_m} to {highest_m} m, its rim {} to {} m)",
                        low[cell], high[cell]
                    ));
                }
            } else if surely_m > f64::NEG_INFINITY {
                astray.push(format!("{at:?} is dry at {ground_m} m, in a course's bed under its surface at {surely_m} m"));
            } else if lowest_m.min(rim_at_least_m(cell)) > ground_m + 1e-3 {
                astray.push(format!("{at:?} is dry at {ground_m} m, where a course's water reaches at {lowest_m} m and its rim lies at {} m", rim_at_least_m(cell)));
            } else {
                in_the_band.1 += usize::from(most_m > f64::NEG_INFINITY);
            }
        }
        println!("{} wet cells and {} dry ones may or may not lie in a course's bed", in_the_band.0, in_the_band.1);
        assert!(wet_cells > 0, "no water on the layer");
        assert!(astray.is_empty(), "{} cells, {wet_cells} of them wet: {:#?}", astray.len(), &astray[..astray.len().min(5)]);

        // Some cell within 1 m of every course point holds water, but at a spring's first metres.
        let mut dry_points = Vec::new();
        for (index, course) in world.water.rivers.iter().enumerate() {
            for (k, point) in course.points.iter().enumerate().filter(|(_, point)| point.depth_m >= 0.05) {
                let at = (f64::from(point.x_m), f64::from(point.z_m));
                if !cells_within(size, at, 1.0).any(|cell| layer.data[cell] > 0.0) {
                    dry_points.push(format!("course {index} point {k} at {at:?}, {} m deep", point.depth_m));
                }
            }
        }
        assert!(dry_points.is_empty(), "{} course points are dry: {:#?}", dry_points.len(), &dry_points[..dry_points.len().min(10)]);
    }

    /// A drawing 1024 m across in `country`, with no picture, a lake 61.44 m in radius at its
    /// centre and three rivers: 12 m wide through (-512, -256), (-204.8, -102.4), (0, 0),
    /// (256, 102.4) and (512, 204.8); 6 m wide through (-153.6, -512), (-51.2, -256) and
    /// (0, -25.6); and 5 m wide through (-256, 512), (-102.4, 256) and (-10.24, 25.6). In hill
    /// country, from seed 1, the third river is carried on out of the lake as a second course
    /// flowing out of it, a few points long, that ends on the first point of the first river's
    /// course out of the lake, level at the lake's level.
    fn a_lake_with_three_rivers(country: Country) -> Drawing {
        let river = |points: &[(f64, f64)], width_m: f32| River { points: points.to_vec(), width_m };
        Drawing {
            size_m: 1024,
            high_ground: None,
            rivers: vec![
                river(&[(-512.0, -256.0), (-204.8, -102.4), (0.0, 0.0), (256.0, 102.4), (512.0, 204.8)], 12.0),
                river(&[(-153.6, -512.0), (-51.2, -256.0), (0.0, -25.6)], 6.0),
                river(&[(-256.0, 512.0), (-102.4, 256.0), (-10.24, 25.6)], 5.0),
            ],
            lakes: vec![Lake { x_m: 0.0, z_m: 0.0, radius_m: 61.44 }],
            country,
        }
    }

    /// A second course out of a lake floods no bank of the first: in `a_lake_with_three_rivers`
    /// (hills, seed 1), where two courses flow out of the lake, within 1.5 radii of its centre no
    /// water cell within the banks' reach of either, where its surface at its line's nearest point
    /// lies more than 0.05 m below the lake's level, holds water more than 0.05 m above that
    /// surface; and nowhere do two water cells beside each other step more than their ground does
    /// plus 0.3 m (`held::steps_beyond_the_ground`).
    #[test]
    fn a_second_course_out_of_a_lake_floods_no_bank_of_the_first() {
        let drawing = a_lake_with_three_rivers(Country::hills());
        let world = drawn(1, &drawing);
        let (layer, size) = (&world.layers[WATER_LAYER], world.heights.width);
        let [lake] = &world.water.lakes[..] else { panic!("one lake, not {:?}", world.water.lakes) };
        let outflows: Vec<(usize, &RiverCourse)> = world.water.rivers.iter().enumerate().filter(|(_, course)| course.from_lake == Some(0)).collect();
        assert!(outflows.len() >= 2, "{} courses flow out of the lake", outflows.len());
        let mut above = Vec::new();
        for cell in cells_within(size, (0.0, 0.0), 1.5 * 61.44).filter(|&cell| layer.data[cell] > 0.0) {
            let water_m = world.heights.data[cell] + layer.data[cell];
            for &(index, course) in &outflows {
                let (apart_m, half_width_m, surface_m) = nearest_on_course(course, at_cell(size, cell));
                if apart_m - half_width_m <= BANK_REACH_M && surface_m < lake.level_m - 0.05 && water_m > surface_m + 0.05 {
                    above.push(format!("water at {water_m} m at {:?}, beside course {index}, whose surface there is {surface_m} m", at_cell(size, cell)));
                }
            }
        }
        assert!(above.is_empty(), "{} water cells stand above an outflow beside them (level {} m): {:#?}", above.len(), lake.level_m, &above[..above.len().min(5)]);
        let steps = crate::held::steps_beyond_the_ground(&world);
        assert!(steps.is_empty(), "{} steps between water cells beyond their ground's, the most {:?}", steps.len(), &steps[..steps.len().min(5)]);
    }

    /// What the next stage draws as the water's surface: in each of the nine example worlds,
    /// between two water cells beside each other across a side, the surfaces differ by no more than
    /// their ground does plus 0.3 m (`held::steps_beyond_the_ground`). One kind of step
    /// remains, counted and printed for each world and bounded over the nine: at a mouth, both cells
    /// within the banks' reach (no more than `BANK_REACH_M` beyond half its width, at least
    /// `HALF_WIDTH_MIN_M`, at its line's nearest point, and 1e-3 m) of a course that ends on another
    /// and of that other, where the tributary comes down the river's cut bank and its water meets
    /// the river's beside it, as a channel worked out later follows the bank of the one it ends on
    /// (`profiles`). Over the nine worlds no more than `MOUTH_STEPS_MOST` pairs, none more than
    /// `MOUTH_STEP_MOST_M` beyond their ground.
    #[test]
    fn no_water_steps_beyond_its_ground_in_the_example_worlds() {
        /// The most pairs of water cells a mouth may step beyond their ground over the nine worlds,
        /// and the most one pair may step beyond it, in metres, each set as they were first set
        /// from their measurement (4 pairs and 0.408 m, at (-958, 382) beside (-957, 382), gave 6
        /// and 0.45 m): the count half again, rounded up, and the step the next 0.05 m above. Now 1
        /// pair, 0.314 m, in mountains seed 3. There were 5 there, up to 0.497 m at (112, -41)
        /// beside (113, -41), while a channel's banks read a cell under an earlier channel's water
        /// as bare ground: brook 9 springs just beyond the waterline of course 2, both ending on
        /// river 1, and its banks held it 0.1 m below course 2's bed, 0.51 m under course 2's water a
        /// metre away.
        const MOUTH_STEPS_MOST: usize = 2;
        const MOUTH_STEP_MOST_M: f64 = 0.35;
        // Over the nine worlds, the pairs at a mouth, and those that step more than
        // `MOUTH_STEP_MOST_M`: how much, in which world and where.
        let mouths = std::sync::Mutex::new((0_usize, Vec::<(f64, String, (f64, f64), (f64, f64))>::new()));
        let failures = example_world_failures(|name, _, world| {
            let (mut at_mouths, mut most_m, mut failed) = (0, 0.0_f64, Vec::new());
            let mut above = Vec::new();
            for (beyond_m, a, b) in crate::held::steps_beyond_the_ground(world) {
                if crate::held::at_a_mouth(world, a, b) {
                    at_mouths += 1;
                    most_m = most_m.max(beyond_m);
                    if beyond_m > MOUTH_STEP_MOST_M {
                        above.push((beyond_m, name.to_string(), a, b));
                    }
                } else {
                    failed.push(format!("{name}: water at {a:?} and {b:?} steps {beyond_m:.3} m beyond its ground"));
                }
            }
            println!("{name}: {at_mouths} pairs of water cells at a mouth step beyond their ground, up to {most_m:.3} m");
            let mut all = mouths.lock().expect("the mouths' count");
            all.0 += at_mouths;
            all.1.extend(above);
            failed
        });
        let (count, above) = mouths.into_inner().expect("the mouths' count");
        println!("over the nine worlds: {count} pairs at a mouth, {} of them more than {MOUTH_STEP_MOST_M} m beyond their ground: {above:?}", above.len());
        assert!(failures.is_empty(), "{} steps beyond the ground: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
        assert!(count <= MOUTH_STEPS_MOST, "{count} pairs of water cells at a mouth step beyond their ground, more than {MOUTH_STEPS_MOST}");
        assert!(above.is_empty(), "pairs at a mouth step more than {MOUTH_STEP_MOST_M} m beyond their ground: {above:#?}");
    }

    /// The example's picture at 1024 m, in hill country, with one river 4 m wide drawn from
    /// (409.6, 102.4) through (204.8, 51.2) to the centre and no lake. From seed 2 the river's head
    /// lies on a shoulder: the ground behind its first point falls away, up to 1.2 m below the
    /// water there within its banks' reach.
    fn a_head_on_a_shoulder() -> Drawing {
        let river = River { points: vec![(409.6, 102.4), (204.8, 51.2), (0.0, 0.0)], width_m: 4.0 };
        Drawing { rivers: vec![river], lakes: Vec::new(), ..Drawing::example_valley(1024) }
    }

    /// A spring whose head lies on a shoulder holds none of its water behind its head: in
    /// `a_head_on_a_shoulder`, from seed 2, no cell straight beyond the river's first point
    /// (`behind_the_head`, within its banks' reach and 2 m) holds water, though more than ten of
    /// them lie more than 0.1 m below its surface there, and none of them that lies beside water
    /// lies more than 0.1 m (and 1e-4 m) below that water's surface.
    #[test]
    fn no_water_stands_behind_a_springs_head() {
        let drawing = a_head_on_a_shoulder();
        let world = drawn(2, &drawing);
        let (layer, heights, size) = (&world.layers[WATER_LAYER], &world.heights.data, world.heights.width);
        let index = world.water.rivers.iter().position(|course| course.drawn == Some(0)).expect("the river's course");
        let course = &world.water.rivers[index];
        let head = course.points[0];
        assert!(on_dry_land(course, size), "the river begins at {head:?}, not on dry land");
        assert!((f64::from(head.x_m) - 409.6).hypot(f64::from(head.z_m) - 102.4) <= 4.0, "the river begins at {head:?}");
        let behind = behind_the_head(size, &course_lines(&world)[index], BANK_REACH_M + 2.0, 1e-3);
        let low = behind.surely.iter().filter(|&&cell| heights[cell] < head.surface_m - 0.1).count();
        assert!(low >= 10, "only {low} cells behind the head lie more than 0.1 m below its surface {} m", head.surface_m);
        let wet: Vec<(f64, f64)> = behind.surely.iter().filter(|&&cell| layer.data[cell] > 0.0).map(|&cell| at_cell(size, cell)).collect();
        assert!(wet.is_empty(), "{} cells behind the head hold water, such as {:?}", wet.len(), &wet[..wet.len().min(5)]);
        let mut below: Vec<String> = Vec::new();
        for &cell in &behind.surely {
            for water in beside(cell, size).filter(|&water| layer.data[water] > 0.0) {
                let below_m = heights[water] + layer.data[water] - heights[cell];
                if below_m > 0.1 + 1e-4 {
                    below.push(format!("{:?}, {below_m} m below the water beside it at {:?}", at_cell(size, cell), at_cell(size, water)));
                }
            }
        }
        assert!(below.is_empty(), "{} dry cells behind the head lie below the water beside them: {:#?}", below.len(), &below[..below.len().min(5)]);
    }

    /// The spec's rule that water lies only where it is reached: in each of the nine example
    /// worlds every water cell lies in a course's bed (no farther from its line than half its
    /// width, at least `HALF_WIDTH_MIN_M`, at its line's nearest point, to 1e-3 m), or is a lake's
    /// (`lake_water`), or is joined to one of those through water cells beside each other.
    #[test]
    fn water_lies_only_where_it_is_reached() {
        let failures = example_world_failures(|name, drawing, world| {
            let (layer, size) = (&world.layers[WATER_LAYER], world.heights.width);
            let wet = |cell: usize| layer.data[cell] > 0.0;
            let mut reached: Vec<bool> = lake_water(world, drawing).iter().map(Option::is_some).collect();
            for near in near_lines(size, &course_lines(world), 0.0, 1e-3) {
                for seen in near.iter().filter(|seen| seen.distance_m <= seen.half_width_m.1 + 1e-3 && wet(seen.cell)) {
                    reached[seen.cell] = true;
                }
            }
            let mut waiting: Vec<usize> = (0..size * size).filter(|&cell| reached[cell]).collect();
            while let Some(cell) = waiting.pop() {
                for next in beside(cell, size) {
                    if wet(next) && !reached[next] {
                        reached[next] = true;
                        waiting.push(next);
                    }
                }
            }
            let at = |cell: usize| ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
            let astray: Vec<(f64, f64)> = (0..size * size).filter(|&cell| wet(cell) && !reached[cell]).map(at).collect();
            match astray.len() {
                0 => Vec::new(),
                count => vec![format!("{name}: {count} water cells are joined to no course's bed and no lake, such as {:?}", &astray[..count.min(5)])],
            }
        });
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// No sheet of water lies beside the head-join: in the trial drawing `headjoin` at 1,024 m in
    /// hill country, from seed 3, where the brook raised to its river's level carried its water
    /// over the slope beside it in a sheet of 463 cells, and later stood 2.1 m above its banks
    /// closing in 57 cells of the slope above its bed at that level, no cell from x = -150 to -114
    /// and z = -19 to 27 holds water more than 2 m beyond a course's waterline (half its width at its
    /// line's nearest point, at least `HALF_WIDTH_MIN_M`): the river comes down to the brook.
    #[test]
    fn no_sheet_of_water_lies_beside_the_head_join() {
        let drawing = crate::trial::trial_drawing("headjoin", 1024, Country::hills()).expect("a trial drawing");
        let world = drawn(3, &drawing);
        let (layer, size) = (&world.layers[WATER_LAYER], world.heights.width);
        let half = (size / 2) as i64;
        let mut astray = Vec::new();
        for z in -19..=27_i64 {
            for x in -150..=-114_i64 {
                let cell = ((z + half) * size as i64 + x + half) as usize;
                if layer.data[cell] > 0.0 {
                    let beyond_m = (world.water.rivers.iter())
                        .map(|course| {
                            let (apart_m, half_width_m, _) = nearest_on_course(course, (x as f64, z as f64));
                            apart_m - half_width_m.max(HALF_WIDTH_MIN_M)
                        })
                        .fold(f64::INFINITY, f64::min);
                    if beyond_m > 2.0 {
                        astray.push(format!("({x}, {z}), {beyond_m:.1} m beyond the nearest waterline, {} m deep", layer.data[cell]));
                    }
                }
            }
        }
        assert!(astray.is_empty(), "{} water cells beside the head-join: {:#?}", astray.len(), &astray[..astray.len().min(10)]);
    }

    /// At the head-join the river comes down to the brook. In the trial drawing `headjoin` at 1,024 m
    /// in hill country, from seed 3, course 2 (drawn river 3) ends on river 0 (drawn river 1) at its
    /// head, where its banks hold it 2.1 m below the river's own surface (the prototype measured
    /// 2.09 m). Its last points stand at their own surface: every cell of the foot of its banks (no
    /// more than `BANK_FOOT_M` beyond its waterline, half its width at its line's nearest point and
    /// at least `HALF_WIDTH_MIN_M`, not straight beyond either end) whose nearest point of its line
    /// lies within 24 m before its entry point into river 0's bed, outside that bed and outside river
    /// 0's own bank (no more than `BANK_REACH_M` beyond its waterline and at or above its surface at
    /// its nearest point), stands no more than `ARRIVAL_ALLOWANCE_M` below course 2's surface there;
    /// when the brook was raised to the river it stood up to 2.1 m above them. River 0 keeps its
    /// head's ground: its first point's surface lies within 0.1 m of the ground there, where the
    /// first try at this rule dug a notch 2.8 m deep. And the river's lowering is said, and nothing
    /// else of the junctions.
    #[test]
    fn at_the_head_join_the_river_comes_down_to_the_brook() {
        let drawing = crate::trial::trial_drawing("headjoin", 1024, Country::hills()).expect("a trial drawing");
        let world = drawn(3, &drawing);
        let size = world.heights.width;
        let (brook, river) = (&world.water.rivers[2], &world.water.rivers[0]);
        assert_eq!((brook.drawn, brook.end, river.drawn), (Some(2), CourseEnd::River(0), Some(0)), "the courses at the head-join");
        // Each point of the brook's distance along it, and its entry point into the river's bed.
        let along = along_course_m(brook);
        let inside = |point: &CoursePoint| {
            let (apart_m, half_width_m, _) = nearest_on_course(river, (f64::from(point.x_m), f64::from(point.z_m)));
            apart_m <= half_width_m.max(HALF_WIDTH_MIN_M)
        };
        let last = brook.points.len() - 1;
        let entry = (0..last).rev().take_while(|&k| inside(&brook.points[k])).last().unwrap_or(last);
        let near = brook.points[entry];
        let (mut checked, mut low) = (0, Vec::new());
        for cell in cells_within(size, (f64::from(near.x_m), f64::from(near.z_m)), 30.0) {
            let at = at_cell(size, cell);
            // The cell's nearest place on the brook's line: how far it lies, the half width and the
            // surface there, how far along it lies, and whether it lies straight beyond an end.
            let mut nearest = (f64::INFINITY, 0.0, 0.0, 0.0, false);
            for k in 1..brook.points.len() {
                let (a, b) = (brook.points[k - 1], brook.points[k]);
                let (ax, az, dx, dz) = (f64::from(a.x_m), f64::from(a.z_m), f64::from(b.x_m - a.x_m), f64::from(b.z_m - a.z_m));
                let squared = dx * dx + dz * dz;
                let share = ((at.0 - ax) * dx + (at.1 - az) * dz) / squared;
                let t = share.clamp(0.0, 1.0);
                let apart_m = (ax + dx * t - at.0).hypot(az + dz * t - at.1);
                if apart_m < nearest.0 {
                    let between = |u: f32, v: f32| f64::from(u) + (f64::from(v) - f64::from(u)) * t;
                    let beyond = (k == 1 && share < 0.0) || (k == last && share > 1.0);
                    nearest = (apart_m, between(a.width_m, b.width_m) / 2.0, between(a.surface_m, b.surface_m), along[k - 1] + t * squared.sqrt(), beyond);
                }
            }
            let (apart_m, half_width_m, surface_m, along_m, beyond) = nearest;
            let half_width_m = half_width_m.max(HALF_WIDTH_MIN_M);
            if beyond || apart_m <= half_width_m || apart_m > half_width_m + BANK_FOOT_M || along_m >= along[entry] || along_m < along[entry] - 24.0 {
                continue;
            }
            let ground_m = f64::from(world.heights.data[cell]);
            let (river_apart_m, river_half_m, river_m) = crate::held::nearest_on_course(river, at);
            let river_half_m = river_half_m.max(HALF_WIDTH_MIN_M);
            if river_apart_m <= river_half_m || (river_apart_m - river_half_m <= BANK_REACH_M && ground_m >= river_m) {
                continue;
            }
            checked += 1;
            if ground_m < surface_m - f64::from(ARRIVAL_ALLOWANCE_M) {
                low.push(format!("{at:?} at {ground_m} m, the brook's surface {surface_m} m"));
            }
        }
        println!("{checked} cells of the brook's foot before its entry point checked");
        assert!(checked >= 20, "only {checked} cells of the brook's foot were checked");
        assert!(low.is_empty(), "{} cells of the brook's foot lie more than 0.3 m below its water: {:#?}", low.len(), &low[..low.len().min(10)]);
        let head = river.points[0];
        let ground_m = ground_at(&world, (head.x_m, head.z_m));
        assert!(on_dry_land(river, size), "the river begins at {head:?}, not on dry land");
        assert!((head.surface_m - ground_m).abs() <= 0.1, "the river's head stands at {} m, the ground there at {ground_m} m", head.surface_m);
        let junctions: Vec<&String> = world.notes.iter().filter(|note| note.contains("joins")).collect();
        let [deeper] = &junctions[..] else { panic!("the notes on the junctions: {junctions:?}") };
        let by_m: f32 = deeper.strip_prefix("River 1 was cut ").and_then(|rest| rest.strip_suffix(" m deeper where river 3 joins it.")).and_then(|by| by.parse().ok()).unwrap_or(f32::NAN);
        assert!((1.6..=2.2).contains(&by_m), "{deeper:?}");
    }

    /// `before_the_water`, the world made under the default config but for the water rules `water`.
    fn before_the_water_under(seed: u64, drawing: &Drawing, water: WaterRules) -> (Grid<f32>, Grown) {
        let config = GenConfig { water, ..GenConfig::default() };
        let grown = grow(drawing, &config.water, seed, &AtomicBool::new(false)).expect("the drawing grows");
        let mut heights = at_one_metre(&grown, drawing.size_m);
        let analysis = analyse(&heights, &config.rules);
        add_world_detail(&mut heights, &analysis, seed, &config, Some(&WaterStep { grown: &grown, drawing }));
        (heights, grown)
    }

    /// The smallest brooks in mountains (Review Focus 3): the example at 1,024 m in mountain
    /// country with `brook_start_m2` 1,000, from seed 1 (the counting program's
    /// `brooks1000-mountains-s1`), hundreds of junctions on a steep slope, is made; no course's
    /// surface rises downstream; and the water step's passes end before they run out: fewer than
    /// `PASSES_MOST`, so the last asked for no new cap (1 measured: every tributary short there is
    /// stopped by the lake, as in the prototype).
    #[test]
    fn the_smallest_brooks_in_mountains_end_within_the_passes() {
        let water = WaterRules { brook_start_m2: 1_000.0, ..WaterRules::default() };
        let drawing = Drawing { country: Country::mountains(), ..Drawing::example_valley(1024) };
        let started = std::time::Instant::now();
        let world = generate_drawn(1, &drawing, &GenConfig { water, ..GenConfig::default() }, &AtomicBool::new(false)).expect("the drawing gives a world");
        println!("made in {:.0} ms, {} courses", started.elapsed().as_secs_f64() * 1000.0, world.water.rivers.len());
        assert!(world.water.rivers.len() > 100, "only {} courses", world.water.rivers.len());
        for (index, course) in world.water.rivers.iter().enumerate() {
            let rising = course.points.windows(2).position(|pair| pair[1].surface_m > pair[0].surface_m);
            assert!(rising.is_none(), "course {index} rises at point {:?}", rising.map(|k| k + 1));
        }
        let (heights, grown) = before_the_water_under(1, &drawing, water);
        let worked = crate::water::tests::junction_water(&heights, &grown, &drawing, &water, 1, &AtomicBool::new(false)).expect("no flag is set");
        println!("{} passes; notes {:?}", worked.passes, worked.notes);
        assert!(worked.passes < PASSES_MOST, "{} passes, the last of them still asking for a cap", worked.passes);
    }

    /// Where the passes run out (Review Focus 3): in the trial drawing `parallel` at 1,024 m in
    /// mountain country from seed 3 (`trial_drawing`, the counting program's
    /// `parallel-mountains-s3-1024`), the water step's junctions take all `PASSES_MOST` passes and
    /// the last still asks for a new cap, the river's lowering moving its tributary's foot with it;
    /// no course's surface rises downstream; and the tributary the last pass left short stands no
    /// more than `LEFT_SHORT_MOST_M` above what its banks hold where it joins, as `profiles` says:
    /// by what the last pass left short, past `ARRIVAL_ALLOWANCE_M`.
    #[test]
    fn where_the_passes_run_out_a_tributary_stands_above_its_banks_by_what_the_last_left_short() {
        /// The most the tributary left short may stand above its banks, in metres: course 4 is the
        /// one, 0.300 m above them after the third pass (0.30043 m measured, the ledger's 0.30 m),
        /// just past the allowance, so it would ask a fourth pass for a cap, and the passes have run
        /// out.
        const LEFT_SHORT_MOST_M: f32 = 0.31;
        let drawing = crate::trial::trial_drawing("parallel", 1024, Country::mountains()).expect("a trial drawing");
        let water = WaterRules::default();
        let (heights, grown) = before_the_water_under(3, &drawing, water);
        let worked = crate::water::tests::junction_water(&heights, &grown, &drawing, &water, 3, &AtomicBool::new(false)).expect("no flag is set");
        println!("{} passes; left short {:?}; notes {:?}", worked.passes, worked.left_short, worked.notes);
        assert_eq!(worked.passes, PASSES_MOST, "passes");
        for (index, profile) in worked.profiles.iter().enumerate() {
            let rising = profile.surface_m.windows(2).position(|pair| pair[1] > pair[0]);
            assert!(rising.is_none(), "channel {index} rises at point {:?}", rising.map(|k| k + 1));
        }
        assert!(!worked.left_short.is_empty(), "the last pass asked for no new cap");
        for &(index, short_m) in &worked.left_short {
            assert!(short_m > ARRIVAL_ALLOWANCE_M && short_m <= LEFT_SHORT_MOST_M, "channel {index} stands {short_m} m above its banks");
        }
    }

    /// A generation cancelled between the junctions' passes returns `Cancelled` (Review Focus 5): on
    /// the ground of `headjoin` at 1,024 m in hill country from seed 3, where the river comes down
    /// to the brook, the water step works its water out in more than one pass, and with the flag
    /// set before it, the water's surfaces end with `Cancelled` after the first.
    #[test]
    fn a_generation_cancelled_between_the_passes_returns_cancelled() {
        let drawing = crate::trial::trial_drawing("headjoin", 1024, Country::hills()).expect("a trial drawing");
        let water = WaterRules::default();
        let (heights, grown) = before_the_water_under(3, &drawing, water);
        let worked = crate::water::tests::junction_water(&heights, &grown, &drawing, &water, 3, &AtomicBool::new(false)).expect("no flag is set");
        assert!(worked.passes >= 2, "{} passes", worked.passes);
        let cancelled = crate::water::tests::junction_water(&heights, &grown, &drawing, &water, 3, &AtomicBool::new(true));
        assert_eq!(cancelled.err(), Some(LandformError::Cancelled));
    }

    /// The ordinary mouths are untouched: on the 27 worlds of the three trial drawings `headjoin`,
    /// `twoout` and `wide` (as `trial_drawing` gives them, not as their folders hold them) at
    /// 1,024 m in each kind of country from seeds 1, 2 and 3, the only notes on the junctions are
    /// those of the three head-joins in seed 3, each saying river 1 was cut from 1.6 to 2.2 m
    /// deeper where river 3 joins it (the prototype measured 1.66, 2.09 and 1.82 m in gentle, hill
    /// and mountain country), word for word as before the notes were gathered (1.7, 2.1 and 1.8 m);
    /// no other river comes down more than `DEEPER_NOTE_M`, 1 m, and no step is left past it.
    #[test]
    fn on_the_trial_drawings_only_the_head_joins_cut_a_river_deeper() {
        let cases: Vec<(String, Drawing, u64)> = ["headjoin", "twoout", "wide"]
            .iter()
            .flat_map(|&kind| COUNTRY_NAMES.iter().flat_map(move |&country| [1, 2, 3].map(move |seed| (kind, country, seed))))
            .map(|(kind, country, seed)| {
                let drawing = crate::trial::trial_drawing(kind, 1024, Country::named(country).expect("a known country")).expect("a trial drawing");
                (format!("{kind}-{country}-s{seed}"), drawing, seed)
            })
            .collect();
        let notes: Vec<(String, Vec<String>)> = std::thread::scope(|scope| {
            let making: Vec<_> = (cases.iter())
                .map(|(name, drawing, seed)| scope.spawn(move || (name.clone(), drawn(*seed, drawing).notes.into_iter().filter(|note| note.contains("joins")).collect())))
                .collect();
            making.into_iter().map(|made| made.join().expect("a trial world is made")).collect()
        });
        let said: Vec<(String, String)> = notes.into_iter().flat_map(|(name, notes)| notes.into_iter().map(move |note| (name.clone(), note))).collect();
        println!("{said:#?}");
        let names: Vec<&str> = said.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["headjoin-gentle-s3", "headjoin-hills-s3", "headjoin-mountains-s3"], "{said:#?}");
        for (name, note) in &said {
            let by_m: f32 = note.strip_prefix("River 1 was cut ").and_then(|rest| rest.strip_suffix(" m deeper where river 3 joins it.")).and_then(|by| by.parse().ok()).unwrap_or(f32::NAN);
            assert!((1.6..=2.2).contains(&by_m), "{name}: {note:?}");
        }
        let notes: Vec<&str> = said.iter().map(|(_, note)| note.as_str()).collect();
        let before = ["River 1 was cut 1.7 m deeper where river 3 joins it.", "River 1 was cut 2.1 m deeper where river 3 joins it.", "River 1 was cut 1.8 m deeper where river 3 joins it."];
        assert_eq!(notes, before, "the head-joins' notes, as before they were gathered");
    }

    /// The ground of the world `generate_drawn` makes from `drawing` and `seed` under the default
    /// config, as the water finds it, and the land grown: the grown land at 1 m with its detail, the
    /// gullies faded out near its lakes (`add_world_detail`), as `finish_world` hands it on.
    fn before_the_water(seed: u64, drawing: &Drawing) -> (Grid<f32>, Grown) {
        before_the_water_under(seed, drawing, WaterRules::default())
    }

    /// The spread's bound takes no water off a bed or a lake: in the counting program's nine
    /// example worlds (seeds 11, 23 and 42), the water step made again on the world's ground before
    /// its water (`before_the_water`, `bounded_water`) gives the world's ground and `water` layer
    /// bit for bit, and of the cells that held water before the spread was bounded, every one with
    /// a kept level (a bed's or a lake's) holds water after it, standing no lower than that level,
    /// and every cell of a lake stands at its level. What the pass took is printed for each world:
    /// the cells beyond the beds it dried, those it lowered, and the kept cells it brought down to
    /// their kept level.
    #[test]
    fn the_spreads_bound_takes_no_water_off_a_bed_or_a_lake_in_the_example_worlds() {
        let rules = WaterRules::default();
        let failures = example_failures_from(&COUNTED_SEEDS, rules, |name, drawing, world| {
            let (heights, grown) = before_the_water(world.meta.seed, drawing);
            let made = bounded_water(&heights, &grown, drawing, &rules, world.meta.seed);
            let base_m = world.meta.base_elevation_m;
            let same_ground = made.heights.data.iter().zip(&world.heights.data).all(|(&cut, &shifted)| (cut - base_m).to_bits() == shifted.to_bits());
            let same_water = made.after.data.iter().zip(&world.layers[WATER_LAYER].data).all(|(made, held)| made.to_bits() == held.to_bits());
            if !(same_ground && same_water) {
                return vec![format!("{name}: the water step made again gives other ground ({}) or other water ({})", !same_ground, !same_water)];
            }
            let size = made.heights.width;
            let (mut failures, mut kept, mut dried, mut lowered, mut brought_down) = (Vec::new(), 0, 0, 0, 0);
            for cell in (0..size * size).filter(|&cell| made.before.data[cell] > 0.0) {
                let (ground_m, after_m, kept_m) = (made.heights.data[cell], made.after.data[cell], made.kept_level[cell]);
                if kept_m > f32::NEG_INFINITY {
                    kept += 1;
                    if after_m <= 0.0 || ground_m + after_m < kept_m - 1e-4 {
                        failures.push(format!("{name}: the water at {:?}, kept at {kept_m} m, stands {after_m} m deep over {ground_m} m", at_cell(size, cell)));
                    }
                    brought_down += usize::from(after_m < made.before.data[cell]);
                } else if after_m <= 0.0 {
                    dried += 1;
                } else {
                    lowered += usize::from(after_m < made.before.data[cell]);
                }
            }
            for lake in &made.lakes {
                let below = lake.cells.iter().filter(|&&cell| made.heights.data[cell] + made.after.data[cell] < lake.level_m - 1e-4).count();
                if below > 0 {
                    failures.push(format!("{name}: {below} cells of lake {} stand below its level", lake.drawn + 1));
                }
            }
            println!("{name}: of the water before the spread's bound, {kept} kept cells, {brought_down} of them brought down to their level; {dried} cells beyond the beds dried, {lowered} lowered");
            failures
        });
        assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
    }

    /// How many dry cells the measure counts where a course leaves the map
    /// (`dry_cells_but_where_a_course_leaves_the_map`), the most one of them lies below the water it
    /// is read against, in metres, and where.
    #[derive(Debug, Clone, Default)]
    struct DryCells {
        count: usize,
        most_m: f64,
        place: String,
    }

    impl DryCells {
        /// Counts one more cell, `below_m` below the water it is read against, at `place`.
        fn add(&mut self, below_m: f64, place: String) {
            self.count += 1;
            if below_m > self.most_m {
                (self.most_m, self.place) = (below_m, place);
            }
        }

        /// Counts `other`'s cells too, those of the world named `name`.
        fn take(&mut self, other: &DryCells, name: &str) {
            self.count += other.count;
            if other.most_m > self.most_m {
                (self.most_m, self.place) = (other.most_m, format!("{name}: {}", other.place));
            }
        }
    }

    /// The dry cells the measure counts in `world`, named `name` (`held::dry_cells`: a dry cell more
    /// than `UNHELD_LIMIT_M` below the lowest water among its eight neighbours, a course's water read
    /// no higher than its surface at the cell's own nearest point of its line): those where a course
    /// leaves the map, a cell within `BORDER_START_M` of the world's border and within the banks'
    /// reach (no more than `BANK_REACH_M` beyond half its width, at least `HALF_WIDTH_MIN_M`, at its
    /// line's nearest point) of a course that ends at the edge, counted and printed; and a failure
    /// for each other.
    fn dry_cells_but_where_a_course_leaves_the_map(name: &str, world: &World) -> (DryCells, Vec<String>) {
        let half = (world.heights.width / 2) as f64;
        let leaves_the_map = |at: (f64, f64)| {
            let by_the_border = (half - at.0.abs()).min(half - at.1.abs()) <= BORDER_START_M;
            by_the_border
                && (world.water.rivers.iter()).any(|course| {
                    let (apart_m, half_width_m, _) = nearest_on_course(course, at);
                    course.end == CourseEnd::Edge && apart_m - half_width_m.max(HALF_WIDTH_MIN_M) <= BANK_REACH_M
                })
        };
        let (mut here, mut failed) = (DryCells::default(), Vec::new());
        for dry in crate::held::dry_cells(world) {
            if leaves_the_map(dry.at) {
                here.add(dry.below_m, format!("{:?}", dry.at));
            } else {
                let water = if dry.at_lake { "a lake's" } else { "a course's" };
                failed.push(format!("{name}: a dry cell at {:?}, {:.3} m below {water} water beside it", dry.at, dry.below_m));
            }
        }
        println!("{name}: {} dry cells where a course leaves the map, up to {:.3} m below the water beside them", here.count, here.most_m);
        (here, failed)
    }

    /// No dry ground lies below the water beside it, as the measure counts it: in the counting
    /// program's nine example worlds (seeds 11, 23 and 42) it counts no dry cell but where a course
    /// leaves the map (`dry_cells_but_where_a_course_leaves_the_map`). Those are counted, printed and
    /// bounded over the nine worlds: no more than `LEAVING_CELLS_MOST`, in no more than
    /// `LEAVING_WORLDS_MOST` worlds, none more than `LEAVING_BELOW_MOST_M` below the water it is read
    /// against.
    #[test]
    fn no_dry_ground_lies_below_the_water_beside_it() {
        /// The most dry cells where a course leaves the map over the nine worlds, the most worlds
        /// they lie in, and the most one may lie below its water, in metres (4 measured in 2
        /// worlds, the deepest 0.584 m, at (1023, -482) in mountain country from seed 42).
        const LEAVING_CELLS_MOST: usize = 4;
        const LEAVING_WORLDS_MOST: usize = 2;
        const LEAVING_BELOW_MOST_M: f64 = 0.6;
        let leaving = std::sync::Mutex::new((DryCells::default(), 0_usize));
        let failures = example_failures_from(&COUNTED_SEEDS, WaterRules::default(), |name, _, world| {
            let (here, failed) = dry_cells_but_where_a_course_leaves_the_map(name, world);
            let mut all = leaving.lock().expect("the count where a course leaves the map");
            all.1 += usize::from(here.count > 0);
            all.0.take(&here, name);
            failed
        });
        let (cells, worlds) = leaving.into_inner().expect("the count where a course leaves the map");
        println!("over the nine worlds: {} dry cells in {worlds} worlds where a course leaves the map, up to {:.3} m below the water beside them at {}", cells.count, cells.most_m, cells.place);
        assert!(failures.is_empty(), "{failures:#?}");
        assert!(cells.count <= LEAVING_CELLS_MOST, "{} dry cells where a course leaves the map, more than {LEAVING_CELLS_MOST}", cells.count);
        assert!(worlds <= LEAVING_WORLDS_MOST, "dry cells where a course leaves the map in {worlds} worlds, more than {LEAVING_WORLDS_MOST}");
        assert!(cells.most_m <= LEAVING_BELOW_MOST_M, "a dry cell where a course leaves the map lies {} m below its water, at {}", cells.most_m, cells.place);
    }

    /// `drawing` as a drawing folder holds it: each value of its picture of high ground rounded to 8
    /// bits as `write-example` writes it (`round(value * 255)`, clamped to 0 to 255) and read back as
    /// the command line reads a folder, that byte over 255 (`drawing_file.rs`). The rest of a
    /// folder, its size, rivers, lakes and country, reads back as the drawing holds it, so this is
    /// the world `landform generate --drawing` grows from the folder, bit for bit (the head-join
    /// from seed 3 hashed so). The game's drawings share the folders' picture.
    fn as_the_folders_hold_it(mut drawing: Drawing) -> Drawing {
        if let Some(picture) = &mut drawing.high_ground {
            for value in &mut picture.data {
                *value = f32::from((*value * 255.0).round().clamp(0.0, 255.0) as u8) / 255.0;
            }
        }
        drawing
    }

    /// The three things the measure (`unheld`) counts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Counted {
        /// Dry cells below the water abreast of them (`held::dry_cells`), by how far below it.
        DryCells,
        /// Lake cells over lower water beside them (`held::lake_cells_over_water`), by how far over.
        LakeCellsOverWater,
        /// Pairs of water cells that step beyond their ground, not at a mouth
        /// (`held::steps_beyond_the_ground`, `held::at_a_mouth`), by how far beyond.
        StepsOutsideAMouth,
    }

    /// One count the measure leaves on a world of the design's "Done means": the world, named as
    /// the counting program names it, what is counted, the cells counted (world `(x, z)`, in metres,
    /// in index order; for a step, its first cell), so as many as they are, the most one of them
    /// lies below, stands over or steps beyond, in metres (the measurement rounded up to the
    /// centimetre), and why.
    struct Left {
        world: &'static str,
        counted: Counted,
        cells: &'static [(f64, f64)],
        most_m: f64,
        why: &'static str,
    }

    /// Every count the measure leaves on the counting program's nine example worlds and on the 27
    /// worlds of the three trial folders as the folders hold them (`as_the_folders_hold_it`), one by
    /// one, measured at `87fc4d6` (after the junctions' rule); every other count on those worlds is
    /// 0.
    const LEFT: [Left; 4] = [
        Left {
            world: "example-mountains-s23-2048",
            counted: Counted::DryCells,
            // 0.344 and 0.459 m below the water beside them.
            cells: &[(-183.0, 1022.0), (-183.0, 1023.0)],
            most_m: 0.46,
            why: "a brook leaves the map by the south border: its bed's own water at its last point (45.81 m, 2.7 m from the border) stands beside two cells of the border's last rows whose ground falls on below it; the border is the way out, so no water lies on them",
        },
        Left {
            world: "example-mountains-s42-2048",
            counted: Counted::DryCells,
            // 0.312 and 0.584 m below the water beside them.
            cells: &[(1023.0, -483.0), (1023.0, -482.0)],
            most_m: 0.59,
            why: "a brook leaves the map by the east border: its bed's own water at its last point (39.75 m) stands beside two cells of the border's last column whose ground falls on below it",
        },
        Left {
            world: "wide-hills-s3-1024",
            counted: Counted::LakeCellsOverWater,
            // Each 1.553 m over the outflow's water at (-119, 0).
            cells: &[(-120.0, -1.0), (-120.0, 0.0), (-120.0, 1.0)],
            most_m: 1.56,
            why: "river 1, carried 381 m off its drawn line, passes the lake's east shore 1.70 m below its level (7.13 m); the lake's outflow, a brook, enters river 1's bed at its second point, 4 m from its first, and what a course ends on is met after the lake's hold, so it leaves the lake at river 1's surface and the grade, 5.62 m, and the lake's three cells beside its first point stand over its water",
        },
        Left {
            world: "wide-gentle-s3-1024",
            counted: Counted::LakeCellsOverWater,
            // 0.375 m over the outflow's water at (-115, 0).
            cells: &[(-116.0, 0.0)],
            most_m: 0.38,
            why: "river 1 passes the lake's east shore 1.49 m below its level; the lake's outflow, a brook, holds the level to its point at (-116, 0) and drops into river 1 in its next 4 m, so the lake's last cell there stands over the outflow's falling water a metre on",
        },
    ];

    /// What the measure counts in `world`, named `name`: for each of the three things counted, how
    /// many and the most, in metres; and failures where that is not what `LEFT` names for `name`
    /// (0 where it names nothing).
    fn left_failures(name: &str, world: &World) -> Vec<String> {
        let layer = &world.layers[WATER_LAYER];
        // Each thing counted: where each cell lies and how far it lies below, stands over or steps.
        let dry: Vec<((f64, f64), f64)> = crate::held::dry_cells(world).iter().map(|dry| (dry.at, dry.below_m)).collect();
        let over: Vec<((f64, f64), f64)> = crate::held::lake_cells_over_water(world, layer).iter().map(|&(at, over_m)| (at, f64::from(over_m))).collect();
        let steps: Vec<((f64, f64), f64)> =
            (crate::held::steps_beyond_the_ground(world).into_iter()).filter(|&(_, a, b)| !crate::held::at_a_mouth(world, a, b)).map(|(beyond_m, a, _)| (a, beyond_m)).collect();
        let (mut failures, mut said) = (Vec::new(), Vec::new());
        for (counted, mut found) in [(Counted::DryCells, dry), (Counted::LakeCellsOverWater, over), (Counted::StepsOutsideAMouth, steps)] {
            found.sort_by(|a, b| (a.0 .1, a.0 .0).partial_cmp(&(b.0 .1, b.0 .0)).expect("cells at finite places"));
            let most_m = found.iter().map(|&(_, by_m)| by_m).fold(0.0, f64::max);
            let cells: Vec<(f64, f64)> = found.iter().map(|&(at, _)| at).collect();
            let shown: Vec<String> = found.iter().take(10).map(|(at, by_m)| format!("{at:?} {by_m:.3} m")).collect();
            said.push(format!("{counted:?} {} up to {most_m:.3} m {shown:?}", found.len()));
            match LEFT.iter().find(|left| left.world == name && left.counted == counted) {
                Some(left) if cells == left.cells && most_m <= left.most_m => {}
                Some(left) => failures.push(format!("{name}: {counted:?} {} up to {most_m:.3} m at {shown:?}, where {:?} up to {} m are left ({})", found.len(), left.cells, left.most_m, left.why)),
                None if found.is_empty() => {}
                None => failures.push(format!("{name}: {counted:?} {} up to {most_m:.3} m at {shown:?}, where none are left", found.len())),
            }
        }
        println!("{name}: {}", said.join(", "));
        failures
    }

    /// The design's "Done means", as the measure (`unheld`) counts it: on the counting program's
    /// nine example worlds (the example at 2048 m in each kind of country from seeds 11, 23 and 42)
    /// and the 27 worlds of the three trial folders (`headjoin`, `twoout` and `wide` at 1024 m in
    /// each kind of country from seeds 1, 2 and 3, as the folders hold them: the trial drawings with
    /// their picture rounded to 8 bits, `as_the_folders_hold_it`), no dry cell lies below the water
    /// abreast of it, no lake cell stands over lower water and no pair of water cells steps beyond
    /// its ground outside a mouth, but for the counts `LEFT` names one by one, each in exactly the
    /// cells it names and none more than its most.
    #[test]
    fn the_counts_hold_on_the_example_and_the_trial_folders() {
        let named = |country: &str| Country::named(country).expect("a known country");
        let example: Vec<(String, Drawing, u64)> = (COUNTRY_NAMES.iter())
            .flat_map(|&country| COUNTED_SEEDS.map(move |seed| (country, seed)))
            .map(|(country, seed)| (format!("example-{country}-s{seed}-2048"), Drawing { country: named(country), ..Drawing::example_valley(2048) }, seed))
            .collect();
        let folders: Vec<(String, Drawing, u64)> = ["headjoin", "twoout", "wide"]
            .iter()
            .flat_map(|&kind| COUNTRY_NAMES.iter().flat_map(move |&country| [1, 2, 3].map(move |seed| (kind, country, seed))))
            .map(|(kind, country, seed)| {
                let drawing = crate::trial::trial_drawing(kind, 1024, named(country)).expect("a trial drawing");
                (format!("{kind}-{country}-s{seed}-1024"), as_the_folders_hold_it(drawing), seed)
            })
            .collect();
        for left in &LEFT {
            let named_so = LEFT.iter().filter(|other| other.world == left.world && other.counted == left.counted).count();
            assert_eq!(named_so, 1, "LEFT names {:?} of {} {named_so} times", left.counted, left.world);
            assert!(example.iter().chain(&folders).any(|(name, _, _)| name == left.world), "LEFT names {}, which is not grown", left.world);
        }
        // The example's worlds first, then the trial folders', each set made at once, each world on
        // a thread of its own.
        let mut failures = Vec::new();
        for cases in [&example, &folders] {
            failures.extend(std::thread::scope(|scope| {
                let checking: Vec<_> = (cases.iter()).map(|(name, drawing, seed)| scope.spawn(move || left_failures(name, &drawn(*seed, drawing)))).collect();
                checking.into_iter().flat_map(|failures| failures.join().expect("a world is made and counted")).collect::<Vec<String>>()
            }));
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// A drawing 1024 m across in mountain country, with no lake: its picture 64 pixels a side, a
    /// ridge along `x = 0` each pixel `1 - |x| / 0.5` high (no less than 0), `x` the pixel's middle
    /// in shares of half the drawing's size, so the ridge falls to nothing 256 m either side; and
    /// one river 4 m wide drawn from (0, 204.8) on the crest through (230.4, 102.4) to
    /// (486.4, -51.2). From seed 1 the river's head lies on the crest and falls steeply, and a brook
    /// from the west passes a few metres from it, lower, and ends on it a few metres down.
    fn a_head_on_a_ridge() -> Drawing {
        let (side, size_m) = (64, 1024);
        let mut picture = Grid::new(side, side, (size_m / side as u32) as f32);
        for row in 0..side {
            for column in 0..side {
                let x = ((column as f64 + 0.5) / side as f64 - 0.5) * 2.0;
                picture.set(column, row, (1.0 - x.abs() / 0.5).max(0.0) as f32);
            }
        }
        let river = River { points: vec![(0.0, 204.8), (230.4, 102.4), (486.4, -51.2)], width_m: 4.0 };
        Drawing { size_m, high_ground: Some(picture), rivers: vec![river], lakes: Vec::new(), country: Country::mountains() }
    }

    /// No dry ground lies below the water beside it where a brook's bank passes a river's steep
    /// head, which the bank of a channel worked out later may not cut below the water the river's
    /// banks hold (`cut_beds`), as the measure counts it: in `a_head_on_a_shoulder` from seeds 2 and
    /// 4 and `a_head_on_a_ridge` from seed 1 (where, before the hold, 4, 3 and 13 dry cells lay up
    /// to 2.15 m below the water beside them) it counts no dry cell but where a course leaves the
    /// map (`dry_cells_but_where_a_course_leaves_the_map`), and those are counted and printed over
    /// the three worlds and bounded as measured: none, the measure counting no dry cell at all on
    /// these worlds, by the border either.
    #[test]
    fn no_dry_ground_lies_below_water_on_steep_heads() {
        let worlds = [("a head on a shoulder, seed 2", a_head_on_a_shoulder(), 2), ("a head on a shoulder, seed 4", a_head_on_a_shoulder(), 4), ("a head on a ridge, seed 1", a_head_on_a_ridge(), 1)];
        let found: Vec<(DryCells, Vec<String>)> = std::thread::scope(|scope| {
            let checking: Vec<_> = (worlds.iter()).map(|(name, drawing, seed)| scope.spawn(move || dry_cells_but_where_a_course_leaves_the_map(name, &drawn(*seed, drawing)))).collect();
            checking.into_iter().map(|found| found.join().expect("a world generates")).collect()
        });
        let (mut cells, mut worlds_with, mut failures) = (DryCells::default(), 0, Vec::new());
        for ((name, _, _), (here, failed)) in worlds.iter().zip(&found) {
            worlds_with += usize::from(here.count > 0);
            cells.take(here, name);
            failures.extend(failed.iter().cloned());
        }
        println!("over the three worlds: {} dry cells in {worlds_with} worlds where a course leaves the map, up to {:.3} m below the water beside them at {}", cells.count, cells.most_m, cells.place);
        assert!(failures.is_empty(), "{failures:#?}");
        assert_eq!((cells.count, worlds_with), (0, 0), "dry cells where a course leaves the map and worlds they lie in, up to {} m below their water, at {}", cells.most_m, cells.place);
    }

    /// World `(x, z)` of cell `cell` of a `size` x `size` world.
    fn at_cell(size: usize, cell: usize) -> (f64, f64) {
        ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64)
    }

    #[test]
    fn ground_under_water_is_wet_ground_and_bears_no_grass() {
        let world = drawn(1, &Drawing::example_valley(1024));
        let layer = &world.layers[WATER_LAYER];
        let wet = Material::WetGround as u8;
        let alone = GroundCell { strongest: wet, second: wet, blend: 0 };
        let under: Vec<usize> = (0..layer.data.len()).filter(|&cell| layer.data[cell] > 0.05).collect();
        assert!(!under.is_empty(), "no cell lies deeper than 0.05 m under water");
        let other: Vec<(usize, GroundCell)> =
            under.iter().map(|&cell| (cell, world.ground.data[cell])).filter(|&(_, ground)| ground != alone).collect();
        assert!(other.is_empty(), "{} of {} cells under water are not wet ground alone, such as {:?}", other.len(), under.len(), &other[..other.len().min(5)]);
    }

    #[test]
    fn a_low_bank_is_wet_and_a_high_one_is_not() {
        /// The bank wetness reaches this far, and the ground is fully wet up to `FULL_M` above the
        /// water's surface and not wetted at all from `NONE_M`: the default rules.
        const REACH: i64 = 40;
        const FULL_M: f32 = 0.3;
        const NONE_M: f32 = 1.5;
        let drawing = Drawing::example_valley(1024);
        let world = drawn(1, &drawing);
        let unwetted = drawn_with(&drawing, WaterRules { bank_wet_reach_m: 0.0, ..WaterRules::default() });
        assert!(world.heights == unwetted.heights, "the bank's wetness moved the ground");
        let (depth, heights) = (&world.layers[WATER_LAYER].data, &world.heights.data);
        let (wetness, unwetted) = (&world.layers["wetness"].data, &unwetted.layers["wetness"].data);
        let size = world.heights.width as i64;
        // Every third cell along both axes: of the water's cells within the reach of each dry one,
        // the lowest and the highest surface. Ground at most `FULL_M` above the lowest is at most
        // that above the nearest's, and ground more than `NONE_M` above the highest more than that.
        let (mut low, mut high, mut failures) = (0, 0, Vec::new());
        for j in (0..size).step_by(3) {
            for i in (0..size).step_by(3) {
                let cell = (j * size + i) as usize;
                if depth[cell] > 0.0 {
                    continue;
                }
                let mut surfaces: Option<(f32, f32)> = None;
                for dz in -REACH..=REACH {
                    for dx in -REACH..=REACH {
                        let (x, z) = (i + dx, j + dz);
                        if dx * dx + dz * dz > REACH * REACH || !(0..size).contains(&x) || !(0..size).contains(&z) {
                            continue;
                        }
                        let water = (z * size + x) as usize;
                        if depth[water] > 0.0 {
                            let surface = heights[water] + depth[water];
                            surfaces = Some(surfaces.map_or((surface, surface), |(lowest, highest)| (lowest.min(surface), highest.max(surface))));
                        }
                    }
                }
                let ground = heights[cell];
                match surfaces {
                    Some((lowest, _)) if ground - lowest <= FULL_M => {
                        low += 1;
                        if wetness[cell] < 0.999 {
                            failures.push(format!("({i}, {j}), {} m above the water, has wetness {}", ground - lowest, wetness[cell]));
                        }
                    }
                    Some((_, highest)) if ground - highest > NONE_M + 0.001 => {
                        high += 1;
                        if wetness[cell] > unwetted[cell] {
                            failures.push(format!("({i}, {j}), {} m above the water, has wetness {} for {}", ground - highest, wetness[cell], unwetted[cell]));
                        }
                    }
                    _ => {}
                }
            }
        }
        // 121 low and 11,377 high on this world; 409 low before the water reached its waterline,
        // which took most of the lowest banks under it.
        assert!(low >= 60 && high >= 1000, "only {low} low and {high} high bank cells were checked");
        assert!(failures.is_empty(), "{} of {low} low and {high} high bank cells fail: {:#?}", failures.len(), &failures[..failures.len().min(10)]);
    }

    #[test]
    fn a_drawing_without_rivers_or_lakes_still_has_brooks() {
        let world = drawn(1, &without_water(1024));
        assert!(world.water.lakes.is_empty(), "{:?}", world.water.lakes);
        assert!(!world.water.rivers.is_empty(), "no brook");
        assert!(world.water.rivers.iter().all(|course| course.drawn.is_none() && course.from_lake.is_none()));
        assert!(world.layers[WATER_LAYER].data.iter().any(|&depth| depth > 0.0), "the brooks hold no water");
    }

    #[test]
    fn a_threshold_nothing_reaches_leaves_no_water() {
        // Less ground than this drains through the whole grown land.
        let world = drawn_with(&without_water(1024), WaterRules { brook_start_m2: 1e8, ..WaterRules::default() });
        assert_eq!(world.water, Water::default());
        let layer = &world.layers[WATER_LAYER];
        assert_eq!((layer.width, layer.height), (1024, 1024));
        let wet = layer.data.iter().filter(|&&depth| depth != 0.0).count();
        assert_eq!(wet, 0, "{wet} cells hold water");
    }

    #[test]
    fn a_real_data_world_has_no_water() {
        let s = SyntheticSource::new(1, 50_000.0);
        let w = generate(1, &[&s], &GenConfig { size_cells: 256, ..Default::default() }).unwrap();
        assert_eq!(w.water, Water::default());
        assert!(w.layers.keys().map(String::as_str).eq(LAYER_NAMES), "{:?}", w.layers.keys());
        assert!(!w.layers.contains_key(WATER_LAYER));
        assert_eq!(w.meta.timings.water_ms, 0.0);
    }

    #[test]
    fn the_water_is_in_the_worlds_frame() {
        let drawing = Drawing::example_valley(1024);
        let world = drawn(1, &drawing);
        let layer = &world.layers[WATER_LAYER];
        let size = world.heights.width;
        assert!(!world.water.lakes.is_empty(), "no lake");
        assert!(world.water.rivers.iter().all(|course| course.points.iter().all(|point| point.surface_m >= 0.0)));
        for lake in &world.water.lakes {
            let drawn = drawing.lakes[lake.drawn];
            // The lake's cells: those it may cover whose water's surface is its level.
            let cells: Vec<usize> = cells_within(size, (drawn.x_m, drawn.z_m), 1.5 * f64::from(drawn.radius_m))
                .filter(|&cell| layer.data[cell] > 0.0 && (world.heights.data[cell] + layer.data[cell] - lake.level_m).abs() <= 0.01)
                .collect();
            assert!(cells.len() as f32 >= 0.9 * lake.area_m2, "{} cells of {lake:?} lie at its level", cells.len());
            let lowest_m = cells.iter().map(|&cell| world.heights.data[cell]).fold(f32::INFINITY, f32::min);
            let deepest_m = cells.iter().map(|&cell| layer.data[cell]).fold(0.0, f32::max);
            assert!(((lake.level_m - lowest_m) - deepest_m).abs() <= 0.01, "{lake:?}: its bed's lowest ground {lowest_m} m, its greatest depth {deepest_m} m");
        }
    }
}
