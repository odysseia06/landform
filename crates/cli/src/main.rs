mod drawing_file;
mod geotiff;
mod land_cmd;
mod pack_cmd;
mod preview;
mod recipe;

use clap::builder::PossibleValuesParser;
use clap::{ArgGroup, Parser, Subcommand};
use landform_core::{
    generate, generate_drawn, water_rgb, water_surface, CourseEnd, Drawing, ElevationSource, GenConfig, Grid, LandformError, Material,
    trial_drawing, unheld, Country, PackSource, RiverCourse, SyntheticSource, Unheld, Water, WaterRules, World, COUNTRY_NAMES, LAYER_NAMES,
    TRIAL_NAMES, WATER_LAYER,
};
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// The name `write-example` gives the example drawing, `Drawing::example_valley`.
const EXAMPLE_NAME: &str = "valley";

/// The kind of country `write-example` writes unless told another: the example's own.
const EXAMPLE_COUNTRY: &str = "hills";

/// The side of the example drawing `write-example` writes unless told another, in metres.
const EXAMPLE_SIZE_M: u32 = 2048;

/// The side of a trial drawing `write-example` writes unless told another, in metres: the size the
/// counting program grows them at.
const TRIAL_SIZE_M: u32 = 1024;

#[derive(Parser)]
#[command(name = "landform", about = "Inspect terrain packs, generate worlds from them or from drawings, and keep lands as files.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the header of a terrain pack.
    Info {
        /// The pack file to read.
        pack: PathBuf,
    },
    /// Generate a world and write its heights, shaded relief, ground map, layer pictures and metadata.
    #[command(group(ArgGroup::new("input").required(true).args(["synthetic", "pack", "drawing"])))]
    Generate {
        /// Seed that decides which part of the sources becomes the world.
        #[arg(long)]
        seed: u64,
        /// Use the built-in synthetic terrain instead of packs.
        #[arg(long)]
        synthetic: bool,
        /// A terrain pack to draw from; repeat the flag for several, in priority order.
        #[arg(long)]
        pack: Vec<PathBuf>,
        /// A folder holding a drawing (`drawing.toml` and its picture) to grow the land from.
        #[arg(long)]
        drawing: Option<PathBuf>,
        /// With --drawing: the kind of country, replacing the drawing's own; its [tune] still applies.
        #[arg(long, conflicts_with_all = ["synthetic", "pack"], value_parser = PossibleValuesParser::new(COUNTRY_NAMES))]
        country: Option<String>,
        /// World side length in cells; a drawing sets its own.
        #[arg(long, conflicts_with = "drawing", default_value_t = GenConfig::default().size_cells)]
        size: usize,
        /// Directory to write into; created when missing.
        #[arg(long)]
        out: PathBuf,
    },
    /// Write the example drawing, or one of the trial drawings, as a drawing folder.
    WriteExample {
        /// The folder to write into; created when missing.
        #[arg(long)]
        out: PathBuf,
        /// The drawing: the example valley, or one of the trial drawings.
        #[arg(long, default_value = EXAMPLE_NAME, value_parser = PossibleValuesParser::new(std::iter::once(EXAMPLE_NAME).chain(TRIAL_NAMES)))]
        name: String,
        /// The kind of country the drawing is written with.
        #[arg(long, default_value = EXAMPLE_COUNTRY, value_parser = PossibleValuesParser::new(COUNTRY_NAMES))]
        country: String,
        // Its default depends on the drawing, so clap holds none and the help says both.
        #[arg(long, help = format!("Side of the land in metres [default: {EXAMPLE_SIZE_M} for the valley, {TRIAL_SIZE_M} for a trial drawing]"))]
        size: Option<u32>,
    },
    /// Make, edit, ask, hold, regrow, export and describe lands kept as land files (.lfland).
    Land {
        #[command(subcommand)]
        command: land_cmd::LandCommand,
    },
    /// Build a terrain pack from USGS elevation tiles, following a region recipe.
    Pack {
        /// The region recipe (TOML) naming the tiles and the square to cut out.
        #[arg(long)]
        recipe: PathBuf,
        /// Directory holding the tile files the recipe names.
        #[arg(long)]
        tiles: PathBuf,
        /// The pack file to write; its directory is created when missing.
        #[arg(long)]
        out: PathBuf,
    },
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> LandformError {
    LandformError::Io(format!("{}: {error}", path.display()))
}

fn info(path: &Path) -> Result<(), LandformError> {
    let pack = PackSource::open(path)?;
    let header = pack.header();
    println!("Pack:         {}", path.display());
    println!("Size:         {} x {} cells", header.width, header.height);
    println!("Cell size:    {} m", header.cell_size_m);
    println!("Height range: {} m to {} m", header.height_offset, header.highest_m());
    println!("Source id:    {}", header.source_id);
    println!("Attribution:  {}", header.attribution);
    Ok(())
}

fn write_heights(path: &Path, world: &World) -> Result<(), LandformError> {
    let mut bytes = Vec::with_capacity(world.heights.data.len() * 4);
    for h in &world.heights.data {
        bytes.extend_from_slice(&h.to_le_bytes());
    }
    fs::write(path, bytes).map_err(|e| io_error(path, e))
}

/// Writes `data`, `width` by `height` cells, as an 8-bit PNG of the given colour type.
fn write_png(
    path: &Path,
    width: usize,
    height: usize,
    color: png::ColorType,
    data: &[u8],
) -> Result<(), LandformError> {
    let file = File::create(path).map_err(|e| io_error(path, e))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| io_error(path, e))?;
    writer.write_image_data(data).map_err(|e| io_error(path, e))?;
    writer.finish().map_err(|e| io_error(path, e))
}

/// `heights` less their lowest value, as `Frame::Lowest` lowers a world (the lowest found with
/// `f32::min`, then taken from each height in `f32`); `None` when the lowest is already 0.
fn lowered(heights: &Grid<f32>) -> Option<Grid<f32>> {
    let lowest = heights.data.iter().copied().fold(f32::INFINITY, f32::min);
    (lowest != 0.0).then(|| Grid { data: heights.data.iter().map(|h| h - lowest).collect(), ..heights.clone() })
}

/// Writes the pictures of `world`. The shaded relief is drawn from the heights less their lowest, as
/// every world `generate` makes stands (its lowest cell is 0, so for it nothing changes): a land,
/// whose heights stay in the model's frame, then gives the very picture `generate` gives of the same
/// world, where the heights' differences would otherwise round otherwise in a few cells.
fn write_pictures(out: &Path, world: &World) -> Result<(), LandformError> {
    let (width, height) = (world.heights.width, world.heights.height);
    let grey = png::ColorType::Grayscale;
    let relief = lowered(&world.heights);
    write_png(&out.join("hillshade.png"), width, height, grey, &preview::hillshade(relief.as_ref().unwrap_or(&world.heights)))?;
    write_png(&out.join("ground.png"), width, height, png::ColorType::Rgb, &preview::ground_rgb(&world.ground))?;
    for name in LAYER_NAMES {
        let data = preview::layer_grey(name, &world.layers[name]);
        write_png(&out.join(format!("layer_{name}.png")), width, height, grey, &data)?;
    }
    // A drawn world's water layer is a picture of its own, in colour; the other worlds have none.
    if let Some(water) = world.layers.get(WATER_LAYER) {
        let data: Vec<u8> = water.data.iter().flat_map(|&depth| water_rgb(depth)).collect();
        write_png(&out.join("water.png"), width, height, png::ColorType::Rgb, &data)?;
    }
    Ok(())
}

/// The length of `course`, in metres: the distances between its consecutive points, added.
fn course_length_m(course: &RiverCourse) -> f64 {
    course
        .points
        .windows(2)
        .map(|pair| (f64::from(pair[1].x_m) - f64::from(pair[0].x_m)).hypot(f64::from(pair[1].z_m) - f64::from(pair[0].z_m)))
        .sum()
}

/// How a course ends, as `meta.json` says it: "edge", "river N" or "lake N".
fn end_text(end: CourseEnd) -> String {
    match end {
        CourseEnd::Edge => "edge".into(),
        CourseEnd::River(index) => format!("river {index}"),
        CourseEnd::Lake(index) => format!("lake {index}"),
    }
}

/// `meta.json`'s `water`: every course and every lake, in the world's order.
fn water_json(water: &Water) -> serde_json::Value {
    let rivers: Vec<serde_json::Value> = water
        .rivers
        .iter()
        .map(|course| {
            let widths = course.points.iter().map(|point| point.width_m);
            let (narrowest, widest) = widths.fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), w| (low.min(w), high.max(w)));
            serde_json::json!({
                "drawn": course.drawn,
                "from_lake": course.from_lake,
                "end": end_text(course.end),
                "points": course.points.len(),
                "length_m": course_length_m(course),
                "width_m": [narrowest, widest],
            })
        })
        .collect();
    let lakes: Vec<serde_json::Value> = water
        .lakes
        .iter()
        .map(|lake| {
            serde_json::json!({
                "drawn": lake.drawn,
                "level_m": lake.level_m,
                "area_m2": lake.area_m2,
                "x_m": lake.x_m,
                "z_m": lake.z_m,
            })
        })
        .collect();
    serde_json::json!({ "rivers": rivers, "lakes": lakes })
}

/// What `generate` says of a world's water surface: its pieces, points and triangles, and how long
/// building it took.
struct SurfaceCount {
    pieces: usize,
    points: usize,
    triangles: usize,
    ms: f32,
}

/// The surface of `world`'s water (`water_surface`, on rayon's global pool), counted and timed, or
/// `None` for a world without the `water` layer: one cut from data.
fn count_surface(world: &World) -> Result<Option<SurfaceCount>, LandformError> {
    let Some(depth) = world.layers.get(WATER_LAYER) else { return Ok(None) };
    let started = Instant::now();
    let surface = water_surface(&world.heights, depth, &world.water)?;
    let ms = started.elapsed().as_secs_f32() * 1000.0;
    Ok(Some(SurfaceCount {
        pieces: surface.pieces.len(),
        points: surface.pieces.iter().map(|piece| piece.positions.len()).sum(),
        triangles: surface.pieces.iter().map(|piece| piece.indices.len() / 3).sum(),
        ms,
    }))
}

/// Each material's share of the world's cells by strongest material, in `Material::ALL` order.
fn cover_fractions(world: &World) -> [f64; Material::ALL.len()] {
    let mut counts = [0usize; Material::ALL.len()];
    for cell in &world.ground.data {
        counts[usize::from(cell.strongest)] += 1;
    }
    let total = world.ground.data.len().max(1) as f64;
    counts.map(|count| count as f64 / total)
}

/// What `generate` says of a world's water, in one line: how many rivers and brooks, their length,
/// how many lakes and their area, the metres and square metres rounded to whole ones; `None` for a
/// world with no river, brook or lake.
fn water_line(water: &Water) -> Option<String> {
    if water.rivers.is_empty() && water.lakes.is_empty() {
        return None;
    }
    // Folded from 0: a float sum of nothing is -0, which prints as "-0".
    let length_m = water.rivers.iter().map(course_length_m).fold(0.0, |sum, length_m| sum + length_m);
    let area_m2 = water.lakes.iter().map(|lake| f64::from(lake.area_m2)).fold(0.0, |sum, area_m2| sum + area_m2);
    Some(format!("Water: {} rivers and brooks ({length_m:.0} m), {} lakes ({area_m2:.0} m²)", water.rivers.len(), water.lakes.len()))
}

/// What `generate` says of the water the ground does not hold, in one line.
fn unheld_line(found: &Unheld) -> String {
    format!(
        "Unheld: {} dry cells below the water beside them ({} at a lake, {} at a course, the worst {:.2} m), {} lake cells over lower water, {} steps between water cells ({} more at mouths)",
        found.dry(),
        found.dry_lake,
        found.dry_course,
        found.worst_dry_m,
        found.lake_over_water,
        found.water_steps,
        found.mouth_steps
    )
}

/// `meta.json`'s `unheld`: the nine counts of `found` under their names, `worst_dry_at` as
/// `[x, z]`; `null` for a world cut from data.
fn unheld_json(found: Option<&Unheld>) -> serde_json::Value {
    let Some(found) = found else { return serde_json::Value::Null };
    serde_json::json!({
        "dry_lake": found.dry_lake,
        "dry_course": found.dry_course,
        "dry_over_1m": found.dry_over_1m,
        "worst_dry_m": found.worst_dry_m,
        "worst_dry_at": [found.worst_dry_at.0, found.worst_dry_at.1],
        "lake_over_water": found.lake_over_water,
        "water_steps": found.water_steps,
        "mouth_steps": found.mouth_steps,
        "worst_step_m": found.worst_step_m,
    })
}

/// Writes `meta.json`, with `surface` the world's water surface counted and `found` the water its
/// ground does not hold, each `None` for a world cut from data. `notes_about` says, for each of
/// `notes` and in its order, what the note is about as the core writes it (`About`): "land",
/// "river N" or "lake N", N the index in the drawing's rivers or lakes counted from 0, as `drawn`
/// counts a course's drawn river; empty, as `notes` is, for a world cut from data.
fn write_meta(path: &Path, world: &World, surface: Option<&SurfaceCount>, found: Option<&Unheld>) -> Result<(), LandformError> {
    let meta = &world.meta;
    // `serde_json` objects keep their keys sorted, so `materials` is what fixes the order.
    let cover: serde_json::Map<String, serde_json::Value> = Material::ALL
        .iter()
        .zip(cover_fractions(world))
        .map(|(material, fraction)| (material.name().to_string(), fraction.into()))
        .collect();
    let (window, timings) = (&meta.window, &meta.timings);
    let json = serde_json::json!({
        "seed": meta.seed,
        "source_id": meta.source_id,
        "size": world.heights.width,
        "base_elevation_m": meta.base_elevation_m,
        "north": [meta.north.0, meta.north.1],
        "window": {
            "source": window.source,
            "center_x": window.center_x,
            "center_y": window.center_y,
            "rotation_rad": window.rotation_rad,
            "mirrored": window.mirrored,
            "size_m": window.size_m,
        },
        "timings": {
            "grow_ms": timings.grow_ms,
            "pick_ms": timings.pick_ms,
            "resample_ms": timings.resample_ms,
            "analyse_ms": timings.analyse_ms,
            "detail_ms": timings.detail_ms,
            "water_ms": timings.water_ms,
            "cover_ms": timings.cover_ms,
            "total_ms": timings.total_ms,
            "surface_ms": surface.map_or(0.0, |count| count.ms),
        },
        "materials": Material::ALL.map(Material::name),
        "cover": cover,
        "notes": world.notes,
        "notes_about": world.notes_about.iter().map(ToString::to_string).collect::<Vec<String>>(),
        "water": water_json(&world.water),
        "surface": surface.map(|count| serde_json::json!({
            "pieces": count.pieces,
            "points": count.points,
            "triangles": count.triangles,
        })),
        "unheld": unheld_json(found),
    });
    let text = serde_json::to_string_pretty(&json).expect("a JSON value always serialises");
    let mut file = File::create(path).map_err(|e| io_error(path, e))?;
    writeln!(file, "{text}").map_err(|e| io_error(path, e))
}

/// Where a generated world comes from.
enum Input<'a> {
    /// Windows cut from packs, or from the synthetic terrain.
    Sources { synthetic: bool, packs: &'a [PathBuf], size: usize },
    /// Land grown from the drawing in a folder, with another kind of country when `country` is set.
    Drawing { folder: &'a Path, country: Option<&'a str> },
}

fn make_world(seed: u64, input: &Input) -> Result<World, LandformError> {
    match *input {
        Input::Sources { synthetic, packs, size } => {
            let packs = packs.iter().map(|path| PackSource::open(path)).collect::<Result<Vec<_>, _>>()?;
            let synthetic_source = SyntheticSource::standard();
            let sources: Vec<&dyn ElevationSource> = if synthetic {
                vec![&synthetic_source]
            } else {
                packs.iter().map(|p| p as &dyn ElevationSource).collect()
            };
            generate(seed, &sources, &GenConfig { size_cells: size, ..GenConfig::default() })
        }
        Input::Drawing { folder, country } => {
            let (drawing, water) = match country {
                None => drawing_file::read_drawing(folder)?,
                Some(kind) => drawing_file::read_drawing_as(folder, kind)?,
            };
            // Nothing sets this flag: the command line cannot cancel a generation.
            generate_drawn(seed, &drawing, &GenConfig { water, ..GenConfig::default() }, &AtomicBool::new(false))
        }
    }
}

/// What writing a world's files found of it, for what is printed of it.
struct Written {
    /// The water's surface counted, `None` for a world without the `water` layer.
    surface: Option<SurfaceCount>,
    /// The water the ground does not hold, `None` likewise.
    found: Option<Unheld>,
}

/// Writes `world`'s files into `out`, created when missing: `heights.r32`, the pictures and
/// `meta.json`, as `generate` writes them (and `land export`, from this same code).
fn write_world(world: &World, out: &Path) -> Result<Written, LandformError> {
    let surface = count_surface(world)?;
    let found = unheld(world);
    fs::create_dir_all(out).map_err(|e| io_error(out, e))?;
    write_heights(&out.join("heights.r32"), world)?;
    write_pictures(out, world)?;
    write_meta(&out.join("meta.json"), world, surface.as_ref(), found.as_ref())?;
    Ok(Written { surface, found })
}

/// The timings line `generate` prints of a world.
fn timings_line(t: &landform_core::Timings) -> String {
    format!(
        "Timings: grow {:.1} ms, pick {:.1} ms, resample {:.1} ms, analyse {:.1} ms, detail {:.1} ms, cover {:.1} ms, total {:.1} ms",
        t.grow_ms, t.pick_ms, t.resample_ms, t.analyse_ms, t.detail_ms, t.cover_ms, t.total_ms
    )
}

/// What `generate` prints of a world it wrote into `out`: its size and where, its timings, its
/// cover, its water, its water's surface, the water its ground does not hold, and its notes.
fn print_world(world: &World, out: &Path, written: &Written) {
    let size = world.heights.width;
    println!("Wrote a {size} x {size} world to {}", out.display());
    println!("{}", timings_line(&world.meta.timings));
    let shares: Vec<String> = Material::ALL
        .iter()
        .zip(cover_fractions(world))
        .map(|(material, fraction)| format!("{} {:.1}%", material.name(), fraction * 100.0))
        .collect();
    println!("Cover: {}", shares.join(", "));
    if let Some(line) = water_line(&world.water) {
        println!("{line}");
    }
    if let Some(count) = &written.surface {
        println!("Surface: {} pieces, {} points, {} triangles, {:.1} ms", count.pieces, count.points, count.triangles, count.ms);
    }
    if let Some(found) = &written.found {
        println!("{}", unheld_line(found));
    }
    for note in &world.notes {
        println!("Note: {note}");
    }
}

fn generate_world(seed: u64, input: &Input, out: &Path) -> Result<(), LandformError> {
    let world = make_world(seed, input)?;
    let written = write_world(&world, out)?;
    print_world(&world, out, &written);
    Ok(())
}

/// Writes the drawing called `name` (`EXAMPLE_NAME` or one of `TRIAL_NAMES`) into the folder `out`, in
/// the kind of country `country`, `size` metres across (the drawing's own size when `None`).
fn write_example(out: &Path, name: &str, size: Option<u32>, country: &str) -> Result<(), LandformError> {
    let kind = Country::named(country).ok_or_else(|| LandformError::BadConfig(format!("\"{country}\" is not a kind of country")))?;
    let drawing = if name == EXAMPLE_NAME {
        let size = size.unwrap_or(EXAMPLE_SIZE_M);
        Drawing { country: kind, ..Drawing::example_valley(size) }
    } else {
        trial_drawing(name, size.unwrap_or(TRIAL_SIZE_M), kind).ok_or_else(|| LandformError::BadConfig(format!("\"{name}\" is not the name of a drawing")))?
    };
    drawing_file::write_drawing(out, &drawing, &WaterRules::default())?;
    let what = if name == EXAMPLE_NAME { "the example drawing".to_string() } else { format!("the trial drawing {name}") };
    println!("Wrote {what}, {} m across, to {}", drawing.size_m, out.display());
    Ok(())
}

fn main() -> ExitCode {
    let result: Result<(), String> = match Cli::parse().command {
        Command::Info { pack } => info(&pack).map_err(|e| e.to_string()),
        Command::Generate { seed, synthetic, pack, drawing, country, size, out } => {
            let input = match &drawing {
                Some(folder) => Input::Drawing { folder, country: country.as_deref() },
                None => Input::Sources { synthetic, packs: &pack, size },
            };
            generate_world(seed, &input, &out).map_err(|e| e.to_string())
        }
        Command::WriteExample { out, name, country, size } => write_example(&out, &name, size, &country).map_err(|e| e.to_string()),
        Command::Pack { recipe, tiles, out } => pack_cmd::run(&recipe, &tiles, &out),
        Command::Land { command } => match land_cmd::run(command) {
            // A refused edit writes nothing, and says so by its own exit code.
            Err(refused @ LandformError::Refused(_)) => {
                eprintln!("{refused}");
                return ExitCode::from(land_cmd::REFUSED_EXIT);
            }
            other => other.map_err(|e| e.to_string()),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made_up() -> Unheld {
        Unheld {
            dry_lake: 3,
            dry_course: 4,
            dry_over_1m: 1,
            worst_dry_m: 1.2345,
            worst_dry_at: (-12.5, 40.0),
            lake_over_water: 5,
            water_steps: 6,
            mouth_steps: 7,
            worst_step_m: 0.75,
        }
    }

    #[test]
    fn the_unheld_line_says_the_counts_in_words() {
        assert_eq!(
            unheld_line(&made_up()),
            "Unheld: 7 dry cells below the water beside them (3 at a lake, 4 at a course, the worst 1.23 m), 5 lake cells over lower water, 6 steps between water cells (7 more at mouths)"
        );
        assert_eq!(
            unheld_line(&Unheld::default()),
            "Unheld: 0 dry cells below the water beside them (0 at a lake, 0 at a course, the worst 0.00 m), 0 lake cells over lower water, 0 steps between water cells (0 more at mouths)"
        );
    }

    #[test]
    fn the_water_line_says_no_lakes_and_no_courses_as_0() {
        let point = |x_m: f32| landform_core::CoursePoint { x_m, z_m: 0.0, surface_m: 10.0, width_m: 2.0, depth_m: 0.5 };
        let course = RiverCourse { drawn: Some(0), from_lake: None, points: vec![point(-4.0), point(4.0)], end: CourseEnd::Edge };
        let lake = landform_core::LakeWater { drawn: 0, level_m: 10.0, x_m: 0.0, z_m: 0.0, area_m2: 1234.4 };
        let rivers_only = Water { rivers: vec![course.clone()], lakes: Vec::new() };
        assert_eq!(water_line(&rivers_only).as_deref(), Some("Water: 1 rivers and brooks (8 m), 0 lakes (0 m²)"));
        let lake_only = Water { rivers: Vec::new(), lakes: vec![lake] };
        assert_eq!(water_line(&lake_only).as_deref(), Some("Water: 0 rivers and brooks (0 m), 1 lakes (1234 m²)"));
        assert_eq!(water_line(&Water { rivers: vec![course], lakes: vec![lake] }).as_deref(), Some("Water: 1 rivers and brooks (8 m), 1 lakes (1234 m²)"));
        assert_eq!(water_line(&Water::default()), None);
    }

    #[test]
    fn meta_holds_the_nine_fields_or_null() {
        let json = unheld_json(Some(&made_up()));
        assert_eq!(json.as_object().unwrap().len(), 9);
        assert_eq!(json["dry_lake"], 3);
        assert_eq!(json["dry_course"], 4);
        assert_eq!(json["dry_over_1m"], 1);
        assert_eq!(json["worst_dry_m"], 1.2345f32);
        assert_eq!(json["worst_dry_at"], serde_json::json!([-12.5, 40.0]));
        assert_eq!(json["lake_over_water"], 5);
        assert_eq!(json["water_steps"], 6);
        assert_eq!(json["mouth_steps"], 7);
        assert_eq!(json["worst_step_m"], 0.75);
        assert!(unheld_json(None).is_null());
    }

    #[test]
    fn a_trial_name_writes_its_drawing() {
        let folder = std::env::temp_dir().join(format!("landform_cli_unit_{}_headjoin", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        write_example(&folder, "headjoin", Some(256), "hills").unwrap();
        let (drawing, _) = drawing_file::read_drawing(&folder).unwrap();
        let _ = fs::remove_dir_all(&folder);
        assert_eq!((drawing.size_m, drawing.rivers.len(), drawing.lakes.len()), (256, 3, 0));
        assert!(write_example(&folder, "nothing", None, "hills").is_err());
        assert!(!folder.exists());
    }
}
