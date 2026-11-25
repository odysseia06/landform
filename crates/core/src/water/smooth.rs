//! A channel's line made smooth: cut into points at most `POINT_SPACING_M` apart and smoothed, so
//! that no stair of the land model's 8 m cells shows.

/// A channel's points lie at most this far apart, in metres.
pub(super) const POINT_SPACING_M: f64 = 4.0;
/// A channel's line is smoothed by this many passes of a mean over this many points either way,
/// about one cell.
const SMOOTH_PASSES: usize = 3;
const SMOOTH_REACH_POINTS: usize = 2;

/// A point of a channel's line before its width is known.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sample {
    /// World `(x, z)`, in metres.
    pub(super) at_m: (f64, f64),
    /// The distance along its path, in metres.
    pub(super) along_m: f64,
    pub(super) drained_m2: f64,
}

/// How many equal steps, each at most `POINT_SPACING_M`, the way from `from` to `to` is cut into.
pub(super) fn steps(from: (f64, f64), to: (f64, f64)) -> usize {
    ((distance_m(from, to) / POINT_SPACING_M).ceil() as usize).max(1)
}

/// The points the way from `from` to `to` is cut into (`steps`), its ends left out.
pub(super) fn inner_points(from: (f64, f64), to: (f64, f64)) -> impl Iterator<Item = (f64, f64)> {
    let steps = steps(from, to);
    (1..steps).map(move |step| between(from, to, step as f64 / steps as f64))
}

/// The line through `corners` as points at most `POINT_SPACING_M` apart: the way between each two
/// corners cut into `steps`, the distance along the path and the ground drained linear between
/// them. The way from corner `k` to the next gives the points from `k` on, corner `k` first.
pub(super) fn resampled(corners: &[Sample]) -> Vec<Sample> {
    let mut points = Vec::new();
    for pair in corners.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        let steps = steps(from.at_m, to.at_m);
        for step in 0..steps {
            let share = step as f64 / steps as f64;
            points.push(Sample {
                at_m: between(from.at_m, to.at_m, share),
                along_m: from.along_m + (to.along_m - from.along_m) * share,
                drained_m2: from.drained_m2 + (to.drained_m2 - from.drained_m2) * share,
            });
        }
    }
    points.extend(corners.last().copied());
    points
}

/// `points` moved by `SMOOTH_PASSES` passes of a mean over `SMOOTH_REACH_POINTS` points either way,
/// over fewer near a held point, so that the held points stay where they are. The ends are held,
/// and so is any point a pass would move onto a lake cell (`on_a_lake`), from that pass on; the
/// points given must lie on no lake cell. Each point stays a mean of the points given, and each
/// pass keeps consecutive points no further apart than they were.
pub(super) fn smoothed(mut points: Vec<Sample>, on_a_lake: impl Fn((f64, f64)) -> bool) -> Vec<Sample> {
    if points.is_empty() {
        return points;
    }
    let mut line: Vec<(f64, f64)> = points.iter().map(|point| point.at_m).collect();
    let mut held = vec![false; line.len()];
    held[0] = true;
    held[line.len() - 1] = true;
    for _ in 0..SMOOTH_PASSES {
        loop {
            let next = mean_pass(&line, &held);
            let mut moved_onto_a_lake = false;
            for (k, &point) in next.iter().enumerate() {
                // A held point stays where it is, so each round holds a point more, or ends.
                if !held[k] && on_a_lake(point) {
                    held[k] = true;
                    moved_onto_a_lake = true;
                }
            }
            if !moved_onto_a_lake {
                line = next;
                break;
            }
        }
    }
    for (point, at_m) in points.iter_mut().zip(line) {
        point.at_m = at_m;
    }
    points
}

/// One pass over `line`: each point becomes the mean of the points up to `SMOOTH_REACH_POINTS`
/// either way, over fewer where a held point is nearer, so that a held point stays where it is.
/// Both ends must be held.
fn mean_pass(line: &[(f64, f64)], held: &[bool]) -> Vec<(f64, f64)> {
    // The nearest held point at or after each point.
    let mut held_after = vec![line.len() - 1; line.len()];
    for k in (0..line.len() - 1).rev() {
        held_after[k] = if held[k] { k } else { held_after[k + 1] };
    }
    let mut held_before = 0;
    (0..line.len())
        .map(|k| {
            if held[k] {
                held_before = k;
            }
            let reach = SMOOTH_REACH_POINTS.min(k - held_before).min(held_after[k] - k);
            let window = &line[k - reach..=k + reach];
            let (x, z) = window.iter().fold((0.0, 0.0), |(x, z), point| (x + point.0, z + point.1));
            (x / window.len() as f64, z / window.len() as f64)
        })
        .collect()
}

pub(super) fn distance_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// The point `share` of the way from `a` to `b`.
pub(super) fn between(a: (f64, f64), b: (f64, f64), share: f64) -> (f64, f64) {
    (a.0 + (b.0 - a.0) * share, a.1 + (b.1 - a.1) * share)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(x_m: f64, z_m: f64, along_m: f64) -> Sample {
        Sample { at_m: (x_m, z_m), along_m, drained_m2: along_m }
    }

    #[test]
    fn a_line_is_cut_into_points_at_most_4_m_apart_through_its_corners() {
        // An axis step, a diagonal one and a long one.
        let corners = [corner(0.0, 0.0, 0.0), corner(8.0, 0.0, 8.0), corner(16.0, 8.0, 19.3), corner(16.0, 50.0, 61.3)];
        let points = resampled(&corners);
        assert_eq!(points.len(), 2 + 3 + 11 + 1);
        for corner in &corners {
            assert!(points.iter().any(|point| point.at_m == corner.at_m), "the corner {:?} is not a point", corner.at_m);
        }
        for pair in points.windows(2) {
            let apart = distance_m(pair[0].at_m, pair[1].at_m);
            assert!(apart <= POINT_SPACING_M && pair[1].along_m > pair[0].along_m, "{pair:?}");
        }
        let inner: Vec<(f64, f64)> = inner_points(corners[1].at_m, corners[2].at_m).collect();
        assert_eq!(inner, [points[3].at_m, points[4].at_m], "the inner points of the diagonal step");
    }

    #[test]
    fn smoothing_keeps_the_ends_and_the_spacing_and_keeps_off_the_lakes() {
        // A stair of 8 m steps east and south: (0, 0), (8, 0), (8, 8), (16, 8), ... (48, 40).
        let stair: Vec<Sample> =
            (0..12).map(|k| corner(8.0 * f64::from(k / 2 + k % 2), 8.0 * f64::from(k / 2), f64::from(k))).collect();
        let points = resampled(&stair);
        let ends = |line: &[Sample]| (line[0].at_m, line[line.len() - 1].at_m);
        let free = smoothed(points.clone(), |_| false);
        assert_eq!(ends(&free), ends(&points));
        assert!(free.iter().zip(&points).any(|(a, b)| distance_m(a.at_m, b.at_m) > 1.0), "the stair was not smoothed");
        for pair in free.windows(2) {
            assert!(distance_m(pair[0].at_m, pair[1].at_m) <= POINT_SPACING_M + 1e-9, "{pair:?}");
        }
        // Small lakes 2 m inside each outer corner, where the smoothing carries it, on no point of the stair.
        let on_a_lake =
            |at_m: (f64, f64)| (0..6).any(|j| distance_m(at_m, (8.0 * f64::from(j) + 6.0, 8.0 * f64::from(j) + 2.0)) < 1.5);
        assert!(points.iter().all(|point| !on_a_lake(point.at_m)), "the stair itself lies on a lake");
        assert!(free.iter().any(|point| on_a_lake(point.at_m)), "smoothed freely, no point reaches a lake");
        let kept_off = smoothed(points.clone(), on_a_lake);
        assert!(kept_off.iter().all(|point| !on_a_lake(point.at_m)), "a point moved onto a lake");
        assert_eq!(ends(&kept_off), ends(&points));
        for pair in kept_off.windows(2) {
            assert!(distance_m(pair[0].at_m, pair[1].at_m) <= POINT_SPACING_M + 1e-9, "{pair:?}");
        }
    }
}
