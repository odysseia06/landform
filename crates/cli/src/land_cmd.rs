//! `landform land ...`: lands kept as land files (`.lfland`, the core's `Land::write` and
//! `LandFile::read`), made from a drawing folder, edited for another, asked to wait for another,
//! held, regrown, exported as `generate` writes a world, and described.

use crate::{drawing_file, io_error, print_world, water_line, write_png, write_world};
use clap::builder::PossibleValuesParser;
use clap::Subcommand;
use landform_core::{Country, Drawing, Edit, GenConfig, Land, LandFile, LandformError, WaterRules, COUNTRY_NAMES, REACH_MAX_M, REACH_MIN_M};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// The exit code of an edit or a regrowing the core refuses (`LandformError::Refused`): nothing is
/// written.
pub const REFUSED_EXIT: u8 = 3;

/// A marked cell's grey in `held.png` and `by_hand.png`; an unmarked cell is 0.
const MARKED_GREY: u8 = 255;

/// Square metres in a hectare.
const HECTARE_M2: f64 = 10_000.0;

/// What `landform land` does.
#[derive(Subcommand)]
pub enum LandCommand {
    /// Grow a new land from a drawing folder and write it as a land file.
    New {
        /// A folder holding a drawing (`drawing.toml` and its picture), read as `generate --drawing` reads it.
        #[arg(long)]
        drawing: PathBuf,
        /// The seed the land is grown from.
        #[arg(long)]
        seed: u64,
        /// The land file to write; its folder is created when missing.
        #[arg(long)]
        out: PathBuf,
        /// The kind of country, replacing the drawing's own; its [tune] still applies.
        #[arg(long, value_parser = PossibleValuesParser::new(COUNTRY_NAMES))]
        country: Option<String>,
    },
    /// Change a land for the drawing in a folder, which the land then asks for, and write the edited land; a refused edit writes nothing and exits with code 3.
    Edit {
        /// The land file to edit.
        #[arg(long)]
        land: PathBuf,
        /// A folder holding the edited drawing, of the land's size.
        #[arg(long)]
        drawing: PathBuf,
        /// The land file to write; its folder is created when missing.
        #[arg(long)]
        out: PathBuf,
        /// The kind of country, replacing the drawing's own as `land new --country` does; its [tune]
        /// still applies. A land made with `--country` is edited with the same one, or the edit
        /// regrows it all in the folder's kind.
        #[arg(long, value_parser = PossibleValuesParser::new(COUNTRY_NAMES))]
        country: Option<String>,
    },
    /// Set the drawing a land asks for to the one in a folder, growing nothing, and write the land: unless it is the drawing the land was grown from, the land waits to regrow toward it, which `land edit` with that folder does.
    Ask {
        /// The land file to ask a drawing of.
        #[arg(long)]
        land: PathBuf,
        /// A folder holding the drawing asked for, of the land's size.
        #[arg(long)]
        drawing: PathBuf,
        /// The land file to write; its folder is created when missing.
        #[arg(long)]
        out: PathBuf,
        /// The kind of country, replacing the drawing's own as `land new --country` does; its [tune]
        /// still applies.
        #[arg(long, value_parser = PossibleValuesParser::new(COUNTRY_NAMES))]
        country: Option<String>,
    },
    /// Hold the cells of a disc, so no edit changes them, or lift the hold; a lake is held whole, and its hold is lifted whole.
    Hold {
        /// The land file to hold ground in.
        #[arg(long)]
        land: PathBuf,
        /// The disc, as x,z,radius in world metres (x east, z south, from the centre).
        #[arg(long, value_parser = parse_disc, allow_hyphen_values = true)]
        disc: Disc,
        /// Lift the hold on the disc instead.
        #[arg(long)]
        lift: bool,
        /// The land file to write; its folder is created when missing.
        #[arg(long)]
        out: PathBuf,
    },
    /// Regrow the cells of a disc as the drawing the land was grown from has them, inside the disc (and a lake it reaches, taken whole with its band), its band inside its edge but along the world's border, clearing the hand marks of its cells that are not held; a drawing asked for still waits; a refused regrowing writes nothing and exits with code 3.
    Regrow {
        /// The land file to regrow ground in.
        #[arg(long)]
        land: PathBuf,
        /// The disc, as x,z,radius in world metres (x east, z south, from the centre).
        #[arg(long, value_parser = parse_disc, allow_hyphen_values = true)]
        disc: Disc,
        /// The land file to write; its folder is created when missing.
        #[arg(long)]
        out: PathBuf,
    },
    /// Write a land's world as `generate` writes a world, with held.png and by_hand.png.
    Export {
        /// The land file to export.
        #[arg(long)]
        land: PathBuf,
        /// Directory to write into; created when missing.
        #[arg(long)]
        out: PathBuf,
    },
    /// Print what a land file holds: size, seed, drawing, whether a drawing asked for waits to regrow, water, reach, marks, format and the version that wrote it, and the parts of it this Landform does not know, which a land written again drops.
    Info {
        /// The land file to describe.
        #[arg(long)]
        land: PathBuf,
    },
}

/// A disc of world cells: its centre's `x` and `z` and its radius, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Disc {
    x_m: f64,
    z_m: f64,
    radius_m: f64,
}

/// A disc as `--disc` gives it, `x,z,radius` in metres, each finite and the radius above 0.
fn parse_disc(text: &str) -> Result<Disc, String> {
    let refused = || format!("a disc is x,z,radius in metres, each a finite number and the radius above 0, not \"{text}\"");
    let numbers: Vec<f64> = text.split(',').map(|part| part.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| refused())?;
    match numbers[..] {
        [x_m, z_m, radius_m] if [x_m, z_m, radius_m].iter().all(|value| value.is_finite()) && radius_m > 0.0 => Ok(Disc { x_m, z_m, radius_m }),
        _ => Err(refused()),
    }
}

/// One flag a world cell of a land `size` cells a side, in the world's cell order: the cells whose
/// place, `(column - size / 2, row - size / 2)` in metres as the core places a cell, lies within the
/// disc's radius of its centre.
fn disc_cells(size: usize, disc: Disc) -> Vec<bool> {
    let half = size as f64 / 2.0;
    (0..size * size)
        .map(|cell| ((cell % size) as f64 - half - disc.x_m).hypot((cell / size) as f64 - half - disc.z_m) <= disc.radius_m)
        .collect()
}

/// The world cells of `land` that `marks` marks: a cell beyond the list is not marked.
fn marked(land: &Land, marks: &[bool]) -> usize {
    marks.iter().take(land.world.heights.data.len()).filter(|&&on| on).count()
}

/// `count` of the land's cells, as a share in per cent.
fn share_pct(land: &Land, count: usize) -> f64 {
    count as f64 * 100.0 / land.world.heights.data.len().max(1) as f64
}

/// Writes `land` to `out`, its folder created when missing, and says so.
fn write_land(land: &Land, out: &Path) -> Result<(), LandformError> {
    let said = write_land_saying(land, out)?;
    println!("{said}");
    Ok(())
}

/// Writes `land` to `out`, its folder created when missing, and gives what says so, for the caller
/// to print once the write is done.
fn write_land_saying(land: &Land, out: &Path) -> Result<String, LandformError> {
    if let Some(folder) = out.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        std::fs::create_dir_all(folder).map_err(|e| io_error(folder, e))?;
    }
    land.write(out)?;
    let bytes = std::fs::metadata(out).map_err(|e| io_error(out, e))?.len();
    Ok(format!("Wrote a {} m land to {}, {bytes} bytes", land.drawing.size_m, out.display()))
}

/// The drawing in `folder` and its water rules, read as `generate --drawing` reads it, its kind of
/// country replaced by `country` when one is given.
fn read_folder(folder: &Path, country: Option<&str>) -> Result<(Drawing, WaterRules), LandformError> {
    match country {
        None => drawing_file::read_drawing(folder),
        Some(kind) => drawing_file::read_drawing_as(folder, kind),
    }
}

/// What an edit or a regrowing says: the outline's area and the part of it held or shaped by hand,
/// the edit's timings and its notes.
fn print_edit(edit: &Edit) {
    let outline = &edit.outline;
    println!(
        "Outline: {:.2} ha inside, {:.2} ha of it held or shaped by hand",
        outline.inside_m2 / HECTARE_M2,
        outline.fixed_inside_m2 / HECTARE_M2
    );
    let t = &edit.timings;
    println!(
        "Timings: footprint {:.1} ms, grow {:.1} ms, outline {:.1} ms, mix {:.1} ms, fine steps {:.1} ms, join {:.1} ms, total {:.1} ms",
        t.footprint_ms, t.grow_ms, t.outline_ms, t.mix_ms, t.fine_steps_ms, t.join_ms, t.total_ms
    );
    for note in &edit.notes {
        println!("Note: {note}");
    }
}

/// Whether the land waits to regrow (`Land::waiting`), as `land info` and `land ask` say it.
fn waiting_line(land: &Land) -> &'static str {
    if land.waiting() {
        "Waiting:   yes, the asked drawing differs from the one the land was grown from"
    } else {
        "Waiting:   no"
    }
}

/// `BadConfig` unless the drawing folder's water rules are the land's: an edit keeps the land's
/// settings, and a drawing asked for is regrown under them, so a folder whose `[water]` asks for
/// others is refused rather than read in part.
fn check_water(folder: &Path, land: &Land, water: &WaterRules) -> Result<(), LandformError> {
    let differs = drawing_file::water_fields(water).into_iter().zip(drawing_file::water_fields(&land.config.water)).find(|(found, kept)| found != kept);
    match differs {
        None => Ok(()),
        Some(((name, found), (_, kept))) => Err(LandformError::BadConfig(format!(
            "the drawing in {} asks for the water rule {name} = {found}, and the land keeps {kept}; an edit keeps the land's water rules",
            folder.display()
        ))),
    }
}

/// Runs `landform land`'s command. A refused edit or regrowing is `LandformError::Refused`, which the
/// caller turns into `REFUSED_EXIT`; nothing is written then.
pub fn run(command: LandCommand) -> Result<(), LandformError> {
    // Nothing sets this flag: the command line cannot cancel.
    let never = AtomicBool::new(false);
    match command {
        LandCommand::New { drawing, seed, out, country } => {
            let (sketch, water) = read_folder(&drawing, country.as_deref())?;
            let clock = Instant::now();
            let land = Land::grow(&sketch, seed, &GenConfig { water, ..GenConfig::default() }, &never)?;
            println!(
                "Grew a {} m land from seed {seed} in {:.1} ms, the fine steps {:.1} ms of it",
                land.drawing.size_m,
                clock.elapsed().as_secs_f64() * 1000.0,
                land.world.meta.timings.total_ms
            );
            if let Some(line) = water_line(&land.world.water) {
                println!("{line}");
            }
            for note in &land.world.notes {
                println!("Note: {note}");
            }
            write_land(&land, &out)
        }
        LandCommand::Edit { land, drawing, out, country } => {
            let kept = Land::read(&land)?;
            let (edited, water) = read_folder(&drawing, country.as_deref())?;
            check_water(&drawing, &kept, &water)?;
            let edit = kept.edit(&edited, &never)?;
            print_edit(&edit);
            write_land(&edit.land, &out)
        }
        LandCommand::Ask { land, drawing, out, country } => {
            let mut kept = Land::read(&land)?;
            let (asked, water) = read_folder(&drawing, country.as_deref())?;
            // The drawing asked for is regrown under the land's water rules, as an edit is.
            check_water(&drawing, &kept, &water)?;
            kept.ask(asked)?;
            // Said once the land is written, so a write that fails says nothing of what waits.
            let written = write_land_saying(&kept, &out)?;
            println!("{}", waiting_line(&kept));
            println!("{written}");
            Ok(())
        }
        LandCommand::Hold { land, disc, lift, out } => {
            let mut kept = Land::read(&land)?;
            let size = kept.drawing.size_m as usize;
            let area = disc_cells(size, disc);
            let before = kept.marks.held.clone();
            kept.hold(&area, !lift);
            let (was, is) = (|cell: usize| before.get(cell) == Some(&true), |cell: usize| kept.marks.held.get(cell) == Some(&true));
            let changed_beyond = (0..size * size).filter(|&cell| !area[cell] && was(cell) != is(cell)).count();
            // A hold holds every cell of the disc; a lift is said of the cells it lifted, those of the
            // disc that were held before it and are not now.
            let in_disc = (0..size * size).filter(|&cell| area[cell] && (!lift || (was(cell) && !is(cell)))).count();
            let (done, of_disc, on) = if lift { ("Lifted the hold on", "of the disc that were held", "on") } else { ("Held", "of the disc", "of") };
            let held = marked(&kept, &kept.marks.held);
            // Said once the land is written, so a write that fails says nothing was held.
            let written = write_land_saying(&kept, &out)?;
            println!(
                "{done} the {in_disc} cells {of_disc}, and {changed_beyond} cells beyond it {on} the lakes whose water it meets, each lake whole; {held} of the land's {} cells are held ({:.2} %)",
                size * size,
                share_pct(&kept, held)
            );
            println!("{written}");
            Ok(())
        }
        LandCommand::Regrow { land, disc, out } => {
            let kept = Land::read(&land)?;
            let area = disc_cells(kept.drawing.size_m as usize, disc);
            let edit = kept.regrow_area(&area, &never)?;
            print_edit(&edit);
            write_land(&edit.land, &out)
        }
        LandCommand::Export { land, out } => {
            let kept = Land::read(&land)?;
            let world = &kept.world;
            let written = write_world(world, &out)?;
            let (width, height) = (world.heights.width, world.heights.height);
            for (name, marks) in [("held.png", &kept.marks.held), ("by_hand.png", &kept.marks.by_hand)] {
                let grey: Vec<u8> = (0..width * height).map(|cell| if marks.get(cell) == Some(&true) { MARKED_GREY } else { 0 }).collect();
                write_png(&out.join(name), width, height, png::ColorType::Grayscale, &grey)?;
            }
            print_world(world, &out, &written);
            println!(
                "Marks: {} cells held, {} shaped by hand (held.png, by_hand.png)",
                marked(&kept, &kept.marks.held),
                marked(&kept, &kept.marks.by_hand)
            );
            Ok(())
        }
        LandCommand::Info { land } => {
            let LandFile { land: kept, format, version, skipped } = LandFile::read(&land)?;
            let (drawing, world) = (&kept.drawing, &kept.world);
            let country = COUNTRY_NAMES.iter().find(|name| Country::named(name) == Some(drawing.country)).map_or("tuned", |name| *name);
            let (held, by_hand) = (marked(&kept, &kept.marks.held), marked(&kept, &kept.marks.by_hand));
            println!("Land:      {}", land.display());
            println!("Format:    {format}, written by Landform {version}");
            println!("Size:      {} m, {} x {} cells of 1 m", drawing.size_m, world.heights.width, world.heights.height);
            println!("Seed:      {}", kept.seed);
            println!("Country:   {country}");
            println!("Drawing:   {} rivers, {} lakes", drawing.rivers.len(), drawing.lakes.len());
            println!("{}", waiting_line(&kept));
            let water = water_line(&world.water).map_or("none".to_string(), |line| line.trim_start_matches("Water: ").to_string());
            println!("Water:     {water}");
            // An outline reads the reach held to its range (the reader refuses one that is not finite).
            let reach_read = kept.reach_m.clamp(REACH_MIN_M, REACH_MAX_M);
            if reach_read == kept.reach_m {
                println!("Reach:     {} m", kept.reach_m);
            } else {
                println!("Reach:     {} m, read as {reach_read} m, since an outline reaches {REACH_MIN_M} to {REACH_MAX_M} m", kept.reach_m);
            }
            println!("Held:      {held} cells ({:.2} %)", share_pct(&kept, held));
            println!("By hand:   {by_hand} cells ({:.2} %)", share_pct(&kept, by_hand));
            println!("Notes:     {}", world.notes.len());
            if skipped.is_empty() {
                println!("Skipped:   nothing");
            } else {
                let parts: Vec<String> = skipped.iter().map(ToString::to_string).collect();
                println!("Skipped:   {}, which this Landform does not know; a land written again drops them", parts.join(", "));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disc_is_three_finite_numbers_and_a_radius_above_0() {
        assert_eq!(parse_disc("-64,416,60"), Ok(Disc { x_m: -64.0, z_m: 416.0, radius_m: 60.0 }));
        assert_eq!(parse_disc(" 1.5, -2 ,0.25"), Ok(Disc { x_m: 1.5, z_m: -2.0, radius_m: 0.25 }));
        for bad in ["", "1,2", "1,2,3,4", "a,2,3", "1,2,0", "1,2,-3", "inf,2,3", "1,NaN,3", "1;2;3"] {
            let error = parse_disc(bad).unwrap_err();
            assert!(error.contains("x,z,radius") && error.contains(&format!("\"{bad}\"")), "{error}");
        }
    }

    #[test]
    fn a_discs_cells_are_those_whose_place_lies_within_its_radius() {
        let size = 64;
        let cells = disc_cells(size, Disc { x_m: -10.0, z_m: 5.0, radius_m: 3.0 });
        // The cell at (-10, 5) is column 22, row 37; (-13, 5) and (-10, 8) lie on the rim, (-12, 7)
        // 2.83 m from the centre; (-14, 5) and (-10, 9) lie 4 m from it, (-13, 7) 3.61 m.
        let at = |column: usize, row: usize| cells[row * size + column];
        assert!(at(22, 37) && at(19, 37) && at(22, 40) && at(20, 39));
        assert!(!at(18, 37) && !at(22, 41) && !at(19, 39));
        assert_eq!(cells.iter().filter(|&&on| on).count(), 29, "the lattice points within 3 of a lattice point");
    }
}
