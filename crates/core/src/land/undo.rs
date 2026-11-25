//! A step of undo (the editor tool's plan, stage 2, "Decided while planning" 2): what lies between
//! two lands of one size, kept so that one swap with a land exchanges the two. `Change::between(from,
//! to)` holds, for every grid of the world (the heights, the ground map, each layer by name) and each
//! of the two marks, the runs of cells in which the two lands differ, with `to`'s values; and whole,
//! each other part of `to` that differs from `from`'s: the drawing, the drawing asked for, the seed,
//! the settings, the land model's heights, the first lakes' levels, the water, the notes with their
//! subjects, the world's meta (its timings among it) and the reach. `Change::swap` exchanges those
//! values with a land's: swapped into `from` it makes it `to` and holds `from`'s values, so swapped
//! again it gives `from` back. Undo and redo are that one call, exact by construction, since nothing
//! is worked out again: values are only exchanged.
//!
//! **Bit for bit.** A cell differs when its bits differ (`f32::to_bits`, so 0 and -0 differ, as do
//! two NaNs of other bits); a part held whole differs when the bytes the land file holds it by differ
//! (`file::drawing_bytes` for a drawing, its checksums left at 0 so nothing is hashed, and the
//! file's own writers for the rest, called and not copied). So a swap gives back the land's file
//! byte for byte, its world's timings among it. `Land`, its world and its marks are taken apart
//! field by field in `between`, `swap` and `bytes`, and every part's `heap_bytes` takes its type
//! apart likewise, so a field added to any of them fails to compile until a change holds it.
//!
//! **Runs.** `between` reads the grids and the marks row by row (a row is the world's side of
//! cells) on the threads of the pool the call runs in, each row's runs found from its own cells, so
//! a change is the same on any count of threads; it uses rayon's current pool and nothing else, so
//! the extension calls it inside `pool::run`, as every other parallel step of the core. `swap`
//! exchanges the runs on the calling thread alone and uses no pool, so it is called on any thread,
//! the main thread among them (stage 2's ruling R3). A run never crosses a row. Between two
//! cells of a row that differ, the equal cells of a gap stay in the run when their values cost no
//! more than a run does (`RUN_BYTES`), so a change is as small as runs make it: swapping an equal
//! cell changes nothing. Marks of two lengths (a land read from a file whose marks are shorter than
//! its world, and the land a hold made of it) are held whole.
//!
//! **A step of a part.** A hold, an ask and a reach change a land's held marks, the drawing it asks
//! for or its reach and nothing else, so `Change::to_parts(land, held, asked, reach_m)` makes the
//! step between `land` and the land it is with those parts, without that second land: `between` of
//! the two exactly. `Change::touched` says which parts a change exchanges, so a caller that keeps
//! what it made of a land's parts (the extension's images of the world and the marks, the values it
//! works out from the drawings) makes again only what a swap changed.

use super::file::{drawing_bytes, first_lakes_bytes, meta_bytes, model_bytes, notes_bytes, settings_bytes, water_bytes};
use super::outline::Marks;
use super::Land;
use crate::cover::GroundCell;
use crate::drawing::{Drawing, Lake, River};
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{About, GrownLake};
use crate::water::{CoursePoint, LakeWater, RiverCourse, Water};
use crate::world::{GenConfig, World, WorldMeta};
use rayon::prelude::*;
use std::mem::{size_of, swap};

/// A run of cells: the index of its first cell in its list, and its count of cells.
type Run = (usize, usize);

/// What a run costs beside its values, in bytes: its first cell and its count. A gap of equal cells
/// between two that differ stays in a run when its values cost no more than this.
const RUN_BYTES: usize = size_of::<Run>();

/// The notes of a world and what each is about, which a change holds whole together.
type Notes = (Vec<String>, Vec<About>);

/// Which parts of a land a change exchanges (`Change::touched`), each `true` where the change holds
/// a value of it: a swap leaves every other part as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Touched {
    /// The world: its heights, its ground map, a layer, its water, its notes or its meta (its seed
    /// and timings among it).
    pub world: bool,
    /// The held marks.
    pub held: bool,
    /// The hand marks.
    pub by_hand: bool,
    /// The drawing the land was grown from, or the drawing it asks for.
    pub drawings: bool,
    /// The seed, the settings, the land model's heights, the first lakes' levels or the reach.
    pub rest: bool,
}

/// What lies between two lands of one size (the module's doc): the other land's values in the runs
/// of each grid and mark where the two differ, and the other land's parts whole where they differ.
/// Swapped with one of the two lands, it makes it the other and holds what it was.
#[derive(Debug, Clone)]
pub struct Change {
    /// The world's heights.
    heights: Cells<f32>,
    /// The ground map.
    ground: Cells<GroundCell>,
    /// Each layer of the world, by name, in the names' order.
    layers: Vec<(String, Cells<f32>)>,
    /// The held marks.
    held: Cells<bool>,
    /// The hand marks.
    by_hand: Cells<bool>,
    /// Each part outside the grids and the marks, the other land's, where it differs.
    drawing: Option<Drawing>,
    asked: Option<Drawing>,
    seed: Option<u64>,
    config: Option<GenConfig>,
    model_heights: Option<Grid<f32>>,
    first_lake_levels: Option<Vec<GrownLake>>,
    water: Option<Water>,
    notes: Option<Notes>,
    meta: Option<WorldMeta>,
    reach_m: Option<f32>,
}

impl Change {
    /// What lies between `from` and `to`, with `to`'s values: `swap` into `from` makes it `to`.
    /// `BadConfig` for two lands of different sizes (their drawings', or any grid of their worlds by
    /// its sides, its cell size or its count of cells) or whose worlds hold other layers, which no
    /// edit, hold or setting of a land makes.
    pub fn between(from: &Land, to: &Land) -> Result<Change, LandformError> {
        // Every field is named, so a field added to the land fails to compile until it is held.
        let Land { drawing, asked, seed, config, model_heights, first_lake_levels, world, marks, reach_m } = from;
        let World { heights, layers, ground, meta, notes, notes_about, water } = world;
        let Marks { held, by_hand } = marks;
        if drawing.size_m != to.drawing.size_m {
            return Err(LandformError::BadConfig(format!(
                "a step of undo lies between two lands of one size, not one of {} m and one of {} m",
                drawing.size_m, to.drawing.size_m
            )));
        }
        if !layers.keys().eq(to.world.layers.keys()) {
            let names = |world: &World| world.layers.keys().cloned().collect::<Vec<_>>().join(", ");
            return Err(LandformError::BadConfig(format!(
                "a step of undo lies between two lands whose worlds hold the same layers, not {} and {}",
                names(world),
                names(&to.world)
            )));
        }
        let grids_fit = frame(heights) == frame(&to.world.heights)
            && frame(ground) == frame(&to.world.ground)
            && layers.values().zip(to.world.layers.values()).all(|(a, b)| frame(a) == frame(b));
        if !grids_fit {
            return Err(LandformError::BadConfig(format!(
                "a step of undo lies between two lands of one size, and the grids of these two worlds of {} m are not of one size",
                drawing.size_m
            )));
        }
        let row = heights.width;
        let to_world = &to.world;
        Ok(Change {
            heights: Cells::between(&heights.data, &to_world.heights.data, row),
            ground: Cells::between(&ground.data, &to_world.ground.data, row),
            layers: (layers.iter().zip(to_world.layers.values()))
                .map(|((name, layer), other)| (name.clone(), Cells::between(&layer.data, &other.data, row)))
                .collect(),
            held: Cells::between(held, &to.marks.held, row),
            by_hand: Cells::between(by_hand, &to.marks.by_hand, row),
            drawing: differing(drawing, &to.drawing),
            asked: differing(asked, &to.asked),
            seed: differing(seed, &to.seed),
            config: differing(config, &to.config),
            model_heights: differing(model_heights, &to.model_heights),
            first_lake_levels: differing(first_lake_levels, &to.first_lake_levels),
            water: differing(water, &to_world.water),
            notes: (notes_bytes(notes, notes_about) != notes_bytes(&to_world.notes, &to_world.notes_about))
                .then(|| (to_world.notes.clone(), to_world.notes_about.clone())),
            meta: differing(meta, &to_world.meta),
            reach_m: differing(reach_m, &to.reach_m),
        })
    }

    /// What lies between `land` and the land it is with `held` as its held marks, `asked` as the
    /// drawing it asks for and `reach_m` as its reach, each `None` for the land's own, with those
    /// values: `swap` into `land` makes it that land. It is `between` of the two lands exactly
    /// (tested), made without the second land, since no other part of it differs: the step of
    /// `Land::hold`, `Land::ask` and a reach set, which change only these parts. Made after the
    /// call, of the land as the call left it and the values it had before, it is the step back,
    /// which swapped into the land gives the land before the call. The held marks' runs are found
    /// on rayon's current pool, as `between` finds them (the extension calls it inside `pool::run`);
    /// with `held` `None` it uses no pool.
    pub fn to_parts(land: &Land, held: Option<&[bool]>, asked: Option<&Drawing>, reach_m: Option<f32>) -> Change {
        // Every field is named, so a field added to the land fails to compile until it is weighed.
        let Land { drawing: _, asked: land_asked, seed: _, config: _, model_heights: _, first_lake_levels: _, world, marks, reach_m: land_reach_m } = land;
        let World { heights, layers, ground, meta: _, notes: _, notes_about: _, water: _ } = world;
        let Marks { held: land_held, by_hand } = marks;
        Change {
            heights: Cells::none(heights.data.len()),
            ground: Cells::none(ground.data.len()),
            layers: layers.iter().map(|(name, layer)| (name.clone(), Cells::none(layer.data.len()))).collect(),
            held: held.map_or_else(|| Cells::none(land_held.len()), |held| Cells::between(land_held, held, heights.width)),
            by_hand: Cells::none(by_hand.len()),
            drawing: None,
            asked: asked.and_then(|asked| differing(land_asked, asked)),
            seed: None,
            config: None,
            model_heights: None,
            first_lake_levels: None,
            water: None,
            notes: None,
            meta: None,
            reach_m: reach_m.and_then(|reach_m| differing(land_reach_m, &reach_m)),
        }
    }

    /// Which parts of a land the change exchanges: those it holds a value of, the same before and
    /// after a swap.
    pub fn touched(&self) -> Touched {
        let Change { heights, ground, layers, held, by_hand, drawing, asked, seed, config, model_heights, first_lake_levels, water, notes, meta, reach_m } = self;
        Touched {
            world: !heights.is_empty()
                || !ground.is_empty()
                || layers.iter().any(|(_, cells)| !cells.is_empty())
                || water.is_some()
                || notes.is_some()
                || meta.is_some(),
            held: !held.is_empty(),
            by_hand: !by_hand.is_empty(),
            drawings: drawing.is_some() || asked.is_some(),
            rest: seed.is_some() || config.is_some() || model_heights.is_some() || first_lake_levels.is_some() || reach_m.is_some(),
        }
    }

    /// Exchanges the change's values with `land`'s: swapped into one of the two lands it lies
    /// between, it makes it the other, bit for bit (its file byte for byte), and then holds the
    /// first land's values, so the next swap gives that land back. Undo and redo are this one call.
    /// It runs on the calling thread alone and uses no thread pool (stage 2's ruling R3, N1:
    /// 16 ms for the whole land at 2 km and 2 to 4 ms for edit `a` in a release build, where a pool
    /// gained little), so the extension calls it on the main thread directly, never waiting behind
    /// a job. A change is swapped only into one of its two lands; `land` is checked first to be of
    /// their shape (the lengths of its grids and marks, its layers' names), and one that is not
    /// makes it panic before anything is exchanged. A land of that shape that is neither of the two
    /// is the caller's to keep from it, since only the caller knows which state a land is in.
    pub fn swap(&mut self, land: &mut Land) {
        assert!(self.fits(land), "a step of undo is swapped only into one of the two lands it lies between, and this land is of another shape");
        let Change { heights, ground, layers, held, by_hand, drawing, asked, seed, config, model_heights, first_lake_levels, water, notes, meta, reach_m } = self;
        let Land {
            drawing: land_drawing,
            asked: land_asked,
            seed: land_seed,
            config: land_config,
            model_heights: land_model_heights,
            first_lake_levels: land_first_lake_levels,
            world,
            marks,
            reach_m: land_reach_m,
        } = land;
        let World { heights: world_heights, layers: world_layers, ground: world_ground, meta: world_meta, notes: world_notes, notes_about, water: world_water } = world;
        let Marks { held: land_held, by_hand: land_by_hand } = marks;
        heights.swap(&mut world_heights.data);
        ground.swap(&mut world_ground.data);
        for ((_, cells), layer) in layers.iter_mut().zip(world_layers.values_mut()) {
            cells.swap(&mut layer.data);
        }
        held.swap(land_held);
        by_hand.swap(land_by_hand);
        exchange(drawing, land_drawing);
        exchange(asked, land_asked);
        exchange(seed, land_seed);
        exchange(config, land_config);
        exchange(model_heights, land_model_heights);
        exchange(first_lake_levels, land_first_lake_levels);
        exchange(water, world_water);
        if let Some((held_notes, held_about)) = notes {
            swap(held_notes, world_notes);
            swap(held_about, notes_about);
        }
        exchange(meta, world_meta);
        exchange(reach_m, land_reach_m);
    }

    /// Whether the two lands are one, bit for bit: no cell and no part differs.
    pub fn is_empty(&self) -> bool {
        let Change { heights, ground, layers, held, by_hand, drawing, asked, seed, config, model_heights, first_lake_levels, water, notes, meta, reach_m } = self;
        heights.is_empty()
            && ground.is_empty()
            && layers.iter().all(|(_, cells)| cells.is_empty())
            && held.is_empty()
            && by_hand.is_empty()
            && drawing.is_none()
            && asked.is_none()
            && seed.is_none()
            && config.is_none()
            && model_heights.is_none()
            && first_lake_levels.is_none()
            && water.is_none()
            && notes.is_none()
            && meta.is_none()
            && reach_m.is_none()
    }

    /// What the change holds in memory, in bytes: its own size, every vector's length times its
    /// element's size (the runs, the values, the layers' names, and within each part held whole its
    /// picture, rivers, points, lakes, courses, notes and text).
    pub fn bytes(&self) -> usize {
        let Change { heights, ground, layers, held, by_hand, drawing, asked, seed, config, model_heights, first_lake_levels, water, notes, meta, reach_m } = self;
        let layer_bytes: usize = layers.iter().map(|(name, cells)| name.len() + cells.bytes()).sum();
        size_of::<Change>()
            + heights.bytes()
            + ground.bytes()
            + layers.len() * size_of::<(String, Cells<f32>)>()
            + layer_bytes
            + held.bytes()
            + by_hand.bytes()
            + heap(drawing)
            + heap(asked)
            + heap(seed)
            + heap(config)
            + heap(model_heights)
            + heap(first_lake_levels)
            + heap(water)
            + heap(notes)
            + heap(meta)
            + heap(reach_m)
    }

    /// Whether `land` is of the shape of the two lands the change lies between, so that `swap` can
    /// exchange its runs with it: each grid and mark held by runs of the length the two had, and the
    /// same layers by name.
    fn fits(&self, land: &Land) -> bool {
        let world = &land.world;
        self.heights.fits(&world.heights.data)
            && self.ground.fits(&world.ground.data)
            && self.layers.len() == world.layers.len()
            && (self.layers.iter().zip(&world.layers)).all(|((name, cells), (land_name, layer))| name == land_name && cells.fits(&layer.data))
            && self.held.fits(&land.marks.held)
            && self.by_hand.fits(&land.marks.by_hand)
    }
}

/// Equal when the two hold the same runs with the same values, bit for bit, and the same parts
/// whole, compared by the bytes the land file holds each by: so a change worked out on one thread
/// equals the same change worked out on eight.
impl PartialEq for Change {
    fn eq(&self, other: &Self) -> bool {
        let Change { heights, ground, layers, held, by_hand, drawing, asked, seed, config, model_heights, first_lake_levels, water, notes, meta, reach_m } = self;
        heights.same(&other.heights)
            && ground.same(&other.ground)
            && layers.len() == other.layers.len()
            && (layers.iter().zip(&other.layers)).all(|((name, cells), (other_name, other_cells))| name == other_name && cells.same(other_cells))
            && held.same(&other.held)
            && by_hand.same(&other.by_hand)
            && same_part(drawing, &other.drawing)
            && same_part(asked, &other.asked)
            && same_part(seed, &other.seed)
            && same_part(config, &other.config)
            && same_part(model_heights, &other.model_heights)
            && same_part(first_lake_levels, &other.first_lake_levels)
            && same_part(water, &other.water)
            && same_part(notes, &other.notes)
            && same_part(meta, &other.meta)
            && same_part(reach_m, &other.reach_m)
    }
}

/// A grid's sides, its cell size by its bits and its count of cells: two grids of one frame are
/// diffed cell by cell.
fn frame<T>(grid: &Grid<T>) -> (usize, usize, u32, usize) {
    let Grid { width, height, cell_size, data } = grid;
    (*width, *height, cell_size.to_bits(), data.len())
}

/// A cell of a grid or a mark, compared bit for bit.
trait Cell: Copy + Send + Sync {
    fn same(self, other: Self) -> bool;
}

impl Cell for f32 {
    fn same(self, other: Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl Cell for GroundCell {
    fn same(self, other: Self) -> bool {
        self == other
    }
}

impl Cell for bool {
    fn same(self, other: Self) -> bool {
        self == other
    }
}

/// The cells of one list (a grid's values, or a mark a cell) in which two lands differ.
#[derive(Debug, Clone)]
enum Cells<T> {
    /// Where the two lists are of one length: the runs in cell order, none crossing a row, and their
    /// values, the other land's, one run after another.
    Runs { len: usize, runs: Vec<Run>, values: Vec<T> },
    /// Where they are not: the other land's list whole.
    Whole(Vec<T>),
}

impl<T: Cell> Cells<T> {
    /// No cell of a list of `len` cells: what `between` finds of two equal lists.
    fn none(len: usize) -> Cells<T> {
        Cells::Runs { len, runs: Vec::new(), values: Vec::new() }
    }

    /// The cells of `from` that differ from `to`'s, with `to`'s values, found row by row on the
    /// pool's threads, `row` cells a row; `to` whole where the two are not of one length.
    fn between(from: &[T], to: &[T], row: usize) -> Cells<T> {
        if from.len() != to.len() {
            return Cells::Whole(to.to_vec());
        }
        let row = row.max(1);
        let rows: Vec<(Vec<Run>, Vec<T>)> = (from.par_chunks(row).zip(to.par_chunks(row)).enumerate())
            .map(|(index, (from, to))| row_runs(index * row, from, to))
            .collect();
        let (run_count, value_count) = rows.iter().fold((0, 0), |(runs, values), (row_runs, row_values)| (runs + row_runs.len(), values + row_values.len()));
        let (mut runs, mut values) = (Vec::with_capacity(run_count), Vec::with_capacity(value_count));
        for (row_runs, row_values) in rows {
            runs.extend(row_runs);
            values.extend(row_values);
        }
        Cells::Runs { len: from.len(), runs, values }
    }

    /// Exchanges the held values with `list`'s: each run's with the list's cells it covers, or the
    /// whole list.
    fn swap(&mut self, list: &mut Vec<T>) {
        match self {
            Cells::Runs { len: _, runs, values } => {
                let mut at = 0;
                for &(start, count) in runs.iter() {
                    list[start..start + count].swap_with_slice(&mut values[at..at + count]);
                    at += count;
                }
            }
            Cells::Whole(whole) => swap(whole, list),
        }
    }

    /// Whether `list` can be swapped with: a list of the length the runs were found in, or any list
    /// where the change holds one whole.
    fn fits(&self, list: &[T]) -> bool {
        match self {
            Cells::Runs { len, runs: _, values: _ } => list.len() == *len,
            Cells::Whole(_) => true,
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Cells::Runs { len: _, runs, values: _ } if runs.is_empty())
    }

    /// The runs' and the values' bytes, or the whole list's.
    fn bytes(&self) -> usize {
        match self {
            Cells::Runs { len: _, runs, values } => runs.len() * size_of::<Run>() + values.len() * size_of::<T>(),
            Cells::Whole(whole) => whole.len() * size_of::<T>(),
        }
    }

    /// Whether the two hold the same runs and values, or the same whole list, bit for bit.
    fn same(&self, other: &Self) -> bool {
        let same_values = |a: &[T], b: &[T]| a.len() == b.len() && a.iter().zip(b).all(|(&a, &b)| a.same(b));
        match (self, other) {
            (Cells::Runs { len, runs, values }, Cells::Runs { len: other_len, runs: other_runs, values: other_values }) => {
                len == other_len && runs == other_runs && same_values(values, other_values)
            }
            (Cells::Whole(whole), Cells::Whole(other_whole)) => same_values(whole, other_whole),
            _ => false,
        }
    }
}

/// The runs of one row, `first` the index of its first cell, with `to`'s values: each from a cell
/// that differs to the last that differs before a gap of more equal cells than a run's cost in
/// values (`RUN_BYTES`).
fn row_runs<T: Cell>(first: usize, from: &[T], to: &[T]) -> (Vec<Run>, Vec<T>) {
    let gap_most = RUN_BYTES / size_of::<T>();
    let next_differing = |from_cell: usize, before: usize| (from[from_cell..before].iter().zip(&to[from_cell..before])).position(|(&a, &b)| !a.same(b)).map(|at| from_cell + at);
    let (mut runs, mut values) = (Vec::new(), Vec::new());
    let mut cell = 0;
    while let Some(start) = next_differing(cell, from.len()) {
        let mut end = start + 1;
        while let Some(next) = next_differing(end, (end + gap_most + 1).min(from.len())) {
            end = next + 1;
        }
        runs.push((first + start, end - start));
        values.extend_from_slice(&to[start..end]);
        cell = end;
    }
    (runs, values)
}

/// A part of a land a change holds whole where it differs.
trait Part: Clone {
    /// The bytes the land file holds the part by: two parts are one, bit for bit, when these are.
    fn bits(&self) -> Vec<u8>;
    /// What the part holds beyond its own size: its vectors' lengths times their elements' sizes.
    fn heap_bytes(&self) -> usize;
}

impl Part for Drawing {
    fn bits(&self) -> Vec<u8> {
        drawing_bytes(self)
    }

    fn heap_bytes(&self) -> usize {
        let Drawing { size_m: _, high_ground, rivers, lakes, country: _ } = self;
        let picture = high_ground.as_ref().map_or(0, |picture| picture.data.len() * size_of::<f32>());
        let points: usize = rivers.iter().map(|River { points, width_m: _ }| points.len() * size_of::<(f64, f64)>()).sum();
        picture + rivers.len() * size_of::<River>() + points + lakes.len() * size_of::<Lake>()
    }
}

impl Part for u64 {
    fn bits(&self) -> Vec<u8> {
        self.to_le_bytes().to_vec()
    }

    fn heap_bytes(&self) -> usize {
        0
    }
}

impl Part for f32 {
    fn bits(&self) -> Vec<u8> {
        self.to_bits().to_le_bytes().to_vec()
    }

    fn heap_bytes(&self) -> usize {
        0
    }
}

impl Part for GenConfig {
    fn bits(&self) -> Vec<u8> {
        settings_bytes(self)
    }

    /// Nothing: the settings are numbers and records of numbers, taken apart so a field added that
    /// holds a vector fails to compile until it is counted here.
    fn heap_bytes(&self) -> usize {
        let GenConfig { size_cells: _, cell_size_m: _, detail_amplitude_m: _, pick: _, rules: _, water: _ } = self;
        0
    }
}

/// The land model's heights.
impl Part for Grid<f32> {
    fn bits(&self) -> Vec<u8> {
        model_bytes(self)
    }

    fn heap_bytes(&self) -> usize {
        let Grid { width: _, height: _, cell_size: _, data } = self;
        data.len() * size_of::<f32>()
    }
}

impl Part for Vec<GrownLake> {
    fn bits(&self) -> Vec<u8> {
        first_lakes_bytes(self)
    }

    fn heap_bytes(&self) -> usize {
        self.iter().map(|GrownLake { drawn: _, level_m: _ }| size_of::<GrownLake>()).sum()
    }
}

impl Part for Water {
    fn bits(&self) -> Vec<u8> {
        water_bytes(self)
    }

    fn heap_bytes(&self) -> usize {
        let Water { rivers, lakes } = self;
        let points: usize = rivers.iter().map(|RiverCourse { drawn: _, from_lake: _, points, end: _ }| points.len() * size_of::<CoursePoint>()).sum();
        rivers.len() * size_of::<RiverCourse>() + points + lakes.len() * size_of::<LakeWater>()
    }
}

impl Part for Notes {
    fn bits(&self) -> Vec<u8> {
        notes_bytes(&self.0, &self.1)
    }

    fn heap_bytes(&self) -> usize {
        let (notes, notes_about) = self;
        notes.len() * size_of::<String>() + notes.iter().map(String::len).sum::<usize>() + notes_about.len() * size_of::<About>()
    }
}

impl Part for WorldMeta {
    fn bits(&self) -> Vec<u8> {
        meta_bytes(self)
    }

    fn heap_bytes(&self) -> usize {
        let WorldMeta { seed: _, source_id, window: _, base_elevation_m: _, north: _, timings: _ } = self;
        source_id.len()
    }
}

/// `to` where it differs from `from`, bit for bit.
fn differing<T: Part>(from: &T, to: &T) -> Option<T> {
    (from.bits() != to.bits()).then(|| to.clone())
}

/// Exchanges a part held whole with the land's.
fn exchange<T>(held: &mut Option<T>, part: &mut T) {
    if let Some(held) = held {
        swap(held, part);
    }
}

fn heap<T: Part>(part: &Option<T>) -> usize {
    part.as_ref().map_or(0, Part::heap_bytes)
}

/// Whether two parts held whole, or not held, are one, bit for bit.
fn same_part<T: Part>(a: &Option<T>, b: &Option<T>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.bits() == b.bits(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The runs of `from` against `to`, one row of `row` cells at a time.
    fn runs_of<T: Cell>(from: &[T], to: &[T], row: usize) -> Cells<T> {
        Cells::between(from, to, row)
    }

    /// A run keeps a gap of equal cells that costs no more than a run (four `f32`s, sixteen flags),
    /// and starts again after a longer one; no run crosses a row; and the values are the other
    /// list's, bit for bit, a 0 written -0 and a NaN of other bits among them, which swapped in give
    /// it and swapped again give the first list back.
    #[test]
    fn runs_keep_short_gaps_stay_in_their_rows_and_swap_back_bit_for_bit() {
        let from = vec![0.0f32; 24];
        let mut to = from.clone();
        // Row 0 (cells 0 to 11): cells 1, 6 (a 0 written -0) and 11, the gaps of 4 between them kept.
        to[1] = 1.0;
        to[6] = -0.0;
        to[11] = 5.0;
        // Row 1 (cells 12 to 23): a run of its own from cell 12 (a NaN), next to the first row's last
        // cell; then cell 23, after a gap of 9.
        to[12] = f32::from_bits(0x7fc0_0001);
        to[13] = 2.0;
        to[23] = 3.0;
        let mut cells = runs_of(&from, &to, 12);
        assert!(matches!(&cells, Cells::Runs { len: 24, runs, values } if *runs == [(1, 11), (12, 2), (23, 1)] && values.len() == 14), "{cells:?}");
        assert_eq!(cells.bytes(), 3 * RUN_BYTES + 14 * size_of::<f32>());
        let mut list = from.clone();
        cells.swap(&mut list);
        assert!(list.iter().zip(&to).all(|(a, b)| a.to_bits() == b.to_bits()), "swapped in, the other list");
        cells.swap(&mut list);
        assert!(list.iter().zip(&from).all(|(a, b)| a.to_bits() == b.to_bits()), "swapped again, the first");

        let flags = vec![false; 40];
        let mut other = flags.clone();
        other[0] = true;
        other[17] = true;
        other[35] = true;
        let cells = runs_of(&flags, &other, 40);
        assert!(matches!(&cells, Cells::Runs { runs, .. } if *runs == [(0, 18), (35, 1)]), "a gap of 16 flags is kept, of 17 not: {cells:?}");
        assert!(runs_of(&flags, &flags, 40).is_empty());
    }

    /// Lists of two lengths are held whole and swapped whole.
    #[test]
    fn lists_of_two_lengths_are_held_whole() {
        let (short, long) = (vec![true; 3], vec![false; 5]);
        let mut cells = runs_of(&short, &long, 2);
        assert!(matches!(&cells, Cells::Whole(whole) if *whole == long));
        let mut list = short.clone();
        assert!(cells.fits(&[true; 7]), "any list, held whole");
        cells.swap(&mut list);
        assert_eq!(list, long);
        cells.swap(&mut list);
        assert_eq!(list, short);
    }
}
