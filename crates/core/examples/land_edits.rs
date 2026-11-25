//! Counts what an edit does on the editor tool's three sets of edits (the plan's Task 10), through
//! the core's public interface only: each run grows a land (`Land::grow`), edits it (`Land::edit`)
//! and grows the edited drawing whole (`Land::grow`), then measures the band's ground of the three
//! (`band_ground`, `beyond_span`), the water the edited land leaves that the ground does not hold
//! and that neither the kept land nor the land grown whole has at the same place (`new_unheld`),
//! the edited land's courses, and what the edit says it did (`Edit`, its `WaterJoin` and its
//! `EditTimings`).
//!
//! `cargo run --release -p landform_core --example land_edits -- example|kinds|trials [--out <dir>]`
//!
//! - `example`: the example at 2,048 m in hills from seed 11, each of its nine edits (`EDIT_NAMES`,
//!   `example_edit`) and the drawing unchanged;
//! - `kinds`: edits `a`, `b` and `c` of the example at 2,048 m in gentle, hills and mountains from
//!   seeds 11, 23 and 42 (27 runs);
//! - `trials`: the 17 trial drawings (`TRIAL_NAMES`) at 1,024 m in hills from seed 3, each with its
//!   standard edit (`standard_edit`).
//!
//! One tab-separated line a run, its name first (`example-a`, `kinds-b-mountains-23`,
//! `trials-shoulder`), then `key=value` fields (`print_run`), and a last line `TOTAL` with the set's
//! totals (`print_totals`). With `--out <dir>` (created when missing) each run also writes into
//! `<dir>` the kept land, the edited land and the edited drawing grown whole as land files
//! (`<run>-before.lfland`, `<run>-edited.lfland`, `<run>-whole.lfland`; `landform land export` turns
//! each into pictures) and the edit's outline weight as `<run>-weight.r32` (float32 little-endian,
//! one value a world cell, row by row from the north-west, as `heights.r32` is), so a run's files
//! sort together and a set's runs after one another.
//!
//! `cargo run --release -p landform_core --example land_edits -- valley <size_m>` grows the example
//! `size_m` across in hills from seed 11 and makes its edit `a` once, and prints how long the growing
//! took and the edit's area and `EditTimings`: nothing else is held, so the process's peak memory,
//! measured from outside, is a land's and its edit's.

use landform_core::{
    band_ground, beyond_span, example_edit, new_unheld, standard_edit, trial_drawing, BandGround, Country, Drawing, Edit, EditTimings, GenConfig, Grid,
    Land, LandformError, NewUnheld, COUNTRY_NAMES, EDIT_NAMES, TROUGH_SAID_M, TRIAL_NAMES,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// The example's size and seed, in the `example` and `kinds` sets and in `valley`.
const EXAMPLE_SIZE_M: u32 = 2048;
const EXAMPLE_SEED: u64 = 11;
/// The seeds of the `kinds` set, and the example's edits it makes in each kind of country.
const KIND_SEEDS: [u64; 3] = [11, 23, 42];
const KIND_EDITS: [&str; 3] = ["a", "b", "c"];
/// The trial drawings' size and seed.
const TRIAL_SIZE_M: u32 = 1024;
const TRIAL_SEED: u64 = 3;
/// The edit `valley` makes: the brook's point moved 30 m.
const VALLEY_EDIT: &str = "a";
/// Square metres in a hectare.
const M2_PER_HA: f64 = 10_000.0;
/// A share as a percentage.
const PER_CENT: f64 = 100.0;

/// One run of a set: a land grown from `drawing` and `seed`, edited to `edited`.
struct Run {
    name: String,
    drawing: Drawing,
    seed: u64,
    edited: Drawing,
}

/// The runs of `set`, or `None` for a name that is not a set.
fn runs_of(set: &str) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    match set {
        "example" => {
            let drawing = Drawing::example_valley(EXAMPLE_SIZE_M);
            for name in EDIT_NAMES {
                let edited = example_edit(&drawing, name).expect("an edit of the example");
                runs.push(Run { name: format!("example-{name}"), drawing: drawing.clone(), seed: EXAMPLE_SEED, edited });
            }
            runs.push(Run { name: "example-unchanged".to_string(), drawing: drawing.clone(), seed: EXAMPLE_SEED, edited: drawing });
        }
        "kinds" => {
            for kind in COUNTRY_NAMES {
                let drawing = Drawing { country: Country::named(kind).expect("a kind of country"), ..Drawing::example_valley(EXAMPLE_SIZE_M) };
                for seed in KIND_SEEDS {
                    for name in KIND_EDITS {
                        let edited = example_edit(&drawing, name).expect("an edit of the example");
                        runs.push(Run { name: format!("kinds-{name}-{kind}-{seed}"), drawing: drawing.clone(), seed, edited });
                    }
                }
            }
        }
        "trials" => {
            for name in TRIAL_NAMES {
                let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::hills()).expect("a trial drawing");
                let edited = standard_edit(&drawing).expect("its standard edit");
                runs.push(Run { name: format!("trials-{name}"), drawing, seed: TRIAL_SEED, edited });
            }
        }
        _ => return None,
    }
    Some(runs)
}

/// What one run counts.
struct Counted {
    /// The outline's area inside the world, in hectares.
    inside_ha: f64,
    /// The band's ground of the edited land, the kept land and the edited drawing grown whole, each
    /// under the edit's weight; and how far the edited land's bend and slope lie beyond the span of
    /// the other two, as a share of their mean.
    band: [BandGround; 3],
    bend_beyond: f64,
    slope_beyond: f64,
    new: NewUnheld,
    /// The most any course of the edited land rises from one point to the next going down it,
    /// measured here over every course; the most a joined course rose before the mending, and the
    /// most the mending lowered and raised a point.
    rise_m: f32,
    rise_before_m: f32,
    lowered_m: f32,
    raised_m: f32,
    /// The troughs cut and the pits filled, the deepest trough cut and where, the troughs cut deeper
    /// than `TROUGH_SAID_M`, and the pits left.
    cut: usize,
    filled: usize,
    deepest: Option<(f32, (f64, f64))>,
    deep: usize,
    pits_left: usize,
    /// The join's counts: brooks crossing (kept, new) and paired, the places where a course with no
    /// partner was narrowed and ended, courses crossing and wholly inside, and the cells of weight 1
    /// capped at a mended course with the most one was lowered.
    brooks: (usize, usize),
    paired: usize,
    narrowed: usize,
    ended: usize,
    crossings: usize,
    joined_inside: usize,
    capped: usize,
    capped_most_m: f32,
    notes: usize,
    timings: EditTimings,
}

fn count(before: &Land, edit: &Edit, whole: &Land) -> Counted {
    let weight = &edit.outline.weight;
    let band = [band_ground(&edit.land.world, weight), band_ground(&before.world, weight), band_ground(&whole.world, weight)];
    let rise_m = (edit.land.world.water.rivers.iter())
        .flat_map(|course| course.points.windows(2).map(|pair| pair[1].surface_m - pair[0].surface_m))
        .fold(0.0, f32::max);
    let cut = edit.troughs.iter().filter(|trough| !trough.filled);
    let deepest = cut.clone().max_by(|a, b| a.depth_m.total_cmp(&b.depth_m)).map(|trough| (trough.depth_m, trough.at));
    let water = &edit.water;
    Counted {
        inside_ha: edit.outline.inside_m2 / M2_PER_HA,
        bend_beyond: beyond_span(band[0].bend, band[1].bend, band[2].bend),
        slope_beyond: beyond_span(band[0].slope, band[1].slope, band[2].slope),
        band,
        new: new_unheld(&edit.land.world, &before.world, &whole.world, weight).expect("lands hold the water layer"),
        rise_m,
        rise_before_m: water.joins.iter().map(|join| join.rise_before_m).fold(0.0, f32::max),
        lowered_m: water.lowered_most_m,
        raised_m: water.raised_most_m,
        cut: cut.clone().count(),
        filled: edit.troughs.iter().filter(|trough| trough.filled).count(),
        deepest,
        deep: cut.filter(|trough| trough.depth_m > TROUGH_SAID_M).count(),
        pits_left: edit.pits_left,
        brooks: water.brooks_crossing,
        paired: water.brooks_paired,
        narrowed: water.brooks_narrowed.len(),
        ended: water.brooks_ended.len(),
        crossings: water.crossings,
        joined_inside: water.joined_inside,
        capped: water.capped_at_weight_1,
        capped_most_m: water.capped_most_m,
        notes: edit.notes.len(),
        timings: edit.timings,
    }
}

/// A band's ground as `bend=` and `slope=` give it: the edited land's, the kept land's and the
/// whole's, then the share beyond the span in per cent.
fn ground_field(values: [f64; 3], beyond: f64) -> String {
    format!("{:.5}:{:.5}:{:.5}:{:.3}%", values[0], values[1], values[2], beyond * PER_CENT)
}

/// One run's line: `outline_ha=` (the outline's area inside the world), `band_cells=`, `bend=` and
/// `slope=` (`ground_field`), `new_unheld=` (outside : band : inside), `dry=`, `lake=` and `steps=`
/// (each kind outside : band : inside), `rise=` (the most any course of the edited land rises),
/// `rise_before=`, `lowered=` and `raised=` (the join's mending), `troughs=` (cut : filled),
/// `deepest=` (the deepest cut, metres at world `(x,z)`, or `-`), `deep=` (cut deeper than
/// `TROUGH_SAID_M`), `pits_left=`, `brooks=` (kept : new : paired), `narrowed=` and `ended=` (places),
/// `crossings=` (crossing + wholly inside), `capped=` (cells of weight 1 : the most lowered),
/// `notes=` (the edit's), then each of `EditTimings` in milliseconds.
fn print_run(name: &str, c: &Counted) {
    let t = &c.timings;
    let kinds = |counts: [usize; 3]| format!("{}:{}:{}", counts[0], counts[1], counts[2]);
    let deepest = c.deepest.map_or("-".to_string(), |(depth, (x, z))| format!("{depth:.2}@({x:.0},{z:.0})"));
    println!(
        "{name}\toutline_ha={:.2}\tband_cells={}\tbend={}\tslope={}\tnew_unheld={}:{}:{}\tdry={}\tlake={}\tsteps={}\trise={:.5}\trise_before={:.4}\tlowered={:.4}\traised={:.4}\t\
         troughs={}:{}\tdeepest={deepest}\tdeep={}\tpits_left={}\tbrooks={}:{}:{}\tnarrowed={}\tended={}\tcrossings={}+{}\tcapped={}:{:.4}\tnotes={}\t\
         footprint_ms={:.1}\tgrow_ms={:.1}\toutline_ms={:.1}\tmix_ms={:.1}\tfine_steps_ms={:.1}\tjoin_ms={:.1}\ttotal_ms={:.1}",
        c.inside_ha,
        c.band[0].cells,
        ground_field(c.band.map(|band| band.bend), c.bend_beyond),
        ground_field(c.band.map(|band| band.slope), c.slope_beyond),
        c.new.outside(),
        c.new.band(),
        c.new.inside(),
        kinds(c.new.dry),
        kinds(c.new.lake_over_water),
        kinds(c.new.steps),
        c.rise_m,
        c.rise_before_m,
        c.lowered_m,
        c.raised_m,
        c.cut,
        c.filled,
        c.deep,
        c.pits_left,
        c.brooks.0,
        c.brooks.1,
        c.paired,
        c.narrowed,
        c.ended,
        c.crossings,
        c.joined_inside,
        c.capped,
        c.capped_most_m,
        c.notes,
        t.footprint_ms,
        t.grow_ms,
        t.outline_ms,
        t.mix_ms,
        t.fine_steps_ms,
        t.join_ms,
        t.total_ms,
    );
}

/// The set's last line, `TOTAL`: `set=`, `runs=`, `failed=` (runs refused or failed), then over the
/// runs counted the sums of `new_unheld=`, `dry=`, `lake=` and `steps=` (as each run's), the runs
/// with new unheld cells in the band and inside (`band_runs=`, `inside_runs=`, each `[run count,
/// ...]`), the sums of `brooks=`, `narrowed=`, `ended=` and `crossings=`, `bend_most=` and
/// `slope_most=` (the largest share beyond the span and its run), `outline_ha=` (least to most),
/// `rise_most=`, `rise_before_most=`, `lowered_most=` and `raised_most=`, `troughs=` (cut : filled),
/// `deepest=` (the deepest cut and its run), `deep_runs=` (the runs with a trough cut deeper than
/// `TROUGH_SAID_M`), `pits_left=`, `capped=` (cells : the most, and `capped_runs=`), then each of
/// `EditTimings` least to most.
fn print_totals(set: &str, counted: &[(String, Counted)], failed: usize) {
    let sum = |f: &dyn Fn(&Counted) -> usize| counted.iter().map(|(_, c)| f(c)).sum::<usize>();
    let most = |f: &dyn Fn(&Counted) -> f32| counted.iter().map(|(_, c)| f(c)).fold(0.0f32, f32::max);
    let runs_with = |f: &dyn Fn(&Counted) -> usize| {
        let found: Vec<String> = counted.iter().filter(|(_, c)| f(c) > 0).map(|(name, c)| format!("{name} {}", f(c))).collect();
        format!("[{}]", found.join(", "))
    };
    let most_of = |f: &dyn Fn(&Counted) -> f64| {
        counted.iter().map(|(name, c)| (f(c), name.as_str())).fold((0.0f64, "-"), |best, here| if here.0 > best.0 { here } else { best })
    };
    let span = |f: &dyn Fn(&Counted) -> f64| {
        let values = counted.iter().map(|(_, c)| f(c));
        let least = values.clone().fold(f64::INFINITY, f64::min);
        let most = values.fold(f64::NEG_INFINITY, f64::max);
        format!("{least:.1}..{most:.1}")
    };
    let kinds = |f: &dyn Fn(&Counted) -> [usize; 3]| {
        let summed = (0..3).map(|place| counted.iter().map(|(_, c)| f(c)[place]).sum::<usize>().to_string()).collect::<Vec<_>>();
        summed.join(":")
    };
    let (bend_most, bend_run) = most_of(&|c| c.bend_beyond);
    let (slope_most, slope_run) = most_of(&|c| c.slope_beyond);
    let deepest = counted
        .iter()
        .filter_map(|(name, c)| c.deepest.map(|(depth, _)| (depth, name.as_str())))
        .fold(None, |best: Option<(f32, &str)>, here| if best.is_none_or(|best| here.0 > best.0) { Some(here) } else { best });
    let deepest = deepest.map_or("-".to_string(), |(depth, name)| format!("{depth:.2}@{name}"));
    let timing = |f: &dyn Fn(&EditTimings) -> f32| span(&|c| f64::from(f(&c.timings)));
    println!(
        "TOTAL\tset={set}\truns={}\tfailed={failed}\tnew_unheld={}:{}:{}\tdry={}\tlake={}\tsteps={}\tband_runs={}\tinside_runs={}\tbrooks={}:{}:{}\tnarrowed={}\tended={}\tcrossings={}+{}\t\
         bend_most={:.3}%@{bend_run}\tslope_most={:.3}%@{slope_run}\toutline_ha={}\trise_most={:.5}\trise_before_most={:.4}\tlowered_most={:.4}\traised_most={:.4}\t\
         troughs={}:{}\tdeepest={deepest}\tdeep_runs={}\tpits_left={}\tcapped={}:{:.4}\tcapped_runs={}\t\
         footprint_ms={}\tgrow_ms={}\toutline_ms={}\tmix_ms={}\tfine_steps_ms={}\tjoin_ms={}\ttotal_ms={}",
        counted.len(),
        sum(&|c| c.new.outside()),
        sum(&|c| c.new.band()),
        sum(&|c| c.new.inside()),
        kinds(&|c| c.new.dry),
        kinds(&|c| c.new.lake_over_water),
        kinds(&|c| c.new.steps),
        runs_with(&|c| c.new.band()),
        runs_with(&|c| c.new.inside()),
        sum(&|c| c.brooks.0),
        sum(&|c| c.brooks.1),
        sum(&|c| c.paired),
        sum(&|c| c.narrowed),
        sum(&|c| c.ended),
        sum(&|c| c.crossings),
        sum(&|c| c.joined_inside),
        bend_most * PER_CENT,
        slope_most * PER_CENT,
        span(&|c| c.inside_ha),
        most(&|c| c.rise_m),
        most(&|c| c.rise_before_m),
        most(&|c| c.lowered_m),
        most(&|c| c.raised_m),
        sum(&|c| c.cut),
        sum(&|c| c.filled),
        runs_with(&|c| c.deep),
        sum(&|c| c.pits_left),
        sum(&|c| c.capped),
        most(&|c| c.capped_most_m),
        runs_with(&|c| c.capped),
        timing(&|t| t.footprint_ms),
        timing(&|t| t.grow_ms),
        timing(&|t| t.outline_ms),
        timing(&|t| t.mix_ms),
        timing(&|t| t.fine_steps_ms),
        timing(&|t| t.join_ms),
        timing(&|t| t.total_ms),
    );
}

/// The edit's weight as `<dir>/<run>-weight.r32`: float32 little-endian, one value a world cell in
/// the grid's order, row by row from the north-west.
fn write_weight(path: &Path, weight: &Grid<f32>) -> Result<(), LandformError> {
    let bytes: Vec<u8> = weight.data.iter().flat_map(|value| value.to_le_bytes()).collect();
    fs::write(path, bytes).map_err(|error| LandformError::Io(format!("{}: {error}", path.display())))
}

/// A run's four files in `dir`.
fn write_run(dir: &Path, name: &str, before: &Land, edit: &Edit, whole: &Land) -> Result<(), LandformError> {
    before.write(&dir.join(format!("{name}-before.lfland")))?;
    edit.land.write(&dir.join(format!("{name}-edited.lfland")))?;
    whole.write(&dir.join(format!("{name}-whole.lfland")))?;
    write_weight(&dir.join(format!("{name}-weight.r32")), &edit.outline.weight)
}

fn run_set(set: &str, runs: Vec<Run>, out: Option<&Path>) {
    let (config, cancel) = (GenConfig::default(), AtomicBool::new(false));
    let mut kept: Option<(Drawing, u64, Land)> = None;
    let (mut counted, mut failed) = (Vec::new(), 0);
    for run in runs {
        if kept.as_ref().is_none_or(|(drawing, seed, _)| *drawing != run.drawing || *seed != run.seed) {
            let land = Land::grow(&run.drawing, run.seed, &config, &cancel).expect("the set's drawing grows");
            kept = Some((run.drawing.clone(), run.seed, land));
        }
        let before = &kept.as_ref().expect("a kept land").2;
        let edit = match before.edit(&run.edited, &cancel) {
            Ok(edit) => edit,
            Err(error) => {
                failed += 1;
                println!("{}\tFAILED {error}", run.name);
                continue;
            }
        };
        let whole = Land::grow(&run.edited, run.seed, &config, &cancel).expect("the edited drawing grows");
        let found = count(before, &edit, &whole);
        print_run(&run.name, &found);
        if let Some(dir) = out {
            if let Err(error) = write_run(dir, &run.name, before, &edit, &whole) {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        counted.push((run.name, found));
    }
    print_totals(set, &counted, failed);
}

/// `valley <size_m>`: the example `size_m` across, in hills from seed 11, and its edit `a`, once.
fn valley(size_m: u32) {
    let (config, cancel) = (GenConfig::default(), AtomicBool::new(false));
    let drawing = Drawing::example_valley(size_m);
    let clock = Instant::now();
    let land = match Land::grow(&drawing, EXAMPLE_SEED, &config, &cancel) {
        Ok(land) => land,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let grow_ms = clock.elapsed().as_secs_f64() * 1000.0;
    let edited = example_edit(&drawing, VALLEY_EDIT).expect("an edit of the example");
    let edit = match land.edit(&edited, &cancel) {
        Ok(edit) => edit,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let t = edit.timings;
    println!(
        "valley-{size_m}-{VALLEY_EDIT}\tland_grow_ms={grow_ms:.1}\toutline_ha={:.2}\tfootprint_ms={:.1}\tgrow_ms={:.1}\toutline_ms={:.1}\tmix_ms={:.1}\tfine_steps_ms={:.1}\tjoin_ms={:.1}\ttotal_ms={:.1}",
        edit.outline.inside_m2 / M2_PER_HA,
        t.footprint_ms,
        t.grow_ms,
        t.outline_ms,
        t.mix_ms,
        t.fine_steps_ms,
        t.join_ms,
        t.total_ms,
    );
}

fn usage() -> ! {
    eprintln!("usage: land_edits example|kinds|trials [--out <dir>], or land_edits valley <size_m>");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["valley", size] => valley(size.parse().unwrap_or_else(|_| usage())),
        [set, rest @ ..] => {
            let Some(runs) = runs_of(set) else { usage() };
            let out: Option<PathBuf> = match rest {
                [] => None,
                ["--out", dir] => Some(PathBuf::from(dir)),
                _ => usage(),
            };
            if let Some(dir) = &out {
                if let Err(error) = fs::create_dir_all(dir) {
                    eprintln!("{}: {error}", dir.display());
                    std::process::exit(1);
                }
            }
            run_set(set, runs, out.as_deref());
        }
        [] => usage(),
    }
}
