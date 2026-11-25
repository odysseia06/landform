//! The joined ground: the land an edit leaves, the land it kept joined inside the edit's outline to
//! the world the fine steps made from the edit's mixed model (step P10 of the editor tool's
//! prototype, `report2.md`, "The final mechanism"), and the measure the band's ground is judged by.
//!
//! The heights and every layer but the water's are mixed by the outline's weight on the world's
//! cells, as the mixed model's heights are on the model's: the kept land's own values where the
//! weight is 0, the new world's own where it is 1, the mix between. The ground map holds kinds,
//! which cannot be mixed, so each cell takes one land's or the other's, the new one's where the
//! weight passes a fixed, uneven threshold, and the change-over in the band wanders instead of
//! following the outline. The water is joined after the ground, and is not joined here.
//!
//! The band's ground is measured by `band_ground`, which changes nothing, as `unheld` counts water
//! and changes nothing: the mean bend and the mean slope of a world's heights over the band's
//! cells. An edit is judged by how far its land's lie beyond the span between the kept land's and
//! the edited drawing's grown whole (`beyond_span`), at most `BAND_GROUND_BEYOND_MOST` at 2 km (the
//! design's "Done means, restated").

use crate::cover::GroundCell;
use crate::grid::Grid;
use crate::grow::GROW_CELL_M;
use crate::land::model::mixed;
use crate::noise::{fbm, widened};
use crate::water::WATER_LAYER;
use crate::world::World;

/// The seed of the ground map's threshold (`ground_threshold`): its own, the same for every land,
/// every seed and every edit, so a place always changes over at the same weight. The prototype's
/// (round 1, `edit.rs`).
const THRESHOLD_SEED: u64 = 0x6564_6974_0000_0001;
/// The threshold is this middle plus `THRESHOLD_SPREAD` times a field of noise spread over -1..1
/// (`widened`), so it lies between 0.15 and 0.85: a cell of weight 0.15 or less is always the kept
/// land's, one of weight above 0.85 always the new one's, and between, the cells change over along
/// the noise.
const THRESHOLD_MIDDLE: f32 = 0.5;
/// How far the threshold strays from `THRESHOLD_MIDDLE` either way, at most: the noise is held to
/// -1..1, so the threshold lies between 0.15 and 0.85.
const THRESHOLD_SPREAD: f32 = 0.35;
/// The threshold's noise is `fbm` of this many octaves, so the line it draws across the band bends
/// with its first octave and the finer ones fray it.
const THRESHOLD_OCTAVES: u32 = 3;
/// The length of the threshold noise's first octave, in metres, so the line it draws across the
/// band bends every few tens of metres.
const THRESHOLD_WAVELENGTH_M: f64 = 24.0;

/// The band's bend is read on blocks of this many world cells a side: the land model's cell, 8 m,
/// on a drawn world's 1 m cells, so it measures how the big shapes bend and not the detail.
const BEND_BLOCK_CELLS: usize = GROW_CELL_M as usize;
/// The cells in a block of the bend.
const BEND_BLOCK_AREA: f64 = (BEND_BLOCK_CELLS * BEND_BLOCK_CELLS) as f64;
/// The four-neighbour Laplacian weighs its own block this many times against each neighbour.
const LAPLACIAN_CENTRE: f64 = 4.0;

/// An edited land's band ground passes when its mean bend and its mean slope each lie no more than
/// this share of their value beyond the span between the kept land's and the edited drawing's grown
/// whole (`beyond_span`): the design's bound on the example's nine edits and the 27 runs at 2 km
/// ("Done means, restated"; the prototype measured at most 0.82 % for the bend and 0.57 % for the
/// slope).
pub const BAND_GROUND_BEYOND_MOST: f64 = 0.01;

/// The fixed, uneven threshold an edited land's ground map changes over at (P10), at world `(x, z)`
/// in metres: a cell whose weight lies above it takes the new land's ground, one whose weight does
/// not keeps the kept land's. It is `THRESHOLD_MIDDLE` plus `THRESHOLD_SPREAD` times `fbm` of its
/// own seed (`THRESHOLD_SEED`, `THRESHOLD_OCTAVES` octaves, `THRESHOLD_WAVELENGTH_M`) spread over
/// -1..1 (`widened`), so it lies between 0.15 and 0.85 and stands at either often (wherever `fbm`
/// lies a third or more from 0). One field for every land: the same place gives the same threshold
/// on every call, on any thread.
pub fn ground_threshold(x: f64, z: f64) -> f32 {
    THRESHOLD_MIDDLE + THRESHOLD_SPREAD * widened(fbm(THRESHOLD_SEED, x, z, THRESHOLD_OCTAVES, THRESHOLD_WAVELENGTH_M))
}

/// The ground of an edited land (P10): `old`, the world the land kept, joined to `new`, the world
/// the fine steps made from the edit's mixed model (`finish_drawn` in `Frame::Model`), by `weight`,
/// the outline's weight on the world's cells (`Outline::weight`, from 0 to 1).
///
/// **The heights and the layers.** Each cell's height, and its value in every layer but
/// `WATER_LAYER`, is `old`'s, bit for bit, where the weight is 0, `new`'s, bit for bit, where it is
/// 1 (not a mix that comes to it: inside the band's inner edge the land is the mixed model's world
/// exactly, the design's amendment 6), and between `old + w (new - old)`, worked in `f64` and
/// rounded to `f32`, as the mixed model's heights are mixed (`mixed_model`).
///
/// **The ground map**, whose cells are kinds and are not mixed: a cell is `new`'s where its weight
/// lies above `ground_threshold` at its place (world `(i - width / 2, j - height / 2)` for cell
/// `(i, j)`, the half exact, where the fine steps put it), and `old`'s elsewhere. The threshold lies
/// between 0.15 and 0.85, so a cell is `old`'s at weight 0 and `new`'s at weight 1, and every cell
/// is one of the two worlds' own.
///
/// **What the join leaves as `old` has it.** Its `water` layer and its water, the courses and the
/// lakes, untouched: the edit joins the water next, and replaces both. Its notes and what each is
/// about, untouched: they say what the land engine did to the rivers and lakes, so they go with the water, and which of
/// the two worlds' notes hold for a land joined from both is the edit's to say, from what its
/// outline touches; the join reads no drawing. **The meta is `new`'s:** a drawn world's meta says
/// its seed, its source, its window (centred on the land, one for each size), its north and its
/// base elevation, 0 in the model's frame, so the two worlds' metas differ only in the seed, where an
/// edit regrows the whole land from another seed and the land then stands on that one, and in the
/// timings, which are then those of the fine steps that made `new`; the join's own time is the
/// edit's to say.
///
/// Sequential, every cell on its own, so the same worlds and weight give the same world on any
/// thread count.
///
/// # Panics
///
/// When the two worlds do not share a frame and a size, which the edit that calls this makes sure
/// of, both worlds being of one land: when either's `meta.base_elevation_m` is not 0 (a drawn world
/// finished in `Frame::Model`; one finished in `Frame::Lowest` whose lowest cell stood at 0 is the
/// same world), when their heights, ground maps or layers, or `weight`, differ in width, height or
/// cell size, when they hold layers of other names, or when a weight lies outside 0..=1 (or is not
/// a number). It is asserted in every build, since two frames or two sizes joined would make a land
/// that is wrong and says nothing.
pub fn join_ground(old: &World, new: &World, weight: &Grid<f32>) -> World {
    check_joinable(old, new, weight);
    let mix = |old: &Grid<f32>, new: &Grid<f32>| {
        let data = (old.data.iter().zip(&new.data).zip(&weight.data)).map(|((&old, &new), &w)| mixed(old, new, w)).collect();
        Grid { width: old.width, height: old.height, cell_size: old.cell_size, data }
    };
    let layers = old
        .layers
        .iter()
        .map(|(name, layer)| (name.clone(), if name == WATER_LAYER { layer.clone() } else { mix(layer, &new.layers[name]) }))
        .collect();
    let (width, height) = (old.ground.width, old.ground.height);
    let (half_x, half_z) = (width as f64 / 2.0, height as f64 / 2.0);
    let data = (old.ground.data.iter().zip(&new.ground.data).zip(&weight.data))
        .enumerate()
        .map(|(cell, ((&old, &new), &w))| {
            let above = || w > ground_threshold((cell % width) as f64 - half_x, (cell / width) as f64 - half_z);
            if w == 1.0 || (w > 0.0 && above()) {
                new
            } else {
                old
            }
        })
        .collect();
    let ground: Grid<GroundCell> = Grid { width, height, cell_size: old.ground.cell_size, data };
    World {
        heights: mix(&old.heights, &new.heights),
        layers,
        ground,
        meta: new.meta.clone(),
        notes: old.notes.clone(),
        notes_about: old.notes_about.clone(),
        water: old.water.clone(),
    }
}

/// Asserts what `join_ground` needs of its two worlds and its weight (its **Panics**).
fn check_joinable(old: &World, new: &World, weight: &Grid<f32>) {
    let (old_base, new_base) = (old.meta.base_elevation_m, new.meta.base_elevation_m);
    assert!(
        old_base == 0.0 && new_base == 0.0,
        "the ground is joined in the land model's frame (Frame::Model), where a world's base elevation is 0: the kept world's is {old_base} m, the new world's {new_base} m"
    );
    let shape = |width: usize, height: usize, cell_size: f32| (width, height, cell_size.to_bits());
    let heights = shape(old.heights.width, old.heights.height, old.heights.cell_size);
    let of = |what: &str, found: (usize, usize, u32)| {
        assert!(
            found == heights,
            "{what}: {} x {} cells of {} m, where the kept world's heights are {} x {} cells of {} m",
            found.0,
            found.1,
            f32::from_bits(found.2),
            heights.0,
            heights.1,
            f32::from_bits(heights.2)
        );
    };
    of("the new world's heights", shape(new.heights.width, new.heights.height, new.heights.cell_size));
    of("the weight", shape(weight.width, weight.height, weight.cell_size));
    for (name, world) in [("kept", old), ("new", new)] {
        of(&format!("the {name} world's ground map"), shape(world.ground.width, world.ground.height, world.ground.cell_size));
        for (layer, grid) in &world.layers {
            of(&format!("the {name} world's layer {layer}"), shape(grid.width, grid.height, grid.cell_size));
        }
    }
    assert!(
        old.layers.keys().eq(new.layers.keys()),
        "the two worlds hold layers of other names: the kept world {:?}, the new world {:?}",
        old.layers.keys().collect::<Vec<_>>(),
        new.layers.keys().collect::<Vec<_>>()
    );
    if let Some((cell, w)) = weight.data.iter().enumerate().find(|(_, w)| !(0.0..=1.0).contains(*w)) {
        panic!("a weight lies from 0 to 1, but cell {cell}'s is {w}");
    }
}

/// The ground of an edit's band in a world, as the editor tool's design judges it ("Done means,
/// restated"): over the world cells whose weight lies above 0 and below 1, the mean bend and the
/// mean slope of the world's heights. `band_ground` gives it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandGround {
    /// The band's world cells: those whose weight lies above 0 and below 1.
    pub cells: usize,
    /// The mean bend over them, in metres. A cell's bend is that of the block of
    /// `BEND_BLOCK_CELLS` x `BEND_BLOCK_CELLS` cells it lies in, the blocks laid from the world's
    /// first cell: the absolute four-neighbour Laplacian of the blocks' mean heights (the four
    /// neighbouring blocks' means less four times its own, not divided by their spacing). A block of
    /// the outer ring, and a cell beyond the last whole block of a world whose size is not a multiple
    /// of the block, has no four neighbours and a bend of 0, counted in the mean as any other. 0 for
    /// a band of no cells.
    pub bend: f64,
    /// The mean slope over them, in metres per metre: a cell's is the length of its heights'
    /// gradient by central differences, one-sided on the world's edge. 0 for a band of no cells.
    pub slope: f64,
}

/// The band's ground of `world` under an edit's `weight` (`Outline::weight`; `BandGround` says what
/// it measures). It changes nothing. It is the prototype's measure (`edit_probe.rs`, its `bend8`
/// and `slope1`). Its bend is also the earlier trial's `lap8` (`analyse.py`, under
/// `Specs\References\Editor Tool Trial\` in the vault), which the trial read over a ring 32 m wide
/// at the outline and not over the band; the trial had no measure of slope. An edit is judged by
/// how far an edited land's band ground lies beyond the span between the kept land's and the edited
/// drawing's grown whole, each measured over the same band (`beyond_span`). Sums in cell order,
/// sequential, so the same world and weight give the same values on any thread count.
///
/// # Panics
///
/// When `weight` is not the size of `world`'s heights.
pub fn band_ground(world: &World, weight: &Grid<f32>) -> BandGround {
    let heights = &world.heights;
    let (width, height) = (heights.width, heights.height);
    assert!(
        (weight.width, weight.height) == (width, height),
        "the weight is {} x {} cells, the world's heights {width} x {height}",
        weight.width,
        weight.height
    );
    let band: Vec<usize> = (0..width * height).filter(|&cell| weight.data[cell] > 0.0 && weight.data[cell] < 1.0).collect();
    if band.is_empty() {
        return BandGround { cells: 0, bend: 0.0, slope: 0.0 };
    }
    let bends = block_bends(heights);
    let blocks = (width / BEND_BLOCK_CELLS, height / BEND_BLOCK_CELLS);
    let bend_at = |cell: usize| {
        let (block_x, block_z) = ((cell % width) / BEND_BLOCK_CELLS, (cell / width) / BEND_BLOCK_CELLS);
        if block_x < blocks.0 && block_z < blocks.1 {
            bends[block_z * blocks.0 + block_x]
        } else {
            0.0
        }
    };
    let at = |i: usize, j: usize| f64::from(heights.data[j * width + i]);
    let cell_m = f64::from(heights.cell_size);
    // The difference across a cell, central inside and one-sided on the edge; 0 across a world one
    // cell wide.
    let across = |low: usize, high: usize, step: &dyn Fn(usize) -> f64| if high > low { (step(high) - step(low)) / ((high - low) as f64 * cell_m) } else { 0.0 };
    let slope_at = |cell: usize| {
        let (i, j) = (cell % width, cell / width);
        let along_x = across(i.saturating_sub(1), (i + 1).min(width - 1), &|column| at(column, j));
        let along_z = across(j.saturating_sub(1), (j + 1).min(height - 1), &|row| at(i, row));
        along_x.hypot(along_z)
    };
    let (mut bend, mut slope) = (0.0, 0.0);
    for &cell in &band {
        bend += bend_at(cell);
        slope += slope_at(cell);
    }
    let cells = band.len();
    BandGround { cells, bend: bend / cells as f64, slope: slope / cells as f64 }
}

/// The bend of each whole block of `BEND_BLOCK_CELLS` cells a side of `heights`, in block order:
/// the absolute four-neighbour Laplacian of the blocks' mean heights, 0 on the outer ring.
fn block_bends(heights: &Grid<f32>) -> Vec<f64> {
    let (width, height) = (heights.width, heights.height);
    let (across, down) = (width / BEND_BLOCK_CELLS, height / BEND_BLOCK_CELLS);
    let mut means = vec![0.0; across * down];
    for j in 0..down * BEND_BLOCK_CELLS {
        for i in 0..across * BEND_BLOCK_CELLS {
            means[(j / BEND_BLOCK_CELLS) * across + i / BEND_BLOCK_CELLS] += f64::from(heights.data[j * width + i]);
        }
    }
    for mean in &mut means {
        *mean /= BEND_BLOCK_AREA;
    }
    let mut bends = vec![0.0; across * down];
    for block_z in 1..down.saturating_sub(1) {
        for block_x in 1..across.saturating_sub(1) {
            let at = |x: usize, z: usize| means[z * across + x];
            let around = at(block_x - 1, block_z) + at(block_x + 1, block_z) + at(block_x, block_z - 1) + at(block_x, block_z + 1);
            bends[block_z * across + block_x] = (around - LAPLACIAN_CENTRE * at(block_x, block_z)).abs();
        }
    }
    bends
}

/// How far `value` lies beyond the span between `old` and `whole`, as a share of their mean: 0
/// within the span, its ends included; below it, the lower end less `value`, and above it, `value`
/// less the higher end, each divided by the mean's size (infinite when the mean is 0). NaN when any
/// of the three is. The band test: an edited land's bend and slope (`band_ground`) against the kept
/// land's and the edited drawing's grown whole, each at most `BAND_GROUND_BEYOND_MOST` at 2 km.
pub fn beyond_span(value: f64, old: f64, whole: f64) -> f64 {
    if value.is_nan() || old.is_nan() || whole.is_nan() {
        return f64::NAN;
    }
    let (low, high) = (old.min(whole), old.max(whole));
    let beyond = if value < low {
        low - value
    } else if value > high {
        value - high
    } else {
        return 0.0;
    };
    beyond / ((old + whole) / 2.0).abs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cover::LAYER_NAMES;
    use crate::grow::{grow, start_of, About};
    use crate::land::model::{mixed_model, Mixed};
    use crate::land::outline::tests::{old_land, EXAMPLE_SEED};
    use crate::land::outline::{footprint, lake_zones, outline, Marks, REACH_MIN_M};
    use crate::rng::SplitMix64;
    use crate::trial::example_edit;
    use crate::water::{CourseEnd, CoursePoint, LakeWater, RiverCourse, Water};
    use crate::window::Window;
    use crate::world::{finish_drawn, Frame, GenConfig, Timings, WorldMeta};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// Cells across the hand-made worlds: eight blocks of the bend a side.
    const SIZE: usize = 64;
    /// The lowest and highest the threshold may be.
    const THRESHOLD_LOWEST: f32 = 0.15;
    const THRESHOLD_HIGHEST: f32 = 0.85;

    fn bits(values: &[f32]) -> Vec<u32> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    /// A hand-made world `SIZE` cells a side in the model's frame: heights, each of `LAYER_NAMES`
    /// and the `water` layer from `value(layer, cell)` (the heights as layer `"heights"`), every
    /// ground cell `ground`, one course and one lake, a note about river `seed`, and `seed` in its
    /// meta.
    fn world(value: impl Fn(&str, usize) -> f32, ground: GroundCell, seed: u64) -> World {
        let grid = |name: &str| Grid { width: SIZE, height: SIZE, cell_size: 1.0, data: (0..SIZE * SIZE).map(|cell| value(name, cell)).collect() };
        let layers = LAYER_NAMES.iter().copied().chain([WATER_LAYER]).map(|name| (name.to_string(), grid(name))).collect();
        let point = |x_m: f32| CoursePoint { x_m, z_m: seed as f32, surface_m: value("course", 0), width_m: 2.0, depth_m: 0.5 };
        let water = Water {
            rivers: vec![RiverCourse { drawn: Some(0), from_lake: None, points: vec![point(-8.0), point(8.0)], end: CourseEnd::Edge }],
            lakes: vec![LakeWater { drawn: 0, level_m: value("lake", 0), x_m: 0.0, z_m: 0.0, area_m2: 10.0 }],
        };
        let meta = WorldMeta {
            seed,
            source_id: "by hand".to_string(),
            window: Window { source: 0, center_x: 0.0, center_y: 0.0, rotation_rad: 0.0, mirrored: false, size_m: SIZE as f64 },
            base_elevation_m: 0.0,
            north: (0.0, -1.0),
            timings: Timings { total_ms: seed as f32, ..Timings::default() },
        };
        let (notes, notes_about) = (vec![format!("World {seed}.")], vec![About::River(seed as usize)]);
        World { heights: grid("heights"), layers, ground: Grid { width: SIZE, height: SIZE, cell_size: 1.0, data: vec![ground; SIZE * SIZE] }, meta, notes, notes_about, water }
    }

    /// The heights' first two cells, which a mix that comes to weight 0 or 1 would not keep: the
    /// kept world's -0.0 (`0 * x` added to it is +0.0) and 1e30, the new world's 1e-7 and 3e-7
    /// (`1e30 + 1 (3e-7 - 1e30)` is 0 in `f64`).
    const KEPT_FIRST: [f32; 2] = [-0.0, 1e30];
    const NEW_FIRST: [f32; 2] = [1e-7, 3e-7];

    /// The kept world: every value its own (`KEPT_FIRST` its first heights), its ground one kind.
    fn kept() -> World {
        let value = |name: &str, cell: usize| if name == "heights" && cell < 2 { KEPT_FIRST[cell] } else { (name.len() * 1000 + cell) as f32 * 0.25 };
        world(value, GroundCell { strongest: 1, second: 2, blend: 3 }, 1)
    }

    /// The new world: every value other than the kept world's (`NEW_FIRST` its first heights), its
    /// ground another kind.
    fn grown_new() -> World {
        let value = |name: &str, cell: usize| if name == "heights" && cell < 2 { NEW_FIRST[cell] } else { 7.0 - (name.len() * 999 + cell) as f32 * 0.5 };
        world(value, GroundCell { strongest: 4, second: 5, blend: 6 }, 2)
    }

    fn weight_of(value: impl FnMut(usize) -> f32) -> Grid<f32> {
        Grid { width: SIZE, height: SIZE, cell_size: 1.0, data: (0..SIZE * SIZE).map(value).collect() }
    }

    /// Asserts that `joined` keeps what the join does not make as `old` has it (the `water` layer,
    /// the water, the notes and what they are about) and takes `new`'s meta.
    fn assert_rest(joined: &World, old: &World, new: &World, case: &str) {
        assert_eq!(bits(&joined.layers[WATER_LAYER].data), bits(&old.layers[WATER_LAYER].data), "{case}: the water layer is the kept world's");
        assert_eq!((&joined.water, &joined.notes, &joined.notes_about), (&old.water, &old.notes, &old.notes_about), "{case}: the water and the notes are the kept world's");
        assert_eq!((joined.meta.seed, joined.meta.timings), (new.meta.seed, new.meta.timings), "{case}: the meta is the new world's");
        assert_eq!(joined.meta, new.meta, "{case}");
    }

    #[test]
    fn the_threshold_is_fixed_uneven_and_between_its_bounds() {
        /// The place sampled: this many metres a side, every metre, from this corner.
        const SPAN_M: usize = 400;
        const FROM_M: f64 = -1000.5;
        /// Within this of a bound, the bound is reached.
        const REACHED: f32 = 1e-6;
        let sample = || -> Vec<f32> { (0..SPAN_M * SPAN_M).map(|k| ground_threshold(FROM_M + (k % SPAN_M) as f64, FROM_M + (k / SPAN_M) as f64)).collect() };
        let (first, again) = (sample(), sample());
        assert_eq!(bits(&first), bits(&again), "the same place gives the same threshold on every call");
        let (lowest, highest) = first.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &t| (low.min(t), high.max(t)));
        println!("the threshold over {SPAN_M} m a side: {lowest} to {highest}");
        assert!(lowest >= THRESHOLD_LOWEST && highest <= THRESHOLD_HIGHEST, "it lies between its bounds");
        assert!(lowest - THRESHOLD_LOWEST < REACHED && THRESHOLD_HIGHEST - highest < REACHED, "it reaches both");
        // Uneven: along a row it changes over tens of metres, smoothly from metre to metre.
        let row = &first[..SPAN_M];
        let crossings = row.windows(2).filter(|pair| (pair[0] > THRESHOLD_MIDDLE) != (pair[1] > THRESHOLD_MIDDLE)).count();
        let steepest = row.windows(2).map(|pair| (pair[1] - pair[0]).abs()).fold(0.0, f32::max);
        println!("along {SPAN_M} m it crosses its middle {crossings} times, by at most {steepest} a metre");
        assert!((SPAN_M / 100..=SPAN_M / 4).contains(&crossings), "{crossings} crossings over {SPAN_M} m");
        assert!(steepest < THRESHOLD_SPREAD, "{steepest} a metre");
        // The threshold of a place does not depend on the threshold of another being asked first.
        assert_eq!(ground_threshold(12.5, -40.0).to_bits(), ground_threshold(12.5, -40.0).to_bits());
    }

    #[test]
    fn weight_0_gives_the_kept_world_and_weight_1_the_new_bit_for_bit() {
        let (old, new) = (kept(), grown_new());
        let none = join_ground(&old, &new, &weight_of(|_| 0.0));
        assert_eq!(bits(&none.heights.data), bits(&old.heights.data), "weight 0: the heights, -0.0 among them");
        assert_eq!(bits(&none.heights.data[..2]), bits(&KEPT_FIRST));
        for (name, layer) in &old.layers {
            assert_eq!(bits(&none.layers[name].data), bits(&layer.data), "weight 0: layer {name}");
        }
        assert_eq!(none.ground, old.ground, "weight 0: the ground map");
        assert_rest(&none, &old, &new, "weight 0");

        let all = join_ground(&old, &new, &weight_of(|_| 1.0));
        assert_eq!(bits(&all.heights.data), bits(&new.heights.data), "weight 1: the heights, 1e-7 and 3e-7 among them");
        assert_eq!(bits(&all.heights.data[..2]), bits(&NEW_FIRST));
        for name in LAYER_NAMES {
            assert_eq!(bits(&all.layers[name].data), bits(&new.layers[name].data), "weight 1: layer {name}");
        }
        assert_eq!(all.ground, new.ground, "weight 1: the ground map");
        assert_rest(&all, &old, &new, "weight 1");
        assert_eq!(all.layers.keys().collect::<Vec<_>>(), old.layers.keys().collect::<Vec<_>>());
    }

    #[test]
    fn between_0_and_1_the_join_is_the_mix_and_at_one_half_the_mean() {
        let (old, new) = (kept(), grown_new());
        let half = join_ground(&old, &new, &weight_of(|_| 0.5));
        for (name, joined, (o, n)) in std::iter::once(("heights", &half.heights, (&old.heights, &new.heights)))
            .chain(LAYER_NAMES.iter().map(|&name| (name, &half.layers[name], (&old.layers[name], &new.layers[name]))))
        {
            for cell in 2..SIZE * SIZE {
                // Quarters and halves of metres: their mean is exact.
                let mean = (f64::from(o.data[cell]) + f64::from(n.data[cell])) / 2.0;
                assert_eq!(f64::from(joined.data[cell]), mean, "{name} at cell {cell}");
            }
        }
        assert_rest(&half, &old, &new, "weight 0.5");
        // Any weight between: the mix in f64, rounded to f32.
        let mut random = SplitMix64::new(5);
        let weight = weight_of(|_| random.next_f64() as f32);
        let joined = join_ground(&old, &new, &weight);
        for cell in 0..SIZE * SIZE {
            let w = f64::from(weight.data[cell]);
            for (name, found, o, n) in std::iter::once(("heights", &joined.heights, &old.heights, &new.heights))
                .chain(LAYER_NAMES.iter().map(|&name| (name, &joined.layers[name], &old.layers[name], &new.layers[name])))
            {
                let (o, n) = (f64::from(o.data[cell]), f64::from(n.data[cell]));
                let expected = if w == 0.0 {
                    o as f32
                } else if w == 1.0 {
                    n as f32
                } else {
                    (o + w * (n - o)) as f32
                };
                assert_eq!(found.data[cell].to_bits(), expected.to_bits(), "{name} at cell {cell}, weight {w}");
            }
        }
        assert_rest(&joined, &old, &new, "weights between");
    }

    #[test]
    fn the_ground_map_takes_the_new_cell_above_the_threshold_and_no_cell_of_neither() {
        let (old, new) = (kept(), grown_new());
        let mut random = SplitMix64::new(9);
        // Weights spread over 0..1, with cells of exactly 0 and exactly 1 among them.
        let weight = weight_of(|cell| match cell % 7 {
            0 => 0.0,
            1 => 1.0,
            _ => random.next_f64() as f32,
        });
        let joined = join_ground(&old, &new, &weight);
        let half = SIZE as f64 / 2.0;
        let (mut taken, mut kept_in_band) = (0, 0);
        for cell in 0..SIZE * SIZE {
            let found = joined.ground.data[cell];
            assert!(found == old.ground.data[cell] || found == new.ground.data[cell], "cell {cell} is neither world's: {found:?}");
            let w = weight.data[cell];
            let threshold = ground_threshold((cell % SIZE) as f64 - half, (cell / SIZE) as f64 - half);
            assert!((THRESHOLD_LOWEST..=THRESHOLD_HIGHEST).contains(&threshold));
            assert_eq!(found == new.ground.data[cell], w > threshold, "cell {cell}: weight {w}, threshold {threshold}");
            if w > 0.0 && w < 1.0 {
                if w > threshold {
                    taken += 1;
                } else {
                    kept_in_band += 1;
                }
            }
        }
        println!("in the band {taken} cells take the new ground and {kept_in_band} keep the kept ground");
        assert!(taken > 0 && kept_in_band > 0);
    }

    #[test]
    #[should_panic(expected = "the land model's frame")]
    fn a_world_in_another_frame_is_not_joined() {
        let mut lowered = grown_new();
        lowered.meta.base_elevation_m = 12.5;
        join_ground(&kept(), &lowered, &weight_of(|_| 0.5));
    }

    #[test]
    #[should_panic(expected = "the new world's heights: 32 x 32 cells")]
    fn worlds_of_two_sizes_are_not_joined() {
        let mut smaller = grown_new();
        smaller.heights = Grid::new(SIZE / 2, SIZE / 2, 1.0);
        join_ground(&kept(), &smaller, &weight_of(|_| 0.5));
    }

    #[test]
    #[should_panic(expected = "the weight: 64 x 63 cells")]
    fn a_weight_of_another_size_is_not_taken() {
        join_ground(&kept(), &grown_new(), &Grid::new(SIZE, SIZE - 1, 1.0));
    }

    #[test]
    #[should_panic(expected = "cell 3's is 1.5")]
    fn a_weight_beyond_0_to_1_is_not_taken() {
        join_ground(&kept(), &grown_new(), &weight_of(|cell| if cell == 3 { 1.5 } else { 0.5 }));
    }

    #[test]
    #[should_panic(expected = "layers of other names")]
    fn worlds_with_other_layers_are_not_joined() {
        let mut other = grown_new();
        other.layers.remove(WATER_LAYER);
        join_ground(&kept(), &other, &weight_of(|_| 0.5));
    }

    #[test]
    fn the_bands_bend_and_slope_are_read_on_its_cells_alone() {
        // The band: the weight rises across columns 10 to 52; it is 0 before them and 1 after.
        let weight = weight_of(|cell| ((cell % SIZE) as f32 - 9.0).clamp(0.0, 44.0) / 44.0);
        let band: Vec<usize> = (0..SIZE * SIZE).filter(|&cell| (10..53).contains(&(cell % SIZE))).collect();
        assert!(band.iter().all(|&cell| weight.data[cell] > 0.0 && weight.data[cell] < 1.0));
        // A plane bends nowhere and slopes alike everywhere, its edges too.
        let plane = world(|name, cell| if name == "heights" { 0.25 * (cell % SIZE) as f32 + 0.5 * (cell / SIZE) as f32 } else { 0.0 }, GroundCell::default(), 1);
        let flat = band_ground(&plane, &weight);
        assert_eq!((flat.cells, flat.bend), (band.len(), 0.0));
        assert!((flat.slope - 0.25f64.hypot(0.5)).abs() < 1e-12, "{flat:?}");
        // Heights of the column squared: the blocks' means along a row are 64 k² + 56 k + 17.5 for
        // block k, whose second difference is 128; down a column they do not change. The slope is
        // twice the column inside, and one-sided at the world's first and last columns.
        let bowl = world(|name, cell| if name == "heights" { ((cell % SIZE) * (cell % SIZE)) as f32 } else { 0.0 }, GroundCell::default(), 1);
        let blocks = SIZE / BEND_BLOCK_CELLS;
        let bend_of = |cell: usize| {
            let (x, z) = ((cell % SIZE) / BEND_BLOCK_CELLS, (cell / SIZE) / BEND_BLOCK_CELLS);
            if x == 0 || z == 0 || x == blocks - 1 || z == blocks - 1 {
                0.0
            } else {
                128.0
            }
        };
        let bend = band.iter().map(|&cell| bend_of(cell)).sum::<f64>() / band.len() as f64;
        let slope = band.iter().map(|&cell| 2.0 * (cell % SIZE) as f64).sum::<f64>() / band.len() as f64;
        let found = band_ground(&bowl, &weight);
        assert_eq!(found.cells, band.len());
        assert!((found.bend - bend).abs() < 1e-9 && (found.slope - slope).abs() < 1e-9, "{found:?}, expected bend {bend} and slope {slope}");
        let everywhere = band_ground(&bowl, &weight_of(|_| 0.5));
        // One-sided at the edges: 1 - 0 at the first column, 63² - 62² at the last.
        let last = ((SIZE - 1).pow(2) - (SIZE - 2).pow(2)) as f64;
        let slope = (1.0 + (1..SIZE - 1).map(|i| 2.0 * i as f64).sum::<f64>() + last) / SIZE as f64;
        assert!((everywhere.slope - slope).abs() < 1e-9, "with the world's edges: {everywhere:?}, expected slope {slope}");
        // A band of no cells.
        assert_eq!(band_ground(&bowl, &weight_of(|cell| (cell % 2) as f32)), BandGround { cells: 0, bend: 0.0, slope: 0.0 });
    }

    #[test]
    fn beyond_the_span_is_a_share_of_the_spans_mean() {
        assert_eq!(beyond_span(1.0, 0.9, 1.1), 0.0);
        assert_eq!(beyond_span(0.9, 0.9, 1.1), 0.0, "an end is within");
        assert_eq!(beyond_span(1.1, 1.1, 0.9), 0.0, "either way round");
        assert_eq!(beyond_span(0.5, 1.0, 3.0), 0.25, "below: (1 - 0.5) / 2");
        assert_eq!(beyond_span(4.0, 3.0, 1.0), 0.5, "above: (4 - 3) / 2");
        assert_eq!(beyond_span(2.0, 2.0, 2.0), 0.0);
        assert_eq!(beyond_span(3.0, 2.0, 2.0), 0.5);
        assert_eq!(beyond_span(1.0, 0.0, 0.0), f64::INFINITY);
        assert!(beyond_span(f64::NAN, 1.0, 2.0).is_nan() && beyond_span(1.0, f64::NAN, 2.0).is_nan() && beyond_span(1.0, 1.0, f64::NAN).is_nan());
    }

    /// The prototype's band ground on the example (`runs2\example_final.jsonl`): for each edit, the
    /// band's cells, then the bend and the slope of the edited land, the kept land, the edited
    /// drawing grown whole and the mixed model's world.
    const PROTOTYPE_BAND: [(&str, usize, [f64; 4], [f64; 4]); 3] = [
        ("a", 518_253, [0.67072, 0.6721, 0.67131, 0.6702], [0.35309, 0.35525, 0.35277, 0.35295]),
        ("c", 512_871, [0.89249, 0.89385, 0.89073, 0.89267], [0.42775, 0.43034, 0.42724, 0.42733]),
        ("d", 249_764, [0.67624, 0.68711, 0.67368, 0.67472], [0.37602, 0.37627, 0.37578, 0.37586]),
    ];
    /// The prototype recorded its bends and slopes to five decimals, so each of ours is asked to lie
    /// within half the last of them: to round to the prototype's.
    const PROTOTYPE_WITHIN: f64 = 5e-6;

    /// Step P10 on the example at 2,048 m in hills from seed 11, with the brook's point moved 30 m
    /// (`a`), the main river's middle point moved 60 m (`c`) and a hill painted (`d`): the whole
    /// chain from the kept land (its model grown, its world finished in the model's frame) through
    /// the edited drawing's model grown whole, the outline with its lakes' zones, the mixed model
    /// and its world, to the joined ground. Every cell of weight 0 is the kept land's bit for bit
    /// (heights, ground map, every layer but `water`), every cell of weight 1 the mixed model's
    /// world's, and the band's bend and slope lie no more than `BAND_GROUND_BEYOND_MOST` of their
    /// value beyond the span between the kept land's and the edited drawing's grown whole (the
    /// prototype: 0.09 % for `a`'s bend at the most). The band's cells are the prototype's exactly,
    /// and the four worlds' bends and slopes its values to its five decimals (`PROTOTYPE_BAND`,
    /// `PROTOTYPE_WITHIN`), so the chain of Tasks 2 to 5 measures as the prototype's did.
    #[test]
    fn the_examples_band_ground_lies_within_one_percent_of_the_span() {
        let old = old_land();
        let config = GenConfig::default();
        let none = Marks::none(old.world.heights.width);
        for (name, prototype_cells, prototype_bend, prototype_slope) in PROTOTYPE_BAND {
            let drawing = example_edit(&old.drawing, name).expect("an edit of the example");
            let new_model = grow(&drawing, &config.water, EXAMPLE_SEED, &AtomicBool::new(false)).expect("the edited drawing grows");
            let new_start = start_of(&drawing, &config.water, EXAMPLE_SEED).expect("the edited drawing is valid");
            let found = footprint(&old.start, &new_start, &old.drawing, &drawing);
            let zones = lake_zones(&old.world, &old.model, &new_model, &old.drawing, &drawing);
            let shape = outline(drawing.size_m, &found, &zones, &none, REACH_MIN_M);
            let Mixed { model: mixed, troughs, .. } = mixed_model(&old.model, &new_model, &drawing, &config.water, EXAMPLE_SEED, &shape.model_weight).expect("the mixed model");
            let mixed_world = finish_drawn(EXAMPLE_SEED, &drawing, &config, &mixed, Frame::Model, &AtomicBool::new(false)).expect("the mixed model's world");
            let clock = Instant::now();
            let joined = join_ground(&old.world, &mixed_world, &shape.weight);
            let join_ms = clock.elapsed().as_secs_f64() * 1000.0;
            let whole = finish_drawn(EXAMPLE_SEED, &drawing, &config, &new_model, Frame::Model, &AtomicBool::new(false)).expect("the world grown whole");

            // Every layer but the water's, by the worlds' own names.
            assert!(joined.layers.keys().eq(old.world.layers.keys()) && old.world.layers.contains_key(WATER_LAYER), "edit {name}: the layers");
            let mixed_layers: Vec<&str> = old.world.layers.keys().map(String::as_str).filter(|&layer| layer != WATER_LAYER).collect();
            let (mut outside, mut inside) = (0, 0);
            for (cell, &weight) in shape.weight.data.iter().enumerate() {
                let (w, from) = if weight == 0.0 {
                    (0, &old.world)
                } else if weight == 1.0 {
                    (1, &mixed_world)
                } else {
                    continue;
                };
                let same = joined.heights.data[cell].to_bits() == from.heights.data[cell].to_bits()
                    && joined.ground.data[cell] == from.ground.data[cell]
                    && mixed_layers.iter().all(|&layer| joined.layers[layer].data[cell].to_bits() == from.layers[layer].data[cell].to_bits());
                assert!(same, "edit {name}: cell {cell} of weight {w} is not that world's bit for bit");
                if w == 0 {
                    outside += 1;
                } else {
                    inside += 1;
                }
            }
            assert_eq!(bits(&joined.layers[WATER_LAYER].data), bits(&old.world.layers[WATER_LAYER].data), "edit {name}: the water layer is the kept land's");
            assert_eq!(joined.water, old.world.water, "edit {name}: the water is the kept land's");

            let measured @ [edited, before, grown_whole, mixed_ground] = [&joined, &old.world, &whole, &mixed_world].map(|world| band_ground(world, &shape.weight));
            assert!(measured.iter().all(|ground| ground.cells == edited.cells && ground.cells > 0), "edit {name}: one band");
            let (bend_beyond, slope_beyond) = (beyond_span(edited.bend, before.bend, grown_whole.bend), beyond_span(edited.slope, before.slope, grown_whole.slope));
            println!(
                "edit {name}: join {join_ms:.1} ms; {} troughs; {outside} cells of weight 0, {inside} of weight 1, band {} cells (prototype {prototype_cells})",
                troughs.len(),
                edited.cells
            );
            println!(
                "  bend  edited {:.5}, kept {:.5}, whole {:.5}, mixed {:.5}; beyond the span {:.3} % (prototype {:.5}, {:.5}, {:.5}, {:.5}; {:.3} %)",
                edited.bend,
                before.bend,
                grown_whole.bend,
                mixed_ground.bend,
                bend_beyond * 100.0,
                prototype_bend[0],
                prototype_bend[1],
                prototype_bend[2],
                prototype_bend[3],
                beyond_span(prototype_bend[0], prototype_bend[1], prototype_bend[2]) * 100.0
            );
            println!(
                "  slope edited {:.5}, kept {:.5}, whole {:.5}, mixed {:.5}; beyond the span {:.3} % (prototype {:.5}, {:.5}, {:.5}, {:.5}; {:.3} %)",
                edited.slope,
                before.slope,
                grown_whole.slope,
                mixed_ground.slope,
                slope_beyond * 100.0,
                prototype_slope[0],
                prototype_slope[1],
                prototype_slope[2],
                prototype_slope[3],
                beyond_span(prototype_slope[0], prototype_slope[1], prototype_slope[2]) * 100.0
            );
            assert!(outside > 0 && inside > 0, "edit {name}");
            assert_eq!(edited.cells, prototype_cells, "edit {name}: the band's cells, against the prototype's");
            for (k, (ground, world)) in measured.iter().zip(["edited", "kept", "whole", "mixed"]).enumerate() {
                let (bend, slope) = (prototype_bend[k], prototype_slope[k]);
                assert!((ground.bend - bend).abs() <= PROTOTYPE_WITHIN, "edit {name}: the {world} land's bend {} against the prototype's {bend}", ground.bend);
                assert!((ground.slope - slope).abs() <= PROTOTYPE_WITHIN, "edit {name}: the {world} land's slope {} against the prototype's {slope}", ground.slope);
            }
            assert!(bend_beyond <= BAND_GROUND_BEYOND_MOST, "edit {name}: the band's bend lies {:.2} % beyond the span", bend_beyond * 100.0);
            assert!(slope_beyond <= BAND_GROUND_BEYOND_MOST, "edit {name}: the band's slope lies {:.2} % beyond the span", slope_beyond * 100.0);
        }
    }
}
