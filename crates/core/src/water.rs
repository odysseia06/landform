//! The water of a drawn world: the channels its water follows, traced on the land model's coarse
//! grid (`channel.rs`), their lines smoothed (`smooth.rs`) and their widths made to wander and
//! their deep lines found (`shape.rs`), the drawn lakes' water found on the 1 m ground, settled to
//! the level that ground holds and their shores graded (`lake.rs`), the channels' beds cut into
//! the 1 m ground (`bed.rs`), the water beyond the beds bounded by the ground (`bound_spread`),
//! and how wet the water makes the ground beside it (`bank.rs`).
//! `add_water` runs them in that order and gives the world its rivers and lakes as plain data. The lakes and the banks
//! share one spread from source cells (`spread.rs`), which also gives, before the fine shapes are
//! added, how far the detail's gullies are faded out near the lakes (`gully_factor`).

mod bank;
mod bed;
mod channel;
mod lake;
mod shape;
mod smooth;
mod spread;

use crate::drawing::Drawing;
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{Grown, Note};
use crate::water_rules::WaterRules;
use crate::world::check_cancel;
use bed::{above_floor_notes, cut_beds, profiles, Profile, Worked};
use channel::{line_notes, trace_channels, Channel, ChannelEnd, ChannelPoint};
use lake::{fill_lakes, LakeShape};
use shape::{shape_channels, Shape};
use spread::Spread;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::AtomicBool;

/// How far a bank reaches beyond its bed's edge and the least half width of a bed, which the
/// measure of water the ground does not hold (`held.rs`) and an edit's joined water read.
pub(crate) use bed::{BANK_REACH_M, HALF_WIDTH_MIN_M};

/// How wet the water makes the ground beside it, which an edit's joined water takes as the floor of
/// its band's wetness (`land::water::join_water`), as the cover does of every drawn world's.
pub(crate) use bank::bank_wetness;

/// Where a drawn lake's water may lie on the 1 m ground (within `LAKE_REACH_RADII` of its radii of
/// its centre and `POCKET_MARGIN_M` beyond, on the cells where the land model holds it at its level),
/// which an edit's lake zones read (`land::outline::lake_zones`).
pub(crate) use lake::{model_lake_cells, LAKE_REACH_RADII, POCKET_MARGIN_M};

/// How far beyond it a bank's foot is read, and how near the world's border a channel comes in
/// across it, for the world's tests.
#[cfg(test)]
pub(crate) use bed::{BANK_FOOT_M, BORDER_START_M};

/// How far below the river a tributary may arrive before the river comes down to it, and how many
/// passes the junctions may take, for the world's tests.
#[cfg(test)]
pub(crate) use bed::{ARRIVAL_ALLOWANCE_M, PASSES_MOST};

/// How near two drawn lakes' model levels lie when they stand at one level, and how far a lake's
/// scan steps down at a time, for the world's tests.
#[cfg(test)]
pub(crate) use lake::{AT_LEVEL_M, SETTLE_STEP_M};

/// In a drawn world the fine shapes cut no gully within this many metres of where the land model
/// holds a lake's water (`gully_factor`), so no gully runs through a lake's rim and lets its water
/// out below its level...
pub(crate) const GULLY_CLEAR_M: f64 = 8.0;
/// ...and the gullies come back to their whole cut over this many metres beyond that.
pub(crate) const GULLY_FADE_M: f64 = 16.0;

/// A kept cell whose water stands more than this above its kept level, in metres, holds water
/// spread over it from elsewhere, and comes down to its kept level (`bound_spread`); one less high
/// keeps what it holds. A spring's head holds water a hair deep whose kept level single precision
/// reads at its ground, which would go dry without the margin.
const KEPT_MARGIN_M: f32 = 1e-4;

/// The name of a drawn world's layer of the water's depth, in metres, 0 on land.
pub const WATER_LAYER: &str = "water";

/// A point of a river's or a brook's course.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoursePoint {
    /// World `x`, in metres.
    pub x_m: f32,
    /// World `z`, in metres.
    pub z_m: f32,
    /// The water's surface, in the world's frame, in metres; it never rises downstream.
    pub surface_m: f32,
    /// The water's width, in metres: on the 1 m ground the bed is cut this wide, from waterline to
    /// waterline, and at least about 1.41 m (half a cell's diagonal either side of the line). It
    /// wanders along the course, wide stretches and narrow ones in turn
    /// (`WaterRules::width_wander`). Where the foot of its banks (the cells just beyond the
    /// waterline nearest that point, on both sides; a cell under a lake's water, or under the water
    /// in the bed of a course worked out before it other than the one it ends on, counted at that
    /// water's surface) falls below the line's ground there, the surface lies at least 0.1 m below
    /// every cell of the foot, unless a later step raised it: water backing up behind a lake, a
    /// tributary raised to meet the course it ends on (as that course comes down to it, by up to 3 m
    /// and never below a lake it ends in, no more than 0.3 m above what its banks hold once a pass
    /// of the junctions asks for no new cap; more where the 3 m or a lake stop it, and where the
    /// three passes run out with a cap still asked for, by what the last left short), or a course
    /// out of a lake held at the lake's level beside it, or by the lake's grade past it where its
    /// banks hold. Elsewhere that need
    /// not hold: at a spring's head on level ground the water starts level with the ground, and the
    /// foot is read as the channels worked out before cut it, so where one worked out later cuts it
    /// lower (a tributary's mouth in its river's bank) the surface may stand above it. The water
    /// reaches beyond its waterlines over ground below the level it carries, joined to the bed,
    /// chiefly where its surface was raised, and stands there only where the ground closes it in, no
    /// higher than the rim it would cross to get out. The world's `water` layer shows where it lies.
    pub width_m: f32,
    /// The water's depth at the deep line, in metres: `WaterRules::water_fill` times the bed's
    /// depth there. The bed's depth is `WaterRules::bed_depth_share` of the width before the
    /// wander, divided by how much the wander widened it, so a wide stretch is shallow and a narrow
    /// one deep, kept within the rules' bounds, and grown from 0 over a spring's first 12 m. The
    /// deep line lies off the course's line toward the outside of a bend
    /// (`WaterRules::bend_shift`), and the water shallows from it to nothing at both waterlines.
    pub depth_m: f32,
}

/// How a course ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CourseEnd {
    /// At the world's border.
    Edge,
    /// On the course of this index in `Water::rivers`: on one of its points, or, for a course into
    /// a drawn lake that holds no water, on the course flowing out of that lake.
    River(usize),
    /// In the lake of this index in `Water::lakes`.
    Lake(usize),
}

/// A river or a brook, as the line its water follows.
#[derive(Debug, Clone, PartialEq)]
pub struct RiverCourse {
    /// The index in `Drawing::rivers` of the drawn river it is part of, or `None` for a brook. A
    /// drawn river that crosses a lake is two courses, one ending in the lake and one flowing out
    /// of it.
    pub drawn: Option<usize>,
    /// The index in `Water::lakes` of the lake it flows out of.
    pub from_lake: Option<usize>,
    /// Upstream first, at most 4 m apart, on the world's cells: from `-size / 2` to `size / 2 - 1`
    /// metres along both axes.
    pub points: Vec<CoursePoint>,
    pub end: CourseEnd,
}

/// A drawn lake that holds water.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeWater {
    /// Its index in `Drawing::lakes`.
    pub drawn: usize,
    /// Its level, in the world's frame, in metres.
    pub level_m: f32,
    /// World `x` of the mean position of its water's cells, in metres.
    pub x_m: f32,
    /// World `z` of the mean position of its water's cells, in metres.
    pub z_m: f32,
    /// Its water's area, in square metres.
    pub area_m2: f32,
}

/// A world's rivers, brooks and lakes; empty for a world cut from a source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Water {
    /// The drawn rivers' courses first, in the drawing's order and each river's from upstream,
    /// then the brooks, those draining most where they end first.
    pub rivers: Vec<RiverCourse>,
    /// The drawn lakes that hold water, in the drawing's order; a lake left out is not listed.
    pub lakes: Vec<LakeWater>,
}

impl Water {
    /// Moves every surface and level down by `by_m`, as the world's heights are moved so that the
    /// lowest cell is 0.
    pub(crate) fn lower(&mut self, by_m: f32) {
        for point in self.rivers.iter_mut().flat_map(|course| &mut course.points) {
            point.surface_m -= by_m;
        }
        for lake in &mut self.lakes {
            lake.level_m -= by_m;
        }
    }
}

/// What the cover reads of the water: depth per cell, and how wet the bank makes each cell (0 to 1).
pub(crate) struct WaterField {
    /// The water's depth over the ground, in metres; 0 on land.
    pub depth_m: Grid<f32>,
    /// How wet the water makes each cell, 0 to 1 (`bank::bank_wetness`).
    pub bank_wet: Grid<f32>,
}

/// Channels, then lakes, then beds, then the bank wetness, on the heights after the fine shapes.
///
/// `heights` is the world's grid of 1 m cells, cell `(i, j)` at world `(i - width / 2, j - height
/// / 2)`, in the land model's frame (the world's shift to a lowest cell of 0 comes later), and
/// `grown` the land grown from `drawing`, from which the heights were resampled; `seed` is the
/// world's. First the channels are traced (`trace_channels`) and shaped, each one's width made to
/// wander and its deep line found (`shape_channels`, from `seed`): they read only the land model,
/// so where they run and how wide they are does not hang on the lakes. Then the drawn lakes' water
/// is found, each lake settled to the level its 1 m ground holds, knowing the beds of the channels
/// flowing out of it, and their shores graded (`fill_lakes`, which adds its notes to `notes` and
/// reads the flag at each level it looks at). Every note is written with what it is about, where
/// the river or lake it says something of is known (`Note`). Then a note for each drawn river that leaves its line
/// or does not reach its drawn end (`line_notes`); the channels' water is worked out with each
/// lake's level (the settled one), the floor under each point and the lakes found, their cells
/// ground at their lake's level for the foot of a bank, a course out of a lake holding its level
/// along its stretch beside it, the lakes' rims held, and a river brought down to a tributary its
/// banks hold below it, in passes between which the flag is read (`profiles`, `floor_m`), with
/// notes for the channels whose water stands far above their floor (`above_floor_notes`) and on the
/// junctions that brought a river down, or left a tributary standing above its banks, by more than
/// a metre (`profiles`).
///
/// Then the beds and banks are cut, the lakes' rims held against them as `profiles` held them
/// (`cut_beds`), the beds' water worked out alone, each channel's filled out from its bed but for
/// the cells inside what it ends on: the bed of the channel, or the cells of the lake, whose water
/// is that one's; a channel that ends in a lake left out marks its bed there as anywhere. Then the
/// lakes' water is put back over the ground as cut (`put_back_lakes`): every cell each lake settled
/// with holds water to its level, or the beds' water where that is deeper (an outflow's: a channel
/// ending in the lake marks none of its cells); so a lake cell a bank lowered near the mouth of a
/// channel flowing in is deepened by as much. No course takes a lake's water off a cell: a course
/// out of a lake stands at its level beside it. Then the water beyond the beds is bounded by the
/// ground (`bound_spread`): it stands only where the ground closes it in, no higher than the rim it
/// would cross to get out, a bed's cells under another channel's water come down to their own
/// surface, and water that no path across cells' sides through water joins to a bed or a lake is
/// dropped (`beds_and_lakes`). Last, how wet the water makes each cell (`bank_wetness`). The flag
/// is read after the lakes, between the passes of the water's surfaces and after them, and after
/// the beds.
///
/// The courses are the channels in their order, so a channel's index is its course's; a course's
/// lake is an index into `Water::lakes`, the lakes found in the drawing's order (`courses`). A
/// course that ends in a lake left out ends on the first course flowing out of that lake, or at the
/// edge when none does, and one flowing out of it flows out of no lake. Each point's surface is its
/// profile's, still in the land model's frame; its width is its channel's after the wander, and
/// its depth the water's at the deep line, `water_fill` times the profile's depth of the bed.
pub(crate) fn add_water(
    heights: &mut Grid<f32>,
    grown: &Grown,
    drawing: &Drawing,
    rules: &WaterRules,
    seed: u64,
    notes: &mut Vec<Note>,
    cancel: &AtomicBool,
) -> Result<(Water, WaterField), LandformError> {
    // The channels read only the land model, so they are traced and shaped first, and each lake
    // settles knowing the courses out of it.
    let mut channels = trace_channels(grown, drawing, rules);
    let shapes = shape_channels(&mut channels, seed, rules);
    let mut depth = Grid::new(heights.width, heights.height, heights.cell_size);
    let lakes = fill_lakes(heights, &mut depth, grown, drawing, &channels, rules, notes, cancel)?;
    check_cancel(cancel)?;

    // The lakes' depth is cleared, so the beds' water is worked out alone, and put back after.
    for &cell in lakes.iter().flat_map(|lake| &lake.cells) {
        depth.data[cell] = 0.0;
    }
    notes.extend(line_notes(&channels, grown, drawing));
    let levels: Vec<f32> = (0..drawing.lakes.len())
        .map(|drawn| lakes.iter().find(|lake| lake.drawn == drawn).map_or(f32::NAN, |lake| lake.level_m))
        .collect();
    let size = (heights.width, heights.height);
    let floor = |point: &ChannelPoint| floor_m(grown, size, (point.x_m, point.z_m));
    let Worked { profiles: found, notes: junction_notes, .. } = profiles(&channels, &shapes, heights, &floor, &lakes, &levels, rules, cancel)?;
    notes.extend(above_floor_notes(&channels, &found));
    notes.extend(junction_notes);
    check_cancel(cancel)?;

    beds_and_lakes(heights, &mut depth, &lakes, &channels, &shapes, &found, rules);
    check_cancel(cancel)?;

    let bank_wet = bank_wetness(heights, &depth, rules);
    let water = Water {
        rivers: courses(&channels, &found, &lakes, rules),
        lakes: (lakes.iter())
            .map(|lake| LakeWater {
                drawn: lake.drawn,
                level_m: lake.level_m,
                x_m: lake.x_m,
                z_m: lake.z_m,
                area_m2: lake.area_m2,
            })
            .collect(),
    };
    Ok((water, WaterField { depth_m: depth, bank_wet }))
}

/// The drawn lake, the first of `lakes`, whose water cell `cell` holds, or `None` when it holds
/// none.
fn lake_of_cell(lakes: &[LakeShape], cell: usize) -> Option<usize> {
    lakes.iter().find(|lake| lake.cells.binary_search(&cell).is_ok()).map(|lake| lake.drawn)
}

/// The level of the first of `lakes` whose water cell `cell` holds, or `None` when it holds none.
fn lake_level_at(lakes: &[LakeShape], cell: usize) -> Option<f32> {
    lakes.iter().find(|lake| lake.cells.binary_search(&cell).is_ok()).map(|lake| lake.level_m)
}

/// Cuts the beds of `channels` (shaped by `shapes`, their water `profiles`) into `heights` and puts
/// their water and the water of `lakes` into `depth`, as `add_water` describes it: `lakes` are the
/// lakes `fill_lakes` found, their water cleared from `depth`. The beds are cut beside the lakes,
/// their rims held (`cut_beds`), so a channel that ends in a lake left out marks its bed there as
/// anywhere; then the lakes' water is put back (`put_back_lakes`), and last the water beyond the
/// beds is bounded by the ground (`bound_spread`).
fn beds_and_lakes(
    heights: &mut Grid<f32>,
    depth: &mut Grid<f32>,
    lakes: &[LakeShape],
    channels: &[Channel],
    shapes: &[Shape],
    profiles: &[Profile],
    rules: &WaterRules,
) {
    let kept_level = unbounded_beds_and_lakes(heights, depth, lakes, channels, shapes, profiles, rules);
    bound_spread(heights, depth, &kept_level);
}

/// `beds_and_lakes` before the spread is bounded: the beds cut and their water filled out from
/// them (`cut_beds`), the lakes' water put back (`put_back_lakes`); returns each cell's kept level,
/// the higher of the highest surface of the channels whose water starts in it and the level of the
/// lake that holds it, NEG_INFINITY where neither does.
fn unbounded_beds_and_lakes(
    heights: &mut Grid<f32>,
    depth: &mut Grid<f32>,
    lakes: &[LakeShape],
    channels: &[Channel],
    shapes: &[Shape],
    profiles: &[Profile],
    rules: &WaterRules,
) -> Vec<f32> {
    let mut kept_level = cut_beds(heights, depth, channels, shapes, profiles, lakes, rules);
    put_back_lakes(lakes, heights, depth);
    for lake in lakes {
        for &cell in &lake.cells {
            kept_level[cell] = kept_level[cell].max(lake.level_m);
        }
    }
    kept_level
}

/// Bounds the water beyond the beds by the ground, last in `beds_and_lakes`: `heights` is the
/// ground as the beds cut it, `depth` the water over it, the beds' filled out from them and the
/// lakes', and `kept_level` each cell's kept level (`unbounded_beds_and_lakes`), NEG_INFINITY where
/// it has none.
///
/// A wet cell with a kept level is kept; every other wet cell holds spread water. Each spread cell's
/// rim is found by a lowest-rim flood over the spread cells, across the eight neighbours, from
/// outside in. Its ways out are a dry cell beside it, at that cell's ground; a kept cell beside it,
/// at its kept level, not its water's surface, so a bed under another channel's sheet is a way out
/// at its own level; and, for a cell on the world's border, the border, below everything. Its rim is
/// the higher of its own ground and the lowest of those. The cells are taken lowest rim first, ties
/// by index, and each passes on to the spread cells beside it not yet taken the higher of their
/// ground and its rim, which becomes theirs where it is lower. A spread cell's water then stands at
/// the lower of what it had and its rim, and the cell is dry where that lies at or below its ground,
/// so also where its rim is its ground, however thin its water (a depth below single precision's
/// step at its height reads as the ground itself); a kept cell whose water stands more than
/// `KEPT_MARGIN_M` above its kept level comes down to it, so the water another channel carried over
/// a bed runs off along it, while one within the margin, as a spring's head a hair deep, keeps it.
/// Last, the water of every cell that no path across cells' sides through water joins to a kept
/// cell is dropped. So beyond its bed water stands only where the ground closes it in, no higher
/// than the rim it would cross to get out: on a slope, or beyond a course's last point at the
/// border, it is not there, and a hollow stands level. Every other cell keeps its depth bit for
/// bit.
///
/// The pass reads and writes only the wet cells (`WetCells`), and takes them in an order that
/// depends only on the grid, sequentially. An edit's joined water bounds the band's water by it
/// (`land::water::join_water`).
pub(crate) fn bound_spread(heights: &Grid<f32>, depth: &mut Grid<f32>, kept_level: &[f32]) {
    debug_assert_eq!((heights.width, heights.height), (depth.width, depth.height), "the depth lies on the heights' cells");
    debug_assert_eq!(kept_level.len(), heights.data.len(), "one kept level per cell");
    let wet = WetCells::of(depth);
    let (width, height) = (heights.width, heights.height);
    let ground_m = |cell: usize| heights.data[cell];
    let kept = |place: usize| kept_level[wet.cells[place]] > f32::NEG_INFINITY;
    let count = wet.cells.len();
    // Each spread cell's rim: first the lowest way out beside it, then what the flood brings in.
    let mut rim_m = vec![f32::INFINITY; count];
    let mut waiting = BinaryHeap::new();
    for place in (0..count).filter(|&place| !kept(place)) {
        let cell = wet.cells[place];
        let (i, j) = (cell % width, cell / width);
        let on_border = i == 0 || j == 0 || i + 1 == width || j + 1 == height;
        let mut out_m = if on_border { f32::NEG_INFINITY } else { f32::INFINITY };
        wet.around(place, |next, found| match found {
            None => out_m = out_m.min(ground_m(next)),
            Some(other) if kept(other) => out_m = out_m.min(kept_level[next]),
            Some(_) => {}
        });
        if out_m < f32::INFINITY {
            rim_m[place] = out_m.max(ground_m(cell));
            waiting.push(Rim { rim_m: rim_m[place], place });
        }
    }
    let mut taken = vec![false; count];
    while let Some(Rim { rim_m: reached_m, place }) = waiting.pop() {
        if taken[place] {
            continue;
        }
        taken[place] = true;
        wet.around(place, |next, found| {
            if let Some(other) = found.filter(|&other| !kept(other) && !taken[other]) {
                let through_m = reached_m.max(ground_m(next));
                if through_m < rim_m[other] {
                    rim_m[other] = through_m;
                    waiting.push(Rim { rim_m: through_m, place: other });
                }
            }
        });
    }
    // The spread water at its rim, dry where that lies at or below its ground however thin it is
    // (a depth below single precision's step there reads as the ground itself), and a kept cell's
    // at its kept level.
    for (place, &cell) in wet.cells.iter().enumerate() {
        let surface_m = ground_m(cell) + depth.data[cell];
        let (bound_m, above) = match kept(place) {
            true => (kept_level[cell], surface_m > kept_level[cell] + KEPT_MARGIN_M),
            false => (rim_m[place], surface_m > rim_m[place] || rim_m[place] <= ground_m(cell)),
        };
        if above {
            depth.data[cell] = (bound_m - ground_m(cell)).max(0.0);
        }
    }
    // The water joined to a kept cell across the cells' sides; the rest is dropped.
    let mut joined = vec![false; count];
    let mut stack: Vec<usize> = (0..count).filter(|&place| kept(place) && depth.data[wet.cells[place]] > 0.0).collect();
    for &place in &stack {
        joined[place] = true;
    }
    while let Some(place) = stack.pop() {
        wet.beside(place, |next| {
            if !joined[next] && depth.data[wet.cells[next]] > 0.0 {
                joined[next] = true;
                stack.push(next);
            }
        });
    }
    for (place, &cell) in wet.cells.iter().enumerate() {
        if !joined[place] {
            depth.data[cell] = 0.0;
        }
    }
}

/// A spread cell waiting in `bound_spread`'s flood, by its place among the wet cells, and the rim
/// it was reached at, in metres: ordered so that the lowest rim is taken first, and of equal rims
/// the lowest place, which is the lowest index.
#[derive(Debug, Clone, Copy)]
struct Rim {
    rim_m: f32,
    place: usize,
}

impl Ord for Rim {
    fn cmp(&self, other: &Self) -> Ordering {
        other.rim_m.total_cmp(&self.rim_m).then(other.place.cmp(&self.place))
    }
}

impl PartialOrd for Rim {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Rim {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Rim {}

/// The wet cells of a grid, as `bound_spread` reads them without a grid of its own: their indices
/// in order, each cell's place among them standing for it, and where each row's begin.
struct WetCells {
    width: usize,
    height: usize,
    /// The cells that hold water (a depth above 0), in index order.
    cells: Vec<usize>,
    /// The place among `cells` of each row's first wet cell, and `cells.len()` after the last
    /// row: row `j`'s wet cells are `cells[rows[j]..rows[j + 1]]`.
    rows: Vec<usize>,
}

impl WetCells {
    /// The wet cells of `depth`.
    fn of(depth: &Grid<f32>) -> WetCells {
        let cells: Vec<usize> = (depth.data.iter().enumerate()).filter(|(_, &depth_m)| depth_m > 0.0).map(|(cell, _)| cell).collect();
        let rows = (0..=depth.height).map(|row| cells.partition_point(|&cell| cell < row * depth.width)).collect();
        WetCells { width: depth.width, height: depth.height, cells, rows }
    }

    /// Calls `visit` with each of the eight cells around the wet cell at `place` that lie on the
    /// grid, and its place among the wet cells, `None` for a dry one.
    fn around(&self, place: usize, mut visit: impl FnMut(usize, Option<usize>)) {
        let cell = self.cells[place];
        let (i, j) = (cell % self.width, cell / self.width);
        let (first, last) = (i.saturating_sub(1), (i + 1).min(self.width - 1));
        for row in j.saturating_sub(1)..=(j + 1).min(self.height - 1) {
            let (from, to) = (self.rows[row], self.rows[row + 1]);
            // The row's wet cells are in index order, so its three are found walking from the
            // first at or after the leftmost.
            let mut at = from + self.cells[from..to].partition_point(|&other| other < row * self.width + first);
            for next in row * self.width + first..=row * self.width + last {
                let found = at < to && self.cells[at] == next;
                if next != cell {
                    visit(next, found.then_some(at));
                }
                at += usize::from(found);
            }
        }
    }

    /// Calls `visit` with the place of each wet cell beside the wet cell at `place` across its
    /// sides.
    fn beside(&self, place: usize, mut visit: impl FnMut(usize)) {
        let cell = self.cells[place];
        let (i, j) = (cell % self.width, cell / self.width);
        let in_row = |row: usize, cell: usize| {
            let (from, to) = (self.rows[row], self.rows[row + 1]);
            self.cells[from..to].binary_search(&cell).ok().map(|at| from + at)
        };
        if i > 0 && place > 0 && self.cells[place - 1] == cell - 1 {
            visit(place - 1);
        }
        if i + 1 < self.width && self.cells.get(place + 1) == Some(&(cell + 1)) {
            visit(place + 1);
        }
        if let Some(above) = (j > 0).then(|| in_row(j - 1, cell - self.width)).flatten() {
            visit(above);
        }
        if let Some(below) = (j + 1 < self.height).then(|| in_row(j + 1, cell + self.width)).flatten() {
            visit(below);
        }
    }
}

/// Puts the water of `lakes` back into `depth`, which holds the beds' water alone, over `heights`
/// as the beds cut them: each of their cells holds water to its lake's level, or the beds' water
/// where that is deeper.
fn put_back_lakes(lakes: &[LakeShape], heights: &Grid<f32>, depth: &mut Grid<f32>) {
    for lake in lakes {
        for &cell in &lake.cells {
            depth.data[cell] = depth.data[cell].max(lake.level_m - heights.data[cell]);
        }
    }
}

/// The courses of `channels`, whose water is `profiles`, with `lakes` the lakes found, as
/// `add_water` describes them.
fn courses(channels: &[Channel], profiles: &[Profile], lakes: &[LakeShape], rules: &WaterRules) -> Vec<RiverCourse> {
    let lake_of = |drawn: usize| lakes.iter().position(|lake| lake.drawn == drawn);
    // The first other channel flowing out of drawn lake `drawn`.
    let flowing_out_of = |drawn: usize, index: usize| {
        (channels.iter().enumerate()).position(|(other, channel)| other != index && channel.from_lake == Some(drawn))
    };
    (channels.iter().zip(profiles).enumerate())
        .map(|(index, (channel, profile))| {
            let end = match channel.end {
                ChannelEnd::Edge => CourseEnd::Edge,
                ChannelEnd::Channel(joined) => CourseEnd::River(joined),
                ChannelEnd::Lake(drawn) => match (lake_of(drawn), flowing_out_of(drawn, index)) {
                    (Some(lake), _) => CourseEnd::Lake(lake),
                    (None, Some(out)) => CourseEnd::River(out),
                    (None, None) => CourseEnd::Edge,
                },
            };
            let points = (channel.points.iter().zip(&profile.surface_m).zip(&profile.depth_m))
                .map(|((point, &surface_m), &depth_m)| CoursePoint {
                    x_m: point.x_m as f32,
                    z_m: point.z_m as f32,
                    surface_m,
                    width_m: point.width_m,
                    depth_m: rules.water_fill * depth_m,
                })
                .collect();
            RiverCourse { drawn: channel.drawn, from_lake: channel.from_lake.and_then(lake_of), points, end }
        })
        .collect()
}

/// What the gullies the fine shapes cut along the water lines are multiplied by on each cell of a
/// `width` x `height` grid of 1 m cells laid out as `add_water` reads it, in index order, in a world
/// grown from `drawing` as `grown`, before its water is put in: 0 where the land model holds the
/// water of a lake it lists at its level (as `fill_lakes` asks it of a cell before it reads the
/// 1 m ground: within `LAKE_REACH_RADII` of the lake's radii of its centre, the model's water there,
/// read bilinearly, above 0, its flooded height no more than `AT_LEVEL_M` below the level) and
/// within `GULLY_CLEAR_M` of such a cell, rising linearly to 1 over the next `GULLY_FADE_M`, and 1
/// beyond; the distances are those of `Spread`. `None` when no lake the model lists holds water, so
/// the world takes the detail of a world with no lakes. Sequential and deterministic.
pub(crate) fn gully_factor(grown: &Grown, drawing: &Drawing, (width, height): (usize, usize)) -> Option<Vec<f32>> {
    let mut held: Vec<usize> = Vec::new();
    for (drawn, lake) in drawing.lakes.iter().enumerate() {
        let level_m = grown.lakes.iter().find(|listed| listed.drawn == drawn).map(|listed| listed.level_m);
        if let Some(level_m) = level_m.filter(|level| level.is_finite()) {
            held.extend(lake::model_lake_cells(grown, lake, level_m, (width, height)));
        }
    }
    if held.is_empty() {
        return None;
    }
    held.sort_unstable();
    held.dedup();
    // The spread runs over the held cells' bounding box and the reach around it: every cell within
    // reach of a held cell lies there, so the distances are those of a spread over the whole grid.
    let reach_m = GULLY_CLEAR_M + GULLY_FADE_M;
    let beyond = reach_m.ceil() as usize;
    let (columns, rows) = held.iter().fold(((usize::MAX, 0), (usize::MAX, 0)), |(columns, rows), &cell| {
        let (column, row) = (cell % width, cell / width);
        ((columns.0.min(column), columns.1.max(column)), (rows.0.min(row), rows.1.max(row)))
    });
    let (left, top) = (columns.0.saturating_sub(beyond), rows.0.saturating_sub(beyond));
    let (right, bottom) = ((columns.1 + beyond).min(width - 1), (rows.1 + beyond).min(height - 1));
    let (patch_width, patch_height) = (right - left + 1, bottom - top + 1);
    let on_grid = |local: usize| (top + local / patch_width) * width + left + local % patch_width;
    let mut is_held = vec![false; patch_width * patch_height];
    for &cell in &held {
        is_held[(cell / width - top) * patch_width + cell % width - left] = true;
    }
    let spread = Spread::new(patch_width, patch_height, &is_held, reach_m);
    let mut factor = vec![1.0; width * height];
    for local in 0..patch_width * patch_height {
        factor[on_grid(local)] = ((spread.distance_m(local) - GULLY_CLEAR_M) / GULLY_FADE_M).clamp(0.0, 1.0) as f32;
    }
    Some(factor)
}

/// The floor under world `(x, z)`, in metres, in the land model's frame, on a `width` x `height`
/// world laid out as `add_water` reads it: the land model's flooded surface there
/// (`Grown::heights` plus `Grown::water_depth_m`), read bilinearly between the four model cells
/// around it as `fill_lakes` reads the model. It is the valley's floor in a valley and the lake's
/// level in a lake; unlike the 1 m ground, it holds no gully the fine shapes cut along a water line
/// and no lake's bed.
pub(crate) fn floor_m(grown: &Grown, (width, height): (usize, usize), (x_m, z_m): (f64, f64)) -> f32 {
    let at = (x_m + width as f64 / 2.0, z_m + height as f64 / 2.0);
    lake::model_m(grown, at, |cell| lake::flooded_m(grown, cell)) as f32
}

/// The share of a bank's or a shore's cut made `distance_m` out from the water's edge, with
/// `reach_m` (above 0) the cut's reach: 1 at the edge, 0 from `reach_m` on, along a smooth step
/// that is flat at both ends, so the cut leaves no step at its reach.
fn fade(distance_m: f64, reach_m: f64) -> f64 {
    let reach = (distance_m / reach_m).clamp(0.0, 1.0);
    1.0 - reach * reach * (3.0 - 2.0 * reach)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::drawing::Lake;
    use crate::water::channel::ChannelPoint;
    use crate::water::lake::LAKE_CELLS_MIN;
    use crate::grow::tests::{grown, plain};
    use crate::grow::{About, GrownLake, GROW_CELL_M, GROW_MARGIN_M};
    use crate::resample::resample;
    use crate::source::HeightBlock;
    use crate::window::Window;

    /// Grown land `size_m` across at 1 m, resampled as `generate_drawn` resamples it, with no fine
    /// shapes.
    pub(crate) fn at_one_metre(land: &Grown, size_m: u32) -> Grid<f32> {
        let (size, margin_m) = (f64::from(size_m), f64::from(GROW_MARGIN_M));
        let block = HeightBlock { origin_x: -margin_m, origin_y: -margin_m, grid: land.heights.clone() };
        let window =
            Window { source: 0, center_x: size / 2.0, center_y: size / 2.0, rotation_rad: 0.0, mirrored: false, size_m: size };
        resample(&block, &window, size_m as usize, 1.0)
    }

    /// A point of a line of water as the tests read it, in metres: world `x` and `z`, the water's
    /// half width there, no less than `HALF_WIDTH_MIN_M`, and its surface.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct LinePoint {
        pub x_m: f64,
        pub z_m: f64,
        pub half_width_m: f64,
        pub surface_m: f64,
    }

    /// What a cell sees of a line of water (`near_lines`): its distance to the line's nearest
    /// point, and the least and the most half width and surface among the points of the line that
    /// lie no more than the slack farther from it, each linear along its stretch.
    #[derive(Debug, Clone, Copy)]
    pub(crate) struct NearLine {
        pub cell: usize,
        pub distance_m: f64,
        pub half_width_m: (f64, f64),
        pub surface_m: (f64, f64),
    }

    /// For each of `lines`, on a `size` x `size` grid of 1 m cells whose cell `(i, j)` lies at world
    /// `(i - size / 2, j - size / 2)`, the cells that lie no more than `beyond_m` (and the slack
    /// `slack_m`) beyond the least half width at their nearest points of the line, what they see of
    /// it (`NearLine`), in index order. Worked out over every stretch, each looking at every cell as
    /// far from it as the line's widest half width and `beyond_m`, so the nearest point of the line
    /// is found wherever it is.
    pub(crate) fn near_lines(size: usize, lines: &[Vec<LinePoint>], beyond_m: f64, slack_m: f64) -> Vec<Vec<NearLine>> {
        let half = (size / 2) as f64;
        // Each cell's distance to the nearest point of the line at hand, and the cells it was set
        // for.
        let mut nearest_m = vec![f64::INFINITY; size * size];
        let mut touched: Vec<usize> = Vec::new();
        let mut found = Vec::with_capacity(lines.len());
        for line in lines {
            let reach_m = line.iter().map(|point| point.half_width_m).fold(0.0, f64::max) + beyond_m + slack_m;
            // Each stretch's cells within reach, with the distance to its nearest point and the
            // half width and the surface there.
            let each_near = |visit: &mut dyn FnMut(usize, f64, f64, f64)| {
                for pair in line.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let cells = |from: f64, to: f64| {
                        let low = (from.min(to) - reach_m + half).ceil().max(0.0) as usize;
                        low..=((from.max(to) + reach_m + half).floor().max(0.0) as usize).min(size - 1)
                    };
                    for j in cells(a.z_m, b.z_m) {
                        for i in cells(a.x_m, b.x_m) {
                            let (x, z) = (i as f64 - half, j as f64 - half);
                            let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
                            let squared = dx * dx + dz * dz;
                            let t = if squared > 0.0 { (((x - a.x_m) * dx + (z - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
                            let between = |u: f64, v: f64| u + (v - u) * t;
                            let apart_m = (a.x_m + dx * t - x).hypot(a.z_m + dz * t - z);
                            if apart_m <= reach_m {
                                visit(j * size + i, apart_m, between(a.half_width_m, b.half_width_m), between(a.surface_m, b.surface_m));
                            }
                        }
                    }
                }
            };
            each_near(&mut |cell, apart_m, _, _| {
                if nearest_m[cell] == f64::INFINITY {
                    touched.push(cell);
                }
                nearest_m[cell] = nearest_m[cell].min(apart_m);
            });
            // The points as near as the nearest, give or take the slack, with their half widths
            // and surfaces, gathered by cell.
            let mut ties: Vec<(usize, f64, f64)> = Vec::new();
            each_near(&mut |cell, apart_m, half_width_m, surface_m| {
                if apart_m <= nearest_m[cell] + slack_m {
                    ties.push((cell, half_width_m, surface_m));
                }
            });
            ties.sort_unstable_by_key(|&(cell, _, _)| cell);
            let mut near = Vec::new();
            for of_cell in ties.chunk_by(|a, b| a.0 == b.0) {
                let (cell, distance_m) = (of_cell[0].0, nearest_m[of_cell[0].0]);
                let range = |value: fn(&(usize, f64, f64)) -> f64| {
                    of_cell.iter().map(value).fold((f64::INFINITY, f64::NEG_INFINITY), |(least, most), v| (least.min(v), most.max(v)))
                };
                let (half_width_m, surface_m) = (range(|tie| tie.1), range(|tie| tie.2));
                if distance_m - half_width_m.0 <= beyond_m + slack_m {
                    near.push(NearLine { cell, distance_m, half_width_m, surface_m });
                }
            }
            for cell in touched.drain(..) {
                nearest_m[cell] = f64::INFINITY;
            }
            found.push(near);
        }
        found
    }

    /// Whether the land `grown` holds water at `level_m` (in the land model's frame) at cell
    /// `cell` of a `size` x `size` grid of 1 m cells, as `fill_lakes` asks it of a cell before
    /// taking it as a lake's (`lake::model_holds_water_at`).
    pub(crate) fn model_holds_water(grown: &Grown, size: usize, cell: usize, level_m: f32) -> bool {
        lake::model_holds_water_at(grown, ((cell % size) as f64, (cell / size) as f64), level_m)
    }

    /// The cells beside `cell` across its sides on a `size` x `size` grid.
    pub(crate) fn beside(cell: usize, size: usize) -> impl Iterator<Item = usize> {
        let (i, j) = (cell % size, cell / size);
        [(i > 0).then(|| cell - 1), (i + 1 < size).then(|| cell + 1), (j > 0).then(|| cell - size), (j + 1 < size).then(|| cell + size)]
            .into_iter()
            .flatten()
    }

    /// The level one line's water stands at in each of `near`, its cells in index order on a grid
    /// `size` cells a side, NEG_INFINITY where it does not reach: `seed` gives the level of a cell
    /// its water starts in, and `carry` the level a cell is reached at from a cell beside it whose
    /// level is given, or `None` when it is not; a cell keeps the highest level it is reached at.
    /// Worked out by going over the cells again whenever a level rises, first found first, so not
    /// in the order `cut_beds` takes them.
    fn fill(
        near: &[NearLine],
        size: usize,
        seed: impl Fn(&NearLine) -> Option<f64>,
        carry: impl Fn(&NearLine, f64) -> Option<f64>,
    ) -> Vec<f64> {
        let mut level_m: Vec<f64> = near.iter().map(|seen| seed(seen).unwrap_or(f64::NEG_INFINITY)).collect();
        let mut waiting: std::collections::VecDeque<usize> = (0..near.len()).filter(|&k| level_m[k] > f64::NEG_INFINITY).collect();
        while let Some(k) = waiting.pop_front() {
            for next in beside(near[k].cell, size) {
                if let Ok(n) = near.binary_search_by_key(&next, |seen| seen.cell) {
                    if let Some(carried_m) = carry(&near[n], level_m[k]).filter(|&carried_m| carried_m > level_m[n]) {
                        level_m[n] = carried_m;
                        waiting.push_back(n);
                    }
                }
            }
        }
        level_m
    }

    /// The reach of water, as the tests read it: for each cell of `heights` (laid out as
    /// `near_lines` says), the lowest and the highest level the water of `lines` may stand at
    /// there, NEG_INFINITY where no line's water reaches. A line's water fills the cells of its bed
    /// (no farther from its line than its half width at the nearest point) whose ground lies below
    /// its surface there, at that surface, and reaches from each cell it fills the cells beside it
    /// that lie within reach of its banks (no more than `BANK_REACH_M` beyond the half width at
    /// their nearest point), at the lower of that cell's level and the surface at their own nearest
    /// point, where their ground lies below that; a cell keeps the highest level it is reached at
    /// (`reached_by_line`), and holds the highest of the lines that reach it. A line's water holds
    /// no cell of `ends` (one per line: the bed of the line it ends on, the lake it ends in, the
    /// cells behind the head of one that begins on dry land). Distances and heights are read with
    /// `slack_m` of slack: the lowest level counts the lines that reach the cell however they are
    /// read within it, the highest every line that may.
    pub(crate) fn reach_of_water(heights: &Grid<f32>, lines: &[Vec<LinePoint>], ends: &[Ends], slack_m: f64) -> Vec<(f64, f64)> {
        let size = heights.width;
        let mut reached = vec![(f64::NEG_INFINITY, f64::NEG_INFINITY); size * size];
        for (near, ends) in near_lines(size, lines, BANK_REACH_M, slack_m).into_iter().zip(ends) {
            let (surely, maybe) = reached_by_line(heights, &near, ends, slack_m);
            for ((seen, surely_m), maybe_m) in near.iter().zip(surely).zip(maybe) {
                reached[seen.cell] = (reached[seen.cell].0.max(surely_m), reached[seen.cell].1.max(maybe_m));
            }
        }
        reached
    }

    /// Of one line's cells within reach of its banks (`near`, as `near_lines` gives them), the
    /// level its water surely stands at, at least, and the level it may stand at, at most, read
    /// with `slack_m` of slack as `reach_of_water` reads them, NEG_INFINITY where it does not reach
    /// (surely or maybe): the surely reached none that may lie in `ends`, what the line's water
    /// does not hold, the maybe reached none that surely does.
    pub(crate) fn reached_by_line(heights: &Grid<f32>, near: &[NearLine], ends: &Ends, slack_m: f64) -> (Vec<f64>, Vec<f64>) {
        let size = heights.width;
        let ground_m = |seen: &NearLine| f64::from(heights.data[seen.cell]);
        let out = |cells: &[usize], seen: &NearLine| cells.binary_search(&seen.cell).is_ok();
        let surely_open = |seen: &NearLine| seen.distance_m - seen.half_width_m.0 <= BANK_REACH_M - slack_m && !out(&ends.maybe, seen);
        let maybe_open = |seen: &NearLine| seen.distance_m - seen.half_width_m.1 <= BANK_REACH_M + slack_m && !out(&ends.surely, seen);
        let surely = |seen: &NearLine, level_m: f64| {
            let level_m = level_m.min(seen.surface_m.0);
            (surely_open(seen) && ground_m(seen) < level_m - slack_m).then_some(level_m)
        };
        let maybe = |seen: &NearLine, level_m: f64| {
            let level_m = level_m.min(seen.surface_m.1);
            (maybe_open(seen) && ground_m(seen) < level_m + slack_m).then_some(level_m)
        };
        let surely_in_bed = |seen: &NearLine| (seen.distance_m <= seen.half_width_m.0 - slack_m).then(|| surely(seen, f64::INFINITY)).flatten();
        let maybe_in_bed = |seen: &NearLine| (seen.distance_m <= seen.half_width_m.1 + slack_m).then(|| maybe(seen, f64::INFINITY)).flatten();
        (fill(near, size, surely_in_bed, surely), fill(near, size, maybe_in_bed, maybe))
    }

    /// The level the courses' beds keep a cell's water at, as the tests read it (`kept_by_lines`),
    /// in metres.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) struct KeptBy {
        /// The lowest it surely is: the highest surface of the lines that surely keep water there,
        /// read low; NEG_INFINITY where none surely does.
        pub surely_m: f64,
        /// The lowest it may be if a line keeps water there: the lowest surface, read low, of the
        /// lines that may; INFINITY where none may.
        pub least_m: f64,
        /// The highest it may be: the highest surface, read high, of the lines that may keep water
        /// there; NEG_INFINITY where none may.
        pub most_m: f64,
    }

    /// For each cell of `heights` (laid out as `near_lines` says), the level the water of `lines`
    /// keeps there whatever lies beside it, as the tests read it (`KeptBy`): the highest surface, at
    /// the cell's nearest point, of the lines whose water starts in it (in its bed, no farther from
    /// the line than its half width there, its ground below that surface, none of `ends`, one per
    /// line, as `reach_of_water` reads them), read with `slack_m` of slack.
    pub(crate) fn kept_by_lines(heights: &Grid<f32>, lines: &[Vec<LinePoint>], ends: &[Ends], slack_m: f64) -> Vec<KeptBy> {
        let size = heights.width;
        let none = KeptBy { surely_m: f64::NEG_INFINITY, least_m: f64::INFINITY, most_m: f64::NEG_INFINITY };
        let mut kept = vec![none; size * size];
        for (near, ends) in near_lines(size, lines, 0.0, slack_m).iter().zip(ends) {
            for seen in near {
                let (ground_m, cell) = (f64::from(heights.data[seen.cell]), seen.cell);
                let out = |cells: &[usize]| cells.binary_search(&cell).is_ok();
                if seen.distance_m <= seen.half_width_m.0 - slack_m && ground_m < seen.surface_m.0 - slack_m && !out(&ends.maybe) {
                    kept[cell].surely_m = kept[cell].surely_m.max(seen.surface_m.0);
                }
                if seen.distance_m <= seen.half_width_m.1 + slack_m && ground_m < seen.surface_m.1 + slack_m && !out(&ends.surely) {
                    kept[cell].least_m = kept[cell].least_m.min(seen.surface_m.0);
                    kept[cell].most_m = kept[cell].most_m.max(seen.surface_m.1);
                }
            }
        }
        kept
    }

    /// What a cell is to the rims the tests work out (`rims`).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub(crate) enum Held {
        /// A dry cell: a way out at its ground.
        Dry,
        /// Water kept whatever lies beside it, a bed's or a lake's: a way out at this level.
        Kept(f64),
        /// Spread water over ground read at this height, in metres: it passes on the higher of
        /// that and its rim.
        Spread(f64),
    }

    /// The rim of each cell of a grid laid out as `heights`, what it is being `held`, as the tests
    /// read `bound_spread`: for spread water, the higher of its ground (as `held` reads it) and
    /// the lowest way out beside it among its eight neighbours, a dry cell at its ground in
    /// `heights`, kept water at its level, other spread water at the higher of that one's ground
    /// and rim, and the grid's border below everything for a cell on it; INFINITY for every other
    /// cell and for spread water no way out reaches. Worked out by going over the cells again
    /// whenever a rim falls, first found first, so not in the order `bound_spread` takes them.
    pub(crate) fn rims(heights: &Grid<f32>, held: &[Held]) -> Vec<f64> {
        let (width, height) = (heights.width, heights.height);
        let around = |cell: usize| {
            let (i, j) = (cell % width, cell / width);
            (j.saturating_sub(1)..=(j + 1).min(height - 1))
                .flat_map(move |row| (i.saturating_sub(1)..=(i + 1).min(width - 1)).map(move |column| row * width + column))
                .filter(move |&next| next != cell)
        };
        let mut rim = vec![f64::INFINITY; held.len()];
        let mut waiting = std::collections::VecDeque::new();
        for (cell, &held_as) in held.iter().enumerate() {
            let Held::Spread(ground_m) = held_as else { continue };
            let (i, j) = (cell % width, cell / width);
            let on_border = i == 0 || j == 0 || i + 1 == width || j + 1 == height;
            let mut out_m = if on_border { f64::NEG_INFINITY } else { f64::INFINITY };
            for next in around(cell) {
                match held[next] {
                    Held::Dry => out_m = out_m.min(f64::from(heights.data[next])),
                    Held::Kept(level_m) => out_m = out_m.min(level_m),
                    Held::Spread(_) => {}
                }
            }
            if out_m < f64::INFINITY {
                rim[cell] = ground_m.max(out_m);
                waiting.push_back(cell);
            }
        }
        while let Some(cell) = waiting.pop_front() {
            for next in around(cell) {
                if let Held::Spread(ground_m) = held[next] {
                    let through_m = ground_m.max(rim[cell]);
                    if through_m < rim[next] {
                        rim[next] = through_m;
                        waiting.push_back(next);
                    }
                }
            }
        }
        rim
    }

    /// The water of a drawn world as the water step makes it, and as it stood before its spread was
    /// bounded (`bounded_water`).
    pub(crate) struct Bounded {
        /// The ground as the beds cut it.
        pub heights: Grid<f32>,
        /// The water's depth before the spread was bounded, and after.
        pub before: Grid<f32>,
        pub after: Grid<f32>,
        /// Each cell's kept level (`unbounded_beds_and_lakes`).
        pub kept_level: Vec<f32>,
        /// The lakes found, each with its level and its cells.
        pub lakes: Vec<LakeShape>,
    }

    /// The water `add_water` puts into `heights`, a drawn world's ground before its water, grown
    /// from `drawing` as `grown`, from the world's `seed` and under `rules`, made by `add_water`'s
    /// steps in its order, with the depth before `bound_spread` kept.
    pub(crate) fn bounded_water(heights: &Grid<f32>, grown: &Grown, drawing: &Drawing, rules: &WaterRules, seed: u64) -> Bounded {
        let mut heights = heights.clone();
        let mut channels = trace_channels(grown, drawing, rules);
        let shapes = shape_channels(&mut channels, seed, rules);
        let mut depth = Grid::new(heights.width, heights.height, heights.cell_size);
        let lakes = fill_lakes(&mut heights, &mut depth, grown, drawing, &channels, rules, &mut Vec::new(), &AtomicBool::new(false)).expect("no flag is set");
        for &cell in lakes.iter().flat_map(|lake| &lake.cells) {
            depth.data[cell] = 0.0;
        }
        let levels = levels_of(drawing, &lakes);
        let size = (heights.width, heights.height);
        let floor = |point: &ChannelPoint| floor_m(grown, size, (point.x_m, point.z_m));
        let found = profiles(&channels, &shapes, &heights, &floor, &lakes, &levels, rules, &AtomicBool::new(false)).expect("no flag is set").profiles;
        let kept_level = unbounded_beds_and_lakes(&mut heights, &mut depth, &lakes, &channels, &shapes, &found, rules);
        let before = depth.clone();
        bound_spread(&heights, &mut depth, &kept_level);
        Bounded { heights, before, after: depth, kept_level, lakes }
    }

    /// The level of each of `drawing`'s lakes as `fill_lakes` found and settled it (`lakes`), or NaN,
    /// as `add_water` hands them to the channels.
    fn levels_of(drawing: &Drawing, lakes: &[LakeShape]) -> Vec<f32> {
        (0..drawing.lakes.len()).map(|drawn| lakes.iter().find(|lake| lake.drawn == drawn).map_or(f32::NAN, |lake| lake.level_m)).collect()
    }

    /// The channels' water `add_water` works out on `heights`, a drawn world's ground before its
    /// water, grown from `drawing` as `grown`, from the world's `seed` and under `rules`, made by
    /// `add_water`'s steps in its order up to the water's surfaces (`profiles`), which read `cancel`.
    pub(crate) fn junction_water(heights: &Grid<f32>, grown: &Grown, drawing: &Drawing, rules: &WaterRules, seed: u64, cancel: &AtomicBool) -> Result<Worked, LandformError> {
        let mut heights = heights.clone();
        let mut channels = trace_channels(grown, drawing, rules);
        let shapes = shape_channels(&mut channels, seed, rules);
        let mut depth = Grid::new(heights.width, heights.height, heights.cell_size);
        let lakes = fill_lakes(&mut heights, &mut depth, grown, drawing, &channels, rules, &mut Vec::new(), &AtomicBool::new(false)).expect("no flag is set");
        let size = (heights.width, heights.height);
        let floor = |point: &ChannelPoint| floor_m(grown, size, (point.x_m, point.z_m));
        profiles(&channels, &shapes, &heights, &floor, &lakes, &levels_of(drawing, &lakes), rules, cancel)
    }

    /// The cells inside what a line ends on, whose water is not the line's, as the tests read
    /// them, each list in index order: those surely inside, however distances are read within the
    /// slack, and those that may be.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct Ends {
        pub surely: Vec<usize>,
        pub maybe: Vec<usize>,
    }

    impl Ends {
        /// These cells and `other`'s.
        pub(crate) fn and(mut self, other: Ends) -> Ends {
            for (cells, more) in [(&mut self.surely, other.surely), (&mut self.maybe, other.maybe)] {
                cells.extend(more);
                cells.sort_unstable();
                cells.dedup();
            }
            self
        }
    }

    /// For each of `lines`, the cells inside the bed of the line it ends on, `ends_on[k]` (no
    /// farther from that line than its half width at their nearest point), read with `slack_m`; none
    /// for a line that ends on no other.
    pub(crate) fn inside_the_lines_ended_on(size: usize, lines: &[Vec<LinePoint>], ends_on: &[Option<usize>], slack_m: f64) -> Vec<Ends> {
        let beds: Vec<Ends> = (near_lines(size, lines, 0.0, slack_m).iter())
            .map(|near| Ends {
                surely: near.iter().filter(|seen| seen.distance_m <= seen.half_width_m.0 - slack_m).map(|seen| seen.cell).collect(),
                maybe: near.iter().filter(|seen| seen.distance_m <= seen.half_width_m.1 + slack_m).map(|seen| seen.cell).collect(),
            })
            .collect();
        ends_on.iter().map(|end| end.map_or_else(Ends::default, |joined| beds[joined].clone())).collect()
    }

    /// The cells straight beyond the first point of `line`, on a `size` x `size` grid laid out as
    /// `near_lines` says, as the tests read them (`Ends`, read with `slack_m` of slack): those whose
    /// nearest point of the line is its first point, and which lie before it along its first
    /// stretch of any length. A line that begins on dry land holds none of its water there
    /// (`cut_beds`). Only the cells within the line's widest half width and `beyond_m` of the first
    /// point are looked at.
    pub(crate) fn behind_the_head(size: usize, line: &[LinePoint], beyond_m: f64, slack_m: f64) -> Ends {
        let half = (size / 2) as f64;
        let head = line[0];
        let Some(first) = (1..line.len()).find(|&k| (line[k].x_m - head.x_m).hypot(line[k].z_m - head.z_m) > 0.0) else { return Ends::default() };
        let length_m = (line[first].x_m - head.x_m).hypot(line[first].z_m - head.z_m);
        let along = ((line[first].x_m - head.x_m) / length_m, (line[first].z_m - head.z_m) / length_m);
        let reach_m = line.iter().map(|point| point.half_width_m).fold(0.0, f64::max) + beyond_m + slack_m;
        let apart_m = |a: &LinePoint, b: &LinePoint, (x, z): (f64, f64)| {
            let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
            let squared = dx * dx + dz * dz;
            let t = if squared > 0.0 { (((x - a.x_m) * dx + (z - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
            (a.x_m + dx * t - x).hypot(a.z_m + dz * t - z)
        };
        // The stretches after the first that may lie as near a cell within reach as the head.
        let others: Vec<usize> = (first + 1..line.len())
            .filter(|&k| apart_m(&line[k - 1], &line[k], (head.x_m, head.z_m)) <= 2.0 * reach_m + slack_m)
            .collect();
        let cells = |at: f64| ((at - reach_m + half).ceil().max(0.0) as usize)..=((at + reach_m + half).floor().max(0.0) as usize).min(size - 1);
        let mut ends = Ends::default();
        for j in cells(head.z_m) {
            for i in cells(head.x_m) {
                let (x, z) = (i as f64 - half, j as f64 - half);
                let head_m = (x - head.x_m).hypot(z - head.z_m);
                if head_m > reach_m {
                    continue;
                }
                let before_m = (x - head.x_m) * along.0 + (z - head.z_m) * along.1;
                let rest_m = others.iter().map(|&k| apart_m(&line[k - 1], &line[k], (x, z))).fold(f64::INFINITY, f64::min);
                if before_m < -slack_m && head_m < rest_m - slack_m {
                    ends.surely.push(j * size + i);
                }
                if before_m < slack_m && head_m <= rest_m + slack_m {
                    ends.maybe.push(j * size + i);
                }
            }
        }
        ends
    }

    /// The side of the hand-made world with a lake, in metres.
    const LAKE_WORLD_M: u32 = 256;
    /// Its lake's level, in metres.
    const LEVEL_M: f32 = 10.0;

    /// A hand-made world 256 m across with a lake at its centre, level at 10 m, and a brook flowing
    /// out of it to the east and one flowing into it from the west; its 1 m ground, the land model's
    /// grid and the drawing.
    ///
    /// The 1 m ground: a basin 8 m high within 30 m of the centre; to the east a valley whose floor
    /// falls from 9.9 m at 30 m east 0.02 m per metre, its sides rising 0.2 m per metre across, so
    /// the lake's water (within 1.5 radii, 60 m, of its centre) reaches 30 m down it; beyond that,
    /// from 60 to 80 m from the centre, a sill holds the valley no lower than 9.85 m, 0.15 m below
    /// the level, so the ground holds the lake at its level; to the west a shelf 9.85 m high from
    /// 30 to 45 m west, within 6 m of the axis, under the inflow's mouth; elsewhere 12 m. The model
    /// holds 1 m of water over the basin and 0.1 m along the valley's first 32 m and over the
    /// shelf, each up to the level, and lies on the 1 m ground without the sill elsewhere; the
    /// brooks run along the axis, the outflow from the basin's last cell east, the inflow into its
    /// first cell west from beyond the border.
    fn lake_world() -> (Grid<f32>, Grown, Drawing) {
        let size = LAKE_WORLD_M as usize;
        let half = (size / 2) as f64;
        let ground = |x: f64, z: f64| -> f64 {
            if x.hypot(z) <= 30.0 {
                8.0
            } else if (-45.0..=-30.0).contains(&x) && z.abs() <= 6.0 {
                9.85
            } else if x > 0.0 {
                (9.9 - 0.02 * (x.max(30.0) - 30.0) + 0.2 * z.abs()).min(12.0)
            } else {
                12.0
            }
        };
        let sill = |x: f64, z: f64| if x > 0.0 && x.hypot(z) > 60.0 && x.hypot(z) <= 80.0 { ground(x, z).max(9.85) } else { ground(x, z) };
        let mut heights = Grid::new(size, size, 1.0);
        for j in 0..size {
            for i in 0..size {
                heights.data[j * size + i] = sill(i as f64 - half, j as f64 - half) as f32;
            }
        }
        let side = size / GROW_CELL_M as usize + 64;
        let origin_m = -half - f64::from(GROW_MARGIN_M);
        let at_m = |index: usize| origin_m + index as f64 * f64::from(GROW_CELL_M);
        let index = |at_m: f64| ((at_m - origin_m) / f64::from(GROW_CELL_M)) as usize;
        let grid = |value: f32| Grid { width: side, height: side, cell_size: GROW_CELL_M, data: vec![value; side * side] };
        let (mut model_heights, mut water_depth_m) = (grid(0.0), grid(0.0));
        for j in 0..side {
            for i in 0..side {
                let (x, z) = (at_m(i), at_m(j));
                let along_valley = (32.0..=64.0).contains(&x) || (-40.0..=-32.0).contains(&x);
                let water_m = if x.hypot(z) <= 30.0 {
                    1.0
                } else if along_valley && z.abs() <= 8.0 {
                    0.1
                } else {
                    0.0
                };
                water_depth_m.data[j * side + i] = water_m;
                model_heights.data[j * side + i] = if water_m > 0.0 { LEVEL_M - water_m } else { ground(x, z) as f32 };
            }
        }
        let mut drained_m2 = grid(64.0);
        let mut receivers: Vec<u32> = (0..side * side).map(|cell| cell as u32).collect();
        let row = index(0.0) * side;
        // The outflow, from the basin's last cell (24 m east) to the grid's edge, and the inflow,
        // from beyond the west border into the basin's first cell (24 m west).
        for (from, to, drained) in [(index(24.0), side - 1, 2e6), (index(-224.0), index(-24.0), 1e6)] {
            for column in from..to {
                receivers[row + column] = (row + column + 1) as u32;
                drained_m2.data[row + column] = drained + 100.0 * (column - from) as f32;
            }
        }
        let land = Grown {
            heights: model_heights,
            drained_m2,
            water_depth_m,
            receivers,
            rivers: Vec::new(),
            lakes: vec![GrownLake { drawn: 0, level_m: LEVEL_M }],
            notes: Vec::new(),
            notes_about: Vec::new(),
        };
        let drawing = Drawing { lakes: vec![Lake { x_m: 0.0, z_m: 0.0, radius_m: 40.0 }], ..plain(LAKE_WORLD_M) };
        (heights, land, drawing)
    }

    #[test]
    fn lake_water_never_stands_above_its_outflow() {
        let rules = WaterRules::default();
        let (before, land, drawing) = lake_world();
        let size = before.width;
        let world_m = |cell: usize| ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
        // The lake alone, as the beds find it: settled knowing the courses out of it, as
        // `add_water` traces and shapes them. Its ground holds it at the model's level.
        let mut channels = trace_channels(&land, &drawing, &rules);
        shape_channels(&mut channels, 1, &rules);
        let (mut filled, mut lake_depth) = (before.clone(), Grid::new(size, size, 1.0));
        let alone = fill_lakes(&mut filled, &mut lake_depth, &land, &drawing, &channels, &rules, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
        let [alone] = &alone[..] else { panic!("one lake, not {alone:?}") };
        assert_eq!(alone.level_m, LEVEL_M, "the lake's ground does not hold it at the model's level");
        let reaches_m = alone.cells.iter().map(|&cell| world_m(cell).0).fold(f64::NEG_INFINITY, f64::max);
        assert!(reaches_m >= 55.0, "the lake's water reaches only {reaches_m} m east");

        let mut heights = before.clone();
        let (water, field) = add_water(&mut heights, &land, &drawing, &rules, 1, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
        let [lake] = &water.lakes[..] else { panic!("one lake, not {:?}", water.lakes) };
        let outflow = water.rivers.iter().find(|course| course.from_lake == Some(0)).expect("a brook flows out of the lake");
        let inflow = water.rivers.iter().find(|course| course.end == CourseEnd::Lake(0)).expect("a brook flows into the lake");
        assert_eq!(outflow.points[0].surface_m, LEVEL_M, "the outflow leaves the lake below its level");
        assert!(outflow.points.iter().all(|point| point.z_m == 0.0), "the outflow leaves the axis");
        let first_m = f64::from(outflow.points[0].x_m);

        // From 4 m along the outflow on, no cell within its banks' reach holds water above its
        // surface there. Along its stretch beside the lake it stands at the lake's level, and the
        // ground below the level beside it holds the lake's water; from its first point below the
        // level on, where its banks hold it, its banks are dry beyond its waterline. The surface and
        // the half width along the line, by `x`, are linear between its points, which lie on the
        // axis.
        let below_m = f64::from(outflow.points.iter().find(|point| point.surface_m < LEVEL_M).expect("the outflow comes down").x_m);
        let water_at = |x: f64| {
            let k = outflow.points.iter().rposition(|point| f64::from(point.x_m) <= x).expect("a point west of x");
            let (a, b) = (outflow.points[k], outflow.points[(k + 1).min(outflow.points.len() - 1)]);
            let share = if b.x_m > a.x_m { ((x - f64::from(a.x_m)) / f64::from(b.x_m - a.x_m)) as f32 } else { 0.0 };
            let half_width_m = f64::from(a.width_m + (b.width_m - a.width_m) * share) / 2.0;
            (a.surface_m + (b.surface_m - a.surface_m) * share, half_width_m)
        };
        let mut checked = 0;
        for cell in 0..size * size {
            let (x, z) = world_m(cell);
            if x < first_m + 4.0 || x > f64::from(outflow.points[outflow.points.len() - 1].x_m) {
                continue;
            }
            let (outflow_m, half_width_m) = water_at(x);
            if z.abs() > half_width_m + 12.0 {
                continue;
            }
            checked += 1;
            let depth_m = field.depth_m.data[cell];
            if x >= below_m && z.abs() > half_width_m + 0.5 {
                assert_eq!(depth_m, 0.0, "water on the outflow's bank at ({x}, {z})");
            } else if depth_m > 0.0 {
                let water_m = heights.data[cell] + depth_m;
                assert!(water_m <= outflow_m + 1e-3, "water at {water_m} m at ({x}, {z}), above the outflow's {outflow_m} m");
            }
        }
        assert!(checked > 1000, "only {checked} cells beside the outflow were checked");

        // No cell is taken off the lake: every cell it settled with holds its water at the level,
        // and its area and centre are those it was found with.
        let off = alone.cells.iter().filter(|&&cell| heights.data[cell] + field.depth_m.data[cell] < LEVEL_M - 1e-4).count();
        assert_eq!(off, 0, "{off} cells were taken off the lake");
        assert_eq!((lake.area_m2, lake.x_m, lake.z_m), (alone.area_m2, alone.x_m, alone.z_m), "the lake's area and centre");

        // The cells of the shelf the inflow's banks lowered keep the lake's water, at its level.
        let lowered: Vec<usize> = (alone.cells.iter().copied())
            .filter(|&cell| world_m(cell).0 <= -30.0 && heights.data[cell] < filled.data[cell])
            .collect();
        assert!(!lowered.is_empty(), "the inflow, ending at {:?}, lowered no cell of the shelf", inflow.points.last());
        for &cell in &lowered {
            let water_m = heights.data[cell] + field.depth_m.data[cell];
            assert!(water_m >= LEVEL_M - 1e-4, "water at {water_m} m at {:?}, below the lake's level", world_m(cell));
        }
    }

    #[test]
    fn a_set_flag_cancels_the_water() {
        let (mut heights, land, drawing) = lake_world();
        let result = add_water(&mut heights, &land, &drawing, &WaterRules::default(), 1, &mut Vec::new(), &AtomicBool::new(true));
        assert_eq!(result.err(), Some(LandformError::Cancelled));
    }

    /// The side of the hand-made grids of the lake tests below, in cells of 1 m; cell `(i, j)` lies
    /// at world `(i - 32, j - 32)`.
    const SMALL_M: usize = 64;

    /// The index of the cell at world `(x, z)` on a grid `SMALL_M` cells a side.
    fn small_cell((x, z): (i32, i32)) -> usize {
        let half = (SMALL_M / 2) as i32;
        ((z + half) * SMALL_M as i32 + x + half) as usize
    }

    /// A channel through `points`, in world metres, 2 m wide, flowing out of `from_lake` and ending
    /// `end`, with its water at `surface_m` all along it.
    fn straight(points: &[(f64, f64)], from_lake: Option<usize>, end: ChannelEnd, surface_m: f32) -> (Channel, Profile) {
        let points: Vec<ChannelPoint> = points.iter().map(|&(x_m, z_m)| ChannelPoint { x_m, z_m, width_m: 2.0, drained_m2: 0.0 }).collect();
        let water = Profile { surface_m: vec![surface_m; points.len()], depth_m: vec![0.5; points.len()], above_floor_m: 0.0, on_dry_land: false };
        (Channel { drawn: None, from_lake, points, end }, water)
    }

    /// Drawn lake `drawn` at `level_m` holding the cells at world `cells`, in index order; its centre
    /// is not read.
    fn small_lake(drawn: usize, level_m: f32, cells: impl Iterator<Item = (i32, i32)>) -> LakeShape {
        let mut cells: Vec<usize> = cells.map(small_cell).collect();
        cells.sort_unstable();
        LakeShape { drawn, level_m, x_m: 0.0, z_m: 0.0, area_m2: cells.len() as f32, cells, held_bed: Vec::new() }
    }

    /// A shape for each of `channels` that changes nothing: no wander, and the deep line on the line.
    fn unshaped(channels: &[Channel]) -> Vec<Shape> {
        (channels.iter())
            .map(|channel| Shape { factor: vec![1.0; channel.points.len()], deep_shift_m: vec![0.0; channel.points.len()] })
            .collect()
    }

    #[test]
    fn no_outflow_drains_a_lake_cell() {
        // Lake 0 at 10 m on ground 9 m high, with two outflows 2 m wide running east: one along
        // z = 0 at the lake's level, one along z = 8 a metre below it; and an outflow of lake 1
        // along z = -5, far below. (20, 2) lies 2 m from the outflow at the level and 6 m from the
        // lower one, within its banks' reach; (20, -6) and (20, -10) within reach of the outflow
        // at the level and of lake 1's; (20, -20) within reach of none. Each keeps the lake's water
        // at its level once the beds are cut.
        let mut heights = Grid { width: SMALL_M, height: SMALL_M, cell_size: 1.0, data: vec![9.0; SMALL_M * SMALL_M] };
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let lake = small_lake(0, 10.0, [(20, -20), (20, -10), (20, -6), (20, 2)].into_iter());
        let flows = [
            straight(&[(0.0, 0.0), (20.0, 0.0), (30.0, 0.0)], Some(0), ChannelEnd::Edge, 10.0),
            straight(&[(0.0, 8.0), (20.0, 8.0), (30.0, 8.0)], Some(0), ChannelEnd::Edge, 9.0),
            straight(&[(0.0, -5.0), (20.0, -5.0), (30.0, -5.0)], Some(1), ChannelEnd::Edge, 5.0),
        ];
        let (channels, profiles): (Vec<Channel>, Vec<Profile>) = flows.into_iter().unzip();
        beds_and_lakes(&mut heights, &mut depth, std::slice::from_ref(&lake), &channels, &unshaped(&channels), &profiles, &WaterRules::default());
        for &cell in &lake.cells {
            let water_m = heights.data[cell] + depth.data[cell];
            assert!((water_m - 10.0).abs() < 1e-4, "the water of lake 0 at cell {cell} stands at {water_m} m, not its level");
        }
    }

    #[test]
    fn a_lake_keeps_every_cell_it_settled_with() {
        // On ground 9 m high: lake 0 lies wholly within reach of its outflow, a metre below its
        // level; 23 of lake 1's 30 cells lie within reach of its own, which would leave 7, fewer
        // than 16; lake 2 has no outflow. No outflow takes a lake's water off a cell, so none is
        // left out here: a lake is left out only by its own ground (`fill_lakes`).
        let mut heights = Grid { width: SMALL_M, height: SMALL_M, cell_size: 1.0, data: vec![9.0; SMALL_M * SMALL_M] };
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let lakes = vec![
            small_lake(0, 10.0, (0..20).map(|x| (x, 0))),
            small_lake(1, 10.0, (-30..0).map(|x| (x, 25))),
            small_lake(2, 10.0, (-30..-10).map(|x| (x, -25))),
        ];
        let flows = [
            straight(&[(0.0, 0.0), (15.0, 0.0), (30.0, 0.0)], Some(0), ChannelEnd::Edge, 9.0),
            straight(&[(-10.0, 25.0), (0.0, 25.0), (10.0, 25.0)], Some(1), ChannelEnd::Edge, 9.0),
        ];
        let (channels, profiles): (Vec<Channel>, Vec<Profile>) = flows.into_iter().unzip();
        beds_and_lakes(&mut heights, &mut depth, &lakes, &channels, &unshaped(&channels), &profiles, &WaterRules::default());
        // Every one of the 70 cells holds its lake's water at the level.
        for lake in &lakes {
            for &cell in &lake.cells {
                let water_m = heights.data[cell] + depth.data[cell];
                assert!((water_m - 10.0).abs() < 1e-4, "the water of lake {} at cell {cell} stands at {water_m} m, not its level", lake.drawn);
            }
        }
        assert_eq!(lakes.iter().map(|lake| lake.cells.len()).sum::<usize>(), 70);
    }

    #[test]
    fn an_inflow_into_a_lake_left_out_marks_its_bed_there() {
        let rules = WaterRules::default();
        // The land of `lake_world`, its brooks flowing into and out of the lake, on 1 m ground
        // 12 m high but for a pit 9 m high, the cells within 1.5 m of the inflow's last point:
        // fewer than 16 cells below the lake's level, so the lake holds no water and is left out.
        let (_, land, drawing) = lake_world();
        let size = LAKE_WORLD_M as usize;
        let traced = trace_channels(&land, &drawing, &rules);
        let index = traced.iter().position(|channel| channel.end == ChannelEnd::Lake(0)).expect("an inflow");
        let out = traced.iter().position(|channel| channel.from_lake == Some(0)).expect("an outflow");
        let end = traced[index].points[traced[index].points.len() - 1];
        let world_m = |cell: usize| ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
        let in_pit = |cell: usize| (world_m(cell).0 - end.x_m).hypot(world_m(cell).1 - end.z_m) <= 1.5;
        let mut heights = Grid { width: size, height: size, cell_size: 1.0, data: vec![12.0; size * size] };
        let pit: Vec<usize> = (0..size * size).filter(|&cell| in_pit(cell)).collect();
        assert!(!pit.is_empty() && pit.len() < LAKE_CELLS_MIN, "a pit of {} cells", pit.len());
        for &cell in &pit {
            heights.data[cell] = 9.0;
        }
        let mut notes = Vec::new();
        let (water, field) = add_water(&mut heights, &land, &drawing, &rules, 1, &mut notes, &AtomicBool::new(false)).unwrap();
        assert!(water.lakes.is_empty(), "a lake is kept: {:?}", water.lakes);
        assert!(notes.contains(&(About::Lake(0), "Lake 1 holds no water; it was left out.".to_string())), "{notes:?}");
        // The inflow ends on the course out of the lake, and its bed where the lake was holds its
        // water, at its surface at the cell's nearest point of its line, as anywhere else.
        let course = &water.rivers[index];
        assert_eq!(course.end, CourseEnd::River(out), "the inflow ends {:?}, not on the outflow", course.end);
        let mut inside = 0;
        for &cell in &pit {
            let (apart_m, half_width_m, surface_m) = crate::held::nearest_on_course(course, world_m(cell));
            if apart_m <= half_width_m {
                inside += 1;
                let water_m = f64::from(heights.data[cell] + field.depth_m.data[cell]);
                assert!(field.depth_m.data[cell] > 0.0 && (water_m - surface_m).abs() < 1e-3, "the water at {:?} stands at {water_m} m, not the inflow's {surface_m} m", world_m(cell));
            }
        }
        assert!(inside > 0, "no cell of the pit lies in the inflow's bed");
    }

    #[test]
    fn courses_name_the_lakes_kept() {
        // Drawn lakes 0 and 3 were left out; 1 and 2 are kept, as lakes 0 and 1.
        let lakes = [small_lake(1, 10.0, std::iter::empty()), small_lake(2, 8.0, std::iter::empty())];
        let line = [(0.0, 0.0), (4.0, 0.0)];
        let flows = [
            // Into lake 0, out of which channel 1 flows.
            straight(&line, None, ChannelEnd::Lake(0), 12.0),
            // Out of lake 0, into lake 1.
            straight(&line, Some(0), ChannelEnd::Lake(1), 11.0),
            // Out of lake 1, into lake 2.
            straight(&line, Some(1), ChannelEnd::Lake(2), 10.0),
            // Into lake 3, out of which nothing flows.
            straight(&line, None, ChannelEnd::Lake(3), 9.0),
            // Out of lake 2, onto channel 0.
            straight(&line, Some(2), ChannelEnd::Channel(0), 8.0),
        ];
        let (channels, profiles): (Vec<Channel>, Vec<Profile>) = flows.into_iter().unzip();
        let found = courses(&channels, &profiles, &lakes, &WaterRules::default());
        let ends: Vec<(Option<usize>, CourseEnd)> = found.iter().map(|course| (course.from_lake, course.end)).collect();
        assert_eq!(
            ends,
            [
                (None, CourseEnd::River(1)),
                (None, CourseEnd::Lake(0)),
                (Some(0), CourseEnd::Lake(1)),
                (None, CourseEnd::Edge),
                (Some(1), CourseEnd::River(0)),
            ]
        );
        for (course, water) in found.iter().zip(&profiles) {
            assert!(course.points.iter().zip(&water.surface_m).all(|(point, &surface_m)| point.surface_m == surface_m));
        }
    }

    #[test]
    fn a_course_point_holds_the_waters_width_and_its_depth_at_the_deep_line() {
        let rules = WaterRules::default();
        let (mut channel, mut water) = straight(&[(0.0, 0.0), (4.0, 0.0), (8.0, 0.0)], None, ChannelEnd::Edge, 10.0);
        for (k, (point, depth_m)) in channel.points.iter_mut().zip(&mut water.depth_m).enumerate() {
            (point.width_m, *depth_m) = (3.0 + k as f32, 0.4 + 0.3 * k as f32);
        }
        let found = courses(std::slice::from_ref(&channel), std::slice::from_ref(&water), &[], &rules);
        for ((point, course_point), &depth_m) in channel.points.iter().zip(&found[0].points).zip(&water.depth_m) {
            assert_eq!(course_point.width_m, point.width_m, "the width");
            assert_eq!(course_point.depth_m, rules.water_fill * depth_m, "the water's depth at the deep line");
        }
    }

    #[test]
    fn another_seed_wanders_the_same_lines_differently() {
        let rules = WaterRules::default();
        let (before, land, drawing) = lake_world();
        let water_from = |seed: u64| {
            let mut heights = before.clone();
            let (water, field) = add_water(&mut heights, &land, &drawing, &rules, seed, &mut Vec::new(), &AtomicBool::new(false)).unwrap();
            (water, field.depth_m, heights)
        };
        let (first, again, other) = (water_from(1), water_from(1), water_from(2));
        assert!(first == again, "the same seed gave other water");
        let (first, other) = (&first.0.rivers, &other.0.rivers);
        assert_eq!(first.len(), other.len());
        // Each course follows the same line under the same surface, and is shaped otherwise.
        for (index, (one, two)) in first.iter().zip(other).enumerate() {
            assert_eq!(one.points.len(), two.points.len(), "course {index}");
            for (a, b) in one.points.iter().zip(&two.points) {
                assert_eq!((a.x_m, a.z_m), (b.x_m, b.z_m), "course {index} moved");
                assert!((a.surface_m - b.surface_m).abs() < 1e-4, "course {index}: the surface moved from {} to {} m", a.surface_m, b.surface_m);
            }
            let same = one.points.iter().zip(&two.points).filter(|(a, b)| a.width_m == b.width_m).count();
            assert!(same < one.points.len() / 2, "course {index}: {same} of {} widths are the same under another seed", one.points.len());
        }
    }

    /// The side of the hand-made world of the gully tests, in metres; cell `(i, j)` lies at world
    /// `(i - 128, j - 128)`.
    const GULLY_WORLD_M: u32 = 256;

    /// A hand-made land model of a world `GULLY_WORLD_M` across that lists `lakes`: on its cells
    /// of 8 m, ground 10 m high under 1 m of water at and west of x = -32 m, and dry ground 20 m
    /// high east of it. Read bilinearly at 1 m, it holds water at its level, 11 m, on the cells
    /// west of x = -24 m: at x = -25 m and less.
    fn half_flooded(lakes: Vec<GrownLake>) -> Grown {
        let side = ((GULLY_WORLD_M as f32 + 2.0 * GROW_MARGIN_M) / GROW_CELL_M) as usize;
        let at_m = |index: usize| index as f64 * f64::from(GROW_CELL_M) - f64::from(GULLY_WORLD_M) / 2.0 - f64::from(GROW_MARGIN_M);
        let wet = |cell: usize| at_m(cell % side) <= -32.0;
        let grid = |value: &dyn Fn(usize) -> f32| Grid { width: side, height: side, cell_size: GROW_CELL_M, data: (0..side * side).map(value).collect() };
        Grown {
            heights: grid(&|cell| if wet(cell) { 10.0 } else { 20.0 }),
            drained_m2: grid(&|_| 64.0),
            water_depth_m: grid(&|cell| if wet(cell) { 1.0 } else { 0.0 }),
            receivers: (0..side * side).map(|cell| cell as u32).collect(),
            rivers: Vec::new(),
            lakes,
            notes: Vec::new(),
            notes_about: Vec::new(),
        }
    }

    #[test]
    fn the_gullies_come_back_away_from_a_lake() {
        let size = GULLY_WORLD_M as usize;
        let lake = Lake { x_m: -45.0, z_m: 0.0, radius_m: 30.0 };
        let listed = || vec![GrownLake { drawn: 0, level_m: 11.0 }];
        let drawing = Drawing { lakes: vec![lake], ..plain(GULLY_WORLD_M) };
        let factor = gully_factor(&half_flooded(listed()), &drawing, (size, size)).expect("the lake holds water");
        assert_eq!(factor.len(), size * size);
        let at = |(x, z): (i32, i32)| factor[(z + 128) as usize * size + (x + 128) as usize];
        // Along z = 0 the lake's water reaches x = -25 m: 0 on it and within 8 m of it, rising
        // linearly to 1 over the next 16 m, and 1 beyond.
        for (x, expected) in [(-60, 0.0), (-25, 0.0), (-21, 0.0), (-17, 0.0), (-13, 0.25), (-9, 0.5), (-5, 0.75), (-1, 1.0), (10, 1.0), (127, 1.0)] {
            assert_eq!(at((x, 0)), expected, "{} m east of the lake's water", x + 25);
        }
        // The lake's water lies within 1.5 radii, 45 m, of its centre. The model's water beyond is
        // not the lake's: at (-45, -60), 15 m from the lake's nearest water at (-45, -45), the
        // gullies are cut by 7 in 16, and at (-45, -110) whole.
        assert_eq!(at((-45, -60)), 0.4375);
        assert_eq!(at((-45, -110)), 1.0);

        // No lake, no factor: none listed, none drawn, a listed lake whose level is not a number,
        // or a listed lake whose model holds no water within its reach.
        let elsewhere = Drawing { lakes: vec![Lake { x_m: 60.0, ..lake }], ..drawing.clone() };
        let not_a_number = vec![GrownLake { drawn: 0, level_m: f32::NAN }];
        let cases = [
            ("none listed", half_flooded(Vec::new()), &drawing),
            ("none drawn", half_flooded(listed()), &plain(GULLY_WORLD_M)),
            ("a level that is not a number", half_flooded(not_a_number), &drawing),
            ("no water within its reach", half_flooded(listed()), &elsewhere),
        ];
        for (name, land, drawing) in cases {
            assert!(gully_factor(&land, drawing, (size, size)).is_none(), "{name}: a factor");
        }
    }

    #[test]
    fn the_example_without_its_lake_has_no_gully_factor() {
        let with = Drawing::example_valley(512);
        let without = Drawing { lakes: Vec::new(), ..with.clone() };
        let factor = gully_factor(&grown(&with, 1), &with, (512, 512)).expect("the example's lake holds water");
        assert!(factor.contains(&0.0) && factor.contains(&1.0), "the factor is never 0 or never 1");
        assert!(gully_factor(&grown(&without, 1), &without, (512, 512)).is_none(), "a factor without the lake");
    }

    /// World `(x, z)` of the cell `cell` of a grid `SMALL_M` cells a side, as `small_cell` lays it
    /// out.
    fn small_at(cell: usize) -> (i32, i32) {
        let half = (SMALL_M / 2) as i32;
        (cell as i32 % SMALL_M as i32 - half, cell as i32 / SMALL_M as i32 - half)
    }

    /// A grid `SMALL_M` cells a side, laid out as `small_cell` lays it out, holding `value` at each
    /// cell's world `(x, z)`.
    fn small_grid(value: impl Fn(i32, i32) -> f32) -> Grid<f32> {
        let data = (0..SMALL_M * SMALL_M).map(|cell| value(small_at(cell).0, small_at(cell).1)).collect();
        Grid { width: SMALL_M, height: SMALL_M, cell_size: 1.0, data }
    }

    /// A channel along `z = z_m` from `x = from_m` to `x = to_m`, its points 2 m apart, `width_m`
    /// wide and ending `end`, with its water at `surface_m(x)` at each point and its bed 0.5 m deep
    /// under the deep line.
    fn along_x(from_m: i32, to_m: i32, z_m: i32, width_m: f32, end: ChannelEnd, surface_m: impl Fn(f64) -> f32) -> (Channel, Profile) {
        let points: Vec<ChannelPoint> =
            (from_m..=to_m).step_by(2).map(|x| ChannelPoint { x_m: f64::from(x), z_m: f64::from(z_m), width_m, drained_m2: 0.0 }).collect();
        let water = Profile {
            surface_m: points.iter().map(|point| surface_m(point.x_m)).collect(),
            depth_m: vec![0.5; points.len()],
            above_floor_m: 0.0,
            on_dry_land: false,
        };
        (Channel { drawn: None, from_lake: None, points, end }, water)
    }

    /// The beds of `flows` cut into `heights`, their water and that of `lakes` put in, as
    /// `beds_and_lakes` makes it before the spread is bounded and after: the depth before the pass,
    /// and `depth` after it.
    fn bounded_by_hand(heights: &mut Grid<f32>, depth: &mut Grid<f32>, lakes: &[LakeShape], flows: Vec<(Channel, Profile)>) -> Grid<f32> {
        let (channels, profiles): (Vec<Channel>, Vec<Profile>) = flows.into_iter().unzip();
        let kept_level = unbounded_beds_and_lakes(heights, depth, lakes, &channels, &unshaped(&channels), &profiles, &WaterRules::default());
        let before = depth.clone();
        bound_spread(heights, depth, &kept_level);
        before
    }

    #[test]
    fn a_sheet_on_a_slope_beside_a_bed_is_gone_and_the_beds_water_stays() {
        // A channel 4 m wide along z = 0, its water at 10 m, on ground 12 m high north of it and
        // falling 0.05 m a metre south of it from 10 m at z = 1. Its water fills its bed and is
        // carried over the slope at 10 m as far as its banks reach, 14 m south of its line: a sheet
        // with lower ground beyond it, which nothing closes in.
        let mut heights = small_grid(|_, z| if z < 1 { 12.0 } else { 10.0 - 0.05 * (z - 1) as f32 });
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let before = bounded_by_hand(&mut heights, &mut depth, &[], vec![along_x(-24, 24, 0, 4.0, ChannelEnd::Edge, |_| 10.0)]);
        // A cell's distance from the channel's line, from (-24, 0) to (24, 0).
        let apart_m = |cell: usize| f64::from((small_at(cell).0.abs() - 24).max(0)).hypot(f64::from(small_at(cell).1));
        let sheet = (0..SMALL_M * SMALL_M).filter(|&cell| apart_m(cell) > 2.0 && before.data[cell] > 0.0).count();
        assert!(sheet > 300, "only {sheet} cells beside the bed held water before the pass");
        for cell in 0..SMALL_M * SMALL_M {
            if apart_m(cell) > 2.0 {
                assert_eq!(depth.data[cell], 0.0, "water beside the bed at {:?}", small_at(cell));
            } else {
                assert_eq!(depth.data[cell].to_bits(), before.data[cell].to_bits(), "the bed's water at {:?}", small_at(cell));
            }
        }
    }

    #[test]
    fn a_hollow_beside_a_bed_fills_level_to_the_lower_of_the_beds_surface_and_its_rim() {
        // A channel 4 m wide along z = 0 from the west border, its water falling from 11 m there to
        // 9 m at x = 30, on ground 12 m high but for two hollows 8 m deep from x = -10 to 10 and
        // z = 4 to 8 either side of it, each joined to its bed by a sill 8.5 m high at z = 2 and 3.
        // From the southern hollow, at z = -9 to -14 and x = -1 to 1, a notch 9.2 m high runs to
        // ground 7 m high beyond the banks' reach. The fill gives each cell of a hollow the
        // channel's surface at its nearest point: stepped along it, 10.29 m at x = -10 and 9.65 m
        // at x = 10.
        let hollow = |x: i32, z: i32| (-10..=10).contains(&x) && (4..=8).contains(&z.abs());
        let sill = |x: i32, z: i32| (-10..=10).contains(&x) && (2..=3).contains(&z.abs());
        let notch = |x: i32, z: i32| (-1..=1).contains(&x) && (-14..=-9).contains(&z);
        let mut heights = small_grid(|x, z| match () {
            _ if hollow(x, z) => 8.0,
            _ if sill(x, z) => 8.5,
            _ if notch(x, z) => 9.2,
            _ if (-1..=1).contains(&x) && z <= -15 => 7.0,
            _ => 12.0,
        });
        let surface_m = |x: f64| (11.0 - 2.0 * (x + 32.0) / 62.0) as f32;
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let flow = along_x(-32, 30, 0, 4.0, ChannelEnd::Edge, surface_m);
        // The surface at x = 11, between the points at x = 10 and 12.
        let at_11_m = (flow.1.surface_m[21] + flow.1.surface_m[22]) / 2.0;
        let before = bounded_by_hand(&mut heights, &mut depth, &[], vec![flow]);
        let water_m = |depth: &Grid<f32>, at: (i32, i32)| heights.data[small_cell(at)] + depth.data[small_cell(at)];
        assert!(water_m(&before, (-10, 6)) - water_m(&before, (10, 6)) > 0.5, "the hollow was not stepped before the pass");
        // The northern hollow and its sill's outer row are level at the lowest water beside them,
        // the channel's waterline at x = 11, 9.61 m, below its bed at x = 10. The southern one is
        // level at its notch, 9.2 m, which lies lower; the notch is dry.
        for x in -10..=10 {
            for z in 3..=8 {
                let (north_m, south_m) = (water_m(&depth, (x, z)), water_m(&depth, (x, -z)));
                assert!((north_m - at_11_m).abs() < 1e-4, "the water at ({x}, {z}) stands at {north_m} m, not {at_11_m} m");
                assert!((south_m - 9.2).abs() < 1e-4, "the water at ({x}, {}) stands at {south_m} m, not 9.2 m", -z);
            }
        }
        for at in (-1..=1).flat_map(|x| (-14..=-9).map(move |z| (x, z))) {
            assert!(before.data[small_cell(at)] > 0.0 && depth.data[small_cell(at)] == 0.0, "the notch at {at:?} held no water, or holds some");
        }
    }

    #[test]
    fn beyond_a_courses_last_point_at_the_border_no_water_stands_and_its_bed_keeps_its_own() {
        // A channel 4 m wide along z = 0 from the west border to x = 26, 5 m short of the east
        // border, which it ends at, its water at 10 m, on ground 12 m high but for a trough from
        // x = 24 to the east border and z = -4 to 4, level at 9 m: its water is carried beside its
        // bed and beyond its last point to the border, and nothing but the border lies lower than
        // its water there. The ground beside the trough stands higher, and its bed holds its water
        // at 10 m.
        let mut heights = small_grid(|x, z| if x >= 24 && z.abs() <= 4 { 9.0 } else { 12.0 });
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let before = bounded_by_hand(&mut heights, &mut depth, &[], vec![along_x(-32, 26, 0, 4.0, ChannelEnd::Edge, |_| 10.0)]);
        let apart_m = |cell: usize| f64::from((small_at(cell).0 - 26).max(0)).hypot(f64::from(small_at(cell).1));
        let beyond = (0..SMALL_M * SMALL_M).filter(|&cell| small_at(cell).0 > 28 && before.data[cell] > 0.0).count();
        assert!(beyond >= 20, "only {beyond} cells beyond the last point held water before the pass");
        let mut kept = 0;
        for cell in 0..SMALL_M * SMALL_M {
            if apart_m(cell) > 2.0 {
                assert_eq!(depth.data[cell], 0.0, "water beyond the bed at {:?}", small_at(cell));
            } else {
                kept += usize::from(before.data[cell] > 0.0 && small_at(cell).0 > 24);
                assert_eq!(depth.data[cell].to_bits(), before.data[cell].to_bits(), "the bed's water at {:?}", small_at(cell));
            }
        }
        assert!(kept >= 10, "only {kept} of the bed's last cells held water");
    }

    #[test]
    fn the_border_is_a_way_out_only_for_a_cell_on_it() {
        // On ground 12 m high, at the middle of each edge three cells in a row inward, all holding
        // water at 10 m: on the border spread water over ground 9.5 m high, inside it spread water
        // over ground 9 m high, and inside that a kept cell, its kept level 10 m, over ground 9 m.
        // The cell on the border gets out below everything and is dry; the one inside it gets out
        // only across it, so it stands at its ground, 9.5 m.
        let half = (SMALL_M / 2) as i32;
        let rows = |step: i32| [(half - 1 - step, 0), (-half + step, 0), (0, half - 1 - step), (0, -half + step)];
        let ground = |x: i32, z: i32| match () {
            _ if rows(0).contains(&(x, z)) => 9.5,
            _ if rows(1).contains(&(x, z)) || rows(2).contains(&(x, z)) => 9.0,
            _ => 12.0,
        };
        let heights = small_grid(ground);
        let mut depth = small_grid(|x, z| if ground(x, z) < 10.0 { 10.0 - ground(x, z) } else { 0.0 });
        let kept_level = small_grid(|x, z| if rows(2).contains(&(x, z)) { 10.0 } else { f32::NEG_INFINITY }).data;
        bound_spread(&heights, &mut depth, &kept_level);
        for (on, inside, kept) in rows(0).into_iter().zip(rows(1)).zip(rows(2)).map(|((on, inside), kept)| (on, inside, kept)) {
            assert_eq!(depth.data[small_cell(on)], 0.0, "water on the border at {on:?}");
            assert_eq!(heights.data[small_cell(inside)] + depth.data[small_cell(inside)], 9.5, "the water inside the border at {inside:?}");
            assert_eq!(depth.data[small_cell(kept)], 1.0, "the kept water at {kept:?}");
        }
    }

    #[test]
    fn hair_deep_spread_water_goes_where_its_rim_lies_at_its_ground() {
        // A kept cell at (0, 0), its level 100.5 m, over ground 100 m high, with spread water
        // 3e-6 m deep beside it across a side at (1, 0), over ground 100 m high, less deep than
        // single precision's step there, and dry ground 99.5 m high beyond it at (2, 0); everything
        // else dry, 101 m high, but for (-3, 1), spread water as thin over ground 100 m high beside
        // a kept bed from (-5, 0) to (-1, 0) and higher ground. The first gets out at its own
        // ground, so it is dry; the second's rim is the bed's level, so it keeps its hair.
        let hair_m = 3e-6;
        let bed = |x: i32, z: i32| z == 0 && (-5..=0).contains(&x);
        let ground = |x: i32, z: i32| match (x, z) {
            _ if bed(x, z) => 100.0,
            (1, 0) | (-3, 1) => 100.0,
            (2, 0) => 99.5,
            _ => 101.0,
        };
        let heights = small_grid(ground);
        let mut depth = small_grid(|x, z| match (x, z) {
            _ if bed(x, z) => 0.5,
            (1, 0) | (-3, 1) => hair_m,
            _ => 0.0,
        });
        assert_eq!(100.0_f32 + hair_m, 100.0, "the hair reads at its ground in single precision");
        let kept_level = small_grid(|x, z| if bed(x, z) { 100.5 } else { f32::NEG_INFINITY }).data;
        bound_spread(&heights, &mut depth, &kept_level);
        assert_eq!(depth.data[small_cell((1, 0))], 0.0, "the hair of water whose rim is its ground is still there");
        assert_eq!(depth.data[small_cell((-3, 1))], hair_m, "the hair of water closed in by the bed is gone");
        assert!((-5..=0).all(|x| depth.data[small_cell((x, 0))] == 0.5), "the bed's water moved");
    }

    #[test]
    fn a_beds_cells_under_another_channels_higher_sheet_come_down_to_their_own_surface() {
        // Channel A, 4 m wide along z = -8, its water at 10 m, and channel B, 4 m wide along z = 0,
        // its water at 8.5 m, both from the west border to x = 30, on a shelf 9 m high from z = -6
        // to 9 and ground 12 m high beyond it. A's water is carried over the shelf at 10 m as far as
        // its banks reach, z = 6, B's bed with it: a sheet. B's bed is a way out at its own
        // surface, which lies below the shelf.
        let mut heights = small_grid(|_, z| if (-6..=9).contains(&z) { 9.0 } else { 12.0 });
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let flows = vec![along_x(-32, 30, -8, 4.0, ChannelEnd::Edge, |_| 10.0), along_x(-32, 30, 0, 4.0, ChannelEnd::Edge, |_| 8.5)];
        let before = bounded_by_hand(&mut heights, &mut depth, &[], flows);
        let water_m = |depth: &Grid<f32>, at: (i32, i32)| heights.data[small_cell(at)] + depth.data[small_cell(at)];
        for x in -30..=30 {
            assert!((water_m(&before, (x, 0)) - 10.0).abs() < 1e-4, "B's bed at ({x}, 0) held {} m before the pass", water_m(&before, (x, 0)));
            for z in -1..=1 {
                assert!((water_m(&depth, (x, z)) - 8.5).abs() < 1e-4, "B's bed at ({x}, {z}) stands at {} m, not its own 8.5 m", water_m(&depth, (x, z)));
            }
            for z in (-5..=6).filter(|z: &i32| z.abs() > 1) {
                assert_eq!(depth.data[small_cell((x, z))], 0.0, "the sheet at ({x}, {z}) is still there");
            }
            for z in -10..=-6 {
                let cell = small_cell((x, z));
                assert_eq!(depth.data[cell].to_bits(), before.data[cell].to_bits(), "A's bed at ({x}, {z})");
            }
        }
    }

    #[test]
    fn a_pond_the_pass_cuts_off_is_dropped() {
        // On ground 10 m high, a bed along z = 0 from x = -20 to 20 holding water at 10 m, its kept
        // level. Spread water, 0.1 m deep, at 9.9 m, over a link from (0, 1) to (0, 3) and a ring
        // of cells 2 from (0, 6) along either axis, on ground 9.8 m high, with ground 9.7 m high
        // east of the ring; inside the ring a pond at 9.5 m on ground 9 m high. And one cell, (21,
        // 1), beside the bed's end only across a corner, in a hollow 9 m deep holding water at
        // 9.5 m.
        let ring = |x: i32, z: i32| x.abs().max((z - 6).abs()) == 2;
        let pond = |x: i32, z: i32| x.abs().max((z - 6).abs()) < 2;
        let link = |x: i32, z: i32| x == 0 && (1..=3).contains(&z);
        let ground = |x: i32, z: i32| match () {
            _ if z == 0 && x.abs() <= 20 => 9.5,
            _ if ring(x, z) || link(x, z) => 9.8,
            _ if pond(x, z) || (x, z) == (21, 1) => 9.0,
            _ if x == 3 && (4..=8).contains(&z) => 9.7,
            _ => 10.0,
        };
        let water = |x: i32, z: i32| match () {
            _ if z == 0 && x.abs() <= 20 => 10.0,
            _ if ring(x, z) || link(x, z) => 9.9,
            _ if pond(x, z) || (x, z) == (21, 1) => 9.5,
            _ => ground(x, z),
        };
        let heights = small_grid(ground);
        let mut depth = small_grid(|x, z| water(x, z) - ground(x, z));
        let kept_level = small_grid(|x, z| if z == 0 && x.abs() <= 20 { 10.0 } else { f32::NEG_INFINITY }).data;
        bound_spread(&heights, &mut depth, &kept_level);
        // The ring and the link drain east, which leaves the pond inside the ring, held at 9.8 m,
        // and the cell across the corner cut off from the bed: both are dropped.
        let wet: Vec<(i32, i32)> = (0..SMALL_M * SMALL_M).filter(|&cell| depth.data[cell] > 0.0).map(small_at).collect();
        assert!(wet.iter().all(|&(x, z)| z == 0 && x.abs() <= 20), "water off the bed at {:?}", wet.iter().filter(|&&(_, z)| z != 0).collect::<Vec<_>>());
        assert_eq!(wet.len(), 41, "the bed's cells");
    }

    #[test]
    fn a_springs_head_keeps_its_hair_of_water_and_a_kept_cell_above_its_level_comes_down() {
        // On ground 10 m high, kept cells at 10 m: at (-4, 0) a spring's head, its water a hair
        // above its ground, which single precision reads as its kept level; at (0, 0) water a hair
        // more than the margin above it, on ground 9.5 m high; at (4, 0) water 0.1 m above it.
        let hair_m = KEPT_MARGIN_M / 2.0;
        let heights = small_grid(|x, z| if z == 0 && x >= 0 { 9.5 } else { 10.0 });
        let water = |x: i32, z: i32| match (x, z) {
            (-4, 0) => 10.0 + hair_m,
            (0, 0) => 10.0 + 2.0 * KEPT_MARGIN_M,
            (4, 0) => 10.1,
            _ => 0.0,
        };
        let mut depth = small_grid(|x, z| (water(x, z) - if z == 0 && x >= 0 { 9.5 } else { 10.0 }).max(0.0));
        let kept_level = small_grid(|x, z| if water(x, z) > 0.0 { 10.0 } else { f32::NEG_INFINITY }).data;
        let hair = depth.data[small_cell((-4, 0))];
        assert!(hair > 0.0 && 10.0 + hair > 10.0, "no hair of water at the head");
        bound_spread(&heights, &mut depth, &kept_level);
        assert_eq!(depth.data[small_cell((-4, 0))], hair, "the spring's head lost its hair of water");
        for x in [0, 4] {
            assert_eq!(depth.data[small_cell((x, 0))], 0.5, "the kept cell at ({x}, 0) stands {} m deep, not at its level", depth.data[small_cell((x, 0))]);
        }
    }

    #[test]
    fn the_same_grid_gives_the_same_depths_taken_in_any_order_of_discovery() {
        // Ground on steps of 0.1 m (so many cells and rims tie) from 9.5 m to 10.5 m, laid at
        // random, water at 10.2 m wherever it lies lower, and two crossing strips kept at 10.2 m
        // where they hold water: the grid and its seven turns and mirror images, each with its
        // cells in another order, the pass worked out on each and turned back.
        let size = 48;
        let mut random = crate::rng::SplitMix64::new(7);
        let ground: Vec<f32> = (0..size * size).map(|_| 9.5 + (random.next_u64() % 11) as f32 / 10.0).collect();
        let depth: Vec<f32> = ground.iter().map(|&ground_m| (10.2 - ground_m).max(0.0)).collect();
        let kept: Vec<f32> = (0..size * size)
            .map(|cell| if (cell / size == 20 || cell % size == 30) && depth[cell] > 0.0 { 10.2 } else { f32::NEG_INFINITY })
            .collect();
        // Where cell (i, j) goes under each of the eight turns and mirror images.
        let turned = |turn: usize, cell: usize| {
            let (i, j) = (cell % size, cell / size);
            let (i, j) = if turn & 4 != 0 { (j, i) } else { (i, j) };
            let i = if turn & 1 != 0 { size - 1 - i } else { i };
            let j = if turn & 2 != 0 { size - 1 - j } else { j };
            j * size + i
        };
        let grid = |data: Vec<f32>| Grid { width: size, height: size, cell_size: 1.0, data };
        let bounded = |turn: usize| {
            let put = |values: &[f32]| {
                let mut out = vec![0.0; size * size];
                for cell in 0..size * size {
                    out[turned(turn, cell)] = values[cell];
                }
                out
            };
            let (heights, mut water, kept_level) = (grid(put(&ground)), grid(put(&depth)), put(&kept));
            bound_spread(&heights, &mut water, &kept_level);
            (0..size * size).map(|cell| water.data[turned(turn, cell)]).collect::<Vec<f32>>()
        };
        let first = bounded(0);
        let changed = first.iter().zip(&depth).filter(|(after, before)| after != before).count();
        let dried = first.iter().zip(&depth).filter(|(&after, &before)| after == 0.0 && before > 0.0).count();
        assert!(changed > 100 && dried > 50 && dried < changed, "the pass changed {changed} cells and dried {dried} of them");
        for turn in 1..8 {
            let other = bounded(turn);
            let differ = (0..size * size).filter(|&cell| other[cell].to_bits() != first[cell].to_bits()).count();
            assert_eq!(differ, 0, "turn {turn} gives other depths at {differ} cells");
        }
    }

    #[test]
    fn beside_a_course_out_of_a_lake_water_at_its_level_stays_only_where_the_ground_closes_it_in() {
        // Lake 0 at 10 m on ground 8 m high from x = -30 to -10 and z = -10 to 10, and a course
        // out of it, 4 m wide along z = 0 from x = -12 to the east border, standing at the lake's
        // level to x = 4 and falling 0.05 m a metre beyond, on ground 10.5 m high. Beside its
        // stretch at the level, from x = -9 to 2, ground 9.9 m high from its waterline out to
        // z = 5 and -5. North that strip is closed in, by the lake, the course and the higher
        // ground; south it opens on ground 9.5 m high reaching beyond the banks' reach and on to
        // ground 9 m high from z = -17.
        let strip = |x: i32| (-9..=2).contains(&x);
        let mut heights = small_grid(|x, z| match () {
            _ if (-30..=-10).contains(&x) && z.abs() <= 10 => 8.0,
            _ if strip(x) && (2..=5).contains(&z.abs()) => 9.9,
            _ if strip(x) && (-16..=-6).contains(&z) => 9.5,
            _ if strip(x) && z <= -17 => 9.0,
            _ => 10.5,
        });
        let mut depth = Grid::new(SMALL_M, SMALL_M, 1.0);
        let lake = small_lake(0, 10.0, (-30..=-10).flat_map(|x| (-10..=10).map(move |z| (x, z))));
        let (out, water) = along_x(-12, 30, 0, 4.0, ChannelEnd::Edge, |x| (10.0 - 0.05 * (x - 4.0).max(0.0)) as f32);
        let before = bounded_by_hand(&mut heights, &mut depth, std::slice::from_ref(&lake), vec![(Channel { from_lake: Some(0), ..out }, water)]);
        let water_m = |depth: &Grid<f32>, at: (i32, i32)| heights.data[small_cell(at)] + depth.data[small_cell(at)];
        for x in -9..=2 {
            for z in 3..=5 {
                for (name, depth) in [("before", &before), ("after", &depth)] {
                    assert!((water_m(depth, (x, z)) - 10.0).abs() < 1e-4, "{name} the pass, the water at ({x}, {z}) stands at {} m", water_m(depth, (x, z)));
                }
            }
            for z in -14..=-3 {
                assert!(before.data[small_cell((x, z))] > 0.0, "no water at ({x}, {z}) before the pass");
                assert_eq!(depth.data[small_cell((x, z))], 0.0, "water at ({x}, {z}), on the strip that opens south");
            }
        }
        for &cell in &lake.cells {
            assert!((heights.data[cell] + depth.data[cell] - 10.0).abs() < 1e-4, "the lake's cell at {:?}", small_at(cell));
        }
    }
}
