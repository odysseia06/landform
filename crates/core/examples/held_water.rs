//! Grows a set of drawn worlds and counts the water their ground does not hold (`unheld`).
//!
//! `cargo run --release -p landform_core --example held_water -- <set> [only]`, `<set>` one of:
//!
//! - `drawings`: the 17 trial drawings x 3 kinds of country x seeds 1, 2 and 3, at 1,024 m;
//! - `example`: the example drawing at 2,048 m x 3 kinds of country x seeds 11, 23 and 42;
//! - `brooks`: the example at 1,024 m, seed 1, with `brook_start_m2` 1,000, 5,000 and 20,000 x 3
//!   kinds of country.
//!
//! `only`, when given, is a comma-separated list of pieces of names; only the worlds whose name
//! holds one of them are grown. One line a world, tab-separated: its name (`headjoin-hills-s3-1024`)
//! and then `key=value` fields: `abreast=` (dry cells at lakes : at courses : over 1 m : the worst
//! in metres, at world `(x,z)`), `lakeover=`, `waterstep=`, `mouthstep=`, `worststep=` (metres),
//! `levels=` (the lakes' levels), `notes=`, `water_ms=` and `total_ms=` (the whole generation),
//! `wet=` (water cells), `lake=` (cells at a lake's level), `hash=` and `dhash=` (of the heights and
//! of the water's depths), and `causes=`: the finer kinds of the dry cells counted at 0.3 m, each
//! `kind:cells:over 1 m:worst@(x,z);`. A world that is refused or panics prints `REFUSED` or
//! `PANIC`. The last line holds the set's totals.

use landform_core::{generate_drawn, trial_drawing, unheld, Country, CourseEnd, Drawing, GenConfig, RiverCourse, Unheld, WaterRules, World, COUNTRY_NAMES, TRIAL_NAMES, WATER_LAYER};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// Seeds of the trial drawings.
const DRAWING_SEEDS: [u64; 3] = [1, 2, 3];
/// Side of the trial drawings' worlds, in metres.
const DRAWING_SIZE_M: u32 = 1024;
/// Seeds of the example's worlds.
const EXAMPLE_SEEDS: [u64; 3] = [11, 23, 42];
/// Side of the example's worlds, in metres.
const EXAMPLE_SIZE_M: u32 = 2048;
/// The `brook_start_m2` the brooks set grows its worlds with, in square metres.
const BROOK_STARTS_M2: [f32; 3] = [1_000.0, 5_000.0, 20_000.0];
/// Side of the brooks set's worlds, in metres, and their seed.
const BROOKS_SIZE_M: u32 = 1024;
const BROOKS_SEED: u64 = 1;

/// How far beyond its waterline a course's water counts as near a cell when the causes name what
/// holds a dry cell's water, in metres, and how far a lake's edge is said to lie beside a course.
const CAUSE_REACH_M: f64 = 12.5;
const LAKE_BESIDE_COURSE_M: f64 = 14.0;
/// A dry cell this near the world's border is "at the border", in metres.
const BORDER_M: f64 = 9.0;
/// How near a course's surface must lie to a level for the water to be the course's own, and for it
/// to be level with the course's end, in metres.
const SURFACE_MATCH_M: f64 = 0.02;
/// A course's first metres out of a lake, in metres along it.
const OUT_OF_A_LAKE_M: f64 = 60.0;
/// The limit and the deep limit the causes count at, in metres.
const CAUSE_LIMIT_M: f32 = 0.3;
const CAUSE_DEEP_M: f32 = 1.0;
/// How near a surface must lie to a lake's level to be the lake's, in metres.
const LAKE_LEVEL_WITHIN_M: f64 = 0.002;

/// One world to grow.
struct Case {
    name: String,
    drawing: Drawing,
    seed: u64,
    rules: WaterRules,
}

/// The example drawing `size_m` across in the kind of country `country`.
fn example(size_m: u32, country: &str) -> Drawing {
    Drawing { country: Country::named(country).expect("a known kind of country"), ..Drawing::example_valley(size_m) }
}

fn cases_of(set: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    match set {
        "drawings" => {
            for kind in TRIAL_NAMES {
                for country in COUNTRY_NAMES {
                    for seed in DRAWING_SEEDS {
                        let drawing = trial_drawing(kind, DRAWING_SIZE_M, Country::named(country).expect("a known kind of country")).expect("a trial drawing");
                        cases.push(Case { name: format!("{kind}-{country}-s{seed}-{DRAWING_SIZE_M}"), drawing, seed, rules: WaterRules::default() });
                    }
                }
            }
        }
        "example" => {
            for country in COUNTRY_NAMES {
                for seed in EXAMPLE_SEEDS {
                    cases.push(Case { name: format!("example-{country}-s{seed}-{EXAMPLE_SIZE_M}"), drawing: example(EXAMPLE_SIZE_M, country), seed, rules: WaterRules::default() });
                }
            }
        }
        "brooks" => {
            for start in BROOK_STARTS_M2 {
                for country in COUNTRY_NAMES {
                    let rules = WaterRules { brook_start_m2: start, ..WaterRules::default() };
                    cases.push(Case { name: format!("brooks{start}-{country}-s{BROOKS_SEED}-{BROOKS_SIZE_M}"), drawing: example(BROOKS_SIZE_M, country), seed: BROOKS_SEED, rules });
                }
            }
        }
        _ => {
            eprintln!("usage: held_water drawings|example|brooks [only]");
            std::process::exit(2);
        }
    }
    cases
}

/// FNV-1a over the bit patterns of `values`.
fn hash(values: impl Iterator<Item = u32>) -> u64 {
    values.fold(0xcbf2_9ce4_8422_2325, |hash, value| (hash ^ u64::from(value)).wrapping_mul(0x0100_0000_01b3))
}

/// Where `(x, z)` lies against a course.
struct Against {
    /// Distance to the course's line.
    apart_m: f64,
    /// The surface there, linear between points.
    surface_m: f64,
    /// The half width of the water there, at least 0.707 m.
    half_width_m: f64,
    /// Straight beyond the first point.
    beyond_first: bool,
    /// Straight beyond the last point.
    beyond_last: bool,
    /// Metres along the course to the nearest place on it.
    along_m: f64,
}

fn against(course: &RiverCourse, x: f64, z: f64) -> Against {
    let points = &course.points;
    let half_width = |k: usize| (f64::from(points[k].width_m) / 2.0).max(std::f64::consts::FRAC_1_SQRT_2);
    let mut best = Against {
        apart_m: (f64::from(points[0].x_m) - x).hypot(f64::from(points[0].z_m) - z),
        surface_m: f64::from(points[0].surface_m),
        half_width_m: half_width(0),
        beyond_first: false,
        beyond_last: false,
        along_m: 0.0,
    };
    let mut along = 0.0;
    for k in 1..points.len() {
        let (ax, az, bx, bz) = (f64::from(points[k - 1].x_m), f64::from(points[k - 1].z_m), f64::from(points[k].x_m), f64::from(points[k].z_m));
        let (dx, dz) = (bx - ax, bz - az);
        let squared = dx * dx + dz * dz;
        let length = squared.sqrt();
        if squared > 0.0 {
            let raw = ((x - ax) * dx + (z - az) * dz) / squared;
            let t = raw.clamp(0.0, 1.0);
            let apart_m = (ax + dx * t - x).hypot(az + dz * t - z);
            if apart_m < best.apart_m {
                let surface = |k: usize| f64::from(points[k].surface_m);
                best = Against {
                    apart_m,
                    surface_m: surface(k - 1) + (surface(k) - surface(k - 1)) * t,
                    half_width_m: half_width(k - 1) + (half_width(k) - half_width(k - 1)) * t,
                    beyond_first: k == 1 && raw < 0.0,
                    beyond_last: k + 1 == points.len() && raw > 1.0,
                    along_m: along + length * t,
                };
            }
        }
        along += length;
    }
    best
}

/// Dry cells more than 0.3 m below the water beside them, by what that water is, as
/// `kind:cells:over 1 m:worst@(x,z);` for each kind.
fn causes(world: &World) -> String {
    let size = world.heights.width;
    let half = (size / 2) as f64;
    let heights = &world.heights.data;
    let depth = &world.layers[WATER_LAYER].data;
    let wet = |cell: usize| depth[cell] > 0.0;
    let surface = |cell: usize| heights[cell] + depth[cell];
    // Kind -> (cells, cells over 1 m, worst, where).
    let mut kinds: BTreeMap<String, (usize, usize, f32, usize)> = BTreeMap::new();
    for j in 0..size {
        for i in 0..size {
            let cell = j * size + i;
            if wet(cell) {
                continue;
            }
            let mut lowest: Option<(f32, usize)> = None;
            for nj in j.saturating_sub(1)..=(j + 1).min(size - 1) {
                for ni in i.saturating_sub(1)..=(i + 1).min(size - 1) {
                    let other = nj * size + ni;
                    if other != cell && wet(other) && lowest.is_none_or(|low| surface(other) < low.0) {
                        lowest = Some((surface(other), other));
                    }
                }
            }
            let Some((level, water)) = lowest else { continue };
            let below = level - heights[cell];
            if below <= CAUSE_LIMIT_M {
                continue;
            }
            let (x, z) = ((water % size) as f64 - half, (water / size) as f64 - half);
            let level = f64::from(level);
            let kind = if world.water.lakes.iter().any(|lake| (level - f64::from(lake.level_m)).abs() < LAKE_LEVEL_WITHIN_M) {
                // What lies near this piece of the lake's edge: a course out of a lake, any other course, or none.
                let near = |out: bool| {
                    let mut courses = world.water.rivers.iter().filter(|course| course.from_lake.is_some() == out);
                    courses.any(|course| {
                        let found = against(course, x, z);
                        found.apart_m - found.half_width_m <= LAKE_BESIDE_COURSE_M
                    })
                };
                if near(true) {
                    "lake, beside a course out of it".to_string()
                } else if near(false) {
                    "lake, beside another course".to_string()
                } else {
                    "lake, away from every course".to_string()
                }
            } else {
                // The course whose water this is: the one whose surface at its nearest point is the
                // level, the nearest waterline first; else any course within reach.
                let mut found: Option<(f64, usize, bool)> = None;
                for (index, course) in world.water.rivers.iter().enumerate() {
                    let at = against(course, x, z);
                    if at.apart_m - at.half_width_m > CAUSE_REACH_M {
                        continue;
                    }
                    let matches = (at.surface_m - level).abs() < SURFACE_MATCH_M;
                    let better = found.is_none_or(|f| (matches && !f.2) || (matches == f.2 && at.apart_m - at.half_width_m < f.0));
                    if better {
                        found = Some((at.apart_m - at.half_width_m, index, matches));
                    }
                }
                match found {
                    None => "no course near".to_string(),
                    Some((_, index, matches)) => {
                        let course = &world.water.rivers[index];
                        let at = against(course, x, z);
                        let last = f64::from(course.points[course.points.len() - 1].surface_m);
                        let end = match course.end {
                            CourseEnd::River(_) => "river",
                            CourseEnd::Lake(_) => "lake",
                            CourseEnd::Edge => "edge",
                        };
                        let border = (half - x.abs()).min(half - z.abs()) <= BORDER_M;
                        let place = if at.apart_m <= at.half_width_m { "bed" } else { "spread" };
                        if border {
                            "at the border".to_string()
                        } else if at.beyond_last {
                            format!("beyond the last point (ends on {end})")
                        } else if at.beyond_first {
                            "behind the first point".to_string()
                        } else if end != "edge" && (level - last).abs() < SURFACE_MATCH_M {
                            format!("level with its end on a {end} ({place})")
                        } else if course.from_lake.is_some() && at.along_m < OUT_OF_A_LAKE_M {
                            format!("first 60 m out of a lake ({place})")
                        } else if !matches {
                            format!("carried level, mid-course ({place})")
                        } else {
                            format!("mid-course ({place})")
                        }
                    }
                }
            };
            let entry = kinds.entry(kind).or_insert((0, 0, 0.0, 0));
            entry.0 += 1;
            if below > CAUSE_DEEP_M {
                entry.1 += 1;
            }
            if below > entry.2 {
                (entry.2, entry.3) = (below, cell);
            }
        }
    }
    let mut out = String::new();
    for (kind, (count, deep, worst, cell)) in &kinds {
        out += &format!("{kind}:{count}:{deep}:{worst:.2}@({},{});", (*cell % size) as i64 - (size / 2) as i64, (*cell / size) as i64 - (size / 2) as i64);
    }
    out
}

/// The totals of a set.
#[derive(Default)]
struct Totals {
    worlds: usize,
    refused: usize,
    panicked: usize,
    unheld: Unheld,
    worlds_with_dry_cells: usize,
    water_ms: f32,
    total_ms: u128,
}

impl Totals {
    fn add(&mut self, found: &Unheld, water_ms: f32, total_ms: u128) {
        self.worlds += 1;
        let sum = &mut self.unheld;
        sum.dry_lake += found.dry_lake;
        sum.dry_course += found.dry_course;
        sum.dry_over_1m += found.dry_over_1m;
        if found.worst_dry_m > sum.worst_dry_m {
            (sum.worst_dry_m, sum.worst_dry_at) = (found.worst_dry_m, found.worst_dry_at);
        }
        sum.lake_over_water += found.lake_over_water;
        sum.water_steps += found.water_steps;
        sum.mouth_steps += found.mouth_steps;
        sum.worst_step_m = sum.worst_step_m.max(found.worst_step_m);
        self.worlds_with_dry_cells += usize::from(found.dry() > 0);
        self.water_ms += water_ms;
        self.total_ms += total_ms;
    }
}

/// The line of a world that grew.
fn line_of(name: &str, world: &World, found: &Unheld, total_ms: u128) -> String {
    let depth = &world.layers[WATER_LAYER].data;
    let heights = &world.heights.data;
    let wet = depth.iter().filter(|&&d| d > 0.0).count();
    let lake = (0..depth.len()).filter(|&c| depth[c] > 0.0 && world.water.lakes.iter().any(|lake| (heights[c] + depth[c] - lake.level_m).abs() < LAKE_LEVEL_WITHIN_M as f32)).count();
    let levels: Vec<String> = world.water.lakes.iter().map(|lake| format!("{:.3}", lake.level_m)).collect();
    format!(
        "{name}\tabreast={}:{}:{}:{:.2}@({},{})\tlakeover={}\twaterstep={}\tmouthstep={}\tworststep={:.2}\tlevels={}\tnotes={}\twater_ms={:.0}\ttotal_ms={total_ms}\twet={wet}\tlake={lake}\thash={:016x}\tdhash={:016x}\tcauses={}",
        found.dry_lake,
        found.dry_course,
        found.dry_over_1m,
        found.worst_dry_m,
        found.worst_dry_at.0,
        found.worst_dry_at.1,
        found.lake_over_water,
        found.water_steps,
        found.mouth_steps,
        found.worst_step_m,
        levels.join(","),
        world.notes.join(" | "),
        world.meta.timings.water_ms,
        hash(heights.iter().map(|h| h.to_bits())),
        hash(depth.iter().map(|d| d.to_bits())),
        causes(world),
    )
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(set) = args.get(1) else {
        eprintln!("usage: held_water drawings|example|brooks [only]");
        std::process::exit(2);
    };
    let only: Option<Vec<&str>> = args.get(2).map(|only| only.split(',').collect());
    let mut totals = Totals::default();
    for case in cases_of(set).iter().filter(|case| only.as_ref().is_none_or(|only| only.iter().any(|piece| case.name.contains(piece)))) {
        let config = GenConfig { size_cells: case.drawing.size_m as usize, water: case.rules, ..GenConfig::default() };
        let started = Instant::now();
        let grown = catch_unwind(AssertUnwindSafe(|| generate_drawn(case.seed, &case.drawing, &config, &AtomicBool::new(false))));
        let total_ms = started.elapsed().as_millis();
        match grown {
            Ok(Ok(world)) => match unheld(&world) {
                Some(found) => {
                    totals.add(&found, world.meta.timings.water_ms, total_ms);
                    println!("{}", line_of(&case.name, &world, &found, total_ms));
                }
                None => {
                    totals.refused += 1;
                    println!("{}\tNO WATER LAYER", case.name);
                }
            },
            Ok(Err(error)) => {
                totals.refused += 1;
                println!("{}\tREFUSED {error}", case.name);
            }
            Err(_) => {
                totals.panicked += 1;
                println!("{}\tPANIC", case.name);
            }
        }
    }
    let sum = &totals.unheld;
    println!(
        "TOTAL\tworlds={}\trefused={}\tpanicked={}\tdry={} (lake {}, course {})\tworlds_with_dry_cells={}\tover_1m={}\tworst_dry={:.2}@({},{})\tlakeover={}\twaterstep={}\tmouthstep={}\tworststep={:.2}\twater_ms={:.0}\ttotal_ms={}",
        totals.worlds,
        totals.refused,
        totals.panicked,
        sum.dry(),
        sum.dry_lake,
        sum.dry_course,
        totals.worlds_with_dry_cells,
        sum.dry_over_1m,
        sum.worst_dry_m,
        sum.worst_dry_at.0,
        sum.worst_dry_at.1,
        sum.lake_over_water,
        sum.water_steps,
        sum.mouth_steps,
        sum.worst_step_m,
        totals.water_ms,
        totals.total_ms,
    );
}
