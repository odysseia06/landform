//! What a step of undo costs (stage 2's Task 2, Step 4): on the example at 2,048 m in hill country
//! from seed 11, the step of each of the example's nine edits, of a change of seed, of a disc of
//! 200 m regrown as drawn where its ground changes (round a disc of 60 m held through a change of
//! seed, the hold lifted, then regrown inside itself, stage 2's Task 3; stage 2's ruling R3) and
//! of a hold of a disc of 60 m: what it holds in memory, the time of `Change::between` and of
//! `Change::swap`, each the least of three, printed and bounded, and how many such steps the
//! history's budget holds (the plan's "Decided while planning" 4). In a test binary of its own, so
//! that `cargo test` runs nothing beside it. Each step is swapped in once more and the land's file
//! compared with the land after, byte for byte.

use landform_core::{example_edit, Change, Drawing, GenConfig, Land, EDIT_NAMES};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// The most `Change::between` may take at 2,048 m, in milliseconds, the least of `RUNS`: an undo
/// step is worked out on a worker after the edit, so about a seventh of the edit (1.4 to 1.5 s) and
/// a fifteenth of the design's budget of 3 s.
const BETWEEN_MS_MOST: f64 = 200.0;
/// The most `Change::swap` may take at 2,048 m, in milliseconds, the least of `RUNS`: it runs when
/// the person undoes, redoes or takes a finished regrowth, before the land is shown again.
const SWAP_MS_MOST: f64 = 100.0;
/// How many times each is timed.
const RUNS: usize = 3;
/// The history's budget of undo steps, in bytes: 1,024 MB (the plan's "Decided while planning" 4).
const BUDGET_BYTES: usize = 1024 << 20;
/// Bytes in a megabyte, for printing.
const MB: f64 = (1 << 20) as f64;
/// The example's seed and size, and the seed the land is regrown from.
const SEED: u64 = 11;
const SIZE_M: u32 = 2048;
const OTHER_SEED: u64 = 23;
/// The prototype's held case (ii), on the brook's valley: where the disc is regrown and held.
const ON_THE_BROOK: (f64, f64) = (111.0, 167.0);
/// The radius of the disc regrown as drawn, in metres. A disc of `HELD_M` at its middle is held
/// first through the change of seed and lifted, so its ground differs from the drawing grown whole
/// from that seed where it is regrown: seed 11's land in its middle, the change of seed's band round
/// it. Held whole through the change of seed, the disc of 200 m is refused: lake 1 lies within
/// `BAND_M` of it.
const REGROWN_M: f64 = 200.0;
/// The radius of the disc held, in metres (the prototype's held discs), alone and through the change
/// of seed before the disc of `REGROWN_M` is regrown.
const HELD_M: f64 = 60.0;

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// One flag a world cell: the cells whose place lies within `radius_m` of `(x, z)`.
fn disc((x, z): (f64, f64), radius_m: f64) -> Vec<bool> {
    let size = SIZE_M as usize;
    let half = size as f64 / 2.0;
    (0..size * size).map(|cell| ((cell % size) as f64 - half - x).hypot((cell / size) as f64 - half - z) <= radius_m).collect()
}

/// The least of `RUNS` timings of `work`, in milliseconds, and what its last run gave.
fn least<T>(mut work: impl FnMut() -> T) -> (f64, T) {
    let mut least_ms = f64::INFINITY;
    let mut last = None;
    for _ in 0..RUNS {
        let clock = Instant::now();
        let made = work();
        least_ms = least_ms.min(clock.elapsed().as_secs_f64() * 1000.0);
        last = Some(made);
    }
    (least_ms, last.expect("at least one run"))
}

/// The step from `land` to `after` timed and bounded, swapped in once more and checked, and printed
/// on one line with `said`, what the change was; its bytes and the two times.
fn measure(name: &str, said: &str, land: &Land, after: &Land) -> (usize, f64, f64) {
    let (between_ms, change) = least(|| Change::between(land, after).unwrap_or_else(|error| panic!("{name}: {error}")));
    let mut step = change.clone();
    let mut swapped = land.clone();
    let mut swap_ms = f64::INFINITY;
    for _ in 0..RUNS {
        let clock = Instant::now();
        step.swap(&mut swapped);
        swap_ms = swap_ms.min(clock.elapsed().as_secs_f64() * 1000.0);
    }
    assert!(swapped.to_bytes() == after.to_bytes(), "{name}: swapped in three times, the land after, byte for byte");
    let bytes = change.bytes();
    println!(
        "{name}: {said}; step {:.2} MB, {} fit in 1,024 MB; between {between_ms:.1} ms, swap {swap_ms:.1} ms",
        bytes as f64 / MB,
        BUDGET_BYTES / bytes
    );
    assert!(between_ms < BETWEEN_MS_MOST, "{name}: between took {between_ms:.1} ms");
    assert!(swap_ms < SWAP_MS_MOST, "{name}: swap took {swap_ms:.1} ms");
    (bytes, between_ms, swap_ms)
}

/// Every step of the example at 2,048 m is worked out in less than `BETWEEN_MS_MOST` and swapped in
/// less than `SWAP_MS_MOST`, the least of three each, and swapped in it gives the land after, byte
/// for byte; what each holds and how many fit in the budget is printed.
#[test]
fn a_step_of_undo_is_worked_out_and_swapped_in_well_within_an_edit() {
    let land = Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &never()).expect("the example grows");
    let mut largest = (0, 0.0f64, 0.0f64);
    let mut keep = |(bytes, between_ms, swap_ms): (usize, f64, f64)| {
        largest = (largest.0.max(bytes), largest.1.max(between_ms), largest.2.max(swap_ms));
    };
    let inside = |edit: &landform_core::Edit| format!("{:.1} ha inside the outline", edit.outline.inside_m2 / 10_000.0);
    for name in EDIT_NAMES {
        let edited = example_edit(&land.drawing, name).expect("an edit of the example");
        let edit = land.edit(&edited, &never()).unwrap_or_else(|error| panic!("edit {name}: {error}"));
        keep(measure(&format!("edit {name}"), &inside(&edit), &land, &edit.land));
    }
    let reseeded = land.reseed(OTHER_SEED, &never()).expect("the land from seed 23");
    keep(measure("seed 23", &inside(&reseeded), &land, &reseeded.land));
    let held_middle = disc(ON_THE_BROOK, HELD_M);
    let mut held_disc = land.clone();
    held_disc.hold(&held_middle, true);
    let mut lifted = held_disc.reseed(OTHER_SEED, &never()).expect("seed 23 round the held disc").land;
    lifted.hold(&held_middle, false);
    let regrown = lifted.regrow_area(&disc(ON_THE_BROOK, REGROWN_M), &never()).expect("the disc regrown as drawn");
    keep(measure(&format!("a disc of {REGROWN_M} m regrown where its ground changes"), &inside(&regrown), &lifted, &regrown.land));
    let mut held = land.clone();
    let cells = held.hold(&disc(ON_THE_BROOK, HELD_M), true);
    keep(measure(&format!("a hold of {HELD_M} m"), &format!("{cells} cells held"), &land, &held));
    println!(
        "largest: step {:.2} MB, between {:.1} ms, swap {:.1} ms",
        largest.0 as f64 / MB,
        largest.1,
        largest.2
    );
}
