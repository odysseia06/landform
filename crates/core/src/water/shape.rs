//! The shape of each channel's bed along it: how its width wanders, wide stretches and narrow ones
//! in turn (`Shape::factor`), and where its deep line lies, moved toward the outside of a bend
//! (`Shape::deep_shift_m`). Numbers per point only; nothing here touches the ground.

use super::channel::{Channel, ChannelEnd, ChannelPoint};
use super::smooth::{distance_m, POINT_SPACING_M};
use crate::rng::{mix64, SplitMix64};
use crate::water_rules::WaterRules;

/// The wander's wavelength is never shorter than this, in metres: channel points lie 4 m apart, so
/// a shorter wave would be noise.
const WANDER_LENGTH_MIN_M: f64 = 24.0;
/// Each extreme of the wander, a wide stretch or a narrow one, lies between this share of
/// `width_wander` and the whole of it.
const SWING_LEAST: f64 = 0.5;
/// A swing of the wander from one extreme to the next lasts as many waves as it rises or falls
/// (in shares of `width_wander`) over this. With extremes between `SWING_LEAST` and 1 a swing rises
/// or falls by 1 to 2 and lasts a third to two thirds of a wave, half a wave on average: so a wave
/// holds a wide stretch and a narrow one, and the factor changes by at most 1.5 times this, in
/// shares of `width_wander` per wave, wherever it is (the smooth step's steepest is 1.5).
const SWING_RISE_PER_WAVE: f64 = 3.0;
/// The deep line takes its whole shift on a bend whose radius is at most this many widths, and a
/// share of it on a wider one: this many widths over the radius.
const BEND_TIGHT_WIDTHS: f64 = 3.0;
/// A bend's radius is measured through the points this many widths either way along the line, or
/// `POINT_SPACING_M`, whichever is more.
const BEND_ARM_WIDTHS: f64 = 1.0;
/// Xored into the seed and mixed: the sub-seed of the wander ("shape" in ASCII and a number).
const SHAPE_TAG: u64 = 0x7368_6170_6500_0001;

/// One value per point of a channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Shape {
    /// What the point's width was multiplied by: 1 where it does not wander.
    pub factor: Vec<f32>,
    /// How far the deep line lies from the channel's line, in metres, along `(dz, -dx)` for the
    /// unit direction of flow `(dx, dz)` there: positive on one side, negative on the other.
    pub deep_shift_m: Vec<f32>,
}

/// Multiplies every point's `width_m` by its factor and returns each channel's shape, in the
/// channels' order. `seed` is the world's; the wander mixes in its own tag (`SHAPE_TAG`).
/// Sequential and deterministic.
///
/// **The wander.** A point's factor is `1 + width_wander × n`, with `n` a smooth value between -1
/// and 1. Along the channel `n` swings from extreme to extreme, the extremes alternately above and
/// below 0 (a wide stretch, then a narrow one), each of a size between `SWING_LEAST` and 1, drawn;
/// from one extreme to the next it follows the smooth step `3t² - 2t³`, flat at both, over as many
/// waves as the swing's rise over `SWING_RISE_PER_WAVE`. So a large swing takes longer than a small
/// one, a wave holds two swings on average, and `n` never changes faster than
/// `1.5 × SWING_RISE_PER_WAVE` per wave. Its draws depend on the seed and on the channel's first
/// point's coordinates, so not on the channel's place in the list; the channel starts at a drawn
/// place within its first swing. `n` is read at a phase, in waves, that advances from one point to
/// the next by the distance between them over the wavelength there: `wander_length_widths` times
/// their mean width before the wander, and at least `WANDER_LENGTH_MIN_M`. Wide and narrow
/// extremes alternate and are alike in size, so over a long channel `n` averages 0.
///
/// **The end.** A channel that ends on another is no wider at its last point, after the wander,
/// than that channel at the point it ends on (its point nearest the last point, which lies on it)
/// after that channel's wander: so each channel is shaped after the channel it ends on, whatever
/// their places in the list. Where the factor at the last point is above the end's ceiling (that
/// width over the last point's width before the wander), over the last wavelength (the last
/// point's, measured back along the channel) the factor is brought down toward the ceiling, by the
/// share of its excess that a smooth step gives, from none a wavelength from the end to all of it
/// at the last point. The ceiling is never below `1 - width_wander`: a channel drawn wider than the
/// one it ends on is narrowed only as far as the wander reaches, and with no wander nothing
/// changes. A brook is never wider than what it ends on before the wander (`trace_channels`), so
/// for a brook the ceiling always holds.
///
/// **The deep line.** At a point it lies `bend_shift × (width / 2) × min(1, BEND_TIGHT_WIDTHS ×
/// width / R)` toward the outside of the bend, `width` the point's width after the wander. `R` is
/// the radius of the circle through the point and the two points `BEND_ARM_WIDTHS` widths along
/// the line either way, or `POINT_SPACING_M` if more (the first point at least that far along, or
/// the channel's end when it comes sooner): on points 4 m apart that measures the bend at the
/// scale of the river, and exactly on a circle. On a straight the shift is 0, and so it is where the
/// line turns back exactly onto itself, which leaves no outside. A point with no line on one side
/// of it (an end, or a point every point of one side lies on) takes the bend of the nearest point
/// with line on both sides, the earlier of two; when there is none, it has no shift.
pub(crate) fn shape_channels(channels: &mut [Channel], seed: u64, rules: &WaterRules) -> Vec<Shape> {
    let seed = mix64(seed ^ SHAPE_TAG);
    let mut shaped: Vec<Option<Shape>> = vec![None; channels.len()];
    for index in ends_first(channels) {
        let mut factor = wander(&channels[index].points, seed, rules);
        if let Some(ceiling) = end_ceiling(channels, index, &shaped, rules) {
            bring_down(&mut factor, &channels[index].points, ceiling, rules);
        }
        let points = &mut channels[index].points;
        for (point, &f) in points.iter_mut().zip(&factor) {
            point.width_m *= f;
        }
        shaped[index] = Some(Shape { factor, deep_shift_m: deep_shifts(points, rules) });
    }
    shaped.into_iter().map(|shape| shape.expect("every channel is shaped")).collect()
}

/// The indices of `channels`, each after the channel it ends on, whatever their places in the list,
/// and otherwise in the list's order: the order `shape_channels` shapes them in and `profiles`
/// works out their water in. An end on a channel not in the list counts as no end on a channel.
pub(super) fn ends_first(channels: &[Channel]) -> Vec<usize> {
    let joined = |index: usize| match channels[index].end {
        ChannelEnd::Channel(joined) if joined < channels.len() => Some(joined),
        _ => None,
    };
    let mut placed = vec![false; channels.len()];
    let mut order = Vec::with_capacity(channels.len());
    for first in 0..channels.len() {
        // `first` and the channels its end leads to that are not yet placed, downstream last.
        let mut waiting = Vec::new();
        let mut next = Some(first);
        while let Some(index) = next.filter(|&index| !placed[index] && !waiting.contains(&index)) {
            waiting.push(index);
            next = joined(index);
        }
        for &index in waiting.iter().rev() {
            placed[index] = true;
            order.push(index);
        }
    }
    order
}

/// The world `(x, z)` of `point`, in metres.
fn at(point: &ChannelPoint) -> (f64, f64) {
    (point.x_m, point.z_m)
}

/// The wander's wavelength where a channel is `width_m` wide before the wander, in metres.
fn wavelength_m(width_m: f64, rules: &WaterRules) -> f64 {
    (f64::from(rules.wander_length_widths) * width_m).max(WANDER_LENGTH_MIN_M)
}

/// The factor of each of `points`, a channel's, before its end is met, as `shape_channels` describes
/// it, with `seed` the wander's sub-seed.
fn wander(points: &[ChannelPoint], seed: u64, rules: &WaterRules) -> Vec<f32> {
    let Some(first) = points.first() else { return Vec::new() };
    let channel_seed = mix64(mix64(seed ^ first.x_m.to_bits()) ^ first.z_m.to_bits());
    let (mut swings, mut phase) = Swings::new(channel_seed);
    let mut factor = Vec::with_capacity(points.len());
    for (k, point) in points.iter().enumerate() {
        if k > 0 {
            let before = &points[k - 1];
            let mean_width_m = (f64::from(before.width_m) + f64::from(point.width_m)) / 2.0;
            phase += distance_m(at(before), at(point)) / wavelength_m(mean_width_m, rules);
        }
        factor.push(1.0 + rules.width_wander * swings.at(phase) as f32);
    }
    factor
}

/// The smooth value `n` of one channel's wander, read along its phase in waves: swings from extreme
/// to extreme, as `shape_channels` describes them, drawn as the phase reaches them.
struct Swings {
    draws: SplitMix64,
    /// The phase at which the current swing starts, and its length, in waves.
    start: f64,
    length: f64,
    /// The extremes the current swing runs from and to.
    from: f64,
    to: f64,
}

impl Swings {
    /// The swings drawn from `seed`, and the phase the channel starts at, within the first swing.
    fn new(seed: u64) -> (Self, f64) {
        let mut draws = SplitMix64::new(seed);
        let above = draws.next_u64() & 1 == 1;
        let from = Self::extreme(&mut draws, above);
        let to = Self::extreme(&mut draws, !above);
        let length = Self::length(from, to);
        let phase = draws.next_f64() * length;
        (Self { draws, start: 0.0, length, from, to }, phase)
    }

    /// The next extreme: its size drawn between `SWING_LEAST` and 1, above 0 or below it.
    fn extreme(draws: &mut SplitMix64, above: bool) -> f64 {
        let size = SWING_LEAST + (1.0 - SWING_LEAST) * draws.next_f64();
        if above {
            size
        } else {
            -size
        }
    }

    /// The length in waves of the swing from `from` to `to`.
    fn length(from: f64, to: f64) -> f64 {
        (to - from).abs() / SWING_RISE_PER_WAVE
    }

    /// `n` at `phase`, which never falls from one call to the next.
    fn at(&mut self, phase: f64) -> f64 {
        while phase >= self.start + self.length {
            self.start += self.length;
            self.from = self.to;
            self.to = Self::extreme(&mut self.draws, self.from < 0.0);
            self.length = Self::length(self.from, self.to);
        }
        let t = ((phase - self.start) / self.length).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * t * t * (3.0 - 2.0 * t)
    }
}

/// The highest factor the last point of channel `index` may have, as `shape_channels` describes it,
/// when it ends on a channel; `shaped` holds the shapes of the channels already shaped, whose
/// widths are their widths after the wander.
fn end_ceiling(channels: &[Channel], index: usize, shaped: &[Option<Shape>], rules: &WaterRules) -> Option<f32> {
    let channel = &channels[index];
    let ChannelEnd::Channel(joined) = channel.end else { return None };
    if shaped.get(joined).is_none_or(Option::is_none) {
        debug_assert!(false, "channel {index} is shaped before the channel {joined} it ends on");
        return None;
    }
    let last = channel.points.last().filter(|last| last.width_m > 0.0)?;
    let squared = |point: &&ChannelPoint| (point.x_m - last.x_m).powi(2) + (point.z_m - last.z_m).powi(2);
    let joined_m = channels[joined].points.iter().min_by(|a, b| squared(a).total_cmp(&squared(b)))?.width_m;
    // The largest factor that leaves the last point no wider than the joined channel there.
    let mut ceiling = joined_m / last.width_m;
    while last.width_m * ceiling > joined_m {
        ceiling = ceiling.next_down();
    }
    Some(ceiling.max(1.0 - rules.width_wander))
}

/// `factor`, of a channel along `points` (their widths before the wander), brought down over its
/// last wavelength to `ceiling` at its last point when it lies above it there, as `shape_channels`
/// describes it.
fn bring_down(factor: &mut [f32], points: &[ChannelPoint], ceiling: f32, rules: &WaterRules) {
    let (Some(&last), Some(last_point)) = (factor.last(), points.last()) else { return };
    if last <= ceiling {
        return;
    }
    let reach_m = wavelength_m(f64::from(last_point.width_m), rules);
    let mut from_end_m = 0.0;
    for k in (0..points.len()).rev() {
        if k + 1 < points.len() {
            from_end_m += distance_m(at(&points[k]), at(&points[k + 1]));
        }
        if from_end_m >= reach_m {
            break;
        }
        let t = 1.0 - from_end_m / reach_m;
        let share = (t * t * (3.0 - 2.0 * t)) as f32;
        // All of the excess at the last point, where `share` is exactly 1.
        let lowered = factor[k].min(ceiling) * share + factor[k] * (1.0 - share);
        factor[k] = lowered.min(factor[k]);
    }
}

/// The shift of the deep line at each of `points`, a channel's after the wander, as
/// `shape_channels` describes it.
fn deep_shifts(points: &[ChannelPoint], rules: &WaterRules) -> Vec<f32> {
    let bends: Vec<Option<f64>> = (0..points.len()).map(|k| bend(points, k)).collect();
    // The nearest point at or before each point whose bend is measured, then the nearest at or after.
    let mut before: Vec<Option<usize>> = Vec::with_capacity(points.len());
    for (k, bend) in bends.iter().enumerate() {
        before.push(if bend.is_some() { Some(k) } else { before.last().copied().flatten() });
    }
    let mut after: Vec<Option<usize>> = vec![None; points.len()];
    for k in (0..points.len()).rev() {
        after[k] = if bends[k].is_some() { Some(k) } else { after.get(k + 1).copied().flatten() };
    }
    (0..points.len())
        .map(|k| {
            let nearest = match (before[k], after[k]) {
                (Some(b), Some(a)) => Some(if k - b <= a - k { b } else { a }),
                (b, a) => b.or(a),
            };
            let curvature = nearest.and_then(|j| bends[j]).unwrap_or(0.0);
            deep_shift_m(points[k].width_m, curvature, rules)
        })
        .collect()
}

/// The signed curvature of the line of `points` at point `k`, in 1 / m: 1 over the radius of the
/// circle through the point and the ends of its arms (`arm_end`), positive where the line turns
/// from `(dx, dz)` toward `(-dz, dx)`, so that the outside lies along `(dz, -dx)`; 0 where the line
/// turns back exactly onto itself. `None` when either arm has no length.
fn bend(points: &[ChannelPoint], k: usize) -> Option<f64> {
    let arm_m = (BEND_ARM_WIDTHS * f64::from(points[k].width_m)).max(POINT_SPACING_M);
    let back = arm_end(points, k, arm_m, (0..k).rev())?;
    let ahead = arm_end(points, k, arm_m, k + 1..points.len())?;
    let (a, b, c) = (at(&points[back]), at(&points[k]), at(&points[ahead]));
    let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
    let lengths = distance_m(a, b) * distance_m(b, c) * distance_m(a, c);
    Some(if lengths > 0.0 { 2.0 * cross / lengths } else { 0.0 })
}

/// The index of the first of `points` reached over `steps` (indices walked away from point `k`,
/// nearest first) at least `arm_m` along the line from point `k`, or of the last one walked when the
/// line ends sooner; `None` when every point walked lies on point `k`.
fn arm_end(points: &[ChannelPoint], k: usize, arm_m: f64, steps: impl Iterator<Item = usize>) -> Option<usize> {
    let mut from = at(&points[k]);
    let mut along_m = 0.0;
    let mut reached = None;
    for j in steps {
        along_m += distance_m(from, at(&points[j]));
        from = at(&points[j]);
        if along_m > 0.0 {
            reached = Some(j);
        }
        if along_m >= arm_m {
            break;
        }
    }
    reached
}

/// The deep line's shift at a point `width_m` wide where the line's curvature is `curvature`, as
/// `shape_channels` describes it: toward the outside, so of the curvature's sign.
fn deep_shift_m(width_m: f32, curvature: f64, rules: &WaterRules) -> f32 {
    let width_m = f64::from(width_m);
    let tightness = (BEND_TIGHT_WIDTHS * width_m * curvature.abs()).min(1.0);
    let size_m = f64::from(rules.bend_shift) * width_m / 2.0 * tightness;
    if size_m > 0.0 {
        size_m.copysign(curvature) as f32
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Points of the hand-made channels lie this far apart, in metres.
    const SPACING_M: f64 = 4.0;

    fn point((x_m, z_m): (f64, f64), width_m: f32) -> ChannelPoint {
        ChannelPoint { x_m, z_m, width_m, drained_m2: 0.0 }
    }

    fn channel(points: Vec<ChannelPoint>, end: ChannelEnd) -> Channel {
        Channel { drawn: Some(0), from_lake: None, points, end }
    }

    /// A channel of `count` points `SPACING_M` apart from `from` along `(dx, dz)`, a unit
    /// direction, `width_m` wide, ending `end`.
    fn straight(from: (f64, f64), (dx, dz): (f64, f64), count: usize, width_m: f32, end: ChannelEnd) -> Channel {
        let points = (0..count)
            .map(|k| point((from.0 + dx * SPACING_M * k as f64, from.1 + dz * SPACING_M * k as f64), width_m))
            .collect();
        channel(points, end)
    }

    /// The brief's channel: 400 points 4 m apart, east along `z = 0` from `x = -800`, ending at the edge.
    fn long_straight(width_m: f32) -> Channel {
        straight((-800.0, 0.0), (1.0, 0.0), 400, width_m, ChannelEnd::Edge)
    }

    /// A channel along a circle round `centre` of `radius_m`, from the angle `from` (radians, from
    /// `+x` toward `+z`) turning by `turn` (counter-clockwise from `+x` toward `+z` when positive),
    /// its points at most `SPACING_M` apart, `width_m` wide, ending at the edge.
    fn arc(centre: (f64, f64), radius_m: f64, from: f64, turn: f64, width_m: f32) -> Channel {
        let steps = (turn.abs() * radius_m / SPACING_M).ceil() as usize;
        let points = (0..=steps)
            .map(|k| {
                let angle = from + turn * k as f64 / steps as f64;
                point((centre.0 + radius_m * angle.cos(), centre.1 + radius_m * angle.sin()), width_m)
            })
            .collect();
        channel(points, ChannelEnd::Edge)
    }

    /// `channels` shaped with `seed` and `rules`, and their shapes.
    fn shaped(mut channels: Vec<Channel>, seed: u64, rules: &WaterRules) -> (Vec<Channel>, Vec<Shape>) {
        let shapes = shape_channels(&mut channels, seed, rules);
        assert_eq!(shapes.len(), channels.len());
        for (channel, shape) in channels.iter().zip(&shapes) {
            assert_eq!((shape.factor.len(), shape.deep_shift_m.len()), (channel.points.len(), channel.points.len()));
        }
        (channels, shapes)
    }

    /// The one channel `channel` shaped with `seed` and `rules`, and its shape.
    fn shaped_one(channel: Channel, seed: u64, rules: &WaterRules) -> (Channel, Shape) {
        let (mut channels, mut shapes) = shaped(vec![channel], seed, rules);
        (channels.remove(0), shapes.remove(0))
    }

    fn wander(width_wander: f32) -> WaterRules {
        WaterRules { width_wander, ..WaterRules::default() }
    }

    /// The rules at the ends of their ranges that make the most of a shape: the widest wander over
    /// the shortest wave, and the largest shift.
    fn extreme() -> WaterRules {
        WaterRules { width_wander: 0.5, wander_length_widths: 3.0, bend_shift: 0.5, ..WaterRules::default() }
    }

    /// How many times `factor` crosses 1 from one point to the next.
    fn crossings(factor: &[f32]) -> usize {
        factor.windows(2).filter(|pair| (pair[0] > 1.0) != (pair[1] > 1.0)).count()
    }

    /// The unit direction of flow at point `k` of `points`: from the point before to the point after.
    fn direction(points: &[ChannelPoint], k: usize) -> (f64, f64) {
        let (before, after) = (&points[k.saturating_sub(1)], &points[(k + 1).min(points.len() - 1)]);
        let (dx, dz) = (after.x_m - before.x_m, after.z_m - before.z_m);
        let length = dx.hypot(dz);
        (dx / length, dz / length)
    }

    /// How far point `k`'s deep line lies farther from `centre` than the point itself, in metres.
    fn outward_m(points: &[ChannelPoint], shape: &Shape, k: usize, centre: (f64, f64)) -> f64 {
        let (dx, dz) = direction(points, k);
        let shift = f64::from(shape.deep_shift_m[k]);
        let (x, z) = (points[k].x_m + shift * dz, points[k].z_m - shift * dx);
        (x - centre.0).hypot(z - centre.1) - (points[k].x_m - centre.0).hypot(points[k].z_m - centre.1)
    }

    #[test]
    fn the_width_wanders_within_its_share() {
        for seed in 1..=10 {
            let (channel, shape) = shaped_one(long_straight(16.0), seed, &WaterRules::default());
            let factor = &shape.factor;
            assert!(factor.iter().all(|f| (0.75..=1.25).contains(f)), "seed {seed}: a factor beyond 1 ± 0.25: {factor:?}");
            let (least, most) = factor.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &f| (lo.min(f), hi.max(f)));
            assert!(least <= 0.85 && most >= 1.15, "seed {seed}: the factor only spans {least} to {most}");
            let mean = factor.iter().map(|&f| f64::from(f)).sum::<f64>() / factor.len() as f64;
            assert!((mean - 1.0).abs() <= 0.03, "seed {seed}: the factor's mean is {mean}");
            for (k, (point, &f)) in channel.points.iter().zip(factor).enumerate() {
                assert_eq!(point.width_m, 16.0 * f, "seed {seed}: point {k} is not its width times its factor");
            }

            let (_, wide) = shaped_one(long_straight(16.0), seed, &wander(0.5));
            assert!(wide.factor.iter().all(|f| (0.5..=1.5).contains(f)), "seed {seed}: a factor beyond 1 ± 0.5: {:?}", wide.factor);
            let (least, most) = wide.factor.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &f| (lo.min(f), hi.max(f)));
            assert!(least < 0.75 && most > 1.25, "seed {seed}: with a share of 0.5 the factor only spans {least} to {most}");
        }
    }

    #[test]
    fn the_wander_is_smooth() {
        let rules = WaterRules::default();
        assert_eq!((rules.width_wander, rules.wander_length_widths), (0.25, 6.0));
        for seed in 1..=10 {
            let (_, shape) = shaped_one(long_straight(16.0), seed, &rules);
            for (k, pair) in shape.factor.windows(2).enumerate() {
                assert!((pair[1] - pair[0]).abs() < 0.05, "seed {seed}: the factor jumps from {} to {} at point {k}", pair[0], pair[1]);
            }
        }
    }

    #[test]
    fn the_wander_keeps_its_length() {
        let rules = WaterRules::default();
        for seed in 1..=10 {
            // A wave of 6 widths, 96 m: 16.7 waves over the 1,600 m, two crossings each.
            let (_, river) = shaped_one(long_straight(16.0), seed, &rules);
            let crossed = crossings(&river.factor);
            assert!((20..=60).contains(&crossed), "seed {seed}: the 16 m channel's factor crosses 1 {crossed} times");
            // Six widths would be 7.2 m, but no wave is shorter than 24 m: 66.7 waves over the 1,600 m.
            // Counted per wave as above (1.2 to 3.6 crossings), a 24 m wave crosses 80 to 240 times;
            // a 7.2 m wave would cross at least 266 times.
            let (_, brook) = shaped_one(long_straight(1.2), seed, &rules);
            let crossed = crossings(&brook.factor);
            assert!((80..=240).contains(&crossed), "seed {seed}: the 1.2 m channel's factor crosses 1 {crossed} times");
        }
    }

    #[test]
    fn the_same_seed_gives_the_same_wander_and_another_seed_another() {
        let rules = WaterRules::default();
        let channels = || vec![long_straight(16.0), arc((0.0, 300.0), 150.0, 0.0, PI, 16.0)];
        let first = shaped(channels(), 7, &rules);
        assert_eq!(first, shaped(channels(), 7, &rules), "the same seed gave another shape");
        let other = shaped(channels(), 8, &rules);
        for k in 0..2 {
            assert_ne!(first.1[k].factor, other.1[k].factor, "channel {k}: another seed gave the same wander");
            assert_ne!(first.0[k], other.0[k], "channel {k}: another seed gave the same widths");
        }
    }

    #[test]
    fn a_channels_wander_does_not_depend_on_its_place_in_the_list() {
        let rules = WaterRules::default();
        let (a, b) = (long_straight(16.0), arc((0.0, 300.0), 150.0, 0.0, PI, 12.0));
        let (forward, forward_shapes) = shaped(vec![a.clone(), b.clone()], 5, &rules);
        let (backward, backward_shapes) = shaped(vec![b, a], 5, &rules);
        assert_eq!((&forward[0], &forward_shapes[0]), (&backward[1], &backward_shapes[1]), "the straight channel");
        assert_eq!((&forward[1], &forward_shapes[1]), (&backward[0], &backward_shapes[0]), "the arc");
    }

    /// A main channel 400 points long, east along `z = 0` from `x = -800`, `main_m` wide, and a
    /// tributary `tributary_m` wide coming south from `(0, -400)` that ends on it at `(0, 0)`, its
    /// point 200; the tributary first, so it ends on a channel listed after it.
    fn junction(tributary_m: f32, main_m: f32) -> Vec<Channel> {
        let tributary = straight((0.0, -400.0), (0.0, 1.0), 101, tributary_m, ChannelEnd::Channel(1));
        let main = long_straight(main_m);
        assert_eq!((tributary.points[100].x_m, tributary.points[100].z_m), (main.points[200].x_m, main.points[200].z_m));
        vec![tributary, main]
    }

    #[test]
    fn a_junctions_shapes_do_not_depend_on_its_order_in_the_list() {
        let rules = WaterRules::default();
        let mut brought_down = 0;
        for seed in 1..=20 {
            // The tributary listed before the channel it ends on, and after it.
            let forward = junction(4.0, 4.0);
            let backward = vec![forward[1].clone(), Channel { end: ChannelEnd::Channel(0), ..forward[0].clone() }];
            let (forward, forward_shapes) = shaped(forward, seed, &rules);
            let (backward, backward_shapes) = shaped(backward, seed, &rules);
            assert_eq!((&forward[0].points, &forward_shapes[0]), (&backward[1].points, &backward_shapes[1]), "seed {seed}: the tributary");
            assert_eq!((&forward[1], &forward_shapes[1]), (&backward[0], &backward_shapes[0]), "seed {seed}: the channel it ends on");
            let free = Channel { end: ChannelEnd::Edge, ..junction(4.0, 4.0).remove(0) };
            brought_down += usize::from(shaped_one(free, seed, &rules).1 != forward_shapes[0]);
        }
        assert!(brought_down > 0, "no seed brought the tributary down to the channel it ends on");
    }

    #[test]
    fn no_wander_changes_nothing() {
        let rules = wander(0.0);
        // A spring widening from 1.2 m to 16 m, and a tributary ending on it, listed first, 12 m
        // wide: wider than the 8.6 m it ends on, as a drawn river may be.
        let mut channels = junction(12.0, 16.0);
        for (k, point) in channels[1].points.iter_mut().enumerate() {
            point.width_m = 1.2 + 14.8 * (k as f32 / 399.0);
        }
        let before = channels.clone();
        let (after, shapes) = shaped(channels, 3, &rules);
        for (c, shape) in shapes.iter().enumerate() {
            assert!(shape.factor.iter().all(|&f| f == 1.0), "channel {c}: a factor is not 1: {:?}", shape.factor);
            for (k, (was, is)) in before[c].points.iter().zip(&after[c].points).enumerate() {
                assert_eq!(was.width_m.to_bits(), is.width_m.to_bits(), "channel {c}, point {k}: the width changed");
                assert_eq!((was.x_m, was.z_m, was.drained_m2), (is.x_m, is.z_m, is.drained_m2), "channel {c}, point {k} moved");
            }
        }
    }

    #[test]
    fn a_channel_is_no_wider_than_what_it_joins_at_its_end() {
        let rules = WaterRules::default();
        // The last wavelength of a 4 m channel: six widths are 24 m, the least wavelength.
        let last_wave_m = 24.0;
        let mut brought_down = 0;
        for seed in 1..=20 {
            let (channels, shapes) = shaped(junction(4.0, 4.0), seed, &rules);
            let (tributary, main) = (&channels[0], &channels[1]);
            let end_m = tributary.points[100].width_m;
            assert!(end_m <= main.points[200].width_m, "seed {seed}: the tributary ends {end_m} m wide on {} m", main.points[200].width_m);

            // Alone, ending at the edge, it has the same wander but for its last wavelength,
            // where it is only ever brought down, and stays within its share.
            let free = Channel { end: ChannelEnd::Edge, ..junction(4.0, 4.0).remove(0) };
            let (alone, alone_shape) = shaped_one(free, seed, &rules);
            for (k, (&is, &was)) in shapes[0].factor.iter().zip(&alone_shape.factor).enumerate() {
                let from_end_m = SPACING_M * (100 - k) as f64;
                if from_end_m >= last_wave_m {
                    assert_eq!(is, was, "seed {seed}: point {k}, {from_end_m} m from the end, changed");
                } else {
                    assert!(is <= was && (0.75..=1.25).contains(&is), "seed {seed}: point {k} has {is}, alone {was}");
                }
            }
            if alone.points[100].width_m > main.points[200].width_m {
                brought_down += 1;
            }
        }
        assert!(brought_down > 0, "no seed made the tributary wider than the channel it joins");
    }

    #[test]
    fn a_bend_moves_the_deep_line_outward() {
        // The width stays 16 m, so that the bend alone moves the deep line.
        let rules = wander(0.0);
        let width = 16.0;
        let full_m = 0.3 * width / 2.0;
        let centre = (0.0, 0.0);
        let shifts = |turn: f64, radius_m: f64| {
            let (channel, shape) = shaped_one(arc(centre, radius_m, 0.0, turn, width as f32), 1, &rules);
            let outward: Vec<f64> = (0..channel.points.len()).map(|k| outward_m(&channel.points, &shape, k, centre)).collect();
            (shape.deep_shift_m, outward)
        };

        // A half turn at three widths: the whole shift, on the outside.
        let (tight, outward) = shifts(PI, 3.0 * width);
        for (k, (&shift, &out)) in tight.iter().zip(&outward).enumerate() {
            assert!((f64::from(shift) - full_m).abs() <= 0.15 * full_m, "point {k} of the tight bend is shifted {shift} m, not {full_m} m");
            assert!(out > 0.0, "point {k} of the tight bend: the deep line lies inside the line, by {out} m");
        }
        // The same bend turning the other way: the other sign, still on the outside.
        let (mirrored, outward) = shifts(-PI, 3.0 * width);
        for (k, ((&shift, &other), &out)) in mirrored.iter().zip(&tight).zip(&outward).enumerate() {
            assert!(shift.signum() == -other.signum() && out > 0.0, "point {k} turning the other way: {shift} m, {out} m outward");
        }
        // At twelve widths, a quarter of it.
        let (gentle, outward) = shifts(PI, 12.0 * width);
        for (k, (&shift, &out)) in gentle.iter().zip(&outward).enumerate() {
            let expected = full_m / 4.0;
            assert!((f64::from(shift) - expected).abs() <= 0.25 * expected, "point {k} of the gentle bend is shifted {shift} m, not {expected} m");
            assert!(out > 0.0, "point {k} of the gentle bend: the deep line lies inside the line, by {out} m");
        }
        // On a straight, none.
        let (_, straight) = shaped_one(long_straight(width as f32), 1, &rules);
        assert!(straight.deep_shift_m.iter().all(|shift| shift.abs() < 0.01), "a straight's deep line moves: {:?}", straight.deep_shift_m);
        // With no bend shift, exactly none on the tight bend.
        let (_, unshifted) = shaped_one(arc(centre, 3.0 * width, 0.0, PI, width as f32), 1, &WaterRules { bend_shift: 0.0, ..rules });
        assert!(unshifted.deep_shift_m.iter().all(|&shift| shift == 0.0), "with no bend shift: {:?}", unshifted.deep_shift_m);
    }

    #[test]
    fn a_kink_keeps_the_deep_line_in_the_bed() {
        let width = 16.0;
        let east = |to_x: f64, z: f64| (0..=10).map(move |k| point((to_x - 40.0 + SPACING_M * k as f64, z), width)).collect::<Vec<_>>();
        // East to (0, 0), then south: a right angle.
        let south: Vec<ChannelPoint> = (1..=10).map(|k| point((0.0, SPACING_M * k as f64), width)).collect();
        let right_angle = [east(0.0, 0.0), south.clone()].concat();
        // East to (0, 0), round a half circle of 2 m about (0, 2), and back west along z = 4.
        let round: Vec<ChannelPoint> = [-PI / 6.0, PI / 6.0, PI / 2.0]
            .iter()
            .map(|&angle| point((2.0 * angle.cos(), 2.0 + 2.0 * angle.sin()), width))
            .collect();
        let west: Vec<ChannelPoint> = (1..=10).map(|k| point((-SPACING_M * k as f64, 4.0), width)).collect();
        let hairpin = [east(0.0, 0.0), round, west].concat();
        // The right angle with its corner given twice.
        let repeated = [east(0.0, 0.0), vec![point((0.0, 0.0), width)], south].concat();
        let two = vec![point((0.0, 0.0), width), point((SPACING_M, 0.0), width)];

        for rules in [WaterRules::default(), extreme()] {
            for (name, points) in [("right angle", &right_angle), ("hairpin", &hairpin), ("repeated point", &repeated), ("two points", &two)] {
                let (channel, shape) = shaped_one(channel(points.clone(), ChannelEnd::Edge), 2, &rules);
                assert!(shape.factor.iter().all(|f| f.is_finite()), "{name}: {:?}", shape.factor);
                let mut largest_share: f64 = 0.0;
                for (k, (&shift, point)) in shape.deep_shift_m.iter().zip(&channel.points).enumerate() {
                    let most = f64::from(rules.bend_shift) * f64::from(point.width_m) / 2.0;
                    assert!(shift.is_finite() && f64::from(shift).abs() <= most * (1.0 + 1e-6), "{name}, point {k}: shifted {shift} m, at most {most} m");
                    largest_share = largest_share.max(f64::from(shift).abs() / most);
                }
                // A corner far tighter than three widths takes the whole shift.
                if name == "right angle" || name == "hairpin" {
                    assert!(largest_share > 0.999, "{name}: the deep line moves only {largest_share} of the whole shift at the corner");
                }
            }
        }
    }

    #[test]
    fn a_wide_river_and_a_narrow_brook_keep_their_shape() {
        for rules in [WaterRules::default(), extreme()] {
            let share = rules.width_wander;
            for width in [64.0, 0.2] {
                for channel in [long_straight(width), arc((0.0, 0.0), 300.0, 0.0, PI, width)] {
                    let (channel, shape) = shaped_one(channel, 4, &rules);
                    for (k, (&f, point)) in shape.factor.iter().zip(&channel.points).enumerate() {
                        assert!(f.is_finite() && (1.0 - share..=1.0 + share).contains(&f), "{width} m, point {k}: factor {f}");
                        assert!(point.width_m.is_finite() && point.width_m > 0.0, "{width} m, point {k}: {} m wide", point.width_m);
                        let shift = shape.deep_shift_m[k];
                        assert!(shift.is_finite() && shift.abs() <= point.width_m / 2.0, "{width} m, point {k}: shifted {shift} m");
                    }
                }
            }
        }
    }
}
