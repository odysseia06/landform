//! The editor tool's land and its edit (`Land`, the plan's Task 7): a land grown, edited, held,
//! regrown from another seed and in an area, and what an edit refuses, on the example at 2,048 m in
//! hill country from seed 11 unless a test says otherwise. Every edit taken is checked outside its
//! outline and on held ground bit for bit (`assert_kept`), its water by geometry; every edit refused,
//! cancelled or failed leaves the land given bit for bit as it was (`differences`). A step of undo
//! (`Change`, stage 2's Task 2) between a land and the land after it is swapped in and out, the
//! land's file compared byte for byte each time.

use landform_core::{
    area_outline, band_ground, beyond_span, early_area_outline, early_outline, example_edit, finish_drawn, footprint, generate_drawn, grow, hill_edit, join_water, model_water, new_unheld,
    standard_edit, start_of, trial_drawing, unheld, About, Change, Country, CoursePoint, Drawing, Edit, Frame, GenConfig, Grid, Held, Lake, LakeWater, Land, LandformError, Marks, NewUnheld, River, RiverCourse,
    Touched, lake_zones, outline, World, BAND_GROUND_BEYOND_MOST, BAND_M, EDIT_NAMES, GROW_CELL_M, GROW_MARGIN_M, LAKE_ZONE_MARGIN_M, REACH_MAX_M, REACH_MIN_M, TROUGH_SAID_M, WATER_LAYER,
};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

/// The example's seed and size.
const SEED: u64 = 11;
const SIZE_M: u32 = 2048;
/// The radius of the prototype's held discs (`report2.md`, table 8).
const DISC_M: f64 = 60.0;
/// The prototype's held case (i): a disc whose edge lies 100 m from the brook's moved stretch.
const BESIDE_THE_BROOK: (f64, f64) = (-64.0, 416.0);
/// The prototype's held case (ii): a disc on the brook's valley, 250 m below its moved point.
const ON_THE_BROOK: (f64, f64) = (111.0, 167.0);
/// A disc on the main river where it crosses edit `a`'s outline, east of the brook's mouth: more
/// than `BAND_M` from the brook's cells in the footprint and from the lake's zone.
const ON_THE_MAIN_RIVER: (f64, f64) = (300.0, 4.0);
/// A hill painted 100 m beyond the edge of case (i)'s disc, west of it, away from every river.
const HILL_BESIDE_THE_DISC: (f64, f64) = (-224.0, 416.0);
/// Where edit `d` paints its hill, and a disc held on its top.
const HILL_TOP: (f64, f64) = (700.0, -700.0);
const HILL_TOP_DISC_M: f64 = 40.0;
/// A disc far from every river and lake of the example, held through a change of seed.
const FAR_FROM_THE_WATER: (f64, f64) = (-700.0, 600.0);
/// The seed the land is regrown from.
const OTHER_SEED: u64 = 23;
/// The cancel flag is set from another thread this long into an edit: in the growing at 2 km, then
/// in the fine steps (the prototype's whole edit took 1.6 to 2.0 s, its growing 0.8 s).
const CANCEL_AFTER_MS: [u64; 2] = [300, 1000];

fn never() -> AtomicBool {
    AtomicBool::new(false)
}

/// The example land, grown once for every test that reads it.
fn example() -> &'static Land {
    static LAND: OnceLock<Land> = OnceLock::new();
    LAND.get_or_init(|| Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &never()).expect("the example grows"))
}

fn edit_of(land: &Land, name: &str) -> Drawing {
    example_edit(&land.drawing, name).unwrap_or_else(|| panic!("{name} is an edit of the example"))
}

/// One flag a world cell of a land `size` cells a side: the cells whose place lies within `radius_m`
/// of `(x, z)`.
fn disc(size: usize, (x, z): (f64, f64), radius_m: f64) -> Vec<bool> {
    (0..size * size)
        .map(|cell| {
            let (cx, cz) = place(size, cell);
            (cx - x).hypot(cz - z) <= radius_m
        })
        .collect()
}

/// World `(x, z)` of world cell `cell` of a land `size` cells a side.
fn place(size: usize, cell: usize) -> (f64, f64) {
    let half = size as f64 / 2.0;
    ((cell % size) as f64 - half, (cell / size) as f64 - half)
}

/// `land` with the discs held.
fn holding(land: &Land, discs: &[((f64, f64), f64)]) -> Land {
    let mut held = land.clone();
    let size = land.drawing.size_m as usize;
    for &(at, radius_m) in discs {
        held.hold(&disc(size, at, radius_m), true);
    }
    held
}

/// The weight of the world cell a place lies on, as the join reads a course point's.
fn weight_at(weight: &Grid<f32>, x: f64, z: f64) -> f32 {
    let cell = |at: f64, count: usize| ((at + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
    weight.data[cell(z, weight.height) * weight.width + cell(x, weight.width)]
}

fn point_bits(point: &CoursePoint) -> [u32; 5] {
    [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m].map(f32::to_bits)
}

fn bits(data: &[f32]) -> Vec<u32> {
    data.iter().map(|value| value.to_bits()).collect()
}

/// The parts of a world that differ from another's bit for bit, but its timings.
fn world_differences(a: &World, b: &World) -> Vec<String> {
    let mut parts = Vec::new();
    let mut differ = |name: &str, same: bool| {
        if !same {
            parts.push(name.to_string());
        }
    };
    differ("heights", bits(&a.heights.data) == bits(&b.heights.data) && (a.heights.width, a.heights.height) == (b.heights.width, b.heights.height));
    differ("ground", a.ground == b.ground);
    differ("layer names", a.layers.keys().eq(b.layers.keys()));
    for (name, layer) in &a.layers {
        differ(name, b.layers.get(name).is_some_and(|other| bits(&layer.data) == bits(&other.data)));
    }
    let course = |world: &World| -> Vec<_> { world.water.rivers.iter().map(|c| (c.drawn, c.from_lake, c.end, c.points.iter().map(point_bits).collect::<Vec<_>>())).collect() };
    differ("courses", course(a) == course(b));
    let lakes = |world: &World| -> Vec<_> { world.water.lakes.iter().map(|l| (l.drawn, [l.level_m, l.x_m, l.z_m, l.area_m2].map(f32::to_bits))).collect() };
    differ("lakes", lakes(a) == lakes(b));
    differ("notes", a.notes == b.notes && a.notes_about == b.notes_about);
    differ("meta", a.meta == b.meta && a.meta.base_elevation_m.to_bits() == b.meta.base_elevation_m.to_bits());
    parts
}

/// The parts of a land that differ from another's bit for bit, but its world's timings.
fn differences(a: &Land, b: &Land) -> Vec<String> {
    // Every field is named, so a field added to the land fails to compile until it is compared.
    let Land { drawing: _, asked: _, seed: _, config: _, model_heights: _, first_lake_levels: _, world: _, marks: _, reach_m: _ } = a;
    let mut parts = world_differences(&a.world, &b.world);
    let lakes = |land: &Land| -> Vec<_> { land.first_lake_levels.iter().map(|lake| (lake.drawn, lake.level_m.to_bits())).collect() };
    for (name, same) in [
        ("drawing", a.drawing == b.drawing),
        ("asked", a.asked == b.asked),
        ("seed", a.seed == b.seed),
        ("config", a.config == b.config),
        ("model heights", bits(&a.model_heights.data) == bits(&b.model_heights.data)),
        ("first lake levels", lakes(a) == lakes(b)),
        ("marks", a.marks == b.marks),
        ("reach", a.reach_m.to_bits() == b.reach_m.to_bits()),
    ] {
        if !same {
            parts.push(name.to_string());
        }
    }
    parts
}

/// The lake of `drawing` that is `lake` (the first at the same place with the same radius not
/// taken by an earlier lake of `before`), as an edit pairs a kept lake.
fn lake_partner(before: &[Lake], drawing: &Drawing, index: usize) -> Option<usize> {
    let mut taken = vec![false; drawing.lakes.len()];
    let mut partner = None;
    for (drawn, lake) in before.iter().enumerate().take(index + 1) {
        let found = (0..drawing.lakes.len()).find(|&n| !taken[n] && drawing.lakes[n] == *lake);
        if let Some(n) = found {
            taken[n] = true;
        }
        if drawn == index {
            partner = found;
        }
    }
    partner
}

/// The most points of an edited land's courses `assert_kept` allows at weight 0 that are not the kept
/// land's, in one edit: the one class W1 read both ways still finds (the review of Task 6, N1). Where
/// the join gives a course of the band its kept line (`join_water`), a point of the band next to the
/// outline may be moved onto a cell of weight 0 just outside it, beside its course's next point in
/// the band; it is never on held or hand-shaped ground. Measured on every edit of these tests and
/// on the 17 trials at 1,024 m in hills from seeds 1 and 3, each with its standard edit: one point,
/// on `repeated` from seed 3, 0.30 m from the kept line, its next point at weight 0.0013.
const NEW_POINTS_AT_WEIGHT_0_MOST: usize = 1;

/// W1 and held ground: every world cell `edit` weighs 0, every held or hand-shaped cell among them
/// (each of which it weighs 0), keeps `before`'s height, ground, every layer and its water bit for
/// bit; every course point of `before` at weight 0 is one of the edited land's, bit for bit, and
/// every course point of the edited land at weight 0 is one of `before`'s, bit for bit, but at most
/// `NEW_POINTS_AT_WEIGHT_0_MOST` of its class (each on unmarked ground, beside a point of its course
/// at weight above 0); every lake of `before` whose middle lies at weight 0 is one of its lakes,
/// bit for bit, naming its place in the edited drawing, and every lake of the edited land whose
/// middle lies at weight 0 one of `before`'s; and the held marks are `before`'s, and every cell of
/// weight 0 keeps its hand mark. Returns the cells and course points checked, and how many points of
/// that class it found.
fn assert_kept(name: &str, before: &Land, edit: &Edit) -> (usize, usize, usize) {
    let (weight, after) = (&edit.outline.weight, &edit.land.world);
    let old = &before.world;
    assert_eq!((weight.width, weight.height), (old.heights.width, old.heights.height), "{name}: the weight's size");
    let mut cells = 0;
    for (cell, &w) in weight.data.iter().enumerate() {
        if edit.land.marks.fixed(cell) {
            assert_eq!(w, 0.0, "{name}: fixed cell {cell} weighs {w}");
        }
        if w != 0.0 {
            continue;
        }
        cells += 1;
        assert_eq!(after.heights.data[cell].to_bits(), old.heights.data[cell].to_bits(), "{name}: the height of cell {cell}, of weight 0");
        assert_eq!(after.ground.data[cell], old.ground.data[cell], "{name}: the ground of cell {cell}, of weight 0");
        for (layer, grid) in &old.layers {
            assert_eq!(after.layers[layer].data[cell].to_bits(), grid.data[cell].to_bits(), "{name}: the {layer} of cell {cell}, of weight 0");
        }
    }
    let at_0 = |point: &CoursePoint| weight_at(weight, f64::from(point.x_m), f64::from(point.z_m)) == 0.0;
    let points: BTreeSet<[u32; 5]> = after.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
    let outside: Vec<&CoursePoint> = old.water.rivers.iter().flat_map(|course| &course.points).filter(|point| at_0(point)).collect();
    let missing = outside.iter().filter(|point| !points.contains(&point_bits(point))).count();
    assert_eq!(missing, 0, "{name}: {missing} of the {} course points outside the outline are missing", outside.len());
    let kept: BTreeSet<[u32; 5]> = old.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
    let mut moved = 0;
    for course in &after.water.rivers {
        for (k, point) in course.points.iter().enumerate().filter(|&(_, point)| at_0(point) && !kept.contains(&point_bits(point))) {
            moved += 1;
            let (x, z) = (f64::from(point.x_m), f64::from(point.z_m));
            let index = |at: f64, count: usize| ((at + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
            let cell = index(z, weight.height) * weight.width + index(x, weight.width);
            let beside_the_band = [k.wrapping_sub(1), k + 1].into_iter().filter_map(|n| course.points.get(n)).any(|next| !at_0(next));
            assert!(!edit.land.marks.fixed(cell) && beside_the_band, "{name}: a point of weight 0 not the kept land's, at ({x}, {z}), on fixed ground or away from the band");
        }
    }
    assert!(moved <= NEW_POINTS_AT_WEIGHT_0_MOST, "{name}: {moved} course points at weight 0 are not the kept land's");
    for lake in old.water.lakes.iter().filter(|lake| weight_at(weight, f64::from(lake.x_m), f64::from(lake.z_m)) == 0.0) {
        let drawn = lake_partner(&before.drawing.lakes, &edit.land.drawing, lake.drawn).expect("a lake outside the outline is in the edited drawing");
        let kept = after.water.lakes.iter().find(|other| other.drawn == drawn).unwrap_or_else(|| panic!("{name}: lake {lake:?} outside the outline is gone"));
        assert_eq!([kept.level_m, kept.x_m, kept.z_m, kept.area_m2].map(f32::to_bits), [lake.level_m, lake.x_m, lake.z_m, lake.area_m2].map(f32::to_bits), "{name}: lake {lake:?}");
    }
    // And the other way: every lake of the edited land whose middle lies at weight 0 is a kept one.
    let lake_bits = |lake: &LakeWater| [lake.level_m, lake.x_m, lake.z_m, lake.area_m2].map(f32::to_bits);
    for lake in after.water.lakes.iter().filter(|lake| weight_at(weight, f64::from(lake.x_m), f64::from(lake.z_m)) == 0.0) {
        let kept = old.water.lakes.iter().any(|other| lake_partner(&before.drawing.lakes, &edit.land.drawing, other.drawn) == Some(lake.drawn) && lake_bits(other) == lake_bits(lake));
        assert!(kept, "{name}: lake {lake:?}, its middle at weight 0, is not the kept land's");
    }
    // The marks: no edit holds or lifts, and every cell of weight 0 keeps its hand mark (an area
    // regrown clears it only on its cells that are not held, which weigh above 0).
    let mark = |marks: &[bool], cell: usize| marks.get(cell) == Some(&true);
    for cell in 0..weight.data.len() {
        assert_eq!(mark(&edit.land.marks.held, cell), mark(&before.marks.held, cell), "{name}: the held mark of cell {cell}");
        if weight.data[cell] == 0.0 {
            assert_eq!(mark(&edit.land.marks.by_hand, cell), mark(&before.marks.by_hand, cell), "{name}: the hand mark of cell {cell}, of weight 0");
        }
    }
    assert_eq!(edit.notes.len(), edit.notes_about.len(), "{name}: a subject for each of the edit's notes");
    assert_eq!(after.notes.len(), after.notes_about.len(), "{name}: a subject for each of the land's notes");
    (cells, outside.len(), moved)
}

/// The edited drawing grown whole, in the model's frame, to judge an edit's water by.
fn grown_whole(drawing: &Drawing, seed: u64) -> World {
    let config = GenConfig::default();
    let model = grow(drawing, &config.water, seed, &never()).expect("the drawing grows");
    finish_drawn(seed, drawing, &config, &model, Frame::Model, &never()).expect("its world")
}

/// The sentence of a refused edit.
fn refused(result: Result<Edit, LandformError>) -> String {
    match result {
        Err(LandformError::Refused(sentence)) => sentence,
        Err(other) => panic!("refused, not {other}"),
        Ok(_) => panic!("refused, not taken"),
    }
}

/// The most a course's surface rises from one point to the next going down it.
fn rise_of(points: &[CoursePoint]) -> f32 {
    points.windows(2).map(|pair| pair[1].surface_m - pair[0].surface_m).fold(0.0, f32::max)
}

/// The size of the example the tests of the drawing asked for (stage 2's Task 1) edit: small, so
/// each edit is quick.
const SMALL_M: u32 = 512;
/// The brook's point `point_added` adds, by its place in the brook's points.
const ADDED_POINT: usize = 3;
/// How far east a test moves the added point, as edit `a` moves the brook's second point.
const MOVED_EAST_M: f64 = 30.0;
/// The middle of a disc of the example at 512 m far from its water, beyond `BAND_M` of every river
/// and the lake, which the tests of the drawing asked for regrow and hold.
const AWAY_FROM_THE_SMALL_WATER: (f64, f64) = (150.0, -180.0);
/// That disc's radius.
const AWAY_RADIUS_M: f64 = 20.0;
/// The size of the land the test of a signed 0 grows, the smallest the example is drawn at here.
const SIGNED_ZERO_M: u32 = 256;

/// The example at 512 m in hill country from seed 11, grown once for every test that reads it.
fn small_example() -> &'static Land {
    static LAND: OnceLock<Land> = OnceLock::new();
    LAND.get_or_init(|| Land::grow(&Drawing::example_valley(SMALL_M), SEED, &GenConfig::default(), &never()).expect("the example grows"))
}

/// `drawing` with a point added on the brook's line where it already runs: its third stretch split
/// at its middle, a change the land model's start does not show.
fn point_added(drawing: &Drawing) -> Drawing {
    let mut added = drawing.clone();
    let (a, b) = (added.rivers[1].points[ADDED_POINT - 1], added.rivers[1].points[ADDED_POINT]);
    added.rivers[1].points.insert(ADDED_POINT, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0));
    added
}

/// Stage 2's Task 1: a land grown asks for the drawing it was grown from, so nothing waits; an edit
/// asks for the drawing it was given, whose ground it grew, so nothing waits after it either.
#[test]
fn a_land_grown_or_edited_asks_for_its_own_drawing() {
    for land in [small_example(), example()] {
        assert!(land.asked == land.drawing && !land.waiting(), "the example at {} m", land.drawing.size_m);
    }
    let land = small_example();
    let edited = edit_of(land, "a");
    let edit = land.edit(&edited, &never()).expect("edit a");
    assert!(edit.outline.inside_m2 > 0.0, "edit a changes ground");
    assert!(edit.land.drawing == edited && edit.land.asked == edited && !edit.land.waiting());
}

/// Stage 2's Task 1 (Review Focus 4): a point added on the brook's line where it already runs gives
/// the land back bit for bit, its own drawing with it and the edit's note as before, but asking for
/// the drawing with the point, which waits; the land written as a file and read still waits with
/// the point; and moving the new point is an edit, of the land read as of the land that never
/// waited, the same land bit for bit but its timings, after which nothing waits.
#[test]
fn waiting_drawing_is_kept_and_moves_as_an_edit() {
    let land = small_example();
    let added = point_added(&land.drawing);
    let edit = land.edit(&added, &never()).expect("a point added");
    assert_eq!(differences(&edit.land, land), ["asked"]);
    assert!(edit.land.drawing == land.drawing && edit.land.asked == added && edit.land.waiting());
    assert_eq!((edit.timings.grow_ms, edit.outline.inside_m2), (0.0, 0.0), "nothing grown");
    assert_eq!(edit.notes, ["This edit changes no ground; the land keeps its drawing as it was."]);
    assert_eq!(edit.notes_about, [About::Land]);

    // Saved and opened again, it still waits with the point.
    let dir = std::env::temp_dir().join(format!("landform_waiting_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("waiting.lfland");
    edit.land.write(&path).expect("the waiting land is written");
    let read = Land::read(&path).expect("the waiting land is read");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(differences(&read, &edit.land), Vec::<String>::new());
    assert!(read.waiting() && read.asked == added);

    // The point moved, as the tool would move it next: an edit.
    let mut moved = read.asked.clone();
    moved.rivers[1].points[ADDED_POINT].0 += MOVED_EAST_M;
    let from_waiting = read.edit(&moved, &never()).expect("the new point moved");
    let from_plain = land.edit(&moved, &never()).expect("the same drawing from the land that never waited");
    assert!(from_waiting.outline.inside_m2 > 0.0 && from_waiting.timings.grow_ms > 0.0, "the moved point regrows");
    assert_eq!(differences(&from_waiting.land, &from_plain.land), Vec::<String>::new());
    assert_eq!(bits(&from_waiting.outline.weight.data), bits(&from_plain.outline.weight.data));
    assert_eq!((&from_waiting.notes, &from_waiting.notes_about), (&from_plain.notes, &from_plain.notes_about));
    assert!(from_waiting.land.drawing == moved && from_waiting.land.asked == moved && !from_waiting.land.waiting());
    assert_kept("the added point moved", &read, &from_waiting);
    println!("the added point moved {MOVED_EAST_M} m: {:.2} ha inside; {:?}", from_waiting.outline.inside_m2 / 10_000.0, from_waiting.timings);
}

/// Stage 2's Task 1: a regrowth of the land's own drawing, from another seed or on an area, and a
/// hold leave the drawing asked for as it was, so what waits still waits; the same seed again gives
/// the waiting land back bit for bit.
#[test]
fn reseed_regrow_area_and_hold_leave_what_waits_waiting() {
    let land = small_example();
    let waiting = land.edit(&point_added(&land.drawing), &never()).expect("a point added").land;
    assert!(waiting.waiting());
    let reseeded = waiting.reseed(OTHER_SEED, &never()).expect("regrown from another seed");
    assert!(reseeded.outline.inside_m2 == f64::from(SMALL_M * SMALL_M) && reseeded.land.seed == OTHER_SEED, "the whole land from seed 23");
    assert!(reseeded.land.drawing == waiting.drawing && reseeded.land.asked == waiting.asked && reseeded.land.waiting(), "reseed");
    let again = waiting.reseed(SEED, &never()).expect("the same seed");
    assert_eq!(differences(&again.land, &waiting), Vec::<String>::new(), "the same seed");

    let area = disc(SMALL_M as usize, AWAY_FROM_THE_SMALL_WATER, AWAY_RADIUS_M);
    let regrown = waiting.regrow_area(&area, &never()).expect("an area regrown");
    assert!(regrown.outline.inside_m2 > 0.0, "the area is regrown");
    assert!(regrown.land.drawing == waiting.drawing && regrown.land.asked == waiting.asked && regrown.land.waiting(), "regrow_area");

    let mut held = waiting.clone();
    assert!(held.hold(&area, true) > 0, "the area is held");
    assert_eq!(differences(&held, &waiting), ["marks"], "a hold changes the marks alone");
    assert!(held.waiting());
}

/// Stage 2's Task 1: `edit_from_seed` from another seed regrows the whole land toward the drawing
/// given, which it asks for; from the land's own seed it is `edit` exactly, bit for bit but its
/// timings; and a land asking for a drawing (`ask`) regrown from another seed toward it is that
/// drawing's edit from the seed, in one growing.
#[test]
fn an_edit_from_another_seed_regrows_the_whole_land_toward_the_drawing_given() {
    let land = small_example();
    let edited = edit_of(land, "a");
    let edit = land.edit_from_seed(&edited, OTHER_SEED, &never()).expect("edit a from seed 23");
    assert!(edit.outline.inside_m2 == f64::from(SMALL_M * SMALL_M), "the whole land is inside the outline");
    assert_eq!(edit.notes[0], "This edit regrows the whole land but held and hand-shaped ground: the seed changed.");
    assert_eq!((edit.land.seed, edit.land.world.meta.seed), (OTHER_SEED, OTHER_SEED));
    assert!(edit.land.drawing == edited && edit.land.asked == edited && !edit.land.waiting());

    let own = land.edit_from_seed(&edited, SEED, &never()).expect("edit a from the land's own seed");
    let plain = land.edit(&edited, &never()).expect("edit a");
    assert!(plain.outline.inside_m2 > 0.0 && plain.outline.inside_m2 < f64::from(SMALL_M * SMALL_M));
    assert_eq!(differences(&own.land, &plain.land), Vec::<String>::new(), "the land's own seed");
    assert_eq!(bits(&own.outline.weight.data), bits(&plain.outline.weight.data));
    assert_eq!(bits(&own.outline.model_weight), bits(&plain.outline.model_weight));
    assert_eq!(own.outline.inside_m2.to_bits(), plain.outline.inside_m2.to_bits());
    assert_eq!((&own.troughs, own.pits_left, &own.water), (&plain.troughs, plain.pits_left, &plain.water));
    assert_eq!((&own.notes, &own.notes_about), (&plain.notes, &plain.notes_about));

    let mut asking = land.clone();
    asking.ask(edited.clone()).expect("edit a asked for");
    let regrown = asking.edit_from_seed(&asking.asked, OTHER_SEED, &never()).expect("what waits regrown from seed 23");
    assert_eq!(differences(&regrown.land, &edit.land), Vec::<String>::new(), "what waits, from seed 23");
    assert!(!regrown.land.waiting());
}

/// Stage 2's Task 1: `ask` refuses what an edit refuses, in an edit's words (a drawing of another
/// size, one with a river of one point), the land bit for bit afterwards; it takes another drawing
/// as the one asked for, which waits, and the land's own again, after which nothing waits. Whether a
/// land waits is said as an edit's empty footprint tells another drawing, bit for bit: a picture
/// whose 0 is written -0 is a drawing equal by value, and another drawing to both.
#[test]
fn asking_refuses_what_an_edit_refuses_and_waits_as_an_empty_footprint_tells() {
    let land = small_example();
    let mut asking = land.clone();
    let mut one_point = land.drawing.clone();
    one_point.rivers[1].points.truncate(1);
    for (name, drawing, words) in [("another size", Drawing::example_valley(1024), ["1024 m", "512 m"]), ("a river of one point", one_point, ["river 2", "two points"])] {
        let refused = asking.ask(drawing.clone()).expect_err(name);
        assert!(matches!(&refused, LandformError::BadConfig(reason) if words.iter().all(|word| reason.contains(word))), "{name}: {refused:?}");
        assert_eq!(Some(refused.clone()), land.check_ask(&drawing).err(), "{name}: checked before asking, in ask's words");
        assert_eq!(Some(refused), land.edit(&drawing, &never()).err(), "{name}: in an edit's words");
        assert_eq!(differences(&asking, land), Vec::<String>::new(), "{name}: the land as it was");
    }
    let edited = edit_of(land, "a");
    assert_eq!(land.check_ask(&edited), Ok(()), "edit a is a drawing the land may ask for");
    asking.ask(edited.clone()).expect("edit a asked for");
    assert!(asking.waiting() && asking.asked == edited);
    assert_eq!(differences(&asking, land), ["asked"]);
    asking.ask(land.drawing.clone()).expect("the land's own drawing asked for");
    assert!(!asking.waiting());
    assert_eq!(differences(&asking, land), Vec::<String>::new());

    let mut drawing = Drawing::example_valley(SIGNED_ZERO_M);
    drawing.high_ground.as_mut().expect("the example has a picture").data[0] = 0.0;
    let zero = Land::grow(&drawing, SEED, &GenConfig::default(), &never()).expect("the example grows");
    let mut signed = drawing.clone();
    signed.high_ground.as_mut().expect("a picture").data[0] = -0.0;
    assert!(signed == drawing, "equal by value");
    let edit = zero.edit(&signed, &never()).expect("a picture's 0 written -0");
    assert_eq!(edit.outline.inside_m2, 0.0, "an empty footprint");
    assert_eq!(edit.notes, ["This edit changes no ground; the land keeps its drawing as it was."], "another drawing to the empty footprint");
    assert!(edit.land.waiting(), "and to `waiting`");
    let mut asking = zero.clone();
    asking.ask(signed).expect("the picture's -0 asked for");
    assert!(asking.waiting());
    let same = zero.edit(&drawing, &never()).expect("the land's own drawing");
    assert!(same.notes.is_empty() && !same.land.waiting());
}

#[test]
fn a_land_grown_is_the_drawn_world_in_the_models_frame() {
    let land = example();
    let config = GenConfig::default();
    let drawn = generate_drawn(SEED, &land.drawing, &config, &never()).expect("the example grows");
    let base = drawn.meta.base_elevation_m;
    assert!(base != 0.0 && land.world.meta.base_elevation_m == 0.0, "the land in the model's frame, the drawn world lowered by {base} m");
    let shifted = |value: f32| (value - base).to_bits();
    assert_eq!(land.world.heights.data.iter().map(|&h| shifted(h)).collect::<Vec<_>>(), bits(&drawn.heights.data), "the heights less the base");
    let courses = |world: &World, lower: &dyn Fn(f32) -> u32| -> Vec<_> {
        (world.water.rivers.iter())
            .map(|course| (course.drawn, course.from_lake, course.end, course.points.iter().map(|p| [p.x_m.to_bits(), p.z_m.to_bits(), lower(p.surface_m), p.width_m.to_bits(), p.depth_m.to_bits()]).collect::<Vec<_>>()))
            .collect()
    };
    assert_eq!(courses(&land.world, &shifted), courses(&drawn, &f32::to_bits), "the courses, their surfaces less the base");
    let lakes = |world: &World, lower: &dyn Fn(f32) -> u32| -> Vec<_> { world.water.lakes.iter().map(|l| (l.drawn, lower(l.level_m), [l.x_m, l.z_m, l.area_m2].map(f32::to_bits))).collect() };
    assert_eq!(lakes(&land.world, &shifted), lakes(&drawn, &f32::to_bits), "the lakes, their levels less the base");
    assert!(land.world.ground == drawn.ground, "the ground map");
    for (name, layer) in &drawn.layers {
        assert_eq!(bits(&land.world.layers[name].data), bits(&layer.data), "the {name} layer");
    }
    assert!(land.world.layers.contains_key(WATER_LAYER) && land.world.layers.len() == drawn.layers.len());
    assert_eq!((&land.world.notes, &land.world.notes_about), (&drawn.notes, &drawn.notes_about));
    let model = grow(&land.drawing, &config.water, SEED, &never()).expect("the example grows");
    assert_eq!(bits(&land.model_heights.data), bits(&model.heights.data), "grow's heights");
    assert_eq!(land.first_lake_levels, model.lakes, "grow's lakes");
    assert!(!land.world.water.lakes.is_empty() && !land.world.water.rivers.is_empty());
    assert_eq!((land.seed, land.config, land.reach_m), (SEED, config, REACH_MIN_M));
    assert_eq!(land.marks, Marks::none(SIZE_M as usize));
}

/// Review Focus 2: the same drawing gives an empty footprint, and the land back bit for bit with no
/// growing done; the drawing being the land's own, bit for bit, the edit says nothing.
#[test]
fn the_same_drawing_again_gives_the_land_back_without_growing() {
    let land = example();
    let edit = land.edit(&land.drawing, &never()).expect("an edit that changes nothing");
    assert_eq!(differences(&edit.land, land), Vec::<String>::new());
    assert!(edit.land.world.meta.timings == land.world.meta.timings, "the timings too");
    assert_eq!((edit.timings.grow_ms, edit.timings.fine_steps_ms, edit.timings.mix_ms), (0.0, 0.0, 0.0));
    assert!(edit.outline.weight.data.iter().all(|&w| w == 0.0) && edit.outline.inside_m2 == 0.0 && edit.outline.model_weight.iter().all(|&w| w == 0.0));
    assert!(edit.troughs.is_empty() && edit.pits_left == 0 && edit.notes.is_empty() && edit.notes_about.is_empty());
    assert_eq!(edit.water, Default::default());
    println!("the same drawing again: {:?}", edit.timings);
}

/// The controller's ruling R18 (its review's S5): a change the land model's start does not show,
/// a point added on the brook's line where it already runs (its third stretch split at its middle)
/// or the picture of high ground raised 0.001 everywhere (less than the footprint's tolerance
/// shows), gives an empty footprint: the land back bit for bit, its own drawing with it, with no
/// growing, and one note about the land saying the land keeps its drawing. Since stage 2's Task 1
/// the drawing given is kept as the one asked for, and waits (`waiting_drawing_is_kept_and_moves_as_an_edit`).
#[test]
fn an_edit_that_changes_no_ground_keeps_the_lands_drawing_and_says_so() {
    const RAISED_BY: f32 = 0.001;
    let land = example();
    let added = point_added(&land.drawing);
    let mut raised = land.drawing.clone();
    let picture = raised.high_ground.as_mut().expect("the example has a picture");
    picture.data.iter_mut().for_each(|value| *value += RAISED_BY);
    for (name, drawing) in [("a point added on the brook's line", added), ("the picture raised", raised)] {
        assert!(drawing != land.drawing, "{name}: another drawing");
        let edit = land.edit(&drawing, &never()).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(differences(&edit.land, land), ["asked"], "{name}: the land, its drawing among it, but the drawing asked for");
        assert!(edit.land.asked == drawing && edit.land.waiting(), "{name}: the drawing given waits");
        assert!(edit.land.drawing == land.drawing && edit.land.world.meta.timings == land.world.meta.timings, "{name}: its own drawing");
        assert_eq!((edit.timings.grow_ms, edit.outline.inside_m2), (0.0, 0.0), "{name}: nothing grown");
        assert_eq!(edit.notes, ["This edit changes no ground; the land keeps its drawing as it was."], "{name}");
        assert_eq!(edit.notes_about, [About::Land], "{name}");
        println!("{name}: {:?}; {:?}", edit.notes, edit.timings);
    }
}

/// Each of the example's nine edits keeps every cell of weight 0, its course points and its lakes,
/// bit for bit, and adds no course point at weight 0 (`NEW_POINTS_AT_WEIGHT_0_MOST`'s class is not
/// met on the example); the timings are printed (bounded in `edit_time.rs`, a test binary of its own).
#[test]
fn each_of_the_examples_nine_edits_keeps_the_land_outside_its_outline() {
    let land = example();
    for name in EDIT_NAMES {
        let edit = land.edit(&edit_of(land, name), &never()).unwrap_or_else(|error| panic!("edit {name}: {error}"));
        let (cells, points, moved) = assert_kept(name, land, &edit);
        assert_eq!(moved, 0, "edit {name}: course points at weight 0 not the kept land's");
        assert!(edit.outline.inside_m2 > 0.0, "edit {name} changes something");
        println!(
            "edit {name}: {:.2} ha inside, {cells} cells and {points} course points kept, {} troughs, crossings {} (+{} inside), notes {:?}; {:?}",
            edit.outline.inside_m2 / 10_000.0,
            edit.troughs.len(),
            edit.water.crossings,
            edit.water.joined_inside,
            edit.notes,
            edit.timings
        );
    }
}

/// The prototype's held case (i): a held disc whose edge lies 100 m from the brook's moved stretch.
/// Taken, it would leave 57 dry cells beside the disc that the kept land does not have (the
/// prototype's count), where the band takes the moved valley in part and a tributary stops short of
/// the brook: refused (the controller's ruling R17), naming what the edit changed nearest the first
/// of them, river 2 (the brook; ruling R18), the held cell nearest that cell, and the distance from
/// river 2's new line (its smoothed line in the edited drawing's model) to the held cell; the land
/// is untouched.
#[test]
fn held_ground_beside_a_moved_river_refuses_the_edit() {
    let land = holding(example(), &[(BESIDE_THE_BROOK, DISC_M)]);
    let before = land.clone();
    let edited = edit_of(&land, "a");
    let sentence = refused(land.edit(&edited, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("River 2's new line runs within 150 m of held ground at ("), "{sentence}");
    let (x, z) = parse_place(&sentence);
    let size = SIZE_M as usize;
    let cell = ((z + SIZE_M as i64 / 2) as usize) * size + (x + SIZE_M as i64 / 2) as usize;
    assert!(land.marks.held[cell], "the place named, ({x}, {z}), is held");
    let model = grow(&edited, &GenConfig::default().water, SEED, &never()).expect("the edited drawing grows");
    let line = &model.rivers.iter().find(|river| river.drawn == 1).expect("the brook is laid").line;
    let apart_m = line.windows(2).map(|pair| to_segment((x as f64, z as f64), pair[0], pair[1])).fold(f64::INFINITY, f64::min);
    assert!(sentence.contains(&format!(" at ({x}, {z}), {} m from it; ", apart_m.round())), "{sentence}: the new line {apart_m:.1} m from the held cell");
    assert!(sentence.ends_with(" m from it; the water there would not be held: move the line, or lift the hold there."), "{sentence}");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// The distance from `p` to the segment from `a` to `b`, in metres.
fn to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let squared = dx * dx + dz * dz;
    let t = if squared > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
    (p.0 - a.0 - dx * t).hypot(p.1 - a.1 - dz * t)
}

/// The first place `(x, z)` a sentence names.
fn parse_place(sentence: &str) -> (i64, i64) {
    let (_, rest) = sentence.split_once(" at (").expect("a place");
    let (inside, _) = rest.split_once(')').expect("a place");
    let (x, z) = inside.split_once(", ").expect("a place");
    (x.parse().expect("x"), z.parse().expect("z"))
}

/// The prototype's held case (ii), the plan's Step 1: a held disc of 60 m on the brook's valley 250 m
/// below its moved point. The moved stretch runs on through the disc's band into the disc, and the
/// join there leaves no water unheld (the controller's ruling R17): taken, every held cell bit for
/// bit, the brook's points through the disc the kept land's, its water not rising where it enters or
/// leaves the disc nor anywhere down it, no new unheld water, and the tight join said.
#[test]
fn held_ground_on_a_moved_rivers_valley_stays_and_the_tight_join_is_said() {
    let land = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let edited = edit_of(&land, "a");
    let edit = land.edit(&edited, &never()).expect("the brook's point moved past held ground on its valley");
    assert_kept("a, the brook's valley held", &land, &edit);
    let held: Vec<usize> = (0..land.marks.held.len()).filter(|&cell| land.marks.held[cell]).collect();
    assert_eq!(held.len(), HELD_DISC_CELLS, "the disc's cells");
    assert!(held.iter().all(|&cell| edit.outline.weight.data[cell] == 0.0));
    let through = through_the_disc(&land, &edit, 1, ON_THE_BROOK);
    assert_eq!(rise_of(&through), 0.0, "the brook's water never rises");
    assert_eq!(edit.water.rise_most_m, 0.0);
    let whole = grown_whole(&edited, SEED);
    let found = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
    assert_eq!((found.outside(), found.band(), found.inside()), (0, 0, 0), "new unheld water: {found:?}");
    assert_tight_join(&edit, ON_THE_BROOK);
    println!("a, the brook's valley held: {} held cells kept, the brook's course through the disc {} points, notes {:?}; {:?}", held.len(), through.len(), edit.notes, edit.timings);
}

/// The cells of a disc of `DISC_M` held by `holding`: those within 60 m of its centre.
const HELD_DISC_CELLS: usize = 11_289;

/// The points of the one course of river `drawn` of the edited land that runs through the disc of
/// `DISC_M` at `at` (more than 10 of them there, each the kept land's bit for bit), all of its
/// points.
fn through_the_disc(land: &Land, edit: &Edit, drawn: usize, at: (f64, f64)) -> Vec<CoursePoint> {
    let in_disc = |point: &CoursePoint| (f64::from(point.x_m) - at.0).hypot(f64::from(point.z_m) - at.1) < DISC_M - 1.0;
    let through: Vec<_> = (edit.land.world.water.rivers.iter()).filter(|course| course.drawn == Some(drawn) && course.points.iter().any(in_disc)).collect();
    assert_eq!(through.len(), 1, "one course of river {} runs through the disc", drawn + 1);
    let kept: BTreeSet<[u32; 5]> = land.world.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
    let points = through[0].points.iter().filter(|point| in_disc(point)).count();
    assert!(points > 10, "river {} runs through the disc: {points} points", drawn + 1);
    assert!(through[0].points.iter().filter(|point| in_disc(point)).all(|point| kept.contains(&point_bits(point))), "its points in the disc are the kept land's");
    through[0].points.clone()
}

/// A held disc on a river the edit does not move, where it crosses the outline (the main river,
/// for the brook's point moved): its cells stay bit for bit, the river runs through it as it ran,
/// and its water does not rise where it enters or leaves the disc, nor anywhere down the river. The
/// footprint reaches within `BAND_M` of the disc's rim (worked out here from the footprint), so the
/// join beside it is tight and said.
#[test]
fn a_river_through_held_ground_stays_as_it_was() {
    let land = holding(example(), &[(ON_THE_MAIN_RIVER, DISC_M)]);
    let edited = edit_of(&land, "a");
    let edit = land.edit(&edited, &never()).expect("the edit is taken round the held disc");
    assert_kept("a, the main river held", &land, &edit);
    let held: Vec<usize> = (0..land.marks.held.len()).filter(|&cell| land.marks.held[cell]).collect();
    assert!(held.iter().all(|&cell| edit.outline.weight.data[cell] == 0.0));
    assert!(held.iter().any(|&cell| edit.outline.weight.data[cell] == 0.0) && edit.outline.weight.data.iter().any(|&w| w > 0.0));
    let in_disc = |point: &CoursePoint| (f64::from(point.x_m) - ON_THE_MAIN_RIVER.0).hypot(f64::from(point.z_m) - ON_THE_MAIN_RIVER.1) < DISC_M - 1.0;
    let through: Vec<_> = (edit.land.world.water.rivers.iter()).filter(|course| course.drawn == Some(0) && course.points.iter().any(in_disc)).collect();
    assert_eq!(through.len(), 1, "one course of the main river runs through the disc");
    let main = through[0];
    let through = main.points.iter().filter(|point| in_disc(point)).count();
    assert!(through > 10, "the main river runs through the disc: {through} points");
    let kept: BTreeSet<[u32; 5]> = land.world.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
    assert!(main.points.iter().filter(|point| in_disc(point)).all(|point| kept.contains(&point_bits(point))), "its points in the disc are the kept land's");
    assert!(main.points.iter().any(|point| weight_at(&edit.outline.weight, f64::from(point.x_m), f64::from(point.z_m)) > 0.0), "it crosses the outline");
    assert_eq!(rise_of(&main.points), 0.0, "the main river's water never rises");
    assert_eq!(edit.water.rise_most_m, 0.0);
    let whole = grown_whole(&edited, SEED);
    let found = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
    assert_eq!((found.outside(), found.band(), found.inside()), (0, 0, 0), "new unheld water: {found:?}");
    let water = GenConfig::default().water;
    let found = footprint(&start_of(&land.drawing, &water, SEED).expect("a start"), &start_of(&edited, &water, SEED).expect("a start"), &land.drawing, &edited);
    let changed = covered(SIZE_M, &found.model_cells, &found.lake_discs);
    let size = SIZE_M as usize;
    let from_disc_m = (0..changed.len())
        .filter(|&cell| changed[cell])
        .map(|cell| (place(size, cell).0 - ON_THE_MAIN_RIVER.0).hypot(place(size, cell).1 - ON_THE_MAIN_RIVER.1) - DISC_M)
        .fold(f64::INFINITY, f64::min);
    assert!(from_disc_m < f64::from(BAND_M), "the footprint lies {from_disc_m:.1} m from the disc");
    assert_tight_join(&edit, ON_THE_MAIN_RIVER);
    println!("the main river held: {} held cells kept, {through} points through the disc, the footprint {from_disc_m:.1} m from the disc, notes {:?}", held.len(), edit.notes);
}

/// The design's amendment 10: a hill painted beside held ground is not refused, and the steep join
/// is still marked. Case (i)'s disc, with a hill painted 100 m beyond its edge in place of the
/// brook's point moved: taken, the held cells kept, and the tight join said at the disc's middle,
/// about the land (the controller's ruling R18).
#[test]
fn a_hill_painted_beside_held_ground_is_not_refused() {
    let land = holding(example(), &[(BESIDE_THE_BROOK, DISC_M)]);
    let edited = hill_edit(&land.drawing, HILL_BESIDE_THE_DISC).expect("a hill");
    let edit = land.edit(&edited, &never()).expect("a hill beside held ground is taken");
    assert_kept("a hill beside held ground", &land, &edit);
    assert_tight_join(&edit, BESIDE_THE_BROOK);
    println!("a hill beside held ground: {:.2} ha, notes {:?}", edit.outline.inside_m2 / 10_000.0, edit.notes);
}

/// The world cells a footprint covers (the model cells' squares, 9 x 9 points with edges shared,
/// and the lakes' discs), worked out here from the footprint as `outline` reads it.
fn covered(size_m: u32, model_cells: &[bool], discs: &[(f64, f64, f64)]) -> Vec<bool> {
    let size = size_m as usize;
    let side = (model_cells.len() as f64).sqrt() as usize;
    let (cell_m, margin_m) = (f64::from(GROW_CELL_M), f64::from(GROW_MARGIN_M));
    (0..size * size)
        .map(|cell| {
            let (x, z) = place(size, cell);
            let (a, b) = (x + size as f64 / 2.0 + margin_m, z + size as f64 / 2.0 + margin_m);
            let near = |at: f64| ((at - cell_m / 2.0) / cell_m).ceil().max(0.0) as usize..=(((at + cell_m / 2.0) / cell_m).floor() as usize).min(side - 1);
            near(b).any(|j| near(a).any(|i| model_cells[j * side + i])) || discs.iter().any(|&(dx, dz, r)| (x - dx).hypot(z - dz) <= r)
        })
        .collect()
}

/// The edit says the join tight beside held ground at `at`, the middle of a held disc, about the
/// land, once.
fn assert_tight_join(edit: &Edit, (x, z): (f64, f64)) {
    let tight = format!("The join beside held ground at ({x}, {z}) is tight: the new land is only partly taken within 150 m of it.");
    let said: Vec<usize> = (0..edit.notes.len()).filter(|&at| edit.notes[at] == tight).collect();
    assert_eq!(said.len(), 1, "{tight} once in {:?}", edit.notes);
    assert_eq!(edit.notes_about[said[0]], About::Land);
}

/// Review Focus 6: held cells inside the footprint itself (the very ground a painted hill covers):
/// the hill is taken round them, they stay bit for bit, and the edit says how much of its footprint
/// was held, the share worked out here from the footprint and the marks, and that the join round
/// them is tight.
#[test]
fn held_ground_inside_a_hills_footprint_stays_and_the_edit_says_how_much() {
    let land = holding(example(), &[(HILL_TOP, HILL_TOP_DISC_M)]);
    let edited = edit_of(&land, "d");
    let edit = land.edit(&edited, &never()).expect("a hill over held ground is taken round it");
    assert_kept("d over held ground", &land, &edit);
    let water = GenConfig::default().water;
    let found = footprint(&start_of(&land.drawing, &water, SEED).expect("a start"), &start_of(&edited, &water, SEED).expect("a start"), &land.drawing, &edited);
    let changed = covered(SIZE_M, &found.model_cells, &found.lake_discs);
    let count = changed.iter().filter(|&&on| on).count();
    let held = (0..changed.len()).filter(|&cell| changed[cell] && land.marks.held[cell]).count();
    assert!(held > 0 && held < count, "{held} of the footprint's {count} cells held");
    let said = format!("{:.1} % of the ground the edited drawing changes is held, and was kept as it was.", 100.0 * held as f64 / count as f64);
    assert!(edit.notes.contains(&said), "{said} in {:?}", edit.notes);
    let at = edit.notes.iter().position(|note| *note == said).expect("said");
    assert_eq!(edit.notes_about[at], About::Land);
    assert!((0..changed.len()).any(|cell| changed[cell] && !land.marks.held[cell] && edit.outline.weight.data[cell] > 0.0), "the hill is taken round the held cells");
    assert_tight_join(&edit, HILL_TOP);
    println!("d over held ground: {held} of {count} footprint cells held; notes {:?}", edit.notes);
}

/// The design's restated Done means: four edits in a row (the brook's point, the main river's
/// point, the hill, the brook removed), each kept outside its outline, leave no unheld water (the
/// example has none either), none new by the land before each and that edit's drawing grown whole,
/// and no pit, and the same drawing again leaves the land bit for bit.
#[test]
fn four_edits_in_a_row_leave_nothing_unheld_and_no_pit() {
    let mut land = example().clone();
    println!("the example: unheld {:?}", unheld(&land.world));
    for name in ["a", "c", "d", "g"] {
        let edited = edit_of(&land, name);
        let edit = land.edit(&edited, &never()).unwrap_or_else(|error| panic!("edit {name}: {error}"));
        assert_kept(name, &land, &edit);
        let whole = grown_whole(&edited, SEED);
        let found = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
        assert_eq!((found.outside(), found.band(), found.inside()), (0, 0, 0), "edit {name} in a row: new unheld water {found:?}");
        assert_eq!(edit.pits_left, 0, "edit {name} in a row: pits left");
        let left = unheld(&edit.land.world).expect("a land with water");
        let cells = [left.dry_lake, left.dry_course, left.lake_over_water, left.water_steps, left.mouth_steps];
        assert_eq!(cells, [0; 5], "edit {name} in a row: unheld water {left:?}");
        println!("edit {name} in a row: unheld {:?}, troughs {:?}, notes {:?}", unheld(&edit.land.world), edit.troughs, edit.notes);
        land = edit.land;
    }
    let again = land.edit(&land.drawing, &never()).expect("the same drawing again");
    assert_eq!(differences(&again.land, &land), Vec::<String>::new());
    assert_eq!(again.timings.grow_ms, 0.0);
}

/// Review Focus 2: another seed regrows everything but a held disc, which stays bit for bit, with a
/// band round it only (a tight join, said): every cell more than `BAND_M` from it weighs 1, and the
/// land stands on the new seed. The same seed again gives the land back. A disc held on the brook's valley (the
/// prototype's case (ii)) is taken too: the land regrown from seed 23 round it leaves no water unheld
/// beside it that the kept land does not have (the controller's ruling R17), and the tight join is
/// said.
#[test]
fn another_seed_regrows_everything_but_a_held_disc() {
    let land = holding(example(), &[(FAR_FROM_THE_WATER, DISC_M)]);
    let edit = land.reseed(OTHER_SEED, &never()).expect("the land regrown from another seed");
    assert_kept("seed 23", &land, &edit);
    assert_tight_join(&edit, FAR_FROM_THE_WATER);
    let size = SIZE_M as usize;
    for (cell, &w) in edit.outline.weight.data.iter().enumerate() {
        let (x, z) = place(size, cell);
        let from_disc_m = (x - FAR_FROM_THE_WATER.0).hypot(z - FAR_FROM_THE_WATER.1) - DISC_M;
        if from_disc_m > f64::from(BAND_M) + 1.0 {
            assert_eq!(w, 1.0, "cell {cell}, {from_disc_m:.0} m from the held disc");
        }
    }
    assert_eq!((edit.land.seed, edit.land.world.meta.seed), (OTHER_SEED, OTHER_SEED));
    assert!(edit.outline.inside_m2 == f64::from(SIZE_M * SIZE_M), "the whole land is inside the outline");
    let again = land.reseed(SEED, &never()).expect("the same seed");
    assert_eq!(differences(&again.land, &land), Vec::<String>::new());
    println!("seed 23: {} troughs, {} pits left, notes {:?}; {:?}", edit.troughs.len(), edit.pits_left, edit.notes, edit.timings);
    let on_the_brook = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let edit = on_the_brook.reseed(OTHER_SEED, &never()).expect("the land regrown from another seed round a disc held on the brook");
    assert_kept("seed 23 round the brook's disc", &on_the_brook, &edit);
    assert_tight_join(&edit, ON_THE_BROOK);
    println!("seed 23 round the brook's disc: notes {:?}", edit.notes);
}

/// A change of seed beside a large hold (every cell west of x = -600 m held) that would leave water
/// unheld beside it is refused, naming the regrown ground, since a change of seed changes no drawn
/// river or lake, over the held cell nearest the first such water; the land is untouched.
#[test]
fn a_change_of_seed_beside_held_ground_names_the_regrown_ground() {
    const WEST_OF_M: f64 = -600.0;
    let size = SIZE_M as usize;
    let mut land = example().clone();
    land.hold(&(0..size * size).map(|cell| place(size, cell).0 < WEST_OF_M).collect::<Vec<_>>(), true);
    let before = land.clone();
    let sentence = refused(land.reseed(OTHER_SEED, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("The regrown ground runs over held ground at ("), "{sentence}");
    assert!(sentence.ends_with("; the water there would not be held: keep the edit farther from it, or lift the hold there."), "{sentence}");
    let (x, z) = parse_place(&sentence);
    assert!(land.marks.held[((z + SIZE_M as i64 / 2) as usize) * size + (x + SIZE_M as i64 / 2) as usize], "the place named, ({x}, {z}), is held");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// The width the brook is drawn at in place of its 4 m.
const WIDER_BROOK_M: f32 = 8.0;

/// `land`'s drawing with the brook drawn `WIDER_BROOK_M` wide.
fn wider_brook(land: &Land) -> Drawing {
    let mut wider = land.drawing.clone();
    wider.rivers[1].width_m = WIDER_BROOK_M;
    wider
}

/// The mean width of the points of river `drawn`'s courses in `world` at whose weight `keep` holds,
/// and how many there are.
fn mean_width(world: &World, drawn: usize, weight: &Grid<f32>, keep: impl Fn(f32) -> bool) -> (f64, usize) {
    let widths: Vec<f64> = (world.water.rivers.iter().filter(|course| course.drawn == Some(drawn)))
        .flat_map(|course| &course.points)
        .filter(|point| keep(weight_at(weight, f64::from(point.x_m), f64::from(point.z_m))))
        .map(|point| f64::from(point.width_m))
        .collect();
    (widths.iter().sum::<f64>() / widths.len().max(1) as f64, widths.len())
}

/// The controller's ruling: a river whose width changes is an edit. The brook drawn 8 m wide in
/// place of 4 m: its land model's start is the same, so the width puts the cells along its line into
/// the footprint (`WIDTH_CHANGE_REACH_M`), and the edited brook takes the new width inside the
/// outline: its points of weight 1 are as wide as the drawing grown whole's there, on the mean,
/// within a hundredth, and the widest of them wider than a brook drawn 4 m wide can be (its width
/// times one and `width_wander`); nothing new is unheld.
#[test]
fn a_river_whose_width_changes_is_an_edit() {
    const SAME_WITHIN: f64 = 0.01;
    let land = example();
    let wider = wider_brook(land);
    let water = GenConfig::default().water;
    let (old_start, new_start) = (start_of(&land.drawing, &water, SEED).expect("a start"), start_of(&wider, &water, SEED).expect("a start"));
    assert_eq!(old_start, new_start, "the start does not show the width");
    let found = footprint(&old_start, &new_start, &land.drawing, &wider);
    let cells = found.model_cells.iter().filter(|&&on| on).count();
    assert!(cells > 100 && found.lake_discs.is_empty(), "the width puts the brook into the footprint: {cells} cells");
    let edit = land.edit(&wider, &never()).expect("the brook widened");
    assert_kept("the brook widened", land, &edit);
    let weight = &edit.outline.weight;
    let (inside, count) = mean_width(&edit.land.world, 1, weight, |w| w == 1.0);
    let (before, _) = mean_width(&land.world, 1, weight, |w| w == 1.0);
    let whole = grown_whole(&wider, SEED);
    let (grown, _) = mean_width(&whole, 1, weight, |w| w == 1.0);
    assert!(count > 100, "the brook's points inside the outline: {count}");
    assert!((inside - grown).abs() <= SAME_WITHIN * grown, "the brook's mean width inside {inside:.3} m, grown whole {grown:.3} m");
    let widest = (edit.land.world.water.rivers.iter().filter(|course| course.drawn == Some(1)))
        .flat_map(|course| &course.points)
        .filter(|point| weight_at(weight, f64::from(point.x_m), f64::from(point.z_m)) == 1.0)
        .map(|point| point.width_m)
        .fold(0.0, f32::max);
    let drawn_before = land.drawing.rivers[1].width_m * (1.0 + water.width_wander);
    assert!(widest > drawn_before, "the widest point inside, {widest} m, against {drawn_before} m for the brook drawn before");
    let unheld_new = new_unheld(&edit.land.world, &land.world, &whole, weight).expect("worlds with water");
    assert_eq!((unheld_new.outside(), unheld_new.band(), unheld_new.inside()), (0, 0, 0), "new unheld water: {unheld_new:?}");
    println!(
        "the brook widened: {cells} footprint cells, {:.2} ha; mean width inside {inside:.3} m against {before:.3} m before and {grown:.3} m grown whole ({count} points), the widest {widest:.2} m; {:?}",
        edit.outline.inside_m2 / 10_000.0,
        edit.timings
    );
}

/// The disc of the prototype's case (ii) held and the brook widened, its line the same: taken, every
/// held cell bit for bit, the brook's points through the disc the kept land's, its water not rising
/// where it enters or leaves the disc nor anywhere down it, and no new unheld water.
#[test]
fn a_river_widened_through_held_ground_runs_on_through_it() {
    let land = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let wider = wider_brook(&land);
    let edit = land.edit(&wider, &never()).expect("the brook widened through held ground where its line is kept");
    assert_kept("the brook widened through held ground", &land, &edit);
    let held: Vec<usize> = (0..land.marks.held.len()).filter(|&cell| land.marks.held[cell]).collect();
    assert!(held.iter().all(|&cell| edit.outline.weight.data[cell] == 0.0));
    let in_disc = |point: &CoursePoint| (f64::from(point.x_m) - ON_THE_BROOK.0).hypot(f64::from(point.z_m) - ON_THE_BROOK.1) < DISC_M - 1.0;
    let through: Vec<_> = (edit.land.world.water.rivers.iter()).filter(|course| course.drawn == Some(1) && course.points.iter().any(in_disc)).collect();
    assert_eq!(through.len(), 1, "one course of the brook runs through the disc");
    let kept: BTreeSet<[u32; 5]> = land.world.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
    let points = through[0].points.iter().filter(|point| in_disc(point)).count();
    assert!(points > 10, "the brook runs through the disc: {points} points");
    assert!(through[0].points.iter().filter(|point| in_disc(point)).all(|point| kept.contains(&point_bits(point))), "its points in the disc are the kept land's");
    assert_eq!(rise_of(&through[0].points), 0.0, "the brook's water never rises");
    assert_eq!(edit.water.rise_most_m, 0.0);
    let whole = grown_whole(&wider, SEED);
    let found = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
    assert_eq!((found.outside(), found.band(), found.inside()), (0, 0, 0), "new unheld water: {found:?}");
    println!("the brook widened through held ground: {} held cells kept, {points} points through the disc, notes {:?}", held.len(), edit.notes);
}

/// The cells of the example's lake, worked out here as the land tells a lake's water: wet cells
/// whose surface lies within 2 mm of the lake's level, within 1.5 of its radii and 16 m of its centre.
fn lake_cells(land: &Land) -> Vec<usize> {
    let (lake, water) = (&land.drawing.lakes[0], land.world.water.lakes.iter().find(|lake| lake.drawn == 0).expect("the lake holds water"));
    let (size, depth) = (SIZE_M as usize, &land.world.layers[WATER_LAYER]);
    (0..size * size)
        .filter(|&cell| {
            let (x, z) = place(size, cell);
            let surface_m = f64::from(land.world.heights.data[cell] + depth.data[cell]);
            depth.data[cell] > 0.0 && (x - lake.x_m).hypot(z - lake.z_m) <= 1.5 * f64::from(lake.radius_m) + 16.0 && (surface_m - f64::from(water.level_m)).abs() < 0.002
        })
        .collect()
}

/// `Land::hold`: holding one cell of the lake holds every cell of its water, and lifting the hold on
/// one cell of it lifts it on all.
#[test]
fn holding_one_cell_of_a_lake_holds_all_of_its_water() {
    let mut land = example().clone();
    let cells = lake_cells(&land);
    assert!(cells.len() > 10_000, "the lake's {} cells", cells.len());
    let mut one = vec![false; land.marks.held.len()];
    one[cells[cells.len() / 2]] = true;
    land.hold(&one, true);
    let held: Vec<usize> = (0..land.marks.held.len()).filter(|&cell| land.marks.held[cell]).collect();
    assert_eq!(held, cells, "every cell of the lake's water, and nothing else");
    let mut other = vec![false; land.marks.held.len()];
    other[cells[0]] = true;
    land.hold(&other, false);
    assert!(!land.marks.held.contains(&true), "lifted from one cell, the hold is lifted from the whole lake");
    let mut beside = vec![false; land.marks.held.len()];
    beside[0] = true;
    land.hold(&beside, true);
    assert_eq!(land.marks.held.iter().filter(|&&held| held).count(), 1, "a cell away from every lake holds itself alone");
    println!("the lake: {} cells held at once", cells.len());
}

/// An edit that moves a lake any of whose cells is held is refused, naming the lake (its water
/// would otherwise stay on the held cells, naming no lake); the land is untouched.
#[test]
fn a_held_lake_cannot_be_moved() {
    let mut land = example().clone();
    let cells = lake_cells(&land);
    let mut one = vec![false; land.marks.held.len()];
    one[cells[0]] = true;
    land.hold(&one, true);
    let before = land.clone();
    let mut moved = land.drawing.clone();
    moved.lakes[0].x_m += 50.0;
    let sentence = refused(land.edit(&moved, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("Lake 1 is held at (") && sentence.ends_with("; lift the hold on it first."), "{sentence}");
    let (x, z) = parse_place(&sentence);
    let size = SIZE_M as usize;
    assert!(land.marks.held[((z + size as i64 / 2) as usize) * size + (x + size as i64 / 2) as usize], "the place named is held");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// W4: a lake whose zone the outline takes is taken whole or the edit is refused. Edit `e2` (a lake
/// added on the brook) takes the example lake's zone; held ground 50 m west of the lake's edge lowers
/// the weight of its zone's cells, and the edit is refused naming lake 1.
#[test]
fn a_lake_beside_held_ground_is_not_taken_in_part() {
    const WEST_OF_THE_LAKE: (f64, f64) = (-272.0, -160.0);
    const RADIUS_M: f64 = 30.0;
    let land = holding(example(), &[(WEST_OF_THE_LAKE, RADIUS_M)]);
    let before = land.clone();
    let sentence = refused(land.edit(&edit_of(&land, "e2"), &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("Lake 1 would not be taken whole: its water and shore lie within 150 m of held ground"), "{sentence}");
    assert!(sentence.ends_with("; keep the edit farther from the lake, or lift the hold there."), "{sentence}");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
    let free = example().edit(&edit_of(&land, "e2"), &never()).expect("without the hold the edit is taken");
    assert_kept("e2", example(), &free);
}

/// Stage 2's ruling R18, N2: the refusals a tool says before an edit runs are the edit's own words.
/// A lake moved while held: `held_lake_refusal` is the sentence `edit` refuses it with, and there
/// is none for the land's own drawing or for edit `a`, which moves no lake. An area on the lake with
/// held ground 50 m west of its edge: `area_lake_refusal`, from the land's own lakes' zones and the
/// area's outline as a tool works them out, is the sentence `regrow_area` refuses it with, and there
/// is none on the land without the hold.
#[test]
fn the_refusals_said_before_an_edit_runs_are_the_edits_own() {
    const WEST_OF_THE_LAKE: (f64, f64) = (-272.0, -160.0);
    const HELD_M: f64 = 30.0;
    const AREA_M: f64 = 20.0;
    let mut land = example().clone();
    let cells = lake_cells(&land);
    let mut one = vec![false; land.marks.held.len()];
    one[cells[0]] = true;
    land.hold(&one, true);
    let mut moved = land.drawing.clone();
    moved.lakes[0].x_m += 50.0;
    assert_eq!(land.held_lake_refusal(&moved), Some(refused(land.edit(&moved, &never()))));
    assert_eq!(land.held_lake_refusal(&land.drawing), None);
    assert_eq!(land.held_lake_refusal(&edit_of(&land, "a")), None);

    let size = SIZE_M as usize;
    let lake = &example().drawing.lakes[0];
    let area = disc(size, (lake.x_m, lake.z_m), AREA_M);
    let held = holding(example(), &[(WEST_OF_THE_LAKE, HELD_M)]);
    let said = |land: &Land| {
        let model = land.kept_model().expect("the kept model is derived");
        let zones = lake_zones(&land.world, &model, &model, &land.drawing, &land.drawing);
        let shape = area_outline(SIZE_M, &area, &zones, &land.marks.regrown(&area));
        land.area_lake_refusal(&area, &zones, &shape)
    };
    let sentence = refused(held.regrow_area(&area, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("Lake 1 would not be taken whole"), "{sentence}");
    assert_eq!(said(&held), Some(sentence));
    assert_eq!(said(example()), None);
}

/// A lake the edit adds whose water would lie where held ground lowers the weight is refused, naming
/// the lake (edit `e1`, a lake of 60 m away from the rivers, a disc held 100 m beyond its edge).
#[test]
fn a_new_lake_beside_held_ground_is_refused() {
    const WEST_OF_THE_NEW_LAKE: (f64, f64) = (-700.0, 300.0);
    const RADIUS_M: f64 = 40.0;
    let land = holding(example(), &[(WEST_OF_THE_NEW_LAKE, RADIUS_M)]);
    let before = land.clone();
    let sentence = refused(land.edit(&edit_of(&land, "e1"), &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("Lake 2's new water lies within 150 m of held ground") && sentence.ends_with("; move the lake, or lift the hold there."), "{sentence}");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// The controller's ruling 1: an edit derives the kept model from the land's fields alone, so the
/// same edit of a land and of a land rebuilt from its fields (a land that has passed through nothing
/// but them) gives the same land bit for bit, after a first edit too.
#[test]
fn an_edit_of_a_land_rebuilt_from_its_fields_is_the_same_edit() {
    let rebuilt = |land: &Land| Land {
        drawing: land.drawing.clone(),
        asked: land.asked.clone(),
        seed: land.seed,
        config: land.config,
        model_heights: land.model_heights.clone(),
        first_lake_levels: land.first_lake_levels.clone(),
        world: land.world.clone(),
        marks: land.marks.clone(),
        reach_m: land.reach_m,
    };
    let land = example();
    let first = land.edit(&edit_of(land, "a"), &never()).expect("edit a");
    let again = rebuilt(land).edit(&edit_of(land, "a"), &never()).expect("edit a of the rebuilt land");
    assert_eq!(differences(&first.land, &again.land), Vec::<String>::new(), "edit a");
    let second = first.land.edit(&edit_of(&first.land, "c"), &never()).expect("edit c after a");
    let rebuilt_second = rebuilt(&first.land).edit(&edit_of(&first.land, "c"), &never()).expect("edit c of the rebuilt land");
    assert_eq!(differences(&second.land, &rebuilt_second.land), Vec::<String>::new(), "edit c after a");
}

/// `Land::regrow_area`: the land's own drawing regrown as drawn on a disc shaped by hand clears the
/// hand marks there, keeps the held ones, and, the area being its own outline (stage 2's Task 3),
/// weighs above 0 the disc's cells alone and keeps the land outside it bit for bit, the brook that
/// runs through it among it; an area of no cell gives the land back.
#[test]
fn regrowing_an_area_as_drawn_clears_its_hand_marks() {
    const IN_THE_BROOKS_VALLEY: (f64, f64) = (140.0, 560.0);
    const RADIUS_M: f64 = 40.0;
    /// The ring of hand-shaped ground round the held disc, and the area's beyond it, in metres.
    const RING_M: f64 = 20.0;
    let size = SIZE_M as usize;
    let mut land = holding(example(), &[(FAR_FROM_THE_WATER, DISC_M)]);
    let area = disc(size, IN_THE_BROOKS_VALLEY, RADIUS_M);
    land.marks.by_hand = area.clone();
    let edit = land.regrow_area(&area, &never()).expect("an area regrown");
    assert!(!edit.land.marks.by_hand.contains(&true), "the hand marks of the area are cleared");
    assert_eq!(edit.land.marks.held, land.marks.held, "the held marks stay");
    assert_outline_is_the_area("an area regrown", &area, &edit);
    let (_, _, moved) = assert_kept("an area regrown", &land, &edit);
    assert_eq!(moved, 0, "an area regrown: no course point outside it but the kept land's");
    assert_eq!(edit.land.drawing, land.drawing);
    let none = land.regrow_area(&vec![false; size * size], &never()).expect("no area");
    assert_eq!(differences(&none.land, &land), Vec::<String>::new());
    println!("an area regrown: {:.2} ha, notes {:?}; {:?}", edit.outline.inside_m2 / 10_000.0, edit.notes, edit.timings);
    // Stage 2's ruling R4 (N1): a held cell's ground does not change, so a held cell of the
    // area keeps its hand mark. Hand marks on the held disc and a ring round it, inside the area.
    let shaped = disc(size, FAR_FROM_THE_WATER, DISC_M + RING_M);
    land.marks.by_hand = shaped.clone();
    let area = disc(size, FAR_FROM_THE_WATER, DISC_M + 2.0 * RING_M);
    let edit = land.regrow_area(&area, &never()).expect("an area round a held disc regrown");
    let (held, by_hand) = (&land.marks.held, &edit.land.marks.by_hand);
    let kept = (0..size * size).filter(|&cell| shaped[cell] && held[cell]).count();
    assert!(kept > 0 && (0..size * size).filter(|&cell| shaped[cell]).all(|cell| by_hand[cell] == held[cell]), "the held cells keep their hand marks, the others lose them");
    assert_eq!(edit.land.marks.held, land.marks.held, "the held marks stay");
    let (_, _, moved) = assert_kept("an area round a held disc", &land, &edit);
    assert_eq!(moved, 0, "an area round a held disc: no course point outside it but the kept land's");
    println!("an area round a held disc regrown: {kept} held cells keep their hand marks, notes {:?}", edit.notes);
}

/// The radius of the disc stage 2's Task 3 regrows as drawn, in metres (a disc 400 m across).
const REGROWN_M: f64 = 200.0;
/// The radius of the part of case (ii)'s held disc the stage 1 gate's case 6b lifted and regrew.
const LIFTED_M: f64 = 30.0;
/// The ramp of a weight across the example's middle the test of the land joined with itself reads,
/// as `s((x + z) / RAMP_ACROSS_M + 1/2)`: the band 212 m wide along the line from the north-east to
/// the south-west through the middle, so the lake's water lies in it, partly at weight 0, and the
/// courses out of it start in it.
const RAMP_ACROSS_M: f64 = 300.0;
/// A weight the same on every cell, so every cell lies in the band and every course touches it.
const EVERYWHERE_WEIGHT: f32 = 0.5;
/// The trials' size and seed, as the editor tool's sets grow them (`land_sets.rs`).
const TRIAL_SIZE_M: u32 = 1024;
const TRIAL_SEED: u64 = 3;

/// The smooth step an outline's weight rises by, `t² (3 - 2 t)` with `t` held to 0..1, worked in
/// `f64` and kept in `f32`: a point `d` metres from the nearest point outside the outline or fixed
/// weighs `smooth(d / BAND_M)`.
fn smooth(t: f64) -> f32 {
    let t = t.clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// Stage 2's Task 3 (question 1, answer (a)): the outline of an area regrown as drawn, which touches
/// no lake's zone, is the area: every world cell of it that is not fixed weighs above 0, and every
/// other cell 0; its area is the area's.
fn assert_outline_is_the_area(name: &str, area: &[bool], edit: &Edit) {
    let weight = &edit.outline.weight.data;
    for (cell, &w) in weight.iter().enumerate() {
        assert_eq!(w > 0.0, area[cell] && !edit.land.marks.fixed(cell), "{name}: cell {cell} weighs {w}");
    }
    assert_eq!(edit.outline.inside_m2, area.iter().filter(|&&on| on).count() as f64, "{name}: the outline's area is the area's");
}

/// How the cells `edit` weighs as `pick` asks differ from `before`'s, bit for bit: for the heights,
/// the ground map and each layer, the cells that differ and the most by which they do (0 for the
/// ground map).
fn differing(before: &Land, edit: &Edit, pick: impl Fn(f32) -> bool) -> Vec<(String, usize, f32)> {
    let (old, new, weight) = (&before.world, &edit.land.world, &edit.outline.weight.data);
    let cells: Vec<usize> = (0..weight.len()).filter(|&cell| pick(weight[cell])).collect();
    let grid = |name: &str, a: &[f32], b: &[f32]| {
        let differ: Vec<f32> = cells.iter().filter(|&&cell| a[cell].to_bits() != b[cell].to_bits()).map(|&cell| (a[cell] - b[cell]).abs()).collect();
        (name.to_string(), differ.len(), differ.iter().copied().fold(0.0, f32::max))
    };
    let mut found = vec![grid("heights", &old.heights.data, &new.heights.data)];
    found.push(("ground map".to_string(), cells.iter().filter(|&&cell| old.ground.data[cell] != new.ground.data[cell]).count(), 0.0));
    for (name, layer) in &old.layers {
        found.push(grid(name, &layer.data, &new.layers[name].data));
    }
    found
}

/// Stage 2's Task 3 (Review Focus 8) with stage 2's ruling R4 (S1), on the land just grown,
/// whose ground is the drawing grown whole: a disc of 200 m on the brook's valley, and one on the
/// main river at (300, 4), where a brook ends on the river in the band, regrown as drawn, are each
/// their own outline and give the land back bit for bit but the timings, its water with it (before
/// stage 2's ruling R4 the band's water moved on 778 and 1,164 cells, by up to 0.0119 m and
/// 0.197 m, where a band cell inside the river's bed took the brook's surface, and elsewhere by
/// single precision's step).
#[test]
fn a_disc_regrown_on_the_land_just_grown_gives_the_land_back_bit_for_bit() {
    let land = example();
    for at in [ON_THE_BROOK, ON_THE_MAIN_RIVER] {
        let area = disc(SIZE_M as usize, at, REGROWN_M);
        let edit = land.regrow_area(&area, &never()).expect("the disc regrown as drawn");
        assert_outline_is_the_area("the disc regrown", &area, &edit);
        let (cells, points, moved) = assert_kept("the disc regrown", land, &edit);
        assert_eq!(moved, 0, "no course point outside the disc but the kept land's");
        assert_eq!(differences(&edit.land, land), Vec::<String>::new(), "the disc at {at:?} regrown on the land just grown");
        assert_eq!(edit.notes, Vec::<String>::new(), "the disc at {at:?}: no note");
        println!("a disc of {REGROWN_M} m at {at:?} regrown on the land just grown: {cells} cells and {points} course points outside it kept bit for bit, the land as it was; {:?}", edit.timings);
    }
}

/// The drawings the test of ground regrown as it was grows: the example (its own size and seed),
/// and four trials at `TRIAL_SIZE_M` from `TRIAL_SEED`: `twoout` (a lake with two rivers out of
/// it), `wide` (whose lake level `grow` gives a last bit above `model_water`'s), `headjoin` (whose
/// notes about river 1 stand apart, a note about river 2 between them) and `lake3`.
const REGROWN_AS_IT_WAS: [&str; 4] = ["twoout", "wide", "headjoin", "lake3"];
/// The disc that lies far from each trial's lake, and the disc on a lake, its middle the lake's
/// drawn centre: radii and place in metres.
const FAR_DISC: ((f64, f64), f64) = ((-300.0, 300.0), 100.0);
const REGROWN_ON_THE_LAKE_M: f64 = 60.0;

/// Stage 2's ruling R8 (M1): an area regrown as drawn on ground the drawing grows as it is
/// gives the land's file back but its timings in `META`, by a disc that takes a lake whole and by
/// one far from it (a disc of 200 m at the middle where the drawing has no lake), on the example
/// and the trials `REGROWN_AS_IT_WAS`. Before stage 2's ruling R8 a regrowth of `wide` set its
/// lake one bit lower,
/// as `model_water` gives it (bits 1084035204 to 1084035203), and moved, where it took the lake,
/// its heights on 979 cells and its water on 3,023; one of `headjoin` away from river 1 put river
/// 1's notes together.
#[test]
fn ground_regrown_as_it_was_gives_the_lands_file_back_on_five_drawings() {
    let mut lands: Vec<(&str, Land)> = vec![("example", example().clone())];
    for name in REGROWN_AS_IT_WAS {
        let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::hills()).expect("a trial drawing");
        lands.push((name, Land::grow(&drawing, TRIAL_SEED, &GenConfig::default(), &never()).expect("the trial grows")));
    }
    for (name, land) in &lands {
        let size = land.drawing.size_m as usize;
        let near = match land.drawing.lakes.first() {
            Some(lake) => (format!("a disc of {REGROWN_ON_THE_LAKE_M} m on lake 1"), disc(size, (lake.x_m, lake.z_m), REGROWN_ON_THE_LAKE_M)),
            None => (format!("a disc of {REGROWN_M} m at the middle"), disc(size, (0.0, 0.0), REGROWN_M)),
        };
        let far = (format!("a disc of {} m at {:?}", FAR_DISC.1, FAR_DISC.0), disc(size, FAR_DISC.0, FAR_DISC.1));
        for (area_name, area) in [near, far] {
            let edit = land.regrow_area(&area, &never()).unwrap_or_else(|error| panic!("{name}, {area_name}: {error}"));
            let mut back = edit.land.clone();
            back.world.meta.timings = land.world.meta.timings;
            assert!(back.to_bytes() == land.to_bytes(), "{name}, {area_name}: the land's file but its timings: {:?}", differences(&edit.land, land));
            println!("{name}, {area_name}: {:.2} ha, the land's file back but its timings", edit.outline.inside_m2 / 10_000.0);
        }
    }
}

/// The lands in mountains whose lake's water spreads beyond the reach `outline::lake_water` gives
/// it (up a river's valley, 112 to 274 m from the lake's centre), by trial and seed, at
/// `TRIAL_SIZE_M`: the four a census of 105 untouched regrowths found not given back before stage
/// 2's ruling R11 (every trial from seeds 3 and 11 and the example at 1,024 m from seed 11, in
/// each kind of country).
const SPREAD_LAKES: [(&str, u64); 4] = [("lake3", 3), ("lake3", 11), ("chain", 3), ("flat", 11)];

/// Stage 2's ruling R11 (M1): where a lake's water spreads off every bed beyond the reach
/// `lake_water` gives the lake, ground regrown as it was still gives the land's file back but its
/// timings: by a disc of `REGROWN_M` at the middle on each of `SPREAD_LAKES` in mountains. Before
/// stage 2's ruling R11 the band's spread water at the lake's level came down a few steps of single precision, on
/// 3,212, 2,494, 89 and 251 cells, by at most 5.96e-8 m: the join bounded it again by rims worked
/// out again in `f32`.
#[test]
fn ground_regrown_as_it_was_gives_the_land_back_where_a_lakes_water_spreads_beyond_its_reach() {
    for (name, seed) in SPREAD_LAKES {
        let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::mountains()).expect("a trial drawing");
        let land = Land::grow(&drawing, seed, &GenConfig::default(), &never()).expect("the trial grows");
        let size = TRIAL_SIZE_M as usize;
        let edit = land.regrow_area(&disc(size, (0.0, 0.0), REGROWN_M), &never()).unwrap_or_else(|error| panic!("{name} from {seed}: {error}"));
        let mut back = edit.land.clone();
        back.world.meta.timings = land.world.meta.timings;
        assert!(back.to_bytes() == land.to_bytes(), "{name} in mountains from {seed}: the land's file but its timings: {:?}", differences(&edit.land, &land));
        println!("{name} in mountains from {seed}, a disc of {REGROWN_M} m at the middle: the land's file back but its timings");
    }
}

/// The lake edits the test of a lake's outflow makes: the lake moved east, and its radius changed,
/// by these many metres and this share; the trial whose lake has two rivers out of it.
const LAKE_MOVED_EAST_M: f64 = 10.0;
const LAKE_RADIUS_SHARE: f32 = 1.1;
const TWO_OUT: &str = "twoout";

/// Stage 2's ruling R9 (the design's W3: a course that crosses the outline is one course): a
/// lake moved or resized has no partner, so before stage 2's ruling R9 the drawn river out of it
/// pairs with none
/// and is split where it crosses the outline, cut off there and narrowed to nothing at the same
/// place. Now the river's courses no lake partner pairs pair by their order among its courses. On
/// the example's lake moved 10 m east and its radius made 10 % larger, and on `twoout`'s lake moved
/// 10 m east: the course out of the lake is joined with its partner, no drawn river is cut off or
/// narrowed at the outline and no note says so, the land outside the outline is kept, nothing new
/// is unheld and no course rises.
#[test]
fn a_lake_moved_or_resized_keeps_the_river_out_of_it_one_course_across_the_outline() {
    let twoout = Land::grow(&trial_drawing(TWO_OUT, TRIAL_SIZE_M, Country::hills()).expect("a trial"), TRIAL_SEED, &GenConfig::default(), &never()).expect("the trial grows");
    let moved = |drawing: &Drawing| {
        let mut edited = drawing.clone();
        edited.lakes[0].x_m += LAKE_MOVED_EAST_M;
        edited
    };
    let resized = |drawing: &Drawing| {
        let mut edited = drawing.clone();
        edited.lakes[0].radius_m *= LAKE_RADIUS_SHARE;
        edited
    };
    let cases = [("the example's lake moved", example(), moved(&example().drawing)), ("the example's lake resized", example(), resized(&example().drawing)), ("twoout's lake moved", &twoout, moved(&twoout.drawing))];
    for (name, land, edited) in cases {
        let edit = land.edit(&edited, &never()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let outflows: Vec<usize> = (0..land.world.water.rivers.len()).filter(|&j| land.world.water.rivers[j].drawn.is_some() && land.world.water.rivers[j].from_lake == Some(0)).collect();
        assert!(!outflows.is_empty(), "{name}: a drawn river flows out of the lake");
        for &j in &outflows {
            assert!(edit.water.joins.iter().any(|join| join.old == j), "{name}: the kept course {j} out of the lake is joined");
        }
        let split = edit.water.ended_rivers.iter().chain(&edit.water.narrowed_rivers).filter(|river| river.is_some()).count();
        assert_eq!(split, 0, "{name}: no drawn river cut off or narrowed at the outline: {:?} {:?}", edit.water.brooks_ended, edit.water.brooks_narrowed);
        assert!(!edit.notes.iter().any(|note| note.contains("cut off") || note.contains("narrowed to nothing")), "{name}: {:?}", edit.notes);
        let (_, _, moved_points) = assert_kept(name, land, &edit);
        assert_eq!(moved_points, 0, "{name}: no course point at weight 0 but the kept land's");
        let new = new_unheld(&edit.land.world, &land.world, &grown_whole(&edited, land.seed), &edit.outline.weight).expect("lands hold the water layer");
        assert_eq!((new.outside(), new.band(), new.inside()), (0, 0, 0), "{name}: new unheld {new:?}");
        let rise = edit.land.world.water.rivers.iter().map(|course| rise_of(&course.points)).fold(0.0, f32::max);
        assert!(rise <= 0.0, "{name}: a course rises {rise} m");
        println!("{name}: {} courses joined, {} crossing; notes {:?}", edit.water.joins.len(), edit.water.crossings, edit.notes);
    }
}

/// The point of the main river the test of a river's water gone two ways drags, by its place in the
/// river's points, and how far north, in metres (`z` the less by it; the review of stage 2's rulings
/// R8 and R9 called it 30 m south).
const DRAGGED_POINT: usize = 3;
const DRAGGED_NORTH_M: f64 = 30.0;

/// Stage 2's ruling R13: two courses of one drawn river that meet nowhere at the outline are left
/// apart. Dragging the example's main river's fourth point 30 m north sends the brook (drawn river
/// 2) of the edited drawing's world another way, north-west to the world's edge across the outline,
/// where the kept brook runs south wholly inside it. Paired by their river, the new course's band
/// points were drawn toward the kept line's head 280 m away and the course mended down 26.7 m,
/// leaving 9 dry band cells. Now the new course is narrowed to nothing at the outline and said,
/// the note about river 2, nothing new is unheld, every cell outside the outline is kept bit for
/// bit and no course rises.
/// (A brook in the south, joined across the outline with a rise of 0.88 m before the mending, still
/// puts 3 points at weight 0 that are not the kept land's, with or without this rule, so
/// `assert_kept` is not asked here: stage 2's ruling R13 says it.)
#[test]
fn a_rivers_courses_that_meet_nowhere_at_the_outline_are_left_apart() {
    let land = example();
    let mut edited = land.drawing.clone();
    edited.rivers[0].points[DRAGGED_POINT].1 -= DRAGGED_NORTH_M;
    let edit = land.edit(&edited, &never()).expect("the point dragged");
    let brook = (0..land.world.water.rivers.len()).find(|&j| land.world.water.rivers[j].drawn == Some(1)).expect("the kept brook");
    assert!(!edit.water.joins.iter().any(|join| join.old == brook), "the kept brook is not joined with the new one: {:?}", edit.water.joins);
    let narrowed = edit.notes.iter().position(|note| note.starts_with("River 2 was narrowed to nothing at the outline"));
    assert!(narrowed.is_some_and(|at| edit.notes_about[at] == About::River(1)), "said, about river 2: {:?} {:?}", edit.notes, edit.notes_about);
    let new = new_unheld(&edit.land.world, &land.world, &grown_whole(&edited, SEED), &edit.outline.weight).expect("lands hold the water layer");
    assert_eq!((new.outside(), new.band(), new.inside()), (0, 0, 0), "new unheld {new:?}");
    let (old, after, weight) = (&land.world, &edit.land.world, &edit.outline.weight.data);
    let outside_changed = (0..weight.len())
        .filter(|&cell| weight[cell] == 0.0)
        .filter(|&cell| {
            after.heights.data[cell].to_bits() != old.heights.data[cell].to_bits()
                || after.ground.data[cell] != old.ground.data[cell]
                || old.layers.iter().any(|(layer, grid)| after.layers[layer].data[cell].to_bits() != grid.data[cell].to_bits())
        })
        .count();
    assert_eq!(outside_changed, 0, "every cell outside the outline kept");
    let rise = edit.land.world.water.rivers.iter().map(|course| rise_of(&course.points)).fold(0.0, f32::max);
    assert!(rise <= 0.0, "a course rises {rise} m");
    println!("the main river's 4th point 30 m north: {:.2} ha, notes {:?}", edit.outline.inside_m2 / 10_000.0, edit.notes);
}

/// The two-lake example the test of a river's chain of lakes grows: the example at this size, in
/// hills from `SEED`, with a second lake on the main river below the first, here and this wide; and
/// the place below the second lake its first lake is moved to.
const TWO_LAKES_M: u32 = 1024;
const SECOND_LAKE: Lake = Lake { x_m: 308.0, z_m: 78.0, radius_m: 40.0 };
const BELOW_THE_SECOND: (f64, f64) = (450.0, 148.0);

/// Stage 2's ruling R11 (S1): a drawn river's courses pair by their order only where their chain
/// of lakes agrees at every place. The main river of the two-lake example runs head to lake 1,
/// lake 1 to lake 2, lake 2 to the edge. With lake 1 moved below lake 2 and lake 2 made 10 %
/// larger, the order names other stretches (the kept lake 1 to lake 2 against the new lake 2 to
/// lake 1), and the chain says so (lake 2, resized where it was, is the kept lake 2): those
/// courses are not paired by order, as before stage 2's ruling R9 (that ruling alone paired them). With
/// both lakes moved 10 m east, the chain agrees, and both are paired.
#[test]
fn a_rivers_courses_pair_by_order_only_where_its_chain_of_lakes_agrees() {
    let mut drawing = Drawing { country: Country::hills(), ..Drawing::example_valley(TWO_LAKES_M) };
    drawing.lakes.push(SECOND_LAKE);
    let land = Land::grow(&drawing, SEED, &GenConfig::default(), &never()).expect("the two-lake example grows");
    let main: Vec<usize> = (0..land.world.water.rivers.len()).filter(|&j| land.world.water.rivers[j].drawn == Some(0)).collect();
    let starts: Vec<bool> = main.iter().map(|&j| land.world.water.rivers[j].from_lake.is_some()).collect();
    assert_eq!(starts, [false, true, true], "the main river runs head to lake 1, lake 1 to lake 2, lake 2 to the edge");
    let joined = |edit: &Edit, j: usize| edit.water.joins.iter().any(|join| join.old == j);

    let mut reversed = drawing.clone();
    (reversed.lakes[0].x_m, reversed.lakes[0].z_m) = BELOW_THE_SECOND;
    reversed.lakes[1].radius_m *= LAKE_RADIUS_SHARE;
    let edit = land.edit(&reversed, &never()).expect("the lakes reordered");
    assert!(!joined(&edit, main[1]) && !joined(&edit, main[2]), "the kept courses out of the lakes are not paired by order: {:?}", edit.water.joins);
    assert_kept("the lakes reordered", &land, &edit);

    let mut moved = drawing.clone();
    moved.lakes[0].x_m += LAKE_MOVED_EAST_M;
    moved.lakes[1].x_m += LAKE_MOVED_EAST_M;
    let edit = land.edit(&moved, &never()).expect("both lakes moved");
    assert!(joined(&edit, main[1]) && joined(&edit, main[2]), "the kept courses out of the lakes are paired by order: {:?}", edit.water.joins);
    assert_kept("both lakes moved", &land, &edit);
    println!("the two-lake example: the lakes reordered pair none of its courses out of a lake by order, both lakes moved pair both");
}

/// The edits of a lake that splits a drawn river the kept land runs as one course, the tests of
/// stage 2's ruling R20 make (the whole-branch review's M1): a lake of 40 m added on the example's
/// main river at (276, -4), one of 50 m on its brook at (100, 450) and one of 50 m on its main
/// river at (616, 156); on the trials at 1,024 m in hills from seed 3, `chain`'s first lake dragged
/// 50 m down its river, a river 4 m wide led into `wide`'s lake from the south, a hill on `wide` at
/// (-60, 0) that lowers its lake (stage 2's ruling R13, N1.2), and a lake of 30 m added beside
/// `lake3`'s lake on its river 1.
const ON_THE_MAIN_RIVER_40: Lake = Lake { x_m: 276.0, z_m: -4.0, radius_m: 40.0 };
const ON_THE_BROOK_50: Lake = Lake { x_m: 100.0, z_m: 450.0, radius_m: 50.0 };
const DOWN_THE_MAIN_RIVER_50: Lake = Lake { x_m: 616.0, z_m: 156.0, radius_m: 50.0 };
const CHAIN_LAKE_DRAGGED_TO: (f64, f64) = (-209.6, -83.8);
const INTO_WIDES_LAKE: [(f64, f64); 3] = [(-153.6, -500.0), (-150.0, -250.0), (-153.6, -30.0)];
const INTO_WIDES_LAKE_M: f32 = 4.0;
const HILL_ON_WIDE: (f64, f64) = (-60.0, 0.0);
const BESIDE_LAKE3S_LAKE: Lake = Lake { x_m: 95.0, z_m: 38.0, radius_m: 30.0 };

/// The longest stretch between neighbouring points of a joined course the tests of a split river
/// allow: twice the fine steps' cell (4.0 m at most measured on these edits, where the join before
/// stage 2's ruling R20 bridged 336 to 490 m).
const STRETCH_MOST_M: f32 = 2.0 * GROW_CELL_M;

/// The most new steps inside the band's inner edge the test of a split river allows: one, on
/// `wide` with a river led into its lake, where the mixed world before the join holds it too.
const SPLIT_STEPS_INSIDE_MOST: usize = 1;

/// The design's W3 on an edit of a split river: no drawn river cut off at the outline or narrowed
/// to nothing there, and no note says so; no course of the edited land with a stretch longer than
/// `STRETCH_MOST_M`; and, where `lake` names one, a course of drawn river `river` out of the
/// edited drawing's lake `lake` crosses the outline, joined with the kept course's run below the
/// lake. Returns the edit.
fn assert_one_course(name: &str, land: &Land, edited: &Drawing, river: usize, lake: Option<usize>) -> Edit {
    let edit = land.edit(edited, &never()).unwrap_or_else(|error| panic!("{name}: {error}"));
    let split = edit.water.ended_rivers.iter().chain(&edit.water.narrowed_rivers).filter(|river| river.is_some()).count();
    assert_eq!(split, 0, "{name}: no drawn river cut off or narrowed at the outline: {:?} {:?}", edit.water.brooks_ended, edit.water.brooks_narrowed);
    assert!(!edit.notes.iter().any(|note| note.contains("cut off") || note.contains("narrowed to nothing")), "{name}: {:?}", edit.notes);
    let (world, weight) = (&edit.land.world, &edit.outline.weight);
    for (index, course) in world.water.rivers.iter().enumerate() {
        let longest = course.points.windows(2).map(|pair| (pair[1].x_m - pair[0].x_m).hypot(pair[1].z_m - pair[0].z_m)).fold(0.0, f32::max);
        assert!(longest <= STRETCH_MOST_M, "{name}: course {index} has a stretch of {longest} m");
    }
    let Some(lake) = lake else { return edit };
    let lake_index = world.water.lakes.iter().position(|water| water.drawn == lake).unwrap_or_else(|| panic!("{name}: the edited drawing's lake {lake} holds water"));
    let at = |point: &CoursePoint| weight_at(weight, f64::from(point.x_m), f64::from(point.z_m));
    let crosses = |course: &RiverCourse| course.points.iter().any(|point| at(point) == 0.0) && course.points.iter().any(|point| at(point) > 0.0);
    let out_of_it = (0..world.water.rivers.len()).find(|&index| {
        let course = &world.water.rivers[index];
        course.drawn == Some(river) && course.from_lake == Some(lake_index) && crosses(course)
    });
    let out_of_it = out_of_it.unwrap_or_else(|| panic!("{name}: a course of river {} out of the lake crosses the outline", river + 1));
    assert!(edit.water.joins.iter().any(|join| join.course == out_of_it), "{name}: the course out of the lake is joined: {:?}", edit.water.joins);
    edit
}

/// Stage 2's ruling R20 (the whole-branch review's M1): a lake that splits a drawn river the kept
/// land runs as one course leaves the river one course on each side of the lake (the design's W3).
/// The kept course is cut where its water reaches the lake, its run below joined with the course
/// out of the lake and its run above with the course into it, or left out where it lies wholly
/// inside the outline; and a kept course and a new one of the river with no partner that leave the
/// outline at one place are joined. Before it, the kept course was joined with the course into the
/// lake alone: bridged in a straight stretch from the lake to where it leaves the outline (`chain`,
/// 336 m; `wide` with a hill, 490 m; the example's lake added at (616, 156), 408 m, whose backwater
/// the edit takes into its outline since stage 2's ruling R23), or, the course into the lake wholly
/// inside the outline, cut off there beside the course out of the lake, narrowed to nothing at the
/// same place (the example's lakes on the main river and on the brook, `wide` with a river led into
/// its lake, `lake3`). On each, the land outside the outline is kept (`assert_kept`); nothing new is unheld
/// in the band, and inside it no new dry cell or lake cell over water and at most
/// `SPLIT_STEPS_INSIDE_MOST` new step; no course rises; and the band's bend and slope lie within
/// `BAND_GROUND_BEYOND_MOST` of the span at 2,048 m and four times that on the trials, as
/// `land_sets` bounds them (`assert_bars`).
#[test]
fn a_lake_that_splits_a_kept_river_leaves_it_one_course_on_each_side() {
    let trial = |name: &str| Land::grow(&trial_drawing(name, TRIAL_SIZE_M, Country::hills()).expect("a trial"), TRIAL_SEED, &GenConfig::default(), &never()).expect("the trial grows");
    let (chain, wide, lake3) = (trial("chain"), trial("wide"), trial("lake3"));
    let with_lake = |drawing: &Drawing, lake: Lake| {
        let mut edited = drawing.clone();
        edited.lakes.push(lake);
        edited
    };
    let mut dragged = chain.drawing.clone();
    (dragged.lakes[0].x_m, dragged.lakes[0].z_m) = CHAIN_LAKE_DRAGGED_TO;
    let mut led_in = wide.drawing.clone();
    led_in.rivers.push(River { points: INTO_WIDES_LAKE.to_vec(), width_m: INTO_WIDES_LAKE_M });
    let ex = example();
    let cases: [(&str, &Land, Drawing, usize, usize); 7] = [
        ("a lake on the example's main river", ex, with_lake(&ex.drawing, ON_THE_MAIN_RIVER_40), 0, 1),
        ("a lake on the example's brook", ex, with_lake(&ex.drawing, ON_THE_BROOK_50), 1, 1),
        ("a lake down the example's main river", ex, with_lake(&ex.drawing, DOWN_THE_MAIN_RIVER_50), 0, 1),
        ("chain's first lake dragged down its river", &chain, dragged, 0, 0),
        ("a river led into wide's lake", &wide, led_in, 0, 0),
        ("a hill on wide", &wide, hill_edit(&wide.drawing, HILL_ON_WIDE).expect("a hill"), 0, 0),
        ("a lake beside lake3's lake", &lake3, with_lake(&lake3.drawing, BESIDE_LAKE3S_LAKE), 0, 1),
    ];
    for (name, land, edited, river, lake) in cases {
        let edit = assert_one_course(name, land, &edited, river, Some(lake));
        let new = assert_bars(name, land, &edited, &edit);
        println!("{name}: {:.1} ha, {} courses joined; new unheld {new:?}; notes {:?}", edit.outline.inside_m2 / 10_000.0, edit.water.joins.len(), edit.notes);
    }
}

/// The bars of `land_sets` on `edit`, an edit of `land` to `edited`: the land outside the outline
/// kept (`assert_kept`); nothing new unheld in the band, and inside it no new dry cell or lake cell
/// over water and at most `SPLIT_STEPS_INSIDE_MOST` new step; no course rising; and the band's
/// bend and slope within `BAND_GROUND_BEYOND_MOST` of the span at 2,048 m and four times that on
/// the trials. Returns the new unheld water.
fn assert_bars(name: &str, land: &Land, edited: &Drawing, edit: &Edit) -> NewUnheld {
    assert_kept(name, land, edit);
    let whole = grown_whole(edited, land.seed);
    let new = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("lands hold the water layer");
    assert_eq!((new.band(), new.dry[2], new.lake_over_water[2]), (0, 0, 0), "{name}: new unheld {new:?}");
    assert!(new.steps[2] <= SPLIT_STEPS_INSIDE_MOST, "{name}: new unheld {new:?}");
    let rise = edit.land.world.water.rivers.iter().map(|course| rise_of(&course.points)).fold(0.0, f32::max);
    assert!(rise <= 0.0, "{name}: a course rises {rise} m");
    let weight = &edit.outline.weight;
    let (edited_band, kept_band, whole_band) = (band_ground(&edit.land.world, weight), band_ground(&land.world, weight), band_ground(&whole, weight));
    let most = if land.drawing.size_m == SIZE_M { BAND_GROUND_BEYOND_MOST } else { 4.0 * BAND_GROUND_BEYOND_MOST };
    let (bend, slope) = (beyond_span(edited_band.bend, kept_band.bend, whole_band.bend), beyond_span(edited_band.slope, kept_band.slope, whole_band.slope));
    assert!(bend <= most && slope <= most, "{name}: the band's bend {bend} and slope {slope} beyond the span, past {most}");
    new
}

/// The edits of a new lake whose backwater reaches past the outline that the test of stage 2's
/// ruling R23 makes on the example, beside its lake of 50 m added at (616, 156): its lake moved
/// upstream along the main river to (-404, -444) (stage 2's ruling R13, N1.4), and lakes added on
/// the main river, of 40 m at (-524, -598), whose backwater lies in the band, and of 60 m at
/// (900, 296), whose backwater reaches 520 m up the river.
const LAKE_MOVED_UPSTREAM_TO: (f64, f64) = (-404.0, -444.0);
const BACKWATER_IN_THE_BAND: Lake = Lake { x_m: -524.0, z_m: -598.0, radius_m: 40.0 };
const BACKWATER_UP_THE_RIVER: Lake = Lake { x_m: 900.0, z_m: 296.0, radius_m: 60.0 };

/// The area of the outline an edit of `land` to `edited` has before its backwater is taken: the
/// footprint's reach with the lakes' zones (`outline`), in square metres.
fn outline_before_the_backwater_m2(land: &Land, edited: &Drawing) -> f64 {
    let water = &GenConfig::default().water;
    let mut kept_model = model_water(&land.drawing, water, land.seed, &land.model_heights).expect("the kept model's water");
    kept_model.lakes = land.first_lake_levels.clone();
    let new_model = grow(edited, water, land.seed, &never()).expect("the edited drawing grows");
    let found = footprint(&start_of(&land.drawing, water, land.seed).expect("a start"), &start_of(edited, water, land.seed).expect("a start"), &land.drawing, edited);
    let zones = lake_zones(&land.world, &kept_model, &new_model, &land.drawing, edited);
    outline(land.drawing.size_m, &found, &zones, &land.marks, land.reach_m).inside_m2
}

/// Stage 2's ruling R23 (the design's "the outline follows that river ... as far as it must"): a
/// new lake's backwater that reaches past the outline is taken into it. Before it, the new lake
/// stood above the kept river where the course into it crosses the outline (0.95 m on the
/// example's lake of 50 m at (616, 156), 0.16 m on its lake moved upstream, 1.16 m on the lake at
/// (900, 296)), or the band drew the course down toward the kept river, which falls faster there
/// than the new one held flat by the lake (the lake at (-524, -598), 0.30 m); the join mended the
/// course into the lake down to the kept river, and cells beside it held water nothing held (25,
/// 66, 82 and 58). Now the edit takes the kept river's cells over which the new world's river
/// stands more than `BACKWATER_ABOVE_M` above it as a lake's zone, so its outline is larger than
/// the footprint's reach and the lakes' zones give; the river is one course on each side of the
/// lake (`assert_one_course`), and the bars of `land_sets` hold (`assert_bars`).
#[test]
fn a_new_lakes_backwater_past_the_outline_is_taken_into_it() {
    let land = example();
    let with_lake = |lake: Lake| {
        let mut edited = land.drawing.clone();
        edited.lakes.push(lake);
        edited
    };
    let mut moved = land.drawing.clone();
    (moved.lakes[0].x_m, moved.lakes[0].z_m) = LAKE_MOVED_UPSTREAM_TO;
    let cases: [(&str, Drawing, Option<usize>); 4] = [
        ("a lake down the example's main river", with_lake(DOWN_THE_MAIN_RIVER_50), Some(1)),
        ("the example's lake moved upstream", moved, Some(0)),
        ("a lake whose backwater lies in the band", with_lake(BACKWATER_IN_THE_BAND), Some(1)),
        ("a lake whose backwater reaches up the river", with_lake(BACKWATER_UP_THE_RIVER), None),
    ];
    for (name, edited, lake) in cases {
        let edit = assert_one_course(name, land, &edited, 0, lake);
        let before_m2 = outline_before_the_backwater_m2(land, &edited);
        assert!(edit.outline.inside_m2 > before_m2, "{name}: the outline of {} ha takes the backwater beyond {} ha", edit.outline.inside_m2 / 10_000.0, before_m2 / 10_000.0);
        let new = assert_bars(name, land, &edited, &edit);
        println!("{name}: {:.1} ha, {:.1} ha before the backwater, {:.0} ms; new unheld {new:?}; notes {:?}", edit.outline.inside_m2 / 10_000.0, before_m2 / 10_000.0, edit.timings.total_ms, edit.notes);
    }
}

/// Stage 2's ruling R4 (S1): the water join of a land with itself is the identity, by any
/// weight. The land just grown, joined with itself as both the kept world and the new one, keeps its
/// world bit for bit (heights, ground map, every layer, its courses with their lakes and ends, its
/// lakes and notes), by the weight of discs on the brook's valley, on the main river and over the
/// lake (not taken whole, as an area's early outline weighs them), a ramp across the middle through
/// the lake's water, and one weight on every cell; and so do the trials `twoout` (a lake with two
/// rivers out of it) and `wide` at 1,024 m in hills from seed 3.
#[test]
fn a_land_joined_with_itself_keeps_its_water_bit_for_bit() {
    let joined_with_itself = |name: &str, land: &Land, weight: &Grid<f32>| {
        let mut joined = land.world.clone();
        let join = join_water(&mut joined, &land.world, &land.world, weight, &land.drawing, &land.drawing, &land.config.water);
        assert_eq!(world_differences(&joined, &land.world), Vec::<String>::new(), "{name}: the land joined with itself");
        join.joins.len()
    };
    let weights = |land: &Land| {
        let size = land.drawing.size_m as usize;
        let half = size as f64 / 2.0;
        let place = |cell: usize| ((cell % size) as f64 - half, (cell / size) as f64 - half);
        let grid = |weight: &dyn Fn((f64, f64)) -> f32| Grid { width: size, height: size, cell_size: 1.0, data: (0..size * size).map(|cell| weight(place(cell))).collect() };
        let mut weights: Vec<(String, Grid<f32>)> = [(ON_THE_BROOK, REGROWN_M), (ON_THE_MAIN_RIVER, REGROWN_M), ((0.0, 0.0), 1.5 * REGROWN_M)]
            .into_iter()
            .map(|(at, radius_m)| (format!("a disc of {radius_m} m at {at:?}"), early_area_outline(land.drawing.size_m, &disc(size, at, radius_m), &land.marks).weight))
            .collect();
        weights.push(("a ramp across the middle".to_string(), grid(&|(x, z)| smooth((x + z) / RAMP_ACROSS_M + 0.5))));
        weights.push((format!("{EVERYWHERE_WEIGHT} everywhere"), grid(&|_| EVERYWHERE_WEIGHT)));
        weights
    };
    let land = example();
    for (name, weight) in weights(land) {
        let joins = joined_with_itself(&name, land, &weight);
        println!("the example joined with itself by {name}: its world bit for bit, {joins} courses joined");
    }
    for name in ["twoout", "wide"] {
        let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::hills()).expect("a trial drawing");
        let trial = Land::grow(&drawing, TRIAL_SEED, &GenConfig::default(), &never()).expect("the trial grows");
        for (weight_name, weight) in weights(&trial) {
            let joins = joined_with_itself(&format!("{name}, {weight_name}"), &trial, &weight);
            println!("the trial {name} joined with itself by {weight_name}: its world bit for bit, {joins} courses joined");
        }
    }
}

/// The side of the square the test of an area at the world's corner regrows, in metres: the band,
/// so the square's corner cell lies the band from its two inner edges (one of twice the band, held
/// through the change of seed, is refused: the regrown ground's water beside it would not be held).
const CORNER_SQUARE_M: usize = BAND_M as usize;

/// Stage 2's ruling R4 (S2): an area that reaches the world's border takes the land model's
/// margin beyond it into its outline, so the border is no edge of it. A square of 150 m in the
/// north-west corner, held through a change of seed and lifted, so its ground is seed 11's on a land
/// of seed 23, regrown as drawn: each of its cells weighs `smooth(d / BAND_M)`, `d` its distance to
/// the square's two inner edges, so its corner cell weighs 1 (before stage 2's ruling R4 its border
/// cells weighed `smooth(1 / BAND_M)` and no cell more than 0.5); the model's margin beyond the
/// corner weighs 1; every cell outside it is bit for bit, and its ground moves.
#[test]
fn a_square_at_the_worlds_corner_is_regrown_fully_up_to_the_border() {
    let size = SIZE_M as usize;
    let square: Vec<bool> = (0..size * size).map(|cell| cell % size < CORNER_SQUARE_M && cell / size < CORNER_SQUARE_M).collect();
    let mut held = example().clone();
    held.hold(&square, true);
    let mut land = held.reseed(OTHER_SEED, &never()).expect("seed 23 round the corner held").land;
    assert!(land.hold(&square, false) > 0, "the hold lifted");
    let edit = land.regrow_area(&square, &never()).expect("the corner regrown as drawn");
    assert_outline_is_the_area("the corner", &square, &edit);
    let (cells, points, moved) = assert_kept("the corner", &land, &edit);
    assert_eq!(moved, 0, "the corner: no course point outside it but the kept land's");
    let weight = &edit.outline.weight.data;
    for cell in (0..size * size).filter(|&cell| square[cell]) {
        let inside_m = (CORNER_SQUARE_M - cell % size).min(CORNER_SQUARE_M - cell / size);
        assert_eq!(weight[cell], smooth(inside_m as f64 / f64::from(BAND_M)), "the corner: cell {cell}, {inside_m} m inside its inner edges");
    }
    assert_eq!(weight[0], 1.0, "the corner cell");
    assert_eq!(edit.outline.model_weight[0], 1.0, "the model's margin beyond the corner");
    let at_1 = weight.iter().filter(|&&w| w == 1.0).count();
    let inside = differing(&land, &edit, |w| w > 0.0);
    assert!(inside[0].1 > 0, "the corner's ground moves");
    println!("a square of {CORNER_SQUARE_M} m at the north-west corner regrown: {at_1} cells at weight 1; {cells} cells and {points} course points outside it kept bit for bit; inside it {inside:?}; notes {:?}", edit.notes);
}

/// Stage 2's ruling R4 (S2): the whole land painted and regrown as drawn, case (ii)'s disc
/// held: every cell farther than the band and a cell from the held disc weighs 1, to the border
/// (before stage 2's ruling R4 only the cells farther than the band from the border did, 73 % of
/// the land), every other cell less and the held cells 0; the land just grown, so the land comes
/// back bit for bit but the timings, its tight join and share held said.
#[test]
fn the_whole_land_painted_is_regrown_fully_but_round_held_ground() {
    /// The cells within this many metres of the band's distance from the held disc are not asked:
    /// the disc is made of whole cells, so its rim lies within a cell of its radius.
    const RIM_SLACK_M: f64 = 2.0;
    let size = SIZE_M as usize;
    let land = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let edit = land.regrow_area(&vec![true; size * size], &never()).expect("the whole land regrown as drawn");
    let (cells, points, moved) = assert_kept("the whole land", &land, &edit);
    assert_eq!(moved, 0, "the whole land: no course point at weight 0 but the kept land's");
    let held = disc(size, ON_THE_BROOK, DISC_M);
    let band_m = f64::from(BAND_M);
    for (cell, &w) in edit.outline.weight.data.iter().enumerate() {
        let (x, z) = place(size, cell);
        let from_m = (x - ON_THE_BROOK.0).hypot(z - ON_THE_BROOK.1) - DISC_M;
        if held[cell] {
            assert_eq!(w, 0.0, "a held cell, {cell}");
        } else if from_m > band_m + RIM_SLACK_M {
            assert_eq!(w, 1.0, "cell {cell}, {from_m} m from the held disc");
        } else if from_m < band_m - RIM_SLACK_M {
            assert!(w > 0.0 && w < 1.0, "cell {cell}, {from_m} m from the held disc, weighs {w}");
        }
    }
    // The model's margin, far beyond the band from the held disc, weighs 1 all round.
    let model_side = (edit.outline.model_weight.len() as f64).sqrt() as usize;
    let (cell_m, margin_m) = (f64::from(GROW_CELL_M), f64::from(GROW_MARGIN_M));
    let in_margin = |at: usize| at as f64 * cell_m < margin_m || at as f64 * cell_m >= margin_m + size as f64;
    let margin: Vec<f32> = (edit.outline.model_weight.iter().enumerate()).filter(|&(cell, _)| in_margin(cell % model_side) || in_margin(cell / model_side)).map(|(_, &w)| w).collect();
    assert!(!margin.is_empty() && margin.iter().all(|&w| w == 1.0), "the model's margin weighs 1");
    assert_eq!(differences(&edit.land, &land), Vec::<String>::new(), "the whole land regrown on the land just grown");
    let at_1 = edit.outline.weight.data.iter().filter(|&&w| w == 1.0).count();
    println!("the whole land painted: {at_1} cells at weight 1 of {}; {cells} cells and {points} course points at weight 0 kept; notes {:?}", size * size, edit.notes);
}

/// Stage 2's Task 3: a disc of 200 m regrown as drawn where the regrowth changes ground, the land
/// regrown from seed 23 round case (ii)'s held disc and the hold lifted, so the disc's middle is the
/// land of seed 11 and its rim the land of seed 23, the brook running into and out of it: the
/// outline is the disc, every cell outside it is bit for bit, every course point outside it the kept
/// land's both ways, and its ground moves.
#[test]
fn a_disc_regrown_as_drawn_leaves_every_cell_outside_it_bit_for_bit() {
    let size = SIZE_M as usize;
    let held = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let mut land = held.reseed(OTHER_SEED, &never()).expect("seed 23 round the brook's held disc").land;
    assert!(land.hold(&disc(size, ON_THE_BROOK, DISC_M), false) > 0, "the hold lifted");
    let area = disc(size, ON_THE_BROOK, REGROWN_M);
    let edit = land.regrow_area(&area, &never()).expect("the disc regrown as drawn");
    assert_outline_is_the_area("the disc regrown", &area, &edit);
    let (cells, points, moved) = assert_kept("the disc regrown", &land, &edit);
    assert_eq!(moved, 0, "no course point outside the disc but the kept land's");
    let inside = differing(&land, &edit, |w| w > 0.0);
    assert!(inside[0].1 > 0, "the disc's ground moves");
    let water = &edit.water;
    println!(
        "a disc of {REGROWN_M} m regrown round seed 23's held disc: {cells} cells and {points} course points outside it kept bit for bit; inside it {inside:?}; {} crossings, {} joined inside, brooks ended at {:?}, narrowed at {:?}; notes {:?}; {:?}",
        water.crossings, water.joined_inside, water.brooks_ended, water.brooks_narrowed, edit.notes, edit.timings
    );
}

/// Stage 2's Task 3: the stage 1 gate's case 6b. Case (ii)'s disc of 60 m held through edit `a`,
/// then its middle, a disc of 30 m, lifted and regrown as drawn. Stage 1 moved 52.08 ha round the
/// ring that stays held, by up to 3.41 m; now the outline is the lifted disc, every cell outside it
/// is bit for bit, every course point outside it the kept land's, and the disc itself, every cell of
/// it within 30 m of the held ring, moves by little.
#[test]
fn the_stage_1_gates_case_6b_regrows_inside_the_lifted_disc_alone() {
    let size = SIZE_M as usize;
    let held = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let mut land = held.edit(&edit_of(&held, "a"), &never()).expect("edit a beside the held disc").land;
    let lifted = disc(size, ON_THE_BROOK, LIFTED_M);
    assert!(land.hold(&lifted, false) > 0, "the middle of the hold lifted");
    let edit = land.regrow_area(&lifted, &never()).expect("the lifted disc regrown as drawn");
    assert_outline_is_the_area("6b", &lifted, &edit);
    let (cells, points, moved) = assert_kept("6b", &land, &edit);
    assert_eq!(moved, 0, "6b: no course point outside the disc but the kept land's");
    let most = edit.outline.weight.data.iter().copied().fold(0.0, f32::max);
    let inside = differing(&land, &edit, |w| w > 0.0);
    println!(
        "6b: {:.2} ha inside the outline (stage 1: 52.08 ha), the most weight {most}; {cells} cells and {points} course points outside kept bit for bit; inside {inside:?}; notes {:?}; {:?}",
        edit.outline.inside_m2 / 10_000.0, edit.notes, edit.timings
    );
}

/// Stage 2's Task 3 (Review Focus 8): a disc regrown as drawn that reaches into the lake's water
/// takes the lake's zone whole with its band and the zone's margin round it: every cell of the
/// lake's water weighs 1, every cell sampled within the band and margin of it weighs above 0, the
/// outline holds more than the disc (whose early outline is the disc alone), and everything outside
/// the outline is bit for bit, the course points both ways.
#[test]
fn a_disc_reaching_into_the_lakes_water_takes_the_lake_whole_with_its_band() {
    const REACH_IN_M: f64 = 10.0;
    const RADIUS_M: f64 = 40.0;
    const SAMPLE_EVERY: usize = 5;
    let (land, size) = (example(), SIZE_M as usize);
    let lake = lake_cells(land);
    let east = lake.iter().copied().max_by_key(|&cell| cell % size).expect("the lake holds water");
    let (x, z) = place(size, east);
    let area = disc(size, (x + RADIUS_M - REACH_IN_M, z), RADIUS_M);
    assert!(lake.iter().any(|&cell| area[cell]), "the disc reaches into the lake's water");
    let edit = land.regrow_area(&area, &never()).expect("the disc regrown as drawn");
    let weight = &edit.outline.weight.data;
    assert!(lake.iter().all(|&cell| weight[cell] == 1.0), "the lake's water weighs 1");
    let early = early_area_outline(SIZE_M, &area, &land.marks);
    assert_eq!(early.inside_m2, area.iter().filter(|&&on| on).count() as f64, "the early outline is the disc");
    assert!(edit.outline.inside_m2 > early.inside_m2, "the lake's zone and its band are taken");
    // Within the band and margin of the lake's water: of its rim's cells, those beside a cell that is
    // not the lake's, as near as of any of its cells from outside it.
    let around = f64::from(BAND_M) + LAKE_ZONE_MARGIN_M - 1.0;
    let mut is_lake = vec![false; size * size];
    lake.iter().for_each(|&cell| is_lake[cell] = true);
    let rim = lake.iter().copied().filter(|&cell| [cell - 1, cell + 1, cell - size, cell + size].into_iter().any(|next| !is_lake[next]));
    let (lake_x, lake_z): (Vec<f64>, Vec<f64>) = rim.map(|cell| place(size, cell)).unzip();
    let (low_x, high_x) = (lake_x.iter().copied().fold(f64::INFINITY, f64::min) - around, lake_x.iter().copied().fold(f64::NEG_INFINITY, f64::max) + around);
    let (low_z, high_z) = (lake_z.iter().copied().fold(f64::INFINITY, f64::min) - around, lake_z.iter().copied().fold(f64::NEG_INFINITY, f64::max) + around);
    let mut sampled = 0;
    for cell in (0..size * size).step_by(SAMPLE_EVERY) {
        let (cx, cz) = place(size, cell);
        if cx < low_x || cx > high_x || cz < low_z || cz > high_z {
            continue;
        }
        if !is_lake[cell] && lake_x.iter().zip(&lake_z).any(|(&lx, &lz)| (cx - lx).hypot(cz - lz) <= around) {
            sampled += 1;
            assert!(weight[cell] > 0.0, "({cx}, {cz}) lies within the band and margin of the lake's water");
        }
    }
    let (cells, points, moved) = assert_kept("the disc in the lake", land, &edit);
    assert_eq!(moved, 0, "no course point outside the outline but the kept land's");
    let level = |land: &Land| land.world.water.lakes.iter().find(|lake| lake.drawn == 0).map(|lake| lake.level_m);
    println!(
        "a disc of {RADIUS_M} m {REACH_IN_M} m into the lake's {} cells: {:.2} ha inside the outline against the disc's {:.2} ha, {sampled} cells sampled round the lake; {cells} cells and {points} course points outside kept; the lake's level {:?} -> {:?}; notes {:?}",
        lake.len(), edit.outline.inside_m2 / 10_000.0, early.inside_m2 / 10_000.0, level(land), level(&edit.land), edit.notes
    );
}

/// Stage 2's Task 3: an area wholly held, case (ii)'s disc regrown as drawn while it is held, weighs
/// 0 everywhere and gives the land back bit for bit, its file byte for byte, with its notes saying
/// the ground is held; the hand marks on its west half, held cells all of them, stay (stage 2's
/// ruling R8, N6: the shortcut an area held whole takes keeps them, as the general
/// path keeps a held cell's).
#[test]
fn an_area_wholly_held_regrown_as_drawn_gives_the_land_back_bit_for_bit() {
    let size = SIZE_M as usize;
    let mut land = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let area = disc(size, ON_THE_BROOK, DISC_M);
    land.marks.by_hand = (0..size * size).map(|cell| area[cell] && place(size, cell).0 < ON_THE_BROOK.0).collect();
    assert!(land.marks.by_hand.contains(&true), "hand marks on the held disc");
    let edit = land.regrow_area(&area, &never()).expect("the held disc regrown as drawn");
    assert!(edit.outline.weight.data.iter().chain(&edit.outline.model_weight).all(|&w| w == 0.0), "the outline weighs 0 everywhere");
    assert_eq!((edit.outline.inside_m2, edit.outline.fixed_inside_m2), (HELD_DISC_CELLS as f64, HELD_DISC_CELLS as f64));
    assert_eq!(edit.land.marks, land.marks, "the held disc's hand marks stay");
    assert!(edit.land.to_bytes() == land.to_bytes(), "the land's file byte for byte");
    assert_eq!(edit.notes.len(), edit.notes_about.len());
    println!("a held disc regrown: notes {:?}; {:?}", edit.notes, edit.timings);
}

/// The one class `assert_kept` allows (`NEW_POINTS_AT_WEIGHT_0_MOST`), where it is met: the trial
/// `repeated` at 1,024 m in hills from seed 3 with its standard edit, whose join moves one band point
/// of a brook onto a cell of weight 0 beside the band; every other point at weight 0 is the kept
/// land's, bit for bit.
#[test]
fn a_band_point_moved_onto_weight_0_is_the_one_class_found_both_ways() {
    let land = Land::grow(&trial_drawing("repeated", 1024, Country::hills()).expect("a trial"), 3, &GenConfig::default(), &never()).expect("a trial grows");
    let edit = land.edit(&standard_edit(&land.drawing).expect("its standard edit"), &never()).expect("its standard edit is taken");
    let (_, points, moved) = assert_kept("repeated s3", &land, &edit);
    assert_eq!(moved, 1, "repeated s3: the band point moved onto weight 0");
    println!("repeated s3: {points} kept points outside the outline, {moved} point of the edited land at weight 0 not the kept land's");
}

/// Task 8's sentence moved here: `Edit::notes` keep their subjects through `Land::edit`. On trial
/// drawings at 1,024 m in hills (the example notes nothing): the edit's own notes come first, each
/// about the drawn river it names (the controller's ruling R23: `wide`'s river 1 joined across the
/// outline with a rise, mended; cut off there before stage 2's ruling R23 took more of it into the
/// outline) or else about the land (`twoout`'s brook narrowed, `shoulder`'s trough), then notes
/// about what its outline touches; a river the outline does not touch
/// keeps the kept land's notes about it in the edited land, in the mixed world's place, and the edit
/// does not repeat them; every subject names the land or a river or lake of the edited drawing.
#[test]
fn an_edits_notes_keep_their_subjects() {
    let trial = |name: &str, seed: u64| {
        let land = Land::grow(&trial_drawing(name, 1024, Country::hills()).expect("a trial"), seed, &GenConfig::default(), &never()).expect("a trial grows");
        let edit = land.edit(&standard_edit(&land.drawing).expect("its standard edit"), &never()).expect("its standard edit is taken");
        assert_kept(name, &land, &edit);
        for &about in edit.notes_about.iter().chain(&edit.land.world.notes_about) {
            match about {
                About::Land => {}
                About::River(drawn) => assert!(drawn < edit.land.drawing.rivers.len(), "{name}: {about}"),
                About::Lake(drawn) => assert!(drawn < edit.land.drawing.lakes.len(), "{name}: {about}"),
            }
        }
        println!("{name} s{seed}: before {:?} {:?}; edit {:?} {:?}; after {:?} {:?}", land.world.notes, land.world.notes_about, edit.notes, edit.notes_about, edit.land.world.notes, edit.land.world.notes_about);
        (land, edit)
    };
    let (edge, edit) = trial("edge", 1);
    const KEPT: &str = "River 2 ended in a lake; it was carried on to the edge so the lake has a way out.";
    assert_eq!(edge.world.notes.iter().position(|note| note == KEPT), Some(0));
    assert_eq!((edit.land.world.notes[0].as_str(), edit.land.world.notes_about[0]), (KEPT, About::River(1)), "the kept note in the mixed world's place");
    assert!(!edit.notes.iter().any(|note| note == KEPT) && !edit.notes_about.contains(&About::River(1)), "the edit says nothing of a river its outline does not touch");
    let about_river_1 = |notes: &[String], about: &[About]| -> Vec<String> { notes.iter().zip(about).filter(|(_, &a)| a == About::River(0)).map(|(n, _)| n.clone()).collect() };
    assert_eq!(about_river_1(&edit.notes, &edit.notes_about), about_river_1(&edit.land.world.notes, &edit.land.world.notes_about));
    assert!(!about_river_1(&edit.notes, &edit.notes_about).is_empty());

    let (_, edit) = trial("wide", 1);
    assert!(edit.notes[0].starts_with("River 1 rose ") && edit.notes_about[0] == About::River(0), "{:?} {:?}", edit.notes, edit.notes_about);
    let (_, edit) = trial("twoout", 1);
    assert!(edit.notes[0].starts_with("A brook was narrowed to nothing at the outline at (") && edit.notes_about[0] == About::Land, "{:?}", edit.notes);
    let (_, edit) = trial("shoulder", 3);
    let deep: Vec<_> = edit.troughs.iter().filter(|trough| !trough.filled && trough.depth_m > TROUGH_SAID_M).collect();
    assert_eq!(deep.len(), 1, "shoulder's one deep trough: {:?}", edit.troughs);
    let (x, z) = (deep[0].at.0.round() as i64, deep[0].at.1.round() as i64);
    let said = format!("A trough {:.1} m deep was cut at ({x}, {z}) to give a pit in the band a way out.", deep[0].depth_m);
    assert_eq!((edit.notes[0].as_str(), edit.notes_about[0]), (said.as_str(), About::Land));
}

/// Review Focus 4: the cancel flag set before the call, and set from another thread 300 ms and
/// 1,000 ms into an edit (in the growing and in the fine steps at 2 km), each ends it `Cancelled`; a
/// drawing with a river of one point is `BadConfig` naming the river, and one of another size names
/// the sizes, and a land whose world is not its drawing's size names its world; the land given is
/// bit for bit as it was each time. (The flag set during the join, which
/// a clock cannot aim at, is tested inside the crate, `land::tests`, from a hook between its steps.)
#[test]
fn a_cancelled_or_failed_edit_leaves_the_land_as_it_was() {
    let land = example();
    let before = land.clone();
    let edited = edit_of(land, "a");
    assert_eq!(land.edit(&edited, &AtomicBool::new(true)).err(), Some(LandformError::Cancelled));
    assert_eq!(differences(land, &before), Vec::<String>::new());
    for after_ms in CANCEL_AFTER_MS {
        let cancel = AtomicBool::new(false);
        let result = std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(after_ms));
                cancel.store(true, Ordering::Relaxed);
            });
            land.edit(&edited, &cancel)
        });
        assert_eq!(result.err(), Some(LandformError::Cancelled), "the flag set {after_ms} ms in");
        assert_eq!(differences(land, &before), Vec::<String>::new());
    }
    let mut one_point = edited.clone();
    one_point.rivers[1].points.truncate(1);
    match land.edit(&one_point, &never()) {
        Err(LandformError::BadConfig(reason)) => assert!(reason.contains("river 2"), "{reason}"),
        other => panic!("BadConfig naming river 2, not {:?}", other.map(|_| ())),
    }
    match land.edit(&Drawing::example_valley(1024), &never()) {
        Err(LandformError::BadConfig(reason)) => assert!(reason.contains("1024 m") && reason.contains("2048 m"), "{reason}"),
        other => panic!("BadConfig naming the sizes, not {:?}", other.map(|_| ())),
    }
    assert_eq!(differences(land, &before), Vec::<String>::new());
    let mut damaged = land.clone();
    damaged.world.heights = Grid::new(16, 16, 1.0);
    match damaged.edit(&edited, &never()) {
        Err(LandformError::BadConfig(reason)) => assert!(reason.contains("the land's world"), "{reason}"),
        other => panic!("BadConfig naming the land's world, not {:?}", other.map(|_| ())),
    }
}


/// The cells of drawn lake `drawn`'s water in `world`, of `drawing`, worked out here as the land
/// tells a lake's water: wet cells whose surface lies within 2 mm of its level, within 1.5 of its
/// radii and 16 m of its centre; none when the world holds no water for it.
fn lake_cells_of(world: &World, drawing: &Drawing, drawn: usize) -> Vec<usize> {
    let lake = &drawing.lakes[drawn];
    let Some(water) = world.water.lakes.iter().find(|lake| lake.drawn == drawn) else { return Vec::new() };
    let (size, depth) = (world.heights.width, &world.layers[WATER_LAYER]);
    (0..size * size)
        .filter(|&cell| {
            let (x, z) = place(size, cell);
            let surface_m = f64::from(world.heights.data[cell] + depth.data[cell]);
            depth.data[cell] > 0.0 && (x - lake.x_m).hypot(z - lake.z_m) <= 1.5 * f64::from(lake.radius_m) + 16.0 && (surface_m - f64::from(water.level_m)).abs() < 0.002
        })
        .collect()
}

/// `land` with lake 1 held, by one cell of its water (`Land::hold` holds the rest), and the lake's
/// cells.
fn holding_lake_1(land: &Land) -> (Land, Vec<usize>) {
    let cells = lake_cells(land);
    let mut held = land.clone();
    let mut one = vec![false; held.marks.held.len()];
    one[cells[cells.len() / 2]] = true;
    assert_eq!(held.hold(&one, true), cells.len(), "the hold's count of cells it held");
    (held, cells)
}

/// Lake 1's level, place and area, by their bits.
fn lake_1(land: &Land) -> Option<[u32; 4]> {
    land.world.water.lakes.iter().find(|lake| lake.drawn == 0).map(|lake| [lake.level_m, lake.x_m, lake.z_m, lake.area_m2].map(f32::to_bits))
}

/// The whole-branch review's M1: a lake held whole (`Land::hold` holds every cell of its water
/// together) is kept, not refused. Lake 1 of the example held by one of its cells (18,130 cells),
/// each edit whose outline reaches it (`a`, `b`, `c`, `e2`, `g` and `h`: the outline weighs above 0
/// a cell beside the lake's water) is taken round it: every held cell bit for bit (`assert_kept`),
/// the lake's level and its cells as they were, and no new unheld water against the kept land and
/// the edited drawing grown whole.
#[test]
fn a_lake_held_whole_is_kept_and_the_edits_beside_it_are_taken() {
    const LAKE_CELLS: usize = 18_130;
    let land = example();
    let (held, cells) = holding_lake_1(land);
    assert_eq!(cells.len(), LAKE_CELLS);
    let size = SIZE_M as usize;
    let beside = |cell: usize| {
        let (i, j) = (cell % size, cell / size);
        [(i.wrapping_sub(1), j), (i + 1, j), (i, j.wrapping_sub(1)), (i, j + 1)].into_iter().filter(|&(i, j)| i < size && j < size).map(move |(i, j)| j * size + i)
    };
    for name in ["a", "b", "c", "e2", "g", "h"] {
        let edited = edit_of(land, name);
        let edit = held.edit(&edited, &never()).unwrap_or_else(|error| panic!("{name}: {error}"));
        let weight = &edit.outline.weight;
        assert!(cells.iter().flat_map(|&cell| beside(cell)).any(|cell| weight.data[cell] > 0.0), "{name}: the outline reaches the lake");
        assert_kept(name, &held, &edit);
        assert_eq!(lake_1(&edit.land), lake_1(&held), "{name}: lake 1's level, place and area");
        assert_eq!(lake_cells_of(&edit.land.world, &edit.land.drawing, 0), cells, "{name}: lake 1's cells");
        let whole = grown_whole(&edited, SEED);
        let found = new_unheld(&edit.land.world, &held.world, &whole, weight).expect("worlds with water");
        assert_eq!((found.outside(), found.band(), found.inside()), (0, 0, 0), "{name}: new unheld water {found:?}");
        println!("{name} beside the held lake: {:.2} ha, notes {:?}", edit.outline.inside_m2 / 10_000.0, edit.notes);
    }
}

/// The land with lake 1 held regrown from another seed (the whole-branch review's M1). The whole
/// land is the footprint and every zone is taken, but the lake, held whole, is not asked whether it
/// is taken whole (refusal 3): it lies wholly at weight 0, kept. So the regrowth is judged by the
/// water it leaves beside the lake (refusal 4), as beside any held ground; from seed 23 it is
/// refused, naming the regrown ground and the held cell of the lake nearest the first such water:
/// the main river and the brook, regrown on other ground, run beside the lake's kept water at other
/// levels, and the regrown land holds 104 dry cells below the water beside them that the kept land
/// does not (the first at (-59, -286) on the lake's south shore, 0.35 m below it; measured with a
/// print in the refusal, since taken out). The land is untouched.
#[test]
fn a_land_with_a_held_lake_regrown_from_another_seed_is_refused_by_the_water_beside_it() {
    let (held, cells) = holding_lake_1(example());
    let before = held.clone();
    let sentence = refused(held.reseed(OTHER_SEED, &never()));
    println!("seed 23 round the held lake: {sentence}");
    assert!(sentence.starts_with("The regrown ground runs over held ground at (") && sentence.ends_with("; the water there would not be held: keep the edit farther from it, or lift the hold there."), "{sentence}");
    let (x, z) = parse_place(&sentence);
    let size = SIZE_M as usize;
    assert!(cells.contains(&(((z + SIZE_M as i64 / 2) as usize) * size + (x + SIZE_M as i64 / 2) as usize)), "the place named, ({x}, {z}), is a cell of the held lake");
    assert_eq!(differences(&held, &before), Vec::<String>::new());
}

/// The whole-branch review's S1: the cap at a mended course never touches a lake's water. Edit `c`
/// of the example in gentle country from seed 23 mends the main river where it runs into lake 1,
/// which the outline takes (the review found 1,706 of the lake's cells capped below its level).
/// Every cell of the lake's water in the mixed model's world (worked out again from the edited land:
/// `model_water` of its model heights, its first lakes' levels, `finish_drawn`) is its water in the
/// edited land, at its level, and a later `Land::hold` of one cell of it holds every one of them;
/// the cap still lowers the water beside the mended river elsewhere.
#[test]
fn the_cap_at_a_mended_course_leaves_a_lakes_water_at_its_level() {
    const GENTLE_SEED: u64 = 23;
    let config = GenConfig::default();
    let land = Land::grow(&Drawing { country: Country::gentle(), ..Drawing::example_valley(SIZE_M) }, GENTLE_SEED, &config, &never()).expect("the example grows");
    let edited = edit_of(&land, "c");
    let edit = land.edit(&edited, &never()).expect("edit c");
    assert_kept("c gentle 23", &land, &edit);
    assert!(edit.water.capped_at_weight_1 > 0, "the cap still lowers the water beside the mended river: {:?}", edit.water);
    let mut model = model_water(&edited, &config.water, GENTLE_SEED, &edit.land.model_heights).expect("the mixed model");
    model.lakes = edit.land.first_lake_levels.clone();
    let mixed = finish_drawn(GENTLE_SEED, &edited, &config, &model, Frame::Model, &never()).expect("the mixed model's world");
    let lake = edit.land.world.water.lakes.iter().find(|lake| lake.drawn == 0).expect("lake 1 holds water");
    assert!(weight_at(&edit.outline.weight, f64::from(lake.x_m), f64::from(lake.z_m)) > 0.0, "the outline takes lake 1");
    let cells = lake_cells_of(&mixed, &edited, 0);
    assert_eq!(lake_cells_of(&edit.land.world, &edited, 0), cells, "lake 1's water at its level");
    let mut after = edit.land.clone();
    let mut one = vec![false; after.marks.held.len()];
    one[cells[cells.len() / 2]] = true;
    after.hold(&one, true);
    let held: Vec<usize> = (0..after.marks.held.len()).filter(|&cell| after.marks.held[cell]).collect();
    assert_eq!(held, cells, "the hold finds every cell of the lake");
    println!("c gentle 23: lake 1's {} cells held whole; {} cells of weight 1 capped beside the mended river, at most {:.4} m", held.len(), edit.water.capped_at_weight_1, edit.water.capped_most_m);
}

/// The whole-branch review's S2: an edit whose footprint is the whole land says so, first, about
/// the land, with why: another seed, another kind of country, a number of the country changed, on
/// the example at 512 m in hills from seed 1.
#[test]
fn a_whole_land_edit_says_why() {
    let land = Land::grow(&Drawing::example_valley(512), 1, &GenConfig::default(), &never()).expect("the example grows");
    let said = |edit: Edit| (edit.notes[0].clone(), edit.notes_about[0], edit.outline.inside_m2);
    let whole = |why: &str| (format!("This edit regrows the whole land but held and hand-shaped ground: {why}."), About::Land, 512.0 * 512.0);
    assert_eq!(said(land.reseed(2, &never()).expect("another seed")), whole("the seed changed"));
    let mountains = Drawing { country: Country::mountains(), ..land.drawing.clone() };
    assert_eq!(said(land.edit(&mountains, &never()).expect("another kind")), whole("the kind of country changed"));
    let relief = Drawing { country: Country { relief_m: land.drawing.country.relief_m * 1.01, ..land.drawing.country }, ..edit_of(&land, "a") };
    assert_eq!(said(land.edit(&relief, &never()).expect("a number")), whole("a number of the country changed: relief_m"));
    let plain = land.edit(&edit_of(&land, "a"), &never()).expect("edit a");
    assert!(!plain.notes.iter().any(|note| note.starts_with("This edit regrows the whole land")), "{:?}", plain.notes);
}

/// The whole-branch review's S2 (and N5): a change of country beside held ground that would leave
/// water unheld names the regrown ground, not a river whose line did not move. A disc held on the
/// main river where it crosses edit `a`'s outline, the example in gentle country.
#[test]
fn a_change_of_country_beside_held_ground_names_the_regrown_ground() {
    let land = holding(example(), &[(ON_THE_MAIN_RIVER, DISC_M)]);
    let before = land.clone();
    let gentle = Drawing { country: Country::gentle(), ..land.drawing.clone() };
    let sentence = refused(land.edit(&gentle, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("The regrown ground runs ") && sentence.ends_with("; the water there would not be held: keep the edit farther from it, or lift the hold there."), "{sentence}");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// Review Focus 3 through `Land::edit` (the whole-branch review's S9): a lake added on a ridge,
/// where the ground cannot hold it and the engine leaves it out. The land is whole (kept outside
/// the outline, no new unheld water), the edit's notes and the edited land's say which lake, about
/// it, and no zone is taken for it: the outline is the footprint's alone (`early_outline`), bit for
/// bit.
#[test]
fn a_lake_the_ground_cannot_hold_is_left_out_and_said() {
    const RIDGE_LAKE: Lake = Lake { x_m: 316.0, z_m: 420.0, radius_m: 10.0 };
    const LEFT_OUT: &str = "Lake 2 holds no water; it was left out.";
    let land = example();
    let mut drawing = land.drawing.clone();
    drawing.lakes.push(RIDGE_LAKE);
    let edit = land.edit(&drawing, &never()).expect("the lake added");
    assert_kept("a lake on a ridge", land, &edit);
    let about = |notes: &[String], subjects: &[About]| notes.iter().zip(subjects).any(|(note, &subject)| note == LEFT_OUT && subject == About::Lake(1));
    assert!(about(&edit.notes, &edit.notes_about), "{:?}", edit.notes);
    assert!(about(&edit.land.world.notes, &edit.land.world.notes_about), "{:?}", edit.land.world.notes);
    assert!(!edit.land.world.water.lakes.iter().any(|lake| lake.drawn == 1), "the lake holds no water");
    let water = GenConfig::default().water;
    let found = footprint(&start_of(&land.drawing, &water, SEED).expect("a start"), &start_of(&drawing, &water, SEED).expect("a start"), &land.drawing, &drawing);
    let alone = early_outline(SIZE_M, &found, &land.marks, land.reach_m);
    assert!(bits(&edit.outline.weight.data) == bits(&alone.weight.data) && edit.outline.inside_m2 == alone.inside_m2, "the outline is the footprint's alone");
    let whole = grown_whole(&drawing, SEED);
    let new = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
    assert_eq!((new.outside(), new.band(), new.inside()), (0, 0, 0), "new unheld water: {new:?}");
    println!("a lake on a ridge: {:.2} ha, notes {:?}", edit.outline.inside_m2 / 10_000.0, edit.notes);
}

/// The controller's ruling R21: a land the land file's reader would refuse is never made. A river
/// drawn at 0.9 of the land's size (the example at 512 m, its main river 460.8 m wide) gives water
/// wider than the land (576 m), which `Land::grow` refuses with the reader's sentence naming the
/// course, as an edit of the land to that drawing does, the land given untouched.
#[test]
fn a_land_whose_water_the_file_could_not_hold_is_refused() {
    const SIZE: u32 = 512;
    const SHARE: f32 = 0.9;
    let config = GenConfig::default();
    let land = Land::grow(&Drawing::example_valley(SIZE), 1, &config, &never()).expect("the example grows");
    let before = land.clone();
    let mut wide = land.drawing.clone();
    wide.rivers[0].width_m = SHARE * SIZE as f32;
    let reason = |result: Result<Land, LandformError>| match result {
        Err(LandformError::BadConfig(reason)) => reason,
        other => panic!("BadConfig, not {:?}", other.map(|_| ())),
    };
    let grown = reason(Land::grow(&wide, 1, &config, &never()));
    println!("{grown}");
    assert!(grown.starts_with("the land it makes could not be written and read again as a land file") && grown.contains("course"), "{grown}");
    let edited = reason(land.edit(&wide, &never()).map(|edit| edit.land));
    println!("{edited}");
    assert!(edited.starts_with("the land it makes could not be written and read again as a land file") && edited.contains("course"), "{edited}");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// The whole-branch review's N7: the brook taken away and the main river's point moved (edit `c`'s
/// move) in one edit, on the example with its rivers listed brook first (which grows the same
/// valley, drawn rivers joining whatever their order). Paired by order, the brook would be the moved
/// main river and the main river nothing; paired by their lines, the main river is the moved one:
/// every joined course of a drawn river is a course of the main river joined with the moved one,
/// the brook's courses end at the outline, and the land is kept outside it with no new unheld water.
#[test]
fn the_brook_taken_away_and_the_main_river_moved_pair_by_their_lines() {
    let config = GenConfig::default();
    let example = Drawing::example_valley(SIZE_M);
    let brook_first = Drawing { rivers: vec![example.rivers[1].clone(), example.rivers[0].clone()], ..example.clone() };
    let land = Land::grow(&brook_first, SEED, &config, &never()).expect("the example grows");
    let moved = example_edit(&example, "c").expect("edit c").rivers[0].clone();
    let edited = Drawing { rivers: vec![moved], ..brook_first.clone() };
    let edit = land.edit(&edited, &never()).expect("the brook taken away and the main river moved");
    assert_kept("the brook away, the main river moved", &land, &edit);
    let drawn_joins: Vec<_> = edit.water.joins.iter().filter(|join| join.drawn.is_some()).collect();
    assert!(!drawn_joins.is_empty(), "the main river is joined: {:?}", edit.water);
    for join in &drawn_joins {
        assert_eq!((land.world.water.rivers[join.old].drawn, join.drawn), (Some(1), Some(0)), "a joined course of the main river: {join:?}");
    }
    assert!(edit.land.world.water.rivers.iter().all(|course| course.drawn.is_none_or(|drawn| drawn == 0)));
    let whole = grown_whole(&edited, SEED);
    let new = new_unheld(&edit.land.world, &land.world, &whole, &edit.outline.weight).expect("worlds with water");
    assert_eq!(new.band(), 0, "new unheld water in the band: {new:?}");
    println!("the brook away, the main river moved: {} joined, ended {:?} {:?}, new unheld {new:?}, notes {:?}", drawn_joins.len(), edit.water.brooks_ended, edit.water.ended_rivers, edit.notes);
}

/// The gate's problem 2 (the controller's ruling R24): case (ii)'s disc held on the brook's valley,
/// one of the gate's discs, refuses edit `g`, which takes the brook away, by the water it would
/// leave; the sentence names the brook, river 2 of the land's drawing, as the river the edit
/// removes, by its line in the kept model (the land's own model routed again, as an edit derives
/// it), not the main river, whose line the edit does not move (before the ruling: "River 1's new
/// line runs 205 m from held ground at (58, 139); ..." on the gate's folder). The place named is
/// held, the distance is that line's from it in whole metres in the form the band reads it, and
/// the land is untouched.
#[test]
fn a_river_the_edit_removes_is_named_as_removed() {
    let land = holding(example(), &[(ON_THE_BROOK, DISC_M)]);
    let before = land.clone();
    let edited = edit_of(&land, "g");
    assert_eq!(edited.rivers.len() + 1, land.drawing.rivers.len(), "edit g takes the brook away");
    let sentence = refused(land.edit(&edited, &never()));
    println!("{sentence}");
    assert!(sentence.starts_with("River 2, which this edit removes, ran "), "{sentence}");
    assert!(sentence.ends_with("; the water there would not be held: keep the river, or lift the hold there."), "{sentence}");
    let (x, z) = parse_place(&sentence);
    let size = SIZE_M as usize;
    let cell = ((z + SIZE_M as i64 / 2) as usize) * size + (x + SIZE_M as i64 / 2) as usize;
    assert!(land.marks.held[cell], "the place named, ({x}, {z}), is held");
    let kept = model_water(&land.drawing, &land.config.water, land.seed, &land.model_heights).expect("the kept model");
    let line = &kept.rivers.iter().find(|river| river.drawn == 1).expect("the brook is laid in the kept model").line;
    let apart_m = line.windows(2).map(|pair| to_segment((x as f64, z as f64), pair[0], pair[1])).fold(f64::INFINITY, f64::min).round();
    let near = if apart_m == 0.0 {
        format!("ran over held ground at ({x}, {z}); ")
    } else if apart_m < f64::from(BAND_M) {
        format!("ran within {BAND_M} m of held ground at ({x}, {z}), {apart_m} m from it; ")
    } else {
        format!("ran {apart_m} m from held ground at ({x}, {z}); ")
    };
    assert!(sentence.contains(&near), "{sentence}: the brook's line {apart_m} m from the held cell");
    assert_eq!(differences(&land, &before), Vec::<String>::new());
}

/// The radius of the disc a test of a step of undo holds at the example's lake at 512 m: within its
/// water, so the whole lake is held.
const ON_THE_LAKE_M: f64 = 3.0;
/// The reach a test of a step of undo sets, between `REACH_MIN_M` and `REACH_MAX_M`.
const OTHER_REACH_M: f32 = (REACH_MIN_M + REACH_MAX_M) / 2.0;
/// The thread counts a step of undo is worked out on, which must give the same step.
const UNDO_THREADS: [usize; 2] = [1, 8];

/// Stage 2's Task 2: the step of undo between `a` and `b`, the land after it. Swapped into `a` it
/// makes it `b`, field for field and bit for bit and its file byte for byte; swapped again it gives
/// `a` back, a third time `b`; and swapped, it holds `a`'s values: the step from `b` to `a`, exactly.
/// Returns the step as found.
fn assert_undoes(name: &str, a: &Land, b: &Land) -> Change {
    let change = Change::between(a, b).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert!(!change.is_empty(), "{name}: the two lands differ");
    let (a_bytes, b_bytes) = (a.to_bytes(), b.to_bytes());
    assert!(a_bytes != b_bytes, "{name}: the two lands' files differ");
    let mut land = a.clone();
    let mut step = change.clone();
    for (turn, (bytes, other)) in [(&b_bytes, b), (&a_bytes, a), (&b_bytes, b)].into_iter().enumerate() {
        step.swap(&mut land);
        assert_eq!(differences(&land, other), Vec::<String>::new(), "{name}: swap {}", turn + 1);
        assert!(land.to_bytes() == *bytes, "{name}: swap {}, the land's file byte for byte", turn + 1);
    }
    assert!(step == Change::between(b, a).expect("the step back"), "{name}: swapped in, the step holds what the land was");
    println!("{name}: {} bytes", change.bytes());
    change
}

/// The step of undo from `a` to `b` worked out on a pool of `threads` threads.
fn change_on(threads: usize, a: &Land, b: &Land) -> Change {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    pool.install(|| Change::between(a, b)).expect("a step of undo")
}

/// Stage 2's Task 2 (Review Focus 1): the step of an edit (edit `a`, the brook's second point moved)
/// and of an edit from another seed (the whole land, toward edit `a`) each swap in and out bit for
/// bit; the whole land's step holds more; and each step is the same on 1 thread and on 8.
#[test]
fn a_step_of_undo_of_an_edit_swaps_it_in_and_out_bit_for_bit() {
    let land = small_example();
    let edited = edit_of(land, "a");
    let edit = land.edit(&edited, &never()).expect("edit a");
    assert!(edit.outline.inside_m2 > 0.0, "edit a changes ground");
    let step = assert_undoes("edit a", land, &edit.land);
    let whole = land.edit_from_seed(&edited, OTHER_SEED, &never()).expect("edit a from seed 23").land;
    let whole_step = assert_undoes("edit a from seed 23", land, &whole);
    assert!(whole_step.bytes() > step.bytes(), "the whole land holds more than edit a's outline");
    for (name, b) in [("edit a", &edit.land), ("edit a from seed 23", &whole)] {
        let [one, eight] = UNDO_THREADS.map(|threads| change_on(threads, land, b));
        assert!(one == eight, "{name}: the step on {UNDO_THREADS:?} threads");
    }
}

/// Stage 2's Task 2: the step of an area regrown as drawn, a disc held through a change of seed and
/// then lifted, so the regrowth changes its ground, swaps in and out bit for bit.
#[test]
fn a_step_of_undo_of_an_area_regrown_swaps_it_in_and_out_bit_for_bit() {
    let land = small_example();
    let area = disc(SMALL_M as usize, AWAY_FROM_THE_SMALL_WATER, AWAY_RADIUS_M);
    let held = holding(land, &[(AWAY_FROM_THE_SMALL_WATER, AWAY_RADIUS_M)]);
    let mut lifted = held.reseed(OTHER_SEED, &never()).expect("seed 23 round the held disc").land;
    assert!(lifted.hold(&area, false) > 0, "the hold lifted");
    let regrown = lifted.regrow_area(&area, &never()).expect("the disc regrown as drawn");
    let moved = (area.iter().enumerate().filter(|&(_, &on)| on))
        .filter(|&(cell, _)| regrown.land.world.heights.data[cell].to_bits() != lifted.world.heights.data[cell].to_bits())
        .count();
    assert!(moved > 0, "the disc's ground is regrown");
    assert_undoes("a disc regrown", &lifted, &regrown.land);
}

/// Stage 2's Task 2: the step of a hold of a disc on the lake, which holds the lake whole, and the
/// step of lifting it, each swap in and out bit for bit, and hold the marks alone; and a hold of a
/// land whose held marks are shorter than its world (as a land file may keep them), which lengthens
/// them, holds the marks whole and swaps them back bit for bit.
#[test]
fn a_step_of_undo_of_a_hold_and_of_its_lift_swaps_them_bit_for_bit() {
    let land = small_example();
    let size = SMALL_M as usize;
    let lake = land.drawing.lakes[0];
    let area = disc(size, (lake.x_m, lake.z_m), ON_THE_LAKE_M);
    let mut held = land.clone();
    let cells = held.hold(&area, true);
    assert!(cells > area.iter().filter(|&&on| on).count(), "the lake is held whole: {cells} cells");
    let step = assert_undoes("a hold on the lake", land, &held);
    let mut lifted = held.clone();
    assert_eq!(lifted.hold(&area, false), cells, "the lake's hold lifted whole");
    assert_undoes("the hold lifted", &held, &lifted);
    let empty = Change::between(land, land).expect("a land and itself").bytes();
    assert!(step.bytes() < empty + size * size / 8, "a hold's step holds less than the marks at a bit a cell: {} bytes", step.bytes());

    let mut short = land.clone();
    short.marks.held.truncate(size * size / 2);
    let mut lengthened = short.clone();
    assert!(lengthened.hold(&area, true) > 0 && lengthened.marks.held.len() == size * size);
    assert_undoes("a hold of marks shorter than the world", &short, &lengthened);
}

/// Stage 2's Task 2: the step of an ask (a point added on the brook's line, which waits) and of the
/// reach changed each swap in and out bit for bit.
#[test]
fn a_step_of_undo_of_an_ask_and_of_the_reach_swaps_them_bit_for_bit() {
    let land = small_example();
    let mut asking = land.clone();
    asking.ask(point_added(&land.drawing)).expect("a point added on the brook's line");
    assert!(asking.waiting());
    assert_undoes("an ask", land, &asking);
    let mut reaching = land.clone();
    reaching.reach_m = OTHER_REACH_M;
    assert!(OTHER_REACH_M > REACH_MIN_M && OTHER_REACH_M < REACH_MAX_M && reaching.reach_m != land.reach_m);
    assert_undoes("the reach", land, &reaching);
}

/// Stage 2's Task 7: a hold of a disc on the lake holds the lake whole and says so (`hold_lakes`),
/// its lift lifts it whole and says so, and a hold passed again over the lake held changes nothing
/// and names no lake. The step of the hold, of its lift, of an ask, of the reach and of all three at
/// once, made of the land after the call and the parts it had before (`Change::to_parts`), is the
/// step `between` gives from the land after to the land before, exactly, held marks shorter than
/// the world among them, and swapped into the land after gives the land before byte for byte; the
/// land's own parts give an empty step. `touched` names what each step exchanges, and edit `a`'s.
#[test]
fn a_step_of_a_part_is_the_step_between_the_two_lands() {
    let land = small_example();
    let size = SMALL_M as usize;
    let lake = land.drawing.lakes[0];
    let area = disc(size, (lake.x_m, lake.z_m), ON_THE_LAKE_M);
    let mut held = land.clone();
    assert!(land.holds_anything(&area, true) && !land.holds_anything(&area, false), "a hold of unheld ground changes it, a lift does not");
    // The reach worked out once is what the calls of the area work out (stage 2's ruling R14, N4).
    let reach = land.hold_reach(&area);
    assert!(land.holds_anything_in(&reach, true) && !land.holds_anything_in(&reach, false));
    let mut held_by_reach = land.clone();
    let hold = held.hold_lakes(&area, true);
    assert_eq!(held_by_reach.hold_reached(reach, true), hold, "the reach held differs from the area held");
    assert!(held_by_reach.to_bytes() == held.to_bytes(), "the land held by its reach differs from the land held by the area");
    assert!(hold.cells > area.iter().filter(|&&on| on).count() && hold.lakes == [0], "the lake held whole: {hold:?}");
    let mut lifted = held.clone();
    assert!(held.holds_anything(&area, false) && !held.holds_anything(&area, true), "a lift of held ground changes it, a hold does not");
    assert_eq!(lifted.hold_lakes(&area, false), Held { cells: hold.cells, lakes: vec![0] }, "the lake's hold lifted whole");
    assert_eq!(held.clone().hold_lakes(&area, true), Held { cells: 0, lakes: Vec::new() }, "the lake held again");
    // The lake's water held only on the area's cells, as no hold leaves it: holding the area again
    // holds the rest of the lake.
    let mut in_part = land.clone();
    in_part.marks.held = area.clone();
    assert!(in_part.holds_anything(&area, true), "a lake held in part is held whole");
    assert_eq!(in_part.clone().hold_lakes(&area, true).cells, hold.cells - area.iter().filter(|&&on| on).count());
    let mut short = land.clone();
    short.marks.held.truncate(size * size / 2);
    let mut lengthened = short.clone();
    assert!(short.holds_anything(&area, true));
    assert!(lengthened.hold(&area, true) > 0 && lengthened.marks.held.len() == size * size);
    let mut asking = land.clone();
    asking.ask(point_added(&land.drawing)).expect("a point added on the brook's line");
    let mut reaching = land.clone();
    reaching.reach_m = OTHER_REACH_M;
    let mut all = held.clone();
    all.ask(point_added(&land.drawing)).expect("a point added on the brook's line");
    all.reach_m = OTHER_REACH_M;
    let steps = [
        ("a hold", land, &held, Change::to_parts(&held, Some(&land.marks.held), None, None), Touched { held: true, ..Touched::default() }),
        ("its lift", &held, &lifted, Change::to_parts(&lifted, Some(&held.marks.held), None, None), Touched { held: true, ..Touched::default() }),
        ("a hold of marks shorter than the world", &short, &lengthened, Change::to_parts(&lengthened, Some(&short.marks.held), None, None), Touched { held: true, ..Touched::default() }),
        ("an ask", land, &asking, Change::to_parts(&asking, None, Some(&land.asked), None), Touched { drawings: true, ..Touched::default() }),
        ("the reach", land, &reaching, Change::to_parts(&reaching, None, None, Some(land.reach_m)), Touched { rest: true, ..Touched::default() }),
        (
            "all three",
            land,
            &all,
            Change::to_parts(&all, Some(&land.marks.held), Some(&land.asked), Some(land.reach_m)),
            Touched { held: true, drawings: true, rest: true, ..Touched::default() },
        ),
    ];
    for (name, before, after, step, touched) in steps {
        assert!(step == Change::between(after, before).expect("the step back"), "{name}: the step between the two lands");
        assert_eq!(step.touched(), touched, "{name}");
        let (mut swapped, mut swapping) = (after.clone(), step.clone());
        swapping.swap(&mut swapped);
        assert!(swapped.to_bytes() == before.to_bytes(), "{name}: swapped into the land after, the land before byte for byte");
    }
    let own = Change::to_parts(land, Some(&land.marks.held), Some(&land.asked), Some(land.reach_m));
    assert!(own.is_empty() && own.touched() == Touched::default(), "the land's own parts");
    let edit = land.edit(&edit_of(land, "a"), &never()).expect("edit a");
    let touched = Change::between(land, &edit.land).expect("the step of edit a").touched();
    assert_eq!(touched, Touched { world: true, drawings: true, rest: true, ..Touched::default() }, "edit a");
}

/// A change that makes a land of another shape than the two a step lies between, by its name.
type Reshape = (&'static str, fn(&mut Land));

/// The shapes `a_step_swapped_into_a_land_of_another_shape_panics_and_changes_nothing` swaps into.
const OTHER_SHAPES: [Reshape; 7] = [
    ("held marks shorter", |land| {
        let cells = land.marks.held.len();
        land.marks.held.truncate(cells / 2);
    }),
    ("hand marks longer", |land| land.marks.by_hand.push(false)),
    ("a layer more", |land| {
        let flow = land.world.layers["flow"].clone();
        land.world.layers.insert("future".to_string(), flow);
    }),
    ("a layer renamed", |land| {
        let flow = land.world.layers.remove("flow").expect("the flow layer");
        land.world.layers.insert("flux".to_string(), flow);
    }),
    ("the last layer shorter", |land| {
        land.world.layers.values_mut().last().expect("a layer").data.pop();
    }),
    ("the heights shorter", |land| {
        land.world.heights.data.pop();
    }),
    ("the ground longer", |land| {
        let cell = land.world.ground.data[0];
        land.world.ground.data.push(cell);
    }),
];

/// Stage 2's ruling R3 (S2): a step swapped into a land of another shape than the two it
/// lies between panics before it exchanges anything (`Change::swap` checks the shape first), the
/// land's file and the step as they were afterwards: edit `a`'s step, held by runs, against lands
/// with their held marks shorter, their hand marks longer, a layer more, a layer renamed, the last
/// layer shorter, the heights shorter and the ground longer.
#[test]
fn a_step_swapped_into_a_land_of_another_shape_panics_and_changes_nothing() {
    let land = small_example();
    let edit = land.edit(&edit_of(land, "a"), &never()).expect("edit a");
    let step = Change::between(land, &edit.land).expect("the step of edit a");
    for (name, reshape) in OTHER_SHAPES {
        let mut shaped = land.clone();
        reshape(&mut shaped);
        let before = shaped.to_bytes();
        let mut held = step.clone();
        let swapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| held.swap(&mut shaped)));
        assert!(swapped.is_err(), "{name}: the swap panics");
        assert!(shaped.to_bytes() == before, "{name}: the land's file as it was");
        assert!(held == step, "{name}: the step as it was");
    }
}

/// Stage 2's Task 2: the step between a land and itself, or a copy of it, is empty and changes
/// nothing; lands of 512 m and 1,024 m, either way round, are refused, and so are two lands whose
/// worlds hold other layers.
#[test]
fn a_land_and_itself_have_an_empty_step_and_lands_of_two_sizes_none() {
    let land = small_example();
    let copy = land.clone();
    for (name, other) in [("itself", land), ("a copy", &copy)] {
        let mut step = Change::between(land, other).expect(name);
        assert!(step.is_empty(), "{name}: empty");
        let mut swapped = land.clone();
        step.swap(&mut swapped);
        assert!(swapped.to_bytes() == land.to_bytes(), "{name}: an empty step changes nothing");
    }
    let larger = Land::grow(&Drawing::example_valley(1024), SEED, &GenConfig::default(), &never()).expect("the example grows at 1,024 m");
    for (a, b) in [(land, &larger), (&larger, land)] {
        let refused = Change::between(a, b).expect_err("two sizes");
        assert!(matches!(&refused, LandformError::BadConfig(reason) if reason.contains("512 m") && reason.contains("1024 m")), "{refused:?}");
    }
    let mut renamed = land.clone();
    let name = renamed.world.layers.keys().next().expect("a layer").clone();
    let layer = renamed.world.layers.remove(&name).expect("the layer");
    renamed.world.layers.insert(format!("{name}_other"), layer);
    let refused = Change::between(land, &renamed).expect_err("other layers");
    assert!(matches!(&refused, LandformError::BadConfig(reason) if reason.contains("layers")), "{refused:?}");
}
