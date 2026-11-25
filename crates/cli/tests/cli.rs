use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A uniquely named scratch directory under the system temp directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("landform_cli_{}_{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn landform(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_landform")).args(args).output().expect("the binary runs")
}

fn generate_synthetic(out: &Path) -> Output {
    landform(&[
        "generate",
        "--synthetic",
        "--seed",
        "5",
        "--size",
        "256",
        "--out",
        out.to_str().unwrap(),
    ])
}

#[test]
fn generate_writes_reproducible_files() {
    let scratch = Scratch::new("reproducible");
    let a = scratch.path().join("a");
    let b = scratch.path().join("b");
    for dir in [&a, &b] {
        let output = generate_synthetic(dir);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    let heights_a = fs::read(a.join("heights.r32")).unwrap();
    let heights_b = fs::read(b.join("heights.r32")).unwrap();
    assert_eq!(heights_a.len(), 256 * 256 * 4);
    assert_eq!(heights_a, heights_b);
    assert!(a.join("hillshade.png").is_file());
    for name in ["ground.png", "layer_flow.png", "layer_slope.png", "layer_vigour.png", "layer_wetness.png"] {
        assert!(a.join(name).is_file(), "{name} is missing");
        assert_eq!(fs::read(a.join(name)).unwrap(), fs::read(b.join(name)).unwrap(), "{name} differs between runs");
    }

    let meta: serde_json::Value =
        serde_json::from_slice(&fs::read(a.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["seed"], 5);
    assert_eq!(meta["source_id"], "synthetic");
    assert_eq!(meta["size"], 256);
    assert_eq!(meta["north"].as_array().unwrap().len(), 2);
    assert!(meta["window"]["size_m"].is_number());
    assert!(meta["timings"]["total_ms"].is_number());
    assert!(meta["timings"]["analyse_ms"].is_number());
    assert!(meta["timings"]["cover_ms"].is_number());
    assert_eq!(meta["materials"].as_array().unwrap().len(), 6);
    let cover = meta["cover"].as_object().unwrap();
    assert_eq!(cover.len(), 6);
    let total: f64 = cover.values().map(|v| v.as_f64().unwrap()).sum();
    assert!((total - 1.0).abs() < 1e-6, "cover sums to {total}");
    let stdout = String::from_utf8_lossy(&generate_synthetic(&scratch.path().join("c")).stdout).into_owned();
    assert!(stdout.contains("analyse") && stdout.contains("Cover: meadow_grass "), "{stdout}");
}

#[test]
fn missing_pack_exits_with_a_message() {
    let scratch = Scratch::new("missing_pack");
    let output = landform(&[
        "generate",
        "--seed",
        "1",
        "--pack",
        "no_such.lfpk",
        "--out",
        scratch.path().to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("pack"));
}

#[test]
fn info_prints_the_pack_header() {
    let scratch = Scratch::new("info");
    fs::create_dir_all(scratch.path()).unwrap();
    let pack = scratch.path().join("tiny.lfpk");
    let mut heights: landform_core::Grid<f32> = landform_core::Grid::new(4, 4, 10.0);
    for (i, h) in heights.data.iter_mut().enumerate() {
        *h = 100.0 + i as f32;
    }
    landform_core::write_pack(&pack, "tiny", "Test attribution", &heights).unwrap();

    let output = landform(&["info", pack.to_str().unwrap()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("tiny"));
    assert!(text.contains("Test attribution"));
    assert!(text.contains("4 x 4"));
}

/// Writes a small float32 GeoTIFF covering lon -124..-123, lat 47..48 at 0.01 degree, whose
/// height rises from 1000 m in the south to 3000 m in the north.
fn write_tile(path: &Path) {
    use tiff::encoder::{colortype, TiffEncoder};
    use tiff::tags::Tag;

    let (width, height) = (100u32, 100u32);
    let data: Vec<f32> = (0..height)
        .flat_map(|r| {
            let lat = 48.0 - (f64::from(r) + 0.5) * 0.01;
            std::iter::repeat((1000.0 + 2000.0 * (lat - 47.0)) as f32).take(width as usize)
        })
        .collect();
    let mut encoder = TiffEncoder::new(fs::File::create(path).unwrap()).unwrap();
    let mut image = encoder.new_image::<colortype::Gray32Float>(width, height).unwrap();
    let directory = image.encoder();
    directory.write_tag(Tag::ModelPixelScaleTag, &[0.01f64, 0.01, 0.0][..]).unwrap();
    directory.write_tag(Tag::ModelTiepointTag, &[0.0f64, 0.0, 0.0, -124.0, 48.0, 0.0][..]).unwrap();
    directory.write_tag(Tag::GdalNodata, "-999999").unwrap();
    image.write_data(&data).unwrap();
}

/// Writes a recipe for a 5 km square at 50 m cells around the given longitude.
fn write_recipe(path: &Path, center_lon: f64) {
    let text = format!(
        "id = \"test\"\n\
         attribution = \"Test elevation data\"\n\
         tiles = [\"tile.tif\"]\n\
         projection = \"+proj=utm +zone=10 +ellps=GRS80 +units=m +no_defs\"\n\
         center_lon = {center_lon}\n\
         center_lat = 47.5\n\
         size_m = 5000.0\n\
         cell_size_m = 50.0\n"
    );
    fs::write(path, text).unwrap();
}

#[test]
fn pack_builds_a_pack_from_a_tile() {
    let scratch = Scratch::new("pack_builds");
    let tiles = scratch.path().join("tiles");
    fs::create_dir_all(&tiles).unwrap();
    write_tile(&tiles.join("tile.tif"));
    let recipe = scratch.path().join("test.toml");
    write_recipe(&recipe, -123.5);
    let out = scratch.path().join("made").join("test.lfpk");

    let output = landform(&[
        "pack",
        "--recipe",
        recipe.to_str().unwrap(),
        "--tiles",
        tiles.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("100 x 100") && stdout.contains("0.0% no data"), "{stdout}");

    let info = landform(&["info", out.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("100 x 100") && text.contains("Test elevation data"), "{text}");
    assert!(text.contains("Source id:    test"), "{text}");
}

#[test]
fn pack_names_a_missing_tile() {
    let scratch = Scratch::new("pack_missing_tile");
    let tiles = scratch.path().join("tiles");
    fs::create_dir_all(&tiles).unwrap();
    let recipe = scratch.path().join("test.toml");
    write_recipe(&recipe, -123.5);
    let out = scratch.path().join("test.lfpk");

    let output = landform(&[
        "pack",
        "--recipe",
        recipe.to_str().unwrap(),
        "--tiles",
        tiles.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("tile.tif"));
    assert!(!out.exists());
}

#[test]
fn pack_refuses_a_region_that_is_mostly_missing() {
    let scratch = Scratch::new("pack_mostly_missing");
    let tiles = scratch.path().join("tiles");
    fs::create_dir_all(&tiles).unwrap();
    write_tile(&tiles.join("tile.tif"));
    let recipe = scratch.path().join("test.toml");
    write_recipe(&recipe, -122.0);
    let out = scratch.path().join("test.lfpk");

    let output = landform(&[
        "pack",
        "--recipe",
        recipe.to_str().unwrap(),
        "--tiles",
        tiles.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("100.0% no data"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no data"));
    assert!(!out.exists());
}

fn write_example(folder: &Path, size: &str) {
    let output = landform(&["write-example", "--out", folder.to_str().unwrap(), "--size", size]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

fn generate_drawing(folder: &Path, out: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["generate", "--drawing", folder.to_str().unwrap(), "--seed", "3", "--out", out.to_str().unwrap()];
    args.extend_from_slice(extra);
    landform(&args)
}

#[test]
fn a_drawing_folder_gives_a_world() {
    let scratch = Scratch::new("drawing");
    let folder = scratch.path().join("valley");
    write_example(&folder, "256");
    assert!(folder.join("drawing.toml").is_file() && folder.join("high_ground.png").is_file());

    let (out, again, mountains) = (scratch.path().join("a"), scratch.path().join("b"), scratch.path().join("c"));
    let output = generate_drawing(&folder, &out, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.contains("Wrote a 256 x 256 world") && stdout.contains("Timings: grow "), "{stdout}");
    assert_eq!(fs::read(out.join("heights.r32")).unwrap().len(), 256 * 256 * 4);

    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["source_id"], "drawn");
    assert!(meta["timings"]["grow_ms"].as_f64().unwrap() > 0.0);
    let notes: Vec<&str> = meta["notes"].as_array().unwrap().iter().map(|note| note.as_str().unwrap()).collect();
    assert_eq!(stdout.matches("Note: ").count(), notes.len(), "{stdout}");
    assert_eq!(meta["notes_about"].as_array().unwrap().len(), notes.len(), "{}", meta["notes_about"]);

    // The same folder and seed give the same world; another kind of country gives another.
    assert!(generate_drawing(&folder, &again, &[]).status.success());
    assert_eq!(fs::read(out.join("heights.r32")).unwrap(), fs::read(again.join("heights.r32")).unwrap());
    assert!(generate_drawing(&folder, &mountains, &["--country", "mountains"]).status.success());
    assert_ne!(fs::read(out.join("heights.r32")).unwrap(), fs::read(mountains.join("heights.r32")).unwrap());
}

#[test]
fn a_drawing_refuses_the_flags_that_do_not_go_with_it() {
    let scratch = Scratch::new("drawing_flags");
    let folder = scratch.path().join("valley");
    write_example(&folder, "128");
    let out = scratch.path().join("out");

    let output = generate_drawing(&folder, &out, &["--size", "512"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--size"));
    let output = generate_drawing(&folder, &out, &["--synthetic"]);
    assert_eq!(output.status.code(), Some(2));
    let output = generate_drawing(&folder, &out, &["--country", "desert"]);
    assert_eq!(output.status.code(), Some(2));
    let output = landform(&["generate", "--synthetic", "--seed", "1", "--country", "hills", "--out", out.to_str().unwrap()]);
    assert_eq!(output.status.code(), Some(2));
    assert!(!out.exists());
}

#[test]
fn a_bad_drawing_folder_exits_with_the_sentence() {
    let scratch = Scratch::new("drawing_bad");
    let folder = scratch.path().join("valley");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("drawing.toml"), "size_m = 512
country = \"hills\"
[[lakes]]
x_m = 0.0
z_m = 0.0
radius_m = 300.0
").unwrap();
    let output = generate_drawing(&folder, &scratch.path().join("out"), &[]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("lake 1") && stderr.contains("radius_m"), "{stderr}");
}

/// The committed example drawing, grown at its own size.
fn valley_folder() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../drawings/valley")
}

fn read_png(path: &Path) -> (png::OutputInfo, Vec<u8>) {
    let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap())).read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    buffer.truncate(info.buffer_size());
    (info, buffer)
}

#[test]
fn a_drawn_world_writes_its_water() {
    let scratch = Scratch::new("water");
    let out = scratch.path().join("valley");
    let output = generate_drawing(&valley_folder(), &out, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(stdout.lines().any(|line| line.starts_with("Water: ") && line.contains(" rivers and brooks (")), "{stdout}");

    // The water is a colour picture, not a grey layer: blue where the water lies, land elsewhere.
    assert!(!out.join("layer_water.png").exists());
    let (info, pixels) = read_png(&out.join("water.png"));
    assert_eq!((info.width, info.height, info.color_type), (2048, 2048, png::ColorType::Rgb));
    let land = landform_core::water_rgb(0.0);
    let (mut land_pixels, mut water_pixels) = (0usize, 0usize);
    for pixel in pixels.chunks_exact(3) {
        if pixel == land {
            land_pixels += 1;
        } else {
            assert!(pixel[2] > pixel[0], "a water pixel is blue: {pixel:?}");
            water_pixels += 1;
        }
    }
    assert!(water_pixels > 1000 && land_pixels > water_pixels * 10, "{water_pixels} water, {land_pixels} land");

    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    assert!(meta["timings"]["water_ms"].as_f64().unwrap() > 0.0);

    // The line after the water's gives the surface's pieces, points, triangles and time, as
    // meta.json holds them.
    let lines: Vec<&str> = stdout.lines().collect();
    let water_line = lines.iter().position(|line| line.starts_with("Water: ")).expect("a Water line");
    let surface_line = *lines.get(water_line + 1).unwrap_or_else(|| panic!("no line after the Water line: {stdout}"));
    let words: Vec<&str> = surface_line.split(' ').collect();
    assert!(matches!(words[..], ["Surface:", _, "pieces,", _, "points,", _, "triangles,", _, "ms"]), "{surface_line}");
    let count = |word: &str| word.parse::<u64>().unwrap_or_else(|_| panic!("{word} is not a count"));
    let (pieces, points, triangles) = (count(words[1]), count(words[3]), count(words[5]));
    assert!(pieces > 0 && points > pieces && triangles > 0, "{surface_line}");
    assert_eq!(meta["surface"], serde_json::json!({ "pieces": pieces, "points": points, "triangles": triangles }));
    let surface_ms = meta["timings"]["surface_ms"].as_f64().unwrap();
    assert!(surface_ms > 0.0 && words[7].parse::<f64>().unwrap() > 0.0, "{surface_ms} ms");

    let rivers = meta["water"]["rivers"].as_array().unwrap();
    let drawn = |river: &serde_json::Value| river["drawn"].as_u64();
    // Two drawn rivers, one of which crosses the lake and so comes in two courses.
    assert_eq!(rivers.iter().filter(|river| drawn(river).is_some()).count(), 3, "{rivers:?}");
    assert!(rivers.iter().any(|river| drawn(river).is_none()), "brooks come down the side valleys");
    for river in rivers {
        let (width, end) = (river["width_m"].as_array().unwrap(), river["end"].as_str().unwrap());
        assert!(width[0].as_f64().unwrap() <= width[1].as_f64().unwrap());
        assert!(river["length_m"].as_f64().unwrap() > 0.0 && river["points"].as_u64().unwrap() >= 2);
        assert!(end == "edge" || end.starts_with("river ") || end.starts_with("lake "), "{end}");
    }
    assert!(rivers.iter().any(|river| river["end"] == "lake 0") && rivers.iter().any(|river| river["from_lake"] == 0));
    let lakes = meta["water"]["lakes"].as_array().unwrap();
    assert_eq!(lakes.len(), 1);
    assert_eq!(lakes[0]["drawn"], 0);
    for key in ["level_m", "area_m2", "x_m", "z_m"] {
        assert!(lakes[0][key].is_number(), "{key}");
    }
}

#[test]
fn a_world_without_water_writes_none() {
    let scratch = Scratch::new("no_water");
    let out = scratch.path().join("a");
    let output = generate_synthetic(&out);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!out.join("water.png").exists() && !out.join("layer_water.png").exists());
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(!stdout.contains("Water:") && !stdout.contains("Surface:"), "{stdout}");
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["water"], serde_json::json!({ "rivers": [], "lakes": [] }));
    assert_eq!(meta["timings"]["water_ms"], 0.0);
    // A world cut from data has no water layer, so no surface: null, not left out.
    assert!(meta.as_object().unwrap().contains_key("surface") && meta["surface"].is_null(), "{}", meta["surface"]);
    assert_eq!(meta["timings"]["surface_ms"], 0.0);
}

#[test]
fn a_water_table_reaches_the_generation() {
    let scratch = Scratch::new("water_table");
    let folder = scratch.path().join("valley");
    write_example(&folder, "256");
    let brooks = |out: &Path| {
        let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
        meta["water"]["rivers"].as_array().unwrap().iter().filter(|river| river["drawn"].is_null()).count()
    };
    // Nothing of 256 m drains the 50,000 m2 a brook starts at; a brook that starts at 2,000 m2 is everywhere.
    let (plain, small) = (scratch.path().join("plain"), scratch.path().join("small"));
    assert!(generate_drawing(&folder, &plain, &[]).status.success());
    let text = fs::read_to_string(folder.join("drawing.toml")).unwrap();
    fs::write(folder.join("drawing.toml"), format!("{text}
[water]
brook_start_m2 = 2000.0
")).unwrap();
    assert!(generate_drawing(&folder, &small, &[]).status.success());
    assert_eq!(brooks(&plain), 0);
    assert!(brooks(&small) > 0);
    // A bad rule stops the run with the core's sentence.
    fs::write(folder.join("drawing.toml"), format!("{text}
[water]
bank_run = 0.1
")).unwrap();
    let output = generate_drawing(&folder, &scratch.path().join("bad"), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("bank_run"));
}

#[test]
fn a_drawn_world_says_what_the_ground_does_not_hold() {
    let scratch = Scratch::new("unheld");
    let folder = scratch.path().join("valley");
    write_example(&folder, "256");
    let out = scratch.path().join("out");
    let output = generate_drawing(&folder, &out, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();

    // The line comes right after the Surface line.
    let lines: Vec<&str> = stdout.lines().collect();
    let surface_line = lines.iter().position(|line| line.starts_with("Surface: ")).expect("a Surface line");
    let line = *lines.get(surface_line + 1).unwrap_or_else(|| panic!("no line after the Surface line: {stdout}"));
    assert!(line.starts_with("Unheld: "), "{line}");

    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    let unheld = meta["unheld"].as_object().unwrap_or_else(|| panic!("unheld is an object: {}", meta["unheld"]));
    let keys: Vec<&str> = unheld.keys().map(String::as_str).collect();
    for key in
        ["dry_lake", "dry_course", "dry_over_1m", "worst_dry_m", "worst_dry_at", "lake_over_water", "water_steps", "mouth_steps", "worst_step_m"]
    {
        assert!(keys.contains(&key), "{key} is missing from {keys:?}");
    }
    assert_eq!(unheld.len(), 9, "{keys:?}");
    assert_eq!(unheld["worst_dry_at"].as_array().unwrap().len(), 2);
    let count = |key: &str| unheld[key].as_u64().unwrap_or_else(|| panic!("{key} is a count"));

    // The words give the same numbers as the file.
    let worst = unheld["worst_dry_m"].as_f64().unwrap();
    let expected = format!(
        "Unheld: {} dry cells below the water beside them ({} at a lake, {} at a course, the worst {worst:.2} m), {} lake cells over lower water, {} steps between water cells ({} more at mouths)",
        count("dry_lake") + count("dry_course"),
        count("dry_lake"),
        count("dry_course"),
        count("lake_over_water"),
        count("water_steps"),
        count("mouth_steps"),
    );
    assert_eq!(line, expected);
}

/// The committed trial folder `drawings/<name>`.
fn trial_folder(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../drawings").join(name)
}

/// The committed folder `drawings/wide`, grown in hill country from seed 3, says what the core's
/// test of the counts names for that world (`the_counts_hold_on_the_example_and_the_trial_folders`,
/// `wide-hills-s3-1024` in its `LEFT`), which grows the trial drawing as the folders hold it: no dry
/// cell below the water beside it, 3 lake cells over lower water, no step between water cells
/// outside a mouth.
#[test]
fn the_wide_folder_leaves_what_the_core_counts() {
    let scratch = Scratch::new("wide_folder");
    let out = scratch.path().join("wide");
    let output = generate_drawing(&trial_folder("wide"), &out, &["--country", "hills"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    let unheld = &meta["unheld"];
    let count = |key: &str| unheld[key].as_u64().unwrap_or_else(|| panic!("{key} is a count: {unheld}"));
    let counts = (count("dry_lake"), count("dry_course"), count("lake_over_water"), count("water_steps"));
    assert_eq!(counts, (0, 0, 3, 0), "dry cells at a lake and at a course, lake cells over water, steps outside a mouth: {unheld}");
}

#[test]
fn a_world_cut_from_data_says_nothing_of_unheld_water() {
    let scratch = Scratch::new("no_unheld");
    let out = scratch.path().join("a");
    let output = generate_synthetic(&out);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Unheld:"));
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    assert!(meta.as_object().unwrap().contains_key("unheld") && meta["unheld"].is_null(), "{}", meta["unheld"]);
    // Nor does it note anything, or say what a note is about.
    assert!(meta["notes"].as_array().unwrap().is_empty() && meta["notes_about"].as_array().unwrap().is_empty(), "{meta}");
}

#[test]
fn write_example_writes_a_trial_drawing() {
    let scratch = Scratch::new("trial");
    let folder = scratch.path().join("headjoin");
    let output = landform(&["write-example", "--name", "headjoin", "--out", folder.to_str().unwrap(), "--size", "256"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(folder.join("drawing.toml").is_file() && folder.join("high_ground.png").is_file());
    let text = fs::read_to_string(folder.join("drawing.toml")).unwrap();
    assert!(text.contains("size_m = 256") && text.contains("country = \"hills\""), "{text}");

    // It reads back as the trial drawing: three drawn rivers, no lake.
    let out = scratch.path().join("out");
    let output = generate_drawing(&folder, &out, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    let rivers = meta["water"]["rivers"].as_array().unwrap();
    assert_eq!(rivers.iter().filter(|river| river["drawn"].as_u64().is_some()).count(), 3, "{rivers:?}");
    assert!(meta["water"]["lakes"].as_array().unwrap().is_empty());

    // Another kind of country is written under its name.
    let mountains = scratch.path().join("mountains");
    let output = landform(&["write-example", "--name", "twoout", "--country", "mountains", "--out", mountains.to_str().unwrap(), "--size", "256"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(fs::read_to_string(mountains.join("drawing.toml")).unwrap().contains("country = \"mountains\""));
}

/// `meta.json` says what each note is about beside the notes: the trial drawing `headjoin` at its
/// 1,024 m from seed 3 notes its first river carried on and off its line, its second river not
/// reaching its end, and its first river cut deeper where its third joins it, so the subjects are
/// the drawing's rivers counted from 0, as `drawn` counts them.
#[test]
fn meta_says_what_each_note_is_about() {
    let scratch = Scratch::new("notes_about");
    let folder = scratch.path().join("headjoin");
    let output = landform(&["write-example", "--name", "headjoin", "--out", folder.to_str().unwrap()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let out = scratch.path().join("out");
    let output = generate_drawing(&folder, &out, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&fs::read(out.join("meta.json")).unwrap()).unwrap();
    let strings = |key: &str| -> Vec<String> { meta[key].as_array().unwrap().iter().map(|value| value.as_str().unwrap().to_string()).collect() };
    let (notes, about) = (strings("notes"), strings("notes_about"));
    assert_eq!(about, ["river 0", "river 0", "river 1", "river 0"], "{notes:?}");
    assert!(notes[3].starts_with("River 1 was cut") && notes[3].ends_with("where river 3 joins it."), "{notes:?}");
}

#[test]
fn write_example_refuses_a_name_it_does_not_know() {
    let scratch = Scratch::new("trial_bad");
    let folder = scratch.path().join("nothing");
    let output = landform(&["write-example", "--name", "nothing", "--out", folder.to_str().unwrap()]);
    assert_ne!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stderr).contains("nothing"), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!folder.exists());
    let output = landform(&["write-example", "--country", "desert", "--out", folder.to_str().unwrap()]);
    assert_ne!(output.status.code(), Some(0));
    assert!(!folder.exists());
}

#[test]
fn write_example_help_shows_the_size_it_writes_unless_told() {
    let output = landform(&["write-example", "--help"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let help = String::from_utf8_lossy(&output.stdout).into_owned();
    let size = help.lines().find(|line| line.trim_start().starts_with("--size")).unwrap_or_else(|| panic!("no --size in {help}"));
    assert!(size.contains("[default: 2048 for the valley, 1024 for a trial drawing]"), "{size}");
}

#[test]
fn write_example_keeps_the_valley_as_it_was() {
    // With its defaults the command writes the committed folder, byte for byte.
    let scratch = Scratch::new("valley_again");
    let folder = scratch.path().join("valley");
    let output = landform(&["write-example", "--out", folder.to_str().unwrap()]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    for name in ["drawing.toml", "high_ground.png"] {
        assert_eq!(fs::read(folder.join(name)).unwrap(), fs::read(valley_folder().join(name)).unwrap(), "{name} differs");
    }
}

/// A land file's section as Landform writes it: its tag, its body's length, the FNV-1a 64 of the
/// two and the body, and the body.
fn section(tag: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = tag.to_vec();
    out.extend_from_slice(&(body.len() as u64).to_le_bytes());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in out.iter().chain(body) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    out.extend_from_slice(&hash.to_le_bytes());
    out.extend_from_slice(body);
    out
}

/// `landform land <args>`, each path as text.
fn land(args: &[&str]) -> Output {
    let mut all = vec!["land"];
    all.extend_from_slice(args);
    landform(&all)
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn assert_ran(output: &Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The example drawing written at `size` metres into `folder`, then grown from seed 3 into `file`.
fn new_land(folder: &Path, size: &str, file: &Path) -> String {
    write_example(folder, size);
    assert_ran(&land(&["new", "--drawing", text(folder), "--seed", "3", "--out", text(file)]))
}

#[test]
fn a_land_made_and_edited_with_the_same_folder_is_the_same_file() {
    let scratch = Scratch::new("land_same");
    let (folder, made, edited) = (scratch.path().join("valley"), scratch.path().join("a.lfland"), scratch.path().join("b.lfland"));
    let stdout = new_land(&folder, "256", &made);
    assert!(stdout.contains("Grew a 256 m land from seed 3 in ") && stdout.contains("Wrote a 256 m land to "), "{stdout}");
    let stdout = assert_ran(&land(&["edit", "--land", text(&made), "--drawing", text(&folder), "--out", text(&edited)]));
    assert!(stdout.contains("Outline: 0.00 ha inside, 0.00 ha of it held or shaped by hand") && stdout.contains("Timings: footprint "), "{stdout}");
    assert!(fs::read(&made).unwrap() == fs::read(&edited).unwrap(), "the same drawing again writes the land as it was, byte for byte");

    let info = assert_ran(&land(&["info", "--land", text(&edited)]));
    for line in ["Format:    1, written by Landform ", "Size:      256 m, 256 x 256 cells of 1 m", "Seed:      3", "Country:   hills", "Drawing:   2 rivers, 1 lakes", "Reach:     300 m
", "Held:      0 cells (0.00 %)", "Skipped:   nothing"] {
        assert!(info.contains(line), "{line} is not in {info}");
    }

    // A land made with --country is the same file again when edited with the same --country; edited
    // without it, the folder's hills replace the mountains and the whole land is regrown.
    let (mountains, again, hills) = (scratch.path().join("m.lfland"), scratch.path().join("m2.lfland"), scratch.path().join("h.lfland"));
    assert_ran(&land(&["new", "--drawing", text(&folder), "--seed", "3", "--country", "mountains", "--out", text(&mountains)]));
    let stdout = assert_ran(&land(&["edit", "--land", text(&mountains), "--drawing", text(&folder), "--country", "mountains", "--out", text(&again)]));
    assert!(stdout.contains("Outline: 0.00 ha inside"), "{stdout}");
    assert!(fs::read(&mountains).unwrap() == fs::read(&again).unwrap(), "the same folder and kind again writes the land as it was, byte for byte");
    assert!(assert_ran(&land(&["info", "--land", text(&again)])).contains("Country:   mountains"));
    let stdout = assert_ran(&land(&["edit", "--land", text(&mountains), "--drawing", text(&folder), "--out", text(&hills)]));
    assert!(stdout.contains("Outline: 6.55 ha inside"), "the whole 256 m land: {stdout}");
    assert!(assert_ran(&land(&["info", "--land", text(&hills)])).contains("Country:   hills"));
}

#[test]
fn a_refused_edit_exits_3_and_writes_nothing() {
    // The prototype's held case (i) at 2,048 m: a disc of 60 m whose edge lies 100 m from the brook's
    // point moved 30 m east (the core's edit `a`), which leaves water beside the disc unheld.
    let scratch = Scratch::new("land_refused");
    let (made, held, out) = (scratch.path().join("a.lfland"), scratch.path().join("b.lfland"), scratch.path().join("c.lfland"));
    assert_ran(&land(&["new", "--drawing", text(&valley_folder()), "--seed", "11", "--out", text(&made)]));
    let stdout = assert_ran(&land(&["hold", "--land", text(&made), "--disc", "-64,416,60", "--out", text(&held)]));
    assert!(stdout.contains("Held the 11289 cells of the disc, and 0 cells beyond it"), "{stdout}");
    let edited = scratch.path().join("edited");
    fs::create_dir_all(&edited).unwrap();
    let toml = fs::read_to_string(valley_folder().join("drawing.toml")).unwrap();
    assert!(toml.contains("[96.0, 416.0]"));
    fs::write(edited.join("drawing.toml"), toml.replace("[96.0, 416.0]", "[126.0, 416.0]")).unwrap();
    fs::copy(valley_folder().join("high_ground.png"), edited.join("high_ground.png")).unwrap();

    let output = land(&["edit", "--land", text(&held), "--drawing", text(&edited), "--out", text(&out)]);
    assert_eq!(output.status.code(), Some(3), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.starts_with("River 2's new line runs within 150 m of held ground at (") && stderr.trim_end().ends_with("move the line, or lift the hold there."), "{stderr}");
    assert!(!out.exists() && !scratch.path().join("c.lfland.part").exists(), "a refused edit writes nothing");
    // Without the hold the same edit is taken.
    let stdout = assert_ran(&land(&["edit", "--land", text(&made), "--drawing", text(&edited), "--out", text(&out)]));
    assert!(stdout.starts_with("Outline: ") && !stdout.starts_with("Outline: 0.00 ha") && out.is_file(), "{stdout}");
}

/// Stage 2's Task 1: `land ask` sets the drawing a land asks for, growing nothing, and `land info`
/// says the land waits; asked for the folder it was made from it writes the land's file again byte
/// for byte; a waiting land's file is its file with nothing waiting, byte for byte, with the drawing
/// asked for put just before the marks; `land edit` of the waiting land with the asked folder
/// regrows it and leaves nothing waiting; asked for its folder in another kind of country it waits
/// (a whole-land change); a folder of another size, or of other water rules, exits with code 1 and
/// its sentence, writing nothing.
#[test]
fn a_land_asked_for_another_drawing_waits_until_an_edit_regrows_it() {
    let scratch = Scratch::new("land_ask");
    let (folder, made) = (scratch.path().join("valley"), scratch.path().join("a.lfland"));
    new_land(&folder, "256", &made);
    let info = |file: &Path| assert_ran(&land(&["info", "--land", text(file)]));
    assert!(info(&made).contains("Waiting:   no\n"), "{}", info(&made));

    let same = scratch.path().join("same.lfland");
    let stdout = assert_ran(&land(&["ask", "--land", text(&made), "--drawing", text(&folder), "--out", text(&same)]));
    assert!(stdout.starts_with("Waiting:   no\n") && stdout.contains("Wrote a 256 m land to "), "{stdout}");
    assert!(fs::read(&made).unwrap() == fs::read(&same).unwrap(), "the land's own folder writes the land's file again, byte for byte");

    // The brook's second point moved 8 m east.
    let moved = scratch.path().join("moved");
    write_example(&moved, "256");
    let toml = fs::read_to_string(moved.join("drawing.toml")).unwrap();
    assert_eq!(toml.matches("[12.0, 52.0]").count(), 1, "{toml}");
    fs::write(moved.join("drawing.toml"), toml.replace("[12.0, 52.0]", "[20.0, 52.0]")).unwrap();
    let asked = scratch.path().join("asked.lfland");
    let stdout = assert_ran(&land(&["ask", "--land", text(&made), "--drawing", text(&moved), "--out", text(&asked)]));
    const WAITS: &str = "Waiting:   yes, the asked drawing differs from the one the land was grown from\n";
    assert!(stdout.starts_with(WAITS), "{stdout}");
    let said = info(&asked);
    assert!(said.contains(WAITS) && said.contains("Drawing:   2 rivers, 1 lakes") && said.contains("Skipped:   nothing"), "{said}");
    // The land's file with nothing waiting, the drawing asked for put in it just before the marks
    // (stage 2's ruling R1): the bytes before and after it are the plain file's.
    let (plain, waiting) = (fs::read(&made).unwrap(), fs::read(&asked).unwrap());
    let before = plain.iter().zip(&waiting).take_while(|(a, b)| a == b).count();
    assert!(waiting.len() > plain.len() && waiting.ends_with(&plain[before..]), "one run of bytes put into the plain file");
    assert_eq!((&plain[before..before + 4], &waiting[before..before + 4]), (&b"MARK"[..], &b"ASKD"[..]), "the asked drawing, put where the marks began");

    let edited = scratch.path().join("edited.lfland");
    let stdout = assert_ran(&land(&["edit", "--land", text(&asked), "--drawing", text(&moved), "--out", text(&edited)]));
    assert!(stdout.starts_with("Outline: ") && !stdout.starts_with("Outline: 0.00 ha"), "the asked drawing regrown: {stdout}");
    assert!(info(&edited).contains("Waiting:   no\n"), "{}", info(&edited));

    let gentle = scratch.path().join("gentle.lfland");
    let stdout = assert_ran(&land(&["ask", "--land", text(&made), "--drawing", text(&folder), "--country", "gentle", "--out", text(&gentle)]));
    assert!(stdout.starts_with(WAITS), "{stdout}");
    assert!(info(&gentle).contains("Country:   hills"), "the land's own kind of country until it regrows");

    let out = scratch.path().join("out.lfland");
    let other = scratch.path().join("other");
    write_example(&other, "128");
    let output = land(&["ask", "--land", text(&made), "--drawing", text(&other), "--out", text(&out)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("the drawing is 128 m across and the land 256 m"), "{}", String::from_utf8_lossy(&output.stderr));
    let wet = scratch.path().join("wet");
    write_example(&wet, "256");
    let toml = fs::read_to_string(wet.join("drawing.toml")).unwrap();
    fs::write(wet.join("drawing.toml"), format!("{toml}\n[water]\nbank_run = 2.5\n")).unwrap();
    let output = land(&["ask", "--land", text(&made), "--drawing", text(&wet), "--out", text(&out)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("asks for the water rule bank_run = 2.5, and the land keeps 2.0"), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!out.exists(), "a refused ask writes nothing");
}

/// The heights in a `heights.r32`.
fn read_heights(path: &Path) -> Vec<f32> {
    fs::read(path).unwrap().as_chunks::<4>().0.iter().map(|&four| f32::from_le_bytes(four)).collect()
}

#[test]
fn exporting_a_new_land_gives_generates_pictures_and_its_heights_in_the_models_frame() {
    let scratch = Scratch::new("land_export");
    let (folder, file) = (scratch.path().join("valley"), scratch.path().join("a.lfland"));
    new_land(&folder, "256", &file);
    let (generated, exported) = (scratch.path().join("generated"), scratch.path().join("exported"));
    assert!(generate_drawing(&folder, &generated, &[]).status.success());
    let stdout = assert_ran(&land(&["export", "--land", text(&file), "--out", text(&exported)]));
    assert!(stdout.contains("Wrote a 256 x 256 world to ") && stdout.contains("Marks: 0 cells held, 0 shaped by hand"), "{stdout}");

    for name in ["hillshade.png", "ground.png", "layer_flow.png", "layer_slope.png", "layer_vigour.png", "layer_wetness.png", "water.png"] {
        assert!(fs::read(generated.join(name)).unwrap() == fs::read(exported.join(name)).unwrap(), "{name} differs from generate's");
    }
    let meta = |dir: &Path| -> serde_json::Value { serde_json::from_slice(&fs::read(dir.join("meta.json")).unwrap()).unwrap() };
    let (generated_meta, exported_meta) = (meta(&generated), meta(&exported));
    let base = generated_meta["base_elevation_m"].as_f64().unwrap() as f32;
    // The model's frame has its outlets at 0, so its lowest ground may lie above or below it.
    assert!(base != 0.0 && exported_meta["base_elevation_m"] == 0.0, "{base}");
    let (low, model) = (read_heights(&generated.join("heights.r32")), read_heights(&exported.join("heights.r32")));
    assert_eq!(low.len(), 256 * 256);
    let shifted = model.iter().zip(&low).filter(|(model, low)| (**model - base).to_bits() != low.to_bits()).count();
    assert_eq!(shifted, 0, "every exported height less generate's base elevation is generate's height");
    assert_eq!(exported_meta["notes"], generated_meta["notes"]);
    assert_eq!(exported_meta["seed"], 3);
    for name in ["held.png", "by_hand.png"] {
        let (info, pixels) = read_png(&exported.join(name));
        assert_eq!((info.width, info.height, info.color_type), (256, 256, png::ColorType::Grayscale));
        assert!(pixels.iter().all(|&grey| grey == 0), "{name}: a new land has no marks");
    }
}

#[test]
fn a_disc_held_lifted_and_regrown_is_said_and_seen() {
    let scratch = Scratch::new("land_hold");
    let (folder, file) = (scratch.path().join("valley"), scratch.path().join("a.lfland"));
    new_land(&folder, "256", &file);
    let (held, lake, lifted, regrown) = (scratch.path().join("b.lfland"), scratch.path().join("c.lfland"), scratch.path().join("d.lfland"), scratch.path().join("e.lfland"));

    // A disc of 5 m holds the 81 cells within 5 m of its centre; it meets no lake.
    let stdout = assert_ran(&land(&["hold", "--land", text(&file), "--disc", "60,60,5", "--out", text(&held)]));
    assert!(stdout.starts_with("Held the 81 cells of the disc, and 0 cells beyond it of the lakes whose water it meets, each lake whole; 81 of the land's 65536 cells are held (0.12 %)"), "{stdout}");
    assert!(assert_ran(&land(&["info", "--land", text(&held)])).contains("Held:      81 cells (0.12 %)"));
    let exported = scratch.path().join("exported");
    assert_ran(&land(&["export", "--land", text(&held), "--out", text(&exported)]));
    let (_, pixels) = read_png(&exported.join("held.png"));
    let white: Vec<usize> = (0..pixels.len()).filter(|&cell| pixels[cell] == 255).collect();
    assert_eq!(white.len(), 81);
    assert!(white.iter().all(|&cell| ((cell % 256) as f64 - 188.0).hypot((cell / 256) as f64 - 188.0) <= 5.0), "the disc is white at (60, 60)");

    // A disc on the lake's middle holds all of its water (the example's lake at 256 m lies at (-10, -20)).
    let stdout = assert_ran(&land(&["hold", "--land", text(&held), "--disc", "-10,-20,1", "--out", text(&lake)]));
    let beyond: usize = stdout.split(", and ").nth(1).and_then(|rest| rest.split(' ').next()).and_then(|count| count.parse().ok()).unwrap_or_else(|| panic!("{stdout}"));
    assert!(stdout.starts_with("Held the 5 cells of the disc") && beyond > 100, "{stdout}");

    // Lifting the first disc leaves the lake held.
    let stdout = assert_ran(&land(&["hold", "--land", text(&lake), "--disc", "60,60,5", "--lift", "--out", text(&lifted)]));
    assert!(stdout.starts_with("Lifted the hold on the 81 cells of the disc that were held, and 0 cells beyond it on the lakes"), "{stdout}");
    assert!(assert_ran(&land(&["info", "--land", text(&lifted)])).contains(&format!("Held:      {} cells", 5 + beyond)));
    // A lift is said of the cells it lifted, not of every cell of its disc: a disc of 10 m round
    // the first holds 317 cells, of which 81 were held.
    let wider = scratch.path().join("f.lfland");
    let stdout = assert_ran(&land(&["hold", "--land", text(&held), "--disc", "60,60,10", "--lift", "--out", text(&wider)]));
    assert!(stdout.starts_with("Lifted the hold on the 81 cells of the disc that were held, and 0 cells beyond it on the lakes"), "{stdout}");
    assert!(stdout.contains("; 0 of the land's 65536 cells are held (0.00 %)"), "{stdout}");
    let stdout = assert_ran(&land(&["hold", "--land", text(&file), "--disc", "60,60,10", "--lift", "--out", text(&wider)]));
    assert!(stdout.starts_with("Lifted the hold on the 0 cells of the disc that were held"), "a land with nothing held: {stdout}");

    // Regrowing an area as drawn regrows inside it: the disc of 20 m, 1,257 cells, meeting no lake,
    // is the outline, and every height outside it is as it was, bit for bit.
    let stdout = assert_ran(&land(&["regrow", "--land", text(&file), "--disc", "-80,80,20", "--out", text(&regrown)]));
    assert!(stdout.starts_with("Outline: 0.13 ha inside, 0.00 ha of it held or shaped by hand") && regrown.is_file(), "{stdout}");
    let (before, after) = (scratch.path().join("before"), scratch.path().join("after"));
    assert_ran(&land(&["export", "--land", text(&file), "--out", text(&before)]));
    assert_ran(&land(&["export", "--land", text(&regrown), "--out", text(&after)]));
    let (old, new) = (read_heights(&before.join("heights.r32")), read_heights(&after.join("heights.r32")));
    let in_disc = |cell: usize| ((cell % 256) as f64 - 48.0).hypot((cell / 256) as f64 - 208.0) <= 20.0;
    let outside = (0..256 * 256).filter(|&cell| !in_disc(cell));
    assert_eq!(outside.filter(|&cell| old[cell].to_bits() != new[cell].to_bits()).count(), 0, "a height outside the disc changed");
}

#[test]
fn a_bad_land_file_or_drawing_exits_with_the_sentence() {
    let scratch = Scratch::new("land_bad");
    let (folder, file) = (scratch.path().join("valley"), scratch.path().join("a.lfland"));
    new_land(&folder, "256", &file);
    let out = scratch.path().join("out.lfland");

    // A file cut short, and one that is not a land.
    let cut = scratch.path().join("cut.lfland");
    let bytes = fs::read(&file).unwrap();
    fs::write(&cut, &bytes[..bytes.len() - 10]).unwrap();
    let output = land(&["info", "--land", text(&cut)]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.starts_with(&format!("The land file {} cannot be read: its MARK section (the marks) runs 10 bytes past the end of the file", cut.display())), "{stderr}");
    let output = land(&["export", "--land", text(&folder.join("drawing.toml")), "--out", text(&scratch.path().join("x"))]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("so it is not a land file"));
    assert!(!scratch.path().join("x").exists());

    // A file with one byte changed is refused as damaged, never read as another land.
    let damaged = scratch.path().join("damaged.lfland");
    let mut changed = bytes.clone();
    changed[bytes.len() / 2] ^= 0x01;
    fs::write(&damaged, &changed).unwrap();
    let output = land(&["info", "--land", text(&damaged)]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.starts_with(&format!("The land file {} cannot be read: its ", damaged.display())) && stderr.trim_end().ends_with(" is damaged, since its bytes do not give the checksum written with them."), "{stderr}");

    // A section this Landform does not know is skipped and said; a land written again drops it.
    let later = scratch.path().join("later.lfland");
    let mut more = bytes.clone();
    more.extend_from_slice(&section(*b"ZZZZ", &[1, 2, 3]));
    fs::write(&later, &more).unwrap();
    let info = assert_ran(&land(&["info", "--land", text(&later)]));
    assert!(info.contains("Skipped:   the section ZZZZ, which this Landform does not know; a land written again drops them"), "{info}");
    let written = scratch.path().join("written.lfland");
    assert_ran(&land(&["hold", "--land", text(&later), "--disc", "60,60,5", "--lift", "--out", text(&written)]));
    assert!(assert_ran(&land(&["info", "--land", text(&written)])).contains("Skipped:   nothing"));
    assert!(fs::read(&written).unwrap() == bytes, "the land written again is the land without the part skipped");

    // A drawing of another size, and one whose water rules are not the land's.
    let other = scratch.path().join("other");
    write_example(&other, "128");
    let output = land(&["edit", "--land", text(&file), "--drawing", text(&other), "--out", text(&out)]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("the drawing is 128 m across and the land 256 m"), "{}", String::from_utf8_lossy(&output.stderr));
    let wet = scratch.path().join("wet");
    write_example(&wet, "256");
    let toml = fs::read_to_string(wet.join("drawing.toml")).unwrap();
    fs::write(wet.join("drawing.toml"), format!("{toml}\n[water]\nbank_run = 2.5\n")).unwrap();
    let output = land(&["edit", "--land", text(&file), "--drawing", text(&wet), "--out", text(&out)]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(stderr.contains("asks for the water rule bank_run = 2.5, and the land keeps 2.0; an edit keeps the land's water rules"), "{stderr}");
    assert!(!out.exists());
    // A disc that is not three numbers is refused by the parser.
    assert_eq!(land(&["hold", "--land", text(&file), "--disc", "1,2", "--out", text(&out)]).status.code(), Some(2));
    // A hold whose write fails (its --out a folder) says nothing was held.
    let output = land(&["hold", "--land", text(&file), "--disc", "60,60,5", "--out", text(scratch.path())]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Held"), "{}", String::from_utf8_lossy(&output.stdout));

    // A land whose water a damaged file makes wider than the land, which `water_surface` would lay
    // out in buckets beyond counting (a width of 1e30 m once panicked there), and a drawing whose
    // river is: each exits with its sentence, and none panics.
    let mut damaged = landform_core::Land::read(&file).unwrap();
    damaged.world.water.rivers[0].points[3].width_m = 1e30;
    let wide = scratch.path().join("wide.lfland");
    damaged.write(&wide).unwrap();
    let exported = scratch.path().join("wide_export");
    let output = land(&["export", "--land", text(&wide), "--out", text(&exported)]);
    assert_eq!(output.status.code(), Some(1), "{}", String::from_utf8_lossy(&output.stderr));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.starts_with(&format!("The land file {} cannot be read: the water's course 1 has a point whose water is 1000000000000000000000000000000 m wide, where a course's water is above 0 m and at most the land's 256 m wide.", wide.display())),
        "{stderr}"
    );
    assert!(!exported.exists());
    let brook = scratch.path().join("brook");
    write_example(&brook, "256");
    let toml = fs::read_to_string(brook.join("drawing.toml")).unwrap();
    assert_eq!(toml.matches("width_m = 0.5").count(), 1, "{toml}");
    fs::write(brook.join("drawing.toml"), toml.replace("width_m = 0.5", "width_m = 1e30")).unwrap();
    let output = generate_drawing(&brook, &scratch.path().join("brook_world"), &[]);
    assert_eq!(output.status.code(), Some(1), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("river 2 width_m must be at most 256 m, the drawing's size_m"), "{}", String::from_utf8_lossy(&output.stderr));
}
