//! Channels on the land model's coarse grid: the lines the water follows, from the drawn rivers
//! and from the ground that drains enough to hold a brook, each smoothed and given a width.

use super::bed::inside_bed;
use super::smooth::{self, distance_m, Sample};
use crate::drawing::Drawing;
use crate::grow::{About, Grown, GrownRiver, Note, RiverEnd, GROW_CELL_M, GROW_MARGIN_M};
use crate::water_rules::WaterRules;

/// A lake cell holds water deeper than this, in metres, within `LAKE_REACH_RADII` radii of the
/// centre of a lake the model lists, and its water's surface lies within `LAKE_LEVEL_M` of that
/// lake's level: the same water the level was read from (`grow::lake_levels`), not a pool beside it.
const LAKE_DEPTH_M: f32 = 0.3;
const LAKE_REACH_RADII: f64 = 1.5;
const LAKE_LEVEL_M: f64 = 0.05;
/// A drawn river keeps within this many of its widths of its line as drawn, or `CORRIDOR_MIN_M`,
/// whichever is more; one that does not is said (`line_notes`).
const CORRIDOR_WIDTHS: f64 = 3.0;
const CORRIDOR_MIN_M: f64 = 40.0;
/// A channel that ends on another's cell ends on the point of that channel nearest the cell, which
/// lies no farther from it than this, in metres, unless the cell lies inside that channel's bed
/// (within half its width, taken at its nearest point, of its line): from there it may lie farther
/// (`Tracing::snapped`).
const SNAP_REACH_M: f64 = 16.0;

/// How a channel ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ChannelEnd {
    /// At the world's border.
    Edge,
    /// On the channel of this index in the list `trace_channels` returns.
    Channel(usize),
    /// In the drawn lake of this index in `Drawing::lakes`.
    Lake(usize),
}

/// A point of a channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChannelPoint {
    /// World `x`, in metres.
    pub x_m: f64,
    /// World `z`, in metres.
    pub z_m: f64,
    /// The bed's width, in metres.
    pub width_m: f32,
    /// The ground draining through the channel there, in square metres.
    pub drained_m2: f32,
}

/// A line the water follows over the land, inside the world (as `trace_channels` defines it).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Channel {
    /// The index in `Drawing::rivers` of the drawn river it is, or `None` for a brook the model found.
    pub drawn: Option<usize>,
    /// The index in `Drawing::lakes` of the drawn lake it flows out of.
    pub from_lake: Option<usize>,
    /// Upstream first, in world metres, inside the world, at most 4 m apart.
    pub points: Vec<ChannelPoint>,
    /// How it ends.
    pub end: ChannelEnd,
}

/// The channels of `grown`, the land grown from `drawing`: the drawn rivers' first, in the
/// drawing's order and each river's from upstream, then the brooks. Sequential and deterministic;
/// every tie goes to the lower cell index. The drawn rivers are traced in the order
/// `tracing_order` gives (a river after the one it joins, the rivers ending in a lake after the
/// others), so a drawn channel may end on one listed after it; a brook ends only on one listed
/// before it.
///
/// Inside the world means within the span of the world's 1 m cells, from `-size_m / 2` to
/// `size_m / 2 - 1` metres along both axes, its ends included: the world's last column and row of
/// cells lie a metre short of its east and south borders, and a point beyond them would lie on no
/// cell. The border is the edge of that span.
///
/// A lake cell is a coarse cell within `LAKE_REACH_RADII` radii of the centre of a lake
/// `grown.lakes` lists that holds more than `LAKE_DEPTH_M` of water, its surface within
/// `LAKE_LEVEL_M` of that lake's level; it belongs to the first such lake. A drawn river's path
/// starts at the cell nearest the first point of its line and follows `grown.receivers` until it
/// reaches an outlet or a cell already on the path of a river traced before it; a river that
/// ends in a lake (`RiverEnd::JoinsLake`) stops where it enters that
/// lake's cells. Its own cells are those of its path that lie inside the world, off the lake cells
/// and on no earlier path. Its channels are the stretches of its own cells: one ends where the
/// path enters a lake's cells (`Lake`), and the next starts where it leaves them, with
/// `from_lake`; one ends at the border where the path leaves the world (`Edge`), and the next
/// starts where it comes back in; one ends on the earlier river's channel that it meets, that cell
/// its last (`Channel`). A stretch that leaves a lake's cells and comes back into the same lake's,
/// at a cell on its own path or on an earlier one, is part of the lake: it gives no channel, and
/// its cells count as that lake's from then on.
///
/// A river that joins another (`RiverEnd::Joins`) instead follows its path only as far as its own
/// cell nearest its last drawn point; from there its channel runs straight to the nearest point of
/// the joined river's channels and ends on that channel there. The rest of its path is given back:
/// its cells are on no drawn path, so brooks are traced through them as through any other cell (a
/// brook along it ends where it meets a channel kept, and is left out when its water is not its
/// own). The path is followed to its end as before when the joined river has no channel or the
/// straight line would cross a lake cell.
///
/// Then the brooks: every cell inside the world that drains at least `rules.brook_start_m2`, is on
/// no drawn river's path and is no lake cell. Of the brook cells, lake cells and cells outside the
/// world draining as much that drain into a brook cell, the one draining most continues there:
/// its brook goes on; for a lake cell a brook starts there flowing out of that lake; for a cell
/// outside the world one starts there coming in across the border. Every other brook ends there,
/// on that cell's channel. A brook also ends at the border and where it enters a lake's cells. The
/// brooks are listed by the ground drained at their last point, most first.
///
/// Each channel's line runs through its cells' centres; where the path comes in or goes out across
/// the border, from or to the cell outside; and, for one that runs straight to a river, to the
/// point it ends on. A channel that ends on another's cell ends instead on the point of that
/// channel nearest the cell (`snapped`), within `SNAP_REACH_M` of it unless the cell lies inside
/// that channel's bed, so every junction lies on a point of both. The line is cut into points at
/// most `POINT_SPACING_M` apart (`smooth::resampled`); its points outside the world
/// or on a lake cell, from the cell outside on, are dropped, so it reaches within that spacing of
/// the border; then it is smoothed with both ends fixed (`smooth::smoothed`). A point's ground
/// drained is the channel's own: at the cell of the channel it ends on, along a straight line to a
/// river and towards a cell outside, its own nearest cell's. A channel with fewer than two points
/// is left out, and one that would end on it ends on what it ended on.
///
/// Widths: a drawn river that enters at the edge has its stated width at every point. A spring
/// (one that starts inside the land) widens from the smaller of `rules.brook_width_m` and its
/// stated width at the start of its path to its stated width at its own cell nearest its last
/// drawn point, by distance along the path, and keeps its stated width beyond; so one no wider than
/// a brook starts keeps its stated width all along, and none narrows. A brook is
/// `brook_width_m * (drained / brook_start_m2)^width_area_power` wide, and never wider than the
/// channel it ends on is where it ends.
///
/// A brook whose water is not its own is left out the same way, and one that would end on it ends
/// on what it ended on: one that ends on a channel and all of whose points lie inside that
/// channel's bed (`bed::inside_bed`: within half its width of its line, the width taken at the
/// line's point nearest the brook's point, no less than `bed::HALF_WIDTH_MIN_M`), whose water is
/// that channel's; and one that flows out
/// of a lake and ends in the same lake, whose water is the lake's. A drawn river's channel is never
/// left out so. Leaving a brook out can make another one qualify, through the end it passes on: a
/// brook that ended on it may now end on a channel whose bed holds it whole, or in the lake it
/// flows out of. Each channel is made, and the rules asked of it, after every channel it may end
/// on, with its end already passed on past those left out; and leaving one out changes nothing of
/// a channel made before it. So the one pass in that order leaves out every brook the rules reach,
/// however the ends were passed on.
pub(crate) fn trace_channels(grown: &Grown, drawing: &Drawing, rules: &WaterRules) -> Vec<Channel> {
    let cells = Cells::new(grown, drawing.size_m);
    let lake_of = lake_cells(&cells, drawing);
    let count = cells.count();
    let mut tracing = Tracing {
        cells,
        lake_of,
        on_path: vec![false; count],
        course_of: vec![None; count],
        courses: Vec::new(),
        made: Vec::new(),
    };
    for river in tracing_order(&grown.rivers) {
        let points = &drawing.rivers[river.drawn].points;
        tracing.trace_river(river, points[points.len() - 1], rules);
    }
    tracing.trace_brooks(rules);
    tracing.into_channels()
}

/// `rivers` in the order they are traced: a river that joins another (`RiverEnd::Joins`) after
/// that river; of the rivers ready, those that end in no lake (`RiverEnd::JoinsLake`) first; and
/// otherwise in the drawing's order. A river that ends in a lake is so traced after every river
/// that does not and is not waiting for it: one traced first that never reads its own lake's cells
/// would run on down the lake's outflow, and the river that drains the lake would end on it. The
/// joins never lead round a circle (see `carried_on` in `grow/river.rs`), so every river finds its
/// place.
fn tracing_order(rivers: &[GrownRiver]) -> Vec<&GrownRiver> {
    let mut traced = vec![false; rivers.len()];
    let mut order = Vec::with_capacity(rivers.len());
    while order.len() < rivers.len() {
        let ready = |k: &usize| match rivers[*k].end {
            RiverEnd::Joins(joined) => rivers.iter().position(|river| river.drawn == joined).is_none_or(|j| traced[j]),
            _ => true,
        };
        let ends_in_no_lake = |k: &usize| !matches!(rivers[*k].end, RiverEnd::JoinsLake(_));
        let mut waiting = (0..rivers.len()).filter(|&k| !traced[k]);
        let first_ready = waiting.clone().find(|k| ready(k) && ends_in_no_lake(k));
        let next = first_ready.or_else(|| waiting.clone().find(ready)).or_else(|| {
            debug_assert!(false, "the drawn rivers join round a circle; `carried_on` promises they never do");
            waiting.next()
        });
        let Some(next) = next else { break };
        traced[next] = true;
        order.push(&rivers[next]);
    }
    order
}

/// The notes on the drawn rivers of `drawing` whose `channels` (as `trace_channels` made them from
/// `grown`) do not keep to their lines as drawn, in the drawing's order:
///
/// - "River N leaves its drawn line by up to X m." when a point of its channels lies farther from
///   its line as drawn than its corridor, `CORRIDOR_WIDTHS` of its widths or `CORRIDOR_MIN_M`,
///   whichever is more; X is the farthest, in whole metres. For a river the engine carried on past
///   its last drawn point (`RiverEnd::CarriedOn`), the points after the one nearest that point (the
///   first of several as near) do not count: they follow the engine's way to the edge.
/// - "River N does not reach its drawn end; its water goes another way." when the drawing has it
///   end at the edge or on a river (`RiverEnd::Edge` or `Joins`) and its last channel ends farther
///   than its corridor from its last drawn point.
///
/// N counts from 1, and each note is about its river. A river with no channel is not said.
pub(crate) fn line_notes(channels: &[Channel], grown: &Grown, drawing: &Drawing) -> Vec<Note> {
    let mut notes = Vec::new();
    for river in &grown.rivers {
        let drawn = &drawing.rivers[river.drawn];
        let corridor_m = (CORRIDOR_WIDTHS * f64::from(drawn.width_m)).max(CORRIDOR_MIN_M);
        let last_drawn = drawn.points[drawn.points.len() - 1];
        let ours: Vec<&Channel> = channels.iter().filter(|channel| channel.drawn == Some(river.drawn)).collect();
        let mut points: Vec<(f64, f64)> = ours.iter().flat_map(|channel| channel.points.iter().map(|point| (point.x_m, point.z_m))).collect();
        if points.is_empty() {
            continue;
        }
        if river.end == RiverEnd::CarriedOn {
            let from_end_m = |&point: &(f64, f64)| distance_m(point, last_drawn);
            let nearest = (0..points.len()).reduce(|nearest, k| if from_end_m(&points[k]) < from_end_m(&points[nearest]) { k } else { nearest });
            points.truncate(nearest.map_or(0, |nearest| nearest + 1));
        }
        let farthest_m = points.iter().map(|&point| from_line_m(point, &drawn.points)).fold(0.0, f64::max);
        if farthest_m > corridor_m {
            notes.push((About::River(river.drawn), format!("River {} leaves its drawn line by up to {farthest_m:.0} m.", river.drawn + 1)));
        }
        let ends = ours.last().and_then(|channel| channel.points.last()).map(|point| (point.x_m, point.z_m));
        let drawn_to_end = matches!(river.end, RiverEnd::Edge | RiverEnd::Joins(_));
        if drawn_to_end && ends.is_some_and(|ends| distance_m(ends, last_drawn) > corridor_m) {
            notes.push((About::River(river.drawn), format!("River {} does not reach its drawn end; its water goes another way.", river.drawn + 1)));
        }
    }
    notes
}

/// The distance from `point` to the nearest point of the line through `line`, in metres; the
/// distance to its one point when it has one.
fn from_line_m(point: (f64, f64), line: &[(f64, f64)]) -> f64 {
    let along = line.windows(2).map(|pair| {
        let ((ax, az), (bx, bz)) = (pair[0], pair[1]);
        let (dx, dz) = (bx - ax, bz - az);
        let squared = dx * dx + dz * dz;
        let share = if squared > 0.0 { (((point.0 - ax) * dx + (point.1 - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        distance_m(point, (ax + dx * share, az + dz * share))
    });
    along.fold(line.first().map_or(f64::INFINITY, |&first| distance_m(point, first)), f64::min)
}

/// The land model's grid, as `Grown` lays it out, and the world on it.
struct Cells<'a> {
    grown: &'a Grown,
    /// Cells per side.
    side: usize,
    /// World `x` of the first column, and world `z` of the first row, in metres.
    origin_m: f64,
    /// The world's span along both axes, in metres: the middles of its first and its last 1 m
    /// cells.
    world_m: (f64, f64),
}

impl<'a> Cells<'a> {
    fn new(grown: &'a Grown, size_m: u32) -> Self {
        let half_m = f64::from(size_m) / 2.0;
        Self { grown, side: grown.heights.width, origin_m: -half_m - f64::from(GROW_MARGIN_M), world_m: (-half_m, half_m - 1.0) }
    }

    fn count(&self) -> usize {
        self.side * self.side
    }

    /// World `(x, z)` of `cell`, in metres.
    fn at_m(&self, cell: usize) -> (f64, f64) {
        let position_m = |index: usize| self.origin_m + index as f64 * f64::from(GROW_CELL_M);
        (position_m(cell % self.side), position_m(cell / self.side))
    }

    /// Whether world `(x, z)` lies inside the world: within the span of its cells, its ends included.
    fn holds(&self, (x, z): (f64, f64)) -> bool {
        let (first_m, last_m) = self.world_m;
        (first_m..=last_m).contains(&x) && (first_m..=last_m).contains(&z)
    }

    /// Whether `cell` lies inside the world.
    fn inside(&self, cell: usize) -> bool {
        self.holds(self.at_m(cell))
    }

    /// The cell nearest world `(x, z)`, on the grid.
    fn nearest(&self, (x, z): (f64, f64)) -> usize {
        let index = |at_m: f64| {
            (((at_m - self.origin_m) / f64::from(GROW_CELL_M)).round().max(0.0) as usize).min(self.side - 1)
        };
        index(z) * self.side + index(x)
    }

    fn receiver(&self, cell: usize) -> usize {
        self.grown.receivers[cell] as usize
    }

    fn drained_m2(&self, cell: usize) -> f32 {
        self.grown.drained_m2.data[cell]
    }

    /// The distance between the centres of two cells, in metres.
    fn apart_m(&self, a: usize, b: usize) -> f64 {
        distance_m(self.at_m(a), self.at_m(b))
    }
}

/// Each cell's lake, when it is a lake cell: the first lake of `grown.lakes` within
/// `LAKE_REACH_RADII` radii of whose centre it lies and within `LAKE_LEVEL_M` of whose level its
/// flooded height (ground plus water) lies, as an index into `drawing.lakes`.
fn lake_cells(cells: &Cells, drawing: &Drawing) -> Vec<Option<usize>> {
    let grown = cells.grown;
    (0..cells.count())
        .map(|cell| {
            let water_m = grown.water_depth_m.data[cell];
            if water_m <= LAKE_DEPTH_M {
                return None;
            }
            let flooded_m = f64::from(grown.heights.data[cell]) + f64::from(water_m);
            let (x, z) = cells.at_m(cell);
            grown.lakes.iter().find_map(|listed| {
                let lake = &drawing.lakes[listed.drawn];
                let near = (x - lake.x_m).hypot(z - lake.z_m) <= LAKE_REACH_RADII * f64::from(lake.radius_m);
                (near && (flooded_m - f64::from(listed.level_m)).abs() <= LAKE_LEVEL_M).then_some(listed.drawn)
            })
        })
        .collect()
}

/// The cells of a channel being traced, upstream first.
#[derive(Debug, Default)]
struct Run {
    /// The drawn lake it flows out of.
    from_lake: Option<usize>,
    /// The cell outside the world it comes in from, and that cell's distance along the path.
    from_outside: Option<(usize, f64)>,
    cells: Vec<usize>,
    /// Each cell's distance along the path from the path's first cell, in metres.
    along_m: Vec<f64>,
    /// The cell outside the world it goes out to, and that cell's distance along the path.
    to_outside: Option<(usize, f64)>,
    /// For a channel that runs straight from its last cell to a river, the point it ends on there.
    junction_m: Option<(f64, f64)>,
}

impl Run {
    fn starting(from_lake: Option<usize>, from_outside: Option<(usize, f64)>) -> Self {
        Self { from_lake, from_outside, ..Self::default() }
    }

    fn push(&mut self, cell: usize, along_m: f64) {
        self.cells.push(cell);
        self.along_m.push(along_m);
    }
}

/// Why a channel's end was not moved onto the channel it joins as the rules ask
/// (`Tracing::snapped`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Unsnapped {
    /// The point it was moved to, the nearest clear of the lakes, lies `moved_m` from the cell it
    /// ended on, more than `SNAP_REACH_M`, and that cell lies outside the joined channel's bed.
    Far { from_m: (f64, f64), to_m: (f64, f64), moved_m: f64 },
    /// Every straight line onto that channel crosses a lake cell; it was not moved.
    AcrossALake,
}

impl std::fmt::Display for Unsnapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Far { from_m, to_m, moved_m } => {
                write!(f, "its end at {from_m:?} was moved {moved_m} m onto the channel it joins, to {to_m:?}")
            }
            Self::AcrossALake => write!(f, "every straight line from it onto the channel it joins crosses a lake cell"),
        }
    }
}

/// How a traced channel ends.
#[derive(Debug, Clone, Copy)]
enum CourseEnd {
    Edge,
    Lake(usize),
    /// On the channel its last cell belongs to.
    Joins,
    /// On the course of this index, at the run's `junction_m`.
    Onto(usize),
}

/// How wide a traced channel is.
#[derive(Debug, Clone, Copy)]
enum Width {
    /// The same everywhere.
    Stated(f32),
    /// From `from_m` at the start of its path to `to_m` at `full_at_m` along it, and `to_m` beyond.
    Ramp { from_m: f32, to_m: f32, full_at_m: f64 },
    /// By the brook rule, and never wider than what it ends on.
    Brook,
}

/// A channel as the cells it runs through.
#[derive(Debug)]
struct Course {
    drawn: Option<usize>,
    run: Run,
    end: CourseEnd,
    width: Width,
}

impl Course {
    /// Its own cells: all but the last when that is a cell of the channel it ends on.
    fn own_cells(&self) -> &[usize] {
        let joined = usize::from(matches!(self.end, CourseEnd::Joins));
        &self.run.cells[..self.run.cells.len() - joined]
    }
}

/// A course's line once it is made (`Tracing::make`), with its widths, and how it ends.
#[derive(Debug)]
struct Made {
    points: Vec<ChannelPoint>,
    /// `Channel` holds the index of a course, which is kept.
    end: ChannelEnd,
    /// Whether it is kept as a channel: it has the two points a channel needs, and is no brook
    /// whose water is not its own (`Tracing::water_not_its_own`).
    kept: bool,
}

/// The channels as they are traced.
struct Tracing<'a> {
    cells: Cells<'a>,
    /// Each cell's lake, when it is a lake cell or part of a lake.
    lake_of: Vec<Option<usize>>,
    /// Whether each cell is on a drawn river's path.
    on_path: Vec<bool>,
    /// The course each cell is an own cell of.
    course_of: Vec<Option<usize>>,
    courses: Vec<Course>,
    /// Each course's line, once made; every course is made before any course that ends on it.
    made: Vec<Option<Made>>,
}

impl Tracing<'_> {
    /// Adds `course`, the owner of its own cells.
    fn add(&mut self, course: Course) {
        for &cell in course.own_cells() {
            self.course_of[cell] = Some(self.courses.len());
        }
        self.courses.push(course);
        self.made.push(None);
    }

    /// Whether `point` lies on a lake cell.
    fn on_a_lake(&self, point: (f64, f64)) -> bool {
        self.lake_of[self.cells.nearest(point)].is_some()
    }

    /// Whether course `index` is made and kept as a channel.
    fn kept(&self, index: usize) -> bool {
        self.made[index].as_ref().is_some_and(|made| made.kept)
    }

    /// Traces the path of `river`, whose last drawn point is `drawn_end`, as `trace_channels`
    /// describes it, and adds and makes its channels.
    fn trace_river(&mut self, river: &GrownRiver, drawn_end: (f64, f64), rules: &WaterRules) {
        let ends_in = match river.end {
            RiverEnd::JoinsLake(lake) => Some(lake),
            _ => None,
        };
        let mut traced: Vec<(Run, CourseEnd)> = Vec::new();
        let mut open: Option<Run> = None;
        // The cells this river put on its path, in the path's order.
        let mut marked: Vec<usize> = Vec::new();
        let (mut cell, mut along_m) = (self.cells.nearest(river.line[0]), 0.0);
        // The cell before on the path, and its distance along.
        let mut previous: Option<(usize, f64)> = None;
        loop {
            let lake = self.lake_of[cell];
            if let (Some(lake), true) = (lake, lake == ends_in) {
                traced.extend(open.take().map(|run| (run, CourseEnd::Lake(lake))));
                break;
            }
            if self.on_path[cell] {
                if let Some(mut run) = open.take() {
                    let end = match lake {
                        Some(lake) if run.from_lake == Some(lake) => {
                            // Back into the lake it left, on an earlier path: the stretch is part
                            // of the lake.
                            for &stretch in &run.cells {
                                self.lake_of[stretch] = Some(lake);
                            }
                            None
                        }
                        Some(lake) => Some(CourseEnd::Lake(lake)),
                        None if !self.cells.inside(cell) => {
                            run.to_outside = Some((cell, along_m));
                            Some(CourseEnd::Edge)
                        }
                        None => {
                            run.push(cell, along_m);
                            Some(CourseEnd::Joins)
                        }
                    };
                    traced.extend(end.map(|end| (run, end)));
                }
                break;
            }
            self.on_path[cell] = true;
            marked.push(cell);
            if let Some(lake) = lake {
                if let Some(run) = open.take() {
                    if run.from_lake == Some(lake) {
                        // Back into the lake it left: the stretch is part of the lake.
                        for &stretch in &run.cells {
                            self.lake_of[stretch] = Some(lake);
                        }
                    } else {
                        traced.push((run, CourseEnd::Lake(lake)));
                    }
                }
            } else if !self.cells.inside(cell) {
                if let Some(mut run) = open.take() {
                    run.to_outside = Some((cell, along_m));
                    traced.push((run, CourseEnd::Edge));
                }
            } else {
                let (lake_of, cells) = (&self.lake_of, &self.cells);
                let run = open.get_or_insert_with(|| match previous {
                    Some((before, _)) if lake_of[before].is_some() => Run::starting(lake_of[before], None),
                    Some((before, before_along_m)) if !cells.inside(before) => Run::starting(None, Some((before, before_along_m))),
                    _ => Run::starting(None, None),
                });
                run.push(cell, along_m);
            }
            let next = self.cells.receiver(cell);
            if next == cell {
                traced.extend(open.take().map(|run| (run, CourseEnd::Edge)));
                break;
            }
            previous = Some((cell, along_m));
            along_m += self.cells.apart_m(cell, next);
            cell = next;
        }

        // The river's own cell nearest its last drawn point: its run and its place there, the
        // first along the path on a tie.
        let mut nearest_end: Option<(f64, usize, usize)> = None;
        for (run_index, (run, end)) in traced.iter().enumerate() {
            let own = run.cells.len() - usize::from(matches!(end, CourseEnd::Joins));
            for (place, &own_cell) in run.cells[..own].iter().enumerate() {
                let from_end_m = distance_m(self.cells.at_m(own_cell), drawn_end);
                if nearest_end.is_none_or(|(nearest_m, ..)| from_end_m < nearest_m) {
                    nearest_end = Some((from_end_m, run_index, place));
                }
            }
        }

        // A river that joins another runs from its own cell nearest its last drawn point straight
        // to that river; the rest of its path is given back, off every drawn path.
        if let (RiverEnd::Joins(joined), Some((_, run_index, place))) = (river.end, nearest_end) {
            let cut_at = traced[run_index].0.cells[place];
            if let Some((onto, junction_m)) = self.junction(joined, cut_at) {
                traced.truncate(run_index + 1);
                let (run, end) = &mut traced[run_index];
                run.cells.truncate(place + 1);
                run.along_m.truncate(place + 1);
                run.to_outside = None;
                run.junction_m = Some(junction_m);
                *end = CourseEnd::Onto(onto);
                let kept = marked.iter().position(|&cell| cell == cut_at).map_or(marked.len(), |at| at + 1);
                for &cell in &marked[kept..] {
                    self.on_path[cell] = false;
                }
            }
        }

        let width = if river.enters_at_edge {
            Width::Stated(river.width_m)
        } else {
            let full_at_m = nearest_end.map_or(0.0, |(_, run, place)| traced[run].0.along_m[place]);
            Width::Ramp { from_m: rules.brook_width_m.min(river.width_m), to_m: river.width_m, full_at_m }
        };
        let first = self.courses.len();
        for (run, end) in traced {
            self.add(Course { drawn: Some(river.drawn), run, end, width });
        }
        for index in first..self.courses.len() {
            self.make(index, rules);
        }
    }

    /// Where a channel from `from` runs straight onto drawn river `joined`: the course of the
    /// channels made so far for it whose point lies nearest the centre of cell `from`, the first on
    /// a tie, and that point. `None` when it has no channel, or the straight line from the cell,
    /// cut into points (`smooth::inner_points`), would cross a lake cell.
    fn junction(&self, joined: usize, from: usize) -> Option<(usize, (f64, f64))> {
        let from_m = self.cells.at_m(from);
        let mut nearest: Option<(f64, usize, (f64, f64))> = None;
        for (index, course) in self.courses.iter().enumerate() {
            let Some(made) = &self.made[index] else { continue };
            if course.drawn != Some(joined) || !made.kept {
                continue;
            }
            for point in &made.points {
                let at_m = (point.x_m, point.z_m);
                let apart_m = distance_m(from_m, at_m);
                if nearest.is_none_or(|(nearest_m, ..)| apart_m < nearest_m) {
                    nearest = Some((apart_m, index, at_m));
                }
            }
        }
        let (_, onto, junction_m) = nearest?;
        let crosses_a_lake = smooth::inner_points(from_m, junction_m).any(|point| self.on_a_lake(point));
        (!crosses_a_lake).then_some((onto, junction_m))
    }

    /// Traces the brooks as `trace_channels` describes them, and adds and makes them, most drained
    /// first.
    fn trace_brooks(&mut self, rules: &WaterRules) {
        let (cells, count) = (&self.cells, self.cells.count());
        let is_brook: Vec<bool> = (0..count)
            .map(|cell| {
                cells.inside(cell)
                    && self.lake_of[cell].is_none()
                    && !self.on_path[cell]
                    && cells.drained_m2(cell) >= rules.brook_start_m2
            })
            .collect();
        let comes_in = |cell: usize| !cells.inside(cell) && cells.drained_m2(cell) >= rules.brook_start_m2;
        // Of the brook cells, lake cells and cells coming in across the border that drain into
        // each brook cell, the one draining most, the first by index on a tie.
        let mut main_in: Vec<Option<usize>> = vec![None; count];
        for cell in 0..count {
            let next = cells.receiver(cell);
            if next == cell || !is_brook[next] || !(is_brook[cell] || self.lake_of[cell].is_some() || comes_in(cell)) {
                continue;
            }
            if main_in[next].is_none_or(|main| cells.drained_m2(cell) > cells.drained_m2(main)) {
                main_in[next] = Some(cell);
            }
        }

        let mut brooks = Vec::new();
        for source in (0..count).filter(|&cell| is_brook[cell] && main_in[cell].is_none_or(|main| !is_brook[main])) {
            let mut run = match main_in[source] {
                Some(main) if self.lake_of[main].is_some() => Run::starting(self.lake_of[main], None),
                Some(main) => Run::starting(None, Some((main, -cells.apart_m(main, source)))),
                None => Run::starting(None, None),
            };
            let (mut cell, mut along_m) = (source, 0.0);
            run.push(cell, along_m);
            let end = loop {
                let next = cells.receiver(cell);
                if next == cell {
                    break CourseEnd::Edge;
                }
                along_m += cells.apart_m(cell, next);
                if !cells.inside(next) {
                    run.to_outside = Some((next, along_m));
                    break CourseEnd::Edge;
                }
                if let Some(lake) = self.lake_of[next] {
                    break CourseEnd::Lake(lake);
                }
                if is_brook[next] && main_in[next] == Some(cell) {
                    run.push(next, along_m);
                    cell = next;
                } else if is_brook[next] || self.on_path[next] {
                    run.push(next, along_m);
                    break CourseEnd::Joins;
                } else {
                    debug_assert!(false, "a brook drains into a cell inside the world that drains more yet is no brook's, river's or lake's");
                    break CourseEnd::Edge;
                }
            };
            brooks.push(Course { drawn: None, run, end, width: Width::Brook });
        }
        // The ground drained at a brook's last point, which is its own last cell's, and that cell.
        let last_drained = |brook: &Course| {
            let own = brook.own_cells();
            let last = own[own.len() - 1];
            (cells.drained_m2(last), last)
        };
        brooks.sort_by(|a, b| {
            let ((drained_a, cell_a), (drained_b, cell_b)) = (last_drained(a), last_drained(b));
            drained_b.total_cmp(&drained_a).then(cell_a.cmp(&cell_b))
        });
        let first = self.courses.len();
        for brook in brooks {
            self.add(brook);
        }
        for index in first..self.courses.len() {
            self.make(index, rules);
        }
    }

    /// How course `index` ends, on a course kept (`ChannelEnd::Channel` holding its index): one
    /// ending on a course left out takes that course's end.
    fn end_of(&self, mut index: usize) -> ChannelEnd {
        loop {
            let course = &self.courses[index];
            let joined = match course.end {
                CourseEnd::Edge => return ChannelEnd::Edge,
                CourseEnd::Lake(lake) => return ChannelEnd::Lake(lake),
                CourseEnd::Onto(joined) => joined,
                CourseEnd::Joins => match course.run.cells.last().and_then(|&cell| self.course_of[cell]) {
                    Some(joined) => joined,
                    None => {
                        debug_assert!(false, "a course ends on a cell no course owns");
                        return ChannelEnd::Edge;
                    }
                },
            };
            if self.made[joined].is_none() {
                debug_assert!(false, "a course is made before the course it ends on");
                return ChannelEnd::Edge;
            }
            if self.kept(joined) {
                return ChannelEnd::Channel(joined);
            }
            // A course is joined only downstream, so this ends.
            index = joined;
        }
    }

    /// Makes course `index`'s line and widths, and decides whether it is kept, as `trace_channels`
    /// describes it; the course it ends on must be made.
    fn make(&mut self, index: usize, rules: &WaterRules) {
        let end = self.end_of(index);
        let course = &self.courses[index];
        let run = &course.run;
        let own = course.own_cells();
        // The ground drained at each cell; at the cell of the channel it ends on, its own last cell's.
        let drained_m2 = |k: usize| f64::from(self.cells.drained_m2(own[k.min(own.len() - 1)]));
        let corner = |cell: usize, along_m: f64, drained_m2: f64| Sample { at_m: self.cells.at_m(cell), along_m, drained_m2 };
        let mut corners: Vec<Sample> = Vec::new();
        corners.extend(run.from_outside.map(|(cell, along_m)| corner(cell, along_m, drained_m2(0))));
        corners.extend(run.cells.iter().enumerate().map(|(k, &cell)| corner(cell, run.along_m[k], drained_m2(k))));
        corners.extend(run.to_outside.map(|(cell, along_m)| corner(cell, along_m, drained_m2(run.cells.len()))));
        if let (Some(junction_m), Some(last)) = (run.junction_m, corners.last().copied()) {
            let along_m = last.along_m + distance_m(last.at_m, junction_m);
            corners.push(Sample { at_m: junction_m, along_m, drained_m2: last.drained_m2 });
        }
        if let (CourseEnd::Joins, ChannelEnd::Channel(joined)) = (course.end, end) {
            if let Err(unsnapped) = self.snapped(&mut corners, joined) {
                debug_assert!(false, "a channel's end is not moved onto the channel it joins as the rules ask: {unsnapped}");
            }
        }

        let mut points = smooth::resampled(&corners);
        let clear = |point: &Sample| self.cells.holds(point.at_m) && !self.on_a_lake(point.at_m);
        if run.from_outside.is_some() {
            // The points from the cell outside to the first cell: those before the last that is
            // not clear go.
            let coming_in = smooth::steps(corners[0].at_m, corners[1].at_m);
            if let Some(last_out) = points[..coming_in].iter().rposition(|point| !clear(point)) {
                points.drain(..=last_out);
            }
        }
        if run.to_outside.is_some() {
            // The points from the last cell to the cell outside: those from the first that is
            // not clear go.
            let going_out = smooth::steps(corners[corners.len() - 2].at_m, corners[corners.len() - 1].at_m);
            let last_cell = points.len() - 1 - going_out;
            if let Some(first_out) = points[last_cell..].iter().position(|point| !clear(point)) {
                points.truncate(last_cell + first_out);
            }
        }
        let points = smooth::smoothed(points, |point| self.on_a_lake(point));

        // A brook is never wider than the channel it ends on is at that channel's point nearest
        // the brook's last, the first on a tie.
        let cap_m = match (course.width, end, points.last()) {
            (Width::Brook, ChannelEnd::Channel(joined), Some(last)) => match &self.made[joined] {
                Some(joined) => {
                    let from_last = |point: &ChannelPoint| distance_m((point.x_m, point.z_m), last.at_m);
                    let nearest = joined.points.iter().min_by(|a, b| from_last(a).total_cmp(&from_last(b)));
                    nearest.map_or(f32::INFINITY, |point| point.width_m)
                }
                None => {
                    debug_assert!(false, "a course is made before the course it ends on");
                    f32::INFINITY
                }
            },
            _ => f32::INFINITY,
        };
        let points: Vec<ChannelPoint> = (points.iter())
            .map(|sample| {
                let drained_m2 = sample.drained_m2 as f32;
                let width_m = match course.width {
                    Width::Stated(width_m) => width_m,
                    Width::Ramp { from_m, to_m, full_at_m } => ramp_m(from_m, to_m, sample.along_m / full_at_m),
                    Width::Brook => brook_width_m(drained_m2, rules).min(cap_m),
                };
                ChannelPoint { x_m: sample.at_m.0, z_m: sample.at_m.1, width_m, drained_m2 }
            })
            .collect();
        let kept = points.len() >= 2 && !(course.drawn.is_none() && self.water_not_its_own(run.from_lake, &points, end));
        self.made[index] = Some(Made { points, end, kept });
    }

    /// Whether a brook flowing out of `from_lake`, whose line is `points` and which ends `end`, has
    /// no water of its own (`trace_channels`): it ends on a channel inside whose bed all its points
    /// lie, or in the lake it flows out of.
    fn water_not_its_own(&self, from_lake: Option<usize>, points: &[ChannelPoint], end: ChannelEnd) -> bool {
        match end {
            ChannelEnd::Channel(joined) => (self.made[joined].as_ref())
                .is_some_and(|joined| points.iter().all(|point| inside_bed(&joined.points, point))),
            ChannelEnd::Lake(lake) => from_lake == Some(lake),
            ChannelEnd::Edge => false,
        }
    }

    /// Moves the last of `corners`, the cell of the channel it ends on, onto that channel: to the
    /// point of course `joined`'s line nearest it (the first on a tie) of those the straight line
    /// to which, from the corner before, crosses no lake cell. That point should lie within
    /// `SNAP_REACH_M` of the cell. It may lie farther when the cell lies inside the joined
    /// channel's bed (`bed::inside_bed`: within half that channel's width, taken at its point
    /// nearest the cell and no less than `bed::HALF_WIDTH_MIN_M`, of its line), where the water is
    /// the joined channel's. When it lies farther
    /// from a cell outside that bed the end is moved there all the same, as no rule says what else
    /// such a channel should do, and `Unsnapped::Far` says so. When every such line crosses a lake
    /// cell the end is left where it was (`Unsnapped::AcrossALake`).
    fn snapped(&self, corners: &mut [Sample], joined: usize) -> Result<(), Unsnapped> {
        let Some(made) = &self.made[joined] else {
            debug_assert!(false, "a course is made before the course it ends on");
            return Ok(());
        };
        let count = corners.len();
        let (from_m, cell_m) = (corners[count - 2].at_m, corners[count - 1].at_m);
        let mut nearest: Vec<(f64, (f64, f64))> = (made.points.iter())
            .map(|point| (point.x_m, point.z_m))
            .map(|at_m| (distance_m(cell_m, at_m), at_m))
            .collect();
        // A stable sort keeps the first of points equally near first.
        nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
        let crosses_a_lake = |to_m: (f64, f64)| smooth::inner_points(from_m, to_m).any(|point| self.on_a_lake(point));
        let clear = nearest.iter().map(|&(_, at_m)| at_m).find(|&to_m| !crosses_a_lake(to_m));
        let Some(at_m) = clear else { return Err(Unsnapped::AcrossALake) };
        let along_m = corners[count - 2].along_m + distance_m(from_m, at_m);
        corners[count - 1] = Sample { at_m, along_m, ..corners[count - 1] };
        let cell = ChannelPoint { x_m: cell_m.0, z_m: cell_m.1, width_m: 0.0, drained_m2: 0.0 };
        match distance_m(cell_m, at_m) {
            moved_m if moved_m > SNAP_REACH_M && !inside_bed(&made.points, &cell) => {
                Err(Unsnapped::Far { from_m: cell_m, to_m: at_m, moved_m })
            }
            _ => Ok(()),
        }
    }

    /// The courses kept as channels: the drawn ones by their river's index in the drawing, each
    /// river's in the order traced, then the brooks.
    fn into_channels(self) -> Vec<Channel> {
        let mut order: Vec<usize> = (0..self.courses.len()).filter(|&index| self.kept(index)).collect();
        order.sort_by_key(|&index| self.courses[index].drawn.unwrap_or(usize::MAX));
        let mut listed_at = vec![None; self.courses.len()];
        for (place, &index) in order.iter().enumerate() {
            listed_at[index] = Some(place);
        }
        let Self { courses, mut made, .. } = self;
        (order.iter())
            .filter_map(|&index| {
                let Made { points, end, .. } = made[index].take()?;
                let end = match end {
                    ChannelEnd::Channel(joined) => listed_at[joined].map_or_else(
                        || {
                            debug_assert!(false, "a channel ends on a course that is not listed");
                            ChannelEnd::Edge
                        },
                        ChannelEnd::Channel,
                    ),
                    other => other,
                };
                let course = &courses[index];
                Some(Channel { drawn: course.drawn, from_lake: course.run.from_lake, points, end })
            })
            .collect()
    }
}

/// A width going from `from_m` to `to_m` as `share` goes from 0 to 1, and `to_m` beyond.
fn ramp_m(from_m: f32, to_m: f32, share: f64) -> f32 {
    if share >= 1.0 || share.is_nan() {
        return to_m;
    }
    (f64::from(from_m) + (f64::from(to_m) - f64::from(from_m)) * share) as f32
}

/// A brook's width where `drained_m2` drain: `brook_width_m * (drained / brook_start_m2)^width_area_power`.
fn brook_width_m(drained_m2: f32, rules: &WaterRules) -> f32 {
    let relative = f64::from(drained_m2) / f64::from(rules.brook_start_m2);
    (f64::from(rules.brook_width_m) * relative.powf(f64::from(rules.width_area_power))) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::{Country, COUNTRY_NAMES};
    use crate::drawing::{Lake, River};
    use crate::grid::Grid;
    use crate::grow::tests::{example_worlds, grown, plain, ExampleWorld};
    use crate::grow::{GrownLake, RiverEnd};

    /// The channels of an example world under `rules`.
    fn traced(world: &ExampleWorld, rules: &WaterRules) -> Vec<Channel> {
        trace_channels(&world.land, &world.drawing, rules)
    }

    /// World `x` of column `index`, or world `z` of row `index`, as `Grown` documents its cells.
    fn position_m(size_m: u32, index: usize) -> f64 {
        index as f64 * 8.0 - f64::from(size_m) / 2.0 - 256.0
    }

    /// The cell of `land` nearest `point`.
    fn cell_at(land: &Grown, size_m: u32, point: &ChannelPoint) -> usize {
        let side = land.heights.width;
        let index = |at_m: f64| (((at_m - position_m(size_m, 0)) / 8.0).round().max(0.0) as usize).min(side - 1);
        index(point.z_m) * side + index(point.x_m)
    }

    /// Whether each cell of `land` is a lake cell: within 1.5 radii of the centre of a lake the
    /// model lists, holding more than 0.3 m of water whose surface lies within 0.05 m of that
    /// lake's level.
    fn lake_cells(land: &Grown, drawing: &Drawing) -> Vec<bool> {
        let side = land.heights.width;
        (0..side * side)
            .map(|cell| {
                let (x, z) = (position_m(drawing.size_m, cell % side), position_m(drawing.size_m, cell / side));
                let (water_m, flooded_m) = (land.water_depth_m.data[cell], land.heights.data[cell] + land.water_depth_m.data[cell]);
                water_m > 0.3
                    && land.lakes.iter().any(|lake| {
                        let drawn = drawing.lakes[lake.drawn];
                        (x - drawn.x_m).hypot(z - drawn.z_m) <= 1.5 * f64::from(drawn.radius_m)
                            && (flooded_m - lake.level_m).abs() <= 0.05
                    })
            })
            .collect()
    }

    /// Whether `point` lies within the span of the world's 1 m cells, from `-size_m / 2` to
    /// `size_m / 2 - 1` m on both axes.
    fn inside_the_world(drawing: &Drawing, point: &ChannelPoint) -> bool {
        let half_m = f64::from(drawing.size_m) / 2.0;
        let within = |at_m: f64| (-half_m..=half_m - 1.0).contains(&at_m);
        within(point.x_m) && within(point.z_m)
    }

    /// How far `point` lies inside the edge of the span of the world's 1 m cells, in metres.
    fn inside_the_edge_m(drawing: &Drawing, point: &ChannelPoint) -> f64 {
        let half_m = f64::from(drawing.size_m) / 2.0;
        let from_edge = |at_m: f64| (at_m + half_m).min(half_m - 1.0 - at_m);
        from_edge(point.x_m).min(from_edge(point.z_m))
    }

    fn apart_m(a: &ChannelPoint, b: &ChannelPoint) -> f64 {
        (b.x_m - a.x_m).hypot(b.z_m - a.z_m)
    }

    /// The distance from `point` to the nearest point of the line through `line`, in metres.
    fn distance_to_line_m(point: &ChannelPoint, line: &[(f64, f64)]) -> f64 {
        line.windows(2)
            .map(|pair| {
                let ((ax, az), (bx, bz)) = (pair[0], pair[1]);
                let (dx, dz) = (bx - ax, bz - az);
                let squared = dx * dx + dz * dz;
                let along =
                    if squared > 0.0 { (((point.x_m - ax) * dx + (point.z_m - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
                (ax + dx * along - point.x_m).hypot(az + dz * along - point.z_m)
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The width of `channel` at its point nearest `point`, the first on a tie.
    fn width_near(channel: &Channel, point: &ChannelPoint) -> f32 {
        let nearest = channel.points.iter().min_by(|a, b| apart_m(a, point).total_cmp(&apart_m(b, point)));
        nearest.expect("a channel with points").width_m
    }

    /// A brook's width by the rule: `brook_width_m * (drained / brook_start_m2)^width_area_power`.
    fn brook_rule_m(drained_m2: f32, rules: &WaterRules) -> f32 {
        let relative = f64::from(drained_m2) / f64::from(rules.brook_start_m2);
        (f64::from(rules.brook_width_m) * relative.powf(f64::from(rules.width_area_power))) as f32
    }

    #[test]
    fn the_example_rivers_become_three_drawn_channels() {
        let expected = [
            (Some(0), None, ChannelEnd::Lake(0)),
            (Some(0), Some(0), ChannelEnd::Edge),
            (Some(1), None, ChannelEnd::Channel(1)),
        ];
        let mut failures = Vec::new();
        for world in example_worlds() {
            let channels = traced(world, &WaterRules::default());
            let drawn: Vec<(Option<usize>, Option<usize>, ChannelEnd)> =
                channels.iter().filter(|channel| channel.drawn.is_some()).map(|c| (c.drawn, c.from_lake, c.end)).collect();
            if drawn != expected {
                failures.push(format!("{}: {drawn:?}", world.name));
            }
            let first_brook = channels.iter().position(|channel| channel.drawn.is_none()).unwrap_or(channels.len());
            if first_brook != drawn.len() {
                failures.push(format!("{}: a brook comes before a drawn channel, at {first_brook}", world.name));
            }
        }
        assert!(failures.is_empty(), "expected {expected:?}: {failures:#?}");
    }

    /// Every point of a drawn channel lies within `max(3 * width, 40)` metres of its river's line
    /// as drawn.
    #[test]
    fn a_drawn_channel_keeps_to_its_corridor() {
        let mut failures = Vec::new();
        for world in example_worlds() {
            for channel in traced(world, &WaterRules::default()) {
                let Some(drawn) = channel.drawn else { continue };
                let river = &world.drawing.rivers[drawn];
                let reach_m = (3.0 * f64::from(river.width_m)).max(40.0);
                let farthest_m = channel.points.iter().map(|point| distance_to_line_m(point, &river.points)).fold(0.0, f64::max);
                if farthest_m > reach_m {
                    failures.push(format!("{}: river {} strays {farthest_m:.1} m from its line, beyond {reach_m} m", world.name, drawn + 1));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn widths_follow_the_rules() {
        let rules = WaterRules::default();
        let mut failures = Vec::new();
        for world in example_worlds() {
            let name = &world.name;
            let (big, spring) = (&world.drawing.rivers[0], &world.drawing.rivers[1]);
            let channels = traced(world, &rules);
            let widths = |drawn: usize| -> Vec<f32> {
                (channels.iter().filter(|channel| channel.drawn == Some(drawn)))
                    .flat_map(|channel| channel.points.iter().map(|point| point.width_m))
                    .collect()
            };
            let river_0 = widths(0);
            if river_0.is_empty() || river_0.iter().any(|&width| width != big.width_m) {
                failures.push(format!("{name}: river 1 is not {} m wide everywhere", big.width_m));
            }
            let river_1 = widths(1);
            let (first, last) = (river_1.first().copied(), river_1.last().copied());
            if first != Some(rules.brook_width_m) || last != Some(spring.width_m) {
                failures.push(format!("{name}: river 2's width runs from {first:?} to {last:?} m"));
            }
            if let Some(pair) = river_1.windows(2).find(|pair| pair[1] < pair[0]) {
                failures.push(format!("{name}: river 2 narrows from {} to {} m", pair[0], pair[1]));
            }
            for (index, brook) in channels.iter().enumerate().filter(|(_, channel)| channel.drawn.is_none()) {
                let last = brook.points[brook.points.len() - 1];
                let cap_m = match brook.end {
                    ChannelEnd::Channel(joined) => width_near(&channels[joined], &last),
                    _ => f32::INFINITY,
                };
                if last.width_m > cap_m {
                    failures.push(format!("{name}: brook {index} is {} m wide where it joins a channel {cap_m} m wide", last.width_m));
                }
                let off_rule = brook.points.iter().find(|point| {
                    let expected = brook_rule_m(point.drained_m2, &rules).min(cap_m);
                    (point.width_m - expected).abs() > 1e-6 * expected
                });
                if let Some(point) = off_rule {
                    failures.push(format!(
                        "{name}: brook {index} is {} m wide where {} m2 drain; the rule gives {} m and the cap {cap_m} m",
                        point.width_m,
                        point.drained_m2,
                        brook_rule_m(point.drained_m2, &rules)
                    ));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn channels_are_well_formed() {
        let mut failures = Vec::new();
        for world in example_worlds() {
            let (drawing, land) = (&world.drawing, &world.land);
            let lakes = lake_cells(land, drawing);
            let channels = traced(world, &WaterRules::default());
            for (index, channel) in channels.iter().enumerate() {
                let name = format!("{}: channel {index} ({:?})", world.name, channel.drawn);
                if channel.points.len() < 2 {
                    failures.push(format!("{name} has {} points", channel.points.len()));
                }
                for pair in channel.points.windows(2) {
                    let (from, to) = (pair[0], pair[1]);
                    if apart_m(&from, &to) > 4.0 + 1e-9 {
                        failures.push(format!("{name}: points {} m apart at ({}, {})", apart_m(&from, &to), to.x_m, to.z_m));
                    }
                    if to.drained_m2 < 0.99 * from.drained_m2 {
                        failures.push(format!("{name}: the ground drained falls from {} to {} m2", from.drained_m2, to.drained_m2));
                    }
                }
                for point in &channel.points {
                    if !inside_the_world(drawing, point) {
                        failures.push(format!("{name}: the point ({}, {}) lies outside the world", point.x_m, point.z_m));
                    }
                    if lakes[cell_at(land, drawing.size_m, point)] {
                        failures.push(format!("{name}: the point ({}, {}) lies on a lake cell", point.x_m, point.z_m));
                    }
                }
                match channel.end {
                    ChannelEnd::Channel(joined) if joined >= channels.len() || joined == index => {
                        failures.push(format!("{name} ends on channel {joined} of {}", channels.len()));
                    }
                    ChannelEnd::Lake(lake) if lake >= drawing.lakes.len() => failures.push(format!("{name} ends in lake {lake}")),
                    _ => {}
                }
                if channel.from_lake.is_some_and(|lake| lake >= drawing.lakes.len()) {
                    failures.push(format!("{name} flows out of lake {:?}", channel.from_lake));
                }
                // A junction lies exactly on a point of the channel joined.
                if let (ChannelEnd::Channel(joined), Some(last)) = (channel.end, channel.points.last()) {
                    let at_last = |point: &ChannelPoint| (point.x_m, point.z_m) == (last.x_m, last.z_m);
                    let on_it = channels.get(joined).is_some_and(|joined| joined.points.iter().any(at_last));
                    if !on_it {
                        failures.push(format!("{name} ends at ({}, {}), on no point of channel {joined}", last.x_m, last.z_m));
                    }
                }
                // Following the ends leads to the border or a lake, round no circle.
                let mut reached = index;
                for _ in 0..=channels.len() {
                    match channels[reached].end {
                        ChannelEnd::Channel(joined) if joined < channels.len() => reached = joined,
                        _ => break,
                    }
                }
                if matches!(channels[reached].end, ChannelEnd::Channel(joined) if joined < channels.len()) {
                    failures.push(format!("{name}: following the ends goes round a circle"));
                }
            }
        }
        assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
    }

    #[test]
    fn a_course_at_the_border_ends_at_the_edge() {
        // The world's cells span -256 to 255 m at 512 m. 518 m is no multiple of the 8 m cells: they
        // span -259 to 258 m, and the last column of coarse cells inside lies at 253 m, 5 m inside the
        // east edge of that span, the last row as far inside the south one.
        for size_m in [512, 518] {
            let mut drawing = plain(size_m);
            let half_m = f64::from(drawing.size_m) / 2.0;
            drawing.rivers.push(River { points: vec![(4.0 - half_m, -half_m), (4.0 - half_m, half_m)], width_m: 8.0 });
            let land = grown(&drawing, 1);
            let channels = trace_channels(&land, &drawing, &WaterRules::default());
            let drawn: Vec<&Channel> = channels.iter().filter(|channel| channel.drawn == Some(0)).collect();
            assert!(!drawn.is_empty(), "{size_m} m: the river gives no channel: {channels:?}");
            for (index, channel) in channels.iter().enumerate() {
                let outside = channel.points.iter().find(|point| !inside_the_world(&drawing, point));
                assert!(outside.is_none(), "{size_m} m: channel {index} ({:?}) reaches {outside:?}, outside the world", channel.drawn);
            }
            for channel in drawn {
                let last = channel.points[channel.points.len() - 1];
                let from_edge_m = inside_the_edge_m(&drawing, &last);
                assert_eq!(channel.end, ChannelEnd::Edge, "{size_m} m: a channel of the river ends at {last:?}");
                assert!(from_edge_m <= 4.0, "{size_m} m: a channel of the river ends {from_edge_m} m inside the edge, at {last:?}");
            }
        }
    }

    #[test]
    fn a_high_threshold_leaves_only_the_drawn_rivers() {
        let usual = WaterRules::default();
        let high = WaterRules { brook_start_m2: 1e8, ..usual };
        for world in example_worlds() {
            let drawn: Vec<Channel> = traced(world, &usual).into_iter().filter(|channel| channel.drawn.is_some()).collect();
            let found = traced(world, &high);
            assert!(!drawn.is_empty(), "{}: no drawn channel", world.name);
            assert!(
                found == drawn,
                "{}: {} channels with a threshold of 1e8 m2, {} drawn ones with the usual threshold",
                world.name,
                found.len(),
                drawn.len()
            );
        }
    }

    /// Asserts that `first` and `second`, the channels of two drawings that differ only in the
    /// order of their rivers, are the same channels: river `k` of the first is river
    /// `moved_to[k]` of the second.
    fn assert_same_channels(name: &str, first: &[Channel], second: &[Channel], moved_to: &[usize]) {
        assert_eq!(first.len(), second.len(), "{name}: as many channels");
        // Where each channel is among the second ones: the first not yet taken with the same
        // points, of the same river under its index there.
        let mut taken = vec![false; second.len()];
        let place: Vec<usize> = (first.iter())
            .map(|channel| {
                let drawn = channel.drawn.map(|drawn| moved_to[drawn]);
                let found = (0..second.len())
                    .find(|&k| !taken[k] && second[k].drawn == drawn && second[k].points == channel.points)
                    .unwrap_or_else(|| panic!("{name}: no channel matches river {:?}'s ending {:?}", channel.drawn, channel.end));
                taken[found] = true;
                found
            })
            .collect();
        for (index, channel) in first.iter().enumerate() {
            let twin = &second[place[index]];
            let end = match channel.end {
                ChannelEnd::Channel(joined) => ChannelEnd::Channel(place[joined]),
                other => other,
            };
            assert_eq!((twin.from_lake, twin.end), (channel.from_lake, end), "{name}: channel {index} ({:?})", channel.drawn);
        }
    }

    /// The example in `country` with a third river, a brook from the high ground in the south-west
    /// into the lake (the drawing of `a_brook_into_a_lake_on_a_river_joins_it` in `grow/river.rs`).
    fn example_with_a_lake_brook(country: &str) -> Drawing {
        let mut drawing = Drawing { country: Country::named(country).expect("a known country"), ..Drawing::example_valley(2048) };
        drawing.rivers.push(River { points: vec![(-520.0, 200.0), (-330.0, 40.0), (-114.0, -99.0)], width_m: 4.0 });
        drawing
    }

    #[test]
    fn the_rivers_order_does_not_decide_the_channels() {
        let rules = WaterRules::default();
        // Drawings whose rivers are listed in another order, each with where its rivers moved:
        // each example world with its two rivers swapped, and the example with the lake brook in
        // each kind of country, the brook listed last and listed first.
        let reordered = |drawing: &Drawing, order: &[usize]| Drawing {
            rivers: order.iter().map(|&k| drawing.rivers[k].clone()).collect(),
            ..drawing.clone()
        };
        let mut pairs: Vec<(String, Drawing, Drawing, u64, Vec<usize>)> = (example_worlds().iter())
            .map(|world| (world.name.clone(), world.drawing.clone(), reordered(&world.drawing, &[1, 0]), world.seed, vec![1, 0]))
            .collect();
        for country in COUNTRY_NAMES {
            let drawing = example_with_a_lake_brook(country);
            let brook_first = reordered(&drawing, &[2, 0, 1]);
            pairs.push((format!("{country} with a lake brook, seed 1"), drawing, brook_first, 1, vec![1, 2, 0]));
        }
        let grown_pairs: Vec<(Grown, Grown)> = std::thread::scope(|scope| {
            let growing: Vec<_> = (pairs.iter())
                .map(|(_, first, second, seed, _)| scope.spawn(move || (grown(first, *seed), grown(second, *seed))))
                .collect();
            growing.into_iter().map(|pair| pair.join().expect("both drawings grow")).collect()
        });
        for ((name, first, second, _, moved_to), (first_land, second_land)) in pairs.iter().zip(&grown_pairs) {
            let channels = trace_channels(first_land, first, &rules);
            assert_same_channels(name, &channels, &trace_channels(second_land, second, &rules), moved_to);
            if first.rivers.len() == 3 {
                // The brook ends in the lake, and the big river still flows out of it to the edge.
                let ends = |drawn: usize| -> Vec<(Option<usize>, ChannelEnd)> {
                    channels.iter().filter(|channel| channel.drawn == Some(drawn)).map(|channel| (channel.from_lake, channel.end)).collect()
                };
                assert_eq!(ends(2).last(), Some(&(None, ChannelEnd::Lake(0))), "{name}: the lake brook's channels {:?}", ends(2));
                assert!(ends(0).contains(&(Some(0), ChannelEnd::Edge)), "{name}: the big river's channels {:?}", ends(0));
            }
        }
    }

    /// The side of a hand-made world, in metres.
    const HAND_MADE_M: u32 = 128;

    /// The index of the cell `(column, row)` from the centre of a hand-made world.
    fn hand_made_cell((column, row): (i32, i32)) -> usize {
        let side = HAND_MADE_M as usize / 8 + 64;
        let middle = (side / 2) as i32;
        (row + middle) as usize * side + (column + middle) as usize
    }

    /// A hand-made world of flat dry land, with no river and no lake, whose water runs only along
    /// `paths`, each its cells as `(column, row)` from the world's centre and the ground drained at
    /// the first, 100 m2 more at each next; each path's water runs on from its last cell as that
    /// cell's own path, if any, says.
    fn hand_made(paths: &[(&[(i32, i32)], f32)]) -> (Grown, Drawing) {
        let side = HAND_MADE_M as usize / 8 + 64;
        let grid = |value: f32| Grid { width: side, height: side, cell_size: 8.0, data: vec![value; side * side] };
        let mut drained_m2 = grid(64.0);
        let mut receivers: Vec<u32> = (0..side * side).map(|cell| cell as u32).collect();
        for &(path, first_m2) in paths {
            for (k, pair) in path.windows(2).enumerate() {
                drained_m2.data[hand_made_cell(pair[0])] = first_m2 + 100.0 * k as f32;
                receivers[hand_made_cell(pair[0])] = hand_made_cell(pair[1]) as u32;
            }
        }
        let land = Grown {
            heights: grid(0.0),
            drained_m2,
            water_depth_m: grid(0.0),
            receivers,
            rivers: Vec::new(),
            lakes: Vec::new(),
            notes: Vec::new(),
            notes_about: Vec::new(),
        };
        (land, plain(HAND_MADE_M))
    }

    #[test]
    fn a_channel_bending_round_a_lake_keeps_off_its_cells() {
        // A brook bends round two lake cells; smoothed freely, its line would cut across them.
        let path = [
            (0, 4), (1, 3), (2, 2), (3, 1), (3, 0), (2, -1), (1, -1), (0, -2), (-1, -3), (-2, -3), (-3, -4), (-4, -4),
            (-5, -3), (-6, -3), (-7, -3), (-8, -3), (-9, -3),
        ];
        let (mut land, mut drawing) = hand_made(&[(&path, 2_000.0)]);
        for cell in [(2, 0), (2, 1)] {
            land.water_depth_m.data[hand_made_cell(cell)] = 1.0;
        }
        land.lakes.push(GrownLake { drawn: 0, level_m: 1.0 });
        drawing.lakes.push(Lake { x_m: 16.0, z_m: 4.0, radius_m: 8.0 });
        let rules = WaterRules { brook_start_m2: 1_000.0, ..WaterRules::default() };
        let channels = trace_channels(&land, &drawing, &rules);
        let [brook] = &channels[..] else { panic!("one brook, not {channels:?}") };
        assert_eq!((brook.drawn, brook.from_lake, brook.end), (None, None, ChannelEnd::Edge));
        let lakes = lake_cells(&land, &drawing);
        let on_a_lake: Vec<&ChannelPoint> =
            brook.points.iter().filter(|&point| lakes[cell_at(&land, drawing.size_m, point)]).collect();
        assert!(on_a_lake.is_empty(), "points on the lake's cells: {on_a_lake:?}");
        // Away from the lake the line is still smoothed: some point lies off the line through the cells.
        let cells: Vec<(f64, f64)> = path[..path.len() - 1].iter().map(|&(i, j)| (f64::from(i) * 8.0, f64::from(j) * 8.0)).collect();
        let farthest_m = brook.points.iter().map(|point| distance_to_line_m(point, &cells)).fold(0.0, f64::max);
        assert!(farthest_m > 1.0, "the brook keeps within {farthest_m} m of the line through its cells");
        for pair in brook.points.windows(2) {
            assert!(apart_m(&pair[0], &pair[1]) <= 4.0 + 1e-9, "points {} m apart", apart_m(&pair[0], &pair[1]));
        }
    }

    #[test]
    fn a_brook_is_never_wider_than_what_it_joins() {
        // A drawn river 2 m wide crosses the world from west to east along row 0; a brook from the
        // north, draining enough to be 3.4 m wide by the rule, joins it at column 0.
        let river: Vec<(i32, i32)> = (-8..=9).map(|column| (column, 0)).collect();
        let brook = [(0, -4), (0, -3), (0, -2), (0, -1), (0, 0)];
        let (mut land, mut drawing) = hand_made(&[(&river, 3e6), (&brook, 2e6)]);
        let half_m = f64::from(HAND_MADE_M) / 2.0;
        let line = vec![(-half_m, 0.0), (half_m, 0.0)];
        land.rivers.push(GrownRiver { drawn: 0, line: line.clone(), width_m: 2.0, enters_at_edge: true, end: RiverEnd::Edge });
        drawing.rivers.push(River { points: line, width_m: 2.0 });
        let rules = WaterRules::default();
        let channels = trace_channels(&land, &drawing, &rules);
        let ends: Vec<(Option<usize>, ChannelEnd)> = channels.iter().map(|channel| (channel.drawn, channel.end)).collect();
        assert_eq!(ends, [(Some(0), ChannelEnd::Edge), (None, ChannelEnd::Channel(0))]);
        let found = &channels[1];
        for point in &found.points {
            assert!(brook_rule_m(point.drained_m2, &rules) > 3.0, "the rule gives {} m", brook_rule_m(point.drained_m2, &rules));
            assert_eq!(point.width_m, 2.0, "the brook at ({}, {})", point.x_m, point.z_m);
        }
        // The brook's last point, on the river, carries the brook's own water, not the river's.
        let last = found.points[found.points.len() - 1];
        assert_eq!((last.x_m, last.z_m, last.drained_m2), (0.0, 0.0, 2e6 + 300.0));
    }

    /// `path`'s cells as world points, its last left out.
    fn world_points(path: &[(i32, i32)]) -> Vec<(f64, f64)> {
        path[..path.len() - 1].iter().map(|&(column, row)| (f64::from(column) * 8.0, f64::from(row) * 8.0)).collect()
    }

    #[test]
    fn a_spring_narrower_than_a_brook_never_narrows() {
        // A spring 0.8 m wide, narrower than a brook starts (1.2 m), from inside the land to the east edge.
        let path: Vec<(i32, i32)> = [(-4, -4), (-3, -3), (-2, -2), (-1, -1)].into_iter().chain((0..=9).map(|column| (column, 0))).collect();
        let (mut land, mut drawing) = hand_made(&[(&path, 2_000.0)]);
        let line = world_points(&path);
        let ends = vec![line[0], line[line.len() - 1]];
        land.rivers.push(GrownRiver { drawn: 0, line: ends.clone(), width_m: 0.8, enters_at_edge: false, end: RiverEnd::Edge });
        drawing.rivers.push(River { points: ends, width_m: 0.8 });
        let rules = WaterRules::default();
        assert!(rules.brook_width_m > 0.8);
        let channels = trace_channels(&land, &drawing, &rules);
        let [spring] = &channels[..] else { panic!("one channel, not {channels:?}") };
        let widths: Vec<f32> = spring.points.iter().map(|point| point.width_m).collect();
        assert!(widths.windows(2).all(|pair| pair[1] >= pair[0]), "the spring narrows: {widths:?}");
        assert_eq!((widths[0], widths[widths.len() - 1]), (0.8, 0.8));
    }

    #[test]
    fn a_river_back_into_the_lake_it_left_gives_no_channel() {
        // A river crosses the world from west to east along row 0, through two lake cells, out for
        // two cells, back into the same lake for one, and on to the east edge. A brook from the
        // north drains into the second of the two cells between.
        let river: Vec<(i32, i32)> = (-8..=9).map(|column| (column, 0)).collect();
        let brook = [(0, -4), (0, -3), (0, -2), (0, -1), (0, 0)];
        let (mut land, mut drawing) = hand_made(&[(&river, 3e6), (&brook, 2e5)]);
        for cell in [(-3, 0), (-2, 0), (1, 0)] {
            land.water_depth_m.data[hand_made_cell(cell)] = 1.0;
        }
        land.lakes.push(GrownLake { drawn: 0, level_m: 1.0 });
        drawing.lakes.push(Lake { x_m: -8.0, z_m: 0.0, radius_m: 16.0 });
        let half_m = f64::from(HAND_MADE_M) / 2.0;
        let line = vec![(-half_m, 0.0), (half_m, 0.0)];
        land.rivers.push(GrownRiver { drawn: 0, line: line.clone(), width_m: 8.0, enters_at_edge: true, end: RiverEnd::Edge });
        drawing.rivers.push(River { points: line, width_m: 8.0 });
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        let ends: Vec<(Option<usize>, Option<usize>, ChannelEnd)> =
            channels.iter().map(|channel| (channel.drawn, channel.from_lake, channel.end)).collect();
        // The river ends in the lake and leaves it once, at its last cell; the brook ends in the lake.
        assert_eq!(
            ends,
            [(Some(0), None, ChannelEnd::Lake(0)), (Some(0), Some(0), ChannelEnd::Edge), (None, None, ChannelEnd::Lake(0))]
        );
        assert_eq!((channels[1].points[0].x_m, channels[1].points[0].z_m), (16.0, 0.0), "the river leaves the lake at column 2");
    }

    /// A hand-made world (`hand_made`) whose drawn river, `width_m` wide, comes in at the
    /// north-west corner and runs along the diagonal through the cells `(k, k)` to the south-east
    /// one, with the paths of `brooks`.
    fn on_the_diagonal(width_m: f32, brooks: &[(&[(i32, i32)], f32)]) -> (Grown, Drawing) {
        let river: Vec<(i32, i32)> = (-8..=9).map(|k| (k, k)).collect();
        let mut paths: Vec<(&[(i32, i32)], f32)> = vec![(&river, 3e6)];
        paths.extend_from_slice(brooks);
        let (mut land, mut drawing) = hand_made(&paths);
        let half_m = f64::from(HAND_MADE_M) / 2.0;
        let line = vec![(-half_m, -half_m), (half_m, half_m)];
        land.rivers.push(GrownRiver { drawn: 0, line: line.clone(), width_m, enters_at_edge: true, end: RiverEnd::Edge });
        drawing.rivers.push(River { points: line, width_m });
        (land, drawing)
    }

    /// Each channel's river, the lake it flows out of and its end.
    fn ends_of(channels: &[Channel]) -> Vec<(Option<usize>, Option<usize>, ChannelEnd)> {
        channels.iter().map(|channel| (channel.drawn, channel.from_lake, channel.end)).collect()
    }

    #[test]
    fn a_brook_inside_the_bed_it_joins_is_left_out() {
        // Brook B runs beside the river for two cells and joins it; brook C joins it from its one
        // cell, 3 points long. Each of their cells lies 5.7 m from the river's line: inside the bed
        // of a river 16 m wide, outside that of one 4 m wide. Brook X comes from 28 m away and
        // ends on B's second cell.
        let b = [(-4, -3), (-3, -2), (-2, -2)];
        let c = [(1, 0), (1, 1)];
        let x = [(-5, 0), (-4, -1), (-3, -2)];
        let brooks: [(&[(i32, i32)], f32); 3] = [(&b, 2e5), (&c, 1.5e5), (&x, 1e5)];
        let rules = WaterRules::default();
        let (none, edge, onto) = (None, ChannelEnd::Edge, ChannelEnd::Channel);

        // Beside a river 4 m wide all three are brooks of their own, X ending on B.
        let (land, drawing) = on_the_diagonal(4.0, &brooks);
        let narrow = trace_channels(&land, &drawing, &rules);
        assert_eq!(ends_of(&narrow), [(Some(0), none, edge), (none, none, onto(0)), (none, none, onto(0)), (none, none, onto(1))]);
        assert_eq!(narrow[2].points.len(), 3, "C's points: {:?}", narrow[2].points);

        // Beside a river 16 m wide B and C are left out, and X ends on the river instead, at one
        // of its points.
        let (land, drawing) = on_the_diagonal(16.0, &brooks);
        let wide = trace_channels(&land, &drawing, &rules);
        assert_eq!(ends_of(&wide), [(Some(0), none, edge), (none, none, onto(0))]);
        let (river, found) = (&wide[0], &wide[1]);
        let at = |point: &ChannelPoint| (point.x_m, point.z_m);
        assert_eq!(at(&found.points[0]), at(&narrow[3].points[0]), "the brook left is X");
        let last = found.points[found.points.len() - 1];
        assert!(river.points.iter().any(|point| at(point) == at(&last)), "X ends at {last:?}, on no point of the river");
    }

    #[test]
    fn a_brook_partly_inside_the_bed_it_joins_is_kept() {
        // The brook's first two cells lie 28 m and 17 m from the river's line, outside the bed of
        // a river 16 m wide; its last, 5.7 m from it, inside.
        let brook = [(-5, 0), (-4, -1), (-3, -2), (-2, -2)];
        let (land, drawing) = on_the_diagonal(16.0, &[(&brook, 1e5)]);
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        assert_eq!(ends_of(&channels), [(Some(0), None, ChannelEnd::Edge), (None, None, ChannelEnd::Channel(0))]);
        let (river, found) = (&channels[0], &channels[1]);
        let inside: Vec<bool> = found.points.iter().map(|point| inside_bed(&river.points, point)).collect();
        assert!(!inside[0] && inside[inside.len() - 1], "which of the brook's points lie inside the river's bed: {inside:?}");
    }

    /// A hand-made lake, drawn lake 0 at the centre, 32 m in radius, its level 1 m: the cells
    /// `cells` hold 1 m of water on ground at 0.
    fn with_a_lake(land: &mut Grown, drawing: &mut Drawing, cells: &[(i32, i32)]) {
        for &cell in cells {
            land.water_depth_m.data[hand_made_cell(cell)] = 1.0;
        }
        land.lakes.push(GrownLake { drawn: 0, level_m: 1.0 });
        drawing.lakes.push(Lake { x_m: 0.0, z_m: 0.0, radius_m: 32.0 });
    }

    /// Drawn river `drawn` laid along `line`, coming in at the edge, ending `end`.
    fn laid(land: &mut Grown, drawing: &mut Drawing, drawn: usize, line: Vec<(f64, f64)>, end: RiverEnd) {
        land.rivers.push(GrownRiver { drawn, line: line.clone(), width_m: 4.0, enters_at_edge: true, end });
        drawing.rivers.push(River { points: line, width_m: 4.0 });
    }

    #[test]
    fn a_river_into_a_lake_never_takes_its_outflow() {
        // River 2 crosses the world west to east along row 0, through the lake's cells (-1, 0) to
        // (1, 0), and drains it. River 1, drawn first, ends in the lake but never enters its cells:
        // it comes down column 3 from the north and meets row 0 at (3, 0), on river 2's way out.
        let through: Vec<(i32, i32)> = (-8..=9).map(|column| (column, 0)).collect();
        let beside: Vec<(i32, i32)> = (-8..=0).map(|row| (3, row)).collect();
        let (mut land, mut drawing) = hand_made(&[(&through, 3e6), (&beside, 2e5)]);
        with_a_lake(&mut land, &mut drawing, &[(-1, 0), (0, 0), (1, 0)]);
        let half_m = f64::from(HAND_MADE_M) / 2.0;
        laid(&mut land, &mut drawing, 0, vec![(24.0, -half_m), (24.0, -16.0), (8.0, 0.0)], RiverEnd::JoinsLake(0));
        laid(&mut land, &mut drawing, 1, vec![(-half_m, 0.0), (half_m, 0.0)], RiverEnd::Edge);
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        // River 2 flows into the lake and out of it to the edge; river 1 ends on its way out.
        assert_eq!(
            ends_of(&channels),
            [(Some(0), None, ChannelEnd::Channel(2)), (Some(1), None, ChannelEnd::Lake(0)), (Some(1), Some(0), ChannelEnd::Edge)]
        );
    }

    #[test]
    fn a_river_back_into_its_lake_on_an_earlier_path_gives_no_channel() {
        // The lake holds the cells (-2, 0) to (2, 0) and (0, -1). River 1 crosses the world west to
        // east along row 0, through the lake. River 2 comes down column 0 from the north into the
        // lake at (0, -1), leaves it through (1, -2), (2, -2) and (2, -1), and comes back into it at
        // (2, 0), a cell on river 1's path.
        let through: Vec<(i32, i32)> = (-8..=9).map(|column| (column, 0)).collect();
        let back: Vec<(i32, i32)> = (-8..=-1).map(|row| (0, row)).chain([(1, -2), (2, -2), (2, -1), (2, 0)]).collect();
        let (mut land, mut drawing) = hand_made(&[(&through, 3e6), (&back, 2e5)]);
        with_a_lake(&mut land, &mut drawing, &[(-2, 0), (-1, 0), (0, 0), (1, 0), (2, 0), (0, -1)]);
        let half_m = f64::from(HAND_MADE_M) / 2.0;
        laid(&mut land, &mut drawing, 0, vec![(-half_m, 0.0), (half_m, 0.0)], RiverEnd::Edge);
        laid(&mut land, &mut drawing, 1, vec![(0.0, -half_m), (16.0, -8.0)], RiverEnd::Edge);
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        // River 2 ends in the lake; its stretch out of it and back gives no channel.
        assert_eq!(
            ends_of(&channels),
            [(Some(0), None, ChannelEnd::Lake(0)), (Some(0), Some(0), ChannelEnd::Edge), (Some(1), None, ChannelEnd::Lake(0))]
        );
    }

    #[test]
    fn the_example_rivers_keep_to_their_lines_and_reach_their_ends() {
        let mut failures = Vec::new();
        for world in example_worlds() {
            let notes = line_notes(&traced(world, &WaterRules::default()), &world.land, &world.drawing);
            if !notes.is_empty() {
                failures.push(format!("{}: {notes:?}", world.name));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_river_that_leaves_its_line_is_said() {
        // Two rivers drawn crossing at the centre of the example's land. River 1 runs east; at the
        // crossing its water takes river 2's way south and leaves at the south edge, a kilometre
        // from its last drawn point. River 2 comes from the north and ends on river 1 there.
        let crossing = Drawing {
            rivers: vec![
                River { points: vec![(-1024.0, 0.0), (1024.0, 0.0)], width_m: 10.0 },
                River { points: vec![(0.0, -1024.0), (0.0, 1024.0)], width_m: 10.0 },
            ],
            lakes: Vec::new(),
            ..Drawing::example_valley(2048)
        };
        let land = grown(&crossing, 1);
        let channels = trace_channels(&land, &crossing, &WaterRules::default());
        let notes = line_notes(&channels, &land, &crossing);
        let [(leaves_about, leaves), rest @ ..] = &notes[..] else { panic!("{notes:?}") };
        let farthest_m: f64 = (leaves.strip_prefix("River 1 leaves its drawn line by up to "))
            .and_then(|rest| rest.strip_suffix(" m."))
            .and_then(|number| number.parse().ok())
            .unwrap_or_else(|| panic!("{leaves}"));
        assert!(farthest_m > 900.0, "{leaves}");
        assert_eq!(*leaves_about, About::River(0));
        assert_eq!(
            rest,
            [
                (About::River(0), "River 1 does not reach its drawn end; its water goes another way.".to_string()),
                (About::River(1), "River 2 does not reach its drawn end; its water goes another way.".to_string())
            ]
        );
    }

    #[test]
    fn the_way_the_engine_carried_a_river_on_is_not_said() {
        // A river drawn from the west edge to the world's centre, where it ends inside the land:
        // the engine carries it on east to the edge, and its channel follows, far beyond its
        // corridor from the line as drawn.
        let mut drawing = plain(2048);
        drawing.rivers.push(River { points: vec![(-1024.0, 300.0), (0.0, 0.0)], width_m: 8.0 });
        let land = grown(&drawing, 1);
        assert_eq!(land.rivers[0].end, RiverEnd::CarriedOn);
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        let line = &drawing.rivers[0].points;
        let farthest_m = (channels.iter().filter(|channel| channel.drawn == Some(0)))
            .flat_map(|channel| channel.points.iter().map(|point| distance_to_line_m(point, line)))
            .fold(0.0, f64::max);
        assert!(farthest_m > 500.0, "the channel keeps within {farthest_m} m of the line");
        assert!(line_notes(&channels, &land, &drawing).is_empty(), "{:?}", line_notes(&channels, &land, &drawing));
    }

    #[test]
    fn an_end_moved_farther_than_16_m_onto_what_it_joins_is_reported() {
        // Driven directly: the channel joined runs from (40, 0) to (60, 0); a channel whose last
        // corner before it is (0, 0) met it at the cell (8, 0), 32 m from its nearest point.
        let (land, drawing) = hand_made(&[]);
        let count = land.heights.data.len();
        let joined: Vec<ChannelPoint> =
            (10..=15).map(|k| ChannelPoint { x_m: 4.0 * f64::from(k), z_m: 0.0, width_m: 2.0, drained_m2: 0.0 }).collect();
        let tracing = Tracing {
            cells: Cells::new(&land, drawing.size_m),
            lake_of: vec![None; count],
            on_path: vec![false; count],
            course_of: vec![None; count],
            courses: Vec::new(),
            made: vec![Some(Made { points: joined, end: ChannelEnd::Edge, kept: true })],
        };
        let corner = |x_m: f64| Sample { at_m: (x_m, 0.0), along_m: x_m, drained_m2: 0.0 };
        let mut far = [corner(0.0), corner(8.0)];
        assert_eq!(tracing.snapped(&mut far, 0), Err(Unsnapped::Far { from_m: (8.0, 0.0), to_m: (40.0, 0.0), moved_m: 32.0 }));
        // It is moved there all the same, so the junction lies on a point of both.
        assert_eq!((far[1].at_m, far[1].along_m), ((40.0, 0.0), 40.0));
        // Met at (30, 0), 10 m from the same point: moved, and nothing to report.
        let mut near = [corner(24.0), corner(30.0)];
        assert_eq!(tracing.snapped(&mut near, 0), Ok(()));
        assert_eq!((near[1].at_m, near[1].along_m), ((40.0, 0.0), 40.0));
    }

    #[test]
    fn an_end_moved_far_from_inside_the_bed_it_joins_is_allowed() {
        // A river 120 m wide crosses the world from west to east along row 0. Brook X comes down
        // column 3 from row -6 into it at (3, 0): all of it lies within 48 m of the river's line,
        // inside its bed, so it is left out. Brook Y comes west along row -4 into X's cell (3, -4);
        // its end passes on to the river and is moved from that cell 32 m onto the river, to
        // (24, 0), from inside the river's bed. So drawn, Y too lies wholly inside the bed and is
        // left out; coming first down column 7 from the north border, 64 m from the river's line,
        // it is kept.
        let river: Vec<(i32, i32)> = (-8..=9).map(|column| (column, 0)).collect();
        let x: Vec<(i32, i32)> = (-6..=0).map(|row| (3, row)).collect();
        let along_row: Vec<(i32, i32)> = (3..=7).rev().map(|column| (column, -4)).collect();
        let from_the_border: Vec<(i32, i32)> = (-8..=-5).map(|row| (7, row)).chain(along_row.iter().copied()).collect();
        let at = |point: &ChannelPoint| (point.x_m, point.z_m);
        let first_m = |path: &[(i32, i32)]| (f64::from(path[0].0) * 8.0, f64::from(path[0].1) * 8.0);
        for (y, listed) in [(along_row, 0), (from_the_border, 1)] {
            let (mut land, mut drawing) = hand_made(&[(&river, 3e6), (&x, 2e5), (&y, 1e5)]);
            let half_m = f64::from(HAND_MADE_M) / 2.0;
            let line = vec![(-half_m, 0.0), (half_m, 0.0)];
            land.rivers.push(GrownRiver { drawn: 0, line: line.clone(), width_m: 120.0, enters_at_edge: true, end: RiverEnd::Edge });
            drawing.rivers.push(River { points: line, width_m: 120.0 });
            let channels = trace_channels(&land, &drawing, &WaterRules::default());
            let [river_channel, brooks @ ..] = &channels[..] else { panic!("no channel") };
            assert_eq!((river_channel.drawn, river_channel.end), (Some(0), ChannelEnd::Edge));
            assert_eq!(brooks.len(), listed, "Y starting at {:?}: {brooks:?}", first_m(&y));
            for brook in brooks {
                assert_eq!(at(&brook.points[0]), first_m(&y), "a brook other than Y is listed: {brook:?}");
                assert_eq!(brook.end, ChannelEnd::Channel(0));
                let last = brook.points[brook.points.len() - 1];
                assert!(river_channel.points.iter().any(|point| at(point) == at(&last)), "Y ends at {last:?}, on no point of the river");
            }
        }
    }

    #[test]
    fn a_pool_off_the_lakes_level_is_no_lake_cell() {
        // A lake 24 m in radius at the centre holds 1 m of water, up to its level of 1 m, on the
        // cells within one of the centre along both axes. A brook runs south down column 3 across
        // the world, through a pool 0.5 m deep at (3, 2), 29 m from the lake's centre and so within
        // its 1.5 radii; the pool's water stands at 0.5 m, not at the lake's level.
        let brook: Vec<(i32, i32)> = (-7..=9).map(|row| (3, row)).collect();
        let (mut land, mut drawing) = hand_made(&[(&brook, 2e5)]);
        for (column, row) in (-1..=1).flat_map(|column| (-1..=1).map(move |row| (column, row))) {
            land.water_depth_m.data[hand_made_cell((column, row))] = 1.0;
        }
        land.water_depth_m.data[hand_made_cell((3, 2))] = 0.5;
        land.lakes.push(GrownLake { drawn: 0, level_m: 1.0 });
        drawing.lakes.push(Lake { x_m: 0.0, z_m: 0.0, radius_m: 24.0 });
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        // One brook, from the north border to the south one; the pool does not cut it in two.
        assert_eq!(ends_of(&channels), [(None, None, ChannelEnd::Edge)]);
        assert!(!lake_cells(&land, &drawing)[hand_made_cell((3, 2))], "the pool is a lake cell");
    }

    #[test]
    fn a_brook_from_a_lake_back_into_it_is_left_out() {
        // A lake 24 m in radius at the centre holds 1 m of water on the cells within two of the
        // centre along both axes. Brook B leaves it at (2, 0) through the dry cells (3, 0) and
        // (3, 1) and comes back into it at (2, 1); brook Y ends on B's second cell; brook D leaves
        // it at (2, -2) for the east edge.
        let b = [(2, 0), (3, 0), (3, 1), (2, 1)];
        let y = [(5, 2), (4, 2), (3, 1)];
        let d: Vec<(i32, i32)> = (2..=9).map(|column| (column, -2)).collect();
        let (mut land, mut drawing) = hand_made(&[(&b, 2e5), (&y, 1e5), (&d, 3e5)]);
        for (column, row) in (-2..=2).flat_map(|column| (-2..=2).map(move |row| (column, row))) {
            land.water_depth_m.data[hand_made_cell((column, row))] = 1.0;
        }
        land.lakes.push(GrownLake { drawn: 0, level_m: 1.0 });
        drawing.lakes.push(Lake { x_m: 0.0, z_m: 0.0, radius_m: 24.0 });
        let channels = trace_channels(&land, &drawing, &WaterRules::default());
        // D flows out of the lake to the edge; B is left out, and Y, which ended on it, ends in
        // the lake instead.
        assert_eq!(ends_of(&channels), [(None, Some(0), ChannelEnd::Edge), (None, None, ChannelEnd::Lake(0))]);
    }
}
