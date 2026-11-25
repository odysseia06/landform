//! The tests of the beds (`bed.rs`).

use super::*;
use crate::grow::tests::{example_worlds, ExampleWorld};
use crate::water::channel::trace_channels;
use crate::water::lake::{fill_lakes, LakeShape};
use crate::water::shape::shape_channels;
use crate::water::tests::{at_one_metre, behind_the_head, inside_the_lines_ended_on, near_lines, reach_of_water, reached_by_line, Ends, LinePoint};
use crate::water::floor_m;
use std::sync::atomic::AtomicBool;

/// Notes as the steps write them, from what each is about and its sentence.
fn said(notes: &[(About, &str)]) -> Vec<Note> {
    notes.iter().map(|&(about, note)| (about, note.to_string())).collect()
}

/// A `size` x `size` grid of 1 m cells whose cell `(i, j)`, at world `(i - size / 2, j - size / 2)`,
/// holds `height_m` there.
fn ground(size: usize, height_m: impl Fn(f64, f64) -> f64) -> Grid<f32> {
    let half = (size / 2) as f64;
    let mut grid = Grid::new(size, size, 1.0);
    for j in 0..size {
        for i in 0..size {
            grid.data[j * size + i] = height_m(i as f64 - half, j as f64 - half) as f32;
        }
    }
    grid
}

/// Ground falling 2 % to the east from 100 m at `x = 0`.
fn tilted(x: f64) -> f64 {
    100.0 - 0.02 * x
}

/// The value of `grid`, as `ground` lays it out, at world `(x, z)`.
fn at(grid: &Grid<f32>, (x, z): (i32, i32)) -> f32 {
    let half = (grid.width / 2) as i32;
    grid.get((x + half) as usize, (z + half) as usize)
}

/// The floor of hand-made ground that holds no gully and no lake's bed: the ground itself.
fn on_ground(heights: &Grid<f32>) -> impl Fn(&ChannelPoint) -> f32 + '_ {
    move |point| ground_m(heights, point)
}

/// Drawn lake `drawn` at `level_m` (the land model's level too) on `grid`, laid out as `ground`
/// lays it out: its water the cells at whose world `(x, z)` `water` holds, its held bed those at
/// which `held_bed` holds and `water` does not, each in index order. Its centre and area are not
/// read.
fn lake_where(grid: &Grid<f32>, drawn: usize, level_m: f32, water: impl Fn(f64, f64) -> bool, held_bed: impl Fn(f64, f64) -> bool) -> LakeShape {
    let half = (grid.width / 2) as f64;
    let at = |cell: usize| ((cell % grid.width) as f64 - half, (cell / grid.width) as f64 - half);
    let cells: Vec<usize> = (0..grid.data.len()).filter(|&cell| water(at(cell).0, at(cell).1)).collect();
    let bed: Vec<usize> = (0..grid.data.len()).filter(|&cell| held_bed(at(cell).0, at(cell).1) && !water(at(cell).0, at(cell).1)).collect();
    LakeShape { drawn, level_m, x_m: 0.0, z_m: 0.0, area_m2: cells.len() as f32, cells, held_bed: bed }
}

/// A straight channel from `from` to `to`, `width_m` wide, ending `end`, its points 2 m apart
/// (exactly, on whole metres, when the ends are 2 m apart on whole metres).
fn straight(from: (f64, f64), to: (f64, f64), width_m: f32, end: ChannelEnd) -> Channel {
    let steps = ((to.0 - from.0).hypot(to.1 - from.1) / 2.0).ceil() as usize;
    let points = (0..=steps)
        .map(|step| {
            let along = |from: f64, to: f64| from + (to - from) * step as f64 / steps as f64;
            ChannelPoint { x_m: along(from.0, to.0), z_m: along(from.1, to.1), width_m, drained_m2: 0.0 }
        })
        .collect();
    Channel { drawn: Some(0), from_lake: None, points, end }
}

/// The beds of `profile` under `rules`: `water_fill` of the depth below the surface.
fn beds(profile: &Profile, rules: &WaterRules) -> Vec<f32> {
    profile.surface_m.iter().zip(&profile.depth_m).map(|(surface, depth)| surface - rules.water_fill * depth).collect()
}

/// Whether `values` never rise from one to the next.
fn never_rise(values: &[f32]) -> bool {
    values.windows(2).all(|pair| pair[1] <= pair[0])
}

/// A shape for each of `channels` that changes nothing: no wander, and the deep line on the line.
fn unshaped(channels: &[Channel]) -> Vec<Shape> {
    (channels.iter())
        .map(|channel| Shape { factor: vec![1.0; channel.points.len()], deep_shift_m: vec![0.0; channel.points.len()] })
        .collect()
}

/// The water along `channels`, none of which wanders, as `profiles` works it out.
fn unshaped_profiles(channels: &[Channel], heights: &Grid<f32>, floor: &Floor, lake_levels: &[f32], rules: &WaterRules) -> Vec<Profile> {
    found_profiles(channels, &unshaped(channels), heights, floor, &[], lake_levels, rules)
}

/// The water along `channels`, as `profiles` works it out under a flag nobody sets.
fn found_profiles(channels: &[Channel], shapes: &[Shape], heights: &Grid<f32>, floor: &Floor, lakes: &[LakeShape], lake_levels: &[f32], rules: &WaterRules) -> Vec<Profile> {
    profiles(channels, shapes, heights, floor, lakes, lake_levels, rules, &AtomicBool::new(false)).expect("no flag is set").profiles
}

/// `worked_out` under a flag nobody sets.
fn worked(channels: &[Channel], shapes: &[Shape], heights: &Grid<f32>, floor: &Floor, reading: Reading, lake_levels: &[f32], rules: &WaterRules) -> WorkedOut {
    worked_out(channels, shapes, heights, floor, reading, lake_levels, rules, &AtomicBool::new(false)).expect("no flag is set")
}

/// `channel` shaped as `shape_channels` would shape it with `shape`: each point's width
/// multiplied by its factor.
fn shaped(channel: Channel, shape: &Shape) -> Channel {
    let points = (channel.points.iter().zip(&shape.factor))
        .map(|(point, &factor)| ChannelPoint { width_m: point.width_m * factor, ..*point })
        .collect();
    Channel { points, ..channel }
}

/// `heights` cut by `channels`, shaped by `shapes`, with no lake, and the water's depth.
fn cut_shaped(heights: &Grid<f32>, channels: &[Channel], shapes: &[Shape], rules: &WaterRules) -> (Grid<f32>, Grid<f32>) {
    let found = found_profiles(channels, shapes, heights, &on_ground(heights), &[], &[], rules);
    let (mut cut, mut depth) = (heights.clone(), Grid::new(heights.width, heights.height, 1.0));
    cut_beds(&mut cut, &mut depth, channels, shapes, &found, &[], rules);
    (cut, depth)
}

/// `heights` cut by `channels`, none of which wanders, with no lake, and the water's depth.
fn cut(heights: &Grid<f32>, channels: &[Channel], rules: &WaterRules) -> (Grid<f32>, Grid<f32>) {
    cut_shaped(heights, channels, &unshaped(channels), rules)
}

/// The share of the water's depth at the deep line that a bowl holds `across_m` from the line,
/// `half_width_m` its half width, its deep line on the line: `1 - (across / half width)³`.
fn bowl(across_m: f32, half_width_m: f32) -> f32 {
    1.0 - (across_m.abs() / half_width_m).powi(3)
}

fn close(value: f32, expected: f32) -> bool {
    (value - expected).abs() < 1e-4
}

/// The share of a bank's cut made `beyond_m` beyond the bed's edge, by the formula: the smooth
/// step from 1 at the edge to 0 at 12 m.
fn faded(beyond_m: f32) -> f32 {
    let reach = (beyond_m / 12.0).clamp(0.0, 1.0);
    1.0 - reach * reach * (3.0 - 2.0 * reach)
}

/// Where a bank of 1 in 2 leaves ground that lay at `was_m`, `beyond_m` beyond the waterline of
/// water whose surface is `surface_m`: the bank starts at the surface.
fn banked(was_m: f32, surface_m: f32, beyond_m: f32) -> f32 {
    was_m - (was_m - (surface_m + beyond_m / 2.0)).max(0.0) * faded(beyond_m)
}

#[test]
fn a_bed_is_as_deep_and_as_wide_as_the_rules_say() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    // From the west border, so it has its full depth from its first point.
    let channels = [straight((-64.0, 0.0), (50.0, 0.0), 10.0, ChannelEnd::Edge)];
    let [profile] = &unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    for (point, (&surface, &depth)) in channels[0].points.iter().zip(profile.surface_m.iter().zip(&profile.depth_m)) {
        assert!(close(depth, 1.2), "{depth} m deep at x = {}", point.x_m);
        assert!(close(surface, tilted(point.x_m) as f32 - 0.48), "the surface {surface} m at x = {}", point.x_m);
    }
    let (after, depth) = cut(&before, &channels, &rules);
    for x in -30..=30 {
        let surface = at(&before, (x, 0)) - 0.48;
        assert!(close(at(&after, (x, 0)), surface - 0.72), "at the line, x = {x}: {} m, the bed {} m", at(&after, (x, 0)), surface - 0.72);
        // A bowl from waterline to waterline, 5 m either side of the line: the water 0.72 m deep
        // at the line and `1 - (|z| / 5)³` of that across, none on the waterlines.
        for z in -5..=5 {
            let water = 0.72 * bowl(z as f32, 5.0);
            assert!(close(at(&after, (x, z)), surface - water), "({x}, {z}) lies at {} m, not {} m", at(&after, (x, z)), surface - water);
            assert!(close(at(&depth, (x, z)), water), "the water at ({x}, {z}) is {} m deep, not {water} m", at(&depth, (x, z)));
        }
        // The bank starts at the surface, 0.48 m below the ground, and rises 1 in 2: a metre
        // beyond the waterline it would stand above the ground. Beyond the waterline nothing is
        // cut, and nothing is under water.
        for z in (6..=20).flat_map(|z| [-z, z]) {
            assert_eq!(at(&after, (x, z)), at(&before, (x, z)), "({x}, {z}) was cut");
            assert_eq!(at(&depth, (x, z)), 0.0, "water at ({x}, {z}), beyond the waterline");
        }
    }
}

/// Flat ground 100 m high, 128 m a side, and a channel `width_m` wide along `z = 0` from the west
/// border, so deep from its first point: east from `x = -64` to `x = 60`, points 2 m apart.
fn on_the_flat(width_m: f32) -> (Grid<f32>, Channel) {
    (ground(128, |_, _| 100.0), straight((-64.0, 0.0), (60.0, 0.0), width_m, ChannelEnd::Edge))
}

/// The cells at `z` = -20 to 20 m in column `x` of `depth` that hold water.
fn wet_across(depth: &Grid<f32>, x: i32) -> Vec<i32> {
    (-20..=20).filter(|&z| at(depth, (x, z)) > 0.0).collect()
}

/// Asserts that the cells of column `x` that hold water run unbroken from waterline to
/// waterline of water `width_m` wide across the line `z = 0`: the outermost on each side lies
/// within a cell of its waterline.
fn assert_water_as_wide(name: &str, depth: &Grid<f32>, x: i32, width_m: f32) {
    let wet = wet_across(depth, x);
    let (Some(&north), Some(&south)) = (wet.first(), wet.last()) else { panic!("{name}: no water at x = {x}") };
    assert_eq!(wet.len() as i32, south - north + 1, "{name}: the water at x = {x} is broken: {wet:?}");
    for outermost in [-north, south] {
        let off_m = outermost as f32 - width_m / 2.0;
        assert!(off_m.abs() <= 1.0, "{name}: the water at x = {x} reaches {outermost} m from the line, {width_m} m wide");
    }
}

#[test]
fn a_bed_is_a_bowl_between_its_waterlines() {
    let rules = WaterRules::default();
    let (before, channel) = on_the_flat(16.0);
    // 16 m wide, its bed 1.92 m deep: the water 0.6 of that, 1.152 m, at the deep line.
    let deep_m = rules.water_fill * bed_depth_m(16.0, &rules);
    assert!(close(deep_m, 1.152));
    let (_, depth) = cut(&before, &[channel], &rules);
    for x in -40..=40 {
        let line = at(&depth, (x, 0));
        assert!((line - deep_m).abs() <= 0.01, "the water at the line, x = {x}, is {line} m deep, not {deep_m} m");
        for z in [-4, 4] {
            let (water, expected) = (at(&depth, (x, z)), (1.0 - 0.5_f32.powi(3)) * deep_m);
            assert!((water - expected).abs() <= 0.02, "the water at ({x}, {z}) is {water} m deep, not {expected} m");
        }
        for z in [-9, 9] {
            assert_eq!(at(&depth, (x, z)), 0.0, "water at ({x}, {z}), beyond the waterline");
        }
        assert_water_as_wide("16 m", &depth, x, 16.0);
    }
}

#[test]
fn a_wide_stretch_is_shallow_and_a_narrow_one_deep() {
    let rules = WaterRules::default();
    let (before, channel) = on_the_flat(16.0);
    // Widened by a quarter west of x = 0, narrowed to 0.8 from there on.
    let factor: Vec<f32> = channel.points.iter().map(|point| if point.x_m < 0.0 { 1.25 } else { 0.8 }).collect();
    let shape = Shape { deep_shift_m: vec![0.0; factor.len()], factor };
    let channels = [shaped(channel, &shape)];
    let shapes = [shape];
    let unwandered_m = rules.water_fill * bed_depth_m(16.0, &rules);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &[], &[], &rules);
    let (after, depth) = cut_shaped(&before, &channels, &shapes, &rules);
    let surface_m = found[0].surface_m[0];
    assert!(found[0].surface_m.iter().all(|&surface| close(surface, surface_m)), "the surface changes: {:?}", found[0].surface_m);
    assert!(close(surface_m, 100.0 - (1.0 - rules.water_fill) * bed_depth_m(16.0, &rules)), "the surface lies at {surface_m} m");
    for (name, xs, width_m, factor) in [("wide", -50..=-20, 20.0, 1.25_f32), ("narrow", 20..=50, 12.8, 0.8)] {
        for x in xs {
            let (line, expected) = (at(&depth, (x, 0)), unwandered_m / factor);
            assert!((line - expected).abs() <= 0.03, "{name}: the water at the line, x = {x}, is {line} m deep, not {expected} m");
            for z in wet_across(&depth, x) {
                let water_m = at(&after, (x, z)) + at(&depth, (x, z));
                assert!(close(water_m, surface_m), "{name}: the water at ({x}, {z}) stands at {water_m} m, not {surface_m} m");
            }
            assert_water_as_wide(name, &depth, x, width_m);
        }
    }
}

#[test]
fn the_deep_water_lies_where_the_shift_points() {
    let rules = WaterRules::default();
    let before = ground(128, |_, _| 100.0);
    // East and west along z = 0, both from a border. Flowing along `(dx, dz)`, a positive shift
    // points along `(dz, -dx)`: north (-z) flowing east, south (+z) flowing west.
    let east = straight((-64.0, 0.0), (60.0, 0.0), 16.0, ChannelEnd::Edge);
    let west = straight((60.0, 0.0), (-64.0, 0.0), 16.0, ChannelEnd::Edge);
    for (name, channel, shift_m, toward) in [("east, +2.4 m", &east, 2.4, -1), ("east, -2.4 m", &east, -2.4, 1), ("west, +2.4 m", &west, 2.4, 1)] {
        let shapes = [Shape { factor: vec![1.0; channel.points.len()], deep_shift_m: vec![shift_m; channel.points.len()] }];
        let (_, depth) = cut_shaped(&before, std::slice::from_ref(channel), &shapes, &rules);
        for x in -40..=40 {
            let across: Vec<(i32, f32)> = (-12..=12).map(|z| (z, at(&depth, (x, z)))).collect();
            let (deepest, _) = across.iter().copied().fold((0, f32::NEG_INFINITY), |most, seen| if seen.1 > most.1 { seen } else { most });
            assert!(deepest.signum() == toward && (2..=3).contains(&deepest.abs()), "{name}: at x = {x} the deepest cell lies at z = {deepest}");
            let (near, far) = (at(&depth, (x, 6 * toward)), at(&depth, (x, -6 * toward)));
            assert!(near > far, "{name}: at x = {x} the water 6 m toward the shift is {near} m deep, on the other side {far} m");
        }
    }
}

/// The deep line's side end to end: on a channel 16 m wide along a half circle of three widths'
/// radius, shaped by `shape_channels` (with its wander) and cut by `cut_beds`, the deepest water
/// across each cross-section, read along the radius through each point every 0.25 m from
/// waterline to waterline, lies farther from the centre than the line, turning either way.
#[test]
fn the_deepest_water_of_a_bend_lies_toward_its_outside() {
    let rules = WaterRules::default();
    let before = ground(192, |_, _| 100.0);
    let radius_m = 48.0;
    let (mut checked, mut least_m) = (0, f64::INFINITY);
    for turn in [std::f64::consts::PI, -std::f64::consts::PI] {
        // From the south-west of the centre round to the north-east, points at most 4 m apart.
        let steps = (turn.abs() * radius_m / 4.0).ceil() as usize;
        let from = 0.75 * std::f64::consts::PI;
        let points = (0..=steps)
            .map(|k| {
                let angle = from + turn * k as f64 / steps as f64;
                ChannelPoint { x_m: radius_m * angle.cos(), z_m: radius_m * angle.sin(), width_m: 16.0, drained_m2: 0.0 }
            })
            .collect();
        let mut channels = vec![Channel { drawn: Some(0), from_lake: None, points, end: ChannelEnd::Edge }];
        let shapes = shape_channels(&mut channels, 1, &rules);
        let (_, depth) = cut_shaped(&before, &channels, &shapes, &rules);
        let half = (before.width / 2) as f64;
        let depth_at = |(x, z): (f64, f64)| depth.get((x + half).round() as usize, (z + half).round() as usize);
        // Away from the ends, where the spring grows into its bed and the bend is measured short.
        for (k, point) in channels[0].points.iter().enumerate().filter(|(k, _)| (5..=steps - 5).contains(k)) {
            let (out_x, out_z) = (point.x_m / radius_m, point.z_m / radius_m);
            let half_width_m = f64::from(point.width_m) / 2.0;
            let samples = (-(4.0 * half_width_m) as i32..=(4.0 * half_width_m) as i32).map(|step| f64::from(step) * 0.25);
            let (deepest_m, _) = samples
                .map(|off_m| (off_m, depth_at((point.x_m + off_m * out_x, point.z_m + off_m * out_z))))
                .fold((0.0, f32::NEG_INFINITY), |most, seen| if seen.1 > most.1 { seen } else { most });
            checked += 1;
            least_m = least_m.min(deepest_m);
            assert!(deepest_m > 0.0, "turning {turn}: at point {k} the deepest water lies {deepest_m} m outward of the line");
        }
    }
    assert!(checked >= 50, "only {checked} cross-sections were read (the deepest at least {least_m} m outward)");
}

#[test]
fn a_bed_stays_within_its_depths() {
    let rules = WaterRules::default();
    // 24 m wide before the wander, its bed 2.88 m deep by the share, held at the deepest,
    // 2.5 m, and narrowed to half; 2 m wide, 0.24 m, held at the shallowest, 0.3 m, and widened
    // by half. Neither moves past its limit.
    for (width_m, factor, limit_m) in [(24.0, 0.5, rules.bed_depth_max_m), (2.0, 1.5, rules.bed_depth_min_m)] {
        let (before, channel) = on_the_flat(width_m);
        let shape = Shape { factor: vec![factor; channel.points.len()], deep_shift_m: vec![0.0; channel.points.len()] };
        let channels = [shaped(channel, &shape)];
        let shapes = [shape];
        let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &[], &[], &rules);
        assert!(found[0].depth_m.iter().all(|&depth| depth == limit_m), "{width_m} m by {factor}: the bed's depths {:?}", found[0].depth_m);
        let (after, depth) = cut_shaped(&before, &channels, &shapes, &rules);
        for x in -40..=40 {
            let (lowered, water) = (at(&before, (x, 0)) - at(&after, (x, 0)), at(&depth, (x, 0)));
            let (least, most) = (rules.bed_depth_min_m, rules.bed_depth_max_m);
            assert!((least - 1e-4..=most + 1e-4).contains(&lowered), "{width_m} m by {factor}: the line at x = {x} is lowered by {lowered} m");
            let (least, most) = (rules.water_fill * least, rules.water_fill * most);
            assert!((least - 1e-4..=most + 1e-4).contains(&water), "{width_m} m by {factor}: the water at the line, x = {x}, is {water} m deep");
        }
    }
}

#[test]
fn a_narrow_brook_holds_water() {
    let rules = WaterRules::default();
    let before = ground(128, |_, _| 100.0);
    for width_m in [0.2, 1.2] {
        // Along a row of cells, through their middles, 0.25 m and 0.4 m off them, and exactly
        // half way between two rows, from the west border: every cell holding a point of the
        // line, read every 0.25 m, holds water.
        let mut lines = Vec::new();
        for off_m in [0.0, 0.25, 0.4, 0.5] {
            let channel = straight((-64.0, off_m), (60.0, off_m), width_m, ChannelEnd::Edge);
            let (_, depth) = cut(&before, std::slice::from_ref(&channel), &rules);
            for step in -200..=200 {
                let (x, z) = ((f64::from(step) * 0.25).round() as i32, off_m.round() as i32);
                let water = at(&depth, (x, z));
                assert!(water > 0.05, "{width_m} m wide, {off_m} m off the row: the water at ({x}, {z}) is {water} m deep");
            }
            lines.push((format!("{off_m} m off the row"), channel, depth));
        }
        // At 45 degrees, from the south-west border, through no cell's middle.
        let diagonal = straight((-60.0, -59.7), (60.0, 60.3), width_m, ChannelEnd::Edge);
        let (_, depth) = cut(&before, std::slice::from_ref(&diagonal), &rules);
        lines.push(("at 45 degrees".to_string(), diagonal, depth));
        // On each line, every cell whose middle lies within 0.5 m of it holds water.
        for (name, channel, depth) in &lines {
            let mut near = 0;
            for z in -45..=45 {
                for x in -45..=45 {
                    if from_line_m(&channel.points, (f64::from(x), f64::from(z))).0 <= 0.5 {
                        near += 1;
                        let water = at(depth, (x, z));
                        assert!(water > 0.05, "{width_m} m wide, {name}: the water at ({x}, {z}) is {water} m deep");
                    }
                }
            }
            assert!(near >= 90, "{width_m} m wide, {name}: only {near} cells lie within 0.5 m of the line");
        }
    }
}

#[test]
fn a_bed_never_rises_downstream() {
    let rules = WaterRules::default();
    // A bump 3 m high and 12 m across lies across the channel's path at x = 0.
    let bump = |x: f64| 3.0 * (1.0 - x.abs() / 6.0).max(0.0);
    let before = ground(128, |x, _| tilted(x) + bump(x));
    let channels = [straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge)];
    let [profile] = &unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    assert!(never_rise(&profile.surface_m), "the surface rises: {:?}", profile.surface_m);
    assert!(never_rise(&beds(profile, &rules)), "the bed rises: {:?}", beds(profile, &rules));
    let (after, _) = cut(&before, &channels, &rules);
    let line: Vec<f32> = (-50..=50).map(|x| at(&after, (x, 0))).collect();
    assert!(never_rise(&line), "the ground along the line rises: {line:?}");
    // Cut through: the crest lies no higher than the bed at the bump's upstream foot.
    let foot = beds(profile, &rules)[22];
    assert_eq!(channels[0].points[22].x_m, -6.0);
    assert!(at(&after, (0, 0)) <= foot, "the crest lies at {} m, above the bed at its foot, {foot} m", at(&after, (0, 0)));
}

/// Asserts that `profile`, the water along `channel`, lies at exactly `end_m` at every point no
/// farther than `inside_m` from its last along it (across the water it ends on), and that its
/// surface and bed never rise. `free` is the water along the same channel ending nowhere: where
/// that lies above `end_m` at the first of those points, the step between them is taken off its
/// surfaces upstream less 0.05 m per metre of their distance beyond `inside_m` from the end, as
/// far as that is above 0; where it lies below, its surfaces are raised to `end_m`.
fn assert_comes_down_to(
    name: &str,
    channel: &Channel,
    profile: &Profile,
    free: &Profile,
    end_m: f32,
    inside_m: f64,
    rules: &WaterRules,
) {
    let to_end_m = |k: usize| {
        channel.points[k..].windows(2).map(|pair| (pair[1].x_m - pair[0].x_m).hypot(pair[1].z_m - pair[0].z_m)).sum::<f64>()
    };
    let entry = (0..channel.points.len()).find(|&k| to_end_m(k) <= inside_m).expect("the last point is inside");
    for k in entry..channel.points.len() {
        assert_eq!(profile.surface_m[k], end_m, "{name}: the surface at point {k}, {} m from the end", to_end_m(k));
    }
    let step_m = free.surface_m[entry] - end_m;
    for (k, &surface) in profile.surface_m.iter().enumerate() {
        let outside_m = (to_end_m(k) - inside_m).max(0.0) as f32;
        let expected = (free.surface_m[k] - (step_m - 0.05 * outside_m).max(0.0)).max(end_m);
        assert!((surface - expected).abs() < 1e-4, "{name}: {surface} m at point {k}, not {expected} m");
    }
    assert!(never_rise(&profile.surface_m), "{name}: the surface rises: {:?}", profile.surface_m);
    assert!(never_rise(&beds(profile, rules)), "{name}: the bed rises");
}

/// Ground falling 2 % to the east and 1 % to the south, 128 m a side, with a trench `depth_m`
/// deep and 5 m wide along `x = 0`, from `x = -2` to `2`, that stops 8 m short of `z = 0`: the
/// ground of `a_brook_never_climbs_to_join`.
fn in_a_trench(depth_m: f64) -> Grid<f32> {
    ground(128, |x, z| tilted(x) - 0.01 * z - if x.abs() <= 2.0 && z <= -8.0 { depth_m } else { 0.0 })
}

/// A brook 1.2 m wide down from the north along `x = 0`, from (0, -50) to (0, 0), ending on the
/// river listed after it, and that river, 8 m wide, east along `z = 0` from (-50, 0) to (50, 0);
/// and the index of the river's point at (0, 0), where the brook joins it.
fn brook_and_river() -> ([Channel; 2], usize) {
    let brook = Channel { drawn: None, ..straight((0.0, -50.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let junction = river.points.iter().position(|point| (point.x_m, point.z_m) == (0.0, 0.0)).expect("a river point at the junction");
    ([brook, river], junction)
}

/// `in_a_trench`, the trench as deep as makes the brook of `brook_and_river` short by `short_m`
/// where it joins the river, `river` (which ends at `junction`): the lowest cell of its banks' foot
/// outside the river's bed and bank, the trench's at (2, -8), at 100.04 m less the depth, lies
/// `short_m` less `FREEBOARD_M` below the river's surface at the junction, with no junction
/// bringing the river down; so they hold the brook `short_m` below it. With that surface, `e`.
fn trench_short_by(river: &Channel, junction: usize, short_m: f32) -> (Grid<f32>, f32) {
    let rules = WaterRules::default();
    let slope = in_a_trench(0.0);
    let e_m = unshaped_profiles(std::slice::from_ref(river), &slope, &on_ground(&slope), &[], &rules)[0].surface_m[junction];
    let depth_m = 100.04 - (f64::from(e_m) - f64::from(short_m) + f64::from(FREEBOARD_M));
    (in_a_trench(depth_m), e_m)
}

#[test]
fn a_brook_never_climbs_to_join() {
    let rules = WaterRules::default();
    // The river runs east along z = 0; the brook comes down from the north. On the first ground
    // it runs in a trench 3 m deep that stops 8 m short of the river, so its valley and the foot
    // of its banks lie more than a metre below the river where they meet: it climbs only as far
    // as they hold it, and the river comes down to it. On the second it runs on the open slope
    // above the river and follows the bank the river cut down to it, so the grade toward the
    // junction has next to nothing to take off, and the river stays as it is alone.
    let grounds = [("in a trench", in_a_trench(3.0)), ("on the slope", in_a_trench(0.0))];
    for (name, before) in grounds {
        // The brook is listed first and ends on the river, 8 m wide, listed after it.
        let ([brook, river], junction) = brook_and_river();
        let found = unshaped_profiles(&[brook.clone(), river.clone()], &before, &on_ground(&before), &[], &rules);
        let joined = found[1].surface_m[junction];
        let river_alone = unshaped_profiles(std::slice::from_ref(&river), &before, &on_ground(&before), &[], &rules);
        // The brook alone on the ground as the river, worked out before it, has cut it. Worked
        // out first, the river is held off no foot.
        let (mut cut_ground, held) = (before.clone(), vec![f32::NEG_INFINITY; before.data.len()]);
        for_each_lowering(&before, &held, &river, &unshaped(std::slice::from_ref(&river))[0], &found[1], &rules, |cell, lowered_m| {
            cut_ground.data[cell] = cut_ground.data[cell].min(lowered_m);
        });
        let alone = unshaped_profiles(&[Channel { end: ChannelEnd::Edge, ..brook.clone() }], &cut_ground, &on_ground(&before), &[], &rules);
        assert_comes_down_to(name, &brook, &found[0], &alone[0], joined, 4.0, &rules);
        let entry = entry_into(&brook, &river, &found[1]);
        if name == "in a trench" {
            // Its banks hold it at the trench's lowest foot cell, at (2, -8), less 0.1 m, more
            // than a metre below the river alone: the river comes down there, and the brook
            // alone stands no more than 0.3 m below it just before its entry point.
            let banks_m = at(&before, (2, -8)) - FREEBOARD_M;
            assert!(river_alone[0].surface_m[junction] - banks_m > 1.0, "{name}: the banks hold the brook at {banks_m} m");
            assert!((joined - banks_m).abs() < 1e-3, "{name}: the river stands at {joined} m at the junction, its banks holding the brook at {banks_m} m");
            let above_m = alone[0].surface_m[entry - 1] - joined;
            assert!(above_m.abs() <= ARRIVAL_ALLOWANCE_M, "{name}: alone, the brook would stand {above_m} m above the river before its entry point {entry}");
        } else {
            // On the slope it arrives at or below the river's surface, by no more than 0.5 m,
            // where the river's cut bank holds it: no junction brings the river down.
            assert_eq!(found[1], river_alone[0], "{name}: the river moved");
            let above_m = alone[0].surface_m[entry] - joined;
            assert!((-0.5..=0.0).contains(&above_m), "{name}: alone, the brook would stand {above_m} m above the river at its entry point {entry}");
        }
    }
}

#[test]
fn a_channel_standing_far_above_its_floor_is_said() {
    let rules = WaterRules::default();
    // The grounds of `a_brook_never_climbs_to_join`: in a trench that stops 8 m short of the
    // river, a brook's own valley lies more than a metre below the river where they meet, and
    // the river comes down to it; on the open slope it comes in above the river.
    let (in_a_trench, on_the_slope) = (in_a_trench(3.0), in_a_trench(0.0));
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    // Two brooks down the same line, and that line drawn as river 2, all ending on the river.
    let brook = Channel { drawn: None, ..straight((0.0, -50.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(3)) };
    let drawn = Channel { drawn: Some(1), ..brook.clone() };
    let channels = [brook.clone(), drawn, brook, river];
    let worked_on = |heights: &Grid<f32>, floor: &Grid<f32>| {
        profiles(&channels, &unshaped(&channels), heights, &on_ground(floor), &[], &[], &rules, &AtomicBool::new(false)).expect("no flag is set")
    };

    // How far a channel's surface stands at most above the floor `grid` holds; its points lie
    // on whole metres, on cells.
    let above_m = |grid: &Grid<f32>, channel: &Channel, profile: &Profile| {
        (channel.points.iter().zip(&profile.surface_m))
            .map(|(point, &surface)| surface - at(grid, (point.x_m as i32, point.z_m as i32)))
            .fold(0.0, f32::max)
    };

    // The trench in the land grown: no brook is raised above its floor, nor is the river, which
    // comes down to them, and nothing is said of a valley; what is said is that river 1 (the
    // river, drawn first) was cut 2.7 m deeper where river 2 joins it, and up to that where the
    // two brooks do, in one note for both.
    let alone = unshaped_profiles(&channels[3..], &in_a_trench, &on_ground(&in_a_trench), &[], &rules);
    let worked = worked_on(&in_a_trench, &in_a_trench);
    let highs: Vec<f32> = worked.profiles.iter().map(|profile| profile.above_floor_m).collect();
    assert!(highs.iter().all(|&high_m| high_m == 0.0), "{highs:?}");
    assert!(above_floor_notes(&channels, &worked.profiles).is_empty());
    let deeper_m = alone[0].surface_m[25] - worked.profiles[3].surface_m[25];
    assert!((deeper_m - 2.7).abs() < 0.05, "the river was cut {deeper_m} m deeper at the junction");
    assert_eq!(
        worked.notes,
        [
            (About::River(0), format!("River 1 was cut {deeper_m:.1} m deeper where river 2 joins it.")),
            (About::River(0), format!("River 1 was cut up to {deeper_m:.1} m deeper where 2 brooks join it."))
        ]
    );

    // A trench 5.32 m deep in the land grown, its banks holding the brooks 5 m short: the river
    // comes down 3 m at most, so the brooks are raised inside their beds, standing up to about
    // 1.9 m above their floor, the trench's ground under each point, and that is said: river 2 by
    // its number, the two brooks together. The river stands above no floor. The same trench cut
    // into the 1 m ground alone, a gully the land grown does not hold: the same water, and it
    // stands above no floor, as the floor is the land grown, not the 1 m ground.
    let (deep, _) = trench_short_by(&channels[3], 25, 5.0);
    let held_up = worked_on(&deep, &deep);
    let high_m = held_up.profiles[0].above_floor_m;
    assert!((1.5..2.5).contains(&high_m), "the brook stands {high_m} m above its floor");
    assert_eq!(held_up.profiles.iter().map(|profile| profile.above_floor_m).collect::<Vec<f32>>(), [high_m, high_m, high_m, 0.0]);
    for (channel, profile) in channels.iter().zip(&held_up.profiles) {
        assert_eq!(profile.above_floor_m, above_m(&deep, channel, profile));
    }
    assert_eq!(
        above_floor_notes(&channels, &held_up.profiles),
        [(About::River(1), format!("River 2 stands up to {high_m:.1} m above its valley.")), (About::Land, format!("2 brooks stand up to {high_m:.1} m above their valleys."))]
    );
    assert_eq!(above_floor_notes(&channels[2..], &held_up.profiles[2..]), [(About::Land, format!("1 brook stands up to {high_m:.1} m above its valley."))]);
    let deep_gully = worked_on(&deep, &on_the_slope);
    assert_eq!(deep_gully.profiles.iter().map(|profile| &profile.surface_m).collect::<Vec<_>>(), held_up.profiles.iter().map(|profile| &profile.surface_m).collect::<Vec<_>>());
    assert!(deep_gully.profiles.iter().all(|profile| profile.above_floor_m == 0.0));
    assert!(above_floor_notes(&channels, &deep_gully.profiles).is_empty());

    // The same trench cut into the 1 m ground alone, a gully the land grown does not hold: the
    // river comes down as far, and nothing stands above its floor either.
    let gully = worked_on(&in_a_trench, &on_the_slope);
    assert_eq!(gully.profiles.iter().map(|profile| &profile.surface_m).collect::<Vec<_>>(), worked.profiles.iter().map(|profile| &profile.surface_m).collect::<Vec<_>>());
    assert!(gully.profiles.iter().all(|profile| profile.above_floor_m == 0.0));
    assert!(above_floor_notes(&channels, &gully.profiles).is_empty());
    assert_eq!(gully.notes, worked.notes);

    // On the open slope no water stands above its floor, nothing comes down, and nothing is said.
    let slope = worked_on(&on_the_slope, &on_the_slope);
    let highs: Vec<f32> = slope.profiles.iter().map(|profile| profile.above_floor_m).collect();
    assert!(highs.iter().all(|&high_m| high_m == 0.0), "{highs:?}");
    assert!(above_floor_notes(&channels, &slope.profiles).is_empty());
    assert!(slope.notes.is_empty(), "{:?}", slope.notes);

    // How a channel standing above its floor is said: each drawn river so by its number, then the
    // brooks so together, the most of each to one decimal; 0.5 m is not said, a little more is.
    let standing = |above_floor_m: f32| Profile { above_floor_m, ..slope.profiles[0].clone() };
    let high = [standing(1.24), standing(1.24), standing(1.31), standing(0.0)];
    assert_eq!(
        above_floor_notes(&channels, &high),
        said(&[(About::River(1), "River 2 stands up to 1.2 m above its valley."), (About::Land, "2 brooks stand up to 1.3 m above their valleys.")])
    );
    assert_eq!(above_floor_notes(&channels[2..], &high[2..]), said(&[(About::Land, "1 brook stands up to 1.3 m above its valley.")]));
    assert!(above_floor_notes(&channels[..1], &[standing(0.5)]).is_empty());
    assert_eq!(above_floor_notes(&channels[..1], &[standing(0.54)]), said(&[(About::Land, "1 brook stands up to 0.5 m above its valley.")]));
}

/// A brook whose banks hold it 1 m below the river where it joins brings the river down 1 m
/// there: from the cap's point, the first of the stretch nearest the brook's entry point (the
/// river's point at x = -2), on, the river stands at the lower of its own surface (alone) and
/// what the brook's banks hold, and never rises downstream; the brook ends at the river's
/// surface. It takes two passes: the second asks for no new cap.
#[test]
fn a_brook_whose_banks_hold_1_m_less_brings_its_river_down_1_m() {
    let rules = WaterRules::default();
    let (channels, junction) = brook_and_river();
    let (before, e_m) = trench_short_by(&channels[1], junction, 1.0);
    let banks_m = at(&before, (2, -8)) - FREEBOARD_M;
    assert!((e_m - banks_m - 1.0).abs() < 1e-3, "the banks hold the brook at {banks_m} m, the river standing at {e_m} m");
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    let alone = unshaped_profiles(&channels[1..], &before, &on_ground(&before), &[], &rules);
    let (brook, river) = (&worked.profiles[0], &worked.profiles[1]);
    assert_eq!(worked.passes, 2, "passes");
    assert!((river.surface_m[junction] - banks_m).abs() < 1e-3, "the river stands at {} m at the junction, not at {banks_m} m", river.surface_m[junction]);
    let cap_m = river.surface_m[junction];
    for k in junction - 1..channels[1].points.len() {
        let expected = alone[0].surface_m[k].min(cap_m);
        assert!((river.surface_m[k] - expected).abs() < 1e-4, "point {k}: the river at {} m, not {expected} m", river.surface_m[k]);
    }
    assert!(never_rise(&river.surface_m), "the river rises: {:?}", river.surface_m);
    assert_eq!(brook.surface_m[brook.surface_m.len() - 1], cap_m, "the brook's last surface");
}

/// Upstream of a junction the river comes down to its cap along its own fall plus the junction
/// grade, as a tributary arriving high comes down to its river, not in a slot to its head: in
/// `a_brook_whose_banks_hold_1_m_less_brings_its_river_down_1_m`, each surface upstream of the
/// cap's point is the lower of its own (alone) and the cap plus 0.05 m per metre of its distance to
/// that point plus how far its own surface lies above its own there; so the points more than the
/// lowering at that point over 0.05 m upstream of it, the head among them, keep their surface bit
/// for bit, and only those nearer come down.
#[test]
fn upstream_of_a_junction_the_river_comes_down_along_its_own_fall_and_the_grade() {
    let rules = WaterRules::default();
    let (channels, junction) = brook_and_river();
    let (before, _) = trench_short_by(&channels[1], junction, 1.0);
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    let alone = &unshaped_profiles(&channels[1..], &before, &on_ground(&before), &[], &rules)[0].surface_m;
    let (river, at) = (&found[1].surface_m, junction - 1);
    let (cap_m, along_at_m) = (river[at], channel_along_m(&channels[1], at));
    let lowered_m = alone[at] - cap_m;
    assert!(lowered_m > 1.0, "the river came down {lowered_m} m at the cap's point");
    let (mut kept, mut lowered) = (0, 0);
    for k in 0..at {
        let to_m = along_at_m - channel_along_m(&channels[1], k);
        let line_m = (f64::from(cap_m) + 0.05 * to_m + f64::from(alone[k] - alone[at])) as f32;
        let expected = alone[k].min(line_m);
        assert!((river[k] - expected).abs() < 1e-4, "point {k}, {to_m} m upstream of the cap's point: {} m, not {expected} m", river[k]);
        if to_m > f64::from(lowered_m) / 0.05 {
            assert_eq!(river[k], alone[k], "point {k}, {to_m} m upstream of the cap's point, moved");
            kept += 1;
        } else {
            lowered += usize::from(river[k] < alone[k]);
        }
    }
    assert_eq!(river[0], alone[0], "the head moved");
    assert!(kept > 10 && (5..=12).contains(&lowered), "{kept} points kept their surface and {lowered} came down");
}

/// An ordinary mouth on a river falling 10 % caps nothing: a brook comes in slanting against the
/// river's flow, close along its bank, where the river's cut bank holds the brook's water below the
/// river's surface at its entry point, by more than `ARRIVAL_ALLOWANCE_M` (the brook alone, ending
/// nowhere, on the ground as the river cut it, just before its entry point). The foot of its banks
/// there is the river's own bank, at or above the river's surface beside it, and holds it: the
/// water is worked out in one pass, and the river is as it is alone, bit for bit.
#[test]
fn an_ordinary_mouth_on_a_river_falling_10_percent_caps_nothing() {
    let rules = WaterRules::default();
    let before = ground(128, |x, z| 100.0 - 0.1 * x - 0.01 * z);
    let brook = Channel { drawn: None, ..straight((40.0, -12.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let channels = [brook.clone(), river.clone()];
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    let alone = unshaped_profiles(std::slice::from_ref(&river), &before, &on_ground(&before), &[], &rules);
    let (mut cut_ground, held) = (before.clone(), vec![f32::NEG_INFINITY; before.data.len()]);
    for_each_lowering(&before, &held, &river, &unshaped(std::slice::from_ref(&river))[0], &alone[0], &rules, |cell, lowered_m| {
        cut_ground.data[cell] = cut_ground.data[cell].min(lowered_m);
    });
    let brook_alone = unshaped_profiles(&[Channel { end: ChannelEnd::Edge, ..brook.clone() }], &cut_ground, &on_ground(&before), &[], &rules);
    let entry = entry_into(&brook, &river, &alone[0]);
    let (_, _, e_m) = nearest_water(&river, &alone[0], &brook.points[entry]);
    let below_m = e_m - brook_alone[0].surface_m[entry - 1];
    assert!(below_m > ARRIVAL_ALLOWANCE_M, "alone, the brook stands {below_m} m below the river's surface before its entry point");
    assert_eq!(worked.passes, 1, "passes");
    assert_eq!(worked.profiles[1], alone[0], "the river moved");
}

/// A shortfall of 5 m brings the river down by `COME_DOWN_MOST_M`, 3 m, and leaves a step: the
/// brook, raised to the river's lowered surface, stands 2 m above what its banks hold, and both
/// are said, the river cut 3.0 m deeper and the brook joining it 2.0 m above its own banks.
#[test]
fn a_shortfall_of_5_m_brings_the_river_down_3_m_and_leaves_a_step_that_is_said() {
    let rules = WaterRules::default();
    let (channels, junction) = brook_and_river();
    let (before, e_m) = trench_short_by(&channels[1], junction, 5.0);
    let banks_m = at(&before, (2, -8)) - FREEBOARD_M;
    let worked = profiles(&channels, &unshaped(&channels), &before, &on_ground(&before), &[], &[], &rules, &AtomicBool::new(false)).expect("no flag is set");
    let (brook, river) = (&worked.profiles[0], &worked.profiles[1]);
    assert!((river.surface_m[junction] - (e_m - 3.0)).abs() < 1e-3, "the river stands at {} m at the junction, not 3 m below {e_m} m", river.surface_m[junction]);
    assert!(never_rise(&river.surface_m), "the river rises: {:?}", river.surface_m);
    let entry = entry_into(&channels[0], &channels[1], river);
    let step_m = brook.surface_m[entry - 1] - banks_m;
    assert!((step_m - 2.0).abs() < 1e-3, "the brook stands {step_m} m above what its banks hold before its entry point");
    assert_eq!(
        worked.notes,
        said(&[(About::River(0), "River 1 was cut 3.0 m deeper where a brook joins it."), (About::River(0), "A brook joins river 1 2.0 m above its own banks; the water steps there.")])
    );
}

/// The notes on a junction name a river as the other water notes do, by its drawn number, and a
/// brook as a brook: in `a_shortfall_of_5_m_brings_the_river_down_3_m_and_leaves_a_step_that_is_said`,
/// with the tributary drawn as river 3 or a brook, and what it ends on, listed after it, drawn as
/// river 1 or a brook. Each is about the first drawn river it names, and about the land when it
/// names none.
#[test]
fn the_junction_notes_name_drawn_rivers_by_number_and_brooks_as_brooks() {
    let rules = WaterRules::default();
    let ([brook, river], junction) = brook_and_river();
    let (before, _) = trench_short_by(&river, junction, 5.0);
    let notes = |tributary: Option<usize>, ended_on: Option<usize>| {
        let channels = [Channel { drawn: tributary, ..brook.clone() }, Channel { drawn: ended_on, ..river.clone() }];
        profiles(&channels, &unshaped(&channels), &before, &on_ground(&before), &[], &[], &rules, &AtomicBool::new(false)).expect("no flag is set").notes
    };
    let steps = "2.0 m above its own banks; the water steps there.";
    let (one, three) = (About::River(0), About::River(2));
    assert_eq!(notes(Some(2), Some(0)), [(one, "River 1 was cut 3.0 m deeper where river 3 joins it.".to_string()), (three, format!("River 3 joins river 1 {steps}"))]);
    assert_eq!(notes(None, Some(0)), [(one, "River 1 was cut 3.0 m deeper where a brook joins it.".to_string()), (one, format!("A brook joins river 1 {steps}"))]);
    assert_eq!(notes(Some(2), None), [(three, "A brook was cut 3.0 m deeper where river 3 joins it.".to_string()), (three, format!("River 3 joins a brook {steps}"))]);
    assert_eq!(notes(None, None), [(About::Land, "A brook was cut 3.0 m deeper where a brook joins it.".to_string()), (About::Land, format!("A brook joins a brook {steps}"))]);
}

/// A channel drawn as river `drawn + 1` (a brook for `None`) that ends `end`; the notes read
/// nothing of its line.
fn named(drawn: Option<usize>, end: ChannelEnd) -> Channel {
    Channel { drawn, ..straight((0.0, 0.0), (2.0, 0.0), 1.2, end) }
}

/// The notes `junction_notes` gives on `channels` after a last pass in which each tributary of
/// `junctions`, `(index, deeper_m, step_m)`, brought the channel it ends on down `deeper_m` at its
/// entry point under its cap (none for 0) and arrives `step_m` below it, its banks holding it.
fn notes_of(channels: &[Channel], junctions: &[(usize, f32, f32)]) -> Vec<Note> {
    let (mut first, mut last, mut caps) = (vec![None; channels.len()], vec![None; channels.len()], vec![None; channels.len()]);
    for &(index, deeper_m, step_m) in junctions {
        let entry_m = 100.0 - deeper_m;
        first[index] = Some(Arrival { at: 0, entry_m: 100.0, arrival_m: 100.0 - step_m });
        last[index] = Some(Arrival { at: 0, entry_m, arrival_m: entry_m - step_m });
        caps[index] = (deeper_m > 0.0).then_some(Cap { at: 0, level_m: entry_m });
    }
    junction_notes(channels, &first, &last, &caps)
}

/// The notes on the junctions are gathered: the channels cut deeper first, the drawn rivers by
/// their number and then the brooks, each with one note for each drawn tributary, by its number,
/// and one for all the brooks that cut it, the most of them; then the steps left, in the same
/// order and gathered the same way. A junction is judged on its own before it is gathered, so one
/// cut 1 m deeper, or less, is in no note. River 2's tributary is listed first and the brooks of
/// river 1 among its drawn tributaries, as the order of the list does not set the order said.
/// Each note is about the first drawn river it names, the land when it names none.
#[test]
fn the_junction_notes_gather_the_brooks_and_name_the_drawn_rivers() {
    let edge = ChannelEnd::Edge;
    let on = ChannelEnd::Channel;
    let channels = [
        named(Some(1), edge),   // 0: river 2
        named(None, on(0)),     // 1: a brook on river 2
        named(Some(4), on(3)),  // 2: river 5 on river 1
        named(Some(0), edge),   // 3: river 1
        named(None, on(3)),     // 4: a brook on river 1
        named(Some(2), on(3)),  // 5: river 3 on river 1
        named(None, on(3)),     // 6: a brook on river 1
        named(None, on(3)),     // 7: a brook on river 1, cut too little to say
        named(None, edge),      // 8: a brook
        named(None, on(8)),     // 9: a brook on the brook
        named(Some(3), on(8)),  // 10: river 4 on the brook
        named(None, on(8)),     // 11: a brook on the brook
    ];
    let junctions = [(1, 1.3, 0.2), (2, 1.1, 0.5), (4, 2.0, 1.3), (5, 1.5, 1.2), (6, 3.0, 1.6), (7, 0.9, 0.2), (9, 1.2, 1.1), (10, 1.4, 1.9), (11, 2.2, 1.7)];
    assert_eq!(
        notes_of(&channels, &junctions),
        said(&[
            (About::River(0), "River 1 was cut 1.5 m deeper where river 3 joins it."),
            (About::River(0), "River 1 was cut 1.1 m deeper where river 5 joins it."),
            (About::River(0), "River 1 was cut up to 3.0 m deeper where 2 brooks join it."),
            (About::River(1), "River 2 was cut 1.3 m deeper where a brook joins it."),
            (About::River(3), "A brook was cut 1.4 m deeper where river 4 joins it."),
            (About::Land, "2 brooks were cut up to 2.2 m deeper where brooks join them."),
            (About::River(2), "River 3 joins river 1 1.2 m above its own banks; the water steps there."),
            (About::River(0), "2 brooks join river 1 up to 1.6 m above their own banks; the water steps there."),
            (About::River(3), "River 4 joins a brook 1.9 m above its own banks; the water steps there."),
            (About::Land, "2 brooks join brooks up to 1.7 m above their own banks; the water steps there."),
        ])
    );
}

/// One brook junction on a channel reads as a junction alone does, and the steps come after every
/// channel cut deeper: with one brook on river 1, one on the brook, and river 3 on river 1 cut too
/// little to say, each brook's notes name "a brook".
#[test]
fn one_brook_on_a_channel_is_said_as_a_brook() {
    let channels = [named(Some(0), ChannelEnd::Edge), named(None, ChannelEnd::Channel(0)), named(None, ChannelEnd::Edge), named(None, ChannelEnd::Channel(2)), named(Some(2), ChannelEnd::Channel(0))];
    let junctions = [(1, 2.0, 1.5), (3, 1.2, 1.2), (4, 0.5, 2.5)];
    assert_eq!(
        notes_of(&channels, &junctions),
        said(&[
            (About::River(0), "River 1 was cut 2.0 m deeper where a brook joins it."),
            (About::Land, "A brook was cut 1.2 m deeper where a brook joins it."),
            (About::River(2), "River 3 joins river 1 2.5 m above its own banks; the water steps there."),
            (About::River(0), "A brook joins river 1 1.5 m above its own banks; the water steps there."),
            (About::Land, "A brook joins a brook 1.2 m above its own banks; the water steps there."),
        ])
    );
}

/// A river that ends in a lake is not brought below the lake's level: the brook of
/// `brook_and_river`, short by 2.8 m where it joins, brings the river, which ends in a lake 1.5 m
/// below its surface at the junction, down to the lake's level and no lower, and the step left is
/// said, with the river's lowering.
#[test]
fn a_river_that_ends_in_a_lake_is_not_brought_below_it() {
    let rules = WaterRules::default();
    let ([brook, river], junction) = brook_and_river();
    let river = Channel { end: ChannelEnd::Lake(0), ..river };
    let (before, e_m) = trench_short_by(&river, junction, 2.8);
    let level_m = e_m - 1.5;
    let channels = [brook, river];
    let worked = profiles(&channels, &unshaped(&channels), &before, &on_ground(&before), &[], &[level_m], &rules, &AtomicBool::new(false)).expect("no flag is set");
    let river = &worked.profiles[1].surface_m;
    assert!(river.iter().all(|&surface| surface >= level_m - 1e-4), "the river comes down below the lake at {level_m} m: {river:?}");
    for (k, &surface) in river.iter().enumerate().skip(junction - 1) {
        assert!((surface - level_m).abs() < 1e-4, "point {k}: {surface} m, not the lake's level {level_m} m");
    }
    assert!(never_rise(river), "the river rises: {river:?}");
    assert_eq!(
        worked.notes,
        said(&[(About::River(0), "River 1 was cut 1.5 m deeper where a brook joins it."), (About::River(0), "A brook joins river 1 1.3 m above its own banks; the water steps there.")])
    );
}

/// A river out of a lake is not cut below the lake's level beside it: the course of
/// `out_of_a_trench`, out of a lake at 99 m along its stretch beside the lake to (-40, 0), and a
/// brook 1.2 m wide down from the north along `x = -38` in a slot 5 m wide whose floor, 97.9 m,
/// stops 4 m short of the course's line, so its banks hold it at 97.8 m, 1.1 m below the course
/// where it joins. The cap's point, the first of the stretch nearest the brook's entry point, is
/// the stretch's last, at (-40, 0): along the stretch the course stands at the level all the
/// same, and from the next point on at the cap.
#[test]
fn a_river_out_of_a_lake_is_not_cut_below_it_beside_the_lake() {
    let rules = WaterRules::default();
    let in_lake = |x: f64, z: f64| x <= -44.0 && z.abs() <= 20.0;
    let slot = |x: f64, z: f64| (x + 38.0).abs() <= 2.0 && z <= -4.0;
    let before = ground(128, |x, z| {
        if in_lake(x, z) || z.abs() <= 2.0 {
            98.0
        } else if slot(x, z) {
            97.9
        } else {
            100.0
        }
    });
    let lakes = [lake_where(&before, 0, 99.0, in_lake, |_, _| false)];
    let river = Channel { from_lake: Some(0), ..straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge) };
    let brook = Channel { drawn: None, ..straight((-38.0, -40.0), (-38.0, 0.0), 1.2, ChannelEnd::Channel(0)) };
    let channels = [river, brook];
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&lakes), &[99.0], &rules);
    assert_eq!(worked.read[0].left_m, 10.0, "where the course leaves the lake");
    assert_eq!(worked.read[0].caps, [Cap { at: 5, level_m: worked.read[0].caps[0].level_m }], "the caps on the course");
    let (river, cap_m) = (&worked.profiles[0].surface_m, worked.read[0].caps[0].level_m);
    assert!((cap_m - 97.8).abs() < 1e-4, "the cap at {cap_m} m");
    for (k, &surface) in river.iter().enumerate() {
        let along_m = channel_along_m(&channels[0], k);
        if along_m <= 10.0 {
            assert_eq!(surface, 99.0, "point {k}, {along_m} m along, beside the lake");
        } else {
            assert!(surface <= cap_m + 1e-4, "point {k}, {along_m} m along, at {surface} m above the cap");
        }
    }
    assert!(never_rise(river), "the course rises: {river:?}");
    assert_eq!(worked.profiles[1].surface_m.last(), Some(&river[6]), "the brook's last surface");
}

/// A river that begins on dry land 12 m above the junction keeps its head's ground: the brook of
/// `brook_and_river`, 1 m short, brings down a river that begins at (-12, 0), on dry land, and the
/// lowering at the cap's point, 10 m along, over 10 m is more than the junction grade. Upstream of
/// the cap's point the river is lowered by the lowering times its distance along over the cap's
/// point's: its head keeps its surface bit for bit, and it falls more steeply between, every
/// stretch more than alone.
#[test]
fn a_river_that_begins_12_m_above_the_junction_keeps_its_heads_ground() {
    let rules = WaterRules::default();
    let ([brook, river], _) = brook_and_river();
    let river = straight((-12.0, 0.0), (50.0, 0.0), 8.0, river.end);
    let junction = river.points.iter().position(|point| (point.x_m, point.z_m) == (0.0, 0.0)).expect("a river point at the junction");
    let (before, _) = trench_short_by(&river, junction, 1.0);
    let channels = [brook, river];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    let alone = &unshaped_profiles(&channels[1..], &before, &on_ground(&before), &[], &rules)[0].surface_m;
    assert!(found[1].on_dry_land, "the river begins on dry land");
    let (river, at) = (&found[1].surface_m, junction - 1);
    let along_at_m = channel_along_m(&channels[1], at);
    let lowered_m = alone[at] - river[at];
    assert!(f64::from(lowered_m) / along_at_m > 0.05, "the river came down {lowered_m} m, {along_at_m} m from its head");
    assert_eq!(river[0], alone[0], "the head moved");
    for k in 1..=at {
        let expected = (f64::from(alone[k]) - f64::from(lowered_m) * channel_along_m(&channels[1], k) / along_at_m) as f32;
        assert!((river[k] - expected).abs() < 1e-4, "point {k}: {} m, not {expected} m", river[k]);
        assert!(river[k - 1] - river[k] > alone[k - 1] - alone[k], "the stretch to point {k} falls no more steeply than alone");
    }
}

/// Three brooks in a chain end within `PASSES_MOST` passes: brook A comes down from the north along
/// `x = -30` and ends on brook B at (-30, -30); B comes from the west along `z = -30` and ends on
/// brook C at (0, -30); C comes down from the north along `x = 0` and ends on the river of
/// `brook_and_river`. Each runs in a level slot, so its banks hold it short where it joins, the
/// three by 1.2, 0.6 and 0.4 m below the river's surface where C joins it, which each brook,
/// raised, stands at. The caps go down the chain at once, each brook's arrival the lower of its
/// own and the cap on it: the river comes down 1.2 m, to A's, in two passes; no surface rises
/// downstream, and each brook ends at the surface of what it ends on.
#[test]
fn three_brooks_in_a_chain_end_within_the_passes() {
    let rules = WaterRules::default();
    let ([_, river], junction) = brook_and_river();
    let slope = in_a_trench(0.0);
    let e_m = f64::from(unshaped_profiles(std::slice::from_ref(&river), &slope, &on_ground(&slope), &[], &rules)[0].surface_m[junction]);
    // Each slot's floor, `short_m` less `FREEBOARD_M` below the river's surface.
    let floor = |short_m: f64| e_m - short_m + f64::from(FREEBOARD_M);
    let before = ground(128, |x, z| {
        if (x + 30.0).abs() <= 2.0 && z <= -34.0 {
            floor(1.2)
        } else if (z + 30.0).abs() <= 2.0 && (-62.0..=-4.0).contains(&x) {
            floor(0.6)
        } else if x.abs() <= 2.0 && z <= -8.0 {
            floor(0.4)
        } else {
            tilted(x) - 0.01 * z
        }
    });
    let brook = |from: (f64, f64), to: (f64, f64), end: usize| Channel { drawn: None, ..straight(from, to, 1.2, ChannelEnd::Channel(end)) };
    let channels = [brook((-30.0, -62.0), (-30.0, -30.0), 1), brook((-62.0, -30.0), (0.0, -30.0), 2), brook((0.0, -62.0), (0.0, 0.0), 3), river];
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    assert!(worked.passes <= PASSES_MOST, "{} passes", worked.passes);
    assert_eq!(worked.passes, 2, "passes");
    let river = &worked.profiles[3].surface_m;
    assert!((f64::from(river[junction]) - (e_m - 1.2)).abs() < 1e-3, "the river stands at {} m at the junction, not 1.2 m below {e_m} m", river[junction]);
    for (index, (channel, profile)) in channels.iter().zip(&worked.profiles).enumerate() {
        assert!(never_rise(&profile.surface_m), "channel {index} rises: {:?}", profile.surface_m);
        if let ChannelEnd::Channel(joined) = channel.end {
            let last = channel.points[channel.points.len() - 1];
            let (_, _, there_m) = nearest_water(&channels[joined], &worked.profiles[joined], &last);
            assert!((profile.surface_m[channel.points.len() - 1] - there_m).abs() < 1e-4, "channel {index} ends at {:?} m, on {there_m} m", profile.surface_m.last());
        }
    }
}

/// The foot of a channel's banks reads a cell under the water of a channel worked out before it,
/// but for the one it ends on, at that water's surface, as it reads a lake's cell at the lake's
/// level: on ground falling 2 % to the east, a brook 1.2 m wide runs east along `z = 5` from
/// (-20, 5) to the border beside a river 8 m wide along `z = 0`, 1 m beyond its waterline, so the
/// foot of the brook's banks reaches into the river's bed, whose bowl lies up to 0.33 m below the
/// river's surface there. Past its head's growth the brook stands 0.1 m below the river's water
/// beside it (its line's ground less the most its foot falls below it, the river's cells at the
/// river's surface, less `FREEBOARD_M`), not 0.1 m below the floor of the river's bed under it,
/// 0.43 m below the river's water.
#[test]
fn a_channels_banks_read_an_earlier_channels_bed_at_its_water() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let brook = Channel { drawn: None, ..straight((-20.0, 5.0), (62.0, 5.0), 1.2, ChannelEnd::Edge) };
    let channels = [river, brook];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    let mut checked = 0;
    for (k, (point, &surface)) in channels[1].points.iter().zip(&found[1].surface_m).enumerate() {
        if channel_along_m(&channels[1], k) < 12.0 || point.x_m > 48.0 {
            continue;
        }
        checked += 1;
        let (_, _, river_m) = nearest_water(&channels[0], &found[0], point);
        assert!((surface - (river_m - FREEBOARD_M)).abs() < 1e-3, "point {k} at x = {}: the brook at {surface} m, the river beside it at {river_m} m", point.x_m);
    }
    assert!(checked >= 20, "only {checked} points checked");
}

/// The water read in an earlier channel's bed is only the water that bed holds: none straight
/// behind the head of a channel that begins on dry land, where `cut_beds` puts none, though those
/// cells lie inside its half width. On a plateau 100 m high, a river 4 m wide begins at (0, 0) and
/// flows east; behind its head, in a pit 99 m deep, lie the four cells within its half width of
/// its first point, (-1, 0), (-1, ±1) and (-2, 0), dry. A brook 1.2 m wide, listed after it,
/// runs north along `x = -3` on the plateau, its foot reaching the pit. Beside the pit the brook
/// stands at least `FREEBOARD_M` below the pit's ground, as its banks hold it below every dry cell
/// of their foot, not 0.12 m below the plateau, 0.88 m above the dry pit beside it.
#[test]
fn a_channels_banks_read_no_water_behind_a_springs_head() {
    let rules = WaterRules::default();
    let pit = [(-1, 0), (-1, 1), (-1, -1), (-2, 0)];
    let before = ground(128, |x, z| if pit.contains(&(x as i32, z as i32)) { 99.0 } else { 100.0 });
    let river = straight((0.0, 0.0), (60.0, 0.0), 4.0, ChannelEnd::Edge);
    let brook = Channel { drawn: None, ..straight((-3.0, -40.0), (-3.0, 40.0), 1.2, ChannelEnd::Edge) };
    let channels = [river, brook];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    assert!(found[0].on_dry_land, "the river begins on dry land");
    let beside: Vec<(f64, f32)> = (channels[1].points.iter().zip(&found[1].surface_m)).filter(|(point, _)| point.z_m.abs() <= 2.0).map(|(point, &surface)| (point.z_m, surface)).collect();
    assert_eq!(beside.len(), 3, "the brook's points beside the pit");
    for (z, surface) in beside {
        assert!(surface <= 99.0 - FREEBOARD_M + 1e-4, "the brook stands at {surface} m at z = {z}, beside the dry pit at 99 m");
    }
}

/// The river 8 m wide along `z = 0` on ground falling 10 % to the east, from (-50, 0) to (50, 0),
/// and a tributary 2 m wide coming down along its flow at 45 degrees from (-20, -20) to its point at
/// (0, 0), listed after it; their water as `worked_out` works it out, and the water their beds hold
/// as a pass records it, the river's and then the tributary's (`KeptBeds`).
fn a_mouth_inside_a_river() -> ([Channel; 2], WorkedOut, KeptBeds) {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| 100.0 - 0.1 * x);
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let tributary = Channel { drawn: None, ..straight((-20.0, -20.0), (0.0, 0.0), 2.0, ChannelEnd::Channel(0)) };
    let channels = [river, tributary];
    let shapes = unshaped(&channels);
    let worked = worked(&channels, &shapes, &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    let river_bed = bed_cells((128, 128), &channels[0]);
    let mut kept = KeptBeds::default();
    kept.record(&worked.cut, &channels[0], &shapes[0], &worked.profiles[0], &rules, 0, |_| false);
    kept.record(&worked.cut, &channels[1], &shapes[1], &worked.profiles[1], &rules, 1, |cell| river_bed.binary_search(&cell).is_ok());
    (channels, worked, kept)
}

/// A tributary's mouth inside its river's bed leaves that bed's water the river's: in
/// `a_mouth_inside_a_river`, where the tributary's water stands a little higher than the river's
/// over part of the river's bed, as it comes along the flow, every cell of the river's bed that
/// holds water holds the river's, so another tributary of the river reads that bed bare, as the
/// foot of a channel reads the bed of the channel it ends on; and some cell of the tributary's own
/// bed outside the river's holds the tributary's water.
#[test]
fn a_tributarys_mouth_leaves_its_rivers_bed_bare_to_the_others() {
    let ([river, tributary], worked, kept) = a_mouth_inside_a_river();
    let size = (worked.cut.width, worked.cut.height);
    let river_bed = bed_cells(size, &river);
    let read: Vec<usize> = river_bed.iter().copied().filter(|&cell| kept.water_at(cell, worked.cut.data[cell], Some(0)).is_some()).collect();
    assert!(read.is_empty(), "{} cells of the river's bed hold another channel's water, such as {:?}", read.len(), &read[..read.len().min(5)]);
    let own = bed_cells(size, &tributary).into_iter().filter(|cell| river_bed.binary_search(cell).is_err()).filter(|&cell| kept.water_at(cell, worked.cut.data[cell], Some(0)).is_some());
    assert!(own.count() > 20, "the tributary's own bed holds none of its water");
}

/// A cell of an earlier channel's bed that holds none of its water, its ground at or above the
/// water there, is read at its own height: in `a_mouth_inside_a_river`, the river's cells on its
/// waterlines, `z = -4` and `4`, which its bank cuts to its surface, hold no water and read none,
/// while the cells inside them that lie below its water read it; and a cell under its water whose
/// ground stood above the water would read none.
#[test]
fn a_cell_above_an_earlier_beds_water_is_read_at_its_height() {
    let ([river, _], worked, kept) = a_mouth_inside_a_river();
    let (size, half) = (worked.cut.width, (worked.cut.width / 2) as i32);
    let cell = |x: i32, z: i32| ((z + half) as usize) * size + (x + half) as usize;
    let mut checked = 0;
    for x in 10..=40 {
        for z in [-4, 4] {
            checked += 1;
            assert_eq!(kept.water_at(cell(x, z), worked.cut.data[cell(x, z)], None), None, "({x}, {z}) on the waterline at {} m", worked.cut.data[cell(x, z)]);
        }
        let (_, _, surface_m) = nearest_water(&river, &worked.profiles[0], &ChannelPoint { x_m: f64::from(x), z_m: 1.0, width_m: 0.0, drained_m2: 0.0 });
        let (inside, ground_m) = (cell(x, 1), worked.cut.data[cell(x, 1)]);
        assert!(ground_m < surface_m - 0.1, "({x}, 1) at {ground_m} m under the water at {surface_m} m");
        let read = kept.water_at(inside, ground_m, None);
        assert!(read.is_some_and(|read_m| (read_m - surface_m).abs() < 1e-4), "({x}, 1) reads {read:?}, the water at {surface_m} m");
        assert_eq!(kept.water_at(inside, surface_m + 0.5, None), None, "({x}, 1) raised above the water");
    }
    assert_eq!(checked, 62);
}

/// The first tributary's mouth inside its river's bed is the river's water to a second tributary
/// of the river, as `worked_out` works them out (case (b) of `KeptBeds`, through the pass that
/// builds `inside_end`): on ground falling 10 % to the east and rising 5 % either side of `z = 0`,
/// the river 8 m wide along `z = 0` from (-50, 0) to (50, 0); a first tributary 5 m wide coming
/// along its flow at 45 degrees from (-20, -20) to its point at (0, 0), whose bed reaches 2.5 m
/// past its end, inside the river's bed, and whose water there stands at the river's surface at
/// x = 0, above the river's own east of it; and a second, a brook 1.2 m wide, listed after it, down
/// from the south along `x = -1` to (-1, 0), the foot of whose banks reaches the first one's mouth
/// at x = 1, 2 m east of its line, on its low side. The water is worked out in one pass. The
/// second's wished surfaces (`WorkedOut::wished_m`) are bit for bit those of the same brook ending
/// nowhere, worked out alone on the ground as the river and the first cut it, which reads every
/// cell at its ground; were the first's mouth recorded as its water, the second would read the
/// cells at x = 1 at the river's surface at x = 0, and its banks would hold it 0.1 m higher there.
#[test]
fn a_second_tributary_reads_the_first_ones_mouth_inside_the_river_as_the_rivers() {
    let rules = WaterRules::default();
    let before = ground(128, |x, z| 100.0 - 0.1 * x + 0.05 * z.abs());
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let first = Channel { drawn: None, ..straight((-20.0, -20.0), (0.0, 0.0), 5.0, ChannelEnd::Channel(0)) };
    let second = Channel { drawn: None, ..straight((-1.0, 30.0), (-1.0, 0.0), 1.2, ChannelEnd::Channel(0)) };
    let channels = [river.clone(), first.clone(), second.clone()];
    let joined = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    assert_eq!(joined.passes, 1, "passes");
    let two = [river, first];
    let cut_by_two = worked(&two, &unshaped(&two), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    assert_eq!(cut_by_two.passes, 1, "passes of the river and the first alone");
    let alone_second = [Channel { end: ChannelEnd::Edge, ..second }];
    let alone = worked(&alone_second, &unshaped(&alone_second), &cut_by_two.cut, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    assert_eq!(joined.wished_m[2], alone.wished_m[0]);
}

/// A brook's foot reads the bed of the channel it ends on bare, under that channel's water, as
/// the junction reads it: in `an_ordinary_mouth_on_a_river_falling_10_percent_caps_nothing`, where
/// the foot of the brook's banks before its entry point reaches into the river's bed, the brook's
/// wished surfaces (`WorkedOut::wished_m`) are bit for bit those of the same brook ending nowhere,
/// worked out alone on the ground as the river cut it, which reads every cell at its ground.
#[test]
fn a_brooks_banks_read_the_bed_it_ends_on_bare() {
    let rules = WaterRules::default();
    let before = ground(128, |x, z| 100.0 - 0.1 * x - 0.01 * z);
    let brook = Channel { drawn: None, ..straight((40.0, -12.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let channels = [brook.clone(), river.clone()];
    let joined = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    let (mut cut_ground, held) = (before.clone(), vec![f32::NEG_INFINITY; before.data.len()]);
    for_each_lowering(&before, &held, &river, &unshaped(std::slice::from_ref(&river))[0], &joined.profiles[1], &rules, |cell, lowered_m| {
        cut_ground.data[cell] = cut_ground.data[cell].min(lowered_m);
    });
    let alone_brook = [Channel { end: ChannelEnd::Edge, ..brook }];
    let alone = worked(&alone_brook, &unshaped(&alone_brook), &cut_ground, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    assert_eq!(joined.wished_m[0], alone.wished_m[0]);
}

/// The flag is read between the passes: in `a_brook_whose_banks_hold_1_m_less_brings_its_river_down_1_m`,
/// which asks for a second pass, a set flag ends the work with `Cancelled`; on the open slope,
/// where no junction asks for one, the water is worked out in one pass and the set flag changes
/// nothing.
#[test]
fn a_set_flag_ends_the_passes() {
    let rules = WaterRules::default();
    let (channels, junction) = brook_and_river();
    let (trench, _) = trench_short_by(&channels[1], junction, 1.0);
    let set = AtomicBool::new(true);
    let result = profiles(&channels, &unshaped(&channels), &trench, &on_ground(&trench), &[], &[], &rules, &set);
    assert_eq!(result.err(), Some(LandformError::Cancelled));
    let slope = in_a_trench(0.0);
    let on_slope = |cancel: &AtomicBool| profiles(&channels, &unshaped(&channels), &slope, &on_ground(&slope), &[], &[], &rules, cancel).expect("one pass reads no flag");
    let (cancelled, free) = (on_slope(&set), on_slope(&AtomicBool::new(false)));
    assert_eq!((cancelled.passes, cancelled.profiles), (1, free.profiles));
}

#[test]
fn below_a_lake_on_a_slope_the_water_falls_with_the_land() {
    let rules = WaterRules::default();
    // Land falling 1 in 2 to the east, 100 m high at x = 0, its floor; the 1 m ground holds a
    // gully a metre deep along the line, which the floor does not. A channel 4 m wide flows
    // east from (-56, 0) out of a lake whose water is the cells west of x = -52 m within 20 m of
    // the axis, its bed 120 m, and whose level lies 0.2 m above the floor at x = -50. The lake's
    // grade alone would hold it up to 0.05 m per metre, 47 m above the land at the far end.
    let slope = |x: f64| 100.0 - 0.5 * x;
    let in_lake = |x: f64, z: f64| x <= -52.0 && z.abs() <= 20.0;
    let before = ground(128, |x, z| if in_lake(x, z) { 120.0 } else { slope(x) - if z.abs() <= 2.0 { 1.0 } else { 0.0 } });
    let floor = |point: &ChannelPoint| slope(point.x_m) as f32;
    let outflow = Channel { from_lake: Some(0), ..straight((-56.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge) };
    let level = slope(-50.0) as f32 + 0.2;
    let lakes = [lake_where(&before, 0, level, in_lake, |_, _| false)];
    let [profile] = &found_profiles(std::slice::from_ref(&outflow), &unshaped(std::slice::from_ref(&outflow)), &before, &floor, &lakes, &[level], &rules)[..] else {
        panic!("one profile")
    };
    assert!(never_rise(&profile.surface_m), "the surface rises: {:?}", profile.surface_m);
    // Its stretch beside the lake runs to (-48, 0), the far end of its last segment with a lake
    // cell, (-52, 0), within its half width and 1.5 m: the level outright. Beyond, its banks,
    // the ground beside the gully, 1 m above its line, hold it: 0.1 m below them, here its
    // floor, as the land falls, however high the lake's grade would hold it.
    for (k, (point, &surface)) in outflow.points.iter().zip(&profile.surface_m).enumerate() {
        let expected = if point.x_m <= -48.0 { level } else { floor(point) - 0.1 };
        assert!((surface - expected).abs() < 1e-4, "point {k} at x = {}: {surface} m, not {expected} m", point.x_m);
    }
}

/// How far `point` lies from the nearest point of `channel`'s line, and the half width (no
/// less than `HALF_WIDTH_MIN_M`) and surface of its water (`water`) there, linear along each
/// stretch.
fn nearest_water(channel: &Channel, water: &Profile, point: &ChannelPoint) -> (f64, f64, f32) {
    let mut nearest = (f64::INFINITY, 0.0, f32::NAN);
    for k in 1..channel.points.len() {
        let (a, b) = (channel.points[k - 1], channel.points[k]);
        let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
        let squared = dx * dx + dz * dz;
        let t = if squared > 0.0 { (((point.x_m - a.x_m) * dx + (point.z_m - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        let apart_m = (a.x_m + dx * t - point.x_m).hypot(a.z_m + dz * t - point.z_m);
        if apart_m < nearest.0 {
            let between = |u: f32, v: f32| u + (v - u) * t as f32;
            let half_width_m = |point: &ChannelPoint| (f64::from(point.width_m) / 2.0).max(HALF_WIDTH_MIN_M);
            let half_m = half_width_m(&a) + (half_width_m(&b) - half_width_m(&a)) * t;
            nearest = (apart_m, half_m, between(water.surface_m[k - 1], water.surface_m[k]));
        }
    }
    nearest
}

/// The first point of `channel` from which every later point lies inside the bed of `joined`
/// (whose water is `water`): within the half width of `joined`'s water at its nearest point.
fn entry_into(channel: &Channel, joined: &Channel, water: &Profile) -> usize {
    let last = channel.points.len() - 1;
    let inside = |k: usize| {
        let (apart_m, half_width_m, _) = nearest_water(joined, water, &channel.points[k]);
        apart_m <= half_width_m
    };
    (0..last).rev().take_while(|&k| inside(k)).last().unwrap_or(last)
}

#[test]
fn a_tributary_takes_its_rivers_surface_inside_its_bed() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    // A river 8 m wide runs east along z = 0, falling 0.02 m per metre. Brooks come in at 45
    // degrees from the north-west, along the river's flow, and from the north-east, against it;
    // each is inside the river's bed for its last 4 m across (its last four points).
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let junction = river.points.iter().position(|point| (point.x_m, point.z_m) == (0.0, 0.0)).expect("a junction point");
    for from in [(-20.0, -20.0), (20.0, -20.0)] {
        let brook = Channel { drawn: None, ..straight(from, (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
        let found = unshaped_profiles(&[brook.clone(), river.clone()], &before, &on_ground(&before), &[], &rules);
        let (water, joined) = (&found[0], &found[1]);
        let entry = entry_into(&brook, &river, joined);
        assert_eq!(entry, brook.points.len() - 4, "from {from:?}: the entry point");
        let nearest: Vec<f32> = brook.points[entry..].iter().map(|point| nearest_water(&river, joined, point).2).collect();
        assert!(never_rise(&water.surface_m), "from {from:?}: the surface rises: {:?}", water.surface_m);
        for (k, &surface) in water.surface_m.iter().enumerate().skip(entry) {
            let below = nearest[k - entry] - surface;
            assert!(below >= -1e-4, "from {from:?}: point {k} lies {} m above the river's surface", -below);
        }
        if from.0 < 0.0 {
            // Along the flow, the river's surface at the nearest points falls towards the
            // junction: each point takes it, and the last the river's surface there.
            for (k, &surface) in water.surface_m.iter().enumerate().skip(entry) {
                assert!(close(surface, nearest[k - entry]), "from {from:?}: point {k} at {surface} m, not {} m", nearest[k - entry]);
            }
            assert_eq!(water.surface_m[brook.points.len() - 1], joined.surface_m[junction], "from {from:?}: the last surface");
        } else {
            // Against the flow it rises towards the junction, so the running minimum holds the
            // whole stretch at the river's surface where it enters, 0.08 m below the junction's.
            for (k, &surface) in water.surface_m.iter().enumerate().skip(entry) {
                assert!(close(surface, nearest[0]), "from {from:?}: point {k} at {surface} m, not {} m", nearest[0]);
            }
            let below = joined.surface_m[junction] - water.surface_m[brook.points.len() - 1];
            assert!((below - 0.08).abs() < 1e-3, "from {from:?}: the last surface lies {below} m below the junction's");
        }
    }
}

#[test]
fn a_river_leaves_a_lake_at_its_level() {
    let rules = WaterRules::default();
    // Ground falling 2 % to the east, and behind the river's first point, west of x = -51 m, a
    // lake's water, its bed 98 m. The river, 4 m wide, flows out of it east from (-50, 0).
    let in_lake = |x: f64, z: f64| x <= -51.0 && z.abs() <= 10.0;
    let before = ground(128, |x, z| if in_lake(x, z) { 98.0 } else { tilted(x) });
    let river = Channel { from_lake: Some(0), ..straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge) };
    let free = Channel { from_lake: None, ..river.clone() };
    let shapes = unshaped(std::slice::from_ref(&river));
    // Its own water, alone, 0.192 m below its line: 100.808 m where it leaves the lake.
    let own_m = |k: usize| ground_m(&before, &river.points[k]) - 0.4 * 0.48;
    // Its stretch beside the lake runs to (-46, 0), the far end of its last segment with a lake
    // cell, (-51, 0), within its half width and 1.5 m: 4 m. Along it the river stands at the
    // level. Beyond it, at the lowest so far of the level, what its banks hold (the ground beside
    // it, as high as its line across it, less 0.1 m), and the higher of its own surface and the
    // level less 0.05 m per metre past the stretch. A lake above its own surface, at 101.5 m,
    // holds it up only as far as its banks let it: it falls 0.7 m where the stretch ends, follows
    // its banks, then the grade, then its own surface. One below it, at 99.5 m, holds it down.
    for level in [99.5_f32, 101.5] {
        let lakes = [lake_where(&before, 0, level, in_lake, |_, _| false)];
        let [profile] = &found_profiles(std::slice::from_ref(&river), &shapes, &before, &on_ground(&before), &lakes, &[level], &rules)[..] else {
            panic!("one profile")
        };
        assert!(never_rise(&profile.surface_m), "the surface rises: {:?}", profile.surface_m);
        let (mut lowest_m, mut own_lowest_m) = (f32::INFINITY, level);
        for (k, &surface) in profile.surface_m.iter().enumerate() {
            let along_m = channel_along_m(&river, k);
            own_lowest_m = own_lowest_m.min(own_m(k));
            let expected = if along_m <= 4.0 {
                level
            } else {
                let banks_m = ground_m(&before, &river.points[k]) - 0.1;
                let graded_m = (f64::from(level) - 0.05 * (along_m - 4.0)) as f32;
                level.min(banks_m).min(own_lowest_m.max(graded_m))
            };
            lowest_m = lowest_m.min(expected);
            assert!((surface - lowest_m).abs() < 1e-4, "lake at {level} m: point {k}, {along_m} m along, at {surface} m, not {lowest_m} m");
        }
    }
    // A lake that holds no water holds nothing back.
    assert_eq!(found_profiles(&[river], &shapes, &before, &on_ground(&before), &[], &[f32::NAN], &rules), unshaped_profiles(&[free], &before, &on_ground(&before), &[], &rules));
}

/// A channel 4 m wide flowing out of lake 0, level at 99 m, east along `z = 0` from (-50, 0) to
/// (50, 0), on ground 128 m a side: a trench whose floor, 98 m, lies 2 m below the ground beside
/// it, 100 m, so its banks hold it; the lake's water the cells west of `x = -44` m within 20 m of
/// the axis, its bed 98 m too. The cell at `low`, if any, lies at 98.1 m. The channel's own
/// surface is 0.192 m below its line, 97.808 m. Its stretch beside the lake runs to (-40, 0), the
/// far end of its last segment with a lake cell, (-44, 0), within its half width and 1.5 m: 10 m
/// along it.
fn out_of_a_trench(low: Option<(i32, i32)>) -> (Grid<f32>, [LakeShape; 1], Channel) {
    let in_lake = |x: f64, z: f64| x <= -44.0 && z.abs() <= 20.0;
    let before = ground(128, |x, z| {
        if low == Some((x as i32, z as i32)) {
            98.1
        } else if in_lake(x, z) || z.abs() <= 2.0 {
            98.0
        } else {
            100.0
        }
    });
    let lakes = [lake_where(&before, 0, 99.0, in_lake, |_, _| false)];
    let channel = Channel { from_lake: Some(0), ..straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge) };
    (before, lakes, channel)
}

#[test]
fn out_of_a_lake_whose_banks_hold_the_water_keeps_the_level_then_falls_at_the_grade() {
    let rules = WaterRules::default();
    let (before, lakes, channel) = out_of_a_trench(None);
    let channels = [channel];
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&lakes), &[99.0], &rules);
    assert_eq!(worked.read[0].left_m, 10.0, "where the channel leaves the lake");
    // The level to the stretch's end, then 0.05 m per metre down to its own surface.
    for (k, &surface) in worked.profiles[0].surface_m.iter().enumerate() {
        let along_m = channel_along_m(&channels[0], k);
        let expected = if along_m <= 10.0 { 99.0 } else { (99.0 - 0.05 * (along_m - 10.0)).max(97.808) as f32 };
        assert!((surface - expected).abs() < 1e-4, "point {k}, {along_m} m along, at {surface} m, not {expected} m");
    }
}

#[test]
fn where_its_banks_stand_lower_a_course_out_of_a_lake_falls_as_they_let_it() {
    let rules = WaterRules::default();
    // A cell of the foot of its banks, beside the stretch from (-36, 0) to (-34, 0), lies at
    // 98.1 m: the banks of the points 4 and 6 m past the stretch hold the water 0.1 m below it,
    // at 98 m, 1 m below the level, where the grade would hold it at 98.8 m.
    let (before, lakes, channel) = out_of_a_trench(Some((-35, 3)));
    let channels = [channel];
    let worked = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Cut(&lakes), &[99.0], &rules);
    assert_eq!(worked.read[0].left_m, 10.0, "where the channel leaves the lake");
    let surface_m = &worked.profiles[0].surface_m;
    let at_m = |x: f64| surface_m[channels[0].points.iter().position(|point| point.x_m == x).expect("a point there")];
    assert!((at_m(-38.0) - 98.9).abs() < 1e-4, "2 m past the stretch at {} m, not at the grade's 98.9 m", at_m(-38.0));
    assert!((at_m(-36.0) - 98.0).abs() < 1e-4, "4 m past the stretch at {} m, not 1 m below the level", at_m(-36.0));
    // Then it stays there until the grade comes down below it, and falls at the grade to its own
    // surface.
    for (k, &surface) in surface_m.iter().enumerate() {
        let along_m = channel_along_m(&channels[0], k);
        let expected = match along_m {
            along if along <= 10.0 => 99.0,
            along if along < 14.0 => 98.9,
            along => (99.0 - 0.05 * (along - 10.0)).clamp(97.808, 98.0) as f32,
        };
        assert!((surface - expected).abs() < 1e-4, "point {k}, {along_m} m along, at {surface} m, not {expected} m");
    }
}

#[test]
fn inside_a_lakes_tongue_a_course_out_of_it_stands_at_the_level() {
    let rules = WaterRules::default();
    // A lake at 99 m: its basin west of x = -40 m, its bed 97 m, and a tongue of it 13 m wide down
    // the valley of the course out of it, its bed rising from 97 m at x = -40 m to 98.9 m at
    // x = 0; beyond, the course's trench, 98.9 m, between banks 100 m high. The course, 4 m wide,
    // flows east from (-50, 0): its stretch beside the lake runs to (4, 0), the far end of its
    // last segment with a lake cell, (0, 0), within its half width and 1.5 m.
    let basin = |x: f64, z: f64| x <= -40.0 && z.abs() <= 20.0;
    let tongue = |x: f64, z: f64| (-40.0..=0.0).contains(&x) && z.abs() <= 6.0;
    let before = ground(128, |x, z| {
        if basin(x, z) {
            97.0
        } else if tongue(x, z) {
            97.0 + 1.9 * (x + 40.0) / 40.0
        } else if z.abs() <= 2.0 {
            98.9
        } else {
            100.0
        }
    });
    let level_m = 99.0;
    let lakes = [lake_where(&before, 0, level_m, |x, z| basin(x, z) || tongue(x, z), |_, _| false)];
    let channels = [Channel { from_lake: Some(0), ..straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge) }];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &lakes, &[level_m], &rules);
    // Inside the tongue, and to the end of the stretch, the course stands at the level.
    for (k, (point, &surface)) in channels[0].points.iter().zip(&found[0].surface_m).enumerate().filter(|(_, (point, _))| point.x_m <= 4.0) {
        assert_eq!(surface, level_m, "point {k} at x = {} stands at {surface} m, not at the level", point.x_m);
    }
    // The course and the lake's water agree: with the beds cut and the lake's water put back,
    // every cell of the tongue inside the course's bed holds water at the level.
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &lakes, &rules);
    let mut inside = 0;
    for &cell in &lakes[0].cells {
        depth.data[cell] = depth.data[cell].max(level_m - after.data[cell]);
        let (x, z) = ((cell % 128) as i32 - 64, (cell / 128) as i32 - 64);
        if (-40..=0).contains(&x) && z.abs() <= 2 {
            inside += 1;
            let water_m = after.data[cell] + depth.data[cell];
            assert!((water_m - level_m).abs() < 1e-4, "water at {water_m} m at ({x}, {z}) in the tongue");
        }
    }
    assert!(inside > 150, "only {inside} cells of the tongue inside the course's bed");
}

/// The cell of a 128 m grid laid out as `ground` lays it out at world `(x, z)`.
fn cell_at((x, z): (i32, i32)) -> usize {
    ((z + 64) * 128 + x + 64) as usize
}

#[test]
fn no_bank_cuts_a_lakes_rim_below_its_falling_hold() {
    let rules = WaterRules::default();
    // A lake at 99 m: a basin west of x = -40 m and a strip of it north of z = 10 m from x = -40
    // to 40 m, its bed 95 m. A course 4 m wide flows east out of the basin from (-50, 0) along
    // z = 0, on ground 100 m high, its stretch beside the lake running to (-36, 0) (the held bed
    // its cells within 2 m of the line from (-50, 0) to there, not the lake's). South of it the
    // ground lies at 90.3 m, so its banks hold it at 90.2 m past the stretch; north of it, between
    // it and the strip, the lake's rim, 102 m high. The course's bank, rising 1 in 2 from its
    // water, cuts the rim from x = -20 to 20 m 5 m from the lake's water, at z = 5 m, to 93.3 m,
    // and 1 m from it, at z = 9 m, to 98.9 m: below the rim's hold, the level plus 0.1 m less
    // 1 m for every 2 m beyond the first 1.5 m, 97.35 m and 99.1 m.
    let basin = |x: f64, z: f64| x <= -40.0 && z.abs() <= 30.0;
    let strip = |x: f64, z: f64| (-40.0..=40.0).contains(&x) && (10.0..=30.0).contains(&z);
    let in_lake = move |x: f64, z: f64| basin(x, z) || strip(x, z);
    let held_bed = |x: f64, z: f64| (x - x.clamp(-50.0, -36.0)).hypot(z) <= 2.0;
    let before = ground(128, |x, z| {
        if in_lake(x, z) {
            95.0
        } else if z.abs() <= 2.0 {
            100.0
        } else if z < 0.0 {
            90.3
        } else {
            102.0
        }
    });
    let level_m = 99.0;
    let lakes = [lake_where(&before, 0, level_m, in_lake, held_bed)];
    let channels = [Channel { from_lake: Some(0), ..straight((-50.0, 0.0), (60.0, 0.0), 4.0, ChannelEnd::Edge) }];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &lakes, &[level_m], &rules);
    let cut_with = |lakes: &[LakeShape]| {
        let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
        cut_beds(&mut after, &mut depth, &channels, &shapes, &found, lakes, &rules);
        after
    };
    let (held, free) = (cut_with(&lakes), cut_with(&[]));
    // The hold `d` metres from the lake's water.
    let hold_m = |d: f64| (f64::from(level_m) + 0.1 - (d - 1.5).max(0.0) / 2.0) as f32;
    for x in -20..=20 {
        for z in 3..=9 {
            let (cell, hold) = (cell_at((x, z)), hold_m(f64::from(10 - z)));
            assert!(held.data[cell] >= hold - 1e-4, "({x}, {z}), {} m from the lake's water, cut to {} m, below its hold {hold} m", 10 - z, held.data[cell]);
        }
        for (z, cut_m) in [(5, 93.31), (9, 98.88)] {
            let (cell, hold) = (cell_at((x, z)), hold_m(f64::from(10 - z)));
            assert!((free.data[cell] - cut_m).abs() < 0.01, "({x}, {z}): with no lake the bank cuts it to {} m, not {cut_m} m", free.data[cell]);
            assert!((held.data[cell] - hold).abs() < 1e-4, "({x}, {z}): the bank cuts it to {} m, not to its hold {hold} m", held.data[cell]);
        }
    }
}

#[test]
fn a_second_course_out_of_a_lake_wets_no_bank_of_the_first() {
    let rules = WaterRules::default();
    // A lake at 99 m, its water the cells west of x = -31 m within 30 m of the axis, its bed
    // 95 m. The first course, 4 m wide, flows out of it east from (-40, 0) along z = 0 on ground
    // 100 m high; its stretch beside the lake runs to (-26, 0). Beside it to the north a shelf,
    // 98.8 m, from x = -30 to -18 m and z = 3 to 8 m, below the level, so past the stretch its
    // banks hold it at 98.7 m. The second, 2 m wide, flows out of the lake from (-46, 12) through
    // (-43, 6) and ends on the first's first point; its banks reach 12 m beyond its waterline, to
    // x = -27 m. Its water stands at the level and spreads over the lake's bed onto the shelf.
    let in_lake = |x: f64, z: f64| x <= -31.0 && z.abs() <= 30.0;
    let shelf = |x: f64, z: f64| (-30.0..=-18.0).contains(&x) && (3.0..=8.0).contains(&z);
    let before = ground(128, |x, z| if in_lake(x, z) { 95.0 } else if shelf(x, z) { 98.8 } else { 100.0 });
    let level_m = 99.0;
    let lakes = [lake_where(&before, 0, level_m, in_lake, |_, _| false)];
    let first = Channel { from_lake: Some(0), ..straight((-40.0, 0.0), (60.0, 0.0), 4.0, ChannelEnd::Edge) };
    let points = [(-46.0, 12.0), (-43.0, 6.0), (-40.0, 0.0)].map(|(x_m, z_m)| ChannelPoint { x_m, z_m, width_m: 2.0, drained_m2: 0.0 });
    let second = Channel { drawn: None, from_lake: Some(0), points: points.to_vec(), end: ChannelEnd::Channel(0) };
    let channels = [first, second];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &lakes, &[level_m], &rules);
    assert!(found[1].surface_m.iter().all(|&surface| surface == level_m), "the second course stands at {:?} m", found[1].surface_m);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &lakes, &rules);
    for &cell in &lakes[0].cells {
        depth.data[cell] = depth.data[cell].max(level_m - after.data[cell]);
    }
    // The shelf within the second's banks' reach holds water at the level, beside the first's
    // stretch, where the first stands at the level too.
    let water_m = |(x, z): (i32, i32)| after.data[cell_at((x, z))] + depth.data[cell_at((x, z))];
    assert!(depth.data[cell_at((-29, 5))] > 0.0 && (water_m((-29, 5)) - level_m).abs() < 1e-4, "the shelf at (-29, 5): {} m", water_m((-29, 5)));
    // Wherever the first lies more than 0.05 m below the level, no water within its banks' reach
    // stands more than 0.05 m above it. Here that holds of the first's own water only: the first
    // lies below the level only past its stretch, east of x = -26 m, and the second's water
    // reaches no farther east than its banks, x = -27 m, so it cannot reach a cell this reads.
    // The world test `a_second_course_out_of_a_lake_floods_no_bank_of_the_first` (`world.rs`)
    // carries the case where the second course's water could.
    let (mut checked, mut above) = (0, Vec::new());
    for z in -64..64 {
        for x in -64..64 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (apart_m, half_width_m, surface_m) = nearest_water(&channels[0], &found[0], &cell);
            if apart_m - half_width_m <= BANK_REACH_M && surface_m < level_m - 0.05 && depth.data[cell_at((x, z))] > 0.0 {
                checked += 1;
                if water_m((x, z)) > surface_m + 0.05 {
                    above.push(format!("({x}, {z}): water at {} m beside the first at {surface_m} m", water_m((x, z))));
                }
            }
        }
    }
    assert!(checked > 100, "only {checked} wet cells beside the first where it lies below the level");
    assert!(above.is_empty(), "{} cells: {:#?}", above.len(), &above[..above.len().min(10)]);
}

/// How far point `k` of `channel` lies along it from its first point, in metres.
fn channel_along_m(channel: &Channel, k: usize) -> f64 {
    channel.points[..=k].windows(2).map(|pair| (pair[1].x_m - pair[0].x_m).hypot(pair[1].z_m - pair[0].z_m)).sum()
}

#[test]
fn a_short_outflow_still_meets_what_it_ends_on() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    // Out of a lake at 101.5 m, 20 m long: too short to come down at 0.05 m per metre to a lake
    // at 99 m, or to the river it ends on, whose surface at the junction is 99.616 m.
    let into_a_lake = Channel { from_lake: Some(0), ..straight((-10.0, 0.0), (10.0, 0.0), 4.0, ChannelEnd::Lake(1)) };
    let [profile] = &unshaped_profiles(std::slice::from_ref(&into_a_lake), &before, &on_ground(&before), &[101.5, 99.0], &rules)[..] else {
        panic!("one profile")
    };
    let last = profile.surface_m.len() - 1;
    assert_eq!(profile.surface_m[last], 99.0, "into a lake: the end at {} m", profile.surface_m[last]);
    assert!(profile.surface_m[0] < 101.5, "into a lake: the end does not win: {:?}", profile.surface_m);
    assert!(never_rise(&profile.surface_m), "into a lake: the surface rises: {:?}", profile.surface_m);

    let onto_a_river = Channel { from_lake: Some(0), ..straight((0.0, -20.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let junction = river.points.iter().position(|point| (point.x_m, point.z_m) == (0.0, 0.0)).expect("a junction point");
    let found = unshaped_profiles(&[onto_a_river.clone(), river], &before, &on_ground(&before), &[101.5], &rules);
    let (water, joined) = (&found[0], found[1].surface_m[junction]);
    let last = water.surface_m.len() - 1;
    assert!((joined - 99.616).abs() < 1e-3, "the river's surface at the junction is {joined} m");
    assert_eq!(water.surface_m[last], joined, "onto a river: the end at {} m", water.surface_m[last]);
    assert!(water.surface_m[0] < 101.5, "onto a river: the end does not win: {:?}", water.surface_m);
    assert!(never_rise(&water.surface_m), "onto a river: the surface rises: {:?}", water.surface_m);
}

#[test]
fn a_river_reaches_a_lake_no_lower_than_its_level() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    let river = straight((-50.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Lake(0));
    // The river would reach the lake below its level (99.6 m), and above it (98 m).
    let free = unshaped_profiles(&[Channel { end: ChannelEnd::Edge, ..river.clone() }], &before, &on_ground(&before), &[], &rules);
    for level in [99.6, 98.0] {
        let [profile] = &unshaped_profiles(std::slice::from_ref(&river), &before, &on_ground(&before), &[level], &rules)[..] else { panic!("one profile") };
        assert_comes_down_to(&format!("into a lake at {level} m"), &river, profile, &free[0], level, 0.0, &rules);
    }
    // A lake that holds no water holds nothing back.
    let free = Channel { end: ChannelEnd::Edge, ..river.clone() };
    assert_eq!(unshaped_profiles(&[river], &before, &on_ground(&before), &[f32::NAN], &rules), unshaped_profiles(&[free], &before, &on_ground(&before), &[], &rules));
}

#[test]
fn a_bend_is_cut_round_its_outside() {
    let rules = WaterRules::default();
    // East along z = 0 to the corner at (0, 0), then south; the ground falls along both legs.
    let before = ground(128, |x, z| 100.0 - 0.02 * x - 0.02 * z);
    let east = straight((-40.0, 0.0), (0.0, 0.0), 8.0, ChannelEnd::Edge);
    let south = straight((0.0, 0.0), (0.0, 40.0), 8.0, ChannelEnd::Edge);
    let corner = east.points.len() - 1;
    let bend = Channel { points: [&east.points[..], &south.points[1..]].concat(), ..east };
    let channels = [bend];
    let [profile] = &unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    let (bed, surface) = (beds(profile, &rules)[corner], profile.surface_m[corner]);
    let (after, depth) = cut(&before, &channels, &rules);
    // North-east of the corner, outside both legs, the corner alone is nearest: a bowl within
    // 4 m of it, a bank beyond.
    for x in 1..=20 {
        for z in -20..=-1 {
            let distance_m = f64::from(x).hypot(f64::from(z)) as f32;
            let was = at(&before, (x, z));
            let expected = if distance_m <= 4.0 { surface - (surface - bed) * bowl(distance_m, 4.0) } else { banked(was, surface, distance_m - 4.0) };
            assert!(close(at(&after, (x, z)), expected), "({x}, {z}) lies at {} m, not {expected} m", at(&after, (x, z)));
        }
    }
    // Inside the bend, within the bed of both legs, 2 m from the line: (-3, 2) is nearest (-3, 0)
    // on the east leg, half way between its points 18 and 19, and 3 m from (0, 2) on the south
    // leg; (-2, 3) is nearest (0, 3), half way between points 21 and 22, and 3 m from (-2, 0).
    let (surfaces, beds) = (&profile.surface_m, beds(profile, &rules));
    for (cell, k) in [((-3, 2), corner - 2), ((-2, 3), corner + 1)] {
        let (bed, surface) = ((beds[k] + beds[k + 1]) / 2.0, (surfaces[k] + surfaces[k + 1]) / 2.0);
        let (is, water) = (at(&after, cell), at(&depth, cell));
        let expected = (surface - bed) * bowl(2.0, 4.0);
        assert!(close(is, surface - expected), "{cell:?} lies at {is} m, not at the bowl of its nearest point, {} m", surface - expected);
        assert!(close(water, expected), "the water at {cell:?} is {water} m deep, not {expected} m");
    }
}

#[test]
fn a_bank_leaves_no_step_at_its_reach() {
    let rules = WaterRules::default();
    // A channel 4 m wide, from the west border, runs east along the foot of a slope that rises
    // 2 m per metre to the north. Its bed lies 0.48 m below the ground at the line, its surface
    // 0.288 m above the bed; the bank starts at the surface on the waterline, 2 m out.
    let before = ground(128, |x, z| tilted(x) + 2.0 * (-z).max(0.0));
    let channels = [straight((-64.0, 0.0), (60.0, 0.0), 4.0, ChannelEnd::Edge)];
    let (after, _) = cut(&before, &channels, &rules);
    for x in -20..=20 {
        let (bed, surface) = (at(&before, (x, 0)) - 0.48, at(&before, (x, 0)) - 0.192);
        let mut steepest = 0.0_f32;
        for k in 0..=30 {
            let (was, is) = (at(&before, (x, -k)), at(&after, (x, -k)));
            let beyond = k as f32 - 2.0;
            let expected = if beyond <= 0.0 { surface - (surface - bed) * bowl(k as f32, 2.0) } else { banked(was, surface, beyond) };
            assert!((is - expected).abs() < 1e-3, "({x}, {}) lies at {is} m, not {expected} m", -k);
            if beyond >= 12.0 {
                assert_eq!(is, was, "({x}, {}), at or beyond the bank's reach, was cut", -k);
            }
            if k > 0 {
                steepest = steepest.max(is - at(&after, (x, 1 - k)));
            }
        }
        // By the formula: lowered by 2.76 m 9 m beyond the waterline, by 0.41 m 11 m beyond,
        // not at all from 12 m; the steepest step, between 8 and 9 m beyond, 3.43 m against the
        // slope's 2 m. A bank that stopped dead at the reach would leave a step of 24.2 m there.
        let cut_at = |beyond: i32| at(&before, (x, -2 - beyond)) - at(&after, (x, -2 - beyond));
        assert!((cut_at(9) - 2.7644).abs() < 1e-3 && (cut_at(11) - 0.4071).abs() < 1e-3, "cuts {} and {} m", cut_at(9), cut_at(11));
        assert!((steepest - 3.4336).abs() < 1e-3, "at x = {x} the steepest step is {steepest} m");
    }
}

#[test]
fn a_spring_grows_into_its_bed() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    // A channel 4 m wide (0.48 m deep in full) beginning 34 m inside the west border.
    let spring = straight((-30.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge);
    let [profile] = &unshaped_profiles(std::slice::from_ref(&spring), &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    assert_eq!(profile.depth_m[0], 0.0, "the depth at its first point");
    assert!(close(profile.depth_m[3], 0.24), "{} m deep 6 m along", profile.depth_m[3]);
    assert!(profile.depth_m[6..].iter().all(|&depth| depth == bed_depth_m(4.0, &rules)), "not in full from 12 m along");
    assert!(never_rise(&beds(profile, &rules)), "the bed rises: {:?}", beds(profile, &rules));
    let (after, depth) = cut(&before, std::slice::from_ref(&spring), &rules);
    assert_eq!(at(&after, (-30, 0)), at(&before, (-30, 0)), "the ground at its first point was cut");
    assert_eq!(at(&depth, (-30, 0)), 0.0, "water at its first point");
    assert!(close(at(&before, (-18, 0)) - at(&after, (-18, 0)), 0.48), "12 m along, the ground is not lowered by the full depth");

    // Out of a lake that holds water, or from 8 m inside the border, it is deep from its first point;
    // out of a lake that holds none, or from 9 m inside, it grows.
    let first_depth = |channel: Channel, levels: &[f32]| unshaped_profiles(&[channel], &before, &on_ground(&before), levels, &rules)[0].depth_m[0];
    let from_lake = Channel { from_lake: Some(0), ..spring.clone() };
    assert_eq!(first_depth(from_lake.clone(), &[1_000.0]), 0.48);
    assert_eq!(first_depth(from_lake, &[f32::NAN]), 0.0);
    assert_eq!(first_depth(straight((-56.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge), &[]), 0.48);
    assert_eq!(first_depth(straight((-55.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge), &[]), 0.0);
}

#[test]
fn odd_widths_still_cut() {
    let rules = WaterRules::default();
    let before = ground(128, |x, z| tilted(x) + 0.01 * z);
    for (width_m, deep_m) in [(0.5, 0.3), (60.0, 2.5)] {
        let channels = [straight((-60.0, 0.0), (60.0, 0.0), width_m, ChannelEnd::Edge)];
        let (after, depth) = cut(&before, &channels, &rules);
        let unfinite = after.data.iter().chain(&depth.data).filter(|value| !value.is_finite()).count();
        assert_eq!(unfinite, 0, "{width_m} m wide: {unfinite} values are not finite");
        let lowered = at(&before, (0, 0)) - at(&after, (0, 0));
        assert!(close(lowered, deep_m), "{width_m} m wide: the line is lowered by {lowered} m, not {deep_m} m");
        assert!(close(at(&depth, (0, 0)), 0.6 * deep_m), "{width_m} m wide: the water is {} m deep", at(&depth, (0, 0)));
    }
}

/// The distance from world `(x, z)` to the nearest point of the line through `points`, and the
/// width there.
fn from_line_m(points: &[ChannelPoint], (x, z): (f64, f64)) -> (f64, f64) {
    (points.windows(2))
        .map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
            let squared = dx * dx + dz * dz;
            let t = if squared > 0.0 { (((x - a.x_m) * dx + (z - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
            let width_m = f64::from(a.width_m) + (f64::from(b.width_m) - f64::from(a.width_m)) * t;
            ((a.x_m + dx * t - x).hypot(a.z_m + dz * t - z), width_m)
        })
        .fold((f64::INFINITY, 0.0), |nearest, seen| if seen.0 < nearest.0 { seen } else { nearest })
}

#[test]
fn a_channel_on_the_border_cuts_inside_the_grid() {
    let rules = WaterRules::default();
    let size = 64;
    let before = ground(size, |x, z| tilted(x) - 0.01 * z);
    // Down the west border, on it; and from the east border, one metre beyond the last column,
    // to the centre. Both come in across the border, so both are deep from their first point.
    let cases = [
        straight((-32.0, -32.0), (-32.0, 32.0), 8.0, ChannelEnd::Edge),
        straight((32.0, 0.0), (0.0, 0.0), 8.0, ChannelEnd::Edge),
    ];
    for channel in cases {
        let [profile] = &unshaped_profiles(std::slice::from_ref(&channel), &before, &on_ground(&before), &[], &rules)[..] else {
            panic!("one profile")
        };
        // The bed lies 0.96 m below the ground at the line: the water 0.576 m deep there.
        assert!(profile.depth_m.iter().all(|&depth| close(depth, 0.96)), "{:?}", profile.depth_m);
        let deep_m = rules.water_fill * 0.96;
        let (after, depth) = cut(&before, std::slice::from_ref(&channel), &rules);
        let first = (channel.points[0].x_m, channel.points[0].z_m);
        let unfinite = after.data.iter().chain(&depth.data).filter(|value| !value.is_finite()).count();
        assert_eq!(unfinite, 0, "from {first:?}: {unfinite} values are not finite");
        let half = (size / 2) as i32;
        for z in -half..half {
            for x in -half..half {
                let (distance_m, width_m) = from_line_m(&channel.points, (f64::from(x), f64::from(z)));
                let (was, is) = (at(&before, (x, z)), at(&after, (x, z)));
                if distance_m > width_m / 2.0 + 12.0 + 1.0 {
                    assert_eq!(is, was, "from {first:?}: ({x}, {z}), {distance_m} m from the line, was cut");
                } else if distance_m <= width_m / 2.0 {
                    // In the bed: the bowl under the surface at the line's nearest point, wet
                    // inside its waterlines.
                    let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
                    let (_, half_width_m, surface) = nearest_water(&channel, profile, &cell);
                    let expected = was.min(surface - deep_m * bowl(distance_m as f32, half_width_m as f32));
                    assert!((is - expected).abs() < 1e-3, "from {first:?}: ({x}, {z}) in the bed lies at {is} m, not {expected} m");
                    if distance_m < width_m / 2.0 {
                        assert!(at(&depth, (x, z)) > 0.0, "from {first:?}: no water at ({x}, {z}), in the bed");
                    }
                }
            }
        }
    }
}

/// Review Focus 2: a river along a hillside is cut deeper until its downhill bank holds, and no
/// water lies on the slope below.
#[test]
fn a_river_along_a_hillside_is_cut_until_its_bank_holds() {
    let rules = WaterRules::default();
    // A plane falling 30 % across a channel 16 m wide that runs east along z = 0 from the west
    // border, downhill to the south (+z): its waterlines lie 8 m either side of its line.
    let hillside = |z: f64| 100.0 - 0.3 * z;
    let before = ground(128, |_, z| hillside(z));
    let channels = [straight((-64.0, 0.0), (60.0, 0.0), 16.0, ChannelEnd::Edge)];
    let [profile] = &unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    for (point, &surface) in channels[0].points.iter().zip(&profile.surface_m) {
        let below_m = hillside(point.z_m) as f32 - surface;
        assert!(below_m <= 0.3 * 9.5 + 0.2, "at x = {} the surface lies {below_m} m below the ground at the line", point.x_m);
    }
    let (_, depth) = cut(&before, &channels, &rules);
    let (mut foot, mut below) = (0, 0);
    for z in 1..64 {
        for x in -64..64 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (apart_m, half_width_m, surface) = nearest_water(&channels[0], profile, &cell);
            let beyond_m = apart_m - half_width_m;
            if beyond_m <= 0.0 {
                continue;
            }
            below += 1;
            assert_eq!(at(&depth, (x, z)), 0.0, "water at ({x}, {z}), {beyond_m} m beyond the downhill waterline");
            if beyond_m <= 1.5 {
                foot += 1;
                let above_m = at(&before, (x, z)) - surface;
                assert!(above_m >= 0.1 - 1e-4, "({x}, {z}), {beyond_m} m beyond the waterline, stood {above_m} m above the surface");
            }
        }
    }
    assert!(foot >= 120 && below >= 3000, "only {foot} cells of the foot and {below} below the waterline were checked");
}

/// Flat ground 100 m high with a trench 1 m deep across the cells from `from_m` to `to_m` south
/// of the line `z = 0`, and a channel 16 m wide along that line (`on_the_flat`): its waterline
/// lies at `z = 8`.
fn with_trench(from_m: f64, to_m: f64) -> (Grid<f32>, Channel) {
    let (_, channel) = on_the_flat(16.0);
    (ground(128, |_, z| if (from_m..=to_m).contains(&z) { 99.0 } else { 100.0 }), channel)
}

/// The cells of `depth` from `from_m` to `to_m` south of the line `z = 0` that hold water.
fn wet_between(depth: &Grid<f32>, from_m: i32, to_m: i32) -> Vec<(i32, i32)> {
    (from_m..=to_m).flat_map(|z| (-64..64).map(move |x| (x, z))).filter(|&cell| at(depth, cell) > 0.0).collect()
}

#[test]
fn water_does_not_cross_a_bank() {
    let rules = WaterRules::default();
    // A trench 3 m wide from 6 m beyond the waterline, the cells at z = 14, 15 and 16: its
    // bottom lies below the surface, and within the banks' reach.
    let (before, channel) = with_trench(14.0, 16.0);
    let (flat, _) = on_the_flat(16.0);
    let channels = [channel];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    assert_eq!(found, unshaped_profiles(&channels, &flat, &on_ground(&flat), &[], &rules), "the trench moved the water");
    assert!(found[0].surface_m.iter().all(|&surface| surface > 99.0), "the surface lies below the trench's bottom");
    let (_, depth) = cut(&before, &channels, &rules);
    let wet = wet_between(&depth, 9, 20);
    assert!(wet.is_empty(), "{} cells beyond the waterline hold water, such as {:?}", wet.len(), &wet[..wet.len().min(5)]);
}

#[test]
fn a_hollow_beside_the_bed_lowers_the_water() {
    let rules = WaterRules::default();
    // The same trench touching the waterline: the cells at z = 9, 10 and 11, from a metre
    // beyond it.
    let (before, channel) = with_trench(9.0, 11.0);
    let channels = [channel];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    assert!(found[0].surface_m.iter().all(|&surface| surface <= 99.0 - 0.1 + 1e-4), "the surface: {:?}", found[0].surface_m);
    let (_, depth) = cut(&before, &channels, &rules);
    let wet = wet_between(&depth, 8, 20);
    assert!(wet.is_empty(), "{} cells from the waterline out hold water, such as {:?}", wet.len(), &wet[..wet.len().min(5)]);
    assert_water_as_wide("beside a hollow", &depth, 0, 16.0);
}

#[test]
fn water_backs_up_where_a_surface_is_raised() {
    let rules = WaterRules::default();
    let before = ground(128, |x, _| tilted(x));
    // A channel 4 m wide from the west border to x = 50 ends in lake 0, whose level stands
    // 0.5 m above the surface the channel would reach it at alone. Raised to the level, its
    // water backs up over the ground beside its bed that lies below the level: from x = 35 east,
    // across the banks' reach, 2 m + 12 m either side of its line.
    let river = straight((-64.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Lake(0));
    let alone = unshaped_profiles(&[Channel { end: ChannelEnd::Edge, ..river.clone() }], &before, &on_ground(&before), &[], &rules);
    let level = alone[0].surface_m[alone[0].surface_m.len() - 1] + 0.5;
    let channels = [river];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &[], &[level], &rules);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &[], &rules);
    for x in 35..=50 {
        for z in (3..=14).flat_map(|z| [-z, z]) {
            let (ground_m, water_m) = (at(&after, (x, z)), at(&depth, (x, z)));
            assert!(ground_m < level, "({x}, {z}) lies at {ground_m} m, not below the level {level} m");
            assert!(water_m > 0.0 && close(ground_m + water_m, level), "the water at ({x}, {z}) stands at {} m, not at the level {level} m", ground_m + water_m);
        }
        // Beyond the banks' reach no water backs up.
        assert_eq!(at(&depth, (x, 15)), 0.0, "water at ({x}, 15), beyond the banks' reach");
    }
}

/// Water stands no higher than the level it was reached through: a hollow beside a channel,
/// joined to its water only where that water backs up from the lake it ends in, holds the lake's
/// level along its whole length, not the channel's higher surface beside its upstream cells,
/// and stays dry where its floor lies above that level.
#[test]
fn water_stands_no_higher_than_the_level_it_was_reached_through() {
    let rules = WaterRules::default();
    // The ground and the river of `water_backs_up_where_a_surface_is_raised`, the river's
    // surface held up to the lake's level from x = 25 east and above it upstream, and a trench
    // 0.6 m deep and two cells wide along z = 8 and 9, from x = -10 to x = 45: beyond the foot
    // of the river's banks, with higher ground between, and below the level from x = 5 east.
    let trench = |x: f64, z: f64| (-10.0..=45.0).contains(&x) && (8.0..=9.0).contains(&z);
    let before = ground(128, |x, z| tilted(x) - if trench(x, z) { 0.6 } else { 0.0 });
    let river = straight((-64.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Lake(0));
    let flat = ground(128, |x, _| tilted(x));
    let alone = unshaped_profiles(&[Channel { end: ChannelEnd::Edge, ..river.clone() }], &flat, &on_ground(&flat), &[], &rules);
    let level = alone[0].surface_m[alone[0].surface_m.len() - 1] + 0.5;
    let channels = [river];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &[], &[level], &rules);
    assert!(found[0].surface_m[0] > level + 1.0, "the river's surface upstream: {:?}", found[0].surface_m);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &[], &rules);
    let mut held = 0;
    for x in -10..=45 {
        for z in 8..=9 {
            let (ground_m, water_m) = (at(&after, (x, z)), at(&depth, (x, z)));
            if ground_m < level {
                held += 1;
                assert!(water_m > 0.0 && close(ground_m + water_m, level), "the water at ({x}, {z}) stands at {} m, not at the level {level} m", ground_m + water_m);
            } else {
                assert_eq!(water_m, 0.0, "water at ({x}, {z}), whose floor {ground_m} m lies above the level {level} m");
            }
        }
    }
    assert!(held >= 70, "only {held} cells of the trench lie below the level");
}

#[test]
fn a_head_is_held_too() {
    let rules = WaterRules::default();
    // A plane falling 30 % across a channel 4 m wide (0.48 m deep in full) along z = 0: a
    // metre beyond its waterline, 3 m south of its line, its bank falls 0.9 m below the line, so
    // its banks hold its water at 99 m.
    let before = ground(128, |_, z| 100.0 - 0.3 * z);
    // Beginning on dry land, 34 m inside the west border, it is held from its first point, and
    // grows into its bed over its first 12 m, from no depth.
    let spring = straight((-30.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge);
    let [profile] = &unshaped_profiles(std::slice::from_ref(&spring), &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    assert!(profile.surface_m.iter().all(|&surface| close(surface, 99.0)), "the surface: {:?}", profile.surface_m);
    for (point, &depth) in spring.points.iter().zip(&profile.depth_m) {
        let along_m = point.x_m + 30.0;
        let expected = bed_depth_m(4.0, &rules) * (along_m / 12.0).min(1.0) as f32;
        assert!(close(depth, expected), "{along_m} m along the spring the bed is {depth} m deep, not {expected} m");
    }
    assert_eq!(profile.depth_m[0], 0.0);
    // Along its first 12 m, and beyond its first point, no cell beyond its downhill waterline
    // holds water.
    let (_, depth) = cut(&before, std::slice::from_ref(&spring), &rules);
    let mut checked = 0;
    for z in 1..64 {
        for x in -40..=-18 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (apart_m, half_width_m, _) = nearest_water(&spring, profile, &cell);
            if apart_m > half_width_m {
                checked += 1;
                assert_eq!(at(&depth, (x, z)), 0.0, "water at ({x}, {z}), beyond the head's downhill waterline");
            }
        }
    }
    assert!(checked > 1000, "only {checked} cells were checked");
}

/// A spring's head on a ridge whose ground falls away behind it holds none of its water behind
/// its first point: the cells straight beyond that point are neither in its bed's water nor
/// reached from it.
#[test]
fn no_water_lies_behind_a_springs_head() {
    let rules = WaterRules::default();
    // A ridge along x = -30, 100 m high, falling 0.2 m per metre to the west and 0.02 m per metre
    // to the east. A channel 4 m wide begins on its crest, 34 m inside the west border, and
    // flows east down the gentle side; its surface at its first point is the crest's height, and
    // the ground behind it lies up to 2.8 m below that within its banks' reach.
    let ridge = |x: f64| if x < -30.0 { 100.0 - 0.2 * (-30.0 - x) } else { 100.0 - 0.02 * (x + 30.0) };
    let before = ground(128, |x, _| ridge(x));
    let spring = straight((-30.0, 0.0), (50.0, 0.0), 4.0, ChannelEnd::Edge);
    let [profile] = &unshaped_profiles(std::slice::from_ref(&spring), &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    assert!(profile.on_dry_land && close(profile.surface_m[0], 100.0), "the head at {} m", profile.surface_m[0]);
    let (_, depth) = cut(&before, std::slice::from_ref(&spring), &rules);
    let behind: Vec<(i32, i32)> = ((-64..64).flat_map(|z| (-64..-30).map(move |x| (x, z)))).filter(|&cell| at(&depth, cell) > 0.0).collect();
    assert!(behind.is_empty(), "{} cells behind the head hold water, such as {:?}, {} m deep", behind.len(), &behind[..behind.len().min(5)], behind.first().map_or(0.0, |&cell| at(&depth, cell)));
    assert!(wet_across(&depth, 0).len() >= 3, "no water in the bed 30 m along");
}

/// A channel that falls along its own line is not held by it: the cap reads how far the bank
/// falls below the line's ground beside the same place along it, not the ground further down.
#[test]
fn a_course_down_a_slope_is_not_held_by_its_own_fall() {
    let rules = WaterRules::default();
    // A plane falling 40 % along a channel 4 m wide and not at all across it: from the west
    // border to the east one, so no ground lies beyond either end; and from 50 m west to 50 m
    // east, ground lying beyond both ends, uphill behind its first point and downhill past its
    // last. The cells straight beyond an end are no part of its foot.
    let before = ground(128, |x, _| 100.0 - 0.4 * x);
    for (from, to) in [((-64.0, 0.0), (63.0, 0.0)), ((-50.0, 0.0), (50.0, 0.0))] {
        let channels = [straight(from, to, 4.0, ChannelEnd::Edge)];
        let held = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
        let line_m = channels[0].points.iter().map(|point| ground_m(&before, point)).collect();
        let no_falls = [GroundRead { line_m, falls: vec![0.0; channels[0].points.len()], ..GroundRead::default() }];
        let free = worked(&channels, &unshaped(&channels), &before, &on_ground(&before), Reading::Given(&no_falls), &[], &rules).profiles;
        for (k, (&surface, &without)) in held[0].surface_m.iter().zip(&free[0].surface_m).enumerate() {
            assert!((surface - without).abs() <= 1e-3, "from {from:?}: point {k}: the cap moved the surface from {without} m to {surface} m");
        }
    }
}

/// A gully crossing a brook's line between two of its points lets none of its water out: the
/// foot is read against the chord between the line's ground at the two points, which the
/// surface follows between them, not against the gully's floor under the line.
#[test]
fn a_gully_between_two_points_lets_no_water_out() {
    let rules = WaterRules::default();
    // Flat ground 100 m high, and a brook 1.2 m wide (its bed 0.3 m deep, its wished surface
    // 0.12 m below the ground) running east along z = 0 from the west border, its points 2 m
    // apart on even x. A gully 0.5 m deep and a cell wide crosses it at x = 1, midway between
    // the points at x = 0 and x = 2, and runs on beyond its waterlines across the whole grid.
    let before = ground(128, |x, _| if x == 1.0 { 99.5 } else { 100.0 });
    let channels = [straight((-64.0, 0.0), (60.0, 0.0), 1.2, ChannelEnd::Edge)];
    let [profile] = &unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules)[..] else { panic!("one profile") };
    let k = channels[0].points.iter().position(|point| point.x_m == 0.0).expect("a point at x = 0");
    let at_gully_m = (profile.surface_m[k] + profile.surface_m[k + 1]) / 2.0;
    assert!(at_gully_m <= 99.5 - 0.1 + 1e-4, "the surface at the gully lies at {at_gully_m} m, above its floor less 0.1 m");
    // No cell beyond the foot, more than half a cell's diagonal and 1.5 m from the line, holds
    // water; the gully's cells there lie 0.5 m below the ground beside them.
    let (_, depth) = cut(&before, &channels, &rules);
    let beyond_m = HALF_WIDTH_MIN_M + BANK_FOOT_M;
    let wet: Vec<(i32, i32)> = ((-64..64_i32).flat_map(|z| (-64..64_i32).map(move |x| (x, z))))
        .filter(|&(x, z)| f64::from(z.abs()) > beyond_m && at(&depth, (x, z)) > 0.0)
        .collect();
    assert!(wet.is_empty(), "{} cells beyond the foot hold water, such as {:?}", wet.len(), &wet[..wet.len().min(5)]);
    assert!(at(&depth, (1, 0)) > 0.0, "no water in the bed where the gully crosses it");
}

/// The water inside what a channel ends on is not that channel's: a brook into a river leaves
/// no water inside the river's bed above the river's surface there, and a brook into a lake
/// none in the lake above its level.
#[test]
fn no_water_stands_above_what_a_brook_ends_on() {
    let rules = WaterRules::default();
    // A brook 1.2 m wide comes down a slope from the north into a river 8 m wide running east
    // along z = 0, both falling: the ground of `a_brook_never_climbs_to_join`, on the slope.
    let before = ground(128, |x, z| tilted(x) - 0.01 * z);
    let brook = Channel { drawn: None, ..straight((0.0, -50.0), (0.0, 0.0), 1.2, ChannelEnd::Channel(1)) };
    let river = straight((-50.0, 0.0), (50.0, 0.0), 8.0, ChannelEnd::Edge);
    let channels = [brook, river];
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &unshaped(&channels), &found, &[], &rules);
    let mut inside = 0;
    for z in -64..64 {
        for x in -64..64 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (apart_m, half_width_m, surface_m) = nearest_water(&channels[1], &found[1], &cell);
            if apart_m <= half_width_m && at(&depth, (x, z)) > 0.0 {
                inside += 1;
                let water_m = at(&after, (x, z)) + at(&depth, (x, z));
                assert!(water_m <= surface_m + 1e-3, "water at {water_m} m at ({x}, {z}) inside the river's bed, whose surface there is {surface_m} m");
            }
        }
    }
    assert!(inside > 500, "only {inside} cells inside the river's bed hold water");

    // A brook 1.2 m wide comes east from the west border, falling 0.02 m per metre, and ends 6 m
    // inside lake 0, whose level, 99.5 m, lies 0.5 m below the brook's own water where it
    // reaches the lake. The lake's cells lie 1 m below the level, east of x = 0 and, beside
    // the brook's last 10 m on land, south of it from z = 3, beyond the foot of its banks.
    let level_m = 99.5;
    let lake = |x: f64, z: f64| (x >= 0.0 && z.abs() <= 15.0) || ((-10.0..0.0).contains(&x) && (3.0..=15.0).contains(&z));
    let before = ground(128, |x, z| if lake(x, z) { f64::from(level_m) - 1.0 } else { tilted(x) });
    let half = (before.width / 2) as f64;
    let in_lake = |cell: usize| lake((cell % before.width) as f64 - half, (cell / before.width) as f64 - half);
    let lakes = [lake_where(&before, 0, level_m, lake, |_, _| false)];
    let channels = [straight((-64.0, 0.0), (6.0, 0.0), 1.2, ChannelEnd::Lake(0))];
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &lakes, &[level_m], &rules);
    assert!(found[0].surface_m.iter().take(25).all(|&surface_m| surface_m > level_m + 0.3), "the brook stands no higher than {level_m} m");
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &lakes, &rules);
    // The lake's water put back, as `add_water` puts it back.
    let mut lake_cells = 0;
    for cell in (0..after.data.len()).filter(|&cell| in_lake(cell)) {
        lake_cells += 1;
        depth.data[cell] = depth.data[cell].max(level_m - after.data[cell]);
        let water_m = after.data[cell] + depth.data[cell];
        assert!(water_m <= level_m + 1e-3, "water at {water_m} m in the lake at cell {cell}, above its level {level_m} m");
    }
    assert!(lake_cells > 1000, "only {lake_cells} cells in the lake");
}

/// A crest along `x = 0`, 100 m high, falling 0.3 m per metre to the east and 0.02 m per metre to
/// the west, with a groove along `z = 5.5`, 1.5 m deep at its middle and narrowing to nothing 2 m
/// either side of it.
fn crest_with_a_groove(x: f64, z: f64) -> f64 {
    let crest_m = if x < 0.0 { 100.0 + 0.02 * x } else { 100.0 - 0.3 * x };
    crest_m - 1.5 * (1.0 - (z - 5.5).abs() / 2.0).max(0.0)
}

/// A steep head and a brook past it, on `crest_with_a_groove` 128 m a side: a river 4 m wide that
/// begins on the crest at (0, 0.5) and falls east along `z = 0.5` to `x = 60`, its points 2 m
/// apart on even `x`, the foot of its banks the rows from `z = -3` to `-2` and from `3` to `4`;
/// and a brook 1.2 m wide that comes along the groove from the west border, passes 5 m from the
/// river's head, and at (8, 5.5) turns south to end on the river's point at (8, 0.5), 8 m below
/// its head. The brook's water stands level at what its first point gives it until it comes down
/// to the river, more than 0.8 m below the river's water along the river's first 4 m.
fn a_steep_head() -> (Grid<f32>, [Channel; 2]) {
    (ground(128, crest_with_a_groove), steep_head_channels(8.0))
}

/// The river of `a_steep_head`, and its brook coming along `z = 5.5` from the west border to
/// `x = turn_m`, where it turns south to end on the river's point at `(turn_m, 0.5)`.
fn steep_head_channels(turn_m: f64) -> [Channel; 2] {
    let river = straight((0.0, 0.5), (60.0, 0.5), 4.0, ChannelEnd::Edge);
    let along = straight((-64.0, 5.5), (turn_m, 5.5), 1.2, ChannelEnd::Channel(0));
    let down = straight((turn_m, 5.5), (turn_m, 0.5), 1.2, ChannelEnd::Channel(0));
    let points = along.points.iter().chain(&down.points[1..]).copied().collect();
    let brook = Channel { drawn: None, from_lake: None, points, end: ChannelEnd::Channel(0) };
    [river, brook]
}

/// `a_steep_head` with a mouth its banks hold: the groove ends at `x = 8`, the crest's whole
/// height east of it, and the brook keeps along `z = 5.5` to `x = 12` before it turns south to the
/// river's point at (12, 0.5). The foot of its banks beside the river is the river's own bank, at
/// or above the river's surface, and in the groove it stands higher than the river's surface
/// where the brook joins, so no junction brings the river down; the brook still stands level
/// beside the river's head, more than 0.8 m below the river's water along the river's first 4 m.
fn a_steep_head_with_a_held_mouth() -> (Grid<f32>, [Channel; 2]) {
    let crest_m = |x: f64| if x < 0.0 { 100.0 + 0.02 * x } else { 100.0 - 0.3 * x };
    (ground(128, |x, z| if x > 8.0 { crest_m(x) } else { crest_with_a_groove(x, z) }), steep_head_channels(12.0))
}

/// How far along `channel` its line's point nearest `point` lies from its last point, in metres:
/// the first of several as near.
fn to_the_end_m(channel: &Channel, point: &ChannelPoint) -> f64 {
    let (mut nearest, mut along_m) = ((f64::INFINITY, 0.0), 0.0);
    for pair in channel.points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
        let squared = dx * dx + dz * dz;
        let t = if squared > 0.0 { (((point.x_m - a.x_m) * dx + (point.z_m - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        let apart_m = (a.x_m + dx * t - point.x_m).hypot(a.z_m + dz * t - point.z_m);
        if apart_m < nearest.0 {
            nearest = (apart_m, along_m + t * squared.sqrt());
        }
        along_m += squared.sqrt();
    }
    along_m - nearest.1
}

/// `before` cut by channel `index` of `channels` alone, ending nowhere, with no lake: the bed and
/// banks it cuts with no other channel's foot to hold it off, `shapes` and `found` the channels'
/// shapes and water.
fn cut_alone(before: &Grid<f32>, channels: &[Channel], shapes: &[Shape], found: &[Profile], index: usize, rules: &WaterRules) -> Grid<f32> {
    let channel = Channel { end: ChannelEnd::Edge, ..channels[index].clone() };
    let (mut heights, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut heights, &mut depth, &[channel], &shapes[index..=index], &found[index..=index], &[], rules);
    heights
}

/// A channel worked out later cuts no bank's foot below the water it holds: in
/// `a_steep_head_with_a_held_mouth`, where the brook's bank reaches the river's foot near its head
/// and no junction brings the river down, every cell of the river's foot (no more than
/// `BANK_FOOT_M` beyond its waterline, not straight beyond either end) outside the brook's bed
/// stands at or above the river's surface at its nearest point plus `FREEBOARD_M`, less
/// `ROUNDING_M`; so does every cell as far beyond the waterline straight behind the river's head,
/// or at the ground the river alone cuts there where that lies lower (`cut_alone`), as nothing is
/// raised and a bank's own cut starts at its surface. And along the brook, more than 4 m upstream
/// of where it ends, no cell of its bed or its bank (no more than `BANK_REACH_M` beyond its
/// waterline) outside the river's bed holds water more than 0.05 m above the brook's own surface
/// at its nearest point: the river's water does not flood it.
#[test]
fn a_later_channels_bank_does_not_cut_a_held_foot() {
    let rules = WaterRules::default();
    let (before, channels) = a_steep_head_with_a_held_mouth();
    let [river, brook] = &channels;
    let found = unshaped_profiles(&channels, &before, &on_ground(&before), &[], &rules);
    let river_alone = cut_alone(&before, &channels, &unshaped(&channels), &found, 0, &rules);
    assert!(found[0].on_dry_land, "the river does not begin on dry land");
    assert_eq!(found[0], unshaped_profiles(std::slice::from_ref(river), &before, &on_ground(&before), &[], &rules)[0], "a junction brought the river down");
    // The river's and the brook's surfaces at their points at `x` = 0, 2 and 4.
    let (river_m, brook_m) = (&found[0].surface_m[..3], &found[1].surface_m[32..35]);
    assert!(river_m.iter().zip(brook_m).all(|(river_m, brook_m)| *brook_m < river_m - 0.8), "the river's surface {river_m:?} m, the brook's beside it {brook_m:?} m");
    let (after, depth) = cut(&before, &channels, &rules);
    let (mut foot, mut behind, mut beside) = (0, 0, 0);
    let (mut cut_below, mut flooded) = (Vec::new(), Vec::new());
    for z in -64..64 {
        for x in -64..64 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (apart_m, half_width_m, surface_m) = nearest_water(river, &found[0], &cell);
            let (brook_apart_m, brook_half_width_m, brook_m) = nearest_water(brook, &found[1], &cell);
            let (ground_m, alone_m) = (at(&after, (x, z)), at(&river_alone, (x, z)));
            let beyond_m = apart_m - half_width_m;
            if beyond_m > 0.0 && beyond_m <= BANK_FOOT_M && brook_apart_m > brook_half_width_m && x <= 60 {
                let held_m = surface_m + FREEBOARD_M - ROUNDING_M;
                let held_m = if x < 0 {
                    behind += 1;
                    held_m.min(alone_m)
                } else {
                    foot += 1;
                    held_m
                };
                if ground_m < held_m {
                    cut_below.push(format!("({x}, {z}) at {ground_m} m, below {held_m} m"));
                }
            }
            let water_m = ground_m + at(&depth, (x, z));
            let by_the_brook = brook_apart_m - brook_half_width_m <= BANK_REACH_M && to_the_end_m(brook, &cell) > 4.0;
            if by_the_brook && apart_m > half_width_m && at(&depth, (x, z)) > 0.0 {
                beside += 1;
                if water_m > brook_m + 0.05 {
                    flooded.push(format!("({x}, {z}): water at {water_m} m, the brook's surface {brook_m} m"));
                }
            }
        }
    }
    assert!(foot >= 240 && behind >= 10 && beside >= 250, "only {foot} cells of the foot, {behind} behind the head and {beside} wet ones by the brook were checked");
    assert!(cut_below.is_empty(), "{} cells of the river's foot are cut below its water: {:#?}", cut_below.len(), &cut_below[..cut_below.len().min(10)]);
    assert!(flooded.is_empty(), "{} cells by the brook hold water above its surface: {:#?}", flooded.len(), &flooded[..flooded.len().min(10)]);
}

/// A tributary's bed still reaches its river: in `a_steep_head`, every cell of the brook's bed
/// (no farther from its line than its half width, no less than `HALF_WIDTH_MIN_M`) whose nearest
/// point lies no more than 4 m from the brook's last point along it is cut as before the rule,
/// the lower of what the river's bed and banks alone and the brook's alone cut it to, bit for bit;
/// among them two cells of the river's foot that the brook's bed cuts below the river's surface
/// plus `FREEBOARD_M`.
#[test]
fn a_tributarys_bed_still_reaches_its_river() {
    let rules = WaterRules::default();
    let (before, channels) = a_steep_head();
    let shapes = unshaped(&channels);
    let found = found_profiles(&channels, &shapes, &before, &on_ground(&before), &[], &[], &rules);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &found, &[], &rules);
    let alone = |index: usize| cut_alone(&before, &channels, &shapes, &found, index, &rules);
    let (river_alone, brook_alone) = (alone(0), alone(1));
    let (mut checked, mut under_the_foot, mut changed) = (0, 0, Vec::new());
    for z in -64..64 {
        for x in -64..64 {
            let cell = ChannelPoint { x_m: f64::from(x), z_m: f64::from(z), width_m: 0.0, drained_m2: 0.0 };
            let (brook_apart_m, brook_half_width_m, _) = nearest_water(&channels[1], &found[1], &cell);
            if brook_apart_m > brook_half_width_m || to_the_end_m(&channels[1], &cell) > 4.0 {
                continue;
            }
            checked += 1;
            let (ground_m, expected_m) = (at(&after, (x, z)), at(&river_alone, (x, z)).min(at(&brook_alone, (x, z))));
            if ground_m.to_bits() != expected_m.to_bits() {
                changed.push(format!("({x}, {z}) at {ground_m} m, not {expected_m} m"));
            }
            let (apart_m, half_width_m, surface_m) = nearest_water(&channels[0], &found[0], &cell);
            let beyond_m = apart_m - half_width_m;
            under_the_foot += usize::from(beyond_m > 0.0 && beyond_m <= BANK_FOOT_M && ground_m < surface_m + FREEBOARD_M);
        }
    }
    assert!(checked >= 5 && under_the_foot >= 2, "only {checked} cells of the brook's mouth were checked, {under_the_foot} of them under the river's foot");
    assert!(changed.is_empty(), "{} cells of the brook's bed at its mouth are cut otherwise: {changed:#?}", changed.len());
}

/// The ground the surfaces were worked out on is the final ground: for three channels in a chain,
/// the river and the brook of `a_steep_head` and a second brook 1.2 m wide that comes north along
/// `x = 4` from the south border and ends on the brook's point at (4, 5.5), its bank reaching the
/// river's foot beyond its last point, listed upstream first, so worked out in another order than
/// the list's, the heights `cut_beds` leaves equal the copy of them the channels read as
/// `profiles` worked them out, as the last channel left it, bit for bit; and both brooks are held
/// off a foot somewhere, each cutting some cell less low than it would alone (`cut_alone`).
#[test]
fn the_final_ground_is_the_ground_the_surfaces_were_worked_out_on() {
    let rules = WaterRules::default();
    let (before, [river, brook]) = a_steep_head();
    let second = Channel { drawn: None, ..straight((4.0, 63.0), (4.0, 5.5), 1.2, ChannelEnd::Channel(1)) };
    let channels = [second, Channel { end: ChannelEnd::Channel(2), ..brook }, river];
    let shapes = unshaped(&channels);
    let worked = worked(&channels, &shapes, &before, &on_ground(&before), Reading::Cut(&[]), &[], &rules);
    let (mut after, mut depth) = (before.clone(), Grid::new(before.width, before.height, 1.0));
    cut_beds(&mut after, &mut depth, &channels, &shapes, &worked.profiles, &[], &rules);
    let half = (before.width / 2) as i32;
    let differ: Vec<String> = (0..after.data.len())
        .filter(|&cell| after.data[cell].to_bits() != worked.cut.data[cell].to_bits())
        .map(|cell| format!("({}, {}) at {} m, read at {} m", cell as i32 % 128 - half, cell as i32 / 128 - half, after.data[cell], worked.cut.data[cell]))
        .collect();
    assert!(differ.is_empty(), "{} cells differ: {:#?}", differ.len(), &differ[..differ.len().min(10)]);
    // Both brooks are held off a foot somewhere: alone, each would cut some cell lower.
    for (index, name) in [(0, "the second brook"), (1, "the brook")] {
        let alone = cut_alone(&before, &channels, &shapes, &worked.profiles, index, &rules);
        assert!(alone.data.iter().zip(&after.data).any(|(alone, is)| alone < is), "{name} is held off no foot");
    }
}

/// The level of each of an example world's drawn lakes, as `fill_lakes` found and settled it
/// (`lakes`), or NaN, as `add_water` hands them to the channels.
fn lake_levels(world: &ExampleWorld, lakes: &[LakeShape]) -> Vec<f32> {
    (0..world.drawing.lakes.len())
        .map(|drawn| lakes.iter().find(|lake| lake.drawn == drawn).map_or(f32::NAN, |lake| lake.level_m))
        .collect()
}

/// The floor under an example world's channel points (`water::floor_m`), on its `size` grid.
fn example_floor(world: &ExampleWorld, size: (usize, usize)) -> impl Fn(&ChannelPoint) -> f32 + '_ {
    move |point| floor_m(&world.land, size, (point.x_m, point.z_m))
}

/// An example world's land at 1 m with its lakes' water found, settled and their shores graded
/// (`fill_lakes`, under `rules`, with `channels` the world's channels as traced and shaped), and
/// those lakes, as `add_water` finds them before the beds.
fn example_lakes(world: &ExampleWorld, rules: &WaterRules, channels: &[Channel]) -> (Grid<f32>, Vec<LakeShape>) {
    let mut heights = at_one_metre(&world.land, world.drawing.size_m);
    let mut depth = Grid::new(heights.width, heights.height, 1.0);
    let lakes = fill_lakes(&mut heights, &mut depth, &world.land, &world.drawing, channels, rules, &mut Vec::new(), &AtomicBool::new(false))
        .expect("no flag is set");
    (heights, lakes)
}

/// An example world's water before its beds are cut, as `add_water` makes it.
struct ExampleWater {
    /// The land at 1 m with its lakes filled (`example_lakes`).
    heights: Grid<f32>,
    lakes: Vec<LakeShape>,
    /// The channels, shaped from the world's seed (`shape_channels`), their shapes and their
    /// profiles.
    channels: Vec<Channel>,
    shapes: Vec<Shape>,
    profiles: Vec<Profile>,
}

impl ExampleWater {
    /// Cuts the beds into `heights` and returns the water's depth, as `add_water` does before
    /// the lakes are put back, beside the lakes found.
    fn cut(&self, heights: &mut Grid<f32>, rules: &WaterRules) -> Grid<f32> {
        let mut depth = Grid::new(heights.width, heights.height, 1.0);
        cut_beds(heights, &mut depth, &self.channels, &self.shapes, &self.profiles, &self.lakes, rules);
        depth
    }
}

/// An example world's water under `rules`, as `add_water` makes it before the beds (`ExampleWater`).
fn example_water(world: &ExampleWorld, rules: &WaterRules) -> ExampleWater {
    let mut channels = trace_channels(&world.land, &world.drawing, rules);
    let shapes = shape_channels(&mut channels, world.seed, rules);
    let (heights, lakes) = example_lakes(world, rules, &channels);
    let floor = example_floor(world, (heights.width, heights.height));
    let profiles = found_profiles(&channels, &shapes, &heights, &floor, &lakes, &lake_levels(world, &lakes), rules);
    ExampleWater { heights, lakes, channels, shapes, profiles }
}

/// `channel`, shaped by `shape`, as it was traced: each width before the wander, its width over
/// its factor. Its wished surface is worked out from these.
fn traced(channel: &Channel, shape: &Shape) -> Channel {
    let points = (channel.points.iter().zip(&shape.factor))
        .map(|(point, &factor)| ChannelPoint { width_m: point.width_m / factor, ..*point })
        .collect();
    Channel { points, ..channel.clone() }
}

/// In the example worlds, under the shipped rules and with no wander: one profile per channel,
/// the width before the wander never falls, the surface never rises, and a channel ends at the
/// level of the lake it ends in. With no wander the bed never rises either; under the wander it
/// may, where the water narrows, deeper under the same surface.
#[test]
fn no_surface_rises_in_the_example_worlds() {
    let no_wander = WaterRules { width_wander: 0.0, bend_shift: 0.0, ..WaterRules::default() };
    let mut failures = Vec::new();
    for (rules, wanders) in [(WaterRules::default(), true), (no_wander, false)] {
        for world in example_worlds() {
            let ExampleWater { channels, shapes, profiles: found, lakes, .. } = example_water(world, &rules);
            let levels = lake_levels(world, &lakes);
            assert_eq!(found.len(), channels.len(), "{}: one profile per channel", world.name);
            for (index, ((channel, shape), profile)) in channels.iter().zip(&shapes).zip(&found).enumerate() {
                let name = format!("{} (wander {}): channel {index} ({:?})", world.name, rules.width_wander, channel.drawn);
                assert_eq!((profile.surface_m.len(), profile.depth_m.len()), (channel.points.len(), channel.points.len()), "{name}");
                let widths: Vec<f32> = traced(channel, shape).points.iter().map(|point| point.width_m).collect();
                // Rounding a width times its factor, then over it, can leave it a hair off.
                if let Some(k) = (1..widths.len()).find(|&k| widths[k] < widths[k - 1] * (1.0 - 1e-6)) {
                    failures.push(format!("{name}: the width before the wander falls from {} to {} m at point {k}", widths[k - 1], widths[k]));
                }
                let beds = beds(profile, &rules);
                if let Some(k) = (1..beds.len()).find(|&k| !wanders && beds[k] > beds[k - 1]) {
                    failures.push(format!("{name}: the bed rises from {} to {} m at point {k}", beds[k - 1], beds[k]));
                }
                if !never_rise(&profile.surface_m) {
                    failures.push(format!("{name}: the surface rises"));
                }
            // It ends at the level of the lake it ends in. (One that ends on a channel is checked
            // in `tributaries_take_their_rivers_surface_in_the_example_worlds`.)
                let end = profile.surface_m[profile.surface_m.len() - 1];
                let there = match channel.end {
                    ChannelEnd::Lake(lake) => levels.get(lake).copied().filter(|level| level.is_finite()),
                    ChannelEnd::Channel(_) | ChannelEnd::Edge => None,
                };
                if there.is_some_and(|there| (end - there).abs() > 1e-4) {
                    failures.push(format!("{name} ends at {end} m, on {:?} at {there:?} m", channel.end));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
}

/// The wished surface is worked out from the width before the wander, and only what a channel
/// reads of its ground (`GroundRead`: the falls of its banks, which read the waterline as it
/// wanders, and the ground the channels worked out before it cut, as their beds wander) and the
/// meeting with the channel it ends on, which reads that channel's bed as it wanders, read the
/// wander: in the example worlds, with what each channel read of its ground as the wander left
/// it, every channel that ends on no other keeps its surface under the wander, to within the
/// rounding of a width multiplied by its factor and divided by it again; somewhere a tributary
/// meets its river's widened or narrowed bed elsewhere; and somewhere what a channel reads as
/// the wander moves it moves a surface.
#[test]
fn the_wander_moves_a_surface_only_through_its_ground_and_at_a_junction() {
    let rules = WaterRules::default();
    let (mut failures, mut moved, mut by_the_banks) = (Vec::new(), 0, 0);
    for world in example_worlds() {
        let mut channels = trace_channels(&world.land, &world.drawing, &rules);
        let shapes = shape_channels(&mut channels, world.seed, &rules);
        let (heights, lakes) = example_lakes(world, &rules, &channels);
        assert!(shapes.iter().flat_map(|shape| &shape.factor).any(|&factor| factor != 1.0), "{}: nothing wanders", world.name);
        let as_traced = trace_channels(&world.land, &world.drawing, &rules);
        let (floor, levels) = (example_floor(world, (heights.width, heights.height)), lake_levels(world, &lakes));
        let wandered = worked(&channels, &shapes, &heights, &floor, Reading::Cut(&lakes), &levels, &rules);
        let (found, read) = (wandered.profiles, wandered.read);
        let unwandered = worked(&as_traced, &unshaped(&as_traced), &heights, &floor, Reading::Given(&read), &levels, &rules).profiles;
        let on_its_own_banks = found_profiles(&as_traced, &unshaped(&as_traced), &heights, &floor, &lakes, &levels, &rules);
        let moved_m = |a: &Profile, b: &Profile| a.surface_m.iter().zip(&b.surface_m).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        for (index, (wandered, unwandered)) in found.iter().zip(&unwandered).enumerate() {
            let off_m = moved_m(wandered, unwandered);
            match channels[index].end {
                ChannelEnd::Channel(_) => moved += usize::from(off_m > 1e-4),
                ChannelEnd::Edge | ChannelEnd::Lake(_) if off_m > 1e-4 => {
                    failures.push(format!("{}: channel {index} ({:?}): the surface moves by up to {off_m} m", world.name, channels[index].drawn));
                }
                ChannelEnd::Edge | ChannelEnd::Lake(_) => by_the_banks += usize::from(moved_m(wandered, &on_its_own_banks[index]) > 1e-4),
            }
        }
    }
    assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
    assert!(moved > 0, "no tributary meets its river's bed elsewhere under the wander");
    assert!(by_the_banks > 0, "what a channel reads of its ground as the wander moves it moves no surface");
}

/// Where a tributary enters its river's bed slanting against the flow, the river's surface at
/// its nearest points rises towards the junction and the running minimum holds the tributary
/// below it: by 21.2 mm at most (hills seed 1, a brook ending on the big river, with brooks
/// starting at 50,000 m2). A few centimetres below the river, inside its bed, cannot be seen
/// under its water and never makes the water climb; never above the river inside its bed and
/// never rising downstream stay exact (`no_tributary_stands_above_its_river_in_the_example_worlds`
/// and `no_surface_rises_in_the_example_worlds`).
const LEVEL_TOLERANCE_M: f32 = 0.05;

/// Every point of a channel from its entry point into the channel it ends on lies at that
/// channel's surface at its nearest point (the running minimum changes nothing), and its last
/// surface is that channel's at the junction, both within `LEVEL_TOLERANCE_M`, in each example
/// world.
#[test]
fn tributaries_take_their_rivers_surface_in_the_example_worlds() {
    let rules = WaterRules::default();
    let mut failures = Vec::new();
    for world in example_worlds() {
        let ExampleWater { channels, profiles: found, .. } = example_water(world, &rules);
        let mut checked = 0;
        for (index, (channel, water)) in channels.iter().zip(&found).enumerate() {
            let ChannelEnd::Channel(joined) = channel.end else { continue };
            let last = channel.points[channel.points.len() - 1];
            let junction = channels[joined].points.iter().position(|point| (point.x_m, point.z_m) == (last.x_m, last.z_m));
            // A junction on no point of the joined channel gives NaN, and fails.
            let there_m = junction.map_or(f32::NAN, |k| found[joined].surface_m[k]);
            let end_m = water.surface_m[channel.points.len() - 1];
            let off_m = (end_m - there_m).abs();
            if off_m.is_nan() || off_m > LEVEL_TOLERANCE_M {
                let name = format!("{}: channel {index} ({:?})", world.name, channel.drawn);
                failures.push(format!("{name} ends at {end_m} m, on channel {joined} at {there_m} m"));
            }
            let entry = entry_into(channel, &channels[joined], &found[joined]);
            for k in entry..channel.points.len() {
                checked += 1;
                let (_, _, river_m) = nearest_water(&channels[joined], &found[joined], &channel.points[k]);
                let off_m = (water.surface_m[k] - river_m).abs();
                if off_m.is_nan() || off_m > LEVEL_TOLERANCE_M {
                    let at = (channel.points[k].x_m, channel.points[k].z_m);
                    failures.push(format!(
                        "{}: channel {index} ({:?}) point {k} of {} at {at:?}: {} m, the river's {river_m} m",
                        world.name,
                        channel.drawn,
                        channel.points.len(),
                        water.surface_m[k]
                    ));
                }
            }
        }
        if checked == 0 {
            failures.push(format!("{}: no tributary point was checked", world.name));
        }
    }
    assert!(failures.is_empty(), "{} failures: {:#?}", failures.len(), &failures[..failures.len().min(20)]);
}

/// No channel's surface lies more than 1e-4 m above the surface of the channel it ends on, at
/// any of its points inside that channel's bed, in the example worlds.
#[test]
fn no_tributary_stands_above_its_river_in_the_example_worlds() {
    let rules = WaterRules::default();
    let mut worst: Vec<(f32, String)> = Vec::new();
    for world in example_worlds() {
        let ExampleWater { channels, profiles: found, .. } = example_water(world, &rules);
        let (mut most, mut checked) = ((f32::NEG_INFINITY, String::new()), 0);
        for (index, (channel, water)) in channels.iter().zip(&found).enumerate() {
            let ChannelEnd::Channel(joined) = channel.end else { continue };
            for (k, point) in channel.points.iter().enumerate() {
                let (apart_m, half_width_m, river_m) = nearest_water(&channels[joined], &found[joined], point);
                checked += usize::from(apart_m <= half_width_m);
                if apart_m <= half_width_m && water.surface_m[k] - river_m > most.0 {
                    most = (water.surface_m[k] - river_m, format!("channel {index} point {k} at ({}, {})", point.x_m, point.z_m));
                }
            }
        }
        assert!(checked > 0, "{}: no tributary point inside its river's bed was checked", world.name);
        worst.push((most.0, format!("{}: {}", world.name, most.1)));
    }
    let above: Vec<&(f32, String)> = worst.iter().filter(|(most, _)| *most > 1e-4).collect();
    assert!(above.is_empty(), "tributaries above their rivers inside the rivers' beds: {above:#?}");
}

#[test]
fn cutting_only_lowers() {
    let rules = WaterRules::default();
    for world in example_worlds() {
        let water = example_water(world, &rules);
        let before = &water.heights;
        let mut after = before.clone();
        let depth = water.cut(&mut after, &rules);
        let higher = before.data.iter().zip(&after.data).filter(|(was, is)| is.is_nan() || is > was).count();
        let lowered = before.data.iter().zip(&after.data).filter(|(was, is)| is < was).count();
        assert_eq!(higher, 0, "{}: {higher} cells are higher after cutting, or not a number", world.name);
        assert!(lowered > 0, "{}: no cell was cut", world.name);
        assert!(depth.data.iter().all(|value| value.is_finite() && *value >= 0.0), "{}: a depth below 0 or not finite", world.name);
        // No deeper than the deepest bed under a bank of full height. It is no bound in
        // general, as a channel held level through a sill or a bank on ground much steeper
        // than 2 in 1 can cut deeper, but the example worlds keep within it (8.30 m at most, at
        // (179, 780) on the steep face behind river 2's spring head in mountain country, seed 2;
        // 7.50 m in seed 3 and 6.83 m in seed 1 there, and no more than 2.62 m elsewhere).
        let most_m = rules.bed_depth_max_m + BANK_REACH_M as f32 / rules.bank_run;
        let deepest_m = before.data.iter().zip(&after.data).map(|(was, is)| was - is).fold(0.0_f32, f32::max);
        assert!(deepest_m <= most_m + 0.01, "{}: cut {deepest_m} m deep, more than {most_m} m", world.name);
    }
}

/// The lines of `channels`, whose water is `profiles`, as the tests read them
/// (`reach_of_water`): each point's half width no less than `HALF_WIDTH_MIN_M`.
fn channel_lines(channels: &[Channel], profiles: &[Profile]) -> Vec<Vec<LinePoint>> {
    (channels.iter().zip(profiles))
        .map(|(channel, profile)| {
            (channel.points.iter().zip(&profile.surface_m))
                .map(|(point, &surface_m)| LinePoint {
                    x_m: point.x_m,
                    z_m: point.z_m,
                    half_width_m: (f64::from(point.width_m) / 2.0).max(HALF_WIDTH_MIN_M),
                    surface_m: f64::from(surface_m),
                })
                .collect()
        })
        .collect()
}

/// For each of `channels`, whose water is `profiles` and whose lines are `lines`
/// (`channel_lines`), the cells whose water is not its (`Ends`, read with 1e-4 m of slack):
/// inside what it ends on, the bed of the channel it ends on or the cells of the lake it ends
/// in, one of `lakes`; and, for one that begins on dry land, straight beyond its first point
/// (`behind_the_head`).
fn channel_ends(size: usize, channels: &[Channel], profiles: &[Profile], lines: &[Vec<LinePoint>], lakes: &[LakeShape]) -> Vec<Ends> {
    let ends_on: Vec<Option<usize>> = (channels.iter())
        .map(|channel| match channel.end {
            ChannelEnd::Channel(joined) => Some(joined),
            ChannelEnd::Lake(_) | ChannelEnd::Edge => None,
        })
        .collect();
    (channels.iter().zip(profiles).zip(lines).zip(inside_the_lines_ended_on(size, lines, &ends_on, 1e-4)))
        .map(|(((channel, profile), line), ends)| {
            let ends = match channel.end {
                ChannelEnd::Lake(drawn) => match lakes.iter().find(|lake| lake.drawn == drawn) {
                    Some(lake) => ends.and(Ends { surely: lake.cells.clone(), maybe: lake.cells.clone() }),
                    None => ends,
                },
                ChannelEnd::Channel(_) | ChannelEnd::Edge => ends,
            };
            match profile.on_dry_land {
                true => ends.and(behind_the_head(size, line, BANK_REACH_M, 1e-4)),
                false => ends,
            }
        })
        .collect()
}

/// Water lies only where a channel's water reaches, from the channel's bed through cells beside
/// each other within reach of its banks, each reached at the lower of the level it came from and
/// its own surface where its ground lies below that, but for the cells whose water is not the
/// channel's (`channel_ends`), and stands at the highest level the channels reach it at
/// (`reach_of_water`, read with 1e-4 m of slack): in each example world, cut by
/// `cut_beds`, every wet cell's water stands there, and no dry cell is one a channel's water
/// reaches.
#[test]
fn water_stands_where_a_channels_water_reaches() {
    let rules = WaterRules::default();
    for world in example_worlds() {
        let water = example_water(world, &rules);
        let mut heights = water.heights.clone();
        let depth = water.cut(&mut heights, &rules);
        let (channels, found) = (&water.channels, &water.profiles);
        let size = heights.width;
        let lines = channel_lines(channels, found);
        let reached = reach_of_water(&heights, &lines, &channel_ends(size, channels, found, &lines, &water.lakes), 1e-4);
        let at = |cell: usize| ((cell % size) as f64 - (size / 2) as f64, (cell / size) as f64 - (size / 2) as f64);
        let (mut wet, mut failures) = (0, Vec::new());
        for (cell, ((&ground_m, &water_m), &(lowest_m, highest_m))) in heights.data.iter().zip(&depth.data).zip(&reached).enumerate() {
            if water_m > 0.0 {
                wet += 1;
                let water_m = f64::from(ground_m + water_m);
                if water_m < lowest_m - 1e-3 || water_m > highest_m + 1e-3 {
                    failures.push(format!("water at {water_m} m at {:?}, the highest surface that reaches it {lowest_m} to {highest_m} m", at(cell)));
                }
            } else if lowest_m > f64::NEG_INFINITY {
                failures.push(format!("{:?} is dry, {} m below a surface that reaches it", at(cell), lowest_m - f64::from(ground_m)));
            }
        }
        assert!(wet > 0, "{}: no water", world.name);
        assert!(failures.is_empty(), "{}: {} failures: {:#?}", world.name, failures.len(), &failures[..failures.len().min(10)]);
    }
}

/// The nearest point of `points`' line to world `(x, z)`, the first of several as near.
fn nearest_point(points: &[ChannelPoint], (x, z): (f64, f64)) -> (f64, f64) {
    let mut nearest = (f64::INFINITY, (points[0].x_m, points[0].z_m));
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dz) = (b.x_m - a.x_m, b.z_m - a.z_m);
        let squared = dx * dx + dz * dz;
        let share = if squared > 0.0 { (((x - a.x_m) * dx + (z - a.z_m) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
        let point = (a.x_m + dx * share, a.z_m + dz * share);
        let apart_m = (point.0 - x).hypot(point.1 - z);
        if apart_m < nearest.0 {
            nearest = (apart_m, point);
        }
    }
    nearest.1
}

/// Water goes past the foot of a channel's banks only where a later step raised it above the
/// surface those banks held: in each example world, every cell a channel's water surely reaches
/// (`reached_by_line`, read with 1e-4 m of slack, none inside what it ends on, `channel_ends`)
/// more than `BANK_FOOT_M` (and the slack) beyond its waterline at its nearest point has the
/// channel's final surface there more than 1 mm above the wished surface its banks held as
/// `profiles` worked it out (`wished`), both read at that nearest point, linear along the
/// stretch, the lower reading of the one against the higher of the other.
///
/// Three kinds of cell are left out, in this order, each counted and printed for each world:
/// - a cell that holds another channel's water as high, to within `LEVEL_TOLERANCE_M` as at a
///   junction (that channel's water surely reaching it, at the highest level it may stand at
///   there), or a lake's (the cell one of the
///   lake's, its level as high): that water is the other's, and would lie there without this
///   channel;
/// - a cell whose nearest point of the line lies within `BORDER_START_M` of the world's border:
///   the river runs on off the world there, or a stretch runs out along the border, where the
///   foot of its banks is cut off by the border;
/// - a mouth's cell: one within the banks' reach of the channel this one ends on, or of a
///   channel that ends on this one. A river's water laps into its tributary's cut mouth, and a
///   tributary's water lies on its river's cut bank beside the bed, where the channel worked
///   out later cut the other's foot. Over the nine worlds no more than `MOUTH_CELLS_MOST` cells
///   are so left out.
#[test]
fn water_past_a_banks_foot_was_raised() {
    /// The most cells of a mouth the test may leave out over the nine worlds (63 measured).
    const MOUTH_CELLS_MOST: usize = 70;
    let rules = WaterRules::default();
    let (mut failures, mut mouths_all) = (Vec::new(), 0);
    for world in example_worlds() {
        let mut channels = trace_channels(&world.land, &world.drawing, &rules);
        let shapes = shape_channels(&mut channels, world.seed, &rules);
        let (before, lakes) = example_lakes(world, &rules, &channels);
        let (floor, levels) = (example_floor(world, (before.width, before.height)), lake_levels(world, &lakes));
        let WorkedOut { profiles: found, wished_m, .. } = worked(&channels, &shapes, &before, &floor, Reading::Cut(&lakes), &levels, &rules);
        // The surface each channel's banks held at each point, before the running minimum and
        // the steps after it.
        let held: Vec<Profile> = (found.iter().zip(wished_m)).map(|(profile, surface_m)| Profile { surface_m, ..profile.clone() }).collect();
        let water = ExampleWater { heights: before, lakes, channels, shapes, profiles: found };
        let mut heights = water.heights.clone();
        water.cut(&mut heights, &rules);
        let (channels, found, size) = (&water.channels, &water.profiles, heights.width);
        let half = (size / 2) as f64;
        let at = |cell: usize| ((cell % size) as f64 - half, (cell / size) as f64 - half);
        let lines = channel_lines(channels, found);
        let ends = channel_ends(size, channels, found, &lines, &water.lakes);
        let near = near_lines(size, &lines, BANK_REACH_M, 1e-4);
        let near_held = near_lines(size, &channel_lines(channels, &held), BANK_REACH_M, 1e-4);
        let within_reach = |index: usize, cell: usize| {
            (near[index].binary_search_by_key(&cell, |seen| seen.cell).ok())
                .is_some_and(|place| near[index][place].distance_m - near[index][place].half_width_m.0 <= BANK_REACH_M)
        };
        // Each channel's cells its water surely reaches, and the highest level each channel's
        // water may stand at in each cell it surely reaches, by cell.
        let levels: Vec<(Vec<f64>, Vec<f64>)> = near.iter().zip(&ends).map(|(near, ends)| reached_by_line(&heights, near, ends, 1e-4)).collect();
        let surely: Vec<Vec<bool>> = levels.iter().map(|(surely, _)| surely.iter().map(|&level_m| level_m > f64::NEG_INFINITY).collect()).collect();
        let mut reaching: Vec<(usize, usize, f64)> = (near.iter().zip(&levels).enumerate())
            .flat_map(|(index, (near, (surely, maybe)))| {
                (near.iter().zip(surely).zip(maybe)).filter(|((_, surely_m), _)| **surely_m > f64::NEG_INFINITY).map(move |((seen, _), &maybe_m)| (seen.cell, index, maybe_m))
            })
            .collect();
        reaching.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        // The highest water at `cell` of any other channel than `index`, or of a lake.
        let others_m = |index: usize, cell: usize| {
            let from = reaching.partition_point(|entry| entry.0 < cell);
            (reaching[from..].iter().take_while(|entry| entry.0 == cell))
                .filter(|entry| entry.1 != index)
                .map(|entry| entry.2)
                .fold(water.lakes.iter().find(|lake| lake.cells.binary_search(&cell).is_ok()).map_or(f64::NEG_INFINITY, |lake| f64::from(lake.level_m)), f64::max)
        };
        // Whether `cell`'s nearest point of `channel`'s line lies within `BORDER_START_M` of the
        // world's border.
        let by_the_border = |channel: &Channel, cell: usize| {
            let (x, z) = nearest_point(&channel.points, at(cell));
            (half - x.abs()).min(half - z.abs()) <= BORDER_START_M
        };
        // Whether `cell` lies within the banks' reach of the channel `index` ends on, or of a
        // channel that ends on it.
        let at_a_mouth = |index: usize, cell: usize| {
            matches!(channels[index].end, ChannelEnd::Channel(joined) if within_reach(joined, cell))
                || (channels.iter().enumerate()).any(|(other, channel)| channel.end == ChannelEnd::Channel(index) && within_reach(other, cell))
        };
        // The cells left out, and the failures.
        let (mut others, mut border, mut mouths) = (0, 0, 0);
        let mut failed = (0, f64::INFINITY, String::new());
        for (index, ((near, near_held), surely)) in near.iter().zip(&near_held).zip(&surely).enumerate() {
            for ((seen, seen_held), _) in near.iter().zip(near_held).zip(surely).filter(|(_, wet)| **wet) {
                debug_assert_eq!(seen.cell, seen_held.cell, "the same line, the same cells");
                let above_m = seen.surface_m.0 - seen_held.surface_m.1;
                if seen.distance_m - seen.half_width_m.1 <= BANK_FOOT_M + 1e-4 || above_m > 1e-3 {
                    continue;
                }
                let channel = &channels[index];
                if others_m(index, seen.cell) >= seen.surface_m.0 - f64::from(LEVEL_TOLERANCE_M) {
                    others += 1;
                } else if by_the_border(channel, seen.cell) {
                    border += 1;
                } else if at_a_mouth(index, seen.cell) {
                    mouths += 1;
                } else {
                    failed.0 += 1;
                    if above_m < failed.1 {
                        (failed.1, failed.2) = (above_m, format!("{:?}, channel {index} ({:?}, ending {:?})", at(seen.cell), channel.drawn, channel.end));
                    }
                }
            }
        }
        println!(
            "{}: left out, of the cells past a bank's foot: {others} that hold another channel's or a lake's water as high, {border} whose nearest point lies by the world's border, {mouths} at a mouth",
            world.name
        );
        mouths_all += mouths;
        let (count, most_m, place) = failed;
        if count > 0 {
            failures.push(format!("{}: {count} cells, the water there as little as {most_m:.3} m above its held surface at {place}", world.name));
        }
    }
    println!("over the nine worlds: {mouths_all} cells at a mouth left out");
    assert!(failures.is_empty(), "water past a bank's foot that no later step raised: {failures:#?}");
    assert!(mouths_all <= MOUTH_CELLS_MOST, "{mouths_all} cells at a mouth left out, more than {MOUTH_CELLS_MOST}");
}
