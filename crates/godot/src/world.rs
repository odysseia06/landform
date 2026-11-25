use godot::classes::image::Format;
use godot::classes::Image;
use godot::prelude::*;
use rayon::prelude::*;
use std::collections::BTreeMap;

use landform_core::{
    cell_rgb, layer_range, value_grey, water_rgb, About, Grid, GroundCell, LakeWater, Material, RiverCourse, Water, World,
    WorldMeta, DRAWN_SOURCE_ID, LAYER_NAMES, WATER_LAYER,
};

use crate::water::{end_code, index_or_none, water_from, water_state};

/// Why an empty world cannot give what was asked of it.
const EMPTY: &str = "The world is empty: make one with a LandformGenerator.";

/// A finished world: its heights and the layers its config exported as float images, its ground
/// map as a byte image, its rivers, brooks and lakes, and what the generator recorded. A
/// `LandformGenerator` makes real ones, and a `LandformLand` read from a land file holds one, the
/// land as it is; `LandformWorld.new()` is an empty world of size 0 with no images and no water.
/// A land's world stands in the land model's frame: its heights, its rivers' surfaces and its
/// lakes' levels are the land's as it keeps them, its rivers' outlets at 0 and its lowest cell
/// wherever the land puts it, and `get_base_elevation()` is 0; a generated world stands with its
/// lowest cell at 0. Everything that reads a world (Terrain3D's maps, the water's surface, the
/// rivers and lakes) reads either alike. Every image the world hands out or takes in is a copy over the same bytes,
/// so no script can change what the world reads. A world survives an editor reload of the
/// extension: Godot saves its hidden `_state` property before the reload and restores it
/// afterwards.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformWorld {
    /// Every field below as one dictionary, for Godot to carry across an editor reload. Not shown
    /// in the inspector; not part of the API.
    #[var(get = _get_state, set = _set_state, usage_flags = [STORAGE])]
    _state: PhantomVar<VarDictionary>,
    /// `Image.FORMAT_RF`, `size` x `size`; cell `(x, z)` is pixel `(x, z)`. `None` in an empty world.
    heights: Option<Gd<Image>>,
    /// `Image.FORMAT_RGB8`, `size` x `size`: red the strongest material's index, green the
    /// second's, blue the blend. Every index names a material. `None` in an empty world.
    ground: Option<Gd<Image>>,
    /// The exported layers, each `Image.FORMAT_RF` and `size` x `size`, sorted by name; only
    /// names `is_layer_name` accepts.
    layers: BTreeMap<String, Gd<Image>>,
    size: i64,
    seed: i64,
    source_id: GString,
    base_elevation: f32,
    north: Vector2,
    timings: VarDictionary,
    /// What the land engine changed in the drawing or could not honour, one sentence each.
    notes: PackedStringArray,
    /// What each note is about, one for each of `notes` and in its order; empty when a reload
    /// restored a state that had none for them.
    notes_about: Vec<About>,
    /// The rivers, brooks and lakes, their surfaces and levels in the frame of `heights`; empty
    /// unless the world was grown from a drawing. Every end and lake index in it names an item of
    /// its lists.
    water: Water,
}

impl LandformWorld {
    /// Wraps a finished core world, building its images: a generated world, its lowest cell at 0,
    /// or a land's, in the land model's frame (`Frame::Model`), whose heights are taken as they are.
    /// Main thread only.
    pub fn from_world(world: World) -> Gd<Self> {
        let World { heights, layers, ground, meta, notes, notes_about, water } = world;
        let size = heights.width as i64;
        let heights = grid_image(heights);
        let ground = ground_image(ground);
        let layers = layers.into_iter().map(|(name, grid)| (name, grid_image(grid))).collect();
        Self::wrap(size, heights, ground, layers, &meta, &notes, notes_about, water)
    }

    /// Wraps a core world that stays where it is from its parts (`world_parts`, made off the main
    /// thread), as `from_world` wraps one it takes: a land keeps its world whole beside the wrapped
    /// one. Only the images are made here. Main thread only.
    pub(crate) fn of_parts(parts: WorldParts) -> Gd<Self> {
        let WorldParts { width, height, heights, ground, layers, meta, notes, notes_about, water } = parts;
        let layers = layers.into_iter().map(|(name, bytes)| (name, image_from(width, height, Format::RF, bytes))).collect();
        let (heights, ground) = (image_from(width, height, Format::RF, heights), image_from(width, height, Format::RGB8, ground));
        Self::wrap(width as i64, heights, ground, layers, &meta, &notes, notes_about, water)
    }

    /// The world of `size` cells a side from its images and the rest of a core world.
    #[allow(clippy::too_many_arguments)]
    fn wrap(
        size: i64,
        heights: Gd<Image>,
        ground: Gd<Image>,
        layers: BTreeMap<String, Gd<Image>>,
        meta: &WorldMeta,
        notes: &[String],
        notes_about: Vec<About>,
        water: Water,
    ) -> Gd<Self> {
        let t = &meta.timings;
        Gd::from_object(Self {
            _state: PhantomVar::default(),
            heights: Some(heights),
            ground: Some(ground),
            layers,
            size,
            // Shown as the signed integer GDScript passed in.
            seed: meta.seed as i64,
            source_id: GString::from(meta.source_id.as_str()),
            base_elevation: meta.base_elevation_m,
            north: Vector2::new(meta.north.0, meta.north.1),
            timings: vdict! {
                "pick_ms" => t.pick_ms,
                "grow_ms" => t.grow_ms,
                "resample_ms" => t.resample_ms,
                "analyse_ms" => t.analyse_ms,
                "detail_ms" => t.detail_ms,
                "water_ms" => t.water_ms,
                "cover_ms" => t.cover_ms,
                "total_ms" => t.total_ms,
            },
            notes: notes.iter().map(|note| GString::from(note.as_str())).collect(),
            notes_about,
            water,
        })
    }

    /// Side length in cells; 0 in an empty world.
    pub(crate) fn side(&self) -> usize {
        usize::try_from(self.size).unwrap_or(0)
    }

    /// The seed as the core used it.
    pub(crate) fn seed_bits(&self) -> u64 {
        self.seed as u64
    }

    /// The ground map, or why the world has none.
    pub(crate) fn ground(&self) -> Result<&Gd<Image>, String> {
        match &self.ground {
            Some(ground) => Ok(ground),
            None if self.size == 0 => Err(EMPTY.to_string()),
            None => Err("The world has no ground map: generate it again.".to_string()),
        }
    }

    /// The heights, or why the world has none.
    pub(crate) fn heights(&self) -> Result<&Gd<Image>, String> {
        match &self.heights {
            Some(heights) => Ok(heights),
            None if self.size == 0 => Err(EMPTY.to_string()),
            None => Err("The world has no heights: generate it again.".to_string()),
        }
    }

    /// The world's rivers, brooks and lakes.
    pub(crate) fn water(&self) -> &Water {
        &self.water
    }

    /// The layer `name`, or why the world has none by that name.
    pub(crate) fn layer(&self, name: &str) -> Result<&Gd<Image>, String> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(layer);
        }
        Err(if self.size == 0 {
            EMPTY.to_string()
        } else if name == WATER_LAYER && self.source_id != DRAWN_SOURCE_ID {
            format!("The {} world has no water layer: only a world grown from a drawing has water.", self.source_id)
        } else if is_layer_name(name) {
            format!("The world has no {name} layer: name it in its config's export_layers.")
        } else {
            format!("The world has no layer named \"{name}\": the layers are {}.", layer_names_text())
        })
    }

    /// The river or brook at `index`, or `None` after reporting, as an error naming `method` and
    /// the index, that the world has none there.
    fn river(&self, method: &str, index: i64) -> Option<&RiverCourse> {
        let rivers = &self.water.rivers;
        let found = usize::try_from(index).ok().and_then(|at| rivers.get(at));
        or_report(found.ok_or_else(|| out_of_range(method, "river", index, rivers.len())))
    }

    /// The lake at `index`, or `None` after reporting, as an error naming `method` and the index,
    /// that the world has none there.
    fn lake(&self, method: &str, index: i64) -> Option<&LakeWater> {
        let lakes = &self.water.lakes;
        let found = usize::try_from(index).ok().and_then(|at| lakes.get(at));
        or_report(found.ok_or_else(|| out_of_range(method, "lake", index, lakes.len())))
    }
}

/// Whether a world may hold a layer called `name`: one of the core's `LAYER_NAMES`, which every
/// world has, or its `WATER_LAYER`, which a world grown from a drawing has.
pub(crate) fn is_layer_name(name: &str) -> bool {
    LAYER_NAMES.contains(&name) || name == WATER_LAYER
}

/// The names `is_layer_name` accepts, for a message: "flow, slope, vigour, wetness and water".
pub(crate) fn layer_names_text() -> String {
    format!("{} and {WATER_LAYER}", LAYER_NAMES.join(", "))
}

/// Why `method` found no `item` at `index` of a list of `count`, naming both.
fn out_of_range(method: &str, item: &str, index: i64, count: usize) -> String {
    let held = match count {
        0 => "none".to_string(),
        1 => "one, at index 0".to_string(),
        _ => format!("{count}, at indices 0 to {}", count - 1),
    };
    format!("{method}: there is no {item} at index {index}; the world has {held}.")
}

/// The value, or `None` after reporting why there is none as a Godot error.
pub(crate) fn or_report<T>(result: Result<T, String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(reason) => {
            godot_error!("{reason}");
            None
        }
    }
}

/// A side length in cells as Godot's image size.
fn side_i32(cells: usize) -> i32 {
    i32::try_from(cells).expect("the core caps the world size well below i32::MAX")
}

/// `bytes` as an image of `width` x `height` pixels in `format`; the caller sized them to fit.
pub(crate) fn image_from(width: usize, height: usize, format: Format, bytes: Vec<u8>) -> Gd<Image> {
    let bytes = PackedByteArray::from(bytes);
    Image::create_from_data(side_i32(width), side_i32(height), false, format, &bytes)
        .expect("the bytes were sized for the image")
}

/// `image_from` of bytes kept elsewhere, copied once into Godot's array with no copy of them
/// made first (an overlay's picture at 2,048 m is 16 MiB).
pub(crate) fn image_of(width: usize, height: usize, format: Format, bytes: &[u8]) -> Gd<Image> {
    Image::create_from_data(side_i32(width), side_i32(height), false, format, &PackedByteArray::from(bytes))
        .expect("the bytes were sized for the image")
}

/// A float grid's values as an `Image.FORMAT_RF`'s bytes, cell `(x, z)` at pixel `(x, z)`.
fn grid_bytes(grid: &Grid<f32>) -> Vec<u8> {
    grid.data.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/// A float grid as an `Image.FORMAT_RF`, cell `(x, z)` at pixel `(x, z)`. Consumes the grid so it
/// is freed before Godot makes its copies of the bytes.
fn grid_image(grid: Grid<f32>) -> Gd<Image> {
    let (width, height) = (grid.width, grid.height);
    let bytes = grid_bytes(&grid);
    drop(grid);
    image_from(width, height, Format::RF, bytes)
}

/// A core world as plain Rust, ready to be wrapped by `LandformWorld::of_parts`: its images' bytes,
/// `width` x `height` pixels each, and the rest of it, copied. No Godot type: made off the main
/// thread, so that only Godot's images are made on it.
pub(crate) struct WorldParts {
    width: usize,
    height: usize,
    /// The heights as an `Image.FORMAT_RF`'s bytes (`grid_bytes`).
    heights: Vec<u8>,
    /// The ground map as an `Image.FORMAT_RGB8`'s bytes (`ground_bytes`).
    ground: Vec<u8>,
    /// Each layer by name, in the names' order, as an `Image.FORMAT_RF`'s bytes.
    layers: Vec<(String, Vec<u8>)>,
    meta: WorldMeta,
    notes: Vec<String>,
    notes_about: Vec<About>,
    water: Water,
}

/// The parts of `world`, which stays where it is: its grids' bytes, worked out side by side on the
/// threads of the pool the call runs in (rayon's current pool: the extension calls it inside
/// `pool::run`), and the rest copied.
pub(crate) fn world_parts(world: &World) -> WorldParts {
    let World { heights, layers, ground, meta, notes, notes_about, water } = world;
    let ((heights_bytes, ground), layers) = rayon::join(
        || rayon::join(|| grid_bytes(heights), || ground_bytes(ground)),
        || layers.par_iter().map(|(name, grid)| (name.clone(), grid_bytes(grid))).collect(),
    );
    WorldParts {
        width: heights.width,
        height: heights.height,
        heights: heights_bytes,
        ground,
        layers,
        meta: meta.clone(),
        notes: notes.clone(),
        notes_about: notes_about.clone(),
        water: water.clone(),
    }
}

/// The ground map's cells as an `Image.FORMAT_RGB8`'s bytes, cell `(x, z)` at pixel `(x, z)`: red
/// the strongest material's index, green the second's, blue the blend.
fn ground_bytes(ground: &Grid<GroundCell>) -> Vec<u8> {
    ground.data.iter().flat_map(|cell| [cell.strongest, cell.second, cell.blend]).collect()
}

/// The ground map as an `Image.FORMAT_RGB8` (`ground_bytes`). Consumes the grid, as `grid_image`
/// does.
fn ground_image(ground: Grid<GroundCell>) -> Gd<Image> {
    let (width, height) = (ground.width, ground.height);
    let bytes = ground_bytes(&ground);
    drop(ground);
    image_from(width, height, Format::RGB8, bytes)
}

/// `value` as a `T`, or `None` when it is missing or of another type.
pub(crate) fn read<T: FromGodot>(value: Option<Variant>) -> Option<T> {
    value.and_then(|v| v.try_to::<T>().ok())
}

/// `value` as a `T`, or `T::default()` when it is missing or of another type.
fn or_default<T: FromGodot + Default>(value: Option<Variant>) -> T {
    read(value).unwrap_or_default()
}

/// A new image over `image`'s bytes, or `None` for an empty image. Godot shares the bytes until
/// one of the two images is changed, so nothing is copied here, and a change to either image
/// leaves the other as it was. A world hands out and takes in only such copies, so no script can
/// change the images the world reads.
pub(crate) fn copy_of(image: &Gd<Image>) -> Option<Gd<Image>> {
    if image.is_empty() {
        return None;
    }
    let (width, height, format) = (image.get_width(), image.get_height(), image.get_format());
    Image::create_from_data(width, height, image.has_mipmaps(), format, &image.get_data())
}

/// Whether `image` is `size` x `size` in `format` without mipmaps, as a world of that size
/// builds it.
pub(crate) fn fits(image: &Gd<Image>, format: Format, size: i64) -> bool {
    size > 0
        && image.get_format() == format
        && !image.has_mipmaps()
        && i64::from(image.get_width()) == size
        && i64::from(image.get_height()) == size
}

/// The subjects `_get_state` saved, read back: one for each of the world's `notes` notes, each in
/// the form `About::parse` reads; none when they are not as many or one is not in that form.
fn subjects_from(saved: &PackedStringArray, notes: usize) -> Vec<About> {
    let subjects: Option<Vec<About>> = saved.as_slice().iter().map(|subject| About::parse(&subject.to_string())).collect();
    subjects.filter(|subjects| subjects.len() == notes).unwrap_or_default()
}

/// Whether every index in a ground image's bytes names a material.
fn names_materials(ground: &Gd<Image>) -> bool {
    let materials = Material::ALL.len();
    let data = ground.get_data();
    let cells = data.as_slice().as_chunks::<3>().0;
    cells.iter().all(|&[strongest, second, _]| usize::from(strongest) < materials && usize::from(second) < materials)
}

#[godot_api]
impl LandformWorld {
    /// `get_river_end`: the river reaches the world's edge; `get_river_end_index` gives -1.
    #[constant]
    pub const RIVER_ENDS_AT_EDGE: i64 = 0;
    /// `get_river_end`: the river joins another river or brook, whose index `get_river_end_index`
    /// gives.
    #[constant]
    pub const RIVER_ENDS_IN_RIVER: i64 = 1;
    /// `get_river_end`: the river ends in a lake, whose index `get_river_end_index` gives.
    #[constant]
    pub const RIVER_ENDS_IN_LAKE: i64 = 2;

    /// Heights in metres: in a generated world above its lowest cell, which is `0.0`; in a land's
    /// world (`LandformLand.get_world()`) in the land model's frame, its rivers' outlets at 0, so a
    /// cell may lie below 0. `null` in an empty world. A new image on every call, so changing it
    /// leaves the world as it was.
    #[func]
    fn get_height_image(&self) -> Option<Gd<Image>> {
        self.heights.as_ref().and_then(copy_of)
    }

    /// The ground map as an `Image.FORMAT_RGB8`, `size` x `size`, cell `(x, z)` at pixel `(x, z)`:
    /// red the strongest material's index into `get_material_names()`, green the second's, blue
    /// the blend, the second's share times 255, at most 127. `null` in an empty world. A new image
    /// on every call, so changing it leaves the world as it was.
    #[func]
    fn get_ground_image(&self) -> Option<Gd<Image>> {
        self.ground.as_ref().and_then(copy_of)
    }

    /// The materials' names in index order, the indices the ground image holds; the same six in
    /// every world, an empty one too.
    #[func]
    fn get_material_names(&self) -> PackedStringArray {
        Material::ALL.iter().map(|material| GString::from(material.name())).collect()
    }

    /// The ground map in false colour as an `Image.FORMAT_RGB8`: each cell's two materials'
    /// colours mixed by its blend, the pixels of the command line's `ground.png`. Reports an error
    /// and returns `null` in an empty world.
    #[func]
    fn get_ground_preview_image(&self) -> Option<Gd<Image>> {
        let ground = or_report(self.ground())?;
        let data = ground.get_data();
        let cells = data.as_slice().as_chunks::<3>().0;
        let rgb: Vec<u8> = cells.iter().flat_map(|&[strongest, second, blend]| cell_rgb(GroundCell { strongest, second, blend })).collect();
        Some(image_from(self.side(), self.side(), Format::RGB8, rgb))
    }

    /// One layer as a picture, an `Image.FORMAT_RGB8`: the `water` layer coloured by depth, the
    /// pixels of the command line's `water.png` (dry land a pale sand colour, water from a light
    /// blue just above 0 m to a dark blue at 2 m and deeper); any other layer in grey over its
    /// value range, the pixels of the command line's `layer_<name>.png`. Reports an error and
    /// returns `null` for a layer the world does not have.
    #[func]
    fn get_layer_preview_image(&self, name: GString) -> Option<Gd<Image>> {
        let name = name.to_string();
        let layer = or_report(self.layer(&name))?;
        let data = layer.get_data();
        let values = data.as_slice().as_chunks::<4>().0.iter().map(|&value| f32::from_ne_bytes(value));
        let rgb: Vec<u8> = if name == WATER_LAYER {
            values.flat_map(water_rgb).collect()
        } else {
            let range = layer_range(&name).expect("a world holds no layer but the water outside LAYER_NAMES");
            values.flat_map(|value| [value_grey(value, range); 3]).collect()
        };
        Some(image_from(self.side(), self.side(), Format::RGB8, rgb))
    }

    /// The names of the layers the world's config exported, sorted; `water` among them only in a
    /// world grown from a drawing.
    #[func]
    fn get_layer_names(&self) -> PackedStringArray {
        self.layers.keys().map(|name| GString::from(name.as_str())).collect()
    }

    /// One layer as an `Image.FORMAT_RF`, `size` x `size`, cell `(x, z)` at pixel `(x, z)`: `flow`
    /// as `log10` of the drained area in square metres, `slope` in degrees, `vigour` and `wetness`
    /// from 0 to 1, `water` the water's depth in metres, 0 on dry land: it marks only water that
    /// is reached and held, the cells inside a bed below its surface, the lakes at their level, and
    /// beyond the beds only what the ground closes in, no higher than the rim it would cross to get
    /// out. Reports an error and returns `null` for a layer the world does not have. A new
    /// image on every call, so changing it leaves the world as it was.
    #[func]
    fn get_layer_image(&self, name: GString) -> Option<Gd<Image>> {
        or_report(self.layer(&name.to_string())).and_then(copy_of)
    }

    /// Side length in cells; 0 in an empty world.
    #[func]
    fn get_size(&self) -> i64 {
        self.size
    }

    /// The seed the world was made from, as the signed integer GDScript passed in.
    #[func]
    fn get_seed(&self) -> i64 {
        self.seed
    }

    /// Which source the world was cut from.
    #[func]
    fn get_source_id(&self) -> GString {
        self.source_id.clone()
    }

    /// Elevation of the lowest cell above the source's datum, in metres: for a world grown from a
    /// drawing, its lowest cell's height in the land model's frame; 0 for a land's world, whose
    /// heights stand in that frame.
    #[func]
    fn get_base_elevation(&self) -> f32 {
        self.base_elevation
    }

    /// Unit vector in world `(x, z)` pointing to real north.
    #[func]
    fn get_north(&self) -> Vector2 {
        self.north
    }

    /// Wall-clock milliseconds per stage: `pick_ms` (0 for a world grown from a drawing), `grow_ms`
    /// (0 for a world cut from a source), `resample_ms`, `analyse_ms`, `detail_ms`, `water_ms` (0
    /// for a world cut from a source), `cover_ms`, `total_ms`.
    #[func]
    fn get_timings(&self) -> VarDictionary {
        self.timings.duplicate_shallow()
    }

    /// What the land engine changed in the drawing or could not honour, one sentence each; empty
    /// for a world cut from a source.
    #[func]
    fn get_notes(&self) -> PackedStringArray {
        self.notes.clone()
    }

    /// What each of `get_notes()` is about, one for each and in its order: "land", "river N" or
    /// "lake N", N the index in the drawing's rivers or lakes counted from 0, as
    /// `get_river_drawn` and `get_lake_drawn` count (the notes' sentences count from 1, so "Lake 2
    /// was lowered ..." is about "lake 1"). A note that names two rivers is about the first it
    /// names; one that names no river or lake, a note about brooks, is about the land. The same
    /// words as the command line's `notes_about` in `meta.json`. Empty for a world cut from a
    /// source, and for a world a reload restored from a state that had no subjects for its notes.
    #[func]
    fn get_note_subjects(&self) -> PackedStringArray {
        self.notes_about.iter().map(|about| GString::from(about.to_string().as_str())).collect()
    }

    /// Height in metres of cell `(x, z)`; reports an error and returns `NAN` outside the world.
    #[func]
    fn get_height_at_cell(&self, x: i64, z: i64) -> f32 {
        let inside = |v: i64| (0..self.size).contains(&v);
        match &self.heights {
            Some(heights) if inside(x) && inside(z) => heights.get_pixel(x as i32, z as i32).r,
            _ => {
                godot_error!("Cell ({x}, {z}) lies outside the {0} x {0} world.", self.size);
                f32::NAN
            }
        }
    }

    /// How many rivers and brooks the world has, indexed from 0: the drawn rivers first, in the
    /// drawing's order and each from upstream (a drawn river that crosses a lake is two, one into
    /// the lake and one out of it), then the brooks, those draining most where they end first. 0
    /// unless the world was grown from a drawing.
    #[func]
    fn get_river_count(&self) -> i64 {
        self.water.rivers.len() as i64
    }

    /// The line of river `index`, upstream first, its points at most 4 m apart: `(x, surface, z)`,
    /// `x` and `z` in world metres from the world's centre, `x` east and `z` south, on the world's
    /// cells, and `surface` the water's height in the frame of the heights, which never rises
    /// downstream. Reports an error and returns an empty array for an index out of range.
    #[func]
    fn get_river_points(&self, index: i64) -> PackedVector3Array {
        let Some(river) = self.river("get_river_points", index) else {
            return PackedVector3Array::new();
        };
        river.points.iter().map(|point| Vector3::new(point.x_m, point.surface_m, point.z_m)).collect()
    }

    /// The water's width in metres, from waterline to waterline, at each of river `index`'s
    /// points. It is the width after the wander: along a river it runs from 0.75 to 1.25 of the
    /// drawn width at the shipped `width_wander`, wide stretches and narrow ones in turn. The
    /// width is returned as it is, however narrow; on the 1 m ground the bed is cut at least about
    /// 1.41 m wide whatever it says. Reports an error and returns an empty array for an index out
    /// of range.
    #[func]
    fn get_river_widths(&self, index: i64) -> PackedFloat32Array {
        let Some(river) = self.river("get_river_widths", index) else {
            return PackedFloat32Array::new();
        };
        river.points.iter().map(|point| point.width_m).collect()
    }

    /// The water's depth in metres at the deep line at each of river `index`'s points, which is
    /// the water rules' `water_fill` times the bed's depth there; the bed lies that far below the
    /// surface at the deep line, and the water shallows from it to nothing at both waterlines. A
    /// wide stretch is shallow and a narrow one deep, and the depth grows from 0 over a spring's
    /// first 12 m. The deep line lies off the course's line toward the outside of a bend. The bed
    /// may rise downstream under the water, in pools and shallows; the surface in `get_river_points`
    /// may not. Reports an error and returns an empty array for an index out of range.
    #[func]
    fn get_river_depths(&self, index: i64) -> PackedFloat32Array {
        let Some(river) = self.river("get_river_depths", index) else {
            return PackedFloat32Array::new();
        };
        river.points.iter().map(|point| point.depth_m).collect()
    }

    /// The index in the drawing's rivers of the drawn river that river `index` is part of, or -1
    /// for a brook. Reports an error and returns -1 for an index out of range.
    #[func]
    fn get_river_drawn(&self, index: i64) -> i64 {
        self.river("get_river_drawn", index).map_or(-1, |river| index_or_none(river.drawn))
    }

    /// The index of the lake river `index` flows out of, or -1 for none. Reports an error and
    /// returns -1 for an index out of range.
    #[func]
    fn get_river_from_lake(&self, index: i64) -> i64 {
        self.river("get_river_from_lake", index).map_or(-1, |river| index_or_none(river.from_lake))
    }

    /// How river `index` ends: `RIVER_ENDS_AT_EDGE`, `RIVER_ENDS_IN_RIVER` or `RIVER_ENDS_IN_LAKE`.
    /// Reports an error and returns -1 for an index out of range.
    #[func]
    fn get_river_end(&self, index: i64) -> i64 {
        self.river("get_river_end", index).map_or(-1, |river| end_code(river.end).0)
    }

    /// The index of the river or lake that river `index` ends in, as `get_river_end` says, or -1
    /// when it ends at the world's edge. Reports an error and returns -1 for an index out of range.
    #[func]
    fn get_river_end_index(&self, index: i64) -> i64 {
        self.river("get_river_end_index", index).map_or(-1, |river| end_code(river.end).1)
    }

    /// How many lakes the world has, indexed from 0: the drawn lakes that hold water, in the
    /// drawing's order. 0 unless the world was grown from a drawing.
    #[func]
    fn get_lake_count(&self) -> i64 {
        self.water.lakes.len() as i64
    }

    /// The water level of lake `index` in metres, in the frame of the heights. Reports an error and
    /// returns 0 for an index out of range.
    #[func]
    fn get_lake_level(&self, index: i64) -> f32 {
        self.lake("get_lake_level", index).map_or(0.0, |lake| lake.level_m)
    }

    /// The mean position of lake `index`'s water, `(x, z)` in world metres from the world's centre,
    /// `x` east and `z` south. Reports an error and returns `(0, 0)` for an index out of range.
    #[func]
    fn get_lake_position(&self, index: i64) -> Vector2 {
        self.lake("get_lake_position", index).map_or(Vector2::ZERO, |lake| Vector2::new(lake.x_m, lake.z_m))
    }

    /// The area of lake `index`'s water in square metres. Reports an error and returns 0 for an
    /// index out of range.
    #[func]
    fn get_lake_area(&self, index: i64) -> f32 {
        self.lake("get_lake_area", index).map_or(0.0, |lake| lake.area_m2)
    }

    /// The index in the drawing's lakes of lake `index`. Reports an error and returns -1 for an
    /// index out of range.
    #[func]
    fn get_lake_drawn(&self, index: i64) -> i64 {
        self.lake("get_lake_drawn", index).map_or(-1, |lake| lake.drawn as i64)
    }

    /// The world as one dictionary, its images as copies and its water as plain numbers; Godot
    /// reads it before an editor reload.
    #[func]
    fn _get_state(&self) -> VarDictionary {
        let mut layers = VarDictionary::new();
        for (name, image) in &self.layers {
            if let Some(copy) = copy_of(image) {
                layers.set(&GString::from(name.as_str()), &copy);
            }
        }
        let mut state = vdict! {
            "size" => self.size,
            "seed" => self.seed,
            "source_id" => &self.source_id,
            "base_elevation" => self.base_elevation,
            "north" => self.north,
            "timings" => &self.timings,
            "notes" => &self.notes,
            "note_subjects" => &self.get_note_subjects(),
            "layers" => &layers,
            "water" => &water_state(&self.water),
        };
        if let Some(heights) = self.heights.as_ref().and_then(copy_of) {
            state.set("heights", &heights);
        }
        if let Some(ground) = self.ground.as_ref().and_then(copy_of) {
            state.set("ground", &ground);
        }
        state
    }

    /// Restores the world from `_get_state`'s dictionary after an editor reload, keeping copies of
    /// its images, so changing the dictionary's images afterwards leaves the world as it was.
    /// Entries that are missing or of the wrong type become empty values; the size always matches
    /// the height image. An image that is not the format and size this world would build, a layer
    /// under a name no layer has and a ground map holding an index of no material are left out. A
    /// water entry that is missing (a state saved before worlds had water) or wrong in any part
    /// restores as no water. The notes' subjects restore only as one for each note, each in the
    /// form `get_note_subjects` gives; missing (a state saved before notes had subjects) or wrong
    /// in any part, they restore as none, and the notes as they were.
    #[func]
    fn _set_state(&mut self, state: VarDictionary) {
        let image = |key: &str| or_default::<Option<Gd<Image>>>(state.get(key)).as_ref().and_then(copy_of);
        self.heights = image("heights").filter(|heights| fits(heights, Format::RF, i64::from(heights.get_width())));
        let size = self.heights.as_ref().map_or(0, |heights| i64::from(heights.get_width()));
        self.size = size;
        self.ground = image("ground").filter(|ground| fits(ground, Format::RGB8, size) && names_materials(ground));
        self.layers = or_default::<VarDictionary>(state.get("layers"))
            .iter_shared()
            .filter_map(|(name, image)| Some((name.try_to::<GString>().ok()?.to_string(), copy_of(&image.try_to::<Gd<Image>>().ok()?)?)))
            .filter(|(name, image)| is_layer_name(name) && fits(image, Format::RF, size))
            .collect();
        self.seed = or_default(state.get("seed"));
        self.source_id = or_default(state.get("source_id"));
        self.base_elevation = or_default(state.get("base_elevation"));
        self.north = or_default(state.get("north"));
        self.timings = or_default(state.get("timings"));
        self.notes = or_default(state.get("notes"));
        self.notes_about = subjects_from(&or_default(state.get("note_subjects")), self.notes.len());
        self.water = read::<VarDictionary>(state.get("water")).and_then(|water| water_from(&water)).unwrap_or_default();
    }
}
