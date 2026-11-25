//! A drawing as Godot resources: the land's size, a picture of high ground, rivers, lakes and the
//! kind of country, each mirroring a struct of the core.

use godot::classes::image::Format;
use godot::classes::{ITexture2D, Image, Texture2D};
use godot::global::Error;
use godot::prelude::*;
use landform_core::{
    Country, Drawing, GenConfig, Grid, Lake, River, COUNTRY_NAMES, MAX_ROUNDS, MAX_SIZE_CELLS, MIN_ROUNDS, MIN_SIZE_CELLS,
};

use crate::world::image_from;

/// The width a new river starts at, in metres.
const RIVER_WIDTH_M: f64 = 8.0;
/// The radius a new lake starts at, in metres.
const LAKE_RADIUS_M: f64 = 60.0;

/// One number of the hill country, the core's default kind, as the float Godot shows.
fn default_of(field: fn(&Country) -> f32) -> f64 {
    f64::from(field(&Country::default()))
}

/// The numbers of a kind of country: one per field of the core's `Country`, under the same name,
/// each starting at the hill country's value, the core's default. Heights and distances are in
/// metres, areas in square metres; "per round" numbers act once in each round of the model.
/// `LandformCountry.named("mountains")` gives one of the core's kinds. A `LandformDrawing` without
/// one grows hill country.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformCountry {
    /// Height in metres of black ground above the water's ways out, before the rain carves it.
    #[export]
    #[init(val = default_of(|c| c.floor_m))]
    floor_m: f64,
    /// How far white ground stands above black before the rain carves it, in metres.
    #[export]
    #[init(val = default_of(|c| c.relief_m))]
    relief_m: f64,
    /// Amplitude in metres of the rough noise laid over the picture before the first round.
    #[export]
    #[init(val = default_of(|c| c.rough_m))]
    rough_m: f64,
    /// Largest wavelength of the rough noise, in metres.
    #[export]
    #[init(val = default_of(|c| c.rough_size_m))]
    rough_size_m: f64,
    /// How far black ground rises per round, in metres.
    #[export]
    #[init(val = default_of(|c| c.lift_low_m))]
    lift_low_m: f64,
    /// How much further than black ground white ground rises per round, in metres.
    #[export]
    #[init(val = default_of(|c| c.lift_high_m))]
    lift_high_m: f64,
    /// How hard running water cuts: the cut factor where 1 m² of ground drains over a link of
    /// 1 m. Higher digs deeper valleys.
    #[export]
    #[init(val = default_of(|c| c.cut_strength))]
    cut_strength: f64,
    /// The power of the ground drained in the cut factor: higher lets rivers cut far more than
    /// small streams.
    #[export]
    #[init(val = default_of(|c| c.cut_area_power))]
    cut_area_power: f64,
    /// How much the slopes slump per round, in square metres: the sum over the smoothing passes of
    /// their strength times the cell's area over 4. Higher gives softer, rounder hills.
    #[export]
    #[init(val = default_of(|c| c.slump_m2))]
    slump_m2: f64,
    /// How many rounds of lifting, cutting and slumping the land goes through.
    #[export]
    #[init(val = i64::from(Country::default().rounds))]
    rounds: i64,
    /// How much the rock's hardness varies: 0 makes it the same everywhere; otherwise the cut
    /// strength varies by plus or minus this share.
    #[export]
    #[init(val = default_of(|c| c.hardness_variation))]
    hardness_variation: f64,
    /// Size in metres of the patches of harder and softer rock.
    #[export]
    #[init(val = default_of(|c| c.hardness_patch_m))]
    hardness_patch_m: f64,
}

impl LandformCountry {
    /// A new resource holding `country`'s numbers. Every field is named, so a field added to the
    /// core fails to compile until it is added here.
    fn from_core(country: Country) -> Gd<Self> {
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
        } = country;
        Gd::from_object(Self {
            floor_m: f64::from(floor_m),
            relief_m: f64::from(relief_m),
            rough_m: f64::from(rough_m),
            rough_size_m: f64::from(rough_size_m),
            lift_low_m: f64::from(lift_low_m),
            lift_high_m: f64::from(lift_high_m),
            cut_strength: f64::from(cut_strength),
            cut_area_power: f64::from(cut_area_power),
            slump_m2: f64::from(slump_m2),
            rounds: i64::from(rounds),
            hardness_variation: f64::from(hardness_variation),
            hardness_patch_m: f64::from(hardness_patch_m),
        })
    }

    /// The numbers as the core's `Country`, or the message of its `validate` when they cannot
    /// work. Every field is named, with no default to fill the rest, so a field added to the core
    /// fails to compile until it is added here.
    pub fn to_core(&self) -> Result<Country, String> {
        let rounds = u32::try_from(self.rounds)
            .map_err(|_| format!("rounds must be a whole number from {MIN_ROUNDS} to {MAX_ROUNDS}, not {}.", self.rounds))?;
        // Godot's floats have 64 bits and the core's 32: a value beyond their range becomes
        // infinite, which `validate` refuses.
        let country = Country {
            floor_m: self.floor_m as f32,
            relief_m: self.relief_m as f32,
            rough_m: self.rough_m as f32,
            rough_size_m: self.rough_size_m as f32,
            lift_low_m: self.lift_low_m as f32,
            lift_high_m: self.lift_high_m as f32,
            cut_strength: self.cut_strength as f32,
            cut_area_power: self.cut_area_power as f32,
            slump_m2: self.slump_m2 as f32,
            rounds,
            hardness_variation: self.hardness_variation as f32,
            hardness_patch_m: self.hardness_patch_m as f32,
        };
        country.validate().map_err(|e| e.to_string())?;
        Ok(country)
    }
}

#[godot_api]
impl LandformCountry {
    /// A new resource holding the core's kind of country called `name`, one of `get_names()`.
    /// Reports an error and returns `null` for any other name.
    #[func]
    fn named(name: GString) -> Option<Gd<LandformCountry>> {
        let name = name.to_string();
        let country = Country::named(&name).map(Self::from_core);
        if country.is_none() {
            godot_error!("There is no country called \"{name}\": the countries are {}.", COUNTRY_NAMES.join(", "));
        }
        country
    }

    /// The names `named` knows, gentlest first.
    #[func]
    fn get_names() -> PackedStringArray {
        COUNTRY_NAMES.iter().map(|&name| GString::from(name)).collect()
    }
}

/// A river drawn as a line, upstream first.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformRiver {
    /// The river's line, upstream first: two numbers a point, its east then its south in world
    /// metres (`x` then `z`, `(0, 0)` the world's centre), each kept as the 64-bit float it is, as
    /// the core keeps it. Two points or more, none further than half the drawing's size from the
    /// centre on either axis. A river that ends inside the land on no other river and in no lake is
    /// carried on to the edge: `LandformDrawing.rivers` gives the rule.
    ///
    /// In memory, in a land file and through the core they are exact. A drawing saved as a text
    /// resource (`.tres`) keeps a number exactly only when Godot's text for it reads back as it: a
    /// number that equals no 32-bit float and whose shortest text has at most 15 significant
    /// digits (`12.3`, `-153.6`), or a 32-bit float that its short text gives exactly (a whole
    /// number, a half). Godot writes a 64-bit float that equals a 32-bit float as the 32-bit
    /// float's short text, and reads a longer text without rounding it exactly: of 100,000 random
    /// floats from -1,024 to 1,024 saved and read, 11,175 came back one unit in the last place
    /// away, and of 100,000 that equal 32-bit floats, 99,850 (Godot 4.8, measured by the review of
    /// the 64-bit drawing). So a land's drawing is kept in its land file, never in a `.tres`.
    #[export]
    points_m: PackedFloat64Array,
    /// How wide the river is, in metres; above 0 and at most the drawing's `size_m`.
    #[export]
    #[init(val = RIVER_WIDTH_M)]
    width_m: f64,
}

impl LandformRiver {
    /// The river as the core's `River`, each point as it is, or why its numbers are no points: an
    /// odd count of them. `index` counts from 0, the message from 1, as the core numbers rivers.
    /// `Drawing::validate` checks the rest.
    fn to_core(&self, index: usize) -> Result<River, String> {
        let (pairs, rest) = self.points_m.as_slice().as_chunks::<2>();
        if !rest.is_empty() {
            let count = self.points_m.len();
            return Err(format!(
                "The drawing's river {}'s points_m holds {count} {}: give two for each point, its east then its south.",
                index + 1,
                if count == 1 { "number" } else { "numbers" }
            ));
        }
        Ok(River { points: pairs.iter().map(|&[x, z]| (x, z)).collect(), width_m: self.width_m as f32 })
    }

    /// A new resource holding `river`, each point as the core holds it.
    fn from_core(river: &River) -> Gd<Self> {
        let River { points, width_m } = river;
        let points_m = points.iter().flat_map(|&(x, z)| [x, z]).collect();
        Gd::from_object(Self { points_m, width_m: f64::from(*width_m) })
    }
}

/// A lake drawn as a disc.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformLake {
    /// The east of the disc's centre in world metres, `0` the world's centre; inside the world.
    /// A 64-bit float, as the core keeps it; a `.tres` keeps it exactly only when its text is short,
    /// as `LandformRiver.points_m` says.
    #[export]
    x_m: f64,
    /// The south of the disc's centre in world metres, `0` the world's centre (north is `-z`);
    /// inside the world. A 64-bit float, as the core keeps it; a `.tres` keeps it exactly only when
    /// its text is short, as `LandformRiver.points_m` says.
    #[export]
    z_m: f64,
    /// The disc's radius in metres; above 0 and at most half the drawing's size.
    #[export]
    #[init(val = LAKE_RADIUS_M)]
    radius_m: f64,
}

impl LandformLake {
    /// The lake as the core's `Lake`, its centre as it is; `Drawing::validate` checks it.
    fn to_core(&self) -> Lake {
        Lake { x_m: self.x_m, z_m: self.z_m, radius_m: self.radius_m as f32 }
    }

    /// A new resource holding `lake`, its centre as the core holds it.
    fn from_core(lake: &Lake) -> Gd<Self> {
        let Lake { x_m, z_m, radius_m } = *lake;
        Gd::from_object(Self { x_m, z_m, radius_m: f64::from(radius_m) })
    }
}

/// A drawing's picture of high ground held on the CPU, as `LandformDrawing::from_core` gives a land's
/// drawings (`LandformLand.get_drawing()`, `get_asked_drawing()`; stage 2's ruling R18, N1): reading
/// it (`get_image`, as `to_core` and the editor tool's brush read a drawing's picture) hands over its
/// `Image.FORMAT_RF` of the core's values at once, where an `ImageTexture` would read it back from
/// the GPU, stalling every frame in flight, and making one uploads nothing to the GPU. It is never
/// drawn: it holds no texture on the GPU, so drawn it shows nothing. `get_image` gives its own
/// picture, not a copy: whoever reads it copies it before changing it, as `to_core` and the brush
/// do. The picture is a stored property, `picture`, so it survives a reload of the extension and is
/// saved with a resource that holds it; a script makes one as `LandformPicture.new()` with its
/// `picture` set, as the editor tool's new land does (stage 2's ruling R19, N6).
#[derive(GodotClass)]
#[class(tool, init, base = Texture2D)]
pub struct LandformPicture {
    /// The picture, an `Image.FORMAT_RF` of the core's values for a land's drawing; `null` for none.
    /// Named so that no accessor of it is called `get_image`, which would shadow `Texture2D`'s
    /// (stage 2's ruling R19, S1): `get_image` reads it through the texture's own `_get_image`.
    #[export]
    picture: Option<Gd<Image>>,
    base: Base<Texture2D>,
}

impl LandformPicture {
    /// A new picture holding `image`.
    pub fn of(image: Gd<Image>) -> Gd<Self> {
        Gd::from_init_fn(|base| Self { picture: Some(image), base })
    }
}

#[godot_api]
impl ITexture2D for LandformPicture {
    fn get_width(&self) -> i32 {
        self.picture.as_ref().map_or(0, |picture| picture.get_width())
    }

    fn get_height(&self) -> i32 {
        self.picture.as_ref().map_or(0, |picture| picture.get_height())
    }

    fn has_alpha(&self) -> bool {
        false
    }

    fn get_image(&self) -> Option<Gd<Image>> {
        self.picture.clone()
    }
}

/// What a person draws for a `LandformConfig` whose source is the drawing: the land's size, where
/// the ground stands high, the rivers, the lakes and the kind of country the land grows into. World
/// metres: `(0, 0)` is the centre, `x` east, `z` south, north is `-z`.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformDrawing {
    /// Side of the land in metres: the world grown from the drawing has this many cells of one
    /// metre on each side.
    #[export]
    #[init(val = GenConfig::default().size_cells as i64)]
    size_m: i64,
    /// Where the ground stands high: each pixel's red channel, or its grey in a grey picture, from
    /// black for low to white for high, stretched over the whole land with its first row at the
    /// north edge. A compressed texture is decompressed first. `null` leaves the land low everywhere.
    #[export]
    high_ground: Option<Gd<Texture2D>>,
    /// The rivers. A river whose last point lies within 40 m of another river's line joins that
    /// river, whichever of the two comes first in the list. One that ends in a lake joins the
    /// lake when another river already gives it a way out, by passing through it or being carried
    /// on from it. Any other river is carried on in its last direction to the edge, with a note
    /// when it ended inside the land or in a lake. The order of the list decides only which river
    /// is carried on when two end in the same lake with no other way out, or end on each other (the
    /// first), and which trench a cell keeps where two lie equally near it. An empty entry is
    /// refused: give it a `LandformRiver` or remove it.
    #[export]
    rivers: Array<Option<Gd<LandformRiver>>>,
    /// The lakes. An empty entry is refused: give it a `LandformLake` or remove it.
    #[export]
    lakes: Array<Option<Gd<LandformLake>>>,
    /// The kind of country the land grows into; `null` grows hill country.
    #[export]
    country: Option<Gd<LandformCountry>>,
}

impl LandformDrawing {
    /// The drawing as the core's `Drawing`, its points and lakes' centres as they are, or why it
    /// cannot be one: a size that is no whole number of metres, a texture without a picture, an
    /// empty river or lake entry, a river whose `points_m` holds an odd count of numbers, or the
    /// message of the country's or the drawing's `validate`. Rivers and lakes are numbered from 1,
    /// as the core numbers them. Main thread only: it reads the texture's picture.
    pub fn to_core(&self) -> Result<Drawing, String> {
        let size_m = u32::try_from(self.size_m).map_err(|_| {
            format!("size_m must be between {MIN_SIZE_CELLS} and {MAX_SIZE_CELLS} metres, not {}.", self.size_m)
        })?;
        let high_ground = self.high_ground.as_ref().map(|texture| picture(texture, size_m)).transpose()?;
        let rivers = (self.rivers.iter_shared().enumerate())
            .map(|(index, river)| entry(river, "river", index)?.bind().to_core(index))
            .collect::<Result<_, String>>()?;
        let lakes = (self.lakes.iter_shared().enumerate())
            .map(|(index, lake)| Ok(entry(lake, "lake", index)?.bind().to_core()))
            .collect::<Result<_, String>>()?;
        let country = match &self.country {
            Some(country) => country.bind().to_core()?,
            None => Country::default(),
        };
        let drawing = Drawing { size_m, high_ground, rivers, lakes, country };
        drawing.validate().map_err(|e| e.to_string())?;
        Ok(drawing)
    }

    /// A new resource holding `drawing`: its size; its picture of high ground as a `LandformPicture`
    /// over an `Image.FORMAT_RF` of the core's values, held on the CPU, which `to_core` reads back as
    /// they are without the GPU (stage 2's ruling R18, N1); its
    /// rivers and lakes as new resources, each point and centre the 64-bit float the core holds,
    /// so `to_core` gives them back bit for bit; and its kind of country. Taken apart field by
    /// field, so a field added to the core fails to compile until it is given here. Main thread
    /// only: it makes a texture.
    pub fn from_core(drawing: &Drawing) -> Gd<Self> {
        let Drawing { size_m, high_ground, rivers, lakes, country } = drawing;
        let high_ground = high_ground.as_ref().map(|picture| {
            let bytes: Vec<u8> = picture.data.iter().flat_map(|value| value.to_ne_bytes()).collect();
            LandformPicture::of(image_from(picture.width, picture.height, Format::RF, bytes)).upcast::<Texture2D>()
        });
        Gd::from_object(Self {
            size_m: i64::from(*size_m),
            high_ground,
            rivers: rivers.iter().map(|river| Some(LandformRiver::from_core(river))).collect(),
            lakes: lakes.iter().map(|lake| Some(LandformLake::from_core(lake))).collect(),
            country: Some(LandformCountry::from_core(*country)),
        })
    }
}

/// The entry at `index` of the drawing's list of `item`s, or why it is empty. Numbered from 1 in
/// the message, as the core numbers rivers and lakes.
fn entry<T: GodotClass>(entry: Option<Gd<T>>, item: &str, index: usize) -> Result<Gd<T>, String> {
    entry.ok_or_else(|| {
        format!("The drawing's {item} {} is empty: give it a {} or remove it.", index + 1, T::class_id())
    })
}

/// The texture's picture as values from 0 to 1, its first row at the north edge: each pixel's red
/// channel as Godot's `Image.get_pixel` reads it, so a grey picture gives its grey. The picture is
/// converted, as one block, to 32-bit floats and read from its bytes, so a large picture is read
/// quickly; a conversion that does not give one float per pixel is refused. Main thread only.
fn picture(texture: &Gd<Texture2D>, size_m: u32) -> Result<Grid<f32>, String> {
    let image = (texture.get_image())
        .filter(|image| !image.is_empty())
        .ok_or("high_ground's texture holds no picture: give it one or set high_ground to null.")?;
    // A texture may hand out its own image, which must stay as it is: work on a copy.
    let mut copy = Image::new_gd();
    copy.copy_from(&image);
    if copy.is_compressed() {
        let result = copy.decompress();
        if result != Error::OK {
            return Err(format!("high_ground's picture is compressed in a format that cannot be decompressed ({result:?})."));
        }
    }
    copy.clear_mipmaps();
    copy.convert(Format::RF);
    let (width, height) = (copy.get_width() as usize, copy.get_height() as usize);
    let bytes = copy.get_data();
    let format = copy.get_format();
    if format != Format::RF || Some(bytes.len()) != width.checked_mul(height).and_then(|pixels| pixels.checked_mul(4)) {
        return Err(format!(
            "high_ground's picture could not be read: converted to one 32-bit float per pixel, its {width} x {height} pixels gave {} bytes in the format {format:?}.",
            bytes.len()
        ));
    }
    let data: Vec<f32> = bytes.as_slice().as_chunks::<4>().0.iter().map(|&value| f32::from_ne_bytes(value)).collect();
    // The core does not read a picture's cell size; this is the size of a pixel on the land.
    Ok(Grid { width, height, cell_size: size_m as f32 / width as f32, data })
}
