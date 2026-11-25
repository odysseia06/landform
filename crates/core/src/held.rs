//! The measure of water the ground does not hold.
//!
//! Water is held when the ground beside it stands at or above its surface. `unheld` counts, for a
//! drawn world, three ways it is not: dry cells that lie below water beside them, lake water that
//! stands over other water beside it, and steps between neighbouring water cells that the ground
//! beneath does not explain. It reads the world and changes nothing; `unheld_cells` gives the
//! places it counts, by the same code.

use crate::grid::Grid;
use crate::water::{CourseEnd, RiverCourse, BANK_REACH_M, HALF_WIDTH_MIN_M, WATER_LAYER};
use crate::world::World;

/// How far below the water beside it a dry cell may lie, or how far above other water beside it a
/// lake's water may stand, before the measure counts it, in metres.
pub const UNHELD_LIMIT_M: f32 = 0.3;

/// How near a water surface must lie to a lake's level to be the lake's water, in metres.
pub(crate) const LAKE_LEVEL_WITHIN_M: f64 = 0.002;

/// A dry cell whose water abreast of it stands more than this above its ground is counted apart as
/// deep, in metres.
const DEEP_DRY_M: f64 = 1.0;

/// How far beyond its waterline a course's water counts as abreast of a dry cell, measured to the
/// nearest point of the course's line, in metres.
const ABREAST_REACH_M: f64 = 12.0;

/// The water's surfaces in two water cells beside each other across a side may differ by no more
/// than this more than their ground does, in metres, before the pair is a step.
const STEP_BEYOND_GROUND_M: f64 = 0.3;

/// Slack for single precision on a step's size, in metres.
const STEP_SLACK_M: f64 = 1e-4;

/// Slack on the banks' reach when a pair of water cells is placed at a mouth, in metres.
const MOUTH_SLACK_M: f64 = 1e-3;

/// What the measure finds in a world.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Unheld {
    /// Dry cells more than `UNHELD_LIMIT_M` below the water of a lake beside them.
    pub dry_lake: usize,
    /// Dry cells more than `UNHELD_LIMIT_M` below the water abreast of them, where that is a
    /// course's.
    pub dry_course: usize,
    /// Of the two counts above, the cells more than 1 m below.
    pub dry_over_1m: usize,
    /// How far below its water the deepest dry cell lies, in metres; 0 when none is counted.
    pub worst_dry_m: f32,
    /// World `(x, z)`, in metres, of that cell; `(0, 0)` when none is counted.
    pub worst_dry_at: (f32, f32),
    /// Lake cells with water beside them, not at that lake's level, that lies more than
    /// `UNHELD_LIMIT_M` lower; water at another lake's lower level counts as lower water.
    pub lake_over_water: usize,
    /// Pairs of water cells across a side whose surfaces differ by more than their ground does
    /// plus 0.3 m, leaving out those at a mouth.
    pub water_steps: usize,
    /// The same pairs where they lie at a mouth: both cells within the banks' reach of a course
    /// that ends on another and of that other.
    pub mouth_steps: usize,
    /// How far the worst step exceeds its ground's, in metres, over both counts; 0 when none.
    pub worst_step_m: f32,
}

impl Unheld {
    /// The dry cells counted, at lakes and at courses.
    pub fn dry(&self) -> usize {
        self.dry_lake + self.dry_course
    }
}

/// Counts the water of `world` the ground does not hold; `None` for a world without the `water`
/// layer.
///
/// **Dry cells.** For a dry cell, `W` is the lowest water surface (height plus depth) among its
/// eight neighbours that hold water. If `W` lies within 0.002 m of the level of a lake of
/// `world.water.lakes`, the cell is below `W` by `W` less its ground, and counts in `dry_lake`
/// when that is more than `UNHELD_LIMIT_M`. Otherwise the water abreast of it is the highest, over
/// the courses within 12 m of the cell beyond their half width (at the nearest point of their line:
/// the width linear between the stretch's two points, halved, then raised to at least
/// `HALF_WIDTH_MIN_M`, 0.707 m), of the lower of `W` and that course's surface there, linear
/// between points; `W` itself when no course is that near. It counts in `dry_course` when the
/// water abreast of it stands more than the limit above its ground. Taking the course's own
/// surface at the cell's nearest point leaves out the dry edge of a chute, which lies lower than
/// the bed cell upstream of it.
///
/// **Lake over water.** A water cell at a lake's level with a water cell among its eight
/// neighbours that is not at that same lake's level and stands more than the limit lower. A
/// neighbour at another lake's lower level is not at this lake's level, so it counts as lower
/// water: a lake standing over a lower lake beside it is counted.
///
/// **Steps.** Pairs of water cells across a side whose surfaces differ by more than their ground
/// does plus 0.3 m (`steps_beyond_the_ground`), those at a mouth counted apart.
///
/// The counts are those of `unheld_cells`, which gives each cell and pair counted.
pub fn unheld(world: &World) -> Option<Unheld> {
    unheld_cells(world).map(|cells| cells.counts())
}

/// A lake's water cell that `unheld` counts as standing over lower water beside it
/// (`Unheld::lake_over_water`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LakeOverWater {
    /// World `(x, z)` of the cell's middle, in metres.
    pub at: (f64, f64),
    /// How far it stands over the lowest water beside it that is not at its lake's level, in
    /// metres: more than `UNHELD_LIMIT_M`.
    pub over_m: f32,
}

/// A pair of water cells beside each other across a side that `unheld` counts as a step
/// (`Unheld::water_steps`, or `Unheld::mouth_steps` at a mouth).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterStep {
    /// How far the water's surface steps beyond the ground between them, in metres: more than
    /// 0.3 m.
    pub beyond_m: f64,
    /// World `(x, z)` of the two cells' middles, in metres: the west or north one first.
    pub a: (f64, f64),
    pub b: (f64, f64),
    /// Whether the pair lies at a mouth, both cells within the banks' reach of a course that ends on
    /// another and of that other.
    pub at_mouth: bool,
}

/// Every place `unheld` counts in a world: its dry cells in index order, its lake cells over lower
/// water in index order, and its steps, the largest first. It changes nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UnheldCells {
    pub dry: Vec<DryCell>,
    pub lake_over_water: Vec<LakeOverWater>,
    pub steps: Vec<WaterStep>,
}

impl UnheldCells {
    /// What `unheld` gives for the world these places were found in: the counts of each kind, the
    /// deepest dry cell (the first in index order of several as deep) and the largest step.
    pub fn counts(&self) -> Unheld {
        let mut found = Unheld::default();
        let mut worst_m = 0.0_f64;
        for &DryCell { at: (x, z), below_m, at_lake } in &self.dry {
            if at_lake {
                found.dry_lake += 1;
            } else {
                found.dry_course += 1;
            }
            if below_m > DEEP_DRY_M {
                found.dry_over_1m += 1;
            }
            if below_m > worst_m {
                worst_m = below_m;
                (found.worst_dry_m, found.worst_dry_at) = (below_m as f32, (x as f32, z as f32));
            }
        }
        found.lake_over_water = self.lake_over_water.len();
        let mut worst_step_m = 0.0_f64;
        for step in &self.steps {
            if step.at_mouth {
                found.mouth_steps += 1;
            } else {
                found.water_steps += 1;
            }
            worst_step_m = worst_step_m.max(step.beyond_m);
        }
        found.worst_step_m = worst_step_m as f32;
        found
    }
}

/// Every place `unheld` counts in `world`, the measure's own code, so `unheld_cells(world)`'s
/// `counts()` is `unheld(world)`; `None` for a world without the `water` layer. It changes nothing.
pub fn unheld_cells(world: &World) -> Option<UnheldCells> {
    let depth = world.layers.get(WATER_LAYER)?;
    let lake_over_water = lake_cells_over_water(world, depth).into_iter().map(|(at, over_m)| LakeOverWater { at, over_m }).collect();
    let steps = (steps_beyond_the_ground(world).into_iter()).map(|(beyond_m, a, b)| WaterStep { beyond_m, a, b, at_mouth: at_a_mouth(world, a, b) }).collect();
    Some(UnheldCells { dry: dry_cells(world), lake_over_water, steps })
}

/// World `(x, z)` of the middle of the cell `cell` of a grid `size` cells across.
fn at_cell(size: usize, cell: usize) -> (f64, f64) {
    ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64)
}

/// The cells of `grid` among the eight around `(i, j)`, not `(i, j)` itself, as indices.
fn around(grid: &Grid<f32>, i: usize, j: usize) -> impl Iterator<Item = usize> + '_ {
    let width = grid.width;
    (j.saturating_sub(1)..=(j + 1).min(grid.height - 1))
        .flat_map(move |nj| (i.saturating_sub(1)..=(i + 1).min(width - 1)).map(move |ni| (ni, nj)))
        .filter(move |&place| place != (i, j))
        .map(move |(ni, nj)| nj * width + ni)
}

/// Whether the surface `surface_m` lies at the level of a lake of `world`.
fn at_a_lake(world: &World, surface_m: f64) -> bool {
    world.water.lakes.iter().any(|lake| (surface_m - f64::from(lake.level_m)).abs() < LAKE_LEVEL_WITHIN_M)
}

/// How far world `(x, z)` lies from the nearest point of `course`'s line, and the half width and
/// surface of its water there, linear along each stretch: the half width is half the width linear
/// between the stretch's two points, with no floor; its callers raise it to `HALF_WIDTH_MIN_M`
/// after.
pub(crate) fn nearest_on_course(course: &RiverCourse, (x, z): (f64, f64)) -> (f64, f64, f64) {
    let mut nearest = (f64::INFINITY, 0.0, f64::NAN);
    for pair in course.points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (ax, az, bx, bz) = (f64::from(a.x_m), f64::from(a.z_m), f64::from(b.x_m), f64::from(b.z_m));
        let (dx, dz) = (bx - ax, bz - az);
        let squared = dx * dx + dz * dz;
        let t = if squared > 0.0 { (((x - ax) * dx + (z - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        let apart_m = (ax + dx * t - x).hypot(az + dz * t - z);
        if apart_m < nearest.0 {
            let between = |u: f32, v: f32| f64::from(u) + (f64::from(v) - f64::from(u)) * t;
            nearest = (apart_m, between(a.width_m, b.width_m) / 2.0, between(a.surface_m, b.surface_m));
        }
    }
    nearest
}

/// The water abreast of a dry cell at world `at` whose lowest water beside it stands at `level_m`:
/// the highest, over the courses within `ABREAST_REACH_M` of it beyond their half width (at its
/// line's nearest point, interpolated between the stretch's points, then raised to at least
/// `HALF_WIDTH_MIN_M`), of the lower of `level_m` and the course's surface at that point;
/// `level_m` when no course is that near.
fn water_abreast(world: &World, at: (f64, f64), level_m: f64) -> f64 {
    let reached = world.water.rivers.iter().filter_map(|course| {
        let (apart_m, half_width_m, surface_m) = nearest_on_course(course, at);
        (apart_m - half_width_m.max(HALF_WIDTH_MIN_M) <= ABREAST_REACH_M).then(|| surface_m.min(level_m))
    });
    reached.reduce(f64::max).unwrap_or(level_m)
}

/// A dry cell the measure counts (`unheld`): where it lies, how far below the water it is read
/// against, and whether that water is a lake's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DryCell {
    /// World `(x, z)` of the cell's middle, in metres.
    pub at: (f64, f64),
    /// How far below the water it is read against it lies, in metres: more than `UNHELD_LIMIT_M`.
    pub below_m: f64,
    /// Whether that water is a lake's (`Unheld::dry_lake`), not a course's.
    pub at_lake: bool,
}

/// Every dry cell of `world` that `unheld` counts, in index order; none for a world without the
/// `water` layer.
pub(crate) fn dry_cells(world: &World) -> Vec<DryCell> {
    let Some(depth) = world.layers.get(WATER_LAYER) else { return Vec::new() };
    let (heights, size) = (&world.heights, world.heights.width);
    let limit_m = f64::from(UNHELD_LIMIT_M);
    let mut counted = Vec::new();
    for j in 0..heights.height {
        for i in 0..size {
            let cell = j * size + i;
            if depth.data[cell] > 0.0 {
                continue;
            }
            let lowest = around(heights, i, j).filter(|&other| depth.data[other] > 0.0).map(|other| heights.data[other] + depth.data[other]).reduce(f32::min);
            let Some(lowest) = lowest else { continue };
            let (level_m, ground_m) = (f64::from(lowest), f64::from(heights.data[cell]));
            if level_m - ground_m <= limit_m {
                continue;
            }
            let at_lake = at_a_lake(world, level_m);
            let below_m = if at_lake { level_m - ground_m } else { water_abreast(world, at_cell(size, cell), level_m) - ground_m };
            if below_m > limit_m {
                counted.push(DryCell { at: at_cell(size, cell), below_m, at_lake });
            }
        }
    }
    counted
}

/// Every water cell of `world` that `unheld` counts as a lake's over lower water, in index order:
/// world `(x, z)`, in metres, and how far it stands over the lowest water beside it that is not at
/// its lake's level, more than `UNHELD_LIMIT_M`; water at another lake's lower level is such
/// water. `depth` is the world's `water` layer.
pub(crate) fn lake_cells_over_water(world: &World, depth: &Grid<f32>) -> Vec<((f64, f64), f32)> {
    let (heights, size) = (&world.heights, world.heights.width);
    let surface_m = |cell: usize| heights.data[cell] + depth.data[cell];
    let at_level = |cell: usize, level_m: f32| (surface_m(cell) - level_m).abs() < LAKE_LEVEL_WITHIN_M as f32;
    let mut counted = Vec::new();
    for cell in (0..heights.data.len()).filter(|&cell| depth.data[cell] > 0.0) {
        let Some(lake) = world.water.lakes.iter().find(|lake| at_level(cell, lake.level_m)) else { continue };
        let lower = around(heights, cell % size, cell / size).filter(|&other| depth.data[other] > 0.0 && !at_level(other, lake.level_m));
        if let Some(over_m) = lower.map(|other| surface_m(cell) - surface_m(other)).reduce(f32::max).filter(|&over_m| over_m > UNHELD_LIMIT_M) {
            counted.push((at_cell(size, cell), over_m));
        }
    }
    counted
}

/// How far the water's surface steps beyond the ground between two water cells beside each other,
/// in metres, and where the two cells lie, in world metres.
pub(crate) type Step = (f64, (f64, f64), (f64, f64));

/// Every pair of water cells of `world` beside each other across a side whose surfaces (ground
/// plus depth) differ by more than their ground does plus `STEP_BEYOND_GROUND_M` (and 1e-4 m for
/// single precision): how much more, and where the two cells lie, the most first. Empty for a
/// world without the `water` layer.
pub(crate) fn steps_beyond_the_ground(world: &World) -> Vec<Step> {
    let Some(layer) = world.layers.get(WATER_LAYER).map(|layer| &layer.data) else { return Vec::new() };
    let (heights, size) = (&world.heights.data, world.heights.width);
    let mut steps = Vec::new();
    for cell in (0..size * size).filter(|&cell| layer[cell] > 0.0) {
        // Each pair once: the cell and the one east of it, and the one south of it.
        let east = (cell % size + 1 < size).then_some(cell + 1);
        let south = (cell / size + 1 < size).then_some(cell + size);
        for other in [east, south].into_iter().flatten().filter(|&other| layer[other] > 0.0) {
            let surface_m = |cell: usize| f64::from(heights[cell]) + f64::from(layer[cell]);
            let beyond_m = (surface_m(cell) - surface_m(other)).abs() - (f64::from(heights[cell]) - f64::from(heights[other])).abs();
            if beyond_m > STEP_BEYOND_GROUND_M + STEP_SLACK_M {
                steps.push((beyond_m, at_cell(size, cell), at_cell(size, other)));
            }
        }
    }
    steps.sort_by(|a, b| b.0.total_cmp(&a.0));
    steps
}

/// Whether the two cells of a pair, at world `a` and `b`, lie at a mouth: both within the banks'
/// reach (no more than `BANK_REACH_M` beyond half the water's width at its line's nearest point,
/// interpolated between the stretch's points, then raised to at least `HALF_WIDTH_MIN_M`, and
/// 1e-3 m) of a course that ends on another and of that other,
/// where the tributary comes down the river's cut bank and its water meets the river's beside it.
pub(crate) fn at_a_mouth(world: &World, a: (f64, f64), b: (f64, f64)) -> bool {
    let within = |course: &RiverCourse, at: (f64, f64)| {
        let (apart_m, half_width_m, _) = nearest_on_course(course, at);
        apart_m - half_width_m.max(HALF_WIDTH_MIN_M) <= BANK_REACH_M + MOUTH_SLACK_M
    };
    (world.water.rivers.iter()).any(|course| match course.end {
        CourseEnd::River(joined) => [course, &world.water.rivers[joined]].iter().all(|course| within(course, a) && within(course, b)),
        CourseEnd::Edge | CourseEnd::Lake(_) => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::water::{CoursePoint, LakeWater, Water};
    use crate::world::{Timings, WorldMeta};
    use crate::{GroundCell, Window};
    use std::collections::BTreeMap;

    /// Cells across the hand-made worlds; cell `(i, j)` lies at world `(i - 16, j - 16)`.
    const SIZE: usize = 32;
    /// The cell the measures' hand-made cases lie around, `(0, 0)` in world metres.
    const MIDDLE: usize = SIZE / 2;

    fn at(i: usize, j: usize) -> usize {
        j * SIZE + i
    }

    /// A world of flat ground `ground_m` high with no water yet (its `water` layer all 0) and the
    /// lakes and courses of `water`.
    fn flat_world(ground_m: f32, water: Water) -> World {
        let heights = Grid { width: SIZE, height: SIZE, cell_size: 1.0, data: vec![ground_m; SIZE * SIZE] };
        let layer = Grid { width: SIZE, height: SIZE, cell_size: 1.0, data: vec![0.0; SIZE * SIZE] };
        let meta = WorldMeta {
            seed: 0,
            source_id: "by hand".to_string(),
            window: Window { source: 0, center_x: 0.0, center_y: 0.0, rotation_rad: 0.0, mirrored: false, size_m: SIZE as f64 },
            base_elevation_m: 0.0,
            north: (0.0, -1.0),
            timings: Timings::default(),
        };
        World {
            heights,
            layers: BTreeMap::from([(WATER_LAYER.to_string(), layer)]),
            ground: Grid::<GroundCell>::new(SIZE, SIZE, 1.0),
            meta,
            notes: Vec::new(),
            notes_about: Vec::new(),
            water,
        }
    }

    /// Water `depth_m` deep over the ground of `cell`, which is made `surface_m` less `depth_m`.
    fn wet(world: &mut World, cell: usize, surface_m: f32, depth_m: f32) {
        world.heights.data[cell] = surface_m - depth_m;
        world.layers.get_mut(WATER_LAYER).expect("the water layer").data[cell] = depth_m;
    }

    fn lake_at(level_m: f32) -> LakeWater {
        LakeWater { drawn: 0, level_m, x_m: 0.0, z_m: 0.0, area_m2: 1.0 }
    }

    /// A course along `z = 0` from `x = -4` to `x = 4`, 1 m wide, its surface `surface_m(x)`.
    fn course_along_x(surface_m: impl Fn(f32) -> f32, end: CourseEnd) -> RiverCourse {
        let point = |x: f32| CoursePoint { x_m: x, z_m: 0.0, surface_m: surface_m(x), width_m: 1.0, depth_m: 0.5 };
        RiverCourse { drawn: Some(0), from_lake: None, points: vec![point(-4.0), point(4.0)], end }
    }

    /// A world whose ground falls 2 m for each metre east, 11 m at the middle, with one wet cell at
    /// the middle, and the course along `z = 0` falling as the ground does: a chute. The cell
    /// `(MIDDLE + 1, MIDDLE + 1)`, one east and one south of the wet cell, lies at `ground_m`.
    ///
    /// The wet cell, the bed cell upstream, holds the course's surface at its own nearest point,
    /// 11 m, over ground 10.5 m high: its ground stands 1.5 m above a dry cell at 9 m and its water
    /// 2 m. The measure reads the water, and 1.5 m of water would not make a chute: with the course
    /// falling 2 m per metre and the cells a metre apart along it, the bed cell's water stands 2 m
    /// above the course's surface abreast of the dry cell, and water 1.5 m above would stand below
    /// the course's own surface at the bed cell, which no bed holds.
    fn chute_with_a_cell_at(ground_m: f32) -> World {
        let course = course_along_x(|x| 11.0 - 2.0 * x, CourseEnd::Edge);
        let mut world = flat_world(0.0, Water { rivers: vec![course], lakes: Vec::new() });
        for cell in 0..SIZE * SIZE {
            world.heights.data[cell] = 11.0 - 2.0 * ((cell % SIZE) as f32 - MIDDLE as f32);
        }
        wet(&mut world, at(MIDDLE, MIDDLE), 11.0, 0.5);
        world.heights.data[at(MIDDLE + 1, MIDDLE + 1)] = ground_m;
        world
    }

    /// A world of flat ground 10 m high with a lake at 10 m (a lake cell at the middle, 1 m deep)
    /// and the cell east of it dry at `ground_m`.
    fn lake_with_a_dry_cell_at(ground_m: f32) -> World {
        let mut world = flat_world(10.0, Water { rivers: Vec::new(), lakes: vec![lake_at(10.0)] });
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        world.heights.data[at(MIDDLE + 1, MIDDLE)] = ground_m;
        world
    }

    #[test]
    fn a_dry_cell_half_a_metre_below_a_lake_counts_once() {
        let found = unheld(&lake_with_a_dry_cell_at(9.5)).expect("a world with water");
        assert_eq!((found.dry_lake, found.dry_course, found.dry_over_1m), (1, 0, 0), "{found:?}");
        assert!((found.worst_dry_m - 0.5).abs() < 1e-6, "{found:?}");
        assert_eq!(found.worst_dry_at, (1.0, 0.0), "the dry cell lies one metre east of the world's centre");
        assert_eq!((found.lake_over_water, found.water_steps, found.mouth_steps), (0, 0, 0), "{found:?}");
    }

    #[test]
    fn a_dry_cell_a_quarter_of_a_metre_below_a_lake_counts_nowhere() {
        let found = unheld(&lake_with_a_dry_cell_at(9.75)).expect("a world with water");
        assert_eq!(found, Unheld::default());
    }

    #[test]
    fn a_dry_cell_more_than_a_metre_below_a_lake_counts_as_deep() {
        let found = unheld(&lake_with_a_dry_cell_at(8.5)).expect("a world with water");
        assert_eq!((found.dry_lake, found.dry_over_1m), (1, 1), "{found:?}");
        assert!((found.worst_dry_m - 1.5).abs() < 1e-6, "{found:?}");
    }

    #[test]
    fn a_dry_cell_beside_a_chute_level_with_the_courses_surface_there_does_not_count() {
        // Its one wet neighbour, the bed cell upstream, stands 2 m above it, but the course's own
        // surface at the cell's nearest point (9 m at x = 1) is its ground.
        let found = unheld(&chute_with_a_cell_at(9.0)).expect("a world with water");
        assert_eq!(found, Unheld::default());
    }

    #[test]
    fn a_dry_cell_below_the_courses_surface_abreast_of_it_counts_among_the_courses() {
        let found = unheld(&chute_with_a_cell_at(8.5)).expect("a world with water");
        assert_eq!((found.dry_lake, found.dry_course, found.dry_over_1m), (0, 1, 0), "{found:?}");
        assert!((found.worst_dry_m - 0.5).abs() < 1e-6, "{found:?}");
        assert_eq!(found.worst_dry_at, (1.0, 1.0));
    }

    #[test]
    fn a_dry_cell_beside_water_no_course_is_near_counts_against_that_water() {
        // No course within 12 m beyond its half width: the lowest water beside the cell is the water abreast.
        let mut world = flat_world(10.0, Water::default());
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        world.heights.data[at(MIDDLE + 1, MIDDLE)] = 9.0;
        let found = unheld(&world).expect("a world with water");
        assert_eq!((found.dry_lake, found.dry_course), (0, 1), "{found:?}");
    }

    /// A course along `x = 1`, one cell east of the middle, from `z = -4` to `z = 4`, 1 m wide, its
    /// surface `surface_m` throughout: a river that runs past the lake's cell at the middle.
    fn course_beside_the_lake(surface_m: f32) -> RiverCourse {
        let point = |z: f32| CoursePoint { x_m: 1.0, z_m: z, surface_m, width_m: 1.0, depth_m: 0.5 };
        RiverCourse { drawn: Some(0), from_lake: None, points: vec![point(-4.0), point(4.0)], end: CourseEnd::Edge }
    }

    #[test]
    fn a_lake_cell_over_water_beside_it_counts() {
        // The lake's cell at the middle at 10 m, and east of it a cell of a river running past the
        // lake at 9.5 m, as in a world, where the lower water beside a lake is a course's.
        let water = Water { rivers: vec![course_beside_the_lake(9.5)], lakes: vec![lake_at(10.0)] };
        let mut world = flat_world(10.0, water);
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        wet(&mut world, at(MIDDLE + 1, MIDDLE), 9.5, 0.5);
        let found = unheld(&world).expect("a world with water");
        assert_eq!(found.lake_over_water, 1, "{found:?}");
        assert_eq!(found.dry(), 0, "{found:?}");
        // The pair steps 0.5 m over ground that does not step.
        assert_eq!((found.water_steps, found.mouth_steps), (1, 0), "{found:?}");
        assert!((found.worst_step_m - 0.5).abs() < 1e-6, "{found:?}");
    }

    #[test]
    fn a_lake_cell_over_water_is_listed_once_by_the_lowest_water_beside_it() {
        // Two lower cells beside the lake's cell, 0.5 and 0.8 m below its level, and a third 0.2 m.
        let mut world = flat_world(10.0, Water { rivers: Vec::new(), lakes: vec![lake_at(10.0)] });
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        wet(&mut world, at(MIDDLE + 1, MIDDLE), 9.5, 0.5);
        wet(&mut world, at(MIDDLE, MIDDLE + 1), 9.2, 0.5);
        wet(&mut world, at(MIDDLE - 1, MIDDLE), 9.8, 0.5);
        let layer = world.layers[WATER_LAYER].clone();
        let listed = lake_cells_over_water(&world, &layer);
        assert_eq!(listed.len(), 1, "{listed:?}");
        assert_eq!(listed[0].0, (0.0, 0.0), "the lake's cell lies at the world's centre");
        assert!((listed[0].1 - 0.8).abs() < 1e-5, "{listed:?}");
        assert_eq!(unheld(&world).expect("a world with water").lake_over_water, 1);
    }

    #[test]
    fn a_lake_cell_a_quarter_of_a_metre_over_water_beside_it_counts_nowhere() {
        let mut world = flat_world(10.0, Water { rivers: Vec::new(), lakes: vec![lake_at(10.0)] });
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        wet(&mut world, at(MIDDLE + 1, MIDDLE), 9.75, 0.75);
        assert_eq!(unheld(&world).expect("a world with water"), Unheld::default());
    }

    #[test]
    fn a_step_between_cells_within_reach_of_a_course_and_the_one_it_ends_on_is_a_mouth_step() {
        let ends_on_the_other = course_along_x(|_| 10.0, CourseEnd::River(1));
        let other = course_along_x(|_| 10.0, CourseEnd::Edge);
        let mut world = flat_world(10.0, Water { rivers: vec![ends_on_the_other, other], lakes: Vec::new() });
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        wet(&mut world, at(MIDDLE + 1, MIDDLE), 9.5, 0.5);
        let found = unheld(&world).expect("a world with water");
        assert_eq!((found.water_steps, found.mouth_steps), (0, 1), "{found:?}");
        assert!((found.worst_step_m - 0.5).abs() < 1e-6, "the worst step is over both counts: {found:?}");
    }

    #[test]
    fn a_world_without_the_water_layer_gives_none() {
        let mut world = flat_world(10.0, Water::default());
        world.layers.clear();
        assert_eq!(unheld(&world), None);
        assert_eq!(unheld_cells(&world), None);
        assert!(steps_beyond_the_ground(&world).is_empty());
    }

    /// The places behind the counts: a lake's cell over a river's cell beside it (a lake over water
    /// and a step), and a dry cell half a metre below the lake west of it.
    #[test]
    fn the_cells_unheld_counts_are_given_with_their_places() {
        let water = Water { rivers: vec![course_beside_the_lake(9.5)], lakes: vec![lake_at(10.0)] };
        let mut world = flat_world(10.0, water);
        wet(&mut world, at(MIDDLE, MIDDLE), 10.0, 1.0);
        wet(&mut world, at(MIDDLE + 1, MIDDLE), 9.5, 0.5);
        world.heights.data[at(MIDDLE - 1, MIDDLE)] = 9.5;
        let cells = unheld_cells(&world).expect("a world with water");
        assert_eq!(cells.counts(), unheld(&world).expect("a world with water"));
        assert_eq!(cells.dry.len(), 1, "{cells:?}");
        assert_eq!((cells.dry[0].at, cells.dry[0].at_lake), ((-1.0, 0.0), true), "{cells:?}");
        assert!((cells.dry[0].below_m - 0.5).abs() < 1e-6, "{cells:?}");
        assert_eq!(cells.lake_over_water.len(), 1, "{cells:?}");
        assert_eq!(cells.lake_over_water[0].at, (0.0, 0.0));
        assert_eq!(cells.steps.len(), 1, "{cells:?}");
        assert_eq!((cells.steps[0].a, cells.steps[0].b, cells.steps[0].at_mouth), ((0.0, 0.0), (1.0, 0.0), false));
        let counts = cells.counts();
        assert_eq!((counts.dry_lake, counts.lake_over_water, counts.water_steps), (1, 1, 1), "{counts:?}");
    }

    #[test]
    fn a_world_with_no_water_holds_nothing_unheld() {
        assert_eq!(unheld(&flat_world(10.0, Water::default())), Some(Unheld::default()));
    }
}
