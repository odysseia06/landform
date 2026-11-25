//! Nine whole drawn worlds, recorded on Landform `d4733fc` (`master` before the Editor Tool's
//! first stage): they must not change. `generate_drawn` gives bit-identical heights, ground map,
//! layers and water for the same drawing, seed and config, on one thread and on all the machine
//! has. This file is left untouched unless a world is meant to change; a failure prints the
//! fingerprint found, in the form of its record.

use landform_core::{generate_drawn, trial_drawing, Country, CourseEnd, Drawing, GenConfig, World};
use std::sync::atomic::AtomicBool;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a, fed little-endian bytes.
struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(FNV_PRIME);
        }
    }

    fn f32(&mut self, value: f32) {
        self.bytes(&value.to_bits().to_le_bytes());
    }

    fn f64(&mut self, value: f64) {
        self.bytes(&value.to_bits().to_le_bytes());
    }

    /// A count or an index, as 8 bytes.
    fn index(&mut self, value: usize) {
        self.bytes(&(value as u64).to_le_bytes());
    }

    /// 0 for none; 1 and the index for one.
    fn option(&mut self, value: Option<usize>) {
        match value {
            None => self.bytes(&[0]),
            Some(index) => {
                self.bytes(&[1]);
                self.index(index);
            }
        }
    }

    /// Its length, then its bytes.
    fn text(&mut self, text: &str) {
        self.index(text.len());
        self.bytes(text.as_bytes());
    }
}

/// FNV-1a over everything a drawn world gives but its notes and its timings, in this order:
/// every height's bits; every ground cell's two materials and blend; every layer in the order of
/// its name, its name and then each value's bits; every course in the world's order, each point's
/// place, surface, width and depth, then the course's `drawn`, `from_lake` and end; every lake's
/// level, place, area and `drawn`. Then the rest of the world: the size and cell size of the
/// heights, the ground map and each layer, in that order, and the meta (seed, source, window, base
/// elevation and north). The layers, the courses, each course's points and the lakes are preceded
/// by their count, and a name by its length. The timings are left out, since they vary from run to
/// run, and so are the notes: what they say may change without the world changing.
fn fingerprint(world: &World) -> u64 {
    let mut hash = Fnv(FNV_OFFSET);
    for &height in &world.heights.data {
        hash.f32(height);
    }
    for cell in &world.ground.data {
        hash.bytes(&[cell.strongest, cell.second, cell.blend]);
    }
    // `layers` is a `BTreeMap`, so this runs in name order.
    hash.index(world.layers.len());
    for (name, layer) in &world.layers {
        hash.text(name);
        for &value in &layer.data {
            hash.f32(value);
        }
    }
    hash.index(world.water.rivers.len());
    for course in &world.water.rivers {
        hash.index(course.points.len());
        for point in &course.points {
            for value in [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m] {
                hash.f32(value);
            }
        }
        hash.option(course.drawn);
        hash.option(course.from_lake);
        match course.end {
            CourseEnd::Edge => hash.bytes(&[0]),
            CourseEnd::River(index) => {
                hash.bytes(&[1]);
                hash.index(index);
            }
            CourseEnd::Lake(index) => {
                hash.bytes(&[2]);
                hash.index(index);
            }
        }
    }
    hash.index(world.water.lakes.len());
    for lake in &world.water.lakes {
        for value in [lake.level_m, lake.x_m, lake.z_m, lake.area_m2] {
            hash.f32(value);
        }
        hash.index(lake.drawn);
    }

    let sizes = [(world.heights.width, world.heights.height, world.heights.cell_size)]
        .into_iter()
        .chain([(world.ground.width, world.ground.height, world.ground.cell_size)])
        .chain(world.layers.values().map(|layer| (layer.width, layer.height, layer.cell_size)));
    for (width, height, cell_size) in sizes {
        hash.index(width);
        hash.index(height);
        hash.f32(cell_size);
    }
    let meta = &world.meta;
    hash.bytes(&meta.seed.to_le_bytes());
    hash.text(&meta.source_id);
    let window = &meta.window;
    hash.index(window.source);
    for value in [window.center_x, window.center_y, window.rotation_rad] {
        hash.f64(value);
    }
    hash.bytes(&[u8::from(window.mirrored)]);
    hash.f64(window.size_m);
    hash.f32(meta.base_elevation_m);
    hash.f32(meta.north.0);
    hash.f32(meta.north.1);
    hash.0
}

/// A recorded world: its drawing (`valley` for `Drawing::example_valley`, or a trial drawing's
/// name), its size in metres, its kind of country, its seed and its fingerprint on `d4733fc`.
type Record = (&'static str, u32, &'static str, u64, u64);

/// The example in its three kinds of country, from seed 11.
const VALLEY_2048_GENTLE_11: Record = ("valley", 2048, "gentle", 11, 0xd3b6_c062_70bc_0f51);
const VALLEY_2048_HILLS_11: Record = ("valley", 2048, "hills", 11, 0x32d4_3b6d_2bf0_1dcd);
const VALLEY_2048_MOUNTAINS_11: Record = ("valley", 2048, "mountains", 11, 0x79e4_4495_5904_f32d);
/// The three trial drawings that become drawing folders, in hill country from seed 3.
const HEADJOIN_1024_HILLS_3: Record = ("headjoin", 1024, "hills", 3, 0xbcd4_0e59_822c_eecf);
const TWOOUT_1024_HILLS_3: Record = ("twoout", 1024, "hills", 3, 0x4192_4019_d18c_19ca);
const WIDE_1024_HILLS_3: Record = ("wide", 1024, "hills", 3, 0x8fb5_d9ce_57f1_cb42);
/// The example in hill country from two more seeds, and at half the size.
const VALLEY_2048_HILLS_23: Record = ("valley", 2048, "hills", 23, 0xb2de_8d8f_7c04_3e43);
const VALLEY_2048_HILLS_42: Record = ("valley", 2048, "hills", 42, 0xf956_913d_a075_63c8);
const VALLEY_1024_HILLS_11: Record = ("valley", 1024, "hills", 11, 0xea32_dfd7_bf39_5f61);

/// The drawing a record names, in its kind of country.
fn drawing(name: &str, size_m: u32, country: &str) -> Drawing {
    let country = Country::named(country).expect("a kind of country");
    match name {
        "valley" => Drawing { country, ..Drawing::example_valley(size_m) },
        _ => trial_drawing(name, size_m, country).expect("a trial drawing"),
    }
}

/// The fingerprint of the world `generate_drawn` grows from `drawing` and `seed` under the default
/// config, on a pool of `threads` threads.
fn grown_on(threads: usize, drawing: &Drawing, seed: u64) -> u64 {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    let world = pool.install(|| generate_drawn(seed, drawing, &GenConfig::default(), &AtomicBool::new(false)));
    fingerprint(&world.expect("the drawing gives a world"))
}

/// The world a record names gives its recorded fingerprint on one thread and on all the machine has.
fn holds((name, size_m, country, seed, recorded): Record) {
    let drawing = drawing(name, size_m, country);
    let all = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let found: Vec<(usize, u64)> = [1, all].into_iter().map(|threads| (threads, grown_on(threads, &drawing, seed))).collect();
    let wrong: Vec<String> = (found.iter())
        .filter(|&&(_, value)| value != recorded)
        .map(|(threads, value)| format!("on a pool of {threads} (\"{name}\", {size_m}, \"{country}\", {seed}, {value:#018x})"))
        .collect();
    assert!(
        wrong.is_empty(),
        "{name} at {size_m} m, {country}, seed {seed} changed: recorded {recorded:#018x}, found {}",
        wrong.join(", ")
    );
}

/// Flips the lowest bit of `value`.
fn nudge(value: &mut f32) {
    *value = f32::from_bits(value.to_bits() ^ 1);
}

/// One more than `index`, or 0 for none: never what it was.
fn other(index: Option<usize>) -> Option<usize> {
    Some(index.map_or(0, |index| index + 1))
}

/// A change to one thing a world gives, and its name.
type Change = (&'static str, fn(&mut World));

/// Each changes one value of a part the fingerprint reads; the last two change only what it leaves out.
const CHANGES: [Change; 32] = [
    ("a height", |world| nudge(&mut world.heights.data[0])),
    ("a ground cell's strongest material", |world| world.ground.data[0].strongest ^= 1),
    ("a ground cell's second material", |world| world.ground.data[0].second ^= 1),
    ("a ground cell's blend", |world| world.ground.data[0].blend ^= 1),
    ("the first layer", |world| nudge(&mut world.layers.values_mut().next().unwrap().data[0])),
    ("the last layer", |world| nudge(&mut world.layers.values_mut().last().unwrap().data[0])),
    ("a layer's name", |world| {
        let (name, layer) = world.layers.pop_first().unwrap();
        world.layers.insert(format!("{name}_"), layer);
    }),
    ("a course point's x", |world| nudge(&mut world.water.rivers[0].points[0].x_m)),
    ("a course point's z", |world| nudge(&mut world.water.rivers[0].points[0].z_m)),
    ("a course point's surface", |world| nudge(&mut world.water.rivers[0].points[0].surface_m)),
    ("a course point's width", |world| nudge(&mut world.water.rivers[0].points[0].width_m)),
    ("a course point's depth", |world| nudge(&mut world.water.rivers[0].points[0].depth_m)),
    ("a course's last point", |world| nudge(&mut world.water.rivers.last_mut().unwrap().points.last_mut().unwrap().depth_m)),
    ("a course's drawn river", |world| world.water.rivers[0].drawn = other(world.water.rivers[0].drawn)),
    ("a course's lake", |world| world.water.rivers[0].from_lake = other(world.water.rivers[0].from_lake)),
    ("a course's end", |world| {
        let course = &mut world.water.rivers[0];
        course.end = match course.end {
            CourseEnd::Edge => CourseEnd::River(0),
            CourseEnd::River(index) => CourseEnd::River(index + 1),
            CourseEnd::Lake(index) => CourseEnd::Lake(index + 1),
        };
    }),
    ("a lake's level", |world| nudge(&mut world.water.lakes[0].level_m)),
    ("a lake's x", |world| nudge(&mut world.water.lakes[0].x_m)),
    ("a lake's z", |world| nudge(&mut world.water.lakes[0].z_m)),
    ("a lake's area", |world| nudge(&mut world.water.lakes[0].area_m2)),
    ("a lake's drawn lake", |world| world.water.lakes[0].drawn += 1),
    ("the heights' cell size", |world| nudge(&mut world.heights.cell_size)),
    ("the seed", |world| world.meta.seed += 1),
    ("the source", |world| world.meta.source_id.push('_')),
    ("the window's source", |world| world.meta.window.source += 1),
    ("the window's centre", |world| world.meta.window.center_x += 1.0),
    ("the window's rotation", |world| world.meta.window.rotation_rad += 1.0),
    ("the window's mirroring", |world| world.meta.window.mirrored = !world.meta.window.mirrored),
    ("the base elevation", |world| nudge(&mut world.meta.base_elevation_m)),
    ("north", |world| nudge(&mut world.meta.north.1)),
    ("the notes", |world| world.notes.push("A note.".to_string())),
    ("the timings", |world| world.meta.timings.total_ms += 1.0),
];

/// The fingerprint moves when any one value it reads moves, and stays when the notes or the
/// timings change, on the example at 512 m, whose water has rivers, brooks and a lake.
#[test]
fn the_fingerprint_reads_every_part_but_the_notes_and_timings() {
    let world = generate_drawn(1, &drawing("valley", 512, "hills"), &GenConfig::default(), &AtomicBool::new(false))
        .expect("the drawing gives a world");
    assert!(world.water.rivers.len() > 1 && !world.water.lakes.is_empty(), "the example at 512 m has no water to change");
    let original = fingerprint(&world);
    let (read, left_out) = CHANGES.split_at(CHANGES.len() - 2);
    for (what, change) in read {
        let mut changed = world.clone();
        change(&mut changed);
        assert_ne!(fingerprint(&changed), original, "changing {what} leaves the fingerprint as it was");
    }
    for (what, change) in left_out {
        let mut changed = world.clone();
        change(&mut changed);
        assert_eq!(fingerprint(&changed), original, "changing {what} moves the fingerprint");
    }
}

#[test]
fn the_example_in_gentle_country_is_unchanged() {
    holds(VALLEY_2048_GENTLE_11);
}

#[test]
fn the_example_in_hill_country_is_unchanged() {
    holds(VALLEY_2048_HILLS_11);
}

#[test]
fn the_example_in_mountains_is_unchanged() {
    holds(VALLEY_2048_MOUNTAINS_11);
}

#[test]
fn the_trial_headjoin_is_unchanged() {
    holds(HEADJOIN_1024_HILLS_3);
}

#[test]
fn the_trial_twoout_is_unchanged() {
    holds(TWOOUT_1024_HILLS_3);
}

#[test]
fn the_trial_wide_is_unchanged() {
    holds(WIDE_1024_HILLS_3);
}

#[test]
fn the_example_from_seed_23_is_unchanged() {
    holds(VALLEY_2048_HILLS_23);
}

#[test]
fn the_example_from_seed_42_is_unchanged() {
    holds(VALLEY_2048_HILLS_42);
}

#[test]
fn the_example_at_1024_m_is_unchanged() {
    holds(VALLEY_1024_HILLS_11);
}
