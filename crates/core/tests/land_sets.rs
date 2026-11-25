//! The editor tool's three sets of edits held to the design's bounds (the plan's Task 10, Step 3;
//! "Done means, restated" in `Specs/2026-11-19 Editor Tool.md`), in a test binary of their own. Each
//! run grows a land (`Land::grow`), edits it (`Land::edit`) and grows the edited drawing whole
//! (`Land::grow`), as `examples/land_edits.rs` counts them, and asserts:
//!
//! - **the land outside the outline bit for bit, both ways** (`assert_kept_both_ways`): on every
//!   cell of weight 0 the height, the ground map and every layer; every course point and every lake
//!   of the kept land at weight 0 in the edited land, and every one of the edited land at weight 0 in
//!   the kept land, each by its own values' bits (by geometry, not by its place in a list), but the
//!   one class of course point measured (`NEW_POINTS_AT_WEIGHT_0_MOST`);
//! - **no new unheld cell in the band** (`new_unheld`, against the kept land and the land grown
//!   whole), and inside the band's inner edge no new dry cell, no new lake cell over water and at
//!   most `STEPS_INSIDE_MOST` new steps, on the runs `STEPS_INSIDE_RUNS` only;
//! - **the band's ground** (`band_ground`, `beyond_span`): its bend and slope beyond the span between
//!   the kept land and the land grown whole by at most `BAND_GROUND_BEYOND_MOST` at 2,048 m and
//!   `TRIALS_BAND_BEYOND_MOST` on the trials at 1,024 m, but on a trial run with a trough cut deeper
//!   than `TROUGH_SAID_M`, which is counted and named (`DEEP_TROUGH_RUNS`);
//! - **each lake whole** (the design's W4): every lake's water, in the kept land and in the edited
//!   land, wholly at weight 0 or wholly at weight 1 (`assert_lakes_whole`);
//! - **the largest rise** of any course of the edited land at most `RISE_MOST_M`, on every set;
//! - the drawing unchanged gives the land back bit for bit.
//!
//! The 27 runs are three tests, one for each kind of country, so the five tests run side by side.

use landform_core::{
    band_ground, beyond_span, example_edit, new_unheld, standard_edit, trial_drawing, Country, CoursePoint, Drawing, Edit, GenConfig, Grid, Lake,
    LakeWater, Land, World, BAND_GROUND_BEYOND_MOST, EDIT_NAMES, TRIAL_NAMES, TROUGH_SAID_M, WATER_LAYER,
};
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;

/// The example's size and seed.
const EXAMPLE_SIZE_M: u32 = 2048;
const EXAMPLE_SEED: u64 = 11;
/// The seeds of the 27 runs, and the example's edits they make in each kind of country.
const KIND_SEEDS: [u64; 3] = [11, 23, 42];
const KIND_EDITS: [&str; 3] = ["a", "b", "c"];
/// The trial drawings' size and seed.
const TRIAL_SIZE_M: u32 = 1024;
const TRIAL_SEED: u64 = 3;

/// The most the band's bend and slope may each lie beyond the span between the kept land and the
/// land grown whole on the trials at 1,024 m, as a share of their mean: the design's bound, where
/// an outline is 32 to 81 ha of 105 (the prototype measured 2.72 % at most without a deep trough).
const TRIALS_BAND_BEYOND_MOST: f64 = 0.04;
/// The trial runs measured with a trough cut deeper than `TROUGH_SAID_M`, whose band is not held to
/// `TRIALS_BAND_BEYOND_MOST`: `shoulder`, where the land grown whole has a brook on the kept land's
/// ridge at the band's edge and the trough that lets its water out is 14.18 m deep (the prototype's
/// band bend 11.75 % beyond the span there).
const DEEP_TROUGH_RUNS: [&str; 1] = ["trials-shoulder"];
/// The most a course of an edited land may rise from one point to the next going down it, in
/// metres, on every set: the design's bound (the prototype measured at most 0.001 m after its
/// mending; these sets measure 0 on all three).
const RISE_MOST_M: f32 = 0.005;
/// A lake's water, as the land tells it (`Land::hold`, the lakes' zones): wet cells whose surface
/// lies within `LAKE_LEVEL_WITHIN_M` of the lake's level, within `LAKE_REACH_RADII` of its radii
/// and `POCKET_MARGIN_M` more of its drawn centre.
const LAKE_LEVEL_WITHIN_M: f64 = 0.002;
const LAKE_REACH_RADII: f64 = 1.5;
const POCKET_MARGIN_M: f64 = 16.0;
/// The most new steps between water cells an edit may leave inside the band's inner edge, and the
/// runs measured with one: the design's "one step more" than the land grown whole, measured on the
/// trial `wide` (a step of the mixed model's own water there, Task 6's report).
const STEPS_INSIDE_MOST: usize = 1;
const STEPS_INSIDE_RUNS: [&str; 1] = ["trials-wide"];
/// The most course points of an edited land at weight 0 that are not the kept land's, in one edit,
/// and the runs measured with one: a point of the band next to the outline that the join moves
/// toward its kept line onto a cell of weight 0 just outside it, beside its course's next point in
/// the band (the review of Task 6, N1): on `repeated`, 0.30 m from the kept line.
const NEW_POINTS_AT_WEIGHT_0_MOST: usize = 1;
const NEW_POINT_RUNS: [&str; 1] = ["trials-repeated"];

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// One run of a set: a land grown from `drawing` and `seed`, edited to `edited`, named as the
/// program names it.
struct Run {
    name: String,
    drawing: Drawing,
    seed: u64,
    edited: Drawing,
}

/// The bounds a set is held to.
struct Bounds {
    band_beyond_most: f64,
    /// Whether a run with a trough cut deeper than `TROUGH_SAID_M` is let off the band's bound.
    deep_troughs_let_off: bool,
    rise_most_m: Option<f32>,
}

const AT_2_KM: Bounds = Bounds { band_beyond_most: BAND_GROUND_BEYOND_MOST, deep_troughs_let_off: false, rise_most_m: Some(RISE_MOST_M) };
const TRIALS: Bounds = Bounds { band_beyond_most: TRIALS_BAND_BEYOND_MOST, deep_troughs_let_off: true, rise_most_m: Some(RISE_MOST_M) };

/// The example's runs in the kind of country `kind`: edits `a`, `b` and `c` from each of
/// `KIND_SEEDS`.
fn kind_runs(kind: &str) -> Vec<Run> {
    let drawing = Drawing { country: Country::named(kind).expect("a kind of country"), ..Drawing::example_valley(EXAMPLE_SIZE_M) };
    let mut runs = Vec::new();
    for seed in KIND_SEEDS {
        for name in KIND_EDITS {
            let edited = example_edit(&drawing, name).expect("an edit of the example");
            runs.push(Run { name: format!("kinds-{name}-{kind}-{seed}"), drawing: drawing.clone(), seed, edited });
        }
    }
    runs
}

/// The weight of the world cell a place lies on, as the join reads a course point's.
fn weight_at(weight: &Grid<f32>, x: f32, z: f32) -> f32 {
    let cell = |at: f32, count: usize| ((f64::from(at) + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
    weight.data[cell(z, weight.height) * weight.width + cell(x, weight.width)]
}

fn point_bits(point: &CoursePoint) -> [u32; 5] {
    [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m].map(f32::to_bits)
}

/// The lake of `edited` that the kept drawing's lake `index` is, as an edit pairs a kept lake: the
/// first at the same place with the same radius not taken by an earlier kept lake.
fn lake_partner(kept: &[Lake], edited: &Drawing, index: usize) -> Option<usize> {
    let mut taken = vec![false; edited.lakes.len()];
    for (drawn, lake) in kept.iter().enumerate().take(index + 1) {
        let found = (0..edited.lakes.len()).find(|&n| !taken[n] && edited.lakes[n] == *lake);
        if let Some(n) = found {
            taken[n] = true;
        }
        if drawn == index {
            return found;
        }
    }
    None
}

/// W1 both ways (the plan's Global Constraints): every world cell `edit` weighs 0 keeps `before`'s
/// height, ground and every layer bit for bit, the edited land holding the same layers; every
/// course point of `before` at weight 0 is one of the edited land's and every one of the edited
/// land's at weight 0 is one of `before`'s, by their five values' bits, but at most
/// `NEW_POINTS_AT_WEIGHT_0_MOST` of the measured class (each beside a point of its course at weight
/// above 0); and every lake of either land whose middle lies at weight 0 is one of the other's by
/// its level, place and area's bits, naming its partner in the edited drawing. Returns what it
/// checked.
fn assert_kept_both_ways(name: &str, before: &Land, edit: &Edit) -> Kept {
    let (weight, old, new) = (&edit.outline.weight, &before.world, &edit.land.world);
    assert_eq!((weight.width, weight.height), (old.heights.width, old.heights.height), "{name}: the weight's size");
    let outside: Vec<usize> = (0..weight.data.len()).filter(|&cell| weight.data[cell] == 0.0).collect();
    let same = |what: &str, a: &Grid<f32>, b: &Grid<f32>| {
        let differ = outside.iter().filter(|&&cell| a.data[cell].to_bits() != b.data[cell].to_bits()).count();
        assert_eq!(differ, 0, "{name}: {differ} cells of weight 0 differ in {what}");
    };
    same("height", &new.heights, &old.heights);
    let ground = outside.iter().filter(|&&cell| new.ground.data[cell] != old.ground.data[cell]).count();
    assert_eq!(ground, 0, "{name}: {ground} cells of weight 0 differ in the ground map");
    assert!(new.layers.keys().eq(old.layers.keys()), "{name}: the layers' names");
    for (layer, grid) in &old.layers {
        same(layer, &new.layers[layer], grid);
    }

    let at_0 = |point: &CoursePoint| weight_at(weight, point.x_m, point.z_m) == 0.0;
    let points_of = |world: &World| -> BTreeSet<[u32; 5]> { world.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect() };
    let (old_points, new_points) = (points_of(old), points_of(new));
    let kept_outside: Vec<&CoursePoint> = old.water.rivers.iter().flat_map(|course| &course.points).filter(|point| at_0(point)).collect();
    let missing = kept_outside.iter().filter(|point| !new_points.contains(&point_bits(point))).count();
    assert_eq!(missing, 0, "{name}: {missing} of the kept land's {} course points at weight 0 are missing", kept_outside.len());
    let mut moved = 0;
    for course in &new.water.rivers {
        for (k, point) in course.points.iter().enumerate().filter(|&(_, point)| at_0(point) && !old_points.contains(&point_bits(point))) {
            moved += 1;
            let beside_the_band = [k.wrapping_sub(1), k + 1].into_iter().filter_map(|n| course.points.get(n)).any(|next| !at_0(next));
            assert!(beside_the_band, "{name}: a course point at weight 0 not the kept land's, at ({}, {}), away from the band", point.x_m, point.z_m);
        }
    }
    assert!(moved <= NEW_POINTS_AT_WEIGHT_0_MOST, "{name}: {moved} course points at weight 0 are not the kept land's");

    let lake_bits = |lake: &LakeWater| [lake.level_m, lake.x_m, lake.z_m, lake.area_m2].map(f32::to_bits);
    let lakes_outside = |world: &World| -> Vec<LakeWater> { world.water.lakes.iter().filter(|lake| weight_at(weight, lake.x_m, lake.z_m) == 0.0).copied().collect() };
    let (old_lakes, new_lakes) = (lakes_outside(old), lakes_outside(new));
    for lake in &old_lakes {
        let partner = lake_partner(&before.drawing.lakes, &edit.land.drawing, lake.drawn);
        assert!(
            new.water.lakes.iter().any(|other| lake_bits(other) == lake_bits(lake) && Some(other.drawn) == partner),
            "{name}: the kept lake {lake:?} at weight 0 is not in the edited land"
        );
    }
    for lake in &new_lakes {
        assert!(
            old.water.lakes.iter().any(|other| lake_bits(other) == lake_bits(lake) && lake_partner(&before.drawing.lakes, &edit.land.drawing, other.drawn) == Some(lake.drawn)),
            "{name}: the edited land's lake {lake:?} at weight 0 is not the kept land's"
        );
    }
    Kept { cells: outside.len(), points: kept_outside.len(), lakes: (old_lakes.len(), new_lakes.len()), moved }
}

/// The cells of drawn lake `drawn`'s water in `world`, as the land tells a lake's water (the
/// constants above), in the world's cell order; none when the world holds no water for it.
fn lake_cells(world: &World, drawing: &Drawing, drawn: usize) -> Vec<usize> {
    let (Some(lake), Some(water)) = (drawing.lakes.get(drawn), world.water.lakes.iter().find(|water| water.drawn == drawn)) else { return Vec::new() };
    let (size, depth) = (world.heights.width, &world.layers[WATER_LAYER]);
    let (half, reach_m) = (size as f64 / 2.0, LAKE_REACH_RADII * f64::from(lake.radius_m) + POCKET_MARGIN_M);
    let span = |at: f64| ((at - reach_m + half).floor().max(0.0) as usize)..((at + reach_m + half).ceil().max(0.0) as usize + 1).min(size);
    let mut cells = Vec::new();
    for j in span(lake.z_m) {
        for i in span(lake.x_m) {
            let cell = j * size + i;
            let surface_m = f64::from(world.heights.data[cell] + depth.data[cell]);
            if depth.data[cell] > 0.0 && (i as f64 - half - lake.x_m).hypot(j as f64 - half - lake.z_m) <= reach_m && (surface_m - f64::from(water.level_m)).abs() < LAKE_LEVEL_WITHIN_M {
                cells.push(cell);
            }
        }
    }
    cells
}

/// W4 on a real land (the whole-branch review's S9): every lake's water, in the kept land `before`
/// and in the edited land, lies wholly at weight 0 or wholly at weight 1 of `edit`'s outline, so no
/// lake cell lies in the band and none of a lake inside the outline weighs below 1. Returns, for
/// each land, how many lakes hold water and how many of them lie at weight 1.
fn assert_lakes_whole(name: &str, before: &Land, edit: &Edit) -> [(usize, usize); 2] {
    let weight = &edit.outline.weight;
    [("kept", before), ("edited", &edit.land)].map(|(which, land)| {
        let (mut lakes, mut inside) = (0, 0);
        for water in &land.world.water.lakes {
            let cells = lake_cells(&land.world, &land.drawing, water.drawn);
            let at = |keep: fn(f32) -> bool| cells.iter().filter(|&&cell| keep(weight.data[cell])).count();
            let (at_0, at_1) = (at(|w| w == 0.0), at(|w| w == 1.0));
            assert!(
                at_0 + at_1 == cells.len() && (at_0 == 0 || at_1 == 0),
                "{name}: the {which} land's lake {} is not whole: of its {} cells {at_0} weigh 0 and {at_1} weigh 1",
                water.drawn + 1,
                cells.len()
            );
            lakes += usize::from(!cells.is_empty());
            inside += usize::from(at_1 > 0);
        }
        (lakes, inside)
    })
}

/// What `assert_kept_both_ways` checked: the cells of weight 0, the kept land's course points at
/// weight 0, the lakes at weight 0 of the kept land and of the edited land, and the edited land's
/// course points at weight 0 that are not the kept land's.
struct Kept {
    cells: usize,
    points: usize,
    lakes: (usize, usize),
    moved: usize,
}

/// Each run of `runs` through `Land::grow`, `Land::edit` and `Land::grow` of the edited drawing,
/// held to `bounds` and the bounds every set shares (the module's list), each run's measures
/// printed on one line.
fn hold_the_counts(runs: &[Run], bounds: &Bounds) {
    let config = GenConfig::default();
    let mut kept: Option<(&Drawing, u64, Land)> = None;
    let (mut steps_inside, mut deep_troughs, mut new_points) = (Vec::new(), Vec::new(), Vec::new());
    for run in runs {
        if kept.as_ref().is_none_or(|(drawing, seed, _)| **drawing != run.drawing || *seed != run.seed) {
            kept = Some((&run.drawing, run.seed, Land::grow(&run.drawing, run.seed, &config, &never()).expect("the set's drawing grows")));
        }
        let before = &kept.as_ref().expect("a kept land").2;
        let name = run.name.as_str();
        let edit = before.edit(&run.edited, &never()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let whole = Land::grow(&run.edited, run.seed, &config, &never()).expect("the edited drawing grows");

        let kept_outside = assert_kept_both_ways(name, before, &edit);
        if kept_outside.moved > 0 {
            new_points.push(name);
        }
        let [kept_lakes, edited_lakes] = assert_lakes_whole(name, before, &edit);
        if run.edited == run.drawing {
            assert!(edit.land == *before, "{name}: the drawing unchanged gives the land back");
        }

        let new = new_unheld(&edit.land.world, &before.world, &whole.world, &edit.outline.weight).expect("lands hold the water layer");
        assert_eq!(new.band(), 0, "{name}: new unheld cells in the band: {new:?}");
        assert_eq!((new.dry[2], new.lake_over_water[2]), (0, 0), "{name}: new dry cells or lake cells over water inside: {new:?}");
        assert!(new.steps[2] <= STEPS_INSIDE_MOST, "{name}: {} new steps inside", new.steps[2]);
        if new.steps[2] > 0 {
            steps_inside.push(name);
        }

        let weight = &edit.outline.weight;
        let (edited, old, grown) = (band_ground(&edit.land.world, weight), band_ground(&before.world, weight), band_ground(&whole.world, weight));
        let (bend, slope) = (beyond_span(edited.bend, old.bend, grown.bend), beyond_span(edited.slope, old.slope, grown.slope));
        let deep = edit.troughs.iter().any(|trough| !trough.filled && trough.depth_m > TROUGH_SAID_M);
        if bounds.deep_troughs_let_off && deep {
            deep_troughs.push(name);
        } else {
            assert!(
                bend <= bounds.band_beyond_most && slope <= bounds.band_beyond_most,
                "{name}: the band's bend lies {:.3} % and its slope {:.3} % beyond the span, beyond {} %",
                bend * 100.0,
                slope * 100.0,
                bounds.band_beyond_most * 100.0
            );
        }

        let rise = (edit.land.world.water.rivers.iter())
            .flat_map(|course| course.points.windows(2).map(|pair| pair[1].surface_m - pair[0].surface_m))
            .fold(0.0, f32::max);
        if let Some(most) = bounds.rise_most_m {
            assert!(rise <= most, "{name}: a course rises {rise} m");
        }
        println!(
            "{name}: kept {} cells, {} course points ({} moved onto weight 0) and {} / {} lakes at weight 0; lakes whole, kept {} ({} at weight 1), edited {} ({} at weight 1); \
             {:.2} ha, band {} cells, bend {:.3} % and slope {:.3} % beyond the span, new unheld {}/{}/{} (steps inside {}), rise {rise:.5} m, troughs {:?}",
            kept_outside.cells,
            kept_outside.points,
            kept_outside.moved,
            kept_outside.lakes.0,
            kept_outside.lakes.1,
            kept_lakes.0,
            kept_lakes.1,
            edited_lakes.0,
            edited_lakes.1,
            edit.outline.inside_m2 / 10_000.0,
            edited.cells,
            bend * 100.0,
            slope * 100.0,
            new.outside(),
            new.band(),
            new.inside(),
            new.steps[2],
            edit.troughs
        );
    }
    let named = |found: &[&str], allowed: &[&str], what: &str| {
        assert!(found.iter().all(|name| allowed.contains(name)), "{what} on {found:?}, measured on {allowed:?} only");
        println!("{what}: {} runs {found:?}", found.len());
    };
    named(&steps_inside, &STEPS_INSIDE_RUNS, "a new step inside");
    named(&deep_troughs, &DEEP_TROUGH_RUNS, "a trough cut deeper than 5 m, the band let off its bound");
    named(&new_points, &NEW_POINT_RUNS, "a course point moved onto weight 0");
}

/// The example at 2,048 m in hills from seed 11: its nine edits and the drawing unchanged.
#[test]
fn the_examples_nine_edits_and_its_drawing_unchanged_hold_the_counts() {
    let drawing = Drawing::example_valley(EXAMPLE_SIZE_M);
    let mut runs: Vec<Run> = (EDIT_NAMES.iter())
        .map(|name| Run { name: format!("example-{name}"), drawing: drawing.clone(), seed: EXAMPLE_SEED, edited: example_edit(&drawing, name).expect("an edit of the example") })
        .collect();
    runs.push(Run { name: "example-unchanged".to_string(), drawing: drawing.clone(), seed: EXAMPLE_SEED, edited: drawing });
    hold_the_counts(&runs, &AT_2_KM);
}

/// The example at 2,048 m in gentle country: edits `a`, `b` and `c` from seeds 11, 23 and 42.
#[test]
fn the_example_in_gentle_country_holds_the_counts() {
    hold_the_counts(&kind_runs("gentle"), &AT_2_KM);
}

/// The example at 2,048 m in hill country: edits `a`, `b` and `c` from seeds 11, 23 and 42.
#[test]
fn the_example_in_hill_country_holds_the_counts() {
    hold_the_counts(&kind_runs("hills"), &AT_2_KM);
}

/// The example at 2,048 m in mountains: edits `a`, `b` and `c` from seeds 11, 23 and 42, among them
/// the prototype's run whose brooks were paired too far apart (`b`, seed 23).
#[test]
fn the_example_in_mountains_holds_the_counts() {
    hold_the_counts(&kind_runs("mountains"), &AT_2_KM);
}

/// The 17 trial drawings at 1,024 m in hills from seed 3, each with its standard edit.
#[test]
fn the_17_trials_hold_the_counts() {
    let runs: Vec<Run> = (TRIAL_NAMES.iter())
        .map(|name| {
            let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::hills()).expect("a trial drawing");
            let edited = standard_edit(&drawing).expect("its standard edit");
            Run { name: format!("trials-{name}"), drawing, seed: TRIAL_SEED, edited }
        })
        .collect();
    hold_the_counts(&runs, &TRIALS);
}
