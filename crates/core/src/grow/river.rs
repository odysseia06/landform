//! The drawn rivers, laid into the land model's rough shape before the first round: each line
//! smoothed, carried on to the grid's edge where it must be, and cut into the land as a trench.

use super::{smoothstep, About, GrownRiver, Layout, Note, RiverEnd, CELL_M};
use crate::drawing::{Drawing, Lake};
use crate::water_rules::WaterRules;

/// A river's line is followed in steps of half a cell: it is resampled at this spacing or less
/// before it is smoothed, and its trench is cut from samples this far apart, in metres.
const RIVER_STEP_M: f64 = CELL_M / 2.0;
/// A river's line is smoothed by this many passes of a mean over this many metres either way.
const SMOOTH_PASSES: usize = 3;
const SMOOTH_REACH_M: f64 = 32.0;
/// A river's end this close to the world's border, in metres, lies at the edge.
const EDGE_REACH_M: f64 = 8.0;
/// A river ending this close to another river's smoothed line, in metres, joins it.
const JOIN_REACH_M: f64 = 40.0;
/// A trench reaches this far from its line, in metres. Within `TRENCH_LIFT_FULL_M` the lift is cut
/// in full and within `TRENCH_SHAPE_FULL_M` the rough shape is, each fading out to the reach.
pub(crate) const TRENCH_REACH_M: f64 = 64.0;
const TRENCH_LIFT_FULL_M: f64 = 16.0;
const TRENCH_SHAPE_FULL_M: f64 = 8.0;
/// A trench's bed at the start of its line and at its end, in metres above the outlets.
const BED_START_M: f64 = 14.0;
const BED_END_M: f64 = 2.0;
/// A spring's trench reaches its full depth this share of the way along its line.
const SPRING_FULL_SHARE: f64 = 0.35;

/// Lays the drawn rivers into the land before the first round, following the toy's `Sim`. Every
/// line is smoothed first (`smoothed_line`); one with no length is left out, with a note in
/// `notes`; every note is about the river it names. A first point within `EDGE_REACH_M` of the world's border is carried outwards, away
/// from the next, to the grid's edge, and the river's water arrives at the cell it enters by:
/// `water.river_inflow_m2` times the square root of its width over `water.river_inflow_width_m`.
/// Then it is decided, whatever the rivers' order, which rivers join another and which are carried
/// on in their last direction to the grid's edge (`carried_on`), the cell each reaches being an
/// outlet. One carried on from inside a lake, so that the lake has a way out, or from elsewhere
/// inside the land, short of the world's edge, gets a note and ends `CarriedOn`; one carried on
/// from the world's edge ends there (`Edge`). Then the trenches are cut into `heights` and `lift_m`
/// (`Trench`), recorded in the drawing's order, so where two rivers' trenches overlap the order
/// still decides which of two equally near samples a cell keeps. Returns the outlets in index
/// order, none only when no river is laid, since the joins always lead to a river that is carried
/// on; and the rivers laid, in the drawing's order.
pub(super) fn lay_rivers(
    drawing: &Drawing,
    water: &WaterRules,
    layout: &Layout,
    heights: &mut [f64],
    lift_m: &mut [f64],
    incoming_m2: &mut [f64],
    notes: &mut Vec<Note>,
) -> (Vec<usize>, Vec<GrownRiver>) {
    let half_m = f64::from(drawing.size_m) / 2.0;
    let at_edge = |(x, z): (f64, f64)| half_m - x.abs().max(z.abs()) <= EDGE_REACH_M;
    let courses: Vec<Option<Course>> = (drawing.rivers.iter())
        .map(|river| {
            let mut line = smoothed_line(&river.points)?;
            let back = heading(line[0], line.iter())?;
            let on = heading(line[line.len() - 1], line.iter().rev())?;
            let (first, last) = (line[0], line[line.len() - 1]);
            let enters = at_edge(first);
            if enters {
                line.insert(0, layout.edge_along(first, back));
            }
            Some(Course { line, enters, last, carried_to: layout.edge_along(last, on) })
        })
        .collect();
    let leaving = carried_on(&courses, &drawing.lakes);
    let (inflow_m2, inflow_width_m) = (f64::from(water.river_inflow_m2), f64::from(water.river_inflow_width_m));
    let mut trench = vec![Trench::NONE; layout.cells()];
    let (mut outlets, mut laid) = (Vec::new(), Vec::new());
    for (index, ((river, course), leaves)) in drawing.rivers.iter().zip(courses).zip(leaving).enumerate() {
        let number = index + 1;
        // `carried_on` gives a way out to every river laid, and none to one left out.
        let (Some(course), Some(leaves)) = (course, leaves) else {
            notes.push((About::River(index), format!("River {number} has no length; it was left out.")));
            continue;
        };
        let drawn_line = course.smoothed().to_vec();
        let Course { mut line, enters, last, carried_to } = course;
        if enters {
            let relative_width = f64::from(river.width_m) / inflow_width_m;
            incoming_m2[layout.cell_at(line[0])] += inflow_m2 * relative_width.sqrt();
        }
        let end = match leaves {
            Leaves::ByRiver(other) => RiverEnd::Joins(other),
            Leaves::ByLake(lake) => RiverEnd::JoinsLake(lake),
            Leaves::CarriedOn => {
                let end = if drawing.lakes.iter().any(|lake| inside(lake, last)) {
                    notes.push((About::River(index), format!("River {number} ended in a lake; it was carried on to the edge so the lake has a way out.")));
                    RiverEnd::CarriedOn
                } else if !at_edge(last) {
                    notes.push((About::River(index), format!("River {number} ended inside the land; it was carried on to the edge.")));
                    RiverEnd::CarriedOn
                } else {
                    RiverEnd::Edge
                };
                line.push(carried_to);
                outlets.push(layout.cell_at(carried_to));
                end
            }
        };
        Trench::record(&mut trench, layout, &line, enters);
        laid.push(GrownRiver { drawn: index, line: drawn_line, width_m: river.width_m, enters_at_edge: enters, end });
    }
    for (cell, nearest) in trench.iter().enumerate() {
        nearest.cut(&mut heights[cell], &mut lift_m[cell]);
    }
    outlets.sort_unstable();
    outlets.dedup();
    (outlets, laid)
}

/// A drawn river's way over the land before its end is decided.
struct Course {
    /// The smoothed line, after the stretch from the grid's edge when the river enters there.
    line: Vec<(f64, f64)>,
    /// Whether the river enters at the world's edge, so `line` begins at the grid's edge.
    enters: bool,
    /// The smoothed line's last point.
    last: (f64, f64),
    /// Where the river reaches the grid's edge if it is carried on from `last` in its last direction.
    carried_to: (f64, f64),
}

impl Course {
    /// The smoothed line as drawn, without the stretch from the grid's edge.
    fn smoothed(&self) -> &[(f64, f64)] {
        &self.line[usize::from(self.enters)..]
    }

    /// Whether the river gives `lake` a way out: its line, carried on when `carried` is set, leaves
    /// the lake's circle (it crosses it, begins inside it, or is carried on across it), or it is
    /// carried on from inside the circle.
    fn gives_way_out(&self, carried: bool, lake: &Lake) -> bool {
        let centre = (lake.x_m, lake.z_m);
        let leaves = |from: (f64, f64), to: (f64, f64)| {
            !inside(lake, to) && distance_to_line(centre, &[from, to]) < f64::from(lake.radius_m)
        };
        let carried_out = carried && (inside(lake, self.last) || leaves(self.last, self.carried_to));
        carried_out || self.line.windows(2).any(|pair| leaves(pair[0], pair[1]))
    }
}

/// Whether `point` lies inside `lake`'s circle.
fn inside(lake: &Lake, (x, z): (f64, f64)) -> bool {
    (x - lake.x_m).hypot(z - lake.z_m) < f64::from(lake.radius_m)
}

/// How a laid river's water leaves the land, as `carried_on` decides it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Leaves {
    /// Carried on in its last direction to the grid's edge, where the cell it reaches is an outlet.
    CarriedOn,
    /// Joining the river of this index among the courses.
    ByRiver(usize),
    /// Joining the lake of this index, which another river gives a way out.
    ByLake(usize),
}

/// How each river of `courses` leaves the land, in their order: carried on to the grid's edge, or
/// joining the rivers it ends on; `None` for one left out (`None`), and only for one left out. The
/// order of `courses` does not decide which rivers are carried on, except where several rivers
/// could each be the one carried on.
///
/// A river whose last point lies within `JOIN_REACH_M` of another river's smoothed line joins
/// every such river. One that ends near no other line but inside a lake joins every other river
/// that gives one of the lakes it ends in a way out (`Course::gives_way_out`). Any other is carried
/// on. Then, while some joining rivers lead, through the rivers they join, to no river that is
/// carried on, the first of them in the drawing's order that joins nothing, or that lies on a
/// circle of rivers each joining the next, is carried on, and the joins are decided again. So of
/// two rivers that end in a lake with no other way out the first is carried on and the second
/// joins it, and of two that end on each other the first is carried on.
///
/// A joining river is said to join one river (`Leaves::ByRiver`): of those it joins, the one whose
/// water reaches a river carried on in the fewest joins, and of those the one whose line passes
/// nearest its end, the first in the drawing's order on a tie. So following the rivers named never
/// leads round a circle. One that joins through the lakes it ends in is chosen the same way, and is
/// said to join the first of those lakes that the chosen river gives a way out (`Leaves::ByLake`).
fn carried_on(courses: &[Option<Course>], lakes: &[Lake]) -> Vec<Option<Leaves>> {
    let count = courses.len();
    let laid: Vec<(usize, &Course)> = (courses.iter().enumerate()).filter_map(|(i, course)| Some((i, course.as_ref()?))).collect();
    // For each river, the others whose smoothed line passes near its end, and the lakes it ends in.
    let mut near = vec![Vec::new(); count];
    let mut ends_in: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(i, course) in &laid {
        near[i] = (laid.iter())
            .filter(|&&(j, other)| j != i && distance_to_line(course.last, other.smoothed()) <= JOIN_REACH_M)
            .map(|&(j, _)| j)
            .collect();
        ends_in[i] = (0..lakes.len()).filter(|&k| inside(&lakes[k], course.last)).collect();
    }
    let mut carried: Vec<bool> = (0..count).map(|i| courses[i].is_some() && near[i].is_empty() && ends_in[i].is_empty()).collect();
    loop {
        let mut joined = vec![Vec::new(); count];
        for &(i, _) in laid.iter().filter(|&&(i, _)| !carried[i]) {
            joined[i] = if near[i].is_empty() {
                (laid.iter())
                    .filter(|&&(j, other)| j != i && ends_in[i].iter().any(|&k| other.gives_way_out(carried[j], &lakes[k])))
                    .map(|&(j, _)| j)
                    .collect()
            } else {
                near[i].clone()
            };
        }
        // How many joins each river's water takes to reach a river carried on, through the rivers
        // it joins: 0 for one carried on, `None` for one whose water reaches none.
        let mut joins_out: Vec<Option<usize>> = carried.iter().map(|&carried| carried.then_some(0)).collect();
        for joins in 0.. {
            let next: Vec<usize> = (laid.iter())
                .map(|&(i, _)| i)
                .filter(|&i| joins_out[i].is_none() && joined[i].iter().any(|&j| joins_out[j] == Some(joins)))
                .collect();
            if next.is_empty() {
                break;
            }
            for i in next {
                joins_out[i] = Some(joins + 1);
            }
        }
        let stranded = (laid.iter())
            .map(|&(i, _)| i)
            .find(|&i| joins_out[i].is_none() && (joined[i].is_empty() || on_a_circle(i, &joined)));
        if let Some(i) = stranded {
            carried[i] = true;
            continue;
        }
        // Every river laid now reaches a river carried on: following the joins from one that did
        // not would end at a river that joins nothing or come round a circle, and the first such
        // river would have been stranded.
        return (0..count)
            .map(|i| {
                let course = courses[i].as_ref()?;
                if carried[i] {
                    return Some(Leaves::CarriedOn);
                }
                let (river, ..) = (joined[i].iter())
                    .filter_map(|&j| Some((j, joins_out[j]?, distance_to_line(course.last, courses[j].as_ref()?.smoothed()))))
                    .min_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)))?;
                if near[i].is_empty() {
                    let other = courses[river].as_ref()?;
                    ends_in[i].iter().copied().find(|&k| other.gives_way_out(carried[river], &lakes[k])).map(Leaves::ByLake)
                } else {
                    Some(Leaves::ByRiver(river))
                }
            })
            .collect();
    }
}

/// Whether following `joined`, the rivers each river joins, from `river` leads back to it.
fn on_a_circle(river: usize, joined: &[Vec<usize>]) -> bool {
    let mut seen = vec![false; joined.len()];
    let mut waiting = joined[river].clone();
    while let Some(next) = waiting.pop() {
        if next == river {
            return true;
        }
        if !std::mem::replace(&mut seen[next], true) {
            waiting.extend(&joined[next]);
        }
    }
    false
}

/// The sample of a river's line nearest a cell, within `TRENCH_REACH_M`, and the trench it cuts.
#[derive(Debug, Clone, Copy)]
struct Trench {
    /// From the cell to the sample, in metres.
    distance_m: f64,
    /// The bed's height at the sample, in metres.
    bed_m: f64,
    /// How fully the trench is cut there, from 0 to 1.
    strength: f64,
}

impl Trench {
    /// No river near the cell.
    const NONE: Self = Self { distance_m: f64::INFINITY, bed_m: 0.0, strength: 0.0 };

    /// Samples `line` every `RIVER_STEP_M` and keeps, for each cell within `TRENCH_REACH_M` of a
    /// sample, the nearest sample of any line so far (the first on a tie). The bed falls from
    /// `BED_START_M` at the line's start to `BED_END_M` at its end, by distance along the line. The
    /// strength is 1 for a river that `enters` at the edge; a spring's rises from 0 at its first
    /// point to 1 at `SPRING_FULL_SHARE` of its length (a smooth step).
    fn record(trench: &mut [Trench], layout: &Layout, line: &[(f64, f64)], enters: bool) {
        let length_m: f64 = line.windows(2).map(|pair| distance_m(pair[0], pair[1])).sum();
        let mut sample = |(x, z): (f64, f64), behind_m: f64| {
            let share = behind_m / length_m;
            let bed_m = BED_END_M + (BED_START_M - BED_END_M) * (1.0 - share);
            let strength = if enters { 1.0 } else { smoothstep(0.0, SPRING_FULL_SHARE, share) };
            for row in layout.indices_near(z, TRENCH_REACH_M) {
                for column in layout.indices_near(x, TRENCH_REACH_M) {
                    let distance_m = (layout.position_m(column) - x).hypot(layout.position_m(row) - z);
                    let nearest = &mut trench[row * layout.side + column];
                    if distance_m < TRENCH_REACH_M && distance_m < nearest.distance_m {
                        *nearest = Trench { distance_m, bed_m, strength };
                    }
                }
            }
        };
        let mut behind_m = 0.0;
        for pair in line.windows(2) {
            let segment_m = distance_m(pair[0], pair[1]);
            let steps = ((segment_m / RIVER_STEP_M).ceil() as usize).max(1);
            for step in 0..steps {
                let along = step as f64 / steps as f64;
                sample(between(pair[0], pair[1], along), behind_m + segment_m * along);
            }
            behind_m += segment_m;
        }
        sample(line[line.len() - 1], length_m);
    }

    /// Cuts the trench into a cell of height `height_m` rising by `lift_m` each round: the lift is
    /// multiplied by `1 - (1 - smoothstep(TRENCH_LIFT_FULL_M, TRENCH_REACH_M, d)) * strength`, and
    /// the height becomes `min(h, bed + (h - bed) * w)` with
    /// `w = 1 - (1 - smoothstep(TRENCH_SHAPE_FULL_M, TRENCH_REACH_M, d)) * strength`.
    fn cut(&self, height_m: &mut f64, lift_m: &mut f64) {
        if self.distance_m >= TRENCH_REACH_M {
            return;
        }
        let (d, strength) = (self.distance_m, self.strength);
        *lift_m *= 1.0 - (1.0 - smoothstep(TRENCH_LIFT_FULL_M, TRENCH_REACH_M, d)) * strength;
        let kept = 1.0 - (1.0 - smoothstep(TRENCH_SHAPE_FULL_M, TRENCH_REACH_M, d)) * strength;
        *height_m = height_m.min(self.bed_m + (*height_m - self.bed_m) * kept);
    }
}

/// `points` resampled every `RIVER_STEP_M` or less and smoothed by `SMOOTH_PASSES` passes of a
/// mean over `SMOOTH_REACH_M` either way along the line, or over the whole line when it is
/// shorter. Beyond its ends the line is mirrored through them, so each end stays where it was
/// drawn and the line still leaves it in the direction it heads there. `None` when the points are
/// all the same.
pub(crate) fn smoothed_line(points: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    let lengths_m: Vec<f64> = points.windows(2).map(|pair| distance_m(pair[0], pair[1])).collect();
    let length_m: f64 = lengths_m.iter().sum();
    if length_m <= 0.0 {
        return None;
    }
    let steps = ((length_m / RIVER_STEP_M).ceil() as usize).max(1);
    let spacing_m = length_m / steps as f64;
    let mut line = Vec::with_capacity(steps + 1);
    let (mut segment, mut behind_m) = (0, 0.0);
    for step in 0..steps {
        let along_m = step as f64 * spacing_m;
        while along_m > behind_m + lengths_m[segment] && segment + 1 < lengths_m.len() {
            behind_m += lengths_m[segment];
            segment += 1;
        }
        let into = if lengths_m[segment] > 0.0 { ((along_m - behind_m) / lengths_m[segment]).min(1.0) } else { 0.0 };
        line.push(between(points[segment], points[segment + 1], into));
    }
    line.push(points[points.len() - 1]);

    // Point `k` of the line, mirrored through the first point before it and the last after it.
    let mirrored = |line: &[(f64, f64)], k: isize| -> (f64, f64) {
        let through = |end: (f64, f64), (x, z): (f64, f64)| (2.0 * end.0 - x, 2.0 * end.1 - z);
        match usize::try_from(k) {
            Err(_) => through(line[0], line[k.unsigned_abs()]),
            Ok(k) if k > steps => through(line[steps], line[2 * steps - k]),
            Ok(k) => line[k],
        }
    };
    let reach = ((SMOOTH_REACH_M / spacing_m).round() as usize).min(steps) as isize;
    let width = (2 * reach + 1) as f64;
    let mut spare = line.clone();
    for _ in 0..SMOOTH_PASSES {
        for (index, smoothed) in spare.iter_mut().enumerate().take(steps).skip(1) {
            let index = index as isize;
            let (sum_x, sum_z) = (index - reach..=index + reach)
                .map(|k| mirrored(&line, k))
                .fold((0.0, 0.0), |(sum_x, sum_z), (x, z)| (sum_x + x, sum_z + z));
            *smoothed = (sum_x / width, sum_z / width);
        }
        std::mem::swap(&mut line, &mut spare);
    }
    Some(line)
}

/// The unit direction to `end` from the first of `others` that is not `end`: the way a line heads
/// as it reaches `end`. `None` when all of them are `end`.
fn heading<'a>(end: (f64, f64), mut others: impl Iterator<Item = &'a (f64, f64)>) -> Option<(f64, f64)> {
    let &from = others.find(|&&point| point != end)?;
    let length_m = distance_m(from, end);
    Some(((end.0 - from.0) / length_m, (end.1 - from.1) / length_m))
}

/// The distance from `point` to the nearest point of the line through `line`, in metres.
fn distance_to_line(point: (f64, f64), line: &[(f64, f64)]) -> f64 {
    line.windows(2)
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let squared = dx * dx + dz * dz;
            let along = if squared > 0.0 { (((point.0 - a.0) * dx + (point.1 - a.1) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
            distance_m(point, between(a, b, along))
        })
        .fold(f64::INFINITY, f64::min)
}

fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The point `along` of the way from `a` to `b`.
fn between(a: (f64, f64), b: (f64, f64), along: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * along, a.1 + (b.1 - a.1) * along)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{example_picture, example_worlds, grown, largest_drain, plain, position_m, ExampleWorld};
    use super::super::{split_notes, Grown};
    use super::*;
    use crate::drawing::River;

    /// The water a river `width_m` wide brings from beyond the world under the default rules, in
    /// square metres of ground drained.
    fn incoming_m2(width_m: f32) -> f64 {
        let water = WaterRules::default();
        f64::from(water.river_inflow_m2) * (f64::from(width_m) / f64::from(water.river_inflow_width_m)).sqrt()
    }

    /// Points every `step_m` along the line through `points` from its first point, each with the
    /// unit direction of its segment.
    fn along(points: &[(f64, f64)], step_m: f64) -> Vec<((f64, f64), (f64, f64))> {
        let mut samples = Vec::new();
        let mut into_m = 0.0;
        for pair in points.windows(2) {
            let ((ax, az), (bx, bz)) = (pair[0], pair[1]);
            let length_m = (bx - ax).hypot(bz - az);
            if length_m == 0.0 {
                continue;
            }
            let direction = ((bx - ax) / length_m, (bz - az) / length_m);
            while into_m <= length_m {
                samples.push(((ax + direction.0 * into_m, az + direction.1 * into_m), direction));
                into_m += step_m;
            }
            into_m -= length_m;
        }
        samples
    }

    /// The height at world `(x, z)`, bilinear between the four cells around it.
    fn height_at(land: &Grown, size_m: u32, (x, z): (f64, f64)) -> f64 {
        let origin_m = position_m(size_m, 0);
        let (u, v) = ((x - origin_m) / 8.0, (z - origin_m) / 8.0);
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (across, down) = (u - i as f64, v - j as f64);
        let h = |i: usize, j: usize| f64::from(land.heights.get(i, j));
        let upper = h(i, j) + across * (h(i + 1, j) - h(i, j));
        let lower = h(i, j + 1) + across * (h(i + 1, j + 1) - h(i, j + 1));
        upper + down * (lower - upper)
    }

    /// Whether some cell within `reach_m` of world `(x, z)` drains at least `area_m2`.
    fn drains_near(land: &Grown, size_m: u32, (x, z): (f64, f64), reach_m: f64, area_m2: f64) -> bool {
        let side = land.drained_m2.width;
        let span = |at: f64| {
            let first = ((at - reach_m - position_m(size_m, 0)) / 8.0).floor().max(0.0) as usize;
            first..=(first + (2.0 * reach_m / 8.0).ceil() as usize + 1).min(side - 1)
        };
        span(z).any(|j| {
            span(x).any(|i| {
                let near = (position_m(size_m, i) - x).hypot(position_m(size_m, j) - z) <= reach_m;
                near && f64::from(land.drained_m2.get(i, j)) >= area_m2
            })
        })
    }

    #[test]
    fn a_drawn_river_carries_its_water_along_its_line() {
        for ExampleWorld { name, drawing, land, .. } in example_worlds() {
            // In a lake the water crosses still water and need not keep to the line.
            let near_a_lake = |(x, z): (f64, f64)| {
                drawing.lakes.iter().any(|lake| (x - lake.x_m).hypot(z - lake.z_m) <= 1.5 * f64::from(lake.radius_m))
            };
            let points: Vec<(f64, f64)> =
                along(&drawing.rivers[0].points, 8.0).into_iter().map(|(point, _)| point).filter(|&point| !near_a_lake(point)).collect();
            assert!(!points.is_empty(), "{name}: every point of the river's line lies near a lake");
            let inflow_m2 = f64::from(WaterRules::default().river_inflow_m2);
            let dry: Vec<(f64, f64)> = points
                .iter()
                .copied()
                .filter(|&point| !drains_near(land, drawing.size_m, point, 48.0, inflow_m2))
                .collect();
            assert!(
                dry.is_empty(),
                "{name}: at {} of the river's {} points away from a lake no cell within 48 m drains {inflow_m2} m2, such as {:?}",
                dry.len(),
                points.len(),
                dry.first()
            );
        }
    }

    #[test]
    fn a_spring_cuts_a_valley_to_the_river() {
        for ExampleWorld { name, drawing, land, .. } in example_worlds() {
            let size_m = drawing.size_m;
            let line = &drawing.rivers[1].points;
            let (top, bottom) = (height_at(land, size_m, line[0]), height_at(land, size_m, line[line.len() - 1]));
            assert!(bottom < top, "{name}: the spring's last point is at {bottom} m, its first at {top} m");

            let points = along(line, 8.0);
            let mean = |offset_m: f64| {
                let sum: f64 = points
                    .iter()
                    .map(|&((x, z), (dx, dz))| height_at(land, size_m, (x - dz * offset_m, z + dx * offset_m)))
                    .sum();
                sum / points.len() as f64
            };
            let (on, beside) = (mean(0.0), (mean(80.0) + mean(-80.0)) / 2.0);
            assert!(on < beside, "{name}: the spring's line averages {on} m, 80 m to either side {beside} m");
        }
    }

    #[test]
    fn all_water_leaves_at_the_rivers_outlet() {
        let example = Drawing::example_valley(2048);
        let ExampleWorld { drawing, land, .. } = example_worlds()
            .iter()
            .find(|world| world.drawing == example && world.seed == 1)
            .expect("the example in hill country from seed 1");
        let side = land.drained_m2.width;
        // Only the first river enters at the edge; the second is a spring that joins it.
        let incoming = incoming_m2(drawing.rivers[0].width_m);
        let whole = (side * side) as f64 * 64.0 + incoming;
        let ((column, row), drained) = largest_drain(land);
        assert_eq!(column, side - 1, "the outlet at ({column}, {row}) is not on the east edge");
        let error = (drained - whole).abs() / whole;
        assert!(error < 1e-3, "the outlet drains {drained} m2 of the {whole} m2 the grid and the river bring: relative error {error}");
    }

    #[test]
    fn a_river_ending_inside_the_land_is_carried_on_and_noted() {
        let mut drawing = plain(2048);
        drawing.rivers.push(River { points: vec![(-500.0, 0.0), (0.0, 0.0)], width_m: 8.0 });
        let land = grown(&drawing, 1);
        assert_eq!(land.notes, ["River 1 ended inside the land; it was carried on to the edge."]);
        assert_eq!(land.notes_about, [About::River(0)]);
        assert_eq!(ends(&land), [RiverEnd::CarriedOn]);
        let side = land.drained_m2.width;
        let ((column, row), drained) = largest_drain(&land);
        assert_eq!(column, side - 1, "the outlet at ({column}, {row}) is not on the east edge");
        let whole = (side * side) as f64 * 64.0;
        let error = (drained - whole).abs() / whole;
        assert!(error < 1e-3, "the outlet drains {drained} m2 of the grid's {whole} m2: relative error {error}");
    }

    /// The outlets, the notes and the rivers' ends `lay_rivers` gives `drawing` on flat ground.
    fn laid(drawing: &Drawing) -> (Vec<usize>, Vec<Note>, Vec<RiverEnd>) {
        let layout = Layout::new(drawing.size_m);
        let cells = layout.cells();
        let (mut heights, mut lift_m, mut incoming_m2, mut notes) = (vec![0.0; cells], vec![0.0; cells], vec![0.0; cells], Vec::new());
        let water = WaterRules::default();
        let (outlets, rivers) = lay_rivers(drawing, &water, &layout, &mut heights, &mut lift_m, &mut incoming_m2, &mut notes);
        (outlets, notes, rivers.iter().map(|river| river.end).collect())
    }

    /// The ends of the rivers of `land`.
    fn ends(land: &Grown) -> Vec<RiverEnd> {
        land.rivers.iter().map(|river| river.end).collect()
    }

    /// The distance from the last point of river `from` to the smoothed line of river `to`, in metres.
    fn end_to_line_m(drawing: &Drawing, from: usize, to: usize) -> f64 {
        let end = *drawing.rivers[from].points.last().expect("a point");
        distance_to_line(end, &smoothed_line(&drawing.rivers[to].points).expect("a line with length"))
    }

    /// Asserts that all the water of `land` leaves at its largest drain: the grid's own ground and
    /// what the rivers of `drawing` that begin at the world's edge bring.
    fn assert_all_water_leaves_at_one_cell(name: &str, drawing: &Drawing, land: &Grown) {
        let half_m = f64::from(drawing.size_m) / 2.0;
        let incoming: f64 = (drawing.rivers.iter())
            .filter(|river| half_m - river.points[0].0.abs().max(river.points[0].1.abs()) <= EDGE_REACH_M)
            .map(|river| incoming_m2(river.width_m))
            .sum();
        let side = land.drained_m2.width;
        let whole = (side * side) as f64 * 64.0 + incoming;
        let (at, drained) = largest_drain(land);
        let error = (drained - whole).abs() / whole;
        assert!(error < 1e-3, "{name}: the cell at {at:?} drains {drained} m2 of the {whole} m2 the grid and the rivers bring: relative error {error}");
    }

    #[test]
    fn a_brook_into_a_lake_on_a_river_joins_it() {
        let mut drawing = Drawing::example_valley(2048);
        let lake = drawing.lakes[0];
        // From the high ground in the south-west into the lake, on the far side from the spring.
        drawing.rivers.push(River { points: vec![(-520.0, 200.0), (-330.0, 40.0), (-114.0, -99.0)], width_m: 4.0 });
        let end = drawing.rivers[2].points[2];
        assert!((end.0 - lake.x_m).hypot(end.1 - lake.z_m) < f64::from(lake.radius_m), "the brook ends outside the lake");
        for other in [0, 1] {
            let apart_m = end_to_line_m(&drawing, 2, other);
            assert!(apart_m > JOIN_REACH_M, "the brook ends {apart_m} m from river {}'s line", other + 1);
        }
        let (outlets, notes, laid_ends) = laid(&drawing);
        assert_eq!(outlets.len(), 1, "{outlets:?}");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(laid_ends, [RiverEnd::Edge, RiverEnd::Joins(0), RiverEnd::JoinsLake(0)]);
        let land = grown(&drawing, 1);
        assert!(land.notes.is_empty(), "{:?}", land.notes);
        assert_eq!(ends(&land), laid_ends);
        assert_all_water_leaves_at_one_cell("the example and a brook", &drawing, &land);
    }

    #[test]
    fn of_two_rivers_ending_in_a_closed_lake_the_first_is_carried_on() {
        let mut drawing = plain(2048);
        drawing.lakes.push(Lake { x_m: 0.0, z_m: 0.0, radius_m: 112.0 });
        drawing.rivers.push(River { points: vec![(-1024.0, 0.0), (40.0, 0.0)], width_m: 16.0 });
        drawing.rivers.push(River { points: vec![(0.0, -1024.0), (0.0, -60.0)], width_m: 8.0 });
        for (from, to) in [(0, 1), (1, 0)] {
            let apart_m = end_to_line_m(&drawing, from, to);
            assert!(apart_m > JOIN_REACH_M, "river {} ends {apart_m} m from river {}'s line", from + 1, to + 1);
        }
        let (outlets, notes, laid_ends) = laid(&drawing);
        assert_eq!(outlets.len(), 1, "{outlets:?}");
        assert_eq!(notes, [(About::River(0), "River 1 ended in a lake; it was carried on to the edge so the lake has a way out.".to_string())]);
        assert_eq!(laid_ends, [RiverEnd::CarriedOn, RiverEnd::JoinsLake(0)]);
        let land = grown(&drawing, 1);
        assert_eq!((land.notes.clone(), land.notes_about.clone()), split_notes(notes));
        assert_eq!(ends(&land), laid_ends);
        assert_all_water_leaves_at_one_cell("two rivers into one lake", &drawing, &land);
    }

    #[test]
    fn the_rivers_order_does_not_decide_where_they_join() {
        let example = Drawing::example_valley(2048);
        let (outlets, notes, example_ends) = laid(&example);
        assert_eq!(outlets.len(), 1, "{outlets:?}");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(example_ends, [RiverEnd::Edge, RiverEnd::Joins(0)]);
        let mut swapped = example.clone();
        swapped.rivers.reverse();
        let (swapped_outlets, swapped_notes, swapped_ends) = laid(&swapped);
        assert_eq!(swapped_outlets, outlets, "the spring listed first");
        assert!(swapped_notes.is_empty(), "the spring listed first: {swapped_notes:?}");
        assert_eq!(swapped_ends, [RiverEnd::Joins(1), RiverEnd::Edge], "the spring listed first");
        let land = grown(&swapped, 1);
        assert!(land.notes.is_empty(), "{:?}", land.notes);
        assert_all_water_leaves_at_one_cell("the example's rivers swapped", &swapped, &land);
    }

    #[test]
    fn rivers_that_end_on_each_other_still_give_land() {
        let mut drawing = plain(1024);
        drawing.rivers.push(River { points: vec![(-300.0, 0.0), (0.0, 0.0)], width_m: 8.0 });
        drawing.rivers.push(River { points: vec![(20.0, 300.0), (20.0, -300.0), (-150.0, -20.0)], width_m: 8.0 });
        for (from, to) in [(0, 1), (1, 0)] {
            let apart_m = end_to_line_m(&drawing, from, to);
            assert!(apart_m <= JOIN_REACH_M, "river {} ends {apart_m} m from river {}'s line", from + 1, to + 1);
        }
        // The first is treated as not joining, and is carried on from inside the land.
        let (outlets, notes, laid_ends) = laid(&drawing);
        assert_eq!(outlets.len(), 1, "{outlets:?}");
        assert_eq!(notes, [(About::River(0), "River 1 ended inside the land; it was carried on to the edge.".to_string())]);
        assert_eq!(laid_ends, [RiverEnd::CarriedOn, RiverEnd::Joins(0)]);
        let land = grown(&drawing, 1);
        assert_eq!((land.notes.clone(), land.notes_about.clone()), split_notes(notes));
        for (grid_name, grid) in [("heights", &land.heights), ("drained_m2", &land.drained_m2), ("water_depth_m", &land.water_depth_m)] {
            let bad = grid.data.iter().filter(|value| !value.is_finite()).count();
            assert_eq!(bad, 0, "{bad} of the {grid_name} are not finite");
        }
        assert_all_water_leaves_at_one_cell("two rivers ending on each other", &drawing, &land);
        assert_eq!(ends(&land), laid_ends);
    }

    #[test]
    fn a_river_is_said_to_join_the_one_that_leads_out_in_the_fewest_joins() {
        let mut drawing = plain(1024);
        // Along z = -45 from the west edge, ending 25 m past the second river's line.
        drawing.rivers.push(River { points: vec![(-512.0, -45.0), (25.0, -45.0)], width_m: 8.0 });
        // Down x = 0 from the north edge, ending 15 m from the first river's line and 30 m from the third's.
        drawing.rivers.push(River { points: vec![(0.0, -512.0), (0.0, -30.0)], width_m: 8.0 });
        // Along z = 0 from edge to edge.
        drawing.rivers.push(River { points: vec![(-512.0, 0.0), (512.0, 0.0)], width_m: 8.0 });
        let apart_m = |from: usize, to: usize| end_to_line_m(&drawing, from, to);
        assert!(apart_m(0, 1) <= JOIN_REACH_M && apart_m(0, 2) > JOIN_REACH_M, "{} {}", apart_m(0, 1), apart_m(0, 2));
        assert!(apart_m(1, 0) < apart_m(1, 2) && apart_m(1, 2) <= JOIN_REACH_M, "{} {}", apart_m(1, 0), apart_m(1, 2));
        // The second river joins the first and the third; the first's line is the nearer, but the
        // first joins the second, so naming it would lead round a circle.
        let (outlets, notes, laid_ends) = laid(&drawing);
        assert_eq!(outlets.len(), 1, "{outlets:?}");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(laid_ends, [RiverEnd::Joins(1), RiverEnd::Joins(2), RiverEnd::Edge]);
    }

    /// A right-angled corner drawn with 200 m arms becomes a curve of 101 points, the farthest
    /// 13.8 m from the drawn line, whose sharpest bend leaves an angle of 174.9 degrees between
    /// consecutive segments (180 is straight on); recorded when this test was written.
    #[test]
    fn smoothing_keeps_the_ends_and_rounds_a_corner() {
        let drawn = [(-200.0, 0.0), (0.0, 0.0), (0.0, 200.0)];
        let line = smoothed_line(&drawn).expect("a line with length");
        assert_eq!((line[0], line[line.len() - 1]), (drawn[0], drawn[2]), "the ends moved");
        let farthest_m = line.iter().map(|&point| distance_to_line(point, &drawn)).fold(0.0, f64::max);
        assert!(farthest_m <= 40.0, "a point of the smoothed line lies {farthest_m} m from the drawn line");
        let sharpest_deg = line
            .windows(3)
            .map(|three| {
                let (into, out) = ((three[0].0 - three[1].0, three[0].1 - three[1].1), (three[2].0 - three[1].0, three[2].1 - three[1].1));
                let cosine = (into.0 * out.0 + into.1 * out.1) / (into.0.hypot(into.1) * out.0.hypot(out.1));
                cosine.clamp(-1.0, 1.0).acos().to_degrees()
            })
            .fold(180.0, f64::min);
        assert!(sharpest_deg >= 170.0, "consecutive segments of the smoothed line meet at {sharpest_deg} degrees");
    }

    #[test]
    fn awkward_lines_still_give_land() {
        let line = |points: &[(f64, f64)]| River { points: points.to_vec(), width_m: 8.0 };
        let cases = [
            ("a repeated point", vec![line(&[(-256.0, 0.0), (-60.0, 40.0), (-60.0, 40.0), (80.0, -30.0), (256.0, 10.0)])]),
            ("a line that crosses itself", vec![line(&[(-200.0, -200.0), (200.0, 200.0), (200.0, -200.0), (-200.0, 200.0)])]),
            ("two edge-to-edge rivers that cross", vec![line(&[(-256.0, 0.0), (256.0, 0.0)]), line(&[(0.0, -256.0), (0.0, 256.0)])]),
            ("a line with no length", vec![line(&[(30.0, 30.0), (30.0, 30.0)])]),
        ];
        for (name, rivers) in cases {
            let drawing = Drawing { rivers, ..example_picture(512) };
            let land = grown(&drawing, 1);
            for (grid_name, grid) in [("heights", &land.heights), ("drained_m2", &land.drained_m2), ("water_depth_m", &land.water_depth_m)] {
                let bad = grid.data.iter().filter(|value| !value.is_finite()).count();
                assert_eq!(bad, 0, "{name}: {bad} of the {grid_name} are not finite");
            }
            if name == "a line with no length" {
                assert_eq!(land.notes, ["River 1 has no length; it was left out."], "{name}");
                assert_eq!(land.notes_about, [About::River(0)], "{name}");
                assert!(land.rivers.is_empty(), "{name}: {:?}", land.rivers);
            }
        }
    }
}
