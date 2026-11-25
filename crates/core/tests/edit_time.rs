//! What an edit costs: the example's nine edits timed in a test binary of its own, so that `cargo
//! test` runs nothing beside it (the plan's Task 7, Step 6; the design's budget of 3 s an edit at
//! 2 km), an edit beside held ground with a large hold far from it, a disc of 200 m regrown as
//! drawn inside itself (stage 2's Task 3), and a lake whose backwater the edit takes into its outline
//! (stage 2's ruling R23).

use landform_core::{example_edit, Country, Drawing, Edit, GenConfig, Lake, Land, EDIT_NAMES};
use std::sync::atomic::AtomicBool;

/// The most a whole edit of the example at 2,048 m may take, in milliseconds: the design's budget
/// (the prototype took 1.6 to 2.0 s).
const EDIT_MS_MOST: f32 = 3000.0;
/// The example's seed and size.
const SEED: u64 = 11;
const SIZE_M: u32 = 2048;
/// The prototype's held case (ii): a disc of 60 m on the brook's valley below edit `a`'s moved
/// point, which the edit is taken beside.
const ON_THE_BROOK: (f64, f64) = (111.0, 167.0);
const DISC_M: f64 = 60.0;
/// Every cell west of this `x` is held as well, in metres: 624 columns, 1,277,952 cells, all
/// outside edit `a`'s outline (the review of Task 7's case, S3).
const WEST_OF_M: f64 = -400.0;
/// The radius of the disc regrown as drawn on the brook's valley, at case (ii)'s place, in metres.
const REGROWN_M: f64 = 200.0;
/// A lake added on the example's main river whose backwater reaches past the outline, so the edit
/// runs its outline, mix, fine steps and join once more (stage 2's ruling R23); and the seed of
/// the example in mountains it is added to as well, where its backwater reaches up the brook too.
const BACKWATER_LAKE: Lake = Lake { x_m: 616.0, z_m: 156.0, radius_m: 50.0 };
const MOUNTAINS_SEED: u64 = 23;

/// Edit `name` of `land` to `edited`, its timings printed on one line; its total, in milliseconds.
fn timed(name: &str, land: &Land, edited: &Drawing) -> f32 {
    let edit = land.edit(edited, &AtomicBool::new(false)).unwrap_or_else(|error| panic!("edit {name}: {error}"));
    printed(name, &edit)
}

/// `edit`'s timings printed on one line, as edit `name`; its total, in milliseconds.
fn printed(name: &str, edit: &Edit) -> f32 {
    let t = edit.timings;
    println!(
        "edit {name}: footprint {:.1} ms, grow {:.1} ms, outline {:.1} ms, mix {:.1} ms, fine steps {:.1} ms, join {:.1} ms, total {:.1} ms",
        t.footprint_ms, t.grow_ms, t.outline_ms, t.mix_ms, t.fine_steps_ms, t.join_ms, t.total_ms
    );
    t.total_ms
}

/// Each of the example's nine edits, in hill country from seed 11, takes less than `EDIT_MS_MOST`;
/// and so does edit `a` beside case (ii)'s held disc, alone and with 1.28 M cells more held far
/// outside its outline (its tight join is found in a time that grows with the cells, not with the
/// cells times the held cells); so does a disc of 200 m regrown as drawn on the land as grown; and
/// so does a lake added on the main river whose backwater the edit takes into its outline, on the
/// example in hills and in mountains. Each edit's `EditTimings` is printed.
#[test]
fn each_of_the_examples_nine_edits_takes_less_than_three_seconds() {
    let land = Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
    let mut largest = (0.0f32, String::new());
    let mut keep_largest = |total_ms: f32, name: &str| {
        if total_ms > largest.0 {
            largest = (total_ms, name.to_string());
        }
    };
    for name in EDIT_NAMES {
        let edited = example_edit(&land.drawing, name).expect("an edit of the example");
        keep_largest(timed(name, &land, &edited), name);
    }

    let size = SIZE_M as usize;
    let place = |cell: usize| ((cell % size) as f64 - size as f64 / 2.0, (cell / size) as f64 - size as f64 / 2.0);
    let disc_of = |radius_m: f64| -> Vec<bool> { (0..size * size).map(|cell| (place(cell).0 - ON_THE_BROOK.0).hypot(place(cell).1 - ON_THE_BROOK.1) <= radius_m).collect() };
    let disc = disc_of(DISC_M);
    let west: Vec<bool> = (0..size * size).map(|cell| place(cell).0 < WEST_OF_M).collect();
    let mut beside = land.clone();
    beside.hold(&disc, true);
    let mut far = beside.clone();
    far.hold(&west, true);
    let edited = example_edit(&land.drawing, "a").expect("an edit of the example");
    for (name, held) in [("a beside the brook's held disc", &beside), ("a beside it with the west held", &far)] {
        let cells = held.marks.held.iter().filter(|&&on| on).count();
        let named = format!("{name}, {cells} cells held");
        keep_largest(timed(&named, held, &edited), &named);
    }
    let regrown = land.regrow_area(&disc_of(REGROWN_M), &AtomicBool::new(false)).expect("a disc regrown as drawn");
    let named = format!("a disc of {REGROWN_M} m regrown as drawn, {:.2} ha", regrown.outline.inside_m2 / 10_000.0);
    keep_largest(printed(&named, &regrown), &named);
    let mountains = Land::grow(&Drawing { country: Country::mountains(), ..Drawing::example_valley(SIZE_M) }, MOUNTAINS_SEED, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows in mountains");
    for (name, land) in [("a lake whose backwater is taken", &land), ("a lake whose backwater is taken, in mountains", &mountains)] {
        let mut edited = land.drawing.clone();
        edited.lakes.push(BACKWATER_LAKE);
        keep_largest(timed(name, land, &edited), name);
    }
    println!("the largest total: {:.1} ms (edit {})", largest.0, largest.1);
    assert!(largest.0 < EDIT_MS_MOST, "edit {} took {:.0} ms, beyond {EDIT_MS_MOST} ms", largest.1, largest.0);
}
