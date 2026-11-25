//! The land file (`.lfland`): a `Land` as bytes, so that a land read is the land written, field for
//! field and bit for bit (the editor tool's design, **The land file**). Nothing is packed (the plan's
//! decision 6: packing needs a crate, which is a download).
//!
//! **Layout.** Little-endian throughout. `MAGIC` (8 bytes); the format, `LAND_FORMAT`, as a `u32`;
//! the version of Landform that wrote it (`landform_core`'s crate version) as text; the head's
//! checksum, a `u64`; then sections to the end of the file, each a tag of 4 ASCII bytes, its length
//! in bytes as a `u64`, its checksum as a `u64`, and that many bytes. Text is its length in bytes as
//! a `u64`, then UTF-8. Every count and every side of a grid is a `u64`; every number of the land is
//! written as its own type, an `f32` or `f64` by its bits. A grid is its width, its height, its
//! `cell_size` (`f32`) and its values row by row. A list is its count, then its items. A *record* is
//! a struct of named numbers: its count of fields, then each field's name (text), its kind (one
//! byte: `KIND_F32`, `KIND_F64`, `KIND_U32` or `KIND_U64`) and its value. The sections, in the order
//! they are written (`SECTIONS`, with `ASKED_SECTION` just before `MARK` while the land waits):
//!
//! - `DRAW`, the drawing but its country: `size_m` (`u32`); the rivers, each `width_m` (`f32`) and
//!   its points as `x` and `z` (`f64` each); the lakes, each `x_m`, `z_m` (`f64`) and `radius_m`
//!   (`f32`); then 0, or 1 and the picture of high ground as a grid.
//! - `CTRY`, the drawing's kind of country, a record of `Country`'s fields.
//! - `SEED`, the seed (`u64`).
//! - `CONF`, a record of the settings: `size_cells` (`u64`), `cell_size_m`, `detail_amplitude_m`,
//!   and `PickRules`' fields named `pick.<field>`.
//! - `GRUL` and `WRUL`, records of the ground rules' and the water rules' fields.
//! - `REAC`, the reach (`f32`).
//! - `MODL`, the land model's heights, a grid.
//! - `LAKE`, the first lakes' levels: a list of the lake's place in the drawing (`u64`) and its level
//!   (`f32`).
//! - `HGHT`, the world's heights, a grid; `GRND`, the ground map: its width, height and `cell_size`,
//!   then three bytes a cell (`strongest`, `second`, `blend`).
//! - `LAYR`, one for each layer of the world, in the order of their names: the name, then the grid.
//! - `WATR`, the water: the courses, each its `drawn` and `from_lake` (0 for none, or 1 and the
//!   index as a `u64`), its end (0 the border, 1 a course, 2 a lake, then the index as a `u64`) and
//!   its points (`x_m`, `z_m`, `surface_m`, `width_m`, `depth_m`, `f32` each); then the lakes, each
//!   `drawn` (`u64`), `level_m`, `x_m`, `z_m` and `area_m2` (`f32` each).
//! - `NOTE`, the notes as a list of text, then what each is about as a list (0 the land, 1 a river,
//!   2 a lake, then the index as a `u64`).
//! - `META`, the world's meta: `seed` (`u64`), `source_id` (text), the window (`source` as a `u64`,
//!   `center_x`, `center_y`, `rotation_rad` as `f64`, `mirrored` as one byte, `size_m` as `f64`),
//!   `base_elevation_m`, `north` (two `f32`) and the timings, a record of `Timings`' fields, so a
//!   land read holds the timings written.
//! - `ASKD`, the drawing asked for (`Land::asked`), written only while the land waits
//!   (`Land::waiting`): `DRAW`'s body for it, then `CTRY`'s. A land with nothing waiting is written
//!   without it, so its file is the one this Landform wrote before the drawing asked for was kept,
//!   byte for byte, and the format stays 1: a Landform that does not know the section skips it and
//!   says so, and drops it when it writes the land again. It stands just before `MARK`, which every
//!   file holds last, and is refused anywhere else, so a waiting land's file cut short is refused
//!   wherever it is cut.
//! - `MARK`, the marks: the held marks, then the hand marks, each its count of cells and one bit a
//!   cell, the first cell in the lowest bit of the first byte.
//!
//! **The checksums.** Every byte after the magic lies under one checksum, FNV-1a 64 as the tests'
//! fingerprints use it (no crate): the head's covers the format and the version, a section's its
//! tag, its length and its body. They are kept per section, not once for the whole file, so that a
//! damaged section is named ("its MARK section (the marks) is damaged, since its bytes do not give
//! the checksum written with them"), a file cut short still names the section it lacks or runs
//! into (every section before the cut is whole, and verified), a section this Landform does not
//! know is verified as it is skipped, and the sections are hashed side by side on the pool's
//! threads, each from its own bytes alone, so the outcome is the same on any count of threads. A
//! change of any one byte always changes the checksum over it (FNV-1a's step is one to one in the
//! byte, and each later step in the hash), so a file with one byte changed is never read as a
//! land: a byte of the magic says it is not a land file, a byte of the format says another format
//! (which cannot be checked, since a later format may lay its head out otherwise), and every other
//! byte says that its section or the head is damaged (a byte of a section's length may instead run
//! the section past the end of the file, which is said as a file cut short). The first damaged
//! section in the file's order is said before any fault found after it, which the damage may have
//! caused. Two changes or more are refused but for odds of about one in 2^64. A file built by hand
//! with its checksums worked out again is read as what it holds: it is the person's own data, held
//! to the checks below.
//!
//! **Reading.** A section whose tag the reader does not know is skipped, whatever it holds, so a
//! later Landform can add one, before `ASKD` (which stands immediately before `MARK`, so a section
//! between the two is refused); a layer whose name it does not know is skipped likewise, but only
//! when its body is a name and a grid of `f32` that fills the section, since every `LAYR` is read
//! before its name is looked at. A file without a section the land needs, with a known section
//! twice (but `LAYR`, once for each name) or one longer than what it holds, is refused. In a
//! record, a field the reader does not know is skipped when it is of one of the four kinds of
//! number, and refused when its kind byte names another; a field of format 1 the record does not
//! hold is refused, naming it ("its GRUL section (the ground rules) holds no field gully_depth_m,
//! which every land file of format 1 holds"), since a file of format 1 always holds every field of
//! format 1; a field of a known name written as another kind of number is refused. Each part
//! skipped is named in `LandFile::skipped` (`Skipped`), once, in the order the reader met it. A
//! format other than `LAND_FORMAT` is refused. A file without `ASKD` is read with the land's
//! drawing as the drawing asked for, so nothing waits; a waiting land's file cut short inside the
//! section's body names it, and cut at its start or its end lacks `MARK`, so no cut loses what
//! waits without a word; an `ASKD` that does not stand just before `MARK` is refused, naming where
//! it stands, so a file holding it elsewhere cannot be cut into a land with nothing waiting. An
//! `ASKD` equal to the land's drawing, which this Landform never writes, is read as nothing
//! waiting, and dropped when the land is written again.
//!
//! **What a missing or skipped part means.** A Landform that adds a field to a record either raises
//! `LAND_FORMAT`, or reads a file written before the field with the field's value of no effect: the
//! value under which the engine does exactly what it did before the field existed, written down as
//! a constant beside the field, never the struct's `default()`, which moves as accepted values are
//! written into the rules, so a land would come back under rules it was not made with. A Landform
//! reading a newer file of the same format skips what it does not know (a field, a section, a
//! layer) and says so in `LandFile::skipped`; its edits run without that part, and when it writes
//! the land again what it skipped is gone.
//!
//! **What is checked.** The drawing and the settings are checked as `Land::grow` checks them as
//! soon as they are read, so every later part is read against a drawing whose size is valid; the
//! marks are refused when they run beyond the world's cells before they are made. Then the land is
//! checked as an edit, a hold, an export and the water's surface need it, so that none of them
//! panics on it or reads a part as another than the land means (`check`): among the rest, the
//! drawing asked for is one `Drawing::validate` takes, of the land's size, as `Land::ask` asks; every
//! layer's cells are finite; no course ends on itself; no lake of the drawing is given twice among
//! the first lakes or the water's lakes; the window and north are finite and the world was grown
//! from the land's seed; and every course point's water is above 0 m and at most the land's
//! `size_m` wide and no less than 0 m deep, since `water_surface` lays its courses out in buckets
//! over each one's widest reach, so a width far beyond the land's would make it panic or ask for
//! more memory than the machine has. A land `Land::grow` made from a valid drawing under the
//! shipped water rules never comes near either bound; but a river drawn wider than `size_m / (1 +
//! width_wander)`, whose wander widens its water past the land's size, or water rules at the far
//! ends of their ranges that make a brook wider than the land, give a land the reader refuses.
//! Within the bounds a file built by hand can still ask much of `water_surface`, since a course's
//! stretches are laid out by its widest point: 30,000 points more at the bound on one course of the
//! example at 2,048 m asked 4.1 GB of an export and 2.2 GB of an edit (measured by stage 1's review
//! of the land file), while the same land with every point it has set to the bound asked about
//! twice the plain land's (461 MB against 228 MB). What is read as written, being harmless to every
//! step (no panic, nothing read as another part): a finite surface, depth or lake level of any
//! size, among them a course whose surface rises downstream; a course of one point; courses ending
//! on each other; first lakes' levels for fewer lakes than the water holds (a lake the model holds
//! no water in has none); the settings a drawn land does not read (`size_cells` and `pick.*`);
//! model heights finite but far beyond any ground; a reach outside `REACH_MIN_M` to `REACH_MAX_M`,
//! which an outline reads held to that range; marks shorter than the world, read as unmarked beyond
//! them. Every refusal is `LandformError::BadLand`, one sentence naming the file and the fault;
//! nothing a file holds makes the reader panic, and nothing it reads is allocated before the bytes
//! for it are known to be there. The whole file is held in memory to write it and to read it,
//! beside the land: about twice the file (230 MB at 2,048 m, 3.7 GB at 8,192 m).

use super::outline::Marks;
use super::Land;
use crate::country::Country;
use crate::cover::{GroundCell, Material, LAYER_NAMES};
use crate::drawing::{Drawing, Lake, River};
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{model_side, About, GrownLake, GROW_CELL_M};
use crate::rules::GroundRules;
use crate::water::{CourseEnd, CoursePoint, LakeWater, RiverCourse, Water, WATER_LAYER};
use crate::water_rules::WaterRules;
use crate::window::{PickRules, Window};
use crate::world::{check_drawn, GenConfig, Timings, World, WorldMeta};
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The format of the land files this Landform writes, and the only one it reads.
pub const LAND_FORMAT: u32 = 1;

/// The bytes every land file begins with: `LFLAND` and a carriage return and line feed, which a
/// copy that changes line ends breaks.
const MAGIC: [u8; 8] = *b"LFLAND\r\n";

/// The version of Landform written into every land file.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What `Land::write` adds to the land file's name for the file it writes first, and renames to the
/// land file's name once it is whole, so a write that fails leaves the land file as it was.
const PART_SUFFIX: &str = ".part";

/// The bytes of a section's tag.
const TAG_BYTES: usize = 4;

/// A section's tag: four ASCII bytes.
type Tag = [u8; TAG_BYTES];

/// The bytes of a section's length, a `u64`, which follows its tag.
const LENGTH_BYTES: usize = size_of::<u64>();

/// The bytes of a checksum, a `u64`: the head's, after the version, and each section's, after its
/// length.
const CHECKSUM_BYTES: usize = size_of::<u64>();

/// The bytes of a section's head: its tag, its body's length and its checksum.
const SECTION_HEAD_BYTES: usize = TAG_BYTES + LENGTH_BYTES + CHECKSUM_BYTES;

/// FNV-1a's 64-bit offset basis and prime, as the fingerprints of the tests use them: the checksum
/// of the head and of each section.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

const DRAWING: Tag = *b"DRAW";
const COUNTRY: Tag = *b"CTRY";
const SEED: Tag = *b"SEED";
const CONFIG: Tag = *b"CONF";
const GROUND_RULES: Tag = *b"GRUL";
const WATER_RULES: Tag = *b"WRUL";
const REACH: Tag = *b"REAC";
const MODEL: Tag = *b"MODL";
const FIRST_LAKES: Tag = *b"LAKE";
const HEIGHTS: Tag = *b"HGHT";
const GROUND: Tag = *b"GRND";
const LAYER: Tag = *b"LAYR";
const WATER: Tag = *b"WATR";
const NOTES: Tag = *b"NOTE";
const META: Tag = *b"META";
const MARKS: Tag = *b"MARK";
const ASKED: Tag = *b"ASKD";

/// The sections every land file holds, in the order this Landform writes them, with what each holds
/// in words.
const SECTIONS: [(Tag, &str); 16] = [
    (DRAWING, "the drawing"),
    (COUNTRY, "the drawing's kind of country"),
    (SEED, "the seed"),
    (CONFIG, "the settings"),
    (GROUND_RULES, "the ground rules"),
    (WATER_RULES, "the water rules"),
    (REACH, "the reach"),
    (MODEL, "the model's heights"),
    (FIRST_LAKES, "the first lakes' levels"),
    (HEIGHTS, "the world's heights"),
    (GROUND, "the ground map"),
    (LAYER, "a layer of the world"),
    (WATER, "the water"),
    (NOTES, "the notes"),
    (META, "the world's meta"),
    (MARKS, "the marks"),
];

/// The section only a waiting land's file holds, written just before `MARK` (the last of
/// `SECTIONS`) and read only there (`asked_in_place`), with what it holds in words.
const ASKED_SECTION: (Tag, &str) = (ASKED, "the asked drawing");

/// Every section this Landform knows: `SECTIONS`, then `ASKED_SECTION`.
fn known_sections() -> impl Iterator<Item = &'static (Tag, &'static str)> {
    SECTIONS.iter().chain([&ASKED_SECTION])
}

/// The kinds of number a record's field is written as.
const KIND_F32: u8 = 1;
const KIND_F64: u8 = 2;
const KIND_U32: u8 = 3;
const KIND_U64: u8 = 4;

/// The byte before an index that may be missing: none, or one follows.
const NONE: u8 = 0;
const SOME: u8 = 1;

/// The byte of a course's end.
const END_EDGE: u8 = 0;
const END_RIVER: u8 = 1;
const END_LAKE: u8 = 2;

/// The byte of what a note is about.
const ABOUT_LAND: u8 = 0;
const ABOUT_RIVER: u8 = 1;
const ABOUT_LAKE: u8 = 2;

/// A land file as read: the land, what the file says of itself, and what of it the reader skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct LandFile {
    pub land: Land,
    /// The file's format: `LAND_FORMAT`, the only one read.
    pub format: u32,
    /// The version of Landform that wrote it (`landform_core`'s crate version, as text).
    pub version: String,
    /// The parts of the file this Landform does not know, which it skipped, each once, in the order
    /// it met them: the land holds none of them, so a land written again drops them.
    pub skipped: Vec<Skipped>,
}

/// A part of a land file the reader does not know and skips (the module's **Reading**).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skipped {
    /// A section of a tag it does not know, the tag as text (a byte that is not a printable ASCII
    /// character as `?`).
    Section(String),
    /// A layer of the world of a name it does not know.
    Layer(String),
    /// A field of a record it does not know: the tag of the record's section, as text, and the
    /// field's name.
    Field { section: String, name: String },
}

/// "the section ZZZZ", "the layer future", "the field future_m of the GRUL section (the ground
/// rules)".
impl fmt::Display for Skipped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Skipped::Section(tag) => write!(f, "the section {tag}"),
            Skipped::Layer(name) => write!(f, "the layer {name}"),
            Skipped::Field { section, name } => {
                let words = known_sections().find(|(tag, _)| tag_text(*tag) == *section).map_or("a section this Landform does not know", |(_, words)| words);
                write!(f, "the field {name} of the {section} section ({words})")
            }
        }
    }
}

impl LandFile {
    /// Reads the land file at `path` (the module's **Layout** and **Reading**). An error opening or
    /// reading it is `Io` naming the path; a file that is not a land this Landform reads is `BadLand`.
    pub fn read(path: &Path) -> Result<LandFile, LandformError> {
        let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
        from_bytes(&bytes).map_err(|reason| LandformError::BadLand { path: path.to_path_buf(), reason })
    }

    /// The land file whose bytes are `bytes` (those `Land::to_bytes` gives), read as `read` reads a
    /// file; when it is not a land this Landform reads, the fault in the words of `BadLand`'s
    /// `reason`, which names no file. What the Godot extension restores a land from after a reload.
    /// The sections' checksums are worked out on the current rayon pool, as `read` works them out.
    pub fn from_bytes(bytes: &[u8]) -> Result<LandFile, String> {
        from_bytes(bytes)
    }
}

impl Land {
    /// Writes the land to `path` as a land file (the module's **Layout**), field for field and bit for
    /// bit, its world's timings among them. The bytes go first to a file of the same name with
    /// `.part` added, flushed to the disk, which then takes the land file's name, so a write that
    /// fails leaves a land file already at `path` as it was. An error is `Io` naming the path.
    pub fn write(&self, path: &Path) -> Result<(), LandformError> {
        let bytes = self.to_bytes();
        let mut part = path.as_os_str().to_owned();
        part.push(PART_SUFFIX);
        let part = PathBuf::from(part);
        // Flushed to the disk before it takes the land file's name, so the name never points at a
        // file the disk does not yet hold whole.
        let written = File::create(&part).and_then(|mut file| {
            file.write_all(&bytes)?;
            file.sync_all()
        });
        if let Err(error) = written {
            let _ = fs::remove_file(&part);
            return Err(io_error(&part, error));
        }
        fs::rename(&part, path).map_err(|error| {
            let _ = fs::remove_file(&part);
            io_error(path, error)
        })
    }

    /// The bytes of the land's file, those `write` writes (the module's **Layout**), its world's
    /// timings among them, in memory: what the tests of a step of undo compare a land by, byte for
    /// byte (`Change`).
    pub fn to_bytes(&self) -> Vec<u8> {
        to_bytes(self)
    }

    /// The land in the land file at `path`, as `LandFile::read` reads it: equal to the land written,
    /// field for field and bit for bit.
    pub fn read(path: &Path) -> Result<Land, LandformError> {
        LandFile::read(path).map(|file| file.land)
    }
}

fn io_error(path: &Path, error: std::io::Error) -> LandformError {
    LandformError::Io(format!("{}: {error}", path.display()))
}

/// The FNV-1a 64 of `parts`' bytes, one after another: a land file's checksum. A change of any one
/// byte always changes it, since each byte's step is one to one in the byte for the hash before
/// it, and each later step is one to one in the hash.
fn checksum(parts: &[&[u8]]) -> u64 {
    let mut hash = FNV_OFFSET;
    for part in parts {
        for &byte in *part {
            hash = (hash ^ u64::from(byte)).wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// A section's checksum: of its tag and its length, as its head holds them, and of its body.
fn section_checksum(tag_and_length: &[u8], body: &[u8]) -> u64 {
    checksum(&[tag_and_length, body])
}

/// "... is damaged, since its bytes do not give the checksum written with them", of `what`.
fn damaged(what: &str) -> String {
    format!("{what} is damaged, since its bytes do not give the checksum written with them")
}

/// What a section of `tag` holds, in words; "a section this Landform does not know" for another tag.
fn words(tag: Tag) -> &'static str {
    known_sections().find(|(known, _)| *known == tag).map_or("a section this Landform does not know", |(_, words)| words)
}

/// A tag as text, a byte that is not a printable ASCII character as `?`.
fn tag_text(tag: Tag) -> String {
    tag.iter().map(|&byte| if byte.is_ascii_graphic() { char::from(byte) } else { '?' }).collect()
}

/// A section named in a sentence: "its HGHT section (the world's heights)".
fn section_name(tag: Tag) -> String {
    format!("its {} section ({})", tag_text(tag), words(tag))
}

// ---------------------------------------------------------------------------------------------
// Writing.

/// A number of a record.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Value {
    F32(f32),
    F64(f64),
    U32(u32),
    U64(u64),
}

impl Value {
    fn words(self) -> &'static str {
        match self {
            Value::F32(_) => f32::WORDS,
            Value::F64(_) => f64::WORDS,
            Value::U32(_) => u32::WORDS,
            Value::U64(_) => u64::WORDS,
        }
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Value::F32(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::F64(value)
    }
}

impl From<u32> for Value {
    fn from(value: u32) -> Self {
        Value::U32(value)
    }
}

impl From<usize> for Value {
    fn from(value: usize) -> Self {
        Value::U64(value as u64)
    }
}

/// A record as written: each field's name and value, in the struct's order.
struct Record(Vec<(&'static str, Value)>);

/// The bytes of a land file as they are written, and where each section written starts.
struct Bytes(Vec<u8>, Vec<usize>);

impl Bytes {
    fn new() -> Self {
        Bytes(Vec::new(), Vec::new())
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.raw(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.raw(&value.to_le_bytes());
    }

    fn count(&mut self, count: usize) {
        self.u64(count as u64);
    }

    fn f32(&mut self, value: f32) {
        self.u32(value.to_bits());
    }

    fn f64(&mut self, value: f64) {
        self.u64(value.to_bits());
    }

    fn text(&mut self, text: &str) {
        self.count(text.len());
        self.raw(text.as_bytes());
    }

    /// An index that may be missing.
    fn index(&mut self, index: Option<usize>) {
        match index {
            None => self.u8(NONE),
            Some(index) => {
                self.u8(SOME);
                self.count(index);
            }
        }
    }

    fn grid(&mut self, grid: &Grid<f32>) {
        self.count(grid.width);
        self.count(grid.height);
        self.f32(grid.cell_size);
        self.0.reserve(grid.data.len() * 4);
        for value in &grid.data {
            self.f32(*value);
        }
    }

    /// The flags, their count and then one bit each, the first in the lowest bit of the first byte.
    fn flags(&mut self, flags: &[bool]) {
        self.count(flags.len());
        for eight in flags.chunks(8) {
            self.u8(eight.iter().enumerate().filter(|(_, &on)| on).fold(0, |byte, (bit, _)| byte | 1 << bit));
        }
    }

    fn record(&mut self, record: &Record) {
        self.count(record.0.len());
        for &(name, value) in &record.0 {
            self.text(name);
            match value {
                Value::F32(value) => {
                    self.u8(KIND_F32);
                    self.f32(value);
                }
                Value::F64(value) => {
                    self.u8(KIND_F64);
                    self.f64(value);
                }
                Value::U32(value) => {
                    self.u8(KIND_U32);
                    self.u32(value);
                }
                Value::U64(value) => {
                    self.u8(KIND_U64);
                    self.u64(value);
                }
            }
        }
    }

    /// A section of `tag` whose body `body` writes: the tag, the body's length, room for its
    /// checksum (written by `sealed`), the body.
    fn section(&mut self, tag: Tag, body: impl FnOnce(&mut Bytes)) {
        let start = self.0.len();
        self.1.push(start);
        self.raw(&tag);
        self.u64(0);
        self.u64(0);
        body(self);
        let length = (self.0.len() - start - SECTION_HEAD_BYTES) as u64;
        self.0[start + TAG_BYTES..start + TAG_BYTES + LENGTH_BYTES].copy_from_slice(&length.to_le_bytes());
    }

    /// The bytes, each section's checksum written into its head. Each is worked out from its own
    /// section's bytes alone, the sections side by side on the pool's threads, so the bytes are the
    /// same on any count of threads.
    fn sealed(self) -> Vec<u8> {
        let Bytes(mut bytes, starts) = self;
        let sums: Vec<u64> = starts
            .par_iter()
            .map(|&start| {
                let tag_and_length = &bytes[start..start + TAG_BYTES + LENGTH_BYTES];
                let length = u64::from_le_bytes(tag_and_length[TAG_BYTES..].try_into().expect("a length of 8 bytes")) as usize;
                let body_at = start + SECTION_HEAD_BYTES;
                section_checksum(tag_and_length, &bytes[body_at..body_at + length])
            })
            .collect();
        for (start, sum) in starts.into_iter().zip(sums) {
            bytes[start + TAG_BYTES + LENGTH_BYTES..start + SECTION_HEAD_BYTES].copy_from_slice(&sum.to_le_bytes());
        }
        bytes
    }
}

/// Writes and reads a struct of named numbers as a record: each field by its name (with `prefix`
/// before it) and its value. The struct is taken apart field by field, and made field by field, so
/// a new field fails to compile until it is listed here.
macro_rules! named_fields {
    ($write:ident, $read:ident, $Type:ident, $prefix:literal { $($field:ident),* $(,)? }) => {
        fn $write(value: &$Type) -> Vec<(&'static str, Value)> {
            let $Type { $($field),* } = *value;
            vec![$((concat!($prefix, stringify!($field)), Value::from($field))),*]
        }

        fn $read(fields: &mut Fields) -> Result<$Type, String> {
            Ok($Type { $($field: fields.get(concat!($prefix, stringify!($field)))?),* })
        }
    };
}

named_fields!(country_fields, read_country, Country, "" {
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
});

named_fields!(pick_fields, read_pick, PickRules, "pick." { max_tries, max_flat_fraction, flat_epsilon_m, margin_m });

named_fields!(ground_rule_fields, read_ground_rules, GroundRules, "" {
    rock_slope_start_deg,
    rock_slope_full_deg,
    soil_slope_start_deg,
    soil_slope_full_deg,
    scree_reach_m,
    scree_rest_slope_deg,
    wet_flow_start_m2,
    wet_flow_full_m2,
    wet_start,
    wet_full,
    wet_slope_limit_deg,
    dry_ridge_start_m,
    dry_ridge_full_m,
    dry_sun_slope_start_deg,
    dry_sun_slope_full_deg,
    meadow_weight,
    patch_strength,
    patch_size_m,
    transition_width,
    gully_depth_m,
    gully_flow_start_m2,
    gully_flow_full_m2,
    gully_min_slope_deg,
    ledge_spacing_m,
    ledge_height_m,
    ledge_slope_start_deg,
    ledge_slope_full_deg,
    rough_amplitude_m,
    rough_size_m,
    detail_limit_m,
});

named_fields!(water_rule_fields, read_water_rules, WaterRules, "" {
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
});

named_fields!(timing_fields, read_timings, Timings, "" {
    pick_ms,
    grow_ms,
    resample_ms,
    analyse_ms,
    detail_ms,
    water_ms,
    cover_ms,
    total_ms,
});

/// The settings' own fields, then the window-picking rules' as `pick.<field>`; the ground and water
/// rules have records of their own.
fn config_fields(config: &GenConfig) -> Vec<(&'static str, Value)> {
    let GenConfig { size_cells, cell_size_m, detail_amplitude_m, pick, rules: _, water: _ } = *config;
    let mut fields = vec![
        ("size_cells", Value::from(size_cells)),
        ("cell_size_m", Value::from(cell_size_m)),
        ("detail_amplitude_m", Value::from(detail_amplitude_m)),
    ];
    fields.extend(pick_fields(&pick));
    fields
}

/// The bytes of `land`'s file (the module's **Layout**). Every struct is taken apart field by field,
/// so a new field fails to compile until it is written. A land with nothing waiting is written
/// without `ASKD`.
pub(crate) fn to_bytes(land: &Land) -> Vec<u8> {
    let waiting = land.waiting();
    let Land { drawing, asked, seed, config, model_heights, first_lake_levels, world, marks, reach_m } = land;
    let World { heights, layers, ground, meta, notes, notes_about, water } = world;
    let cells = heights.data.len();
    let mut out = Bytes(Vec::with_capacity(4 * (cells * (2 + layers.len()) + model_heights.data.len()) + (1 << 20)), Vec::new());
    out.raw(&MAGIC);
    out.u32(LAND_FORMAT);
    out.text(VERSION);
    // The head's checksum: of the format and the version, every byte of the head after the magic.
    let head = checksum(&[&out.0[MAGIC.len()..]]);
    out.u64(head);

    write_drawing(&mut out, drawing, DrawingIn::Land);
    out.section(SEED, |out| out.u64(*seed));
    write_settings(&mut out, config);
    out.section(REACH, |out| out.f32(*reach_m));
    write_model(&mut out, model_heights);
    write_first_lakes(&mut out, first_lake_levels);

    out.section(HEIGHTS, |out| out.grid(heights));
    out.section(GROUND, |out| {
        out.count(ground.width);
        out.count(ground.height);
        out.f32(ground.cell_size);
        out.0.reserve(ground.data.len() * 3);
        for &GroundCell { strongest, second, blend } in &ground.data {
            out.raw(&[strongest, second, blend]);
        }
    });
    for (name, layer) in layers {
        out.section(LAYER, |out| {
            out.text(name);
            out.grid(layer);
        });
    }
    write_water(&mut out, water);
    write_notes(&mut out, notes, notes_about);
    write_meta(&mut out, meta);
    // Only while the land waits, so a land with nothing waiting is the file it was before; before
    // the marks, so a waiting land's file cut at either end of it lacks them and is refused.
    if waiting {
        write_drawing(&mut out, asked, DrawingIn::Asked);
    }
    out.section(MARKS, |out| {
        let Marks { held, by_hand } = marks;
        out.flags(held);
        out.flags(by_hand);
    });
    out.sealed()
}

/// The settings' sections: `CONF`, `GRUL` and `WRUL`.
fn write_settings(out: &mut Bytes, config: &GenConfig) {
    out.section(CONFIG, |out| out.record(&Record(config_fields(config))));
    out.section(GROUND_RULES, |out| out.record(&Record(ground_rule_fields(&config.rules))));
    out.section(WATER_RULES, |out| out.record(&Record(water_rule_fields(&config.water))));
}

/// The land model's heights, `MODL`.
fn write_model(out: &mut Bytes, model_heights: &Grid<f32>) {
    out.section(MODEL, |out| out.grid(model_heights));
}

/// The first lakes' levels, `LAKE`.
fn write_first_lakes(out: &mut Bytes, first_lake_levels: &[GrownLake]) {
    out.section(FIRST_LAKES, |out| {
        out.count(first_lake_levels.len());
        for &GrownLake { drawn, level_m } in first_lake_levels {
            out.count(drawn);
            out.f32(level_m);
        }
    });
}

/// The water, `WATR`.
fn write_water(out: &mut Bytes, water: &Water) {
    out.section(WATER, |out| {
        let Water { rivers, lakes } = water;
        out.count(rivers.len());
        for RiverCourse { drawn, from_lake, points, end } in rivers {
            out.index(*drawn);
            out.index(*from_lake);
            let (kind, index) = match *end {
                CourseEnd::Edge => (END_EDGE, 0),
                CourseEnd::River(index) => (END_RIVER, index),
                CourseEnd::Lake(index) => (END_LAKE, index),
            };
            out.u8(kind);
            out.count(index);
            out.count(points.len());
            for &CoursePoint { x_m, z_m, surface_m, width_m, depth_m } in points {
                for value in [x_m, z_m, surface_m, width_m, depth_m] {
                    out.f32(value);
                }
            }
        }
        out.count(lakes.len());
        for &LakeWater { drawn, level_m, x_m, z_m, area_m2 } in lakes {
            out.count(drawn);
            for value in [level_m, x_m, z_m, area_m2] {
                out.f32(value);
            }
        }
    });
}

/// The notes and what each is about, `NOTE`.
fn write_notes(out: &mut Bytes, notes: &[String], notes_about: &[About]) {
    out.section(NOTES, |out| {
        out.count(notes.len());
        for note in notes {
            out.text(note);
        }
        out.count(notes_about.len());
        for about in notes_about {
            let (kind, index) = match *about {
                About::Land => (ABOUT_LAND, 0),
                About::River(index) => (ABOUT_RIVER, index),
                About::Lake(index) => (ABOUT_LAKE, index),
            };
            out.u8(kind);
            out.count(index);
        }
    });
}

/// The world's meta, its timings among it, `META`.
fn write_meta(out: &mut Bytes, meta: &WorldMeta) {
    out.section(META, |out| {
        let WorldMeta { seed, source_id, window, base_elevation_m, north, timings } = meta;
        let Window { source, center_x, center_y, rotation_rad, mirrored, size_m } = *window;
        out.u64(*seed);
        out.text(source_id);
        out.count(source);
        out.f64(center_x);
        out.f64(center_y);
        out.f64(rotation_rad);
        out.u8(u8::from(mirrored));
        out.f64(size_m);
        out.f32(*base_elevation_m);
        out.f32(north.0);
        out.f32(north.1);
        out.record(&Record(timing_fields(timings)));
    });
}

/// The bytes `write` writes, its sections' checksums left at 0: what a step of undo compares a part
/// it may hold whole by (`undo::Change::between`).
fn unsealed(write: impl FnOnce(&mut Bytes)) -> Vec<u8> {
    let mut out = Bytes::new();
    write(&mut out);
    out.0
}

// The bytes the land file holds each part of a land outside the world's grids and the marks by, as
// `to_bytes` writes it, its checksums left at 0: every number by its bits, so two parts give the same
// bytes exactly when they are one part bit for bit. A step of undo holds a part whole where these
// differ (`undo::Change::between`); a drawing is compared by `drawing_bytes`.

/// The settings' bytes: `CONF`, `GRUL` and `WRUL`.
pub(super) fn settings_bytes(config: &GenConfig) -> Vec<u8> {
    unsealed(|out| write_settings(out, config))
}

/// The land model's heights' bytes, `MODL`.
pub(super) fn model_bytes(model_heights: &Grid<f32>) -> Vec<u8> {
    unsealed(|out| write_model(out, model_heights))
}

/// The first lakes' levels' bytes, `LAKE`.
pub(super) fn first_lakes_bytes(first_lake_levels: &[GrownLake]) -> Vec<u8> {
    unsealed(|out| write_first_lakes(out, first_lake_levels))
}

/// The water's bytes, `WATR`.
pub(super) fn water_bytes(water: &Water) -> Vec<u8> {
    unsealed(|out| write_water(out, water))
}

/// The notes' bytes with what each is about, `NOTE`.
pub(super) fn notes_bytes(notes: &[String], notes_about: &[About]) -> Vec<u8> {
    unsealed(|out| write_notes(out, notes, notes_about))
}

/// The world's meta's bytes, its timings among them, `META`.
pub(super) fn meta_bytes(meta: &WorldMeta) -> Vec<u8> {
    unsealed(|out| write_meta(out, meta))
}

/// Where a drawing is written in a land file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrawingIn {
    /// The land's own drawing (`Land::drawing`): two sections, `DRAW` and `CTRY`.
    Land,
    /// The drawing asked for (`Land::asked`): one section, `ASKD`, laid out as the two are, `DRAW`'s
    /// body then `CTRY`'s.
    Asked,
}

/// The drawing's sections, as `place` lays them out. The drawing is taken apart field by field, so a
/// new field fails to compile until it is written, in both places alike.
fn write_drawing(out: &mut Bytes, drawing: &Drawing, place: DrawingIn) {
    let Drawing { size_m, high_ground, rivers, lakes, country } = drawing;
    let body = |out: &mut Bytes| {
        out.u32(*size_m);
        out.count(rivers.len());
        for River { points, width_m } in rivers {
            out.f32(*width_m);
            out.count(points.len());
            for &(x, z) in points {
                out.f64(x);
                out.f64(z);
            }
        }
        out.count(lakes.len());
        for &Lake { x_m, z_m, radius_m } in lakes {
            out.f64(x_m);
            out.f64(z_m);
            out.f32(radius_m);
        }
        match high_ground {
            None => out.u8(NONE),
            Some(picture) => {
                out.u8(SOME);
                out.grid(picture);
            }
        }
    };
    let kind = |out: &mut Bytes| out.record(&Record(country_fields(country)));
    match place {
        DrawingIn::Land => {
            out.section(DRAWING, body);
            out.section(COUNTRY, kind);
        }
        DrawingIn::Asked => out.section(ASKED, |out| {
            body(out);
            kind(out);
        }),
    }
}

/// The bytes the land file holds `drawing` by, its `DRAW` and `CTRY` sections, their checksums left
/// at 0: every number by its bits, so two drawings give the same bytes exactly when they are one
/// drawing bit for bit (which `Land::edit` asks of a drawing whose footprint is empty,
/// `Land::waiting` of the drawing asked for, and a step of undo of a drawing it holds whole,
/// `undo::Change::between`). Nothing is hashed and nothing runs on a pool's threads: it is written
/// on the calling thread alone (stage 2's ruling R2, N1).
pub(crate) fn drawing_bytes(drawing: &Drawing) -> Vec<u8> {
    unsealed(|out| write_drawing(out, drawing, DrawingIn::Land))
}

// ---------------------------------------------------------------------------------------------
// Reading.

/// A number a record's field is read as.
trait FieldValue: Sized {
    /// The kind of number, in words.
    const WORDS: &'static str;
    fn from_value(value: Value) -> Option<Self>;
}

impl FieldValue for f32 {
    const WORDS: &'static str = "a 32-bit float";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::F32(value) => Some(value),
            _ => None,
        }
    }
}

impl FieldValue for f64 {
    const WORDS: &'static str = "a 64-bit float";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::F64(value) => Some(value),
            _ => None,
        }
    }
}

impl FieldValue for u32 {
    const WORDS: &'static str = "a 32-bit whole number";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::U32(value) => Some(value),
            _ => None,
        }
    }
}

impl FieldValue for u64 {
    const WORDS: &'static str = "a 64-bit whole number";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::U64(value) => Some(value),
            _ => None,
        }
    }
}

impl FieldValue for usize {
    const WORDS: &'static str = u64::WORDS;
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::U64(value) => usize::try_from(value).ok(),
            _ => None,
        }
    }
}

/// A record as read: its fields by name, the section it was read from, and the names asked for.
struct Fields {
    values: BTreeMap<String, Value>,
    section: Tag,
    asked: BTreeSet<String>,
}

impl Fields {
    /// The field `name` read as `T`; an error when the record does not hold it (a land file of
    /// format 1 holds every field of format 1) or holds it as another kind of number.
    fn get<T: FieldValue>(&mut self, name: &str) -> Result<T, String> {
        let Some(&value) = self.values.get(name) else {
            return Err(format!("{} holds no field {name}, which every land file of format {LAND_FORMAT} holds", section_name(self.section)));
        };
        self.asked.insert(name.to_string());
        T::from_value(value).ok_or_else(|| format!("{} holds {name} as {}, where this Landform reads {}", section_name(self.section), value.words(), T::WORDS))
    }

    /// The fields of the record not asked for, which this Landform does not know, by name.
    fn skipped(&self) -> impl Iterator<Item = Skipped> + '_ {
        let section = tag_text(self.section);
        self.values.keys().filter(|name| !self.asked.contains(*name)).map(move |name| Skipped::Field { section: section.clone(), name: name.clone() })
    }
}

/// The bytes of a section being read, from the start.
struct Read<'a> {
    bytes: &'a [u8],
    at: usize,
    section: Tag,
}

impl<'a> Read<'a> {
    fn new(section: Tag, bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0, section }
    }

    fn short(&self) -> String {
        format!("{} ends before what it holds", section_name(self.section))
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        if count > self.bytes.len() - self.at {
            return Err(self.short());
        }
        let taken = &self.bytes[self.at..self.at + count];
        self.at += count;
        Ok(taken)
    }

    /// `count` items of `size` bytes each, or an error when the section does not hold them all.
    fn items(&mut self, count: usize, size: usize) -> Result<&'a [u8], String> {
        let bytes = count.checked_mul(size).ok_or_else(|| self.short())?;
        self.take(bytes)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut array = [0; N];
        array.copy_from_slice(self.take(N)?);
        Ok(array)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.array::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }

    /// A count or an index: a `u64` that must fit a `usize`.
    fn count(&mut self) -> Result<usize, String> {
        let value = self.u64()?;
        usize::try_from(value).map_err(|_| format!("{} holds a count of {value}, more than this machine can hold", section_name(self.section)))
    }

    fn text(&mut self) -> Result<String, String> {
        let length = self.count()?;
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| format!("{} holds text that is not UTF-8", section_name(self.section)))
    }

    /// One of the bytes in `allowed`, or an error naming `what` it says.
    fn choice(&mut self, allowed: &[u8], what: &str) -> Result<u8, String> {
        let byte = self.u8()?;
        if !allowed.contains(&byte) {
            return Err(format!("{} holds {byte} for {what}, which this Landform does not know", section_name(self.section)));
        }
        Ok(byte)
    }

    fn index(&mut self, what: &str) -> Result<Option<usize>, String> {
        Ok(match self.choice(&[NONE, SOME], what)? {
            NONE => None,
            _ => Some(self.count()?),
        })
    }

    /// A list: its count, then each item as `item` reads it. Nothing is allocated for the count
    /// before its items are read, so a count larger than the section ends at the section's end.
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Result<T, String>) -> Result<Vec<T>, String> {
        let count = self.count()?;
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(item(self)?);
        }
        Ok(items)
    }

    fn f32s(&mut self, count: usize) -> Result<Vec<f32>, String> {
        let bytes = self.items(count, 4)?;
        Ok(bytes.as_chunks::<4>().0.iter().map(|&four| f32::from_le_bytes(four)).collect())
    }

    fn sides(&mut self) -> Result<(usize, usize, f32, usize), String> {
        let (width, height, cell_size) = (self.count()?, self.count()?, self.f32()?);
        let cells = width.checked_mul(height).ok_or_else(|| self.short())?;
        Ok((width, height, cell_size, cells))
    }

    fn grid(&mut self) -> Result<Grid<f32>, String> {
        let (width, height, cell_size, cells) = self.sides()?;
        Ok(Grid { width, height, cell_size, data: self.f32s(cells)? })
    }

    /// Flags, one bit each (`Bytes::flags`), for at most `most` cells, refused as `what` marks
    /// before anything is made when the count is more: a list of flags makes eight times the bytes
    /// it reads.
    fn flags(&mut self, most: usize, what: &str) -> Result<Vec<bool>, String> {
        let count = self.count()?;
        if count > most {
            return Err(format!("its {what} marks run to {count} cells, beyond its world's {most}"));
        }
        let bytes = self.take(count.div_ceil(8))?;
        Ok((0..count).map(|cell| bytes[cell / 8] & 1 << (cell % 8) != 0).collect())
    }

    fn fields(&mut self) -> Result<Fields, String> {
        let mut values = BTreeMap::new();
        let count = self.count()?;
        for _ in 0..count {
            let name = self.text()?;
            let value = match self.choice(&[KIND_F32, KIND_F64, KIND_U32, KIND_U64], &format!("the kind of number of its field {name}"))? {
                KIND_F32 => Value::F32(self.f32()?),
                KIND_F64 => Value::F64(self.f64()?),
                KIND_U32 => Value::U32(self.u32()?),
                _ => Value::U64(self.u64()?),
            };
            if values.insert(name.clone(), value).is_some() {
                return Err(format!("{} holds its field {name} twice", section_name(self.section)));
            }
        }
        Ok(Fields { values, section: self.section, asked: BTreeSet::new() })
    }

    /// An error unless every byte of the section was read.
    fn done(&self) -> Result<(), String> {
        let left = self.bytes.len() - self.at;
        if left > 0 {
            return Err(format!("{} is {left} bytes longer than what it holds", section_name(self.section)));
        }
        Ok(())
    }
}

/// A land file taken apart: what its head says, and each section's tag and body in the file's order.
struct Sections<'a> {
    format: u32,
    version: String,
    found: Vec<(Tag, &'a [u8])>,
}

/// A section as the file holds it: its tag and length as its head holds them, the checksum written
/// with it, and its body.
struct Stored<'a> {
    tag: Tag,
    tag_and_length: &'a [u8],
    checksum: u64,
    body: &'a [u8],
}

/// The head and the sections of a land file, every checksum verified, or the fault in words. A
/// section is damaged when its bytes do not give its checksum; the first damaged in the file's
/// order is said before a fault found after it, which the damage may have caused (a length damaged
/// moves where the next section seems to start), so a file cut short is said as cut short only
/// when every section it still holds whole is as it was written.
fn sections(bytes: &[u8]) -> Result<Sections<'_>, String> {
    if bytes.len() < MAGIC.len() {
        return Err(format!("it is {} bytes long, too short to be a land file", bytes.len()));
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err("it does not begin with the 8 bytes every land file begins with, so it is not a land file".into());
    }
    let mut head = Read::new(*b"head", &bytes[MAGIC.len()..]);
    let format = head.u32().map_err(|_| "its head ends before its format".to_string())?;
    if format != LAND_FORMAT {
        let which = if format > LAND_FORMAT { "a newer format than" } else { "a format other than" };
        return Err(format!("it was written in format {format}, {which} format {LAND_FORMAT}, which this Landform reads"));
    }
    let version = head.text().map_err(|_| "its head ends before the version of Landform that wrote it, or holds it as text that is not UTF-8".to_string())?;
    let head_end = MAGIC.len() + head.at;
    let written = head.u64().map_err(|_| "its head ends before its checksum, so it may have been cut short".to_string())?;
    if checksum(&[&bytes[MAGIC.len()..head_end]]) != written {
        return Err(damaged("its head"));
    }
    let mut at = head_end + CHECKSUM_BYTES;
    let mut stored = Vec::new();
    let mut fault = None;
    while at < bytes.len() {
        let left = bytes.len() - at;
        if left < SECTION_HEAD_BYTES {
            fault = Some(format!("it ends {left} bytes into the head of a section, so it may have been cut short"));
            break;
        }
        let tag_and_length = &bytes[at..at + TAG_BYTES + LENGTH_BYTES];
        let tag: Tag = tag_and_length[..TAG_BYTES].try_into().expect("a tag of 4 bytes");
        let length = u64::from_le_bytes(tag_and_length[TAG_BYTES..].try_into().expect("a length of 8 bytes"));
        let checksum = u64::from_le_bytes(bytes[at + TAG_BYTES + LENGTH_BYTES..at + SECTION_HEAD_BYTES].try_into().expect("a checksum of 8 bytes"));
        let (start, room) = (at + SECTION_HEAD_BYTES, (left - SECTION_HEAD_BYTES) as u64);
        if length > room {
            fault = Some(format!("{} runs {} bytes past the end of the file, so it may have been cut short", section_name(tag), length - room));
            break;
        }
        let end = start + length as usize;
        stored.push(Stored { tag, tag_and_length, checksum, body: &bytes[start..end] });
        at = end;
    }
    // Each section's checksum from its own bytes alone, side by side on the pool's threads; the
    // first damaged in the file's order is the one said, on any count of threads.
    let first_damaged = stored.par_iter().position_first(|section| section_checksum(section.tag_and_length, section.body) != section.checksum);
    if let Some(index) = first_damaged {
        return Err(damaged(&section_name(stored[index].tag)));
    }
    if let Some(fault) = fault {
        return Err(fault);
    }
    Ok(Sections { format, version, found: stored.into_iter().map(|section| (section.tag, section.body)).collect() })
}

/// The one section of `tag`, to be read, or an error saying the file holds none.
fn one_section<'a>(by_tag: &BTreeMap<Tag, Vec<&'a [u8]>>, tag: Tag) -> Result<Read<'a>, String> {
    match by_tag.get(&tag) {
        Some(bodies) => Ok(Read::new(tag, bodies[0])),
        None => Err(format!("it holds no {} section ({}), so it may have been cut short", tag_text(tag), words(tag))),
    }
}

/// An error unless `ASKD`, where the file holds it beside `MARK`, stands just before `MARK`, where
/// `to_bytes` writes it (stage 2's ruling R1): so a waiting land's file cut anywhere is
/// refused, and none can be read as a land with nothing waiting. `order` is the file's tags in its
/// order, each once but `LAYR`; a file without `MARK` is refused later for lacking it.
fn asked_in_place(order: &[Tag]) -> Result<(), String> {
    let Some(at) = order.iter().position(|&tag| tag == ASKED) else { return Ok(()) };
    if !order.contains(&MARKS) || order.get(at + 1) == Some(&MARKS) {
        return Ok(());
    }
    let stands = order.get(at + 1).map_or_else(|| "last in the file".to_string(), |&next| format!("just before {}", section_name(next)));
    Err(format!("{} stands {stands}, where a land file holds it just before {}", section_name(ASKED), section_name(MARKS)))
}

/// The land in `bytes` (the module's **Reading**), or the fault in words.
pub(crate) fn from_bytes(bytes: &[u8]) -> Result<LandFile, String> {
    let Sections { format, version, found } = sections(bytes)?;
    let order: Vec<Tag> = found.iter().map(|&(tag, _)| tag).collect();
    let mut skipped = Vec::new();
    let mut by_tag: BTreeMap<Tag, Vec<&[u8]>> = BTreeMap::new();
    for (tag, body) in found {
        if known_sections().any(|(known, _)| *known == tag) {
            by_tag.entry(tag).or_default().push(body);
        } else {
            skip(&mut skipped, Skipped::Section(tag_text(tag)));
        }
    }
    for (tag, bodies) in &by_tag {
        if *tag != LAYER && bodies.len() > 1 {
            return Err(format!("it holds {} {} sections ({}), where a land has one", bodies.len(), tag_text(*tag), words(*tag)));
        }
    }
    asked_in_place(&order)?;
    let one = |tag: Tag| one_section(&by_tag, tag);

    let mut input = one(DRAWING)?;
    let (size_m, rivers, lakes, high_ground) = read_drawing_but_country(&mut input)?;
    input.done()?;
    let mut input = one(COUNTRY)?;
    let country = read_kind(&mut input, &mut skipped)?;
    input.done()?;
    let drawing = Drawing { size_m, high_ground, rivers, lakes, country };

    let mut input = one(SEED)?;
    let seed = input.u64()?;
    input.done()?;

    let mut input = one(CONFIG)?;
    let mut fields = input.fields()?;
    input.done()?;
    let (size_cells, cell_size_m, detail_amplitude_m) = (fields.get("size_cells")?, fields.get("cell_size_m")?, fields.get("detail_amplitude_m")?);
    let pick = read_pick(&mut fields)?;
    skip_fields(&mut skipped, &fields);
    let mut input = one(GROUND_RULES)?;
    let mut fields = input.fields()?;
    let rules = read_ground_rules(&mut fields)?;
    input.done()?;
    skip_fields(&mut skipped, &fields);
    let mut input = one(WATER_RULES)?;
    let mut fields = input.fields()?;
    let water_rules = read_water_rules(&mut fields)?;
    input.done()?;
    skip_fields(&mut skipped, &fields);
    let config = GenConfig { size_cells, cell_size_m, detail_amplitude_m, pick, rules, water: water_rules };
    // Checked before anything else is read, so every grid and list after it is read against a
    // drawing of a valid size.
    check_settings(&drawing, &config)?;
    let world_cells = drawing.size_m as usize * drawing.size_m as usize;

    let mut input = one(REACH)?;
    let reach_m = input.f32()?;
    input.done()?;
    let mut input = one(MODEL)?;
    let model_heights = input.grid()?;
    input.done()?;
    let mut input = one(FIRST_LAKES)?;
    let first_lake_levels = input.list(|input| {
        let (drawn, level_m) = (input.count()?, input.f32()?);
        Ok(GrownLake { drawn, level_m })
    })?;
    input.done()?;

    let mut input = one(HEIGHTS)?;
    let heights = input.grid()?;
    input.done()?;
    let mut input = one(GROUND)?;
    let (width, height, cell_size, cells) = input.sides()?;
    let data = input.items(cells, 3)?.as_chunks::<3>().0.iter().map(|&[strongest, second, blend]| GroundCell { strongest, second, blend }).collect();
    let ground = Grid { width, height, cell_size, data };
    input.done()?;
    let mut layers = BTreeMap::new();
    for body in by_tag.get(&LAYER).map_or(&[][..], Vec::as_slice) {
        let mut input = Read::new(LAYER, body);
        let name = input.text()?;
        let layer = input.grid()?;
        input.done()?;
        if !LAYER_NAMES.contains(&name.as_str()) && name != WATER_LAYER {
            skip(&mut skipped, Skipped::Layer(name));
            continue;
        }
        if layers.insert(name.clone(), layer).is_some() {
            return Err(format!("it holds the layer {name} twice"));
        }
    }
    // In the order of their names, the order they are written in, so a file cut short names the
    // first layer it lacks.
    let needed: BTreeSet<&str> = LAYER_NAMES.iter().copied().chain([WATER_LAYER]).collect();
    for name in needed {
        if !layers.contains_key(name) {
            return Err(format!("it holds no {} section for the world's layer {name}, so it may have been cut short", tag_text(LAYER)));
        }
    }

    let mut input = one(WATER)?;
    let courses = input.list(|input| {
        let drawn = input.index("whether a course is a drawn river's")?;
        let from_lake = input.index("whether a course flows out of a lake")?;
        let kind = input.choice(&[END_EDGE, END_RIVER, END_LAKE], "how a course ends")?;
        let index = input.count()?;
        let end = match kind {
            END_EDGE => CourseEnd::Edge,
            END_RIVER => CourseEnd::River(index),
            _ => CourseEnd::Lake(index),
        };
        let points = input.list(|input| {
            let [x_m, z_m, surface_m, width_m, depth_m] = [input.f32()?, input.f32()?, input.f32()?, input.f32()?, input.f32()?];
            Ok(CoursePoint { x_m, z_m, surface_m, width_m, depth_m })
        })?;
        Ok(RiverCourse { drawn, from_lake, points, end })
    })?;
    let water_lakes = input.list(|input| {
        let drawn = input.count()?;
        let [level_m, x_m, z_m, area_m2] = [input.f32()?, input.f32()?, input.f32()?, input.f32()?];
        Ok(LakeWater { drawn, level_m, x_m, z_m, area_m2 })
    })?;
    input.done()?;
    let water = Water { rivers: courses, lakes: water_lakes };

    let mut input = one(NOTES)?;
    let notes = input.list(Read::text)?;
    let notes_about = input.list(|input| {
        let kind = input.choice(&[ABOUT_LAND, ABOUT_RIVER, ABOUT_LAKE], "what a note is about")?;
        let index = input.count()?;
        Ok(match kind {
            ABOUT_LAND => About::Land,
            ABOUT_RIVER => About::River(index),
            _ => About::Lake(index),
        })
    })?;
    input.done()?;

    let mut input = one(META)?;
    let meta_seed = input.u64()?;
    let source_id = input.text()?;
    let source = input.count()?;
    let (center_x, center_y, rotation_rad) = (input.f64()?, input.f64()?, input.f64()?);
    let mirrored = input.choice(&[0, 1], "whether the window is mirrored")? == 1;
    let window_size_m = input.f64()?;
    let base_elevation_m = input.f32()?;
    let north = (input.f32()?, input.f32()?);
    let mut fields = input.fields()?;
    let timings = read_timings(&mut fields)?;
    input.done()?;
    skip_fields(&mut skipped, &fields);
    let window = Window { source, center_x, center_y, rotation_rad, mirrored, size_m: window_size_m };
    let meta = WorldMeta { seed: meta_seed, source_id, window, base_elevation_m, north, timings };

    // The drawing asked for, which only a waiting land's file holds, read in the file's order;
    // without it the land asks for its own drawing, the value under which nothing waits. Checked
    // by `check` with the rest.
    let asked = match by_tag.get(&ASKED) {
        None => drawing.clone(),
        Some(bodies) => {
            let mut input = Read::new(ASKED, bodies[0]);
            let (size_m, rivers, lakes, high_ground) = read_drawing_but_country(&mut input)?;
            let country = read_kind(&mut input, &mut skipped)?;
            input.done()?;
            Drawing { size_m, high_ground, rivers, lakes, country }
        }
    };

    let mut input = one(MARKS)?;
    let (held, by_hand) = (input.flags(world_cells, "held")?, input.flags(world_cells, "hand")?);
    input.done()?;

    let world = World { heights, layers, ground, meta, notes, notes_about, water };
    let land = Land { drawing, asked, seed, config, model_heights, first_lake_levels, world, marks: Marks { held, by_hand }, reach_m };
    check(&land)?;
    Ok(LandFile { land, format, version, skipped })
}

/// A drawing but its kind of country: its size, its rivers, its lakes and its picture of high
/// ground, as `read_drawing_but_country` reads them.
type DrawingButCountry = (u32, Vec<River>, Vec<Lake>, Option<Grid<f32>>);

/// A drawing but its kind of country, as `DRAW`'s body holds it and `ASKD`'s begins with
/// (`write_drawing`).
fn read_drawing_but_country(input: &mut Read) -> Result<DrawingButCountry, String> {
    let size_m = input.u32()?;
    let rivers = input.list(|input| {
        let width_m = input.f32()?;
        let points = input.list(|input| Ok((input.f64()?, input.f64()?)))?;
        Ok(River { points, width_m })
    })?;
    let lakes = input.list(|input| {
        let (x_m, z_m, radius_m) = (input.f64()?, input.f64()?, input.f32()?);
        Ok(Lake { x_m, z_m, radius_m })
    })?;
    let high_ground = match input.choice(&[NONE, SOME], "whether there is a picture of high ground")? {
        NONE => None,
        _ => Some(input.grid()?),
    };
    Ok((size_m, rivers, lakes, high_ground))
}

/// A drawing's kind of country, a record of `Country`'s fields, as `CTRY`'s body holds it and
/// `ASKD`'s ends with; the fields this Landform does not know are put among the parts skipped.
fn read_kind(input: &mut Read, skipped: &mut Vec<Skipped>) -> Result<Country, String> {
    let mut fields = input.fields()?;
    let country = read_country(&mut fields)?;
    skip_fields(skipped, &fields);
    Ok(country)
}

/// `part` put among the parts skipped, unless it is there already.
fn skip(skipped: &mut Vec<Skipped>, part: Skipped) {
    if !skipped.contains(&part) {
        skipped.push(part);
    }
}

/// The fields of a record read that this Landform does not know, put among the parts skipped.
fn skip_fields(skipped: &mut Vec<Skipped>, fields: &Fields) {
    for part in fields.skipped() {
        skip(skipped, part);
    }
}

/// An error naming `what` unless every value of `data` is finite.
fn all_finite(data: &[f32], what: &str) -> Result<(), String> {
    match data.iter().position(|value| !value.is_finite()) {
        Some(cell) => Err(format!("{what} hold a value that is not a finite number (value {cell}, counted from 0)")),
        None => Ok(()),
    }
}

/// An error unless the drawing and the settings are valid as `Land::grow` checks them.
fn check_settings(drawing: &Drawing, config: &GenConfig) -> Result<(), String> {
    check_drawn(drawing, config).map_err(|error| match error {
        LandformError::BadConfig(reason) => format!("the land's drawing or settings are not valid: {reason}"),
        other => other.to_string(),
    })
}

/// An error naming the `ASKD` section unless `asked`, the drawing a land of `drawing` asks for, is
/// one it may ask for (`Land::ask`): a drawing `Drawing::validate` takes, as the land's own drawing
/// is checked, of the land's size.
fn check_asked(asked: &Drawing, drawing: &Drawing) -> Result<(), String> {
    let section = section_name(ASKED);
    asked.validate().map_err(|error| match error {
        LandformError::BadConfig(reason) => format!("{section} holds a drawing that is not valid: {reason}"),
        other => format!("{section} holds a drawing that is not valid: {other}"),
    })?;
    if asked.size_m != drawing.size_m {
        return Err(format!("{section} holds a drawing {} m across, where the land is {} m across and asks only for a drawing of its size", asked.size_m, drawing.size_m));
    }
    Ok(())
}

/// An error unless `land`, as read, is a land `Land::grow` and an edit could have made, its drawing
/// and settings already checked (`check_settings`): the drawing it asks for valid as its drawing is,
/// and of its size (`check_asked`); its model's heights on its drawing's model grid
/// and finite; its first lakes and its world's courses, lakes and notes naming rivers and lakes the
/// drawing and the world hold, no lake of the drawing given twice among the first lakes or the
/// water's lakes, and no course ending on itself (so no step reads one part as another); its world
/// in the model's frame, grown from the land's seed, its window and north finite, every grid its
/// drawing's size in cells of 1 m, its heights and every layer's cells finite, its ground map
/// naming materials that exist and its courses' points finite and on the world's cells, their
/// water above 0 m and at most the land's size wide
/// and no less than 0 m deep; its marks no longer than its world; its reach finite. So a land read
/// cannot make an edit, a hold, the water's surface or a picture of it panic.
pub(super) fn check(land: &Land) -> Result<(), String> {
    let (drawing, world) = (&land.drawing, &land.world);
    check_asked(&land.asked, drawing)?;
    let side = model_side(drawing.size_m);
    let model = &land.model_heights;
    if (model.width, model.height, model.cell_size) != (side, side, GROW_CELL_M) {
        return Err(format!(
            "the model's heights are {} x {} cells of {} m, where its drawing's model grid is {side} x {side} cells of {GROW_CELL_M} m",
            model.width, model.height, model.cell_size
        ));
    }
    all_finite(&model.data, "the model's heights")?;
    let mut first = BTreeSet::new();
    for lake in &land.first_lake_levels {
        if lake.drawn >= drawing.lakes.len() || !lake.level_m.is_finite() {
            return Err(format!("the first lakes' levels name lake {} of a drawing of {} lakes, or a level that is not finite", lake.drawn.saturating_add(1), drawing.lakes.len()));
        }
        // An edit takes these as its kept model's lakes, and would find one of two levels.
        if !first.insert(lake.drawn) {
            return Err(format!("the first lakes' levels give lake {} twice", lake.drawn + 1));
        }
    }

    let size = drawing.size_m as usize;
    let fits = |what: String, width: usize, height: usize, cell_size: f32| -> Result<(), String> {
        if (width, height, cell_size) != (size, size, 1.0) {
            return Err(format!("{what} is {width} x {height} cells of {cell_size} m, where its drawing asks for {size} x {size} cells of 1 m"));
        }
        Ok(())
    };
    fits("the world's heights grid".into(), world.heights.width, world.heights.height, world.heights.cell_size)?;
    fits("the ground map".into(), world.ground.width, world.ground.height, world.ground.cell_size)?;
    for (name, layer) in &world.layers {
        fits(format!("the layer {name}"), layer.width, layer.height, layer.cell_size)?;
    }
    // The water's surface, a hold and an edit read the layers' values (the `water` layer's depth
    // tells a course's water and a lake's), and a picture of them would draw them.
    for (name, layer) in &world.layers {
        all_finite(&layer.data, &format!("the layer {name}'s cells"))?;
    }
    if world.meta.base_elevation_m != 0.0 {
        return Err(format!("its world is not in the land model's frame: its base elevation is {} m, not 0", world.meta.base_elevation_m));
    }
    all_finite(&world.heights.data, "the world's heights")?;
    if let Some(cell) = world.ground.data.iter().position(|cell| usize::from(cell.strongest.max(cell.second)) >= Material::ALL.len()) {
        return Err(format!("the ground map names a material beyond the {} there are, at cell {cell}", Material::ALL.len()));
    }

    let (courses, lakes) = (world.water.rivers.len(), world.water.lakes.len());
    let half = size as f32 / 2.0;
    for (index, course) in world.water.rivers.iter().enumerate() {
        let named = |what: &str| format!("the water's course {} {what}", index + 1);
        if course.drawn.is_some_and(|river| river >= drawing.rivers.len()) || course.from_lake.is_some_and(|lake| lake >= lakes) {
            return Err(named("names a river of the drawing or a lake of the water that is not there"));
        }
        let end_fits = match course.end {
            CourseEnd::Edge => true,
            CourseEnd::River(other) => other < courses,
            CourseEnd::Lake(lake) => lake < lakes,
        };
        if !end_fits {
            return Err(named("ends on a course or a lake that is not there"));
        }
        // No course flows into itself; the water's surface would read it as the river it goes on as.
        if course.end == CourseEnd::River(index) {
            return Err(named("ends on itself"));
        }
        if course.points.is_empty() {
            return Err(named("has no points"));
        }
        let on_the_world = |at: f32| at.is_finite() && (-half..=half - 1.0).contains(&at);
        let point_fits = |point: &CoursePoint| {
            on_the_world(point.x_m) && on_the_world(point.z_m) && [point.surface_m, point.width_m, point.depth_m].iter().all(|value| value.is_finite())
        };
        if !course.points.iter().all(point_fits) {
            return Err(named("has a point that is not finite or not on the world's cells"));
        }
        // `water_surface` lays a course out in buckets of its widest water's reach.
        if let Some(point) = course.points.iter().find(|point| !(point.width_m > 0.0 && point.width_m <= size as f32)) {
            return Err(named(&format!("has a point whose water is {} m wide, where a course's water is above 0 m and at most the land's {size} m wide", point.width_m)));
        }
        if let Some(point) = course.points.iter().find(|point| point.depth_m < 0.0) {
            return Err(named(&format!("has a point whose water is {} m deep, below 0 m", point.depth_m)));
        }
    }
    for (index, lake) in world.water.lakes.iter().enumerate() {
        if lake.drawn >= drawing.lakes.len() || ![lake.level_m, lake.x_m, lake.z_m, lake.area_m2].iter().all(|value| value.is_finite()) {
            return Err(format!("the water's lake {} names a lake the drawing does not hold, or a value that is not finite", index + 1));
        }
        // An edit keeps a drawn lake's water by its place in the drawing, and a hold finds its water
        // by its level, so a drawn lake given twice would be read as one of the two.
        if let Some(earlier) = world.water.lakes[..index].iter().position(|other| other.drawn == lake.drawn) {
            return Err(format!("the water's lakes {} and {} are both lake {} of the drawing", earlier + 1, index + 1, lake.drawn + 1));
        }
    }
    let (notes, subjects) = (world.notes.len(), world.notes_about.len());
    if subjects != notes {
        return Err(format!("it gives its {notes} notes {subjects} subjects, not one each"));
    }
    let in_drawing = |about: &About| match *about {
        About::Land => true,
        About::River(index) => index < drawing.rivers.len(),
        About::Lake(index) => index < drawing.lakes.len(),
    };
    if !world.notes_about.iter().all(in_drawing) {
        return Err("a note is about a river or lake its drawing does not hold".into());
    }
    let meta = &world.meta;
    let window = &meta.window;
    if ![window.center_x, window.center_y, window.rotation_rad, window.size_m].iter().all(|value| value.is_finite()) || !(meta.north.0.is_finite() && meta.north.1.is_finite()) {
        return Err("its world's window or north is not a finite number".into());
    }
    // An edit grows from the land's seed, and an export says the world's.
    if meta.seed != land.seed {
        return Err(format!("its world was grown from seed {}, where the land's seed is {}", meta.seed, land.seed));
    }
    // The marks were refused as they were read when they run beyond the world's cells.
    debug_assert!(land.marks.held.len() <= size * size && land.marks.by_hand.len() <= size * size);
    if !land.reach_m.is_finite() {
        return Err(format!("its reach is {} m, not a finite number", land.reach_m));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trial::example_edit;
    use std::sync::atomic::AtomicBool;
    use std::sync::OnceLock;

    /// The example's size and seed for these tests: small enough to grow and edit in a moment.
    const SIZE_M: u32 = 512;
    const EXAMPLE_SEED: u64 = 11;
    /// A disc held far from the example's water at 512 m, beyond `BAND_M` of every river and the lake.
    const HELD_AT: (f64, f64) = (150.0, -180.0);
    const HELD_RADIUS_M: f64 = 20.0;
    /// A square shaped by hand, likewise far from the water: its first corner and its side in cells.
    const BY_HAND_AT: (usize, usize) = (100, 400);
    const BY_HAND_SIDE: usize = 10;
    /// Original land files retained byte for byte for format compatibility tests.
    const STAGE1_FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/stage1");
    /// Stage 1's land files in it, each with its length in bytes.
    const STAGE1_FIXTURES: [(&str, usize); 3] = [("valley_128.lfland", 738_449), ("valley_128_held.lfland", 738_449), ("twoout_128.lfland", 738_159)];

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// One flag a world cell of a land `size` cells a side: the cells whose place lies within
    /// `radius_m` of `(x, z)`.
    fn disc(size: usize, (x, z): (f64, f64), radius_m: f64) -> Vec<bool> {
        let half = size as f64 / 2.0;
        (0..size * size).map(|cell| ((cell % size) as f64 - half - x).hypot((cell / size) as f64 - half - z) <= radius_m).collect()
    }

    /// The land written in these tests: the example at 512 m in hills from seed 11, edited once (the
    /// brook's point moved, `a`), with a disc held and a square marked as shaped by hand.
    fn written() -> &'static Land {
        static LAND: OnceLock<Land> = OnceLock::new();
        LAND.get_or_init(|| {
            let grown = Land::grow(&Drawing::example_valley(SIZE_M), EXAMPLE_SEED, &GenConfig::default(), &never()).expect("the example grows");
            let edited = example_edit(&grown.drawing, "a").expect("a is an edit of the example");
            let mut land = grown.edit(&edited, &never()).expect("the edit is taken").land;
            let size = SIZE_M as usize;
            land.hold(&disc(size, HELD_AT, HELD_RADIUS_M), true);
            for row in BY_HAND_AT.1..BY_HAND_AT.1 + BY_HAND_SIDE {
                for column in BY_HAND_AT.0..BY_HAND_AT.0 + BY_HAND_SIDE {
                    land.marks.by_hand[row * size + column] = true;
                }
            }
            land
        })
    }

    fn f32_bits(data: &[f32]) -> Vec<u32> {
        data.iter().map(|value| value.to_bits()).collect()
    }

    fn grid_bits(grid: &Grid<f32>) -> (usize, usize, u32, Vec<u32>) {
        (grid.width, grid.height, grid.cell_size.to_bits(), f32_bits(&grid.data))
    }

    /// A record's fields with their values' bits.
    fn record_bits(fields: Vec<(&'static str, Value)>) -> Vec<(&'static str, u64)> {
        let bits = |value: Value| match value {
            Value::F32(value) => u64::from(value.to_bits()),
            Value::F64(value) => value.to_bits(),
            Value::U32(value) => u64::from(value),
            Value::U64(value) => value,
        };
        fields.into_iter().map(|(name, value)| (name, bits(value))).collect()
    }

    /// The parts of two lands that differ, every `f32` and `f64` compared by its bits and every list
    /// in its order; the world's timings too unless `timings` is false.
    fn differences(a: &Land, b: &Land, timings: bool) -> Vec<&'static str> {
        // Every field is named, so a field added to the land fails to compile until it is compared.
        let Land { drawing: _, asked: _, seed: _, config: _, model_heights: _, first_lake_levels: _, world: _, marks: _, reach_m: _ } = a;
        let mut found = Vec::new();
        let mut differ = |name: &'static str, same: bool| {
            if !same {
                found.push(name);
            }
        };
        let rivers = |drawing: &Drawing| -> Vec<(u32, Vec<(u64, u64)>)> {
            drawing.rivers.iter().map(|river| (river.width_m.to_bits(), river.points.iter().map(|&(x, z)| (x.to_bits(), z.to_bits())).collect())).collect()
        };
        let lakes = |drawing: &Drawing| -> Vec<(u64, u64, u32)> { drawing.lakes.iter().map(|lake| (lake.x_m.to_bits(), lake.z_m.to_bits(), lake.radius_m.to_bits())).collect() };
        differ("drawing's size", a.drawing.size_m == b.drawing.size_m);
        differ("drawing's picture", a.drawing.high_ground.as_ref().map(grid_bits) == b.drawing.high_ground.as_ref().map(grid_bits));
        differ("drawing's rivers", rivers(&a.drawing) == rivers(&b.drawing));
        differ("drawing's lakes", lakes(&a.drawing) == lakes(&b.drawing));
        differ("drawing's country", record_bits(country_fields(&a.drawing.country)) == record_bits(country_fields(&b.drawing.country)));
        differ("asked drawing", drawing_bytes(&a.asked) == drawing_bytes(&b.asked));
        differ("seed", a.seed == b.seed);
        differ("settings", record_bits(config_fields(&a.config)) == record_bits(config_fields(&b.config)));
        differ("ground rules", record_bits(ground_rule_fields(&a.config.rules)) == record_bits(ground_rule_fields(&b.config.rules)));
        differ("water rules", record_bits(water_rule_fields(&a.config.water)) == record_bits(water_rule_fields(&b.config.water)));
        differ("model's heights", grid_bits(&a.model_heights) == grid_bits(&b.model_heights));
        let first = |land: &Land| -> Vec<(usize, u32)> { land.first_lake_levels.iter().map(|lake| (lake.drawn, lake.level_m.to_bits())).collect() };
        differ("first lakes' levels", first(a) == first(b));
        differ("reach", a.reach_m.to_bits() == b.reach_m.to_bits());
        differ("marks", a.marks == b.marks);
        let (x, y) = (&a.world, &b.world);
        differ("heights", grid_bits(&x.heights) == grid_bits(&y.heights));
        let ground = |world: &World| (world.ground.width, world.ground.height, world.ground.cell_size.to_bits(), world.ground.data.clone());
        differ("ground map", ground(x) == ground(y));
        type GridBits = (usize, usize, u32, Vec<u32>);
        let layers = |world: &World| -> Vec<(String, GridBits)> { world.layers.iter().map(|(name, grid)| (name.clone(), grid_bits(grid))).collect() };
        differ("layers", layers(x) == layers(y));
        type CourseBits = (Option<usize>, Option<usize>, CourseEnd, Vec<[u32; 5]>);
        let courses = |world: &World| -> Vec<CourseBits> {
            let point = |p: &CoursePoint| [p.x_m, p.z_m, p.surface_m, p.width_m, p.depth_m].map(f32::to_bits);
            world.water.rivers.iter().map(|course| (course.drawn, course.from_lake, course.end, course.points.iter().map(point).collect())).collect()
        };
        differ("courses", courses(x) == courses(y));
        let water_lakes = |world: &World| -> Vec<(usize, [u32; 4])> { world.water.lakes.iter().map(|l| (l.drawn, [l.level_m, l.x_m, l.z_m, l.area_m2].map(f32::to_bits))).collect() };
        differ("lakes", water_lakes(x) == water_lakes(y));
        differ("notes", x.notes == y.notes && x.notes_about == y.notes_about);
        let meta = |world: &World| {
            let (meta, window) = (&world.meta, &world.meta.window);
            (
                meta.seed,
                meta.source_id.clone(),
                (window.source, window.center_x.to_bits(), window.center_y.to_bits(), window.rotation_rad.to_bits(), window.mirrored, window.size_m.to_bits()),
                meta.base_elevation_m.to_bits(),
                (meta.north.0.to_bits(), meta.north.1.to_bits()),
            )
        };
        differ("meta", meta(x) == meta(y));
        differ("timings", !timings || record_bits(timing_fields(&x.meta.timings)) == record_bits(timing_fields(&y.meta.timings)));
        found
    }

    /// Where each section of `bytes` starts (its tag), with its tag and its body's length.
    fn starts(bytes: &[u8]) -> Vec<(usize, Tag, usize)> {
        let Sections { version, found, .. } = sections(bytes).expect("the file's sections");
        let mut at = head_bytes(&version);
        found
            .into_iter()
            .map(|(tag, body)| {
                let start = at;
                at += SECTION_HEAD_BYTES + body.len();
                (start, tag, body.len())
            })
            .collect()
    }

    /// The bytes of a file's head, the magic to the head's checksum, for `version`.
    fn head_bytes(version: &str) -> usize {
        MAGIC.len() + size_of::<u32>() + size_of::<u64>() + version.len() + CHECKSUM_BYTES
    }

    /// A section as a file holds it: its tag, its body's length, its checksum and its body.
    fn section_bytes(tag: Tag, body: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        let sum = section_checksum(&out, body);
        out.extend_from_slice(&sum.to_le_bytes());
        out.extend_from_slice(body);
        out
    }

    /// `bytes` with the body of the first section of `tag` replaced by `body`, with its checksum.
    fn replaced(bytes: &[u8], tag: Tag, body: &[u8]) -> Vec<u8> {
        let &(start, _, length) = starts(bytes).iter().find(|(_, found, _)| *found == tag).expect("the section");
        let mut out = bytes[..start].to_vec();
        out.extend_from_slice(&section_bytes(tag, body));
        out.extend_from_slice(&bytes[start + SECTION_HEAD_BYTES + length..]);
        out
    }

    /// `bytes` with a section of `tag` and `body`, with its checksum, put before the first section of
    /// `before`, or at the end for `None`.
    fn inserted(bytes: &[u8], before: Option<Tag>, tag: Tag, body: &[u8]) -> Vec<u8> {
        let at = before.map_or(bytes.len(), |before| starts(bytes).iter().find(|(_, found, _)| *found == before).expect("the section").0);
        let mut out = bytes[..at].to_vec();
        out.extend_from_slice(&section_bytes(tag, body));
        out.extend_from_slice(&bytes[at..]);
        out
    }

    /// The body of the first section of `tag` in `bytes`.
    fn body_of(bytes: &[u8], tag: Tag) -> Vec<u8> {
        sections(bytes).expect("the file's sections").found.iter().find(|(found, _)| *found == tag).expect("the section").1.to_vec()
    }

    fn record_bytes(fields: Vec<(&'static str, Value)>) -> Vec<u8> {
        let mut out = Bytes::new();
        out.record(&Record(fields));
        out.0
    }

    /// The reason the reader refuses `bytes` for; it fails the test if they are read.
    fn refusal(bytes: &[u8]) -> String {
        match from_bytes(bytes) {
            Ok(_) => panic!("{} bytes were read as a land", bytes.len()),
            Err(reason) => reason,
        }
    }

    fn assert_says(reason: &str, words: &str) {
        assert!(reason.contains(words), "\"{reason}\" does not say \"{words}\"");
        assert!(!reason.contains('\n') && !reason.ends_with('.'), "\"{reason}\" is not a clause of one sentence");
    }

    #[test]
    fn a_land_written_and_read_is_the_land_written_bit_for_bit() {
        let land = written();
        // What the test covers: an edited land with held and hand-shaped cells, rivers, a lake and notes.
        assert!(land.marks.held.contains(&true) && land.marks.by_hand.contains(&true));
        assert!(!land.world.water.rivers.is_empty() && !land.world.water.lakes.is_empty() && !land.first_lake_levels.is_empty());
        assert!(land.drawing.high_ground.is_some() && land.world.layers.len() == 5);
        assert!(land.world.meta.timings.total_ms > 0.0, "the timings are written and read too");

        let bytes = to_bytes(land);
        let file = from_bytes(&bytes).expect("the land is read");
        assert_eq!(differences(land, &file.land, true), Vec::<&str>::new());
        assert_eq!((file.format, file.version.as_str()), (LAND_FORMAT, VERSION));
        assert!(to_bytes(&file.land) == bytes, "the land read is written as the same bytes");

        // Through a file on the disk, with nothing left beside it.
        let dir = std::env::temp_dir().join(format!("landform_land_file_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("valley.lfland");
        land.write(&path).unwrap();
        assert!(fs::read(&path).unwrap() == bytes);
        let read = Land::read(&path).unwrap();
        assert_eq!(differences(land, &read, true), Vec::<&str>::new());
        let names: Vec<String> = fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(names, ["valley.lfland"]);
        println!("the example at {SIZE_M} m, edited and held: {} bytes", bytes.len());
    }

    #[test]
    fn an_edit_of_a_land_read_is_the_same_edit_of_the_land_written() {
        let land = written();
        let read = from_bytes(&to_bytes(land)).expect("the land is read").land;
        let edited = example_edit(&land.drawing, "a").unwrap();
        let (a, b) = (land.edit(&edited, &never()).expect("the edit is taken"), read.edit(&edited, &never()).expect("the edit is taken"));
        assert!(a.outline.inside_m2 > 0.0 && a.timings.grow_ms > 0.0, "the edit regrows something");
        assert_eq!(differences(&a.land, &b.land, false), Vec::<&str>::new());
        assert_eq!(f32_bits(&a.outline.weight.data), f32_bits(&b.outline.weight.data));
        assert_eq!(f32_bits(&a.outline.model_weight), f32_bits(&b.outline.model_weight));
        assert_eq!((a.troughs.clone(), a.pits_left, a.water.clone(), a.notes.clone(), a.notes_about.clone()), (b.troughs, b.pits_left, b.water, b.notes, b.notes_about));
        // And of a land read from the edited land's own file.
        let again = from_bytes(&to_bytes(&a.land)).expect("the edited land is read").land;
        let further = example_edit(&a.land.drawing, "c").unwrap();
        let (c, d) = (a.land.edit(&further, &never()).expect("taken"), again.edit(&further, &never()).expect("taken"));
        assert!(c.outline.inside_m2 > 0.0);
        assert_eq!(differences(&c.land, &d.land, false), Vec::<&str>::new());
    }

    #[test]
    fn a_file_cut_short_names_what_it_lacks_and_none_panics() {
        let bytes = to_bytes(written());
        let starts = starts(&bytes);
        assert_eq!(starts.len(), SECTIONS.len() + 4, "each section once, a layer section for each of the five layers");
        let layer_names: Vec<&str> = LAYER_NAMES.iter().copied().chain([WATER_LAYER]).collect::<BTreeSet<_>>().into_iter().collect();
        for (index, &(start, tag, length)) in starts.iter().enumerate() {
            let reason = refusal(&bytes[..start]);
            if tag == LAYER {
                let layers_before = starts[..index].iter().filter(|(_, earlier, _)| *earlier == LAYER).count();
                assert_says(&reason, &format!("holds no LAYR section for the world's layer {}, so it may have been cut short", layer_names[layers_before]));
            } else {
                assert_says(&reason, &format!("it holds no {} section ({}), so it may have been cut short", tag_text(tag), words(tag)));
            }
            assert_says(&refusal(&bytes[..start + 5]), "it ends 5 bytes into the head of a section");
            let cut = start + SECTION_HEAD_BYTES + length / 2;
            assert_says(&refusal(&bytes[..cut]), &format!("{} runs {} bytes past the end of the file", section_name(tag), length - length / 2));
        }
        let head = starts[0].0;
        for cut in 0..head {
            let reason = refusal(&bytes[..cut]);
            let ends = ["too short to be a land file", "its head ends before its format", "its head ends before the version", "its head ends before its checksum"];
            assert!(ends.iter().any(|words| reason.contains(words)), "{cut}: {reason}");
        }
    }

    #[test]
    fn a_wrong_mark_or_another_format_is_refused() {
        let bytes = to_bytes(written());
        let mut wrong = bytes.clone();
        wrong[6] = b'\n';
        assert_says(&refusal(&wrong), "it does not begin with the 8 bytes every land file begins with, so it is not a land file");
        for (format, words) in [(2u32, "it was written in format 2, a newer format than format 1, which this Landform reads"), (0, "it was written in format 0, a format other than format 1")] {
            let mut other = bytes.clone();
            other[8..12].copy_from_slice(&format.to_le_bytes());
            assert_says(&refusal(&other), words);
        }
    }

    #[test]
    fn a_section_that_runs_past_the_end_or_holds_more_is_refused() {
        let bytes = to_bytes(written());
        let all = starts(&bytes);
        for &(start, tag, _) in [all[0], all[all.len() - 1]].iter() {
            let mut longer = bytes.clone();
            let length = (bytes.len() - start - SECTION_HEAD_BYTES + 1000) as u64;
            longer[start + TAG_BYTES..start + TAG_BYTES + LENGTH_BYTES].copy_from_slice(&length.to_le_bytes());
            assert_says(&refusal(&longer), &format!("{} runs 1000 bytes past the end of the file", section_name(tag)));
        }
        let mut huge = bytes.clone();
        huge[all[0].0 + TAG_BYTES..all[0].0 + TAG_BYTES + LENGTH_BYTES].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_says(&refusal(&huge), "its DRAW section (the drawing) runs");
        let found = sections(&bytes).unwrap().found;
        let mut marks = found.iter().find(|(tag, _)| *tag == MARKS).unwrap().1.to_vec();
        marks.extend_from_slice(&[0, 0, 0]);
        assert_says(&refusal(&replaced(&bytes, MARKS, &marks)), "its MARK section (the marks) is 3 bytes longer than what it holds");
        let seed = found.iter().find(|(tag, _)| *tag == SEED).unwrap().1.to_vec();
        assert_says(&refusal(&inserted(&bytes, Some(CONFIG), SEED, &seed)), "it holds 2 SEED sections (the seed), where a land has one");
        assert_says(&refusal(&replaced(&bytes, SEED, &seed[..5])), "its SEED section (the seed) ends before what it holds");
        // Counts no section could hold, which must not be allocated for.
        for (width, height) in [(u64::MAX, u64::MAX), (1 << 40, 1 << 20)] {
            let mut model = Vec::new();
            model.extend_from_slice(&width.to_le_bytes());
            model.extend_from_slice(&height.to_le_bytes());
            model.extend_from_slice(&8f32.to_le_bytes());
            assert_says(&refusal(&replaced(&bytes, MODEL, &model)), "its MODL section (the model's heights) ends before what it holds");
        }
        let mut lakes = Vec::new();
        lakes.extend_from_slice(&u64::MAX.to_le_bytes());
        assert_says(&refusal(&replaced(&bytes, FIRST_LAKES, &lakes)), "its LAKE section (the first lakes' levels) ends before what it holds");
        // Marks of more cells than the world's, refused before they are made: one bit a cell would
        // make eight times the bytes the section holds.
        let cells = (SIZE_M * SIZE_M) as u64;
        let mut marks = Vec::new();
        marks.extend_from_slice(&u64::MAX.to_le_bytes());
        assert_says(&refusal(&replaced(&bytes, MARKS, &marks)), &format!("its held marks run to {} cells, beyond its world's {cells}", u64::MAX));
        let mut marks = Vec::new();
        marks.extend_from_slice(&0u64.to_le_bytes());
        marks.extend_from_slice(&(cells + 1).to_le_bytes());
        assert_says(&refusal(&replaced(&bytes, MARKS, &marks)), &format!("its hand marks run to {} cells, beyond its world's {cells}", cells + 1));
    }

    /// The example at 128 m in hills from seed 11 with a disc of 10 m held at (20, -30): a small
    /// land for a test that reads many files.
    fn small() -> &'static Land {
        static LAND: OnceLock<Land> = OnceLock::new();
        LAND.get_or_init(|| {
            let mut land = Land::grow(&Drawing::example_valley(SMALL_SIZE_M), EXAMPLE_SEED, &GenConfig::default(), &never()).expect("the example grows");
            land.hold(&disc(SMALL_SIZE_M as usize, (20.0, -30.0), 10.0), true);
            land
        })
    }

    /// The size of `small`'s land.
    const SMALL_SIZE_M: u32 = 128;

    #[test]
    fn a_damaged_file_is_refused_as_damaged() {
        let land = written();
        let bytes = to_bytes(land);
        let at = |tag: Tag| starts(&bytes).iter().find(|(_, found, _)| *found == tag).expect("the section").0 + SECTION_HEAD_BYTES;

        // The review's flip: one bit of the marks unholds a held cell.
        let first_held = land.marks.held.iter().position(|&held| held).expect("a held cell");
        let mut flipped = bytes.clone();
        flipped[at(MARKS) + size_of::<u64>() + first_held / 8] ^= 1 << (first_held % 8);
        assert_says(&refusal(&flipped), "its MARK section (the marks) is damaged, since its bytes do not give the checksum written with them");
        // One bit of a height: its exponent's lowest, which doubles or halves it.
        let mut flipped = bytes.clone();
        flipped[at(HEIGHTS) + 2 * size_of::<u64>() + size_of::<f32>() + 300 * size_of::<f32>() + 3] ^= 0x01;
        assert_says(&refusal(&flipped), "its HGHT section (the world's heights) is damaged");
        // One byte of a field's name, which would leave the field missing and read another as
        // unknown; and the same in a mountains land, which once read as hill country's relief.
        let name = b"relief_m";
        let mountains = Land::grow(&Drawing { country: Country::mountains(), ..Drawing::example_valley(256) }, EXAMPLE_SEED, &GenConfig::default(), &never()).unwrap();
        for land in [land, &mountains] {
            let mut renamed = to_bytes(land);
            let found = renamed.windows(name.len()).position(|window| window == name).expect("relief_m in the country");
            renamed[found + name.len() - 1] = b'n';
            assert_says(&refusal(&renamed), "its CTRY section (the drawing's kind of country) is damaged");
        }
        // The version and the head's checksum; a format damaged reads as another format, refused as one.
        let version_at = MAGIC.len() + size_of::<u32>() + size_of::<u64>();
        for at in [version_at, version_at + VERSION.len()] {
            let mut changed = bytes.clone();
            changed[at] ^= 0x10;
            assert_says(&refusal(&changed), "its head is damaged, since its bytes do not give the checksum written with them");
        }
        let mut changed = bytes.clone();
        changed[MAGIC.len()] ^= 0x02;
        assert_says(&refusal(&changed), "it was written in format 3, a newer format than format 1");
        // A section's length made shorter: its body is not the one written, and that is said before
        // whatever the next section's head, now in the middle of a body, seems to say.
        let &(start, _, length) = starts(&bytes).iter().find(|(_, found, _)| *found == HEIGHTS).unwrap();
        let mut shorter = bytes.clone();
        shorter[start + TAG_BYTES..start + TAG_BYTES + LENGTH_BYTES].copy_from_slice(&(length as u64 - 4).to_le_bytes());
        assert_says(&refusal(&shorter), "its HGHT section (the world's heights) is damaged");
        // A tag damaged turns the section into one this Landform does not know, which is damaged too.
        let mut retagged = bytes.clone();
        retagged[start + 1] = b'X';
        assert_says(&refusal(&retagged), "its HXHT section (a section this Landform does not know) is damaged");
    }

    #[test]
    fn a_sample_of_single_byte_changes_is_each_refused() {
        // Every byte of the head and of every section's head, every byte of a body up to
        // `EVERY_BYTE_UP_TO` bytes, and of a longer one its first and last `ENDS` bytes and every
        // `STRIDE`th between; each changed one way, by turns of three.
        const EVERY_BYTE_UP_TO: usize = 4096;
        const ENDS: usize = 64;
        const STRIDE: usize = 4093;
        let bytes = to_bytes(small());
        let starts = starts(&bytes);
        let mut offsets: Vec<(usize, bool)> = (0..starts[0].0).map(|at| (at, false)).collect();
        for &(start, _, length) in &starts {
            // The tag and the checksum are the section's own bytes, so their change is damage; a
            // length changed may instead run past the end or into another section's head.
            offsets.extend((start..start + SECTION_HEAD_BYTES).map(|at| (at, !(start + TAG_BYTES..start + TAG_BYTES + LENGTH_BYTES).contains(&at))));
            let body = start + SECTION_HEAD_BYTES;
            let inside = if length <= EVERY_BYTE_UP_TO {
                (0..length).collect::<Vec<_>>()
            } else {
                (0..ENDS).chain((ENDS..length - ENDS).step_by(STRIDE)).chain(length - ENDS..length).collect()
            };
            offsets.extend(inside.into_iter().map(|at| (body + at, true)));
        }
        let mut refused_as_damaged = 0;
        for (index, &(at, damage)) in offsets.iter().enumerate() {
            let mut changed = bytes.clone();
            changed[at] ^= [0x01, 0x80, 0xFF][index % 3];
            let reason = from_bytes(&changed).err().unwrap_or_else(|| panic!("the byte at {at} changed was read as a land"));
            assert!(!damage || reason.contains("is damaged, since its bytes do not give the checksum written with them"), "byte {at}: {reason}");
            refused_as_damaged += usize::from(reason.contains("is damaged"));
        }
        println!("{} single-byte changes of a {} byte land file, each refused, {refused_as_damaged} of them as damaged", offsets.len(), bytes.len());
        assert!(offsets.len() > 2000);
    }

    #[test]
    fn a_layer_of_one_name_twice_is_refused() {
        let land = written();
        let bytes = to_bytes(land);
        let mut slope = Bytes::new();
        slope.text("slope");
        slope.grid(&land.world.layers["slope"]);
        assert_says(&refusal(&inserted(&bytes, Some(WATER), LAYER, &slope.0)), "it holds the layer slope twice");
        // A layer of a name this Landform does not know may come twice: both are skipped.
        let mut future = Bytes::new();
        future.text("future");
        future.grid(&Grid::new(4, 4, 1.0));
        let twice = inserted(&inserted(&bytes, Some(WATER), LAYER, &future.0), Some(WATER), LAYER, &future.0);
        assert_eq!(differences(land, &from_bytes(&twice).expect("the land is read").land, true), Vec::<&str>::new());
    }

    #[test]
    fn notes_and_what_each_is_about_are_read_as_written() {
        // The trial `wide` at 1,024 m in hills from seed 1 notes its rivers and lowers its lake
        // ("Lake 1 was lowered ..."); one note about the land is put after them, so the file holds
        // subjects of all three kinds.
        let drawing = crate::trial::trial_drawing("wide", 1024, Country::hills()).expect("wide is a trial drawing");
        let mut land = Land::grow(&drawing, 1, &GenConfig::default(), &never()).expect("wide grows");
        let lowered = land.world.notes.iter().position(|note| note.starts_with("Lake 1 was lowered ")).expect("wide from seed 1 lowers its lake");
        assert_eq!(land.world.notes_about[lowered], About::Lake(0));
        assert!(land.world.notes_about.iter().any(|about| matches!(about, About::River(_))), "{:?}", land.world.notes_about);
        land.world.notes.push("A brook was cut 1.4 m deeper where a brook joins it.".into());
        land.world.notes_about.push(About::Land);

        let read = from_bytes(&to_bytes(&land)).expect("the land is read").land;
        assert_eq!(read.world.notes, land.world.notes, "the notes, word for word and in their order");
        assert_eq!(read.world.notes_about, land.world.notes_about, "what each is about, in the notes' order");
        assert_eq!(differences(&land, &read, true), Vec::<&str>::new());
        println!("wide, 1,024 m, hills, seed 1: {} notes, about {:?}", read.world.notes.len(), read.world.notes_about);
    }

    /// The reason `land`, written, is refused for.
    fn refused(change: impl FnOnce(&mut Land)) -> String {
        let mut land = written().clone();
        change(&mut land);
        refusal(&to_bytes(&land))
    }

    #[test]
    fn a_land_whose_parts_do_not_fit_together_is_refused() {
        let small = || Grid::<f32>::new(16, 16, 1.0);
        assert_says(&refused(|land| land.world.heights = small()), "the world's heights grid is 16 x 16 cells of 1 m, where its drawing asks for 512 x 512 cells of 1 m");
        assert_says(&refused(|land| land.world.ground = Grid::new(16, 16, 1.0)), "the ground map is 16 x 16 cells of 1 m, where its drawing asks for 512 x 512");
        assert_says(&refused(|land| *land.world.layers.get_mut("slope").unwrap() = small()), "the layer slope is 16 x 16 cells of 1 m");
        assert_says(&refused(|land| land.world.heights.cell_size = 2.0), "the world's heights grid is 512 x 512 cells of 2 m");
        assert_says(&refused(|land| drop(land.world.layers.remove(WATER_LAYER))), "it holds no LAYR section for the world's layer water");
        // A model of another drawing's grid, and one of the drawing's grid on other cells.
        let other = Land::grow(&Drawing::example_valley(256), EXAMPLE_SEED, &GenConfig::default(), &never()).unwrap();
        let side = model_side(SIZE_M);
        assert_says(
            &refused(|land| land.model_heights = other.model_heights.clone()),
            &format!("the model's heights are {0} x {0} cells of 8 m, where its drawing's model grid is {side} x {side} cells of 8 m", model_side(256)),
        );
        assert_says(&refused(|land| land.model_heights.cell_size = 4.0), &format!("the model's heights are {side} x {side} cells of 4 m"));
        assert_says(&refused(|land| land.world.heights.data[5] = f32::NAN), "the world's heights hold a value that is not a finite number (value 5, counted from 0)");
        assert_says(&refused(|land| land.model_heights.data[7] = f32::INFINITY), "the model's heights hold a value that is not a finite number (value 7, counted from 0)");
        assert_says(&refused(|land| land.world.meta.base_elevation_m = 3.5), "its world is not in the land model's frame: its base elevation is 3.5 m, not 0");
        assert_says(&refused(|land| land.world.ground.data[9].second = 6), "the ground map names a material beyond the 6 there are, at cell 9");
        assert_says(&refused(|land| land.world.water.rivers[0].drawn = Some(7)), "the water's course 1 names a river of the drawing or a lake of the water that is not there");
        assert_says(&refused(|land| land.world.water.rivers[0].end = CourseEnd::River(999)), "the water's course 1 ends on a course or a lake that is not there");
        assert_says(&refused(|land| land.world.water.rivers[1].points.clear()), "the water's course 2 has no points");
        let half = SIZE_M as f32 / 2.0;
        assert_says(&refused(|land| land.world.water.rivers[0].points[3].x_m = half), "the water's course 1 has a point that is not finite or not on the world's cells");
        assert_says(&refused(|land| land.world.water.rivers[0].points[3].depth_m = f32::NAN), "the water's course 1 has a point that is not finite");
        // Water no wider than nothing, or wider than the land, which `water_surface` lays out in
        // buckets over its reach (a width of 1e30 m once panicked there); and water below 0 m deep.
        for width_m in [0.0, -5.0, 513.0, 1e30] {
            assert_says(
                &refused(|land| land.world.water.rivers[0].points[3].width_m = width_m),
                &format!("the water's course 1 has a point whose water is {width_m} m wide, where a course's water is above 0 m and at most the land's 512 m wide"),
            );
        }
        assert_says(&refused(|land| land.world.water.rivers[1].points[2].depth_m = -1.0), "the water's course 2 has a point whose water is -1 m deep, below 0 m");
        // As wide as the land, and no depth at all, as at a spring's head, are read.
        let mut edge = written().clone();
        edge.world.water.rivers[0].points[3].width_m = SIZE_M as f32;
        edge.world.water.rivers[1].points[2].depth_m = 0.0;
        assert_eq!(differences(&edge, &from_bytes(&to_bytes(&edge)).expect("the land is read").land, true), Vec::<&str>::new());
        assert_says(&refused(|land| land.world.water.lakes[0].drawn = 4), "the water's lake 1 names a lake the drawing does not hold");
        assert_says(&refused(|land| land.world.water.lakes[0].level_m = f32::NAN), "the water's lake 1 names a lake the drawing does not hold, or a value that is not finite");
        assert_says(&refused(|land| land.world.water.lakes[0].area_m2 = f32::INFINITY), "the water's lake 1 names a lake the drawing does not hold, or a value that is not finite");
        let lakes = written().world.water.lakes.len();
        assert_says(&refused(|land| land.world.water.rivers[0].from_lake = Some(lakes)), "the water's course 1 names a river of the drawing or a lake of the water that is not there");
        assert_says(&refused(|land| land.world.water.rivers[1].end = CourseEnd::Lake(lakes)), "the water's course 2 ends on a course or a lake that is not there");
        assert_says(&refused(|land| land.world.water.rivers[2].end = CourseEnd::River(2)), "the water's course 3 ends on itself");
        assert_says(
            &refused(|land| {
                let lake = land.world.water.lakes[0];
                land.world.water.lakes.push(lake);
            }),
            &format!("the water's lakes 1 and {} are both lake {} of the drawing", lakes + 1, written().world.water.lakes[0].drawn + 1),
        );
        assert_says(&refused(|land| land.first_lake_levels[0].drawn = 4), "the first lakes' levels name lake 5 of a drawing of 1 lakes");
        assert_says(
            &refused(|land| {
                let lake = land.first_lake_levels[0];
                land.first_lake_levels.push(lake);
            }),
            &format!("the first lakes' levels give lake {} twice", written().first_lake_levels[0].drawn + 1),
        );
        // Layers whose cells are not finite, which the water's surface, a hold and an edit read.
        assert_says(&refused(|land| land.world.layers.get_mut(WATER_LAYER).unwrap().data[11] = f32::NAN), "the layer water's cells hold a value that is not a finite number (value 11, counted from 0)");
        assert_says(&refused(|land| land.world.layers.get_mut("slope").unwrap().data[3] = f32::NEG_INFINITY), "the layer slope's cells hold a value that is not a finite number (value 3, counted from 0)");
        // The world's window and north, which an export writes, and the seed its world was grown from.
        assert_says(&refused(|land| land.world.meta.window.center_x = f64::NAN), "its world's window or north is not a finite number");
        assert_says(&refused(|land| land.world.meta.north.1 = f32::INFINITY), "its world's window or north is not a finite number");
        assert_says(&refused(|land| land.world.meta.seed = 12), "its world was grown from seed 12, where the land's seed is 11");
        let one_more_note = |land: &mut Land| land.world.notes.push("A note.".into());
        assert_says(
            &refused(|land| {
                one_more_note(land);
                land.world.notes_about.clear();
            }),
            "notes 0 subjects, not one each",
        );
        assert_says(
            &refused(|land| {
                one_more_note(land);
                land.world.notes_about.push(About::Lake(3));
            }),
            "a note is about a river or lake its drawing does not hold",
        );
        assert_says(&refused(|land| land.marks.held.push(false)), "its held marks run to 262145 cells, beyond its world's 262144");
        assert_says(&refused(|land| land.reach_m = f32::NAN), "its reach is NaN m, not a finite number");
        assert_says(&refused(|land| land.drawing.rivers[1].points.truncate(1)), "the land's drawing or settings are not valid: river 2 needs at least two points, not 1");
        assert_says(&refused(|land| land.config.cell_size_m = 2.0), "the land's drawing or settings are not valid: cell_size_m must be exactly 1");
        assert_says(&refused(|land| land.config.rules.wet_full = f32::NAN), "the land's drawing or settings are not valid: wet_full");
        assert_says(&refused(|land| land.config.water.water_fill = 7.0), "the land's drawing or settings are not valid: water_fill");
        // Lists of marks shorter than the world read as unmarked beyond them, as `Marks` says, and are kept.
        let mut short = written().clone();
        short.marks.by_hand.truncate(1000);
        assert_eq!(differences(&short, &from_bytes(&to_bytes(&short)).unwrap().land, true), Vec::<&str>::new());
    }

    #[test]
    fn an_unknown_section_layer_or_field_is_skipped_and_said_and_a_missing_field_is_refused() {
        let land = written();
        let bytes = to_bytes(land);
        let read = |bytes: &[u8]| from_bytes(bytes).unwrap_or_else(|reason| panic!("{reason}"));
        assert_eq!(read(&bytes).skipped, Vec::<Skipped>::new(), "a file this Landform wrote holds nothing it skips");

        // A section of a tag this Landform does not know, among the others and at the end.
        let unknown = inserted(&inserted(&bytes, Some(COUNTRY), *b"ZZZZ", &[1, 2, 3]), None, *b"FUT\0", &[9; 40]);
        let file = read(&unknown);
        assert_eq!(differences(land, &file.land, true), Vec::<&str>::new());
        assert_eq!(file.skipped, [Skipped::Section("ZZZZ".into()), Skipped::Section("FUT?".into())]);
        // A layer of a name it does not know.
        let mut layer = Bytes::new();
        layer.text("future");
        layer.grid(&Grid::new(4, 4, 1.0));
        let file = read(&inserted(&bytes, Some(WATER), LAYER, &layer.0));
        assert_eq!(differences(land, &file.land, true), Vec::<&str>::new());
        assert_eq!(file.skipped, [Skipped::Layer("future".into())]);
        // A field it does not know, in a record of rules and in the timings at the end of the meta.
        let mut fields = ground_rule_fields(&land.config.rules);
        fields.insert(3, ("future_m", Value::F64(2.5)));
        let with_rule = replaced(&bytes, GROUND_RULES, &record_bytes(fields));
        let meta = body_of(&bytes, META);
        let timings = record_bytes(timing_fields(&land.world.meta.timings));
        assert!(meta.ends_with(&timings), "the meta ends with the timings' record");
        let mut fields = timing_fields(&land.world.meta.timings);
        fields.push(("future_ms", Value::F32(4.0)));
        let mut later = meta[..meta.len() - timings.len()].to_vec();
        later.extend_from_slice(&record_bytes(fields));
        let file = read(&replaced(&with_rule, META, &later));
        assert_eq!(differences(land, &file.land, true), Vec::<&str>::new());
        let field = |section: &str, name: &str| Skipped::Field { section: section.into(), name: name.into() };
        assert_eq!(file.skipped, [field("GRUL", "future_m"), field("META", "future_ms")]);
        // All of them, each said once, in the order they were met: the sections, then the parts
        // inside the sections as they are read.
        let all = inserted(&inserted(&inserted(&with_rule, Some(COUNTRY), *b"ZZZZ", &[1]), Some(WATER), LAYER, &layer.0), Some(WATER), LAYER, &layer.0);
        let said: Vec<String> = read(&all).skipped.iter().map(Skipped::to_string).collect();
        assert_eq!(said, ["the section ZZZZ", "the field future_m of the GRUL section (the ground rules)", "the layer future"]);

        // A field the file does not hold is refused, naming it: a land file of format 1 holds every
        // field of format 1, and today's default is not the value a land was made with.
        let without = |fields: Vec<(&'static str, Value)>, name: &str| record_bytes(fields.into_iter().filter(|(field, _)| *field != name).collect());
        let no_field = |section: &str, name: &str| format!("its {section} holds no field {name}, which every land file of format 1 holds");
        assert_says(&refusal(&replaced(&bytes, GROUND_RULES, &without(ground_rule_fields(&land.config.rules), "gully_depth_m"))), &no_field("GRUL section (the ground rules)", "gully_depth_m"));
        assert_says(&refusal(&replaced(&bytes, WATER_RULES, &without(water_rule_fields(&land.config.water), "bank_run"))), &no_field("WRUL section (the water rules)", "bank_run"));
        assert_says(&refusal(&replaced(&bytes, CONFIG, &without(config_fields(&land.config), "pick.margin_m"))), &no_field("CONF section (the settings)", "pick.margin_m"));
        assert_says(&refusal(&replaced(&bytes, CONFIG, &without(config_fields(&land.config), "size_cells"))), &no_field("CONF section (the settings)", "size_cells"));
        let mut lacking = meta[..meta.len() - timings.len()].to_vec();
        lacking.extend_from_slice(&without(timing_fields(&land.world.meta.timings), "grow_ms"));
        assert_says(&refusal(&replaced(&bytes, META, &lacking)), &no_field("META section (the world's meta)", "grow_ms"));
        // A record of no field at all names the first field of the struct.
        assert_says(&refusal(&replaced(&bytes, COUNTRY, &record_bytes(Vec::new()))), &no_field("CTRY section (the drawing's kind of country)", "floor_m"));

        // A known field written as another kind, a field twice, a kind it does not know.
        let mut fields = ground_rule_fields(&land.config.rules);
        assert_eq!(fields[19].0, "gully_depth_m");
        fields[19].1 = Value::F64(1.2);
        assert_says(
            &refusal(&replaced(&bytes, GROUND_RULES, &record_bytes(fields))),
            "its GRUL section (the ground rules) holds gully_depth_m as a 64-bit float, where this Landform reads a 32-bit float",
        );
        let mut fields = country_fields(&land.drawing.country);
        fields.push(fields[0]);
        assert_says(&refusal(&replaced(&bytes, COUNTRY, &record_bytes(fields))), "its CTRY section (the drawing's kind of country) holds its field floor_m twice");
        let mut body = record_bytes(vec![("floor_m", Value::F32(30.0))]);
        let kind_at = 8 + 8 + "floor_m".len();
        body[kind_at] = 9;
        assert_says(&refusal(&replaced(&bytes, COUNTRY, &body)), "holds 9 for the kind of number of its field floor_m, which this Landform does not know");
    }

    #[test]
    fn a_files_fault_names_the_file_and_a_missing_file_its_path() {
        let dir = std::env::temp_dir().join(format!("landform_land_fault_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cut.lfland");
        let bytes = to_bytes(written());
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        let error = Land::read(&path).unwrap_err();
        let missing = Land::read(&dir.join("none.lfland")).unwrap_err();
        let unwritable = written().write(&dir.join("no_folder").join("land.lfland")).unwrap_err();
        let _ = fs::remove_dir_all(&dir);
        assert!(matches!(&error, LandformError::BadLand { path: named, .. } if *named == path), "{error:?}");
        let sentence = error.to_string();
        assert!(sentence.starts_with(&format!("The land file {} cannot be read: its ", path.display())) && sentence.ends_with("so it may have been cut short."), "{sentence}");
        assert!(matches!(&missing, LandformError::Io(message) if message.contains("none.lfland")), "{missing:?}");
        assert!(matches!(&unwritable, LandformError::Io(message) if message.contains("land.lfland.part")), "{unwritable:?}");
    }

    /// `land` asking for `asked`, so that it waits.
    fn asking(land: &Land, asked: Drawing) -> Land {
        let mut land = land.clone();
        land.ask(asked).expect("the drawing is asked for");
        assert!(land.waiting());
        land
    }

    /// `written()` asking for edit `c` of its drawing (the main river's fourth point moved) in gentle
    /// country, so that it waits.
    fn waiting() -> Land {
        let land = written();
        asking(land, Drawing { country: Country::gentle(), ..example_edit(&land.drawing, "c").expect("c is an edit of the example") })
    }

    /// Stage 2's Task 1: a waiting land written and read is the land, field for field and bit for
    /// bit, the drawing asked for among it; its file is the file of the same land with nothing
    /// waiting, with `ASKD` put just before `MARK` (stage 2's ruling R1), the bytes before
    /// and after it the plain file's, byte for byte; and `ASKD` is laid out as `DRAW` and `CTRY` are
    /// for the drawing asked for.
    #[test]
    fn a_waiting_land_written_and_read_is_the_land_bit_for_bit() {
        let land = waiting();
        let bytes = to_bytes(&land);
        let file = from_bytes(&bytes).expect("the waiting land is read");
        assert_eq!(differences(&land, &file.land, true), Vec::<&str>::new());
        assert!(file.land.waiting() && file.skipped.is_empty());
        assert!(to_bytes(&file.land) == bytes, "the land read is written as the same bytes");

        let plain = to_bytes(written());
        let all = starts(&bytes);
        let tags: Vec<Tag> = all.iter().map(|&(_, tag, _)| tag).collect();
        assert_eq!(tags[tags.len() - 2..], [ASKED, MARKS], "the asked drawing just before the marks");
        let &(asked_at, _, asked_length) = &all[all.len() - 2];
        let asked_end = asked_at + SECTION_HEAD_BYTES + asked_length;
        let marks_at = starts(&plain).iter().find(|(_, tag, _)| *tag == MARKS).expect("the plain file's marks").0;
        assert_eq!(asked_at, marks_at, "put where the plain file's marks begin");
        assert!(bytes[..asked_at] == plain[..marks_at] && bytes[asked_end..] == plain[marks_at..], "the plain file's bytes before and after it");
        assert_eq!(bytes.len(), plain.len() + SECTION_HEAD_BYTES + asked_length);
        let as_drawn = to_bytes(&Land { drawing: land.asked.clone(), ..land.clone() });
        assert!(body_of(&bytes, ASKED) == [body_of(&as_drawn, DRAWING), body_of(&as_drawn, COUNTRY)].concat(), "DRAW's body, then CTRY's");
        println!("the example at {SIZE_M} m, waiting: {} bytes, {} more than with nothing waiting", bytes.len(), bytes.len() - plain.len());
    }

    /// Stage 2's Task 1: stage 1's three land files, the game's fixtures as stage 1 wrote them, are
    /// read with nothing waiting and nothing skipped, and written again as their own bytes, byte for
    /// byte. They hold the crate's version `0.1.0` in their head: when the version is raised, this
    /// compares the files from their first section on, or the promise is restated then.
    #[test]
    fn stage_1s_land_files_read_and_written_again_are_their_bytes() {
        for (name, length) in STAGE1_FIXTURES {
            let path = Path::new(STAGE1_FIXTURES_DIR).join(name);
            let bytes = fs::read(&path).unwrap_or_else(|error| panic!("stage 1's land file {}: {error}", path.display()));
            assert_eq!(bytes.len(), length, "{name}: stage 1's bytes");
            let file = from_bytes(&bytes).unwrap_or_else(|reason| panic!("{name}: {reason}"));
            assert_eq!(file.version, "0.1.0", "{name}: the version stage 1 wrote");
            assert!(file.skipped.is_empty() && !file.land.waiting() && file.land.asked == file.land.drawing, "{name}: nothing waits");
            assert!(to_bytes(&file.land) == bytes, "{name} written again is its own bytes");
            // Read through the public reader of bytes, as the Godot extension restores a land after a
            // reload, it is the file read from the disk.
            assert!(LandFile::from_bytes(&bytes).ok() == Some(file.clone()), "{name}: read from its bytes");
            assert!(LandFile::read(&path).ok() == Some(file), "{name}: read from the disk");
            println!("{name}: {} bytes, written again byte for byte", bytes.len());
        }
    }

    /// Stage 2's Task 1: a waiting land's file without `ASKD` is read with nothing waiting, the land
    /// asking for its own drawing, and is the file of that land. Cut short anywhere in or beside the
    /// section it is refused (stage 2's ruling R1): at its start or where it ends, at
    /// `MARK`'s start, for lacking `MARK`; inside its body, naming it; inside its head, as a file
    /// that ends inside a section's head. The section twice, holding more or
    /// less than a drawing, lacking a field of format 1, or holding a drawing `Drawing::validate`
    /// refuses or one of another size, is refused, naming it; a field of its country this Landform
    /// does not know is skipped and said.
    #[test]
    fn a_file_without_its_asked_drawing_waits_for_nothing_and_a_wrong_one_is_refused() {
        let land = waiting();
        let bytes = to_bytes(&land);
        let all = starts(&bytes);
        let &(start, tag, length) = &all[all.len() - 2];
        assert_eq!(tag, ASKED);
        let end = start + SECTION_HEAD_BYTES + length;
        let without = [&bytes[..start], &bytes[end..]].concat();
        assert!(without == to_bytes(written()), "the land's file with nothing waiting");
        let read = from_bytes(&without).expect("the file without the asked drawing is read");
        assert!(!read.land.waiting() && read.land.asked == read.land.drawing, "nothing waits");
        assert_eq!(differences(&read.land, written(), true), Vec::<&str>::new());
        for cut in [start, end] {
            assert_says(&refusal(&bytes[..cut]), "it holds no MARK section (the marks), so it may have been cut short");
        }
        assert_says(&refusal(&bytes[..start + 5]), "it ends 5 bytes into the head of a section");
        let cut = start + SECTION_HEAD_BYTES + length / 2;
        assert_says(&refusal(&bytes[..cut]), &format!("its ASKD section (the asked drawing) runs {} bytes past the end of the file", length - length / 2));

        let body = body_of(&bytes, ASKED);
        assert_says(&refusal(&inserted(&bytes, None, ASKED, &body)), "it holds 2 ASKD sections (the asked drawing), where a land has one");
        let mut longer = body.clone();
        longer.extend_from_slice(&[0, 0, 0]);
        assert_says(&refusal(&replaced(&bytes, ASKED, &longer)), "its ASKD section (the asked drawing) is 3 bytes longer than what it holds");
        assert_says(&refusal(&replaced(&bytes, ASKED, &body[..body.len() - 1])), "its ASKD section (the asked drawing) ends before what it holds");
        let as_drawn = to_bytes(&Land { drawing: land.asked.clone(), ..land.clone() });
        let draw = body_of(&as_drawn, DRAWING);
        let lacking = [draw.clone(), record_bytes(country_fields(&land.asked.country).into_iter().filter(|(field, _)| *field != "relief_m").collect())].concat();
        assert_says(&refusal(&replaced(&bytes, ASKED, &lacking)), "its ASKD section (the asked drawing) holds no field relief_m, which every land file of format 1 holds");
        let mut one_point = land.clone();
        one_point.asked.rivers[1].points.truncate(1);
        assert_says(&refusal(&to_bytes(&one_point)), "its ASKD section (the asked drawing) holds a drawing that is not valid: river 2 needs at least two points, not 1");
        let other_size = Land { asked: Drawing::example_valley(256), ..land.clone() };
        assert_says(&refusal(&to_bytes(&other_size)), "its ASKD section (the asked drawing) holds a drawing 256 m across, where the land is 512 m across");

        let mut fields = country_fields(&land.asked.country);
        fields.push(("future_m", Value::F32(1.0)));
        let file = from_bytes(&replaced(&bytes, ASKED, &[draw, record_bytes(fields)].concat())).expect("a field this Landform does not know is skipped");
        assert_eq!(differences(&land, &file.land, true), Vec::<&str>::new());
        let said: Vec<String> = file.skipped.iter().map(Skipped::to_string).collect();
        assert_eq!(said, ["the field future_m of the ASKD section (the asked drawing)"]);
    }

    /// Stage 2's ruling R2 (S2): an `ASKD` that does not stand just before `MARK` is refused,
    /// naming where it stands: after `MARK`, the place Task 1 first wrote it, where a cut at its start
    /// would read as the land with nothing waiting; before `DRAW`; and before a section this Landform
    /// does not know, put before `MARK`. Just before `MARK` it is read, and a cut at its start lacks
    /// `MARK`.
    #[test]
    fn an_asked_drawing_out_of_its_place_is_refused_naming_where_it_stands() {
        let bytes = to_bytes(&waiting());
        let body = body_of(&bytes, ASKED);
        let &(start, _, length) = starts(&bytes).iter().find(|(_, tag, _)| *tag == ASKED).expect("the asked drawing");
        let plain = [&bytes[..start], &bytes[start + SECTION_HEAD_BYTES + length..]].concat();
        assert!(plain == to_bytes(written()), "the waiting file without its asked drawing");
        let wanted = "where a land file holds it just before its MARK section (the marks)";
        let after_marks = inserted(&plain, None, ASKED, &body);
        assert_says(&refusal(&after_marks), &format!("its ASKD section (the asked drawing) stands last in the file, {wanted}"));
        let cut = starts(&after_marks).iter().find(|(_, tag, _)| *tag == ASKED).expect("the asked drawing").0;
        assert!(from_bytes(&after_marks[..cut]).is_ok_and(|file| !file.land.waiting()), "cut there it would read as the land with nothing waiting");
        let before_drawing = inserted(&plain, Some(DRAWING), ASKED, &body);
        assert_says(&refusal(&before_drawing), &format!("its ASKD section (the asked drawing) stands just before its DRAW section (the drawing), {wanted}"));
        let unknown = inserted(&bytes, Some(MARKS), *b"ZZZZ", &[1, 2, 3]);
        assert_says(&refusal(&unknown), &format!("its ASKD section (the asked drawing) stands just before its ZZZZ section (a section this Landform does not know), {wanted}"));
        assert!(from_bytes(&inserted(&plain, Some(MARKS), ASKED, &body)).is_ok_and(|file| file.land.waiting()), "just before the marks it is read");
    }

    /// Stage 2's Task 1: a sample of single-byte changes of a waiting land's `ASKD`, every byte of
    /// its head and of its body its first and last bytes and every `STRIDE`th between, is each
    /// refused, and a change of its tag, its checksum or its body as damaged; every change after
    /// its tag names the section (stage 2's ruling R2, N2: a change of its length says it is
    /// damaged or runs past the end of the file), and a change of its body or its checksum says it is
    /// damaged.
    #[test]
    fn a_byte_changed_in_the_asked_drawing_is_refused_naming_its_section() {
        const ENDS: usize = 64;
        const STRIDE: usize = 1021;
        let land = asking(small(), Drawing { country: Country::gentle(), ..small().drawing.clone() });
        let bytes = to_bytes(&land);
        let &(start, _, length) = starts(&bytes).iter().find(|(_, tag, _)| *tag == ASKED).expect("the asked drawing");
        let body = start + SECTION_HEAD_BYTES;
        let mut offsets: Vec<usize> = (start..body).collect();
        offsets.extend((0..ENDS).chain((ENDS..length - ENDS).step_by(STRIDE)).chain(length - ENDS..length).map(|at| body + at));
        for (index, &at) in offsets.iter().enumerate() {
            let mut changed = bytes.clone();
            changed[at] ^= [0x01, 0x80, 0xFF][index % 3];
            let reason = from_bytes(&changed).err().unwrap_or_else(|| panic!("the byte at {at} changed was read as a land"));
            let in_length = (start + TAG_BYTES..start + TAG_BYTES + LENGTH_BYTES).contains(&at);
            if !in_length {
                assert!(reason.contains("is damaged, since its bytes do not give the checksum written with them"), "byte {at}: {reason}");
            }
            if at >= start + TAG_BYTES {
                assert!(reason.starts_with("its ASKD section (the asked drawing) "), "byte {at}: {reason}");
            }
            if at >= start + TAG_BYTES + LENGTH_BYTES {
                assert!(reason.starts_with("its ASKD section (the asked drawing) is damaged"), "byte {at}: {reason}");
            }
        }
        println!("{} single-byte changes of a waiting land's ASKD section of {length} bytes, each refused", offsets.len());
    }
}
