//! What a drag of the editor tool costs, timed in a test binary of its own so it runs with nothing
//! beside it: `cargo test` runs the integration test binaries one after another, and the tests of
//! the core library that grow the example's edits, run all at once beside it, made one run take up
//! to 118 ms against 10 ms alone.

use landform_core::{early_outline, example_edit, footprint, start_of, Drawing, Marks, WaterRules, REACH_MIN_M};
use std::time::Instant;

/// The most a drag may cost, in milliseconds: the edited drawing's `start_of` and then `footprint`.
const DRAG_MS_MOST: f64 = 50.0;
/// The drag is timed this many times and the least is bounded: the cost with nothing in its way.
const RUNS: usize = 3;
/// The size of the example land the drag is made on, in metres.
const SIZE_M: u32 = 2048;
/// The seed the example land is grown from.
const SEED: u64 = 11;
/// The edit dragged: the brook's point moved 30 m (`example_edit`'s `a`).
const EDIT: &str = "a";
/// Square metres in a hectare.
const M2_PER_HA: f64 = 10_000.0;

/// The milliseconds since `clock`, and the clock started again.
fn lap(clock: &mut Instant) -> f64 {
    let ms = clock.elapsed().as_secs_f64() * 1000.0;
    *clock = Instant::now();
    ms
}

/// The footprint is found while a person drags, so what a drag costs must be quick: the edited
/// drawing's `start_of` and then `footprint`, under `DRAG_MS_MOST` at 2,048 m, the least of `RUNS`
/// runs (the design's amendment 2, "so the tool still draws the outline while a person drags"; the
/// prototype's 25 to 30 ms built both starts). The old land's start is not in the bound, since a
/// tool can keep it for a sitting; it is timed and printed beside it, with `early_outline`, which a
/// drag runs after.
#[test]
fn a_drags_start_and_footprint_take_less_than_fifty_milliseconds_at_two_kilometres() {
    let old = Drawing::example_valley(SIZE_M);
    let water = WaterRules::default();
    let drawing = example_edit(&old, EDIT).expect("an edit of the example");
    let none = Marks::none(SIZE_M as usize);
    let least = |times: &[f64]| times.iter().copied().fold(f64::INFINITY, f64::min);
    let (mut alone, mut with_new_start, mut with_both_starts, mut early) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut area_m2 = 0.0;
    for _ in 0..RUNS {
        let mut clock = Instant::now();
        let old_start = start_of(&old, &water, SEED).expect("the example is valid");
        let old_ms = lap(&mut clock);
        let new_start = start_of(&drawing, &water, SEED).expect("the edited drawing is valid");
        let new_ms = lap(&mut clock);
        let found = footprint(&old_start, &new_start, &old, &drawing);
        let footprint_ms = lap(&mut clock);
        area_m2 = early_outline(drawing.size_m, &found, &none, REACH_MIN_M).inside_m2;
        early.push(lap(&mut clock));
        alone.push(footprint_ms);
        with_new_start.push(new_ms + footprint_ms);
        with_both_starts.push(old_ms + new_ms + footprint_ms);
    }
    println!(
        "edit {EDIT}, the least of {RUNS} runs: the new start and the footprint {:.1} ms (each run {with_new_start:.1?}), with the old start too {:.1} ms (each run {with_both_starts:.1?}), the footprint alone {:.2} ms; early outline {:.1} ms (each run {early:.1?}), {:.2} ha",
        least(&with_new_start),
        least(&with_both_starts),
        least(&alone),
        least(&early),
        area_m2 / M2_PER_HA
    );
    assert!(area_m2 > 0.0, "the edit has an outline");
    assert!(least(&with_new_start) < DRAG_MS_MOST, "the new start and the footprint took {with_new_start:?} ms");
}
