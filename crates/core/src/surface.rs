//! The surface of a world's water as triangles, built from the world's heights and its `water`
//! layer, in pieces a mesh can hold, each point with the flow of the course whose water it is and
//! that course's fall there.

use crate::error::LandformError;
use crate::grid::Grid;
use crate::water::{CourseEnd, RiverCourse, Water};
use rayon::prelude::*;

/// The side of one piece of the surface, in cells.
pub const SURFACE_PIECE_CELLS: usize = 128;

/// How far under its own ground a rim point stands at most, in metres, so that the surface's edge
/// is hidden by the ground.
const RIM_UNDER_M: f32 = 0.05;

/// A point of a piece's block that no square of the surface uses.
const NO_POINT: u32 = u32::MAX;

/// How far beyond a course's waterline its flow reaches, in metres: a point farther beyond every
/// course's waterline is a point of no course. The flow fades from the waterline to nothing here.
const FLOW_REACH_M: f64 = 12.0;

/// The roughness of the open-channel (Manning) formula the speed follows.
const FLOW_ROUGHNESS: f64 = 0.035;

/// The least speed of a point of a course before the fades, in metres per second.
const FLOW_SPEED_MIN: f64 = 0.05;

/// The most speed of a point of a course, in metres per second.
const FLOW_SPEED_MAX: f64 = 4.0;

/// How many times the velocity is smoothed, each point taking the mean of itself and its four
/// neighbours that are points of the surface, so that it turns gently through a junction.
const FLOW_SMOOTH_PASSES: usize = 2;

/// How far along a course on either side of a place the points reach whose drop gives the slope
/// of the speed's formula, in metres, at the least: where the water is wide the reach is
/// `SPEED_REACH_WIDTHS` widths of it.
const SLOPE_REACH_M: f64 = 8.0;

/// The slope a course's speed is worked out from is read as far as this many widths of its water
/// either side of the place, and at least `SLOPE_REACH_M`.
const SPEED_REACH_WIDTHS: f64 = 4.0;

/// The least fall a course's water is read as having when its speed is worked out: half a metre
/// in a kilometre. A river carries its water through a level reach too.
const FLOW_SLOPE_MIN: f64 = 0.0005;

/// How far along a course on either side of a place the points reach whose drop gives the fall,
/// in metres.
const FALL_REACH_M: f64 = 4.0;

/// Over how many metres a course flowing out of a lake gathers its speed from nothing, and a
/// course ending in a lake loses it.
const LAKE_FADE_M: f64 = 12.0;

/// A course loses its fall over this many metres at an end where it meets other water: after its
/// first point when it comes out of a lake, before its last when it ends in a lake or on another
/// course. The water beyond that end is the other water's, and stores no fall of this course.
const FALL_END_FADE_M: f64 = 8.0;

/// The side of a square bucket of the courses' stretches (`CourseIndex`), in metres.
const BUCKET_M: f64 = 16.0;

/// How much farther than its reach a stretch is put in buckets, in metres, so that no rounding of
/// a distance near the reach loses it.
const BUCKET_SLACK_M: f64 = 1.0;

/// A world's water as triangles, in the world's frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterSurface {
    /// Only pieces that hold water, by row and then by column.
    pub pieces: Vec<SurfacePiece>,
}

/// One piece of the water's surface: the squares of the ground whose lowest corner lies in a block
/// of `SURFACE_PIECE_CELLS` cells a side.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacePiece {
    /// The piece's column and row: it covers cells `column * 128 ..` and `row * 128 ..`.
    pub column: usize,
    pub row: usize,
    /// `(x, y, z)` in metres: `x` and `z` from the world's centre, as a course point's; `y` in the
    /// frame of the heights.
    pub positions: Vec<[f32; 3]>,
    /// The water's depth at each point; 0 at a rim point.
    pub depth_m: Vec<f32>,
    /// The flow at each point, `(x, z)` in metres per second: along the course whose water the
    /// point is, at the open-channel formula's speed from the point's own depth and the course's
    /// slope over a long stretch, never less than a lowland river's, kept between 0.05 and 4 m/s
    /// and faded beyond the waterline and at a lake; `(0, 0)` for a point of no course;
    /// then smoothed with the points around it, so it turns gently through a junction.
    pub velocity: Vec<[f32; 2]>,
    /// How steeply the surface falls along the flow at each point, as a ratio: the drop per metre
    /// of the course whose water the point is, about the point's place on its line, 0 or more,
    /// faded to nothing beyond the course's waterline and where the course meets other water (its
    /// first and last `FALL_END_FADE_M` at a lake or another course, but not where a river goes on
    /// into a course of its own drawn river); 0 for a point of no course.
    pub fall: Vec<f32>,
    /// Three indices into this piece's points per triangle.
    pub indices: Vec<u32>,
}

/// The surface of the water `depth` marks over `heights`, its flow and its fall from the courses
/// of `water`. An error when the two grids differ in size or are not square. Call it inside the
/// caller's own rayon pool.
///
/// A point's water is a course's: of each course's nearest place to the point on its line, the
/// one whose distance beyond the course's waterline (the distance less half the water's width
/// there, negative inside the bed) is the smallest, the lower course on a tie, if it is no more than
/// `FLOW_REACH_M`; otherwise the point is of no course, with no flow and no fall. Its flow runs
/// along the stretch of the course the place lies on, at `depth^(2/3) * sqrt(slope) /
/// FLOW_ROUGHNESS` from the point's own depth and the course's drop per metre between its points
/// as far as `SLOPE_REACH_M` or `SPEED_REACH_WIDTHS` widths of its water there, the greater, either
/// side of the place, and at least `FLOW_SLOPE_MIN`, kept between `FLOW_SPEED_MIN` and
/// `FLOW_SPEED_MAX`, then faded by three smooth steps: from the waterline to nothing at
/// `FLOW_REACH_M` beyond it, from nothing over the first `LAKE_FADE_M` of a course out of a lake,
/// and to nothing over the last `LAKE_FADE_M` of a course into one. Its fall is the course's drop
/// per metre between its points as far as `FALL_REACH_M` either side, times the waterline's fade,
/// the start's (a smooth step over the first `FALL_END_FADE_M` of a course out of a lake) and the
/// end's (over the last `FALL_END_FADE_M` of a course into a lake or on a course of another river,
/// not on one of its own drawn river, which is a lake left out). Then the velocity is
/// smoothed `FLOW_SMOOTH_PASSES` times, each point taking the mean of itself and its four
/// neighbours that are points of the surface; the fall is not.
pub fn water_surface(heights: &Grid<f32>, depth: &Grid<f32>, water: &Water) -> Result<WaterSurface, LandformError> {
    let size = heights.width;
    let cells = size * size;
    if heights.height != size || depth.width != size || depth.height != size || heights.data.len() != cells || depth.data.len() != cells {
        return Err(LandformError::BadConfig(format!(
            "the heights ({} by {} cells) and the water's depth ({} by {} cells) must be square grids of one size",
            heights.width, heights.height, depth.width, depth.height
        )));
    }
    let across = size.saturating_sub(1).div_ceil(SURFACE_PIECE_CELLS);
    let courses = CourseIndex::new(water);
    // Each piece is built from the whole-world grids and courses alone, so the split does not show
    // in the result.
    let pieces: Vec<Option<SurfacePiece>> = (0..across * across)
        .into_par_iter()
        .map(|index| build_piece(heights, depth, &courses, index % across, index / across))
        .collect();
    Ok(WaterSurface { pieces: pieces.into_iter().flatten().collect() })
}

/// The piece at `column` and `row`, or `None` when no square of it has a wet corner.
fn build_piece(heights: &Grid<f32>, depth: &Grid<f32>, courses: &CourseIndex, column: usize, row: usize) -> Option<SurfacePiece> {
    let size = heights.width;
    let (first_i, first_j) = (column * SURFACE_PIECE_CELLS, row * SURFACE_PIECE_CELLS);
    // The squares of the piece start at columns `first_i..end_i`; their corners reach `end_i`.
    let (end_i, end_j) = ((first_i + SURFACE_PIECE_CELLS).min(size - 1), (first_j + SURFACE_PIECE_CELLS).min(size - 1));
    let width = end_i - first_i + 1;
    let slot_of = |i: usize, j: usize| (j - first_j) * width + (i - first_i);
    let is_wet = |i: usize, j: usize| depth.data[j * size + i] > 0.0;

    // The squares with a wet corner, by row and then by column, and the points they use.
    let mut slots = vec![NO_POINT; width * (end_j - first_j + 1)];
    let mut squares: Vec<(usize, usize)> = Vec::new();
    for j in first_j..end_j {
        for i in first_i..end_i {
            if is_wet(i, j) || is_wet(i + 1, j) || is_wet(i, j + 1) || is_wet(i + 1, j + 1) {
                squares.push((i, j));
                for (corner_i, corner_j) in [(i, j), (i + 1, j), (i, j + 1), (i + 1, j + 1)] {
                    slots[slot_of(corner_i, corner_j)] = 0;
                }
            }
        }
    }
    if squares.is_empty() {
        return None;
    }

    // Number the used points by row and then by column, and give each its place, its depth, its
    // flow and its fall.
    let half_m = (size / 2) as f32;
    let cell_m = heights.cell_size;
    let flow = PieceFlow::new(depth, f64::from(cell_m), courses, (first_i, first_j), (end_i, end_j));
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut depth_m: Vec<f32> = Vec::new();
    let mut velocity: Vec<[f32; 2]> = Vec::new();
    let mut fall: Vec<f32> = Vec::new();
    for j in first_j..=end_j {
        for i in first_i..=end_i {
            let slot = &mut slots[slot_of(i, j)];
            if *slot == NO_POINT {
                continue;
            }
            *slot = positions.len() as u32;
            let (y, point_depth) = point_height(heights, depth, i, j);
            positions.push([(i as f32 - half_m) * cell_m, y, (j as f32 - half_m) * cell_m]);
            depth_m.push(point_depth);
            let (point_velocity, point_fall) = flow.at(i, j);
            velocity.push(point_velocity);
            fall.push(point_fall);
        }
    }

    // Two triangles a square, both clockwise seen from above: x to the right, z down the page.
    let mut indices: Vec<u32> = Vec::with_capacity(squares.len() * 6);
    for &(i, j) in &squares {
        let (a, b, c, d) = (slots[slot_of(i, j)], slots[slot_of(i + 1, j)], slots[slot_of(i, j + 1)], slots[slot_of(i + 1, j + 1)]);
        indices.extend([a, b, c, b, d, c]);
    }
    Some(SurfacePiece { column, row, positions, depth_m, velocity, fall, indices })
}

/// The height and the depth of the surface's point at cell `(i, j)`: a wet cell's ground plus its
/// depth; for a dry one, a rim point, the lowest water among the eight cells around it, and never
/// above its own ground less `RIM_UNDER_M`.
fn point_height(heights: &Grid<f32>, depth: &Grid<f32>, i: usize, j: usize) -> (f32, f32) {
    let size = heights.width;
    let (ground, water) = (heights.data[j * size + i], depth.data[j * size + i]);
    if water > 0.0 {
        return (ground + water, water);
    }
    let mut lowest = f32::INFINITY;
    for near_j in j.saturating_sub(1)..=(j + 1).min(size - 1) {
        for near_i in i.saturating_sub(1)..=(i + 1).min(size - 1) {
            let near = near_j * size + near_i;
            if (near_i, near_j) != (i, j) && depth.data[near] > 0.0 {
                lowest = lowest.min(heights.data[near] + depth.data[near]);
            }
        }
    }
    (lowest.min(ground - RIM_UNDER_M), 0.0)
}

/// Whether cell `(i, j)` is a point of the surface: a corner of a square with a wet corner, so a
/// cell that is wet or has a wet cell among the eight around it.
fn is_surface_point(depth: &Grid<f32>, i: usize, j: usize) -> bool {
    let size = depth.width;
    (j.saturating_sub(1)..=(j + 1).min(size - 1))
        .any(|near_j| (i.saturating_sub(1)..=(i + 1).min(size - 1)).any(|near_i| depth.data[near_j * size + near_i] > 0.0))
}

/// The flow of the points of one piece's block, worked out over the block and the points within
/// `FLOW_SMOOTH_PASSES` of it, its window: each pass of the smoothing reads the points beside a
/// point, so after the passes the block's points hold exactly what a pass over the whole world
/// gives them, and a point that two pieces share holds the same flow in both.
struct PieceFlow {
    /// The window's first column and row, and its width in points.
    first: (usize, usize),
    width: usize,
    /// The smoothed velocity and the fall of each point of the window, by row and then by column;
    /// zero where the window holds no point of the surface.
    velocity: Vec<[f64; 2]>,
    fall: Vec<f64>,
}

impl PieceFlow {
    /// The flow of the block from `block_first` to `block_end` (its first and last column and
    /// row) of a world of cells `cell_m` metres a side, whose water `depth` marks and `courses`
    /// runs along.
    fn new(depth: &Grid<f32>, cell_m: f64, courses: &CourseIndex, block_first: (usize, usize), block_end: (usize, usize)) -> PieceFlow {
        let last = depth.width - 1;
        let first = (block_first.0.saturating_sub(FLOW_SMOOTH_PASSES), block_first.1.saturating_sub(FLOW_SMOOTH_PASSES));
        let end = ((block_end.0 + FLOW_SMOOTH_PASSES).min(last), (block_end.1 + FLOW_SMOOTH_PASSES).min(last));
        let (width, height) = (end.0 - first.0 + 1, end.1 - first.1 + 1);
        let mut is_point = vec![false; width * height];
        let mut velocity = vec![[0.0; 2]; width * height];
        let mut fall = vec![0.0; width * height];
        for j in first.1..=end.1 {
            for i in first.0..=end.0 {
                if !is_surface_point(depth, i, j) {
                    continue;
                }
                let slot = (j - first.1) * width + (i - first.0);
                is_point[slot] = true;
                let flow = point_flow(depth, cell_m, courses, i, j);
                velocity[slot] = flow.velocity();
                fall[slot] = flow.fall;
            }
        }
        // A point at the window's edge misses the neighbours beyond it, and what it holds is wrong
        // by then; each pass carries that one point farther in, and the block lies as many points
        // in as there are passes.
        for _ in 0..FLOW_SMOOTH_PASSES {
            let before = velocity.clone();
            for row in 0..height {
                for column in 0..width {
                    let slot = row * width + column;
                    if !is_point[slot] {
                        continue;
                    }
                    let (mut sum, mut count) = (before[slot], 1.0);
                    let beside =
                        [(column > 0).then(|| slot - 1), (column + 1 < width).then(|| slot + 1), (row > 0).then(|| slot - width), (row + 1 < height).then(|| slot + width)];
                    for near in beside.into_iter().flatten().filter(|&near| is_point[near]) {
                        sum = [sum[0] + before[near][0], sum[1] + before[near][1]];
                        count += 1.0;
                    }
                    velocity[slot] = [sum[0] / count, sum[1] / count];
                }
            }
        }
        PieceFlow { first, width, velocity, fall }
    }

    /// The velocity and the fall of the point at cell `(i, j)` of the block.
    fn at(&self, i: usize, j: usize) -> ([f32; 2], f32) {
        let slot = (j - self.first.1) * self.width + (i - self.first.0);
        (self.velocity[slot].map(|v| v as f32), self.fall[slot] as f32)
    }
}

/// What the point of the surface at cell `(i, j)` takes from its course before the smoothing: the
/// flow and the fall at its place, world `((i - size / 2) * cell_m, (j - size / 2) * cell_m)`, with
/// its own depth, 0 at a rim point.
fn point_flow(depth: &Grid<f32>, cell_m: f64, courses: &CourseIndex, i: usize, j: usize) -> PointFlow {
    let half = (depth.width / 2) as f64;
    let water = depth.data[j * depth.width + i];
    let own_m = if water > 0.0 { f64::from(water) } else { 0.0 };
    courses.flow_at(((i as f64 - half) * cell_m, (j as f64 - half) * cell_m), own_m)
}

/// What a point of the surface takes from the course whose water it is, before the smoothing.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PointFlow {
    /// The course's direction of flow at the point's place: a unit vector, or `(0, 0)` along a
    /// course of no length.
    direction: [f64; 2],
    /// The speed, its fades included, in metres per second.
    speed: f64,
    /// The course's fall at the place, as a ratio, faded where the water does not run.
    fall: f64,
}

impl PointFlow {
    /// A point of no course: no flow and no fall.
    const STILL: PointFlow = PointFlow { direction: [0.0; 2], speed: 0.0, fall: 0.0 };

    /// The flow as a velocity, `(x, z)` in metres per second.
    fn velocity(&self) -> [f64; 2] {
        [self.direction[0] * self.speed, self.direction[1] * self.speed]
    }
}

/// The place on a course's line nearest a point.
#[derive(Debug, Clone, Copy)]
struct Place {
    /// The course's index in `Water::rivers`.
    course: usize,
    /// The stretch the place lies on: from the course's point of this index to the next.
    stretch: usize,
    /// How far along the stretch the place lies, as a share of its length; 0 on a stretch of no
    /// length.
    share: f64,
    /// How far beyond the course's waterline the point lies, in metres; negative inside the bed.
    beyond_m: f64,
}

/// One course as `CourseIndex` reads it, in `f64`.
struct CourseLine<'a> {
    course: &'a RiverCourse,
    /// Each point's world `(x, z)`, in metres.
    points: Vec<[f64; 2]>,
    /// How far along the course each point lies, in metres from its first.
    along_m: Vec<f64>,
    /// The direction of flow of each stretch, from its point to the next, as a unit vector. A
    /// stretch of no length takes the direction of the next stretch downstream that has a length,
    /// or else of the last one upstream; a course of no length has `(0, 0)`.
    directions: Vec<[f64; 2]>,
    /// How far from the line its water's flow reaches, in metres: `FLOW_REACH_M` beyond its widest
    /// water's waterline.
    reach_m: f64,
}

impl<'a> CourseLine<'a> {
    fn new(course: &'a RiverCourse) -> CourseLine<'a> {
        let points: Vec<[f64; 2]> = course.points.iter().map(|point| [f64::from(point.x_m), f64::from(point.z_m)]).collect();
        let mut along_m = Vec::with_capacity(points.len());
        let mut found: Vec<Option<[f64; 2]>> = Vec::new();
        for (k, point) in points.iter().enumerate() {
            let Some(before) = k.checked_sub(1).map(|before| points[before]) else {
                along_m.push(0.0);
                continue;
            };
            let length_m = (point[0] - before[0]).hypot(point[1] - before[1]);
            along_m.push(along_m[k - 1] + length_m);
            found.push((length_m > 0.0).then(|| [(point[0] - before[0]) / length_m, (point[1] - before[1]) / length_m]));
        }
        // A course of one point has one stretch, from that point to itself.
        if points.len() == 1 {
            found.push(None);
        }
        let mut downstream = None;
        for direction in found.iter_mut().rev() {
            match direction {
                Some(_) => downstream = *direction,
                None => *direction = downstream,
            }
        }
        let mut upstream = None;
        for direction in &mut found {
            match direction {
                Some(_) => upstream = *direction,
                None => *direction = upstream,
            }
        }
        let widest_m = course.points.iter().map(|point| f64::from(point.width_m)).fold(0.0, f64::max);
        CourseLine {
            course,
            points,
            along_m,
            directions: found.into_iter().map(|direction| direction.unwrap_or([0.0; 2])).collect(),
            reach_m: FLOW_REACH_M + widest_m / 2.0,
        }
    }

    /// The points stretch `stretch` runs between: its own and the next, or its own twice on a
    /// course of one point.
    fn ends(&self, stretch: usize) -> (usize, usize) {
        (stretch, (stretch + 1).min(self.points.len() - 1))
    }

    /// The distance from `(x, z)` to the nearest place on stretch `stretch`, in metres, and how far
    /// along the stretch that place lies, as a share of its length (0 on a stretch of no length).
    fn distance_to(&self, stretch: usize, (x, z): (f64, f64)) -> (f64, f64) {
        let (a, b) = self.ends(stretch);
        let ([ax, az], [bx, bz]) = (self.points[a], self.points[b]);
        let (dx, dz) = (bx - ax, bz - az);
        let squared = dx * dx + dz * dz;
        let share = if squared > 0.0 { (((x - ax) * dx + (z - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        ((ax + dx * share - x).hypot(az + dz * share - z), share)
    }

    /// Half the water's width at the place `share` along stretch `stretch`, in metres, between its
    /// points' widths.
    fn half_width_m(&self, stretch: usize, share: f64) -> f64 {
        let (a, b) = self.ends(stretch);
        let (from, to) = (f64::from(self.course.points[a].width_m), f64::from(self.course.points[b].width_m));
        (from + (to - from) * share) / 2.0
    }

    /// The course's drop per metre along it between its points that lie farthest from the place
    /// `along_m` metres along it, on stretch `stretch`, but no more than `reach_m` either side, or
    /// the stretch's own points where they lie farther: fewer at an end. 0 or more, and 0 when those
    /// points lie at one place.
    fn drop_per_m(&self, stretch: usize, along_m: f64, reach_m: f64) -> f64 {
        let (mut up, mut down) = self.ends(stretch);
        let last = self.points.len() - 1;
        while up > 0 && self.along_m[up - 1] >= along_m - reach_m {
            up -= 1;
        }
        while down < last && self.along_m[down + 1] <= along_m + reach_m {
            down += 1;
        }
        let length_m = self.along_m[down] - self.along_m[up];
        if length_m <= 0.0 {
            return 0.0;
        }
        let drop_m = f64::from(self.course.points[up].surface_m) - f64::from(self.course.points[down].surface_m);
        (drop_m / length_m).max(0.0)
    }
}

/// The courses of a world's water, laid out for finding whose water a point is. Their stretches
/// are sorted into square buckets `BUCKET_M` a side, each listing the stretches whose course's reach
/// (`CourseLine::reach_m`, and `BUCKET_SLACK_M` more) from them overlaps it, so a point reads only
/// its own bucket and still finds each course's nearest place wherever that course may claim it.
struct CourseIndex<'a> {
    lines: Vec<CourseLine<'a>>,
    /// The world `(x, z)` of the first bucket's corner, in metres, and the buckets along each axis.
    origin: (f64, f64),
    across: (usize, usize),
    /// Where each bucket's stretches start in `stretches`, by row and then by column, and where
    /// the last one's end.
    starts: Vec<usize>,
    /// Each bucket's stretches, as course and stretch, by course and then by stretch.
    stretches: Vec<(u32, u32)>,
    /// Whether each course ends where it meets other water, so that its fall fades before its last
    /// point (`FALL_END_FADE_M`): in a lake, or at the mouth of a course that ends on another. A
    /// course that ends on a course of its own drawn river is that river going on, not a tributary
    /// meeting it: a drawn lake left out ends the course into it on the course out of it
    /// (`CourseEnd::River`, `courses` in `water.rs`), and the water goes on over its ground, so
    /// its fall is not faded. A drawn river meets no other course of its own anywhere else.
    meets_water: Vec<bool>,
}

impl<'a> CourseIndex<'a> {
    fn new(water: &'a Water) -> CourseIndex<'a> {
        let lines: Vec<CourseLine> = water.rivers.iter().map(CourseLine::new).collect();
        let meets_water = (water.rivers.iter())
            .map(|course| match course.end {
                CourseEnd::Edge => false,
                CourseEnd::Lake(_) => true,
                CourseEnd::River(out) => water.rivers.get(out).is_none_or(|out| course.drawn.is_none() || course.drawn != out.drawn),
            })
            .collect();
        // Each stretch, with the box its course's reach covers from it: west, north, east, south. A
        // course of no points has no stretch.
        let mut boxes: Vec<(u32, u32, [f64; 4])> = Vec::new();
        for (course, line) in lines.iter().enumerate() {
            let reach_m = line.reach_m + BUCKET_SLACK_M;
            for stretch in 0..line.directions.len() {
                let (a, b) = line.ends(stretch);
                let ([ax, az], [bx, bz]) = (line.points[a], line.points[b]);
                boxes.push((course as u32, stretch as u32, [ax.min(bx) - reach_m, az.min(bz) - reach_m, ax.max(bx) + reach_m, az.max(bz) + reach_m]));
            }
        }
        let corner = |side: usize, pick: fn(f64, f64) -> f64, start: f64| boxes.iter().map(|(_, _, bounds)| bounds[side]).fold(start, pick);
        let origin = (corner(0, f64::min, f64::INFINITY), corner(1, f64::min, f64::INFINITY));
        let far = (corner(2, f64::max, f64::NEG_INFINITY), corner(3, f64::max, f64::NEG_INFINITY));
        let across = if boxes.is_empty() {
            (0, 0)
        } else {
            (((far.0 - origin.0) / BUCKET_M).floor() as usize + 1, ((far.1 - origin.1) / BUCKET_M).floor() as usize + 1)
        };
        // The buckets a box overlaps, by row and then by column.
        let buckets_of = |bounds: &[f64; 4]| {
            // The bucket a coordinate falls in along one axis, a column along `x` and a row along `z`.
            let bucket_along = |at: f64, from: f64| ((at - from) / BUCKET_M).floor() as usize;
            let (columns, rows) = (bucket_along(bounds[0], origin.0)..=bucket_along(bounds[2], origin.0), bucket_along(bounds[1], origin.1)..=bucket_along(bounds[3], origin.1));
            rows.flat_map(move |row| columns.clone().map(move |column| row * across.0 + column))
        };
        let mut starts = vec![0usize; across.0 * across.1 + 1];
        for (_, _, bounds) in &boxes {
            for bucket in buckets_of(bounds) {
                starts[bucket + 1] += 1;
            }
        }
        for bucket in 0..across.0 * across.1 {
            starts[bucket + 1] += starts[bucket];
        }
        let mut filled = starts.clone();
        let mut stretches = vec![(0, 0); starts[across.0 * across.1]];
        for &(course, stretch, ref bounds) in &boxes {
            for bucket in buckets_of(bounds) {
                stretches[filled[bucket]] = (course, stretch);
                filled[bucket] += 1;
            }
        }
        CourseIndex { lines, origin, across, starts, stretches, meets_water }
    }

    /// The place on the line of the course whose water world `(x, z)` is, as `water_surface`
    /// describes it, or `None` for a point of no course. On a course whose stretches lie equally
    /// near, the place is on the first of them.
    fn place_of(&self, (x, z): (f64, f64)) -> Option<Place> {
        let (column, row) = (((x - self.origin.0) / BUCKET_M).floor(), ((z - self.origin.1) / BUCKET_M).floor());
        if !(column >= 0.0 && row >= 0.0 && column < self.across.0 as f64 && row < self.across.1 as f64) {
            return None;
        }
        let bucket = row as usize * self.across.0 + column as usize;
        let mut best: Option<Place> = None;
        for of_course in self.stretches[self.starts[bucket]..self.starts[bucket + 1]].chunk_by(|a, b| a.0 == b.0) {
            let (course, line) = (of_course[0].0 as usize, &self.lines[of_course[0].0 as usize]);
            let mut nearest = (f64::INFINITY, 0, 0.0);
            for &(_, stretch) in of_course {
                let (distance_m, share) = line.distance_to(stretch as usize, (x, z));
                if distance_m < nearest.0 {
                    nearest = (distance_m, stretch as usize, share);
                }
            }
            let (distance_m, stretch, share) = nearest;
            let beyond_m = distance_m - line.half_width_m(stretch, share);
            if beyond_m <= FLOW_REACH_M && best.is_none_or(|best| beyond_m < best.beyond_m) {
                best = Some(Place { course, stretch, share, beyond_m });
            }
        }
        best
    }

    /// The flow and the fall at world `(x, z)`, with water `own_m` deep there, before the
    /// smoothing, as `water_surface` describes them.
    fn flow_at(&self, (x, z): (f64, f64), own_m: f64) -> PointFlow {
        let Some(place) = self.place_of((x, z)) else { return PointFlow::STILL };
        let line = &self.lines[place.course];
        let (a, b) = line.ends(place.stretch);
        let along_m = line.along_m[a] + place.share * (line.along_m[b] - line.along_m[a]);
        let width_m = 2.0 * line.half_width_m(place.stretch, place.share);
        let reach_m = SLOPE_REACH_M.max(SPEED_REACH_WIDTHS * width_m);
        let slope = line.drop_per_m(place.stretch, along_m, reach_m).max(FLOW_SLOPE_MIN);
        let speed = (own_m.powf(2.0 / 3.0) * slope.sqrt() / FLOW_ROUGHNESS).clamp(FLOW_SPEED_MIN, FLOW_SPEED_MAX);
        let waterline = 1.0 - smooth_step(place.beyond_m / FLOW_REACH_M);
        let from_lake = if line.course.from_lake.is_some() { smooth_step(along_m / LAKE_FADE_M) } else { 1.0 };
        let length_m = line.along_m[line.points.len() - 1];
        let into_lake = if matches!(line.course.end, CourseEnd::Lake(_)) { smooth_step((length_m - along_m) / LAKE_FADE_M) } else { 1.0 };
        // The fall is faded where the course meets other water: its start's fade only on a course out
        // of a lake, its end's on a course into a lake or at a mouth on another course. The distance left to the
        // last point is taken from the last point itself, so it is exactly 0 at the end.
        let from_water = if line.course.from_lake.is_some() { smooth_step(along_m / FALL_END_FADE_M) } else { 1.0 };
        let into_water = if self.meets_water[place.course] {
            let left_m = (length_m - line.along_m[b]) + (1.0 - place.share) * (line.along_m[b] - line.along_m[a]);
            smooth_step(left_m / FALL_END_FADE_M)
        } else {
            1.0
        };
        PointFlow {
            direction: line.directions[place.stretch],
            speed: speed * waterline * from_lake * into_lake,
            fall: line.drop_per_m(place.stretch, along_m, FALL_REACH_M) * waterline * from_water * into_water,
        }
    }
}

/// 0 up to 0, 1 from 1 on, and a smooth step between, flat at both ends.
fn smooth_step(share: f64) -> f64 {
    let t = share.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Drawing;
    use crate::water::{CourseEnd, CoursePoint, LakeWater, RiverCourse, WATER_LAYER};
    use crate::world::{generate_drawn, GenConfig};
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::AtomicBool;

    const RIM_UNDER: f32 = 0.05;

    /// A point's position and depth as bits.
    type PointBits = ([u32; 3], u32);

    /// A `size` square grid of one metre cells with `f(i, j)` in cell `(i, j)`.
    fn grid(size: usize, f: impl Fn(usize, usize) -> f32) -> Grid<f32> {
        let mut g = Grid::new(size, size, 1.0);
        for j in 0..size {
            for i in 0..size {
                g.set(i, j, f(i, j));
            }
        }
        g
    }

    /// A ground that is not flat, so a wrong cell shows.
    fn rolling(i: usize, j: usize) -> f32 {
        20.0 + 0.37 * i as f32 - 0.11 * j as f32 + ((i * 7 + j * 3) % 5) as f32 * 0.2
    }

    fn surface(heights: &Grid<f32>, depth: &Grid<f32>) -> WaterSurface {
        water_surface(heights, depth, &Water::default()).expect("matching grids give a surface")
    }

    /// The cell a position stands at, in a grid of `size` cells.
    fn cell_of(position: [f32; 3], size: usize) -> (usize, usize) {
        let half = (size / 2) as f32;
        ((position[0] + half) as usize, (position[2] + half) as usize)
    }

    /// The squares with a wet corner, counted straight from the depth.
    fn squares_with_water(depth: &Grid<f32>) -> usize {
        let size = depth.width;
        let wet = |i: usize, j: usize| depth.get(i, j) > 0.0;
        let mut count = 0;
        for j in 0..size - 1 {
            for i in 0..size - 1 {
                if wet(i, j) || wet(i + 1, j) || wet(i, j + 1) || wet(i + 1, j + 1) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Each square of the surface, as its lowest corner's cell, with the piece of each of its
    /// triangles.
    fn squares_of(surface: &WaterSurface, size: usize) -> BTreeMap<(usize, usize), Vec<(usize, usize)>> {
        let mut squares: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for piece in &surface.pieces {
            for triangle in piece.indices.chunks(3) {
                let cells = triangle.iter().map(|&p| cell_of(piece.positions[p as usize], size));
                let corner = cells.fold((usize::MAX, usize::MAX), |(i, j), (ci, cj)| (i.min(ci), j.min(cj)));
                squares.entry(corner).or_default().push((piece.column, piece.row));
            }
        }
        squares
    }

    /// A 300 cell world with a pool across the pieces' lines.
    fn pool_world() -> (Grid<f32>, Grid<f32>) {
        let heights = grid(300, rolling);
        let depth = grid(300, |i, j| {
            let (di, dj) = (i as f32 - 140.0, j as f32 - 130.0);
            if di * di + dj * dj < 45.0 * 45.0 {
                0.25 + 0.01 * (i % 7) as f32
            } else {
                0.0
            }
        });
        (heights, depth)
    }

    #[test]
    fn a_wet_point_stands_at_its_ground_plus_its_depth() {
        let (heights, depth) = pool_world();
        let surface = surface(&heights, &depth);
        let mut wet_points = 0;
        for piece in &surface.pieces {
            for (position, &d) in piece.positions.iter().zip(&piece.depth_m) {
                let (i, j) = cell_of(*position, 300);
                assert_eq!(d.to_bits(), depth.get(i, j).max(0.0).to_bits(), "depth at ({i}, {j})");
                if depth.get(i, j) > 0.0 {
                    wet_points += 1;
                    let expected = heights.get(i, j) + depth.get(i, j);
                    assert_eq!(position[1].to_bits(), expected.to_bits(), "height at ({i}, {j})");
                }
            }
        }
        assert!(wet_points > 1000, "only {wet_points} wet points");
    }

    #[test]
    fn the_surface_holds_each_square_with_a_wet_corner_once() {
        let (heights, depth) = pool_world();
        let surface = surface(&heights, &depth);
        let squares = squares_of(&surface, 300);
        assert_eq!(squares.len(), squares_with_water(&depth));
        for (corner, holders) in &squares {
            assert_eq!(holders.len(), 2, "square {corner:?} has {} triangles", holders.len());
            assert_eq!(holders[0], holders[1], "square {corner:?} lies in two pieces");
        }
        let triangles: usize = surface.pieces.iter().map(|p| p.indices.len() / 3).sum();
        assert_eq!(triangles, 2 * squares_with_water(&depth));
        assert!(surface.pieces.len() >= 4, "the pool reaches {} pieces", surface.pieces.len());
    }

    #[test]
    fn every_triangle_turns_the_same_way_and_stays_in_its_piece() {
        let (heights, depth) = pool_world();
        let surface = surface(&heights, &depth);
        for piece in &surface.pieces {
            let count = piece.positions.len();
            assert_eq!(piece.depth_m.len(), count);
            assert_eq!(piece.velocity.len(), count);
            assert_eq!(piece.fall.len(), count);
            assert_eq!(piece.indices.len() % 3, 0);
            let (first_i, first_j) = (piece.column * SURFACE_PIECE_CELLS, piece.row * SURFACE_PIECE_CELLS);
            for position in &piece.positions {
                let (i, j) = cell_of(*position, 300);
                assert!((first_i..=first_i + SURFACE_PIECE_CELLS).contains(&i), "column {i} outside piece {}", piece.column);
                assert!((first_j..=first_j + SURFACE_PIECE_CELLS).contains(&j), "row {j} outside piece {}", piece.row);
            }
            for triangle in piece.indices.chunks(3) {
                let [a, b, c] = [0, 1, 2].map(|k| {
                    let index = triangle[k] as usize;
                    assert!(index < count, "index {index} of {count} points");
                    piece.positions[index]
                });
                // Seen from above, `x` to the right and `z` down the page: clockwise is positive.
                let turn = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
                assert!(turn > 0.0, "a triangle turns the wrong way: {a:?} {b:?} {c:?}");
            }
        }
    }

    #[test]
    fn a_rim_point_takes_the_lowest_water_beside_it() {
        // A channel along row 8 whose water falls 1 m in 4 m, between banks 1 m above its bed.
        let size = 16;
        let on_bed = |i: usize, j: usize| j == 8 && (3..=10).contains(&i);
        let heights = grid(size, |i, j| 10.0 - 0.25 * i as f32 + if on_bed(i, j) { 0.0 } else { 1.0 });
        let depth = grid(size, |i, j| if on_bed(i, j) { 0.5 } else { 0.0 });
        let surface = surface(&heights, &depth);
        let mut rims = 0;
        for piece in &surface.pieces {
            for (position, &d) in piece.positions.iter().zip(&piece.depth_m) {
                let (i, j) = cell_of(*position, size);
                if d != 0.0 {
                    continue;
                }
                rims += 1;
                let mut lowest = f32::INFINITY;
                for nj in j - 1..=j + 1 {
                    for ni in i - 1..=i + 1 {
                        if depth.get(ni, nj) > 0.0 {
                            let water = heights.get(ni, nj) + depth.get(ni, nj);
                            assert!(position[1] <= water, "the rim at ({i}, {j}) stands above the water at ({ni}, {nj})");
                            lowest = lowest.min(water);
                        }
                    }
                }
                assert_eq!(position[1].to_bits(), lowest.to_bits(), "the rim at ({i}, {j}) is not the lowest water beside it");
            }
        }
        assert!(rims >= 20, "only {rims} rim points");
    }

    #[test]
    fn a_rim_point_lies_under_its_ground() {
        // Water 1 m deep at (5, 5) on ground 10 m: its surface stands at 11 m, 0.5 m above the dry
        // cell (6, 5) at 10.5 m, which the surface still reaches.
        let heights = grid(12, |i, j| if (i, j) == (6, 5) { 10.5 } else { 10.0 });
        let depth = grid(12, |i, j| if (i, j) == (5, 5) { 1.0 } else { 0.0 });
        let surface = surface(&heights, &depth);
        let piece = &surface.pieces[0];
        let at = |cell: (usize, usize)| {
            let index = piece.positions.iter().position(|p| cell_of(*p, 12) == cell).expect("the cell is a point");
            piece.positions[index][1]
        };
        assert_eq!(at((5, 5)), 11.0);
        assert!((at((6, 5)) - (10.5 - RIM_UNDER)).abs() < 1e-5, "stands at {}", at((6, 5)));
        assert!((at((4, 4)) - (10.0 - RIM_UNDER)).abs() < 1e-5, "stands at {}", at((4, 4)));
    }

    #[test]
    fn pieces_share_their_edge_points() {
        // Water across column 128 and row 128: the line between the pieces.
        let heights = grid(300, rolling);
        let depth = grid(300, |i, j| if (118..139).contains(&i) && (118..139).contains(&j) { 0.4 } else { 0.0 });
        let surface = surface(&heights, &depth);
        let pieces: BTreeSet<(usize, usize)> = surface.pieces.iter().map(|p| (p.column, p.row)).collect();
        assert_eq!(surface.pieces.len(), 4);
        assert_eq!(pieces, BTreeSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]));
        let mut held: BTreeMap<(usize, usize), Vec<PointBits>> = BTreeMap::new();
        for piece in &surface.pieces {
            for (position, depth) in piece.positions.iter().zip(&piece.depth_m) {
                held.entry(cell_of(*position, 300)).or_default().push((position.map(f32::to_bits), depth.to_bits()));
            }
        }
        for (cell, copies) in &held {
            assert!(copies.iter().all(|copy| *copy == copies[0]), "the copies of the point {cell:?} differ");
            let expected = match (cell.0 == 128, cell.1 == 128) {
                (true, true) => 4,
                (true, false) | (false, true) => 2,
                (false, false) => 1,
            };
            assert_eq!(copies.len(), expected, "the point {cell:?} is held {} times", copies.len());
        }
        assert_eq!(held[&(128, 128)].len(), 4);
        assert!(held.contains_key(&(128, 120)) && held.contains_key(&(120, 128)));
    }

    #[test]
    fn a_piece_without_water_does_not_exist() {
        let heights = grid(300, rolling);
        let depth = grid(300, |i, j| if (200..210).contains(&i) && (10..20).contains(&j) { 0.3 } else { 0.0 });
        let surface = surface(&heights, &depth);
        let pieces: Vec<(usize, usize)> = surface.pieces.iter().map(|p| (p.column, p.row)).collect();
        assert_eq!(pieces, [(1, 0)]);
    }

    #[test]
    fn no_water_gives_no_pieces() {
        let heights = grid(300, rolling);
        let depth = grid(300, |_, _| 0.0);
        assert!(surface(&heights, &depth).pieces.is_empty());
    }

    #[test]
    fn water_at_the_worlds_edge_ends_there() {
        let size = 40;
        let heights = grid(size, rolling);
        let on_edge = |i: usize, j: usize| i == 0 || j == 0 || i == size - 1 || j == size - 1;
        let depth = grid(size, |i, j| if on_edge(i, j) { 0.5 } else { 0.0 });
        let surface = surface(&heights, &depth);
        let half = (size / 2) as f32;
        let mut triangles = 0;
        for piece in &surface.pieces {
            for position in &piece.positions {
                assert!((-half..=(size - 1) as f32 - half).contains(&position[0]), "x {} outside the world", position[0]);
                assert!((-half..=(size - 1) as f32 - half).contains(&position[2]), "z {} outside the world", position[2]);
                assert!(position.iter().all(|v| v.is_finite()));
            }
            assert!(piece.indices.iter().all(|&index| (index as usize) < piece.positions.len()));
            triangles += piece.indices.len() / 3;
        }
        assert_eq!(triangles, 2 * squares_with_water(&depth));
    }

    #[test]
    fn odd_sizes_give_whole_pieces() {
        for size in [64, 300] {
            let heights = grid(size, rolling);
            let depth = grid(size, |_, _| 1.0);
            let surface = surface(&heights, &depth);
            let squares = squares_of(&surface, size);
            assert_eq!(squares.len(), (size - 1) * (size - 1), "size {size}");
            assert!(squares.values().all(|holders| holders.len() == 2 && holders[0] == holders[1]), "size {size}");
            let across = (size - 1).div_ceil(SURFACE_PIECE_CELLS);
            assert_eq!(surface.pieces.len(), across * across, "size {size}");
            for piece in &surface.pieces {
                let side = |index: usize| SURFACE_PIECE_CELLS.min(size - 1 - index * SURFACE_PIECE_CELLS) + 1;
                assert_eq!(piece.positions.len(), side(piece.column) * side(piece.row), "size {size}, piece ({}, {})", piece.column, piece.row);
            }
        }
        // The last pieces of 300 cells hold 44 points a side, the others 129.
        let surface = surface(&grid(300, rolling), &grid(300, |_, _| 1.0));
        let last = surface.pieces.last().expect("pieces");
        assert_eq!((last.column, last.row, last.positions.len()), (2, 2, 44 * 44));
        assert_eq!(surface.pieces[0].positions.len(), 129 * 129);
    }

    /// A surface that holds every one of its squares whole: indices in range, each point used,
    /// each square's two triangles, the same square count as a direct count.
    fn assert_whole(surface: &WaterSurface, depth: &Grid<f32>) {
        let size = depth.width;
        for piece in &surface.pieces {
            let mut used = vec![false; piece.positions.len()];
            for &index in &piece.indices {
                used[index as usize] = true;
            }
            assert!(used.iter().all(|&u| u), "a point of the piece is used by no triangle");
            assert!(piece.positions.iter().flatten().chain(&piece.depth_m).all(|v| v.is_finite()));
        }
        let squares = squares_of(surface, size);
        assert_eq!(squares.len(), squares_with_water(depth));
        assert!(squares.values().all(|holders| holders.len() == 2));
    }

    #[test]
    fn awkward_water_stays_whole() {
        let size = 16;
        let heights = grid(size, |_, _| 10.0);

        // One wet cell alone: its four squares, nine points, eight triangles.
        let alone = grid(size, |i, j| if (i, j) == (8, 8) { 0.5 } else { 0.0 });
        let one = surface(&heights, &alone);
        assert_whole(&one, &alone);
        let piece = &one.pieces[0];
        assert_eq!((piece.positions.len(), piece.indices.len() / 3), (9, 8));
        assert_eq!(piece.depth_m.iter().filter(|&&d| d > 0.0).count(), 1);

        // A dry cell ringed by wet ones: a rim point under its ground, below the water around it.
        let ring = grid(size, |i, j| if (7..=9).contains(&i) && (7..=9).contains(&j) && (i, j) != (8, 8) { 0.5 } else { 0.0 });
        let ringed = surface(&heights, &ring);
        assert_whole(&ringed, &ring);
        let piece = &ringed.pieces[0];
        let centre = piece.positions.iter().position(|p| cell_of(*p, size) == (8, 8)).expect("the dry cell is a point");
        assert_eq!(piece.depth_m[centre], 0.0);
        assert_eq!(piece.positions[centre][1], 10.0 - RIM_UNDER);

        // A wet line one cell wide.
        let line = grid(size, |i, j| if j == 8 && (3..=12).contains(&i) { 0.2 } else { 0.0 });
        let thin = surface(&heights, &line);
        assert_whole(&thin, &line);
        assert_eq!(squares_with_water(&line), 22);
    }

    #[test]
    fn grids_that_do_not_match_are_refused() {
        let water = Water::default();
        let (small, big) = (grid(8, rolling), grid(9, rolling));
        let wide: Grid<f32> = Grid::new(8, 9, 1.0);
        for (heights, depth) in [(&small, &big), (&big, &small), (&wide, &wide), (&small, &wide)] {
            match water_surface(heights, depth, &water) {
                Err(e @ LandformError::BadConfig(_)) => {
                    let message = e.to_string();
                    assert!(message.contains("square") && message.ends_with('.'), "{message}");
                }
                other => panic!("expected BadConfig, got {other:?}"),
            }
        }
        assert!(water_surface(&small, &small, &water).is_ok());
    }

    #[test]
    fn the_example_worlds_surface_covers_its_water() {
        let world = generate_drawn(1, &Drawing::example_valley(2048), &GenConfig::default(), &AtomicBool::new(false))
            .expect("the drawing gives a world");
        let depth = &world.layers[WATER_LAYER];
        let size = world.heights.width;
        let surface = water_surface(&world.heights, depth, &world.water).expect("a surface");
        let mut seen: Vec<Option<f32>> = vec![None; size * size];
        for piece in &surface.pieces {
            for (position, &d) in piece.positions.iter().zip(&piece.depth_m) {
                assert!(position.iter().all(|v| v.is_finite()) && d.is_finite(), "a number is not finite");
                let (i, j) = cell_of(*position, size);
                let ground = world.heights.get(i, j);
                if d == 0.0 {
                    assert!(position[1] <= ground - RIM_UNDER, "the rim at ({i}, {j}) is not under its ground");
                }
                seen[j * size + i] = Some(position[1]);
            }
        }
        let mut wet = 0;
        for j in 0..size {
            for i in 0..size {
                let d = depth.get(i, j);
                if d > 0.0 {
                    wet += 1;
                    let expected = world.heights.get(i, j) + d;
                    assert_eq!(seen[j * size + i].map(f32::to_bits), Some(expected.to_bits()), "wet cell ({i}, {j})");
                }
            }
        }
        assert!(wet > 0, "the example world has no water");
        let points: usize = surface.pieces.iter().map(|p| p.positions.len()).sum();
        let triangles: usize = surface.pieces.iter().map(|p| p.indices.len() / 3).sum();
        assert_eq!(triangles, 2 * squares_with_water(depth));
        println!("example world surface: {wet} wet cells, {} pieces, {points} points, {triangles} triangles", surface.pieces.len());
    }

    // The flow and the fall. The brief's numbers are repeated here, so that a changed constant shows.

    /// The roughness of the speed's formula and its limits, in metres per second.
    const ROUGHNESS: f64 = 0.035;
    const SPEED_MIN: f64 = 0.05;
    const SPEED_MAX: f64 = 4.0;
    /// How much faster than `SPEED_MAX` a point may come out, in metres per second, through the
    /// rounding of its velocity's two parts to `f32`.
    const ROUNDING: f64 = 1e-4;
    /// The largest angle between the flows of two neighbouring wet points at the hand-made
    /// junction, in degrees, as measured; the test allows 5 degrees more.
    const JUNCTION_LARGEST_DEG: f64 = 46.83;

    /// The side of the hand-made worlds of the flow tests, in cells of 1 m: cell `(i, j)` lies at
    /// world `(i - 80, j - 80)`, and their pieces meet at 48 m along both axes.
    const FLOW_SIZE: usize = 160;

    /// The world place of cell `(i, j)` of a world `FLOW_SIZE` cells a side, in metres.
    fn place((i, j): (usize, usize)) -> (f64, f64) {
        let half = (FLOW_SIZE / 2) as f64;
        (i as f64 - half, j as f64 - half)
    }

    /// The cell at world place `(x, z)`, whole metres, of a world `FLOW_SIZE` cells a side.
    fn cell_at((x, z): (f64, f64)) -> (usize, usize) {
        let half = (FLOW_SIZE / 2) as f64;
        ((x + half) as usize, (z + half) as usize)
    }

    /// The speed of the open-channel formula from a point's `depth_m` and its course's `slope`,
    /// kept within the limits, before any fade.
    fn formula_speed(depth_m: f64, slope: f64) -> f64 {
        (depth_m.powf(2.0 / 3.0) * slope.sqrt() / ROUGHNESS).clamp(SPEED_MIN, SPEED_MAX)
    }

    /// The angle between two directions, in degrees, from 0 to 180.
    fn angle_deg(a: [f64; 2], b: [f64; 2]) -> f64 {
        (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]).abs().to_degrees()
    }

    /// A course through `points` (world `x` and `z` and the water's surface, in metres, upstream
    /// first), its water `width_m` wide and `depth_m` deep at its deep line all along.
    fn course_through(points: &[(f32, f32, f32)], width_m: f32, depth_m: f32, from_lake: Option<usize>, end: CourseEnd) -> RiverCourse {
        let points = points.iter().map(|&(x_m, z_m, surface_m)| CoursePoint { x_m, z_m, surface_m, width_m, depth_m }).collect();
        RiverCourse { drawn: None, from_lake, points, end }
    }

    /// Points along the straight line from `from` to `to`, world metres, `step_m` apart and the
    /// last stretch shorter, with a surface falling `slope` per metre from `top_m`.
    fn straight(from: (f64, f64), to: (f64, f64), step_m: f64, top_m: f64, slope: f64) -> Vec<(f32, f32, f32)> {
        let length_m = (to.0 - from.0).hypot(to.1 - from.1);
        let steps = (length_m / step_m).ceil() as usize;
        (0..=steps)
            .map(|k| {
                let along_m = (k as f64 * step_m).min(length_m);
                let share = along_m / length_m;
                let (x, z) = (from.0 + (to.0 - from.0) * share, from.1 + (to.1 - from.1) * share);
                (x as f32, z as f32, (top_m - slope * along_m) as f32)
            })
            .collect()
    }

    /// The nearest place to `(x, z)` on the line of `course`: its distance, the half width of the
    /// water there and how far along the course it lies, in metres.
    fn nearest(course: &RiverCourse, (x, z): (f64, f64)) -> (f64, f64, f64) {
        let mut best = (f64::INFINITY, 0.0, 0.0);
        let mut along_m = 0.0;
        for pair in course.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (ax, az, dx, dz) = (f64::from(a.x_m), f64::from(a.z_m), f64::from(b.x_m - a.x_m), f64::from(b.z_m - a.z_m));
            let squared = dx * dx + dz * dz;
            let t = if squared > 0.0 { (((x - ax) * dx + (z - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
            let distance = (ax + dx * t - x).hypot(az + dz * t - z);
            if distance < best.0 {
                best = (distance, f64::from(a.width_m + (b.width_m - a.width_m) * t as f32) / 2.0, along_m + t * squared.sqrt());
            }
            along_m += squared.sqrt();
        }
        best
    }

    /// A hand-made world of water `FLOW_SIZE` cells a side: the beds of `courses`, each across its
    /// line at its depth out to half its half width and rounding up to its waterlines, and the
    /// lakes, `(x, z, radius, depth)` in metres, flat; the deeper water where they meet. Its ground
    /// lies at `ground(x, z)` less the water's depth, so the water's surface is `ground`. Returns
    /// its heights, its depth and its water, a lake for each of `lakes`.
    fn flow_world(courses: Vec<RiverCourse>, lakes: &[(f64, f64, f64, f32)], ground: impl Fn(f64, f64) -> f32) -> (Grid<f32>, Grid<f32>, Water) {
        let depth = grid(FLOW_SIZE, |i, j| {
            let (x, z) = place((i, j));
            let mut deepest = 0.0f32;
            for course in &courses {
                let (distance, half_width, _) = nearest(course, (x, z));
                let share = distance / half_width;
                let across = if share <= 0.5 { 1.0 } else if share < 1.0 { 1.0 - (2.0 * share - 1.0).powi(2) } else { 0.0 };
                deepest = deepest.max(course.points[0].depth_m * across as f32);
            }
            for &(lake_x, lake_z, radius, lake_depth) in lakes {
                if (x - lake_x).hypot(z - lake_z) < radius {
                    deepest = deepest.max(lake_depth);
                }
            }
            deepest
        });
        let heights = grid(FLOW_SIZE, |i, j| {
            let (x, z) = place((i, j));
            ground(x, z) - depth.get(i, j)
        });
        let lakes = (lakes.iter().enumerate())
            .map(|(drawn, &(x, z, radius, _))| LakeWater {
                drawn,
                level_m: ground(x, z),
                x_m: x as f32,
                z_m: z as f32,
                area_m2: (std::f64::consts::PI * radius * radius) as f32,
            })
            .collect();
        (heights, depth, Water { rivers: courses, lakes })
    }

    /// The angle of the plane world's course to the world's `x` axis, in degrees, so that it runs
    /// across the grid's cells.
    const PLANE_ANGLE_DEG: f64 = 20.0;

    /// A tilted plane falling `slope` per metre along a straight course 8 m wide and 0.5 m deep at
    /// its deep line, from 60 m before the world's centre to 60 m after it at `PLANE_ANGLE_DEG` to
    /// the `x` axis, its points 4 m apart; and the course's direction of flow.
    fn plane_world(slope: f64) -> ((Grid<f32>, Grid<f32>, Water), [f64; 2]) {
        let down = [PLANE_ANGLE_DEG.to_radians().cos(), PLANE_ANGLE_DEG.to_radians().sin()];
        let (from, to) = ((-60.0 * down[0], -60.0 * down[1]), (60.0 * down[0], 60.0 * down[1]));
        let course = course_through(&straight(from, to, 4.0, 20.0, slope), 8.0, 0.5, None, CourseEnd::Edge);
        let ground = move |x: f64, z: f64| (20.0 - slope * ((x - from.0) * down[0] + (z - from.1) * down[1])) as f32;
        (flow_world(vec![course], &[], ground), down)
    }

    /// A course along the world's `x` axis from `from_x` to `to_x`, 8 m wide and 0.5 m deep at its
    /// deep line, its points 4 m apart, its surface falling 0.01 per metre from `top_m`.
    fn along_x(from_x: f64, to_x: f64, top_m: f64, from_lake: Option<usize>, end: CourseEnd) -> RiverCourse {
        course_through(&straight((from_x, 0.0), (to_x, 0.0), 4.0, top_m, 0.01), 8.0, 0.5, from_lake, end)
    }

    /// The ground the courses of `along_x` run on, falling 0.01 per metre toward `x` from 20 m at
    /// 60 m west.
    fn falling_east(x: f64, _: f64) -> f32 {
        (20.0 - 0.01 * (x + 60.0)) as f32
    }

    /// A point of the surface as the flow tests read it.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Seen {
        depth_m: f32,
        velocity: [f64; 2],
        fall: f32,
    }

    impl Seen {
        fn speed(&self) -> f64 {
            self.velocity[0].hypot(self.velocity[1])
        }
    }

    /// The surface of the water `depth` marks over `heights`, with the courses of `water`.
    fn surface_with((heights, depth, water): &(Grid<f32>, Grid<f32>, Water)) -> WaterSurface {
        water_surface(heights, depth, water).expect("matching grids give a surface")
    }

    /// Every point of `surface` once, by its cell, in a world `size` cells a side; a point that the
    /// line between two pieces puts in both carries the same depth, flow and fall in each.
    fn seen_points(surface: &WaterSurface, size: usize) -> BTreeMap<(usize, usize), Seen> {
        let mut seen: BTreeMap<(usize, usize), Seen> = BTreeMap::new();
        for piece in &surface.pieces {
            for (((position, &depth_m), velocity), &fall) in piece.positions.iter().zip(&piece.depth_m).zip(&piece.velocity).zip(&piece.fall) {
                let point = Seen { depth_m, velocity: velocity.map(f64::from), fall };
                let cell = cell_of(*position, size);
                if let Some(before) = seen.insert(cell, point) {
                    assert_eq!(before, point, "the copies of the point {cell:?} differ");
                }
            }
        }
        seen
    }

    #[test]
    fn water_runs_down_its_course() {
        let slope = 0.01;
        let (world, down) = plane_world(slope);
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        // Each point's distance across the line, signed, and how far along it lies, in metres.
        let start = (-60.0 * down[0], -60.0 * down[1]);
        let across_and_along = |cell: (usize, usize)| {
            let (x, z) = place(cell);
            let (dx, dz) = (x - start.0, z - start.1);
            (dz * down[0] - dx * down[1], dx * down[0] + dz * down[1])
        };
        let away_from_the_ends = |along_m: f64| (12.0..=108.0).contains(&along_m);
        let (mut near_the_line, mut pairs) = (0, 0);
        for (&(i, j), seen) in &points {
            let (across, along) = across_and_along((i, j));
            if !away_from_the_ends(along) {
                continue;
            }
            if across.abs() <= 1.0 {
                near_the_line += 1;
                let turn = angle_deg(seen.velocity, down);
                assert!(turn < 1.0, "the flow at ({i}, {j}) turns {turn} degrees off the course");
                let expected = formula_speed(f64::from(seen.depth_m), slope);
                assert!((seen.speed() - expected).abs() <= 0.05 * expected, "at ({i}, {j}): {} m/s, not {expected} m/s", seen.speed());
            }
            // Toward the waterline the speed only falls: no point beside a wet one, farther from the
            // line on its side, is faster.
            if seen.depth_m > 0.0 {
                for next in [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)] {
                    let Some(other) = points.get(&next) else { continue };
                    let (other_across, other_along) = across_and_along(next);
                    if other_across * across > 0.0 && other_across.abs() > across.abs() && away_from_the_ends(other_along) {
                        pairs += 1;
                        assert!(
                            other.speed() <= seen.speed() * (1.0 + 1e-6),
                            "{next:?}, farther out, at {} m/s is faster than ({i}, {j}) at {} m/s",
                            other.speed(),
                            seen.speed()
                        );
                    }
                }
            }
        }
        assert!(near_the_line > 150, "only {near_the_line} points within 1 m of the line");
        assert!(pairs > 500, "only {pairs} pairs toward the waterline");
    }

    #[test]
    fn speed_stays_within_its_limits() {
        // A level course, which runs at the formula's speed from the least slope (0.5 m deep at its
        // deep line, all along), and one falling 1 in 2. The level one's speed goes with the depth,
        // which the smoothing mixes with that of the points two cells away across the line, so it
        // holds to a hundredth (0.55 % at the most measured, 0.1 % at the median over 189 points; a
        // thousandth does not hold); the clamped one holds to the rounding.
        let cases = [(0.0, formula_speed(0.5, SLOPE_FLOOR), 0.01 * formula_speed(0.5, SLOPE_FLOOR)), (0.5, SPEED_MAX, ROUNDING)];
        for (slope, expected, tolerance) in cases {
            let (world, _) = plane_world(slope);
            let (_, depth, water) = &world;
            let course = &water.rivers[0];
            let points = seen_points(&surface_with(&world), FLOW_SIZE);
            let mut near_the_line = 0;
            for (&cell, seen) in &points {
                assert!(seen.speed() <= SPEED_MAX + ROUNDING, "slope {slope}: {} m/s at {cell:?}", seen.speed());
                let (distance, _, along) = nearest(course, place(cell));
                if distance <= 1.0 && (12.0..=108.0).contains(&along) {
                    near_the_line += 1;
                    assert!((seen.speed() - expected).abs() <= tolerance, "slope {slope}: {} m/s at {cell:?}, not {expected}", seen.speed());
                }
            }
            assert!(near_the_line > 150, "slope {slope}: only {near_the_line} points within 1 m of the line");

            // A rim point holds no water of its own, so before the smoothing it is no faster than the
            // least speed.
            let courses = CourseIndex::new(water);
            let mut rims = 0;
            for (&(i, j), seen) in &points {
                if seen.depth_m == 0.0 {
                    rims += 1;
                    let flow = point_flow(depth, 1.0, &courses, i, j);
                    assert!(flow.speed <= SPEED_MIN, "slope {slope}: the rim at ({i}, {j}) runs at {} m/s", flow.speed);
                    assert!(flow.speed > 0.0, "slope {slope}: the rim at ({i}, {j}), beside the water, has no flow");
                }
            }
            assert!(rims > 100, "slope {slope}: only {rims} rim points");
        }
    }

    /// The least slope a course's water is read as having when its speed is worked out, as the brief
    /// gives it.
    const SLOPE_FLOOR: f64 = 0.0005;

    /// A straight course along the `x` axis from 76 m west to 76 m east, `width_m` wide and 0.5 m
    /// deep at its deep line, its points 4 m apart and its surface level at 20 m but for one step of
    /// 0.3 m down between its points at 56 m and 60 m east; and its heights, depth and water.
    fn stepped_world(width_m: f32) -> (Grid<f32>, Grid<f32>, Water) {
        let mut level = straight((-76.0, 0.0), (76.0, 0.0), 4.0, 20.0, 0.0);
        for point in level.iter_mut().filter(|point| point.0 >= 60.0) {
            point.2 -= 0.3;
        }
        let course = course_through(&level, width_m, 0.5, None, CourseEnd::Edge);
        flow_world(vec![course], &[], |x, _| 20.0 - if x >= 58.0 { 0.3 } else { 0.0 })
    }

    #[test]
    fn a_level_course_still_runs() {
        let (world, _) = plane_world(0.0);
        let course = &world.2.rivers[0];
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        let mut near_the_line = 0;
        for (&cell, seen) in &points {
            let (distance, _, along) = nearest(course, place(cell));
            if distance <= 1.0 && (12.0..=108.0).contains(&along) {
                near_the_line += 1;
                // The formula's, from the point's own depth and the least slope.
                let expected = formula_speed(f64::from(seen.depth_m), SLOPE_FLOOR);
                assert!(expected > 0.3, "the formula gives only {expected} m/s at {cell:?}");
                assert!((seen.speed() - expected).abs() <= 0.05 * expected, "a level course runs at {} m/s at {cell:?}, not {expected} m/s", seen.speed());
            }
        }
        assert!(near_the_line > 150, "only {near_the_line} points within 1 m of the line");
    }

    #[test]
    fn a_wide_rivers_speed_reads_a_long_stretch() {
        // The river, 16 m wide: its slope is read 64 m either side, so beside the step it is the
        // step's 0.3 m over the 80 m from the course's point 4 m west of the centre to its end.
        let world = stepped_world(16.0);
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        // What the slope over 8 m either side gives beside the step: 0.3 m over the 12 m from the
        // course's point at 52 m east to its point at 64 m.
        let short = |depth_m: f64| formula_speed(depth_m, 0.3 / 12.0);
        let (mut beside, mut far) = (0, 0);
        for (&cell, seen) in &points {
            let (x, z) = place(cell);
            if z != 0.0 {
                continue;
            }
            if (57.0..=59.0).contains(&x) {
                beside += 1;
                let (short, floor) = (short(f64::from(seen.depth_m)), formula_speed(f64::from(seen.depth_m), SLOPE_FLOOR));
                assert!(seen.speed() < 0.5 * short, "beside the step the river runs at {} m/s at {cell:?}, over half of {short} m/s", seen.speed());
                assert!(seen.speed() >= floor * (1.0 - 1e-3), "beside the step the river runs at {} m/s at {cell:?}, under the floor's {floor} m/s", seen.speed());
            }
            // 100 m from the step, so beyond the 64 m the slope is read: the floor's.
            if (-45.0..=-35.0).contains(&x) {
                far += 1;
                let floor = formula_speed(f64::from(seen.depth_m), SLOPE_FLOOR);
                assert!((seen.speed() - floor).abs() <= 0.05 * floor, "100 m from the step the river runs at {} m/s at {cell:?}, not the floor's {floor} m/s", seen.speed());
            }
        }
        assert_eq!((beside, far), (3, 11), "points on the line beside the step and far from it");

        // The reach, from both sides: the slope is read to the course's point `SPEED_REACH_WIDTHS`
        // widths along, here at 60 m east, the step's far side. A point on the line 3 m inside that
        // reach from it reads the step, so runs faster than the floor's; one 5 m outside reads none,
        // and runs at the floor's. Both clear the two smoothing passes (2 m either side). A reach of
        // 3 widths puts the first point outside, one of 5 widths the second inside.
        let reach_m = SPEED_REACH_WIDTHS * 16.0;
        let at = |from_step_m: f64| {
            let x = 60.0 - from_step_m;
            let seen = points[&cell_at((x, 0.0))];
            assert!(seen.depth_m > 0.0, "the point at x = {x} is dry");
            (seen.speed(), formula_speed(f64::from(seen.depth_m), SLOPE_FLOOR))
        };
        let (speed, floor) = at(reach_m - 3.0);
        assert!(speed > 1.5 * floor, "3 m inside the reach the river runs at {speed} m/s, not over 1.5 times the floor's {floor} m/s");
        let (speed, floor) = at(reach_m + 5.0);
        assert!((speed - floor).abs() <= 0.05 * floor, "5 m outside the reach the river runs at {speed} m/s, not the floor's {floor} m/s");

        // The same step on a 2 m brook is read over 8 m as before. Read before the smoothing, which
        // averages a brook one cell wide with the rim points on either side of it.
        let (_, depth, water) = stepped_world(2.0);
        let courses = CourseIndex::new(&water);
        let mut beside = 0;
        let row = FLOW_SIZE / 2;
        for i in 0..FLOW_SIZE {
            let (x, _) = place((i, row));
            if (57.0..=59.0).contains(&x) {
                beside += 1;
                let own_m = f64::from(depth.get(i, row));
                let flow = point_flow(&depth, 1.0, &courses, i, row);
                let expected = formula_speed(own_m, 0.3 / 12.0);
                assert!(own_m > 0.4, "the brook is {own_m} m deep at x = {x}");
                assert!((flow.speed - expected).abs() <= 0.05 * expected, "beside the step the brook runs at {} m/s, not {expected} m/s", flow.speed);
            }
        }
        assert_eq!(beside, 3);
        assert_eq!(SPEED_REACH_WIDTHS * 2.0, SLOPE_REACH_M, "a 2 m brook reads the 8 m of before");
    }

    #[test]
    fn the_fall_still_reads_its_own_stretch() {
        let points = seen_points(&surface_with(&stepped_world(16.0)), FLOW_SIZE);
        let mut beside = 0;
        for (&cell, seen) in &points {
            let (x, _) = place(cell);
            if seen.depth_m > 0.0 && (57.0..=59.0).contains(&x) {
                beside += 1;
                assert!(seen.fall >= 0.03, "beside the step the fall at {cell:?} is only {}", seen.fall);
            }
        }
        assert!(beside >= 30, "only {beside} wet points beside the step");
    }

    #[test]
    fn the_examples_drawn_river_runs() {
        use crate::country::Country;
        for kind in ["hills", "mountains"] {
            let drawing = Drawing { country: Country::named(kind).expect("a known kind of country"), ..Drawing::example_valley(2048) };
            let world = generate_drawn(11, &drawing, &GenConfig::default(), &AtomicBool::new(false)).expect("the drawing gives a world");
            let size = world.heights.width;
            let surface = water_surface(&world.heights, &world.layers[WATER_LAYER], &world.water).expect("a surface");
            let speeds: BTreeMap<(usize, usize), f64> = surface
                .pieces
                .iter()
                .flat_map(|piece| piece.positions.iter().zip(&piece.velocity))
                .map(|(position, velocity)| (cell_of(*position, size), f64::from(velocity[0]).hypot(f64::from(velocity[1]))))
                .collect();
            let mut courses = 0;
            for course in world.water.rivers.iter().filter(|course| course.drawn == Some(0)) {
                courses += 1;
                let line = CourseLine::new(course);
                let length_m = line.along_m[line.points.len() - 1];
                let mut seen: Vec<f64> = Vec::new();
                for (point, &along_m) in course.points.iter().zip(&line.along_m) {
                    let near_a_lake = (course.from_lake.is_some() && along_m <= 16.0) || (matches!(course.end, CourseEnd::Lake(_)) && length_m - along_m <= 16.0);
                    if near_a_lake {
                        continue;
                    }
                    let half = (size / 2) as f64;
                    let cell = ((f64::from(point.x_m) + half).round() as usize, (f64::from(point.z_m) + half).round() as usize);
                    let speed = *speeds.get(&cell).unwrap_or_else(|| panic!("{kind}: no point of the surface at {cell:?}, on a course"));
                    assert!(speed >= 0.4, "{kind}: the river runs at {speed} m/s at {cell:?}, {along_m:.0} m along a course of {length_m:.0} m");
                    seen.push(speed);
                }
                seen.sort_by(f64::total_cmp);
                assert!(seen.len() > 100, "{kind}: only {} points checked", seen.len());
                println!(
                    "the example, {kind}, seed 11: the course {:?} to {:?} runs at {:.3} m/s at the least, {:.3} at the median, over {} points",
                    course.from_lake,
                    course.end,
                    seen[0],
                    seen[seen.len() / 2],
                    seen.len()
                );
            }
            assert_eq!(courses, 2, "{kind}: the first drawn river should be two courses");
        }
    }

    #[test]
    fn a_lake_lies_still() {
        let world = flow_world(Vec::new(), &[(0.0, 0.0, 40.0, 2.0)], |_, _| 10.0);
        assert_eq!(world.2.lakes.len(), 1);
        let surface = surface_with(&world);
        let points: usize = surface.pieces.iter().map(|piece| piece.positions.len()).sum();
        assert!(points > 4000, "only {points} points");
        for piece in &surface.pieces {
            assert!(piece.velocity.iter().all(|v| *v == [0.0, 0.0]), "a point of the lake moves");
            assert!(piece.fall.iter().all(|&fall| fall == 0.0), "a point of the lake falls");
        }
    }

    #[test]
    fn flow_fades_at_a_lake() {
        // The speed at `x` on the line z = 0, and the formula's from the point's own depth there.
        let on_the_line = |points: &BTreeMap<(usize, usize), Seen>, x: f64| {
            let seen = points[&cell_at((x, 0.0))];
            (seen.speed(), formula_speed(f64::from(seen.depth_m), 0.01))
        };

        // Out of a lake 10 m around the course's first point, 1 m deep.
        let out = along_x(-60.0, 60.0, 20.0, Some(0), CourseEnd::Edge);
        let points = seen_points(&surface_with(&flow_world(vec![out], &[(-60.0, 0.0, 10.0, 1.0)], falling_east)), FLOW_SIZE);
        let (speed, full) = on_the_line(&points, -60.0);
        assert!(speed < 0.1 * full, "out of the lake: {speed} m/s at the first point, full {full} m/s");
        let (speed, full) = on_the_line(&points, -44.0);
        assert!(speed >= 0.9 * full, "out of the lake: {speed} m/s 16 m along, full {full} m/s");

        // Into a lake 10 m around the course's last point.
        let into = along_x(-60.0, 60.0, 20.0, None, CourseEnd::Lake(0));
        let points = seen_points(&surface_with(&flow_world(vec![into], &[(60.0, 0.0, 10.0, 1.0)], falling_east)), FLOW_SIZE);
        let (speed, full) = on_the_line(&points, 60.0);
        assert!(speed < 0.1 * full, "into the lake: {speed} m/s at the last point, full {full} m/s");
        let (speed, full) = on_the_line(&points, 44.0);
        assert!(speed >= 0.9 * full, "into the lake: {speed} m/s 16 m before the last point, full {full} m/s");

        // Into another course, which goes on along the same line: no fade to its last point.
        let first = along_x(-60.0, 0.0, 20.0, None, CourseEnd::River(1));
        let second = along_x(0.0, 60.0, 19.4, None, CourseEnd::Edge);
        let points = seen_points(&surface_with(&flow_world(vec![first, second], &[], falling_east)), FLOW_SIZE);
        for x in -44..=0 {
            let (speed, full) = on_the_line(&points, f64::from(x));
            assert!(speed >= 0.9 * full, "into another course: {speed} m/s at {x} m, full {full} m/s");
        }
    }

    #[test]
    fn fall_follows_the_course() {
        let slope = 0.01;
        let (world, _) = plane_world(slope);
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        let mut wet = 0;
        for (&cell, seen) in points.iter().filter(|(_, seen)| seen.depth_m > 0.0) {
            wet += 1;
            assert!((f64::from(seen.fall) - slope).abs() <= 0.05 * slope, "the fall at {cell:?} is {}, not {slope}", seen.fall);
        }
        assert!(wet > 500, "only {wet} wet points");

        // A drop of 0.5 m between the points at 0 and 4 m east: the fall beside that stretch.
        let mut stepped = straight((-60.0, 0.0), (60.0, 0.0), 4.0, 20.0, slope);
        for point in stepped.iter_mut().filter(|point| point.0 >= 4.0) {
            point.2 -= 0.5;
        }
        let course = course_through(&stepped, 8.0, 0.5, None, CourseEnd::Edge);
        let ground = |x: f64, z: f64| falling_east(x, z) - if x >= 2.0 { 0.5 } else { 0.0 };
        let points = seen_points(&surface_with(&flow_world(vec![course], &[], ground)), FLOW_SIZE);
        let mut beside = 0;
        for (&cell, seen) in &points {
            let (x, _) = place(cell);
            if seen.depth_m > 0.0 && (1.0..=3.0).contains(&x) {
                beside += 1;
                assert!(seen.fall >= 0.1, "beside the drop the fall at {cell:?} is only {}", seen.fall);
            }
        }
        assert!(beside >= 15, "only {beside} wet points beside the drop");
    }

    /// A 2 m tributary meeting a 12 m river at a right angle: the river, course 0 as the drawn
    /// rivers come before the brooks, along the `x` axis from 76 m west to 76 m east, 1 m deep at its
    /// deep line and falling 0.005 per metre; the tributary from 76 m north (`-z`) down to the
    /// river's point at the centre, 0.3 m deep and falling 0.03 per metre; both with points 4 m
    /// apart.
    fn junction_world() -> (Grid<f32>, Grid<f32>, Water) {
        let river = course_through(&straight((-76.0, 0.0), (76.0, 0.0), 4.0, 20.0, 0.005), 12.0, 1.0, None, CourseEnd::Edge);
        let mouth_m = 20.0 - 0.005 * 76.0;
        let tributary = course_through(&straight((0.0, -76.0), (0.0, 0.0), 4.0, mouth_m + 0.03 * 76.0, 0.03), 2.0, 0.3, None, CourseEnd::River(0));
        let ground = |x: f64, z: f64| (20.0 - 0.005 * (x + 76.0) + 0.03 * (-z).max(0.0)) as f32;
        flow_world(vec![river, tributary], &[], ground)
    }

    #[test]
    fn inside_a_rivers_bed_the_flow_is_the_rivers() {
        // The tributary's course ends on the river's point at the centre, its mouth: the points of the
        // river's bed within 1 m of it lie as near the tributary's line as the river's or nearer, and
        // still run along the river, at the river's speed.
        let points = seen_points(&surface_with(&junction_world()), FLOW_SIZE);
        let mut checked = 0;
        for (&cell, seen) in &points {
            let (x, z) = place(cell);
            if seen.depth_m > 0.0 && x.hypot(z) <= 1.0 {
                checked += 1;
                let turn = angle_deg(seen.velocity, [1.0, 0.0]);
                assert!(turn < 1.0, "the flow at {cell:?}, 1 m from the mouth, turns {turn} degrees off the river");
                let river = formula_speed(f64::from(seen.depth_m), 0.005);
                assert!((seen.speed() - river).abs() <= 0.05 * river, "at {cell:?}: {} m/s, not the river's {river} m/s", seen.speed());
            }
        }
        assert_eq!(checked, 5);
    }

    #[test]
    fn flow_turns_gently_through_a_junction() {
        let points = seen_points(&surface_with(&junction_world()), FLOW_SIZE);
        let (mut largest, mut at, mut pairs) = (0.0f64, (0, 0), 0);
        for (&(i, j), seen) in points.iter().filter(|(_, seen)| seen.depth_m > 0.0) {
            for next in [(i + 1, j), (i, j + 1)] {
                let Some(other) = points.get(&next).filter(|other| other.depth_m > 0.0) else { continue };
                pairs += 1;
                assert!(seen.speed() > 0.0 && other.speed() > 0.0, "a wet point beside ({i}, {j}) has no flow");
                let turn = angle_deg(seen.velocity, other.velocity);
                if turn > largest {
                    (largest, at) = (turn, (i, j));
                }
            }
        }
        println!("the flow turns at most {largest:.2} degrees between neighbouring wet points, at {at:?}, over {pairs} pairs");
        assert!(pairs > 2000, "only {pairs} pairs of wet points");
        assert!(largest < 90.0, "the flow turns {largest} degrees between neighbouring wet points at {at:?}");
        assert!(largest <= JUNCTION_LARGEST_DEG + 5.0, "the flow turns {largest} degrees at {at:?}, more than the {JUNCTION_LARGEST_DEG} measured");
    }

    #[test]
    fn awkward_courses_give_finite_flow() {
        let slope = 0.01;
        let line = straight((-60.0, 0.0), (60.0, 0.0), 4.0, 20.0, slope);
        // The first point, one in the middle and the last, each twice.
        let mut repeated = line.clone();
        for k in [repeated.len() - 1, 15, 0] {
            repeated.insert(k, repeated[k]);
        }
        // East along the `x` axis to the centre, then south.
        let mut kinked = straight((-60.0, 0.0), (0.0, 0.0), 4.0, 20.0, slope);
        kinked.extend(straight((0.0, 0.0), (0.0, 60.0), 4.0, 19.4, slope).into_iter().skip(1));
        let edge = CourseEnd::Edge;
        let cases = [
            ("a course of two points", course_through(&[(-50.0, -10.0, 20.0), (50.0, 15.0, 19.0)], 6.0, 0.4, None, edge), None),
            ("a repeated point", course_through(&repeated, 8.0, 0.5, None, edge), None),
            ("a right-angle kink", course_through(&kinked, 8.0, 0.5, None, edge), None),
            ("a brook one cell wide", course_through(&line, 1.0, 0.2, None, edge), None),
            ("a dry cell in the middle of a river", course_through(&line, 8.0, 0.5, None, edge), Some(cell_at((10.0, 0.0)))),
        ];
        for (name, course, dry) in cases {
            let (heights, mut depth, water) = flow_world(vec![course], &[], falling_east);
            if let Some((i, j)) = dry {
                depth.set(i, j, 0.0);
            }
            let surface = surface_with(&(heights, depth, water));
            let mut flowing = 0;
            for piece in &surface.pieces {
                for (velocity, &fall) in piece.velocity.iter().zip(&piece.fall) {
                    assert!(velocity.iter().all(|v| v.is_finite()) && fall.is_finite(), "{name}: a number is not finite");
                    let speed = f64::from(velocity[0]).hypot(f64::from(velocity[1]));
                    assert!(speed <= SPEED_MAX + ROUNDING && fall >= 0.0, "{name}: {speed} m/s, fall {fall}");
                    if speed > 0.0 {
                        flowing += 1;
                    }
                }
            }
            assert!(flowing > 50, "{name}: only {flowing} points flow");
        }
    }

    #[test]
    fn the_example_worlds_flow_is_finite_and_within_limits() {
        let world = generate_drawn(1, &Drawing::example_valley(2048), &GenConfig::default(), &AtomicBool::new(false))
            .expect("the drawing gives a world");
        let depth = &world.layers[WATER_LAYER];
        let on_threads = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
            pool.install(|| water_surface(&world.heights, depth, &world.water).expect("a surface"))
        };
        // The flow and the fall as bits, so that a difference in the last bit shows.
        let flow_bits = |surface: &WaterSurface| -> Vec<(Vec<[u32; 2]>, Vec<u32>)> {
            (surface.pieces.iter())
                .map(|piece| (piece.velocity.iter().map(|v| v.map(f32::to_bits)).collect(), piece.fall.iter().map(|f| f.to_bits()).collect()))
                .collect()
        };
        let surface = on_threads(1);
        for threads in [4, 16] {
            let again = on_threads(threads);
            assert!(again == surface && flow_bits(&again) == flow_bits(&surface), "the surface differs on {threads} threads");
        }
        let (mut flowing, mut still, mut fastest, mut steepest) = (0, 0, 0.0f64, 0.0f32);
        for piece in &surface.pieces {
            for (velocity, &fall) in piece.velocity.iter().zip(&piece.fall) {
                assert!(velocity.iter().all(|v| v.is_finite()) && fall.is_finite(), "a number is not finite");
                let speed = f64::from(velocity[0]).hypot(f64::from(velocity[1]));
                assert!(speed <= SPEED_MAX + ROUNDING, "{speed} m/s");
                assert!(fall >= 0.0, "a fall of {fall}");
                if speed > 0.0 {
                    flowing += 1;
                } else {
                    still += 1;
                }
                (fastest, steepest) = (fastest.max(speed), steepest.max(fall));
            }
        }
        println!("example world flow: {flowing} points flow, {still} lie still, fastest {fastest:.3} m/s, steepest fall {steepest:.3}");
        assert!(flowing > 10_000, "only {flowing} points flow");
        assert!(still > 0, "no point of the lake lies still");
    }

    /// A 12 m river along the `x` axis (course 0, 1 m deep, falling 0.005 per metre) and a 2 m
    /// tributary falling 1 in 5 from 66 m north (`-z`) to the river's waterline at 6 m north, where it
    /// ends on the river (`CourseEnd::River(0)`): its last point is its own water, not the river's.
    fn mouth_world() -> (Grid<f32>, Grid<f32>, Water) {
        let river = course_through(&straight((-76.0, 0.0), (76.0, 0.0), 4.0, 20.0, 0.005), 12.0, 1.0, None, CourseEnd::Edge);
        let mouth_m = 20.0 - 0.005 * 76.0;
        let tributary = course_through(&straight((0.0, -66.0), (0.0, -6.0), 4.0, mouth_m + 0.2 * 60.0, 0.2), 2.0, 0.1, None, CourseEnd::River(0));
        let ground = |x: f64, z: f64| (20.0 - 0.005 * (x + 76.0) + 0.2 * (-z - 6.0).max(0.0)) as f32;
        flow_world(vec![river, tributary], &[], ground)
    }

    // The fall where the water does not run: no fall beyond a course's end that meets other water,
    // none along its first `FALL_END_FADE_M` out of a lake or its last into other water, none
    // beside its water.

    /// The drop per metre of the steep courses of these tests: 1 in 5.
    const STEEP: f64 = 0.2;

    /// Whether the place of the course `course`'s water at world `(x, z)` is its last point: the
    /// point lies beyond the course's end, in its water.
    fn at_the_end(courses: &CourseIndex, course: usize, (x, z): (f64, f64)) -> bool {
        let last_stretch = courses.lines[course].directions.len() - 1;
        courses.place_of((x, z)).is_some_and(|place| place.course == course && place.stretch == last_stretch && place.share == 1.0)
    }

    /// The fall of the wet point of the surface at world `(x, z)` of a world `FLOW_SIZE` cells a side.
    fn wet_fall(points: &BTreeMap<(usize, usize), Seen>, at: (f64, f64)) -> f64 {
        let seen = points.get(&cell_at(at)).unwrap_or_else(|| panic!("no point of the surface at {at:?}"));
        assert!(seen.depth_m > 0.0, "the point at {at:?} is dry");
        f64::from(seen.fall)
    }

    /// Whether `fall` is the steep courses' own, within 5 %.
    fn is_steep(fall: f64) -> bool {
        (fall - STEEP).abs() <= 0.05 * STEEP
    }

    #[test]
    fn a_lake_beyond_a_steep_course_stores_no_fall() {
        // 100 m along the `x` axis, ending in a lake 10 m around its last point, at 40 m east.
        let course = course_through(&straight((-60.0, 0.0), (40.0, 0.0), 4.0, 30.0, STEEP), 8.0, 0.5, None, CourseEnd::Lake(0));
        let world = flow_world(vec![course], &[(40.0, 0.0, 10.0, 1.0)], |x, _| (30.0 - STEEP * (x + 60.0)) as f32);
        let (_, depth, water) = &world;
        let courses = CourseIndex::new(water);
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        let (mut beyond, mut farther) = (0, 0);
        for (&(i, j), seen) in points.iter().filter(|(_, seen)| seen.depth_m > 0.0) {
            let at = place((i, j));
            if !at_the_end(&courses, 0, at) {
                continue;
            }
            beyond += 1;
            let flow = point_flow(depth, 1.0, &courses, i, j);
            assert_eq!(flow.fall, 0.0, "the point at {at:?} beyond the end stores a fall of {} before the smoothing", flow.fall);
            if (at.0 - 40.0).hypot(at.1) > 3.0 {
                farther += 1;
                assert!(seen.fall <= 0.01, "the point at {at:?} beyond the end stores a fall of {} after the smoothing", seen.fall);
            }
        }
        assert!(beyond > 150, "only {beyond} wet points beyond the end");
        assert!(farther > 100, "only {farther} of them farther than 3 m from the last point");
        // The course's own fall from its open start (the 3 points of water beyond it too) to
        // `FALL_END_FADE_M` before its end: 96 points.
        let mut on_the_line = 0;
        for (&cell, seen) in &points {
            let (x, z) = place(cell);
            if z == 0.0 && seen.depth_m > 0.0 && x <= 40.0 - FALL_END_FADE_M {
                on_the_line += 1;
                assert!(is_steep(f64::from(seen.fall)), "the fall at {cell:?}, on the line, is {}, not {STEEP}", seen.fall);
            }
        }
        assert_eq!(on_the_line, 96);
    }

    #[test]
    fn a_course_out_of_a_lake_starts_without_a_fall() {
        // 100 m along the `x` axis from a lake 10 m around its first point, at 40 m west.
        let course = course_through(&straight((-40.0, 0.0), (60.0, 0.0), 4.0, 26.0, STEEP), 8.0, 0.5, Some(0), CourseEnd::Edge);
        let world = flow_world(vec![course], &[(-40.0, 0.0, 10.0, 1.0)], |x, _| (30.0 - STEEP * (x + 60.0)) as f32);
        let (_, depth, water) = &world;
        let courses = CourseIndex::new(water);
        let (i, j) = cell_at((-40.0, 0.0));
        assert_eq!(point_flow(depth, 1.0, &courses, i, j).fall, 0.0, "the first point stores a fall");
        // The lake's points beyond the start are the course's first place, and store none either.
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        let mut beyond = 0;
        for (&(i, j), seen) in points.iter().filter(|(_, seen)| seen.depth_m > 0.0) {
            let (x, z) = place((i, j));
            if x < -40.0 {
                beyond += 1;
                assert_eq!(point_flow(depth, 1.0, &courses, i, j).fall, 0.0, "the point at ({x}, {z}) before the start stores a fall");
                assert!(seen.fall <= 0.01, "the point at ({x}, {z}) before the start stores {}", seen.fall);
            }
        }
        assert!(beyond > 100, "only {beyond} wet points before the start");
        // From `FALL_END_FADE_M` along its line to its open end at the edge (the 3 points of water
        // beyond it too): 96 points.
        let mut on_the_line = 0;
        for (&cell, seen) in &points {
            let (x, z) = place(cell);
            if z == 0.0 && seen.depth_m > 0.0 && x >= -40.0 + FALL_END_FADE_M {
                on_the_line += 1;
                assert!(is_steep(f64::from(seen.fall)), "the fall at {cell:?}, on the line, is {}, not {STEEP}", seen.fall);
            }
        }
        assert_eq!(on_the_line, 96);
    }

    #[test]
    fn water_beside_a_course_stores_no_fall() {
        let course = course_through(&straight((-60.0, 0.0), (60.0, 0.0), 4.0, 30.0, STEEP), 8.0, 0.5, None, CourseEnd::Edge);
        let world = flow_world(vec![course], &[], |x, _| (30.0 - STEEP * (x + 60.0)) as f32);
        let courses = CourseIndex::new(&world.2);
        let fall_at = |at: (f64, f64)| courses.flow_at(at, 0.3).fall;
        // On the line, away from the ends, the course's own.
        for x in [-30.0, 0.0, 30.0] {
            assert!(is_steep(fall_at((x, 0.0))), "on the line at x = {x} the fall is {}", fall_at((x, 0.0)));
        }
        // Beside the water, 4 m from the line, the fall is the water's.
        assert!(is_steep(fall_at((0.0, 4.0))), "at the waterline the fall is {}", fall_at((0.0, 4.0)));
        // Half way to `FLOW_REACH_M` beyond the waterline, half of it.
        let half = fall_at((0.0, 4.0 + FLOW_REACH_M / 2.0));
        assert!((half - STEEP / 2.0).abs() <= 0.05 * STEEP, "6 m beyond the waterline the fall is {half}, not {}", STEEP / 2.0);
        // `FLOW_REACH_M` beyond the waterline, and farther, none, on either side and along the line.
        for (x, z) in [(0.0, 16.0), (0.0, -16.0), (-30.0, 16.0), (30.0, -16.0), (0.0, 17.0), (0.0, 20.0), (0.0, 40.0)] {
            assert_eq!(fall_at((x, z)), 0.0, "the point at ({x}, {z}) beside the water stores a fall");
        }
        // And the surface's points beside it, rim points, store the fall of the water they lie by.
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        let rims: Vec<_> = points.iter().filter(|(&cell, seen)| seen.depth_m == 0.0 && (-30.0..=30.0).contains(&place(cell).0)).collect();
        assert!(rims.len() > 100, "only {} rim points", rims.len());
        for (cell, seen) in rims {
            assert!(f64::from(seen.fall) >= 0.9 * STEEP, "the rim point {cell:?} stores {}", seen.fall);
        }
    }

    #[test]
    fn a_tributarys_mouth_stores_no_fall() {
        // The last point of the 1-in-5 tributary, 6 m north of the river's line, at its waterline.
        let world = mouth_world();
        let (_, depth, water) = &world;
        let courses = CourseIndex::new(water);
        let (i, j) = cell_at((0.0, -6.0));
        let place_there = courses.place_of((0.0, -6.0)).expect("a place at the mouth");
        assert_eq!(place_there.course, 1, "the mouth's point is not the tributary's water");
        assert_eq!(point_flow(depth, 1.0, &courses, i, j).fall, 0.0, "the mouth stores a fall");
        let points = seen_points(&surface_with(&world), FLOW_SIZE);
        // Half way along the fade, 4 m before the mouth: half the course's fall.
        let half = wet_fall(&points, (0.0, -10.0));
        assert!((half - STEEP / 2.0).abs() <= 0.05 * STEEP, "4 m before the mouth the fall is {half}, not {}", STEEP / 2.0);
        // From `FALL_END_FADE_M` before the mouth up the tributary to its head: 53 points.
        let mut on_the_line = 0;
        for z in -66..=-14 {
            on_the_line += 1;
            let fall = wet_fall(&points, (0.0, f64::from(z)));
            assert!(is_steep(fall), "the fall at z = {z}, on the line, is {fall}, not {STEEP}");
        }
        assert_eq!(on_the_line, 53);
        // The speed takes no fade of the end: 2 m before the last point it is what it was before
        // the fall's fade, bit for bit, before and after the smoothing.
        let (i, j) = cell_at((0.0, -8.0));
        let flow = point_flow(depth, 1.0, &courses, i, j);
        assert_eq!(flow.speed.to_bits(), 0x400605ce087b4723, "the speed 2 m before the mouth is {} m/s", flow.speed);
        let seen = points[&(i, j)];
        assert_eq!((seen.velocity[0].to_bits(), seen.velocity[1].to_bits()), (0x0, 0x3ff4b15f60000000), "the smoothed velocity there is {:?}", seen.velocity);
    }

    #[test]
    fn a_river_through_a_left_out_lake_keeps_its_fall() {
        // A drawn lake left out leaves the river going on: the course into it ends on the course out
        // of it (`CourseEnd::River`, as `water.rs` makes it; both are parts of one drawn river), both
        // falling 1 in 5. That end is no mouth, so the fall is not faded before it: where the second
        // course starts at the first's last point, and where the lake's gap, 64 m, lies between them.
        let ground = |x: f64, _: f64| (30.0 - STEEP * (x + 60.0)) as f32;
        let joined = |first_to: f64, second_from: f64, drawn: (usize, usize)| {
            let first = RiverCourse { drawn: Some(drawn.0), ..course_through(&straight((-60.0, 0.0), (first_to, 0.0), 4.0, 30.0, STEEP), 8.0, 0.5, None, CourseEnd::River(1)) };
            let top_m = 30.0 - STEEP * (second_from + 60.0);
            let second = RiverCourse { drawn: Some(drawn.1), ..course_through(&straight((second_from, 0.0), (60.0, 0.0), 4.0, top_m, STEEP), 8.0, 0.5, None, CourseEnd::Edge) };
            flow_world(vec![first, second], &[], ground)
        };
        // The fall on the first course's line at `x`, before the smoothing.
        let fall_at = |world: &(Grid<f32>, Grid<f32>, Water), x: f64| {
            let courses = CourseIndex::new(&world.2);
            assert_eq!(courses.place_of((x, 0.0)).expect("a place on the line").course, 0, "the point at x = {x} is not the first course's water");
            courses.flow_at((x, 0.0), 0.5).fall
        };
        for (name, world, last_x) in [("starting at its last point", joined(-16.0, -16.0, (0, 0)), -16.0), ("across a gap", joined(-32.0, 32.0, (0, 0)), -32.0)] {
            // At its last point and 4 m and 8 m before it, the fall is the course's own.
            for x in [last_x, last_x - 4.0, last_x - 8.0] {
                let fall = fall_at(&world, x);
                assert!(is_steep(fall), "{name}: the fall at x = {x}, on the line, is {fall}, not {STEEP}");
            }
        }
        // Another drawn river's course meets it at a mouth: the fall fades to nothing at its last point.
        assert_eq!(fall_at(&joined(-16.0, -16.0, (0, 1)), -16.0), 0.0, "a mouth of another drawn river stores a fall");
    }

    #[test]
    fn a_course_from_a_spring_to_the_edge_keeps_its_fall() {
        let course = course_through(&straight((-60.0, 0.0), (60.0, 0.0), 4.0, 30.0, STEEP), 8.0, 0.5, None, CourseEnd::Edge);
        let points = seen_points(&surface_with(&flow_world(vec![course], &[], |x, _| (30.0 - STEEP * (x + 60.0)) as f32)), FLOW_SIZE);
        for at in [(-60.0, 0.0), (-59.0, 0.0), (59.0, 0.0), (60.0, 0.0)] {
            let fall = wet_fall(&points, at);
            assert!(is_steep(fall), "the fall at {at:?} is {fall}, not {STEEP}");
        }
    }

    #[test]
    fn the_examples_lake_stores_no_fall() {
        use crate::country::Country;
        let drawing = Drawing { country: Country::named("hills").expect("a known kind of country"), ..Drawing::example_valley(2048) };
        let world = generate_drawn(11, &drawing, &GenConfig::default(), &AtomicBool::new(false)).expect("the drawing gives a world");
        let surface = water_surface(&world.heights, &world.layers[WATER_LAYER], &world.water).expect("a surface");
        let courses = CourseIndex::new(&world.water);
        // Every point of the surface once, by its place: a point on the line between two pieces is in
        // both, the same, and counts once.
        let points: BTreeMap<[u32; 2], ((f64, f64), f32)> = (surface.pieces.iter())
            .flat_map(|piece| piece.positions.iter().zip(&piece.fall))
            .map(|(position, &fall)| ([position[0].to_bits(), position[2].to_bits()], ((f64::from(position[0]), f64::from(position[2])), fall)))
            .collect();
        let mut ending_in_the_lake = 0;
        for (c, course) in world.water.rivers.iter().enumerate() {
            if !matches!(course.end, CourseEnd::Lake(_)) {
                continue;
            }
            ending_in_the_lake += 1;
            // Every point given to this course whose nearest place on its line is its last point: the
            // point lies beyond the course's end, in its water. (Every point of the surface whose
            // nearest place on the line is its last point, whichever course it was given to, is no
            // test of the fall: that half plane holds other water of the valley, a fall of 1.01 at
            // (168, 717), far from the lake.)
            let (mut beyond, mut largest) = (0, (0.0f32, (0.0, 0.0)));
            for &(at, fall) in points.values() {
                if at_the_end(&courses, c, at) {
                    beyond += 1;
                    if fall > largest.0 {
                        largest = (fall, at);
                    }
                }
            }
            println!("the example, hills, seed 11: course {c} ends in a lake, {beyond} points of the surface lie beyond its end, the largest fall {:.4}", largest.0);
            assert!(beyond > 0, "course {c}: no point of the surface lies beyond its end");
            assert!(largest.0 < 0.02, "course {c}: a point beyond its end, at {:?}, stores a fall of {}", largest.1, largest.0);
        }
        assert!(ending_in_the_lake >= 2, "only {ending_in_the_lake} courses end in a lake");
    }
}
