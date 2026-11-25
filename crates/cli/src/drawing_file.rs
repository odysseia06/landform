//! A drawing as a folder: `drawing.toml` and, when it names one, the picture of high ground.

use crate::{io_error, write_png};
use landform_core::{Country, Drawing, Grid, LandformError, Lake, River, WaterRules, COUNTRY_NAMES};
use serde::Deserialize;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;

/// The file in the folder that holds the drawing's numbers.
const DRAWING_FILE: &str = "drawing.toml";
/// The picture `write_drawing` writes next to it.
const PICTURE_FILE: &str = "high_ground.png";

/// `drawing.toml`, as parsed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DrawingToml {
    size_m: u32,
    /// A file in the same folder, lighter is higher.
    high_ground: Option<String>,
    /// One of `COUNTRY_NAMES`; the core's default kind, hill country, when it is left out.
    country: Option<String>,
    /// Fields of `Country` that override the named kind.
    tune: Option<Tune>,
    /// Fields of `WaterRules` that override the defaults.
    water: Option<WaterTune>,
    #[serde(default)]
    rivers: Vec<RiverToml>,
    #[serde(default)]
    lakes: Vec<LakeToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RiverToml {
    width_m: f32,
    points: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LakeToml {
    x_m: f64,
    z_m: f64,
    radius_m: f32,
}

/// Any field of `Country`, each replacing the named kind's value when present.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tune {
    floor_m: Option<f32>,
    relief_m: Option<f32>,
    rough_m: Option<f32>,
    rough_size_m: Option<f32>,
    lift_low_m: Option<f32>,
    lift_high_m: Option<f32>,
    cut_strength: Option<f32>,
    cut_area_power: Option<f32>,
    slump_m2: Option<f32>,
    rounds: Option<u32>,
    hardness_variation: Option<f32>,
    hardness_patch_m: Option<f32>,
}

impl Tune {
    /// `base` with every field this tune names replaced.
    fn apply(&self, base: Country) -> Country {
        // Every field spelt out, so a new field of `Country` fails to compile until it is tuneable.
        Country {
            floor_m: self.floor_m.unwrap_or(base.floor_m),
            relief_m: self.relief_m.unwrap_or(base.relief_m),
            rough_m: self.rough_m.unwrap_or(base.rough_m),
            rough_size_m: self.rough_size_m.unwrap_or(base.rough_size_m),
            lift_low_m: self.lift_low_m.unwrap_or(base.lift_low_m),
            lift_high_m: self.lift_high_m.unwrap_or(base.lift_high_m),
            cut_strength: self.cut_strength.unwrap_or(base.cut_strength),
            cut_area_power: self.cut_area_power.unwrap_or(base.cut_area_power),
            slump_m2: self.slump_m2.unwrap_or(base.slump_m2),
            rounds: self.rounds.unwrap_or(base.rounds),
            hardness_variation: self.hardness_variation.unwrap_or(base.hardness_variation),
            hardness_patch_m: self.hardness_patch_m.unwrap_or(base.hardness_patch_m),
        }
    }
}

/// Any field of `WaterRules`, each replacing the default when present.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WaterTune {
    brook_start_m2: Option<f32>,
    brook_width_m: Option<f32>,
    width_area_power: Option<f32>,
    river_inflow_m2: Option<f32>,
    river_inflow_width_m: Option<f32>,
    bed_depth_share: Option<f32>,
    bed_depth_min_m: Option<f32>,
    bed_depth_max_m: Option<f32>,
    bank_run: Option<f32>,
    water_fill: Option<f32>,
    bank_wet_full_m: Option<f32>,
    bank_wet_none_m: Option<f32>,
    bank_wet_reach_m: Option<f32>,
    shore_run: Option<f32>,
    shore_reach_m: Option<f32>,
    width_wander: Option<f32>,
    wander_length_widths: Option<f32>,
    bend_shift: Option<f32>,
}

impl WaterTune {
    /// `base` with every field this table names replaced.
    fn apply(&self, base: WaterRules) -> WaterRules {
        // Every field spelt out, so a new field of `WaterRules` fails to compile until it is tuneable.
        WaterRules {
            brook_start_m2: self.brook_start_m2.unwrap_or(base.brook_start_m2),
            brook_width_m: self.brook_width_m.unwrap_or(base.brook_width_m),
            width_area_power: self.width_area_power.unwrap_or(base.width_area_power),
            river_inflow_m2: self.river_inflow_m2.unwrap_or(base.river_inflow_m2),
            river_inflow_width_m: self.river_inflow_width_m.unwrap_or(base.river_inflow_width_m),
            bed_depth_share: self.bed_depth_share.unwrap_or(base.bed_depth_share),
            bed_depth_min_m: self.bed_depth_min_m.unwrap_or(base.bed_depth_min_m),
            bed_depth_max_m: self.bed_depth_max_m.unwrap_or(base.bed_depth_max_m),
            bank_run: self.bank_run.unwrap_or(base.bank_run),
            water_fill: self.water_fill.unwrap_or(base.water_fill),
            bank_wet_full_m: self.bank_wet_full_m.unwrap_or(base.bank_wet_full_m),
            bank_wet_none_m: self.bank_wet_none_m.unwrap_or(base.bank_wet_none_m),
            bank_wet_reach_m: self.bank_wet_reach_m.unwrap_or(base.bank_wet_reach_m),
            shore_run: self.shore_run.unwrap_or(base.shore_run),
            shore_reach_m: self.shore_reach_m.unwrap_or(base.shore_reach_m),
            width_wander: self.width_wander.unwrap_or(base.width_wander),
            wander_length_widths: self.wander_length_widths.unwrap_or(base.wander_length_widths),
            bend_shift: self.bend_shift.unwrap_or(base.bend_shift),
        }
    }
}

/// Each field of `country` as its name and TOML value, in declaration order.
fn country_fields(country: &Country) -> [(&'static str, String); 12] {
    // Taken apart field by field, so a new field fails to compile until it is written out here.
    let Country {
        floor_m,
        relief_m,
        rough_m,
        rough_size_m,
        lift_low_m,
        lift_high_m,
        cut_strength,
        cut_area_power,
        slump_m2,
        rounds,
        hardness_variation,
        hardness_patch_m,
    } = *country;
    // `{:?}` on a float is its shortest text that reads back exactly, with a point or an exponent.
    [
        ("floor_m", format!("{floor_m:?}")),
        ("relief_m", format!("{relief_m:?}")),
        ("rough_m", format!("{rough_m:?}")),
        ("rough_size_m", format!("{rough_size_m:?}")),
        ("lift_low_m", format!("{lift_low_m:?}")),
        ("lift_high_m", format!("{lift_high_m:?}")),
        ("cut_strength", format!("{cut_strength:?}")),
        ("cut_area_power", format!("{cut_area_power:?}")),
        ("slump_m2", format!("{slump_m2:?}")),
        ("rounds", rounds.to_string()),
        ("hardness_variation", format!("{hardness_variation:?}")),
        ("hardness_patch_m", format!("{hardness_patch_m:?}")),
    ]
}

/// Each field of `rules` as its name and TOML value, in declaration order.
pub(crate) fn water_fields(rules: &WaterRules) -> [(&'static str, String); 18] {
    // Taken apart field by field, so a new field fails to compile until it is written out here.
    let WaterRules {
        brook_start_m2,
        brook_width_m,
        width_area_power,
        river_inflow_m2,
        river_inflow_width_m,
        bed_depth_share,
        bed_depth_min_m,
        bed_depth_max_m,
        bank_run,
        water_fill,
        bank_wet_full_m,
        bank_wet_none_m,
        bank_wet_reach_m,
        shore_run,
        shore_reach_m,
        width_wander,
        wander_length_widths,
        bend_shift,
    } = *rules;
    [
        ("brook_start_m2", format!("{brook_start_m2:?}")),
        ("brook_width_m", format!("{brook_width_m:?}")),
        ("width_area_power", format!("{width_area_power:?}")),
        ("river_inflow_m2", format!("{river_inflow_m2:?}")),
        ("river_inflow_width_m", format!("{river_inflow_width_m:?}")),
        ("bed_depth_share", format!("{bed_depth_share:?}")),
        ("bed_depth_min_m", format!("{bed_depth_min_m:?}")),
        ("bed_depth_max_m", format!("{bed_depth_max_m:?}")),
        ("bank_run", format!("{bank_run:?}")),
        ("water_fill", format!("{water_fill:?}")),
        ("bank_wet_full_m", format!("{bank_wet_full_m:?}")),
        ("bank_wet_none_m", format!("{bank_wet_none_m:?}")),
        ("bank_wet_reach_m", format!("{bank_wet_reach_m:?}")),
        ("shore_run", format!("{shore_run:?}")),
        ("shore_reach_m", format!("{shore_reach_m:?}")),
        ("width_wander", format!("{width_wander:?}")),
        ("wander_length_widths", format!("{wander_length_widths:?}")),
        ("bend_shift", format!("{bend_shift:?}")),
    ]
}

/// The preset called `name`, or `BadConfig` listing the three that exist.
fn named_country(name: &str) -> Result<Country, LandformError> {
    Country::named(name).ok_or_else(|| {
        LandformError::BadConfig(format!("country must be one of {}, not \"{name}\"", COUNTRY_NAMES.join(", ")))
    })
}

/// A `BadConfig` that names the file it found the trouble in.
fn bad_file(path: &Path, reason: impl std::fmt::Display) -> LandformError {
    LandformError::BadConfig(format!("{}: {reason}", path.display()))
}

/// Reads the drawing in `folder`: `drawing.toml` and the picture it names. A file that names no
/// country grows hill country, the core's default, as a drawing with no country does in Godot. The
/// result is validated, so a bad drawing fails with `Drawing::validate`'s sentence. The water rules
/// come with it: the defaults with the file's `[water]` table laid over them, validated likewise.
pub fn read_drawing(folder: &Path) -> Result<(Drawing, WaterRules), LandformError> {
    read(folder, None)
}

/// As `read_drawing`, with the kind of country `kind` instead of the file's own; the file's `[tune]`
/// still applies on top of it.
pub fn read_drawing_as(folder: &Path, kind: &str) -> Result<(Drawing, WaterRules), LandformError> {
    read(folder, Some(kind))
}

fn read(folder: &Path, kind: Option<&str>) -> Result<(Drawing, WaterRules), LandformError> {
    let path = folder.join(DRAWING_FILE);
    let text = fs::read_to_string(&path).map_err(|e| io_error(&path, e))?;
    let file: DrawingToml = toml::from_str(&text).map_err(|e| bad_file(&path, e))?;

    let base = match kind.or(file.country.as_deref()) {
        Some(name) => named_country(name)?,
        None => Country::default(),
    };
    let country = file.tune.as_ref().map_or(base, |tune| tune.apply(base));
    let water = file.water.as_ref().map_or(WaterRules::default(), |tune| tune.apply(WaterRules::default()));
    let high_ground = file
        .high_ground
        .as_deref()
        .map(|name| {
            if Path::new(name).file_name() != Some(name.as_ref()) {
                return Err(bad_file(&path, format!("high_ground must be a file name in the drawing's folder, not \"{name}\"")));
            }
            read_picture(&folder.join(name), file.size_m)
        })
        .transpose()?;
    let drawing = Drawing {
        size_m: file.size_m,
        high_ground,
        rivers: file
            .rivers
            .iter()
            .map(|river| River { points: river.points.iter().map(|&[x, z]| (x, z)).collect(), width_m: river.width_m })
            .collect(),
        lakes: file.lakes.iter().map(|l| Lake { x_m: l.x_m, z_m: l.z_m, radius_m: l.radius_m }).collect(),
        country,
    };
    drawing.validate()?;
    water.validate()?;
    Ok((drawing, water))
}

/// The picture at `path` as values from 0 to 1: its first channel divided by that depth's maximum.
/// Grey, grey with alpha, RGB, RGBA and palette pictures of 1 to 16 bits are read.
fn read_picture(path: &Path, size_m: u32) -> Result<Grid<f32>, LandformError> {
    let file = File::open(path).map_err(|e| io_error(path, e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    // Palettes become colours and shallow depths become 8 bit; 16 bit stays as it is.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| io_error(path, e))?;
    let mut buffer = vec![0u8; reader.output_buffer_size().ok_or_else(|| io_error(path, "the picture is too large"))?];
    let info = reader.next_frame(&mut buffer).map_err(|e| io_error(path, e))?;

    let (width, height) = (info.width as usize, info.height as usize);
    let bytes = match info.bit_depth {
        png::BitDepth::Sixteen => 2,
        _ => 1,
    };
    let pixel = info.color_type.samples() * bytes;
    let maximum = if bytes == 2 { 65535.0 } else { 255.0 };
    let mut picture = Grid::<f32>::new(width, height, size_m as f32 / width as f32);
    for row in 0..height {
        let line = &buffer[row * info.line_size..][..width * pixel];
        for column in 0..width {
            let first = &line[column * pixel..];
            let value = if bytes == 2 { u16::from_be_bytes([first[0], first[1]]) as f32 } else { f32::from(first[0]) };
            picture.set(column, row, value / maximum);
        }
    }
    Ok(picture)
}

/// Writes `drawing` into `folder`, created when missing: `drawing.toml` and, when it has a picture,
/// `high_ground.png` as 8-bit grey, each value as `round(value * 255)` clamped to 0 to 255. The
/// country is named when it equals a preset; otherwise it is "hills" with every field that differs
/// under `[tune]`. Every water rule that differs from the default is written under `[water]`. The
/// drawing and the rules are validated first.
pub fn write_drawing(folder: &Path, drawing: &Drawing, water: &WaterRules) -> Result<(), LandformError> {
    drawing.validate()?;
    water.validate()?;
    fs::create_dir_all(folder).map_err(|e| io_error(folder, e))?;

    let mut text = String::new();
    let mut line = |row: String| {
        text.push_str(&row);
        text.push('\n');
    };
    line(format!("size_m = {}", drawing.size_m));
    if let Some(picture) = &drawing.high_ground {
        line(format!("high_ground = \"{PICTURE_FILE}\""));
        let bytes: Vec<u8> = picture.data.iter().map(|value| (value * 255.0).round().clamp(0.0, 255.0) as u8).collect();
        let path = folder.join(PICTURE_FILE);
        write_png(&path, picture.width, picture.height, png::ColorType::Grayscale, &bytes)?;
    }
    let preset = COUNTRY_NAMES.into_iter().find(|name| Country::named(name) == Some(drawing.country));
    line(format!("country = \"{}\"", preset.unwrap_or("hills")));
    if preset.is_none() {
        line(String::new());
        line("[tune]".into());
        for ((name, value), (_, hills)) in country_fields(&drawing.country).into_iter().zip(country_fields(&Country::hills())) {
            if value != hills {
                line(format!("{name} = {value}"));
            }
        }
    }
    let changed: Vec<String> = water_fields(water)
        .into_iter()
        .zip(water_fields(&WaterRules::default()))
        .filter(|((_, value), (_, default))| value != default)
        .map(|((name, value), _)| format!("{name} = {value}"))
        .collect();
    if !changed.is_empty() {
        line(String::new());
        line("[water]".into());
        for row in changed {
            line(row);
        }
    }
    for river in &drawing.rivers {
        let points: Vec<String> = river.points.iter().map(|(x, z)| format!("[{x:?}, {z:?}]")).collect();
        line(String::new());
        line("[[rivers]]".into());
        line(format!("width_m = {:?}", river.width_m));
        line(format!("points = [{}]", points.join(", ")));
    }
    for lake in &drawing.lakes {
        line(String::new());
        line("[[lakes]]".into());
        line(format!("x_m = {:?}", lake.x_m));
        line(format!("z_m = {:?}", lake.z_m));
        line(format!("radius_m = {:?}", lake.radius_m));
    }
    let path = folder.join(DRAWING_FILE);
    fs::write(&path, text).map_err(|e| io_error(&path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// An empty folder of this process's own under the system's temporary folder.
    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("landform_drawing_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("the scratch folder is created");
        folder
    }

    fn message(result: Result<(Drawing, WaterRules), LandformError>) -> String {
        result.expect_err("the folder is refused").to_string()
    }

    /// A folder holding `text` as its `drawing.toml`.
    fn folder_with(name: &str, text: &str) -> PathBuf {
        let folder = scratch(name);
        fs::write(folder.join(DRAWING_FILE), text).expect("the file is written");
        folder
    }

    #[test]
    fn a_drawing_survives_the_folder() {
        let folder = scratch("survives");
        let drawing = Drawing::example_valley(2048);
        write_drawing(&folder, &drawing, &WaterRules::default()).expect("the example is written");
        let (back, water) = read_drawing(&folder).expect("the example is read");
        assert_eq!(water, WaterRules::default());
        assert_eq!((back.size_m, back.country), (drawing.size_m, drawing.country));
        assert_eq!(back.rivers, drawing.rivers);
        assert_eq!(back.lakes, drawing.lakes);
        let (before, after) = (drawing.high_ground.as_ref().unwrap(), back.high_ground.as_ref().unwrap());
        assert_eq!((after.width, after.height), (before.width, before.height));
        assert_eq!(after.cell_size, 8.0);
        let worst = before.data.iter().zip(&after.data).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(worst <= 1.0 / 255.0, "the picture moved by {worst}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_example_is_written_as_the_brief_shows() {
        let folder = scratch("layout");
        write_drawing(&folder, &Drawing::example_valley(2048), &WaterRules::default()).unwrap();
        let text = fs::read_to_string(folder.join(DRAWING_FILE)).unwrap();
        assert!(text.starts_with("size_m = 2048\nhigh_ground = \"high_ground.png\"\ncountry = \"hills\"\n"), "{text}");
        assert!(!text.contains("[tune]") && !text.contains("[water]"), "{text}");
        assert!(text.contains("[[rivers]]\nwidth_m = 16.0\npoints = [[-704.0, -1024.0], [-544.0, -624.0]"), "{text}");
        assert!(text.contains("[[lakes]]\nx_m = -80.0\nz_m = -160.0\nradius_m = 112.0\n"), "{text}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_tuned_country_is_written_as_hills_and_its_differences() {
        let folder = scratch("tuned");
        let mut drawing = Drawing::example_valley(512);
        drawing.country = Country { relief_m: 200.0, rounds: 120, cut_strength: 0.0301, ..Country::mountains() };
        write_drawing(&folder, &drawing, &WaterRules::default()).unwrap();
        let text = fs::read_to_string(folder.join(DRAWING_FILE)).unwrap();
        assert!(text.contains("country = \"hills\"") && text.contains("[tune]"), "{text}");
        assert!(text.contains("relief_m = 200.0") && text.contains("rounds = 120"), "{text}");
        assert!(!text.contains("floor_m"), "a field equal to hills's is not written: {text}");
        assert_eq!(read_drawing(&folder).unwrap().0.country, drawing.country);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn tune_overrides_the_named_country() {
        let folder = folder_with("tune", "size_m = 512\ncountry = \"gentle\"\n[tune]\nrelief_m = 200.0\nrounds = 40\n");
        let gentle = Country::gentle();
        let (drawing, _) = read_drawing(&folder).unwrap();
        assert_eq!(drawing.country, Country { relief_m: 200.0, rounds: 40, ..gentle });
        // `--country` replaces the kind and keeps the tune.
        let (drawing, _) = read_drawing_as(&folder, "mountains").unwrap();
        assert_eq!(drawing.country, Country { relief_m: 200.0, rounds: 40, ..Country::mountains() });
        assert_eq!(drawing.high_ground, None);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_without_a_country_grows_hill_country() {
        let folder = folder_with("no_country", "size_m = 512\n[tune]\nrounds = 40\n");
        assert_eq!(read_drawing(&folder).unwrap().0.country, Country { rounds: 40, ..Country::hills() });
        // `--country` still names the kind, and the tune still applies.
        assert_eq!(read_drawing_as(&folder, "gentle").unwrap().0.country, Country { rounds: 40, ..Country::gentle() });
        let bare = folder_with("no_country_bare", "size_m = 512\n");
        assert_eq!(read_drawing(&bare).unwrap().0.country, Country::hills());
        let _ = fs::remove_dir_all(&folder);
        let _ = fs::remove_dir_all(&bare);
    }

    #[test]
    fn a_missing_picture_names_its_file() {
        let folder = folder_with("missing", "size_m = 512\nhigh_ground = \"hills_up.png\"\ncountry = \"hills\"\n");
        let reason = message(read_drawing(&folder));
        assert!(reason.contains("hills_up.png"), "{reason}");
        let reason = message(read_drawing(&folder.join("nowhere")));
        assert!(reason.contains(DRAWING_FILE), "{reason}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_unknown_country_lists_the_three() {
        let folder = folder_with("country", "size_m = 512\ncountry = \"desert\"\n");
        let reason = message(read_drawing(&folder));
        assert!(reason.contains("desert"), "{reason}");
        for name in ["gentle", "hills", "mountains"] {
            assert!(reason.contains(name), "{reason} should list {name}");
        }
        let reason = message(read_drawing_as(&folder_with("country_flag", "size_m = 512\ncountry = \"hills\"\n"), "desert"));
        assert!(reason.contains("mountains"), "{reason}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let cases = [
            ("size_m = 512\ncountry = \"hills\"\nsise_m = 4\n", "sise_m"),
            ("size_m = 512\ncountry = \"hills\"\n[tune]\nrelief = 4.0\n", "relief"),
            ("size_m = 512\ncountry = \"hills\"\n[[rivers]]\nwidth_m = 4.0\npoints = []\ncolour = 1\n", "colour"),
            ("size_m = 512\ncountry = \"hills\"\n[[lakes]]\nx_m = 0.0\nz_m = 0.0\nradius_m = 4.0\ndepth = 1\n", "depth"),
        ];
        for (text, key) in cases {
            let folder = folder_with("unknown", text);
            let reason = message(read_drawing(&folder));
            assert!(reason.contains(key), "{reason} should name {key}");
            let _ = fs::remove_dir_all(&folder);
        }
    }

    #[test]
    fn a_bad_drawing_fails_with_the_cores_sentence() {
        let text = "size_m = 512\ncountry = \"hills\"\n[[lakes]]\nx_m = 0.0\nz_m = 0.0\nradius_m = 300.0\n";
        let folder = folder_with("bad", text);
        let reason = message(read_drawing(&folder));
        assert!(reason.contains("lake 1") && reason.contains("radius_m"), "{reason}");
        let folder_far = folder_with("far", "size_m = 512\ncountry = \"hills\"\n[tune]\nrounds = 0\n");
        assert!(message(read_drawing(&folder_far)).contains("rounds"));
        let _ = fs::remove_dir_all(&folder);
        let _ = fs::remove_dir_all(&folder_far);
    }

    #[test]
    fn a_water_table_overrides_the_defaults() {
        let text = "size_m = 512\ncountry = \"gentle\"\n[water]\nbrook_start_m2 = 90000.0\nbank_run = 2.5\nshore_reach_m = 20.0\n";
        let folder = folder_with("water", text);
        let defaults = WaterRules::default();
        let wanted = WaterRules { brook_start_m2: 90_000.0, bank_run: 2.5, shore_reach_m: 20.0, ..defaults };
        let (drawing, water) = read_drawing(&folder).unwrap();
        assert_eq!((drawing.country, water), (Country::gentle(), wanted));
        // `--country` replaces the kind only; the water rules stay.
        let (other, water) = read_drawing_as(&folder, "mountains").unwrap();
        assert_eq!((other.country, water), (Country::mountains(), wanted));
        // A file with no `[water]` table holds the defaults.
        let bare = folder_with("water_bare", "size_m = 512\ncountry = \"hills\"\n");
        assert_eq!(read_drawing(&bare).unwrap().1, defaults);
        // The rules read are validated: the core's sentence names the field.
        let bad = folder_with("water_bad", "size_m = 512\n[water]\nbank_run = 0.1\n");
        let reason = message(read_drawing(&bad));
        assert!(reason.contains("bank_run") && reason.contains("0.1"), "{reason}");
        for folder in [folder, bare, bad] {
            let _ = fs::remove_dir_all(&folder);
        }
    }

    #[test]
    fn an_unknown_water_key_is_refused() {
        let folder = folder_with("water_unknown", "size_m = 512\ncountry = \"hills\"\n[water]\nbrook_start = 1000.0\n");
        let reason = message(read_drawing(&folder));
        assert!(reason.contains("brook_start"), "{reason}");
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn default_water_rules_are_not_written() {
        let folder = scratch("water_written");
        let drawing = Drawing::example_valley(512);
        write_drawing(&folder, &drawing, &WaterRules::default()).unwrap();
        let text = fs::read_to_string(folder.join(DRAWING_FILE)).unwrap();
        assert!(!text.contains("[water]") && !text.contains("brook_start_m2"), "{text}");
        // Only the fields that differ are, and they read back.
        let rules = WaterRules { brook_width_m: 2.0, bank_wet_reach_m: 60.0, ..WaterRules::default() };
        write_drawing(&folder, &drawing, &rules).unwrap();
        let text = fs::read_to_string(folder.join(DRAWING_FILE)).unwrap();
        assert!(text.contains("[water]\nbrook_width_m = 2.0\nbank_wet_reach_m = 60.0\n"), "{text}");
        assert!(!text.contains("brook_start_m2") && !text.contains("shore_run"), "{text}");
        assert_eq!(read_drawing(&folder).unwrap().1, rules);
        // Rules that fail their check are not written.
        let bad = WaterRules { bank_run: 0.0, ..WaterRules::default() };
        assert!(write_drawing(&folder, &drawing, &bad).unwrap_err().to_string().contains("bank_run"));
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn every_water_rule_survives_the_folder() {
        // Every field away from its default, and no two alike, so a field written under another's
        // name, or read into another, shows.
        let rules = WaterRules {
            brook_start_m2: 75_000.0,
            brook_width_m: 1.5,
            width_area_power: 0.45,
            river_inflow_m2: 2.0e6,
            river_inflow_width_m: 18.0,
            bed_depth_share: 0.11,
            bed_depth_min_m: 0.35,
            bed_depth_max_m: 2.25,
            bank_run: 2.75,
            water_fill: 0.65,
            bank_wet_full_m: 0.25,
            bank_wet_none_m: 1.75,
            bank_wet_reach_m: 45.0,
            shore_run: 3.5,
            shore_reach_m: 14.0,
            width_wander: 0.2,
            wander_length_widths: 7.0,
            bend_shift: 0.4,
        };
        let (written, defaults) = (water_fields(&rules), water_fields(&WaterRules::default()));
        for (k, (name, value)) in written.iter().enumerate() {
            assert_ne!(value, &defaults[k].1, "{name} is its default");
            assert!(written[..k].iter().all(|(_, other)| other != value), "{name} is {value}, as another field is");
        }
        let folder = scratch("water_all");
        write_drawing(&folder, &Drawing::example_valley(512), &rules).unwrap();
        let text = fs::read_to_string(folder.join(DRAWING_FILE)).unwrap();
        let table = text.split("[water]\n").nth(1).unwrap_or_else(|| panic!("no [water] table: {text}"));
        for (name, value) in &written {
            assert!(table.contains(&format!("{name} = {value}\n")), "{name} = {value} is not in the table: {table}");
        }
        assert_eq!(read_drawing(&folder).unwrap().1, rules);
        let _ = fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_picture_outside_the_folder_is_refused() {
        let folder = folder_with("outside", "size_m = 512\nhigh_ground = \"../x.png\"\ncountry = \"hills\"\n");
        assert!(message(read_drawing(&folder)).contains("high_ground"));
        let _ = fs::remove_dir_all(&folder);
    }

    /// Writes a picture of two pixels by one, `samples` per pixel at 8 or 16 bits.
    fn write_pixels(path: &Path, color: png::ColorType, depth: png::BitDepth, data: &[u8]) {
        let mut encoder = png::Encoder::new(File::create(path).unwrap(), 2, 1);
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder.write_header().unwrap().write_image_data(data).unwrap();
    }

    #[test]
    fn pictures_of_other_kinds_read_by_their_first_channel() {
        let folder = folder_with("kinds", "size_m = 512\nhigh_ground = \"p.png\"\ncountry = \"hills\"\n");
        let picture = folder.join("p.png");
        let first_row = |folder: &Path| read_drawing(folder).unwrap().0.high_ground.unwrap().data;
        // Grey with alpha, 8 bit: the alpha is ignored.
        write_pixels(&picture, png::ColorType::GrayscaleAlpha, png::BitDepth::Eight, &[255, 9, 51, 200]);
        assert_eq!(first_row(&folder), [1.0, 51.0 / 255.0]);
        // RGB, 8 bit: the red channel.
        write_pixels(&picture, png::ColorType::Rgb, png::BitDepth::Eight, &[0, 255, 255, 102, 0, 0]);
        assert_eq!(first_row(&folder), [0.0, 102.0 / 255.0]);
        // RGBA, 16 bit: the red channel over 65535.
        let sixteen = [0x80, 0x00, 1, 2, 3, 4, 5, 6, 0xff, 0xff, 0, 0, 0, 0, 0, 0];
        write_pixels(&picture, png::ColorType::Rgba, png::BitDepth::Sixteen, &sixteen);
        assert_eq!(first_row(&folder), [32768.0 / 65535.0, 1.0]);
        // Grey, 16 bit.
        write_pixels(&picture, png::ColorType::Grayscale, png::BitDepth::Sixteen, &[0, 0, 0x40, 0x00]);
        assert_eq!(first_row(&folder), [0.0, 16384.0 / 65535.0]);
        let _ = fs::remove_dir_all(&folder);
    }
}
