//! The outline of a regrowth while a person drags (`LandformOutline`), worked out off the main
//! thread for a land (`Outlines`, which `LandformLand` holds beside its job): the outline of
//! regrowing the land toward a drawing (the core's `start_of`, `footprint` and `outline`, with the
//! zones of the land's own lakes, and the overlay's picture of it when asked), or the outline of
//! regrowing an area of it as drawn (`area_outline`, with the same zones). One request is worked out at a time, on a thread of the land's outlines: a request waits
//! until the land's next `poll` starts it, or, while one is worked out, until that one ends, and a
//! request made while another waits replaces it, so only the latest is worked out next. Each request
//! has a cancel flag of its own (`jobs::Cancel`, which `jobs::join_workers` sets), and the thread is
//! one of `jobs.rs`'s workers, which `join_workers` waits for; it ends once no request waits. The
//! land's own start and its lakes' zones are kept between requests while the land's drawing, seed
//! and ground stay as they are, and made ahead while no request is asked (`Outlines::ready`). Plain
//! Rust but `LandformOutline`, which Godot holds.

use std::borrow::Cow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use godot::classes::image::Format;
use godot::classes::Image;
use godot::prelude::*;
use landform_core::{area_outline, footprint, lake_zones, outline, start_of, Drawing, Land, Marks, Outline, Start, Whole};
use rayon::prelude::*;

use crate::jobs::{self, Cancel};
use crate::overlay::{marks_overlay, paint, Colours};
use crate::pool;
use crate::step::M2_PER_HA;
use crate::world::{image_from, image_of};

/// A pixel of an outline's weight picture where the weight is 0: outside the outline, or on a held
/// or hand-shaped cell, where a regrowth changes nothing. No other weight reads 0, so the overlay
/// draws the outline's rim where the picture leaves 0.
pub(crate) const WEIGHT_NONE: u8 = 0;

/// A pixel where the weight is 1: the land is the drawing grown whole there, the cell lying about
/// `BAND_M` (150 m) or farther from every point outside the outline or fixed (the weight, an `f32`,
/// is 1 from 149.987 m on). No other weight reads 255, so the overlay draws the band's inner edge
/// where the picture reaches 255.
pub(crate) const WEIGHT_FULL: u8 = 255;

/// The least and the most a weight between 0 and 1 reads: the weight times 255, rounded to the
/// nearest whole number, held to these, so a weight just above 0 (a cell of an area beside its rim
/// weighs `s(1 / 150)`, 1.3e-4, which would round to 0) never reads as outside, and one just below 1
/// never as fully regrown.
const WEIGHT_LEAST_BETWEEN: u8 = 1;
const WEIGHT_MOST_BETWEEN: u8 = 254;

/// What the thread that works out a land's outlines is called, and what a request is said to have
/// met when the thread cannot start.
const OUTLINE_THREAD: &str = "landform-outline";
const OUTLINE_CANNOT_START: &str = "The outline's thread could not start";

/// What an outline whose work panicked, which Godot's output shows above this, is said to have done.
const OUTLINE_STOPPED_UNEXPECTEDLY: &str = "The outline stopped unexpectedly.";

/// The zones of a land's lakes, as the core's `lake_zones` gives them: one sorted list of world
/// cells a lake.
type Zones = Vec<Vec<usize>>;

/// An outline's weight as one pixel of its picture: `WEIGHT_NONE` for 0 (or below, or not a number,
/// which no outline holds), `WEIGHT_FULL` for 1 (or above), and between them the weight times 255,
/// rounded, held to `WEIGHT_LEAST_BETWEEN` to `WEIGHT_MOST_BETWEEN`.
fn weight_byte(weight: f32) -> u8 {
    if weight.is_nan() || weight <= 0.0 {
        WEIGHT_NONE
    } else if weight >= 1.0 {
        WEIGHT_FULL
    } else {
        (weight * f32::from(WEIGHT_FULL)).round().clamp(f32::from(WEIGHT_LEAST_BETWEEN), f32::from(WEIGHT_MOST_BETWEEN)) as u8
    }
}

/// What a `LandformOutline` holds, made on the outlines' thread: the weight picture's bytes, whether
/// the outline changes no ground, its area and why it is the whole land.
pub(crate) struct OutlineParts {
    /// The land's side in cells of 1 m, which is the picture's side in pixels.
    size: usize,
    /// One `weight_byte` a world cell, in the world's cell order (`Image.FORMAT_L8`, cell `(x, z)`
    /// at pixel `(x, z)`).
    weights: Vec<u8>,
    /// No world cell weighs above 0.
    empty: bool,
    /// The world cells inside the outline, fixed ones among them, and those fixed, in square metres
    /// (`Outline::inside_m2`, `Outline::fixed_inside_m2`).
    inside_m2: f64,
    fixed_inside_m2: f64,
    /// Why the outline is the whole land (`Whole::reason`), for an outline whose footprint says so.
    whole: Option<String>,
    /// The sentence the regrowth would be refused with that the request could see before it runs
    /// (`Land::held_lake_refusal` for a drawing, `Land::area_lake_refusal` for an area; stage 2's
    /// ruling R18, N2).
    refusal: Option<String>,
    /// The overlay's picture of the outline over the land's marks (`LandformOverlay.compose`'s bytes,
    /// one RGBA pixel a world cell), made beside the outline for a drawing's request asked with the
    /// overlay's colours (`LandformLand.set_outline_overlay`), with those colours and the edges'
    /// width; `None` otherwise (stage 2's ruling R21, N3).
    overlay: Option<(Vec<u8>, Colours, usize)>,
}

impl OutlineParts {
    /// `outline` of a land `size` cells a side as the parts Godot is given, its picture's bytes made
    /// side by side on the threads of the pool the call runs in (called inside `pool::run`).
    fn of(outline: &Outline, size: usize, whole: Option<String>, refusal: Option<String>) -> Self {
        // Taken apart, so a part added to an outline fails to compile until it is given or let go.
        let Outline { weight, model_weight: _, inside_m2, fixed_inside_m2 } = outline;
        let weights: Vec<u8> = weight.data.par_iter().map(|&value| weight_byte(value)).collect();
        let empty = weights.par_iter().all(|&byte| byte == WEIGHT_NONE);
        Self { size, weights, empty, inside_m2: *inside_m2, fixed_inside_m2: *fixed_inside_m2, whole, refusal, overlay: None }
    }
}

/// The outline of a regrowth of a land, worked out while a person drags (`LandformLand.
/// start_outline`) or paints an area (`LandformLand.start_area_outline`), delivered by
/// `LandformLand.poll` through `outline_ready`: whether it changes no ground, its area, why it is the
/// whole land, and its weight as a picture, from which the overlay draws the outline's rim (where
/// the picture leaves 0) and the band's inner edge (where it reaches 255). An outline made by
/// `LandformOutline.new()`, or one an editor reload of the extension rebuilt, holds nothing: it is
/// empty, of no area and no picture.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformOutline {
    /// `None` in an outline that holds nothing.
    parts: Option<OutlineParts>,
}

impl LandformOutline {
    /// A new outline holding `parts`.
    pub(crate) fn made(parts: OutlineParts) -> Gd<Self> {
        Gd::from_object(Self { parts: Some(parts) })
    }

    /// The weight picture's bytes (`get_weight_image()`'s, one a world cell in the world's cell
    /// order) and the land's side in cells, without a copy; `None` in an outline that holds nothing.
    pub(crate) fn weights(&self) -> Option<(&[u8], usize)> {
        self.parts.as_ref().map(|parts| (parts.weights.as_slice(), parts.size))
    }
}

#[godot_api]
impl LandformOutline {
    /// Whether the regrowth changes no ground: no world cell weighs above 0, so every pixel of
    /// `get_weight_image()` is 0. So for a drawing whose change the footprint does not show (the
    /// land's own, a point added where a line already runs), and for an area of no cell or wholly
    /// held. True in an outline that holds nothing.
    #[func]
    fn is_empty(&self) -> bool {
        self.parts.as_ref().is_none_or(|parts| parts.empty)
    }

    /// The world cells inside the outline, held and hand-shaped ones among them, in hectares: the
    /// whole land for a whole-land change, 0 for an empty footprint; for an area, the area and any
    /// lake it takes whole with its band, so an area wholly held is inside though it changes no
    /// ground.
    #[func]
    fn get_inside_ha(&self) -> f64 {
        self.parts.as_ref().map_or(0.0, |parts| parts.inside_m2 / M2_PER_HA)
    }

    /// The world cells inside the outline that are held or shaped by hand, in hectares.
    #[func]
    fn get_fixed_ha(&self) -> f64 {
        self.parts.as_ref().map_or(0.0, |parts| parts.fixed_inside_m2 / M2_PER_HA)
    }

    /// Why the regrowth changes the whole land but held and hand-shaped ground, in an edit's words
    /// (the core's `Whole::reason`): "the kind of country changed", "a number of the country
    /// changed: relief_m", "numbers of the country changed: cut_strength and rounds"; empty for every
    /// other outline, and for an area.
    #[func]
    fn get_whole_reason(&self) -> GString {
        self.parts.as_ref().and_then(|parts| parts.whole.as_deref()).map_or_else(GString::new, GString::from)
    }

    /// The sentence the regrowth would be refused with, in the core's words, where the request can
    /// see it before the regrowth runs (stage 2's ruling R18, N2): for a drawing, a lake it moves,
    /// resizes or takes away while some of its water is held or shaped by hand
    /// (`Land::held_lake_refusal`); for an area, a lake whose zone its outline takes but not whole, by
    /// the land's own lakes' zones (`Land::area_lake_refusal`). Empty when it sees none: the
    /// refusals that need the drawing grown (a lake added or changed beside fixed ground, a lake a
    /// drawing's outline takes but not whole, the cells only the drawing grown whole adds to an
    /// area's zone) and the water's (the joined land) are said by the regrowth alone.
    #[func]
    fn get_refusal(&self) -> GString {
        self.parts.as_ref().and_then(|parts| parts.refusal.as_deref()).map_or_else(GString::new, GString::from)
    }

    /// The overlay's picture of this outline over the land's held and hand-shaped cells, with no area,
    /// as `LandformOverlay.compose` makes it with these colours and `edge_cells`, an
    /// `Image.FORMAT_RGBA8` of the land's size: made on the outlines' thread beside the outline, so
    /// showing it costs the main thread only the image (stage 2's ruling R21, N3). Only an outline of
    /// a drawing asked while the land had its overlay's colours (`LandformLand.set_outline_overlay`)
    /// holds one, and only for those colours and that width: `null` otherwise, and in an outline that
    /// holds nothing, so a caller showing an area or held ground of its own composes the picture
    /// itself. The marks are the land's in the state the outline was asked in, which is the state it
    /// is delivered in. A new image on every call.
    #[func]
    fn get_overlay_image(&self, rim_colour: Color, band_edge_colour: Color, held_colour: Color, hand_colour: Color, edge_cells: i64) -> Option<Gd<Image>> {
        let parts = self.parts.as_ref()?;
        let (pixels, colours, edge) = parts.overlay.as_ref()?;
        let asked = (paint(rim_colour), paint(band_edge_colour), paint(held_colour), paint(hand_colour));
        let made = (colours.rim, colours.band_edge, colours.held, colours.hand);
        (asked == made && usize::try_from(edge_cells).ok() == Some(*edge)).then(|| image_of(parts.size, parts.size, Format::RGBA8, pixels))
    }

    /// The outline's weight on the land's cells, the share of the regrown land each takes, as an
    /// `Image.FORMAT_L8` of the land's size, cell `(x, z)` at pixel `(x, z)`, as the held picture
    /// is: 0 exactly where the weight is 0 (outside the outline, or on held or hand-shaped ground),
    /// 255 exactly where it is 1 (about 150 m inside the rim or farther, fully regrown), and between
    /// them the weight times 255, rounded, held to 1 to 254, so a weight just above 0 reads 1, never
    /// 0, and one just below 1 reads 254, never 255. A new image on every call; `null` in an outline
    /// that holds nothing.
    #[func]
    fn get_weight_image(&self) -> Option<Gd<Image>> {
        self.parts.as_ref().map(|parts| image_from(parts.size, parts.size, Format::L8, parts.weights.clone()))
    }
}

/// What a request asks the outline of.
pub(crate) enum Asked {
    /// Regrowing the land toward this drawing, which `Drawing::validate` takes and is of the land's
    /// size: its outline, from the land's own start and the drawing's, with the land's marks and
    /// reach and the zones of its own lakes.
    Drawing(Drawing),
    /// Regrowing these cells as drawn, one flag a world cell in the world's cell order: their
    /// outline with the zones of the land's own lakes and the marks the regrowth weighs them by.
    Area(Vec<bool>),
    /// No outline: the land's own start and its lakes' zones made and kept for the requests to come,
    /// nothing delivered and nothing said of a failure, which the request that needs them says
    /// (`Outlines::ready`).
    Ready,
}

/// Which of the land's own start and its lakes' zones a request may take from those kept: each a
/// count that moves on when what it is made of may have changed (`Outlines::forget`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Keys {
    start: u64,
    zones: u64,
}

/// One request: the land as it was when it was asked, what of, and its own cancel flag.
struct Ask {
    land: Arc<Land>,
    asked: Asked,
    /// The overlay's colours and edges' width the outline's picture is made with, for a drawing's
    /// request (`Outlines::set_overlay`); `None` for none.
    overlay: Option<(Colours, usize)>,
    /// The land's state when it was asked: its outline is delivered only while the land is in it.
    state: u64,
    keys: Keys,
    /// The outlines' generation when it was asked (`Mail::generation`): what it gives is let go
    /// once the requests of that generation were let go.
    generation: u64,
    cancel: Cancel,
}

/// What the main thread and the outlines' thread share, under one lock.
#[derive(Default)]
struct Mail {
    /// The latest request not yet started.
    waiting: Option<Ask>,
    /// The generation and the cancel flag of the request being worked out, and whether it is an
    /// outline's (not a `Ready`).
    working: Option<(u64, Cancel, bool)>,
    /// The newest outline worked out and not yet polled, or why it failed, with the land's state it
    /// is of.
    done: Option<(u64, Result<OutlineParts, String>)>,
    /// Whether the outlines' thread runs.
    worker: bool,
    /// Moves on each time every request is let go.
    generation: u64,
    /// The land's own start, `start_of` of its drawing and seed, with the key it was made under.
    start: Option<(u64, Arc<Start>)>,
    /// The zones of the land's own lakes, with the key they were made under.
    zones: Option<(u64, Arc<Zones>)>,
    /// The keys the last `Ready` worked out to its end was asked under, so a land whose start or
    /// zones cannot be made is not asked again until they change.
    readied: Option<Keys>,
}

/// `Mail` and the signal that its request being worked out has ended.
#[derive(Default)]
struct Post {
    mail: Mutex<Mail>,
    changed: Condvar,
}

impl Post {
    fn lock(&self) -> MutexGuard<'_, Mail> {
        self.mail.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The land's own start, kept under `key`, or made now (`start_of` of the land's drawing and
    /// seed) and kept. Plain Rust: called on the outlines' thread.
    fn own_start(&self, key: u64, land: &Land) -> Result<Arc<Start>, String> {
        if let Some((kept, start)) = &self.lock().start {
            if *kept == key {
                return Ok(Arc::clone(start));
            }
        }
        let start = Arc::new(start_of(&land.drawing, &land.config.water, land.seed).map_err(|error| error.to_string())?);
        self.lock().start = Some((key, Arc::clone(&start)));
        Ok(start)
    }

    /// The zones of the land's own lakes, kept under `key`, or made now (`own_zones`) and kept.
    fn own_zones(&self, key: u64, land: &Land) -> Result<Arc<Zones>, String> {
        if let Some((kept, zones)) = &self.lock().zones {
            if *kept == key {
                return Ok(Arc::clone(zones));
            }
        }
        let zones = Arc::new(own_zones(land)?);
        self.lock().zones = Some((key, Arc::clone(&zones)));
        Ok(zones)
    }
}

/// The zones of the lakes a regrowth of the land's own drawing takes, as the land holds them: the
/// core's `lake_zones` of the land's world, with its kept model (`Land::kept_model`) as both models,
/// the land's drawing as both drawings. So each zone holds the cells where the land's water stands
/// at the lake's level, near the lake, and where the kept model holds the lake's water; the
/// regrowth (`Land::regrow_area`) also takes the cells where the drawing grown whole holds it,
/// which only growing it shows. Plain Rust, no thread pool.
fn own_zones(land: &Land) -> Result<Zones, String> {
    let model = land.kept_model().map_err(|error| error.to_string())?;
    Ok(lake_zones(&land.world, &model, &model, &land.drawing, &land.drawing))
}

/// The marks a regrowth of `area` weighs it by, by the core's one rule (`Marks::regrown`, which
/// `Land::regrow_area` reads; stage 2's ruling R12, S2): the land's own marks, not copied, when none
/// of them is a hand mark, since the rule then changes nothing.
fn regrown_marks<'a>(marks: &'a Marks, area: &[bool]) -> Cow<'a, Marks> {
    if !marks.by_hand.contains(&true) {
        return Cow::Borrowed(marks);
    }
    Cow::Owned(marks.regrown(area))
}

impl Ask {
    /// Whether it asks for an outline, not for the land's start and zones alone (`Asked::Ready`).
    fn is_outline(&self) -> bool {
        !matches!(self.asked, Asked::Ready)
    }

    /// Works the outline out in the extension's pool, reading the request's own flag between its
    /// steps: `None` once the flag was set (what was made is let go), otherwise the outline's parts.
    fn work(&self, post: &Post) -> Result<Option<OutlineParts>, String> {
        let stopped = || self.cancel.flag().load(Ordering::Relaxed);
        pool::run(|| {
            if stopped() {
                return Ok(None);
            }
            let land = &*self.land;
            let size_m = land.drawing.size_m;
            let (outline, whole, refusal) = match &self.asked {
                Asked::Ready => {
                    // A failure is the next request's to say, which makes them again.
                    if post.own_start(self.keys.start, land).is_ok() && !stopped() {
                        let _ = post.own_zones(self.keys.zones, land);
                    }
                    if !stopped() {
                        post.lock().readied = Some(self.keys);
                    }
                    return Ok(None);
                }
                Asked::Drawing(drawing) => {
                    let own = post.own_start(self.keys.start, land)?;
                    if stopped() {
                        return Ok(None);
                    }
                    let new = start_of(drawing, &land.config.water, land.seed).map_err(|error| error.to_string())?;
                    if stopped() {
                        return Ok(None);
                    }
                    let found = footprint(&own, &new, &land.drawing, drawing);
                    // The land's own lakes' zones, as an area's request has them, so a lake of the land
                    // the outline reaches is taken whole with its band while a person drags; a lake the
                    // drawing adds or moves needs its new model, which only the regrowth grows
                    // (stage 2's ruling R21, S3). An empty footprint needs none.
                    let zones = if found.is_empty() { Arc::new(Zones::new()) } else { post.own_zones(self.keys.zones, land)? };
                    if stopped() {
                        return Ok(None);
                    }
                    let refusal = land.held_lake_refusal(drawing);
                    (outline(size_m, &found, &zones, &land.marks, land.reach_m), found.whole.as_ref().map(Whole::reason), refusal)
                }
                Asked::Area(area) => {
                    let zones = post.own_zones(self.keys.zones, land)?;
                    if stopped() {
                        return Ok(None);
                    }
                    let outline = area_outline(size_m, area, &zones, &regrown_marks(&land.marks, area));
                    if stopped() {
                        return Ok(None);
                    }
                    let refusal = land.area_lake_refusal(area, &zones, &outline);
                    (outline, None, refusal)
                }
            };
            if stopped() {
                return Ok(None);
            }
            let mut parts = OutlineParts::of(&outline, size_m as usize, whole, refusal);
            if let (Asked::Drawing(_), Some((colours, edge))) = (&self.asked, self.overlay) {
                if stopped() {
                    return Ok(None);
                }
                parts.overlay = Some((marks_overlay(parts.size, &parts.weights, &land.marks, colours, edge), colours, edge));
            }
            Ok(Some(parts))
        })
        .and_then(|worked| worked)
    }
}

/// The outlines' thread: works out the waiting request, then the one waiting after it, until none
/// waits. Each result is kept for `poll` (the newest only) unless its requests were let go; one
/// stopped by its flag gives nothing.
fn work_through(post: &Post) {
    // The lock is held from the end of one request's work to the taking of the next, or to the
    // thread's end when none waits, so a request made after the thread has found none waiting waits
    // for the next `poll` (stage 2's ruling R12, N2).
    let mut mail = post.lock();
    loop {
        let Some(ask) = mail.waiting.take() else {
            mail.worker = false;
            post.changed.notify_all();
            return;
        };
        mail.working = Some((ask.generation, ask.cancel.clone(), ask.is_outline()));
        drop(mail);
        let (state, generation) = (ask.state, ask.generation);
        let outcome = catch_unwind(AssertUnwindSafe(|| ask.work(post)));
        // The land is let go before the work is said to have ended, so that a land the main thread
        // waits to change (`Outlines::let_go_and_wait`) is its own again once it has.
        drop(ask);
        let result = match outcome {
            Ok(Ok(Some(parts))) => Some(Ok(parts)),
            Ok(Ok(None)) => None,
            Ok(Err(sentence)) => Some(Err(sentence)),
            Err(_) => Some(Err(OUTLINE_STOPPED_UNEXPECTEDLY.to_string())),
        };
        mail = post.lock();
        mail.working = None;
        if let Some(result) = result.filter(|_| mail.generation == generation) {
            mail.done = Some((state, result));
        }
        post.changed.notify_all();
    }
}

/// A land's outlines: the request waiting, the one being worked out on the outlines' thread, the
/// newest outline worked out and not yet delivered, and the land's own start and lakes' zones kept
/// for them. Letting it go lets every request go.
pub(crate) struct Outlines {
    post: Arc<Post>,
    /// The keys a request made now is given.
    keys: Keys,
    /// The overlay's colours and edges' width a drawing's request made now composes its picture
    /// with (`set_overlay`), or `None`.
    overlay: Option<(Colours, usize)>,
}

impl Default for Outlines {
    fn default() -> Self {
        Self { post: Arc::new(Post::default()), keys: Keys::default(), overlay: None }
    }
}

impl Outlines {
    /// Asks for the outline of `asked` on `land`, in the land's state `state`: the request waits,
    /// replacing any request waiting, until `poll` starts it, or the request being worked out ends.
    pub(crate) fn ask(&mut self, land: Arc<Land>, asked: Asked, state: u64) {
        let cancel = Cancel::new();
        let mut mail = self.post.lock();
        let generation = mail.generation;
        let replaced = mail.waiting.replace(Ask { land, asked, overlay: self.overlay, state, keys: self.keys, generation, cancel });
        drop(mail);
        drop(replaced);
    }

    /// Starts the outlines' thread when a request waits and the thread does not run, then gives the
    /// newest outline worked out since the last call, or why it failed, when it is of the land in
    /// `state`; one of another state is let go. Returns at once.
    pub(crate) fn poll(&mut self, state: u64) -> Option<Result<OutlineParts, String>> {
        let mut mail = self.post.lock();
        if mail.waiting.is_some() && !mail.worker {
            let post = Arc::clone(&self.post);
            // The thread waits for the lock this holds, so it finds `worker` set.
            match jobs::spawn(OUTLINE_THREAD, move || work_through(&post)) {
                Ok(()) => mail.worker = true,
                Err(error) => {
                    mail.waiting = None;
                    return Some(Err(format!("{OUTLINE_CANNOT_START}: {error}.")));
                }
            }
        }
        let (of, result) = mail.done.take()?;
        (of == state).then_some(result)
    }

    /// Whether an outline was asked for and is not yet delivered: a request waits, is worked out, or
    /// its outline waits for `poll`. A request let go is not counted, nor a `ready` one.
    pub(crate) fn is_working(&self) -> bool {
        let mail = self.post.lock();
        mail.waiting.as_ref().is_some_and(Ask::is_outline)
            || mail.done.is_some()
            || mail.working.as_ref().is_some_and(|&(generation, _, outline)| outline && generation == mail.generation)
    }

    /// Asks the outlines' thread to make the land's own start and its lakes' zones ahead, so that
    /// the first outline asked after the land was read, made or swapped does not wait for them (at
    /// 2,048 m about 45 ms, the zones about 35 of them): a request of its own (`Asked::Ready`),
    /// asked only when none waits or is worked out and neither is kept under the keys a request
    /// made now has, nor was tried under them, in the land's state `state`, and replaced by any
    /// request asked before the thread takes it. It delivers nothing, is not counted by
    /// `is_working`, and holds `land` while it works, as a request does.
    pub(crate) fn ready(&mut self, land: Arc<Land>, state: u64) {
        let mut mail = self.post.lock();
        let kept = mail.start.as_ref().is_some_and(|&(key, _)| key == self.keys.start) && mail.zones.as_ref().is_some_and(|&(key, _)| key == self.keys.zones);
        if kept || mail.waiting.is_some() || mail.working.is_some() || mail.readied == Some(self.keys) {
            return;
        }
        let generation = mail.generation;
        mail.waiting = Some(Ask { land, asked: Asked::Ready, overlay: None, state, keys: self.keys, generation, cancel: Cancel::new() });
    }

    /// Lets every request go, with the land each holds: the one waiting, the one being worked out
    /// (its flag set, so it ends at its next check, and what it gives let go) and the outline not yet
    /// delivered. The lock is given back held.
    fn letting_go(&self) -> MutexGuard<'_, Mail> {
        let mut mail = self.post.lock();
        mail.generation = mail.generation.wrapping_add(1);
        mail.waiting = None;
        mail.done = None;
        if let Some((_, cancel, _)) = &mail.working {
            cancel.set();
        }
        mail
    }

    /// Lets every request go (`letting_go`), without waiting for the one being worked out to end.
    pub(crate) fn let_go(&self) {
        drop(self.letting_go());
    }

    /// Lets every request go and waits for the one being worked out to end, at most one step of it
    /// (the land's start, the drawing's start, the footprint with its outline, the lakes' zones, or an
    /// area's outline, each between two checks of its flag; at 2,048 m at most about 35 ms, the
    /// lakes' zones or a whole-land area's outline), so that it no longer holds the land. Returns
    /// whether a request was being worked out.
    pub(crate) fn let_go_and_wait(&self) -> bool {
        let mail = self.letting_go();
        let working = mail.working.is_some();
        drop(self.post.changed.wait_while(mail, |mail| mail.working.is_some()).unwrap_or_else(PoisonError::into_inner));
        working
    }

    /// Makes each drawing's request from now on compose the overlay's picture of its outline with
    /// these colours and edges' width (`marks_overlay`), or none for `None`.
    pub(crate) fn set_overlay(&mut self, overlay: Option<(Colours, usize)>) {
        self.overlay = overlay;
    }

    /// The land's own start no longer holds (`start`: its drawing or seed may have changed), or the
    /// zones of its lakes (`zones`: its world, land model, drawing, seed or settings may have
    /// changed): those kept are let go, and a request made from now on works them out again.
    pub(crate) fn forget(&mut self, start: bool, zones: bool) {
        let mut mail = self.post.lock();
        if start {
            self.keys.start = self.keys.start.wrapping_add(1);
            mail.start = None;
        }
        if zones {
            self.keys.zones = self.keys.zones.wrapping_add(1);
            mail.zones = None;
        }
    }
}

impl Drop for Outlines {
    fn drop(&mut self) {
        self.let_go();
    }
}

/// The outlines' work runs its parallel parts through the extension's pool, as a land's job does, so
/// no test here starts rayon's global pool, which the test in `pool.rs` checks; the tests that work
/// out an outline take turns with those that call `jobs::join_workers`, which sets every flag.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::land::tests::start_regrow;
    use crate::pool::tests::TURN;
    use landform_core::{example_edit, Country, GenConfig};
    use std::sync::atomic::AtomicBool;
    use std::thread;
    use std::time::{Duration, Instant};

    /// The side of the example the tests grow, large enough that an edit's outline, reaching 300 m
    /// beyond its footprint, leaves the far corner out, and the seed it grows from.
    const SIZE_M: u32 = 512;
    const SEED: u64 = 11;
    /// Two states of a land, as `LandformLand` numbers them: the one the requests are asked in, and
    /// another.
    const STATE: u64 = 1;
    const OTHER_STATE: u64 = 2;
    /// How long an outline is waited for before a test gives up on it, far longer than any takes
    /// here, and the wait between two polls.
    const WAIT_MOST: Duration = Duration::from_secs(120);
    const POLL_EVERY: Duration = Duration::from_millis(1);
    /// A disc of an area: its radius in metres, and how far beyond the lake's drawn rim its middle
    /// lies, on the lake's east, so it reaches into the lake's water.
    const DISC_M: f64 = 10.0;
    const BEYOND_RIM_M: f64 = 5.0;
    /// Another disc's middle, far from the lake and the brook: its east and south in metres.
    const AWAY_AT: (f64, f64) = (-150.0, 180.0);

    /// The example `SIZE_M` across in hills from `SEED`, grown in the pool; with `asked`, asking for
    /// its edit `a` (the brook's second point moved 30 m east).
    fn grown(asked: bool) -> Arc<Land> {
        let land = pool::run(|| {
            let mut land = Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
            if asked {
                let edited = example_edit(&land.drawing, "a").expect("the example's edit a");
                land.ask(edited).expect("the edit is one the land may ask for");
            }
            land
        });
        Arc::new(land.expect("the pool starts"))
    }

    /// The cells of a land `SIZE_M` across within `radius_m` of (`x_m`, `z_m`), cell `(i, j)`
    /// standing at `(i - SIZE_M / 2, j - SIZE_M / 2)`.
    fn disc(x_m: f64, z_m: f64, radius_m: f64) -> Vec<bool> {
        let (size, half) = (SIZE_M as usize, f64::from(SIZE_M) / 2.0);
        (0..size * size).map(|cell| ((cell % size) as f64 - half - x_m).hypot((cell / size) as f64 - half - z_m) <= radius_m).collect()
    }

    /// Polls `outlines` for a land in `state`, `POLL_EVERY` between calls, until no outline is asked
    /// for and not delivered, at most `WAIT_MOST`: every outline delivered, in order.
    fn delivered(outlines: &mut Outlines, state: u64) -> Vec<Result<OutlineParts, String>> {
        let (mut arrived, start) = (Vec::new(), Instant::now());
        loop {
            arrived.extend(outlines.poll(state));
            if !outlines.is_working() {
                return arrived;
            }
            assert!(start.elapsed() < WAIT_MOST, "no outline arrived within {WAIT_MOST:?}");
            thread::sleep(POLL_EVERY);
        }
    }

    /// The one outline delivered for a land in `STATE`, worked out.
    fn the_outline(outlines: &mut Outlines) -> OutlineParts {
        let mut arrived = delivered(outlines, STATE);
        assert_eq!(arrived.len(), 1, "one outline arrives");
        arrived.remove(0).expect("the outline is worked out")
    }

    /// A weight reads 0 exactly when it is 0 and 255 exactly when it is 1, and between them its
    /// share of 255, rounded, held to 1 to 254, never falling as the weight rises.
    #[test]
    fn a_weight_reads_0_only_at_0_and_255_only_at_1() {
        for none in [0.0, -0.0, -1.0, f32::NAN] {
            assert_eq!(weight_byte(none), WEIGHT_NONE, "{none}");
        }
        // The least weight there is, an area's cell beside its rim (`s(1 / 150)`), and half a step.
        let t = 1.0_f32 / 150.0;
        let beside_rim = t * t * (3.0 - 2.0 * t);
        for least in [f32::from_bits(1), f32::MIN_POSITIVE, beside_rim, 0.5 / 255.0, 1.0 / 255.0] {
            assert_eq!(weight_byte(least), WEIGHT_LEAST_BETWEEN, "{least}");
        }
        assert_eq!(weight_byte(0.5), 128);
        for most in [1.0 - f32::EPSILON, 254.6 / 255.0] {
            assert_eq!(weight_byte(most), WEIGHT_MOST_BETWEEN, "{most}");
        }
        for full in [1.0, 2.0, f32::INFINITY] {
            assert_eq!(weight_byte(full), WEIGHT_FULL, "{full}");
        }
        let steps = 100_000;
        let bytes: Vec<u8> = (0..=steps).map(|step| weight_byte(step as f32 / steps as f32)).collect();
        assert!(bytes.windows(2).all(|pair| pair[0] <= pair[1]), "a weight's byte fell as the weight rose");
    }

    /// Of three requests made before a poll the last alone is worked out; the land's own drawing
    /// gives an empty outline and its edit `a` one round the moved point with nothing far from it;
    /// an outline of a state the land has left is let go, and so is every request when the land
    /// changes, which then no longer holds the land.
    #[test]
    fn of_requests_made_in_a_row_the_latest_alone_is_worked_out_and_a_change_lets_them_go() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = grown(false);
        let edited = example_edit(&land.drawing, "a").expect("the example's edit a");
        let gentle = Drawing { country: Country::named("gentle").expect("a kind of country"), ..land.drawing.clone() };
        let mut outlines = Outlines::default();
        for drawing in [edited.clone(), land.drawing.clone(), gentle] {
            outlines.ask(Arc::clone(&land), Asked::Drawing(drawing), STATE);
        }
        let whole = the_outline(&mut outlines);
        assert_eq!(whole.whole.as_deref(), Some("the kind of country changed"));
        assert!(!whole.empty && whole.weights.iter().all(|&byte| byte == WEIGHT_FULL), "the whole land, nothing held, is fully regrown");
        assert_eq!(whole.inside_m2, f64::from(SIZE_M * SIZE_M));

        outlines.ask(Arc::clone(&land), Asked::Drawing(land.drawing.clone()), STATE);
        let own = the_outline(&mut outlines);
        assert!(own.empty && own.inside_m2 == 0.0 && own.whole.is_none());
        assert!(own.weights.len() == (SIZE_M * SIZE_M) as usize && own.weights.iter().all(|&byte| byte == WEIGHT_NONE));

        outlines.ask(Arc::clone(&land), Asked::Drawing(edited.clone()), STATE);
        let moved = the_outline(&mut outlines);
        let (x, z) = edited.rivers[1].points[1];
        let half = f64::from(SIZE_M) / 2.0;
        assert!(!moved.empty && moved.inside_m2 > 0.0 && moved.whole.is_none());
        assert_eq!(moved.weights[(z + half) as usize * SIZE_M as usize + (x + half) as usize], WEIGHT_FULL, "the moved point lies far inside the outline");
        assert_eq!(moved.weights[0], WEIGHT_NONE, "the north-west corner lies beyond the reach");

        // An outline of a state the land has left is let go.
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited.clone()), STATE);
        assert!(delivered(&mut outlines, OTHER_STATE).is_empty(), "an outline of another state arrived");
        // A change of the land lets every request go: the one being worked out and the one waiting.
        // The land's start is not kept, so the request works it out and is still being worked out
        // when the change comes (stage 2's ruling R12, N3).
        outlines.forget(true, true);
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited.clone()), STATE);
        assert!(outlines.poll(STATE).is_none(), "the request was only started");
        let start = Instant::now();
        while outlines.post.lock().working.is_none() {
            assert!(start.elapsed() < WAIT_MOST, "the outlines' thread took no request within {WAIT_MOST:?}");
            thread::yield_now();
        }
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited), STATE);
        assert!(outlines.let_go_and_wait(), "no request was being worked out when the land changed");
        assert!(!outlines.is_working(), "a request let go is still counted");
        assert_eq!(Arc::strong_count(&land), 1, "a request let go still holds the land");
        assert!(delivered(&mut outlines, STATE).is_empty(), "a request let go gave an outline");
    }

    /// An area's outline, of a disc reaching into the lake's water and of one far from it, is the
    /// outline its regrowth weighs it by on the land just grown, byte for byte, and the first takes
    /// the lake whole with its band; and so is the far disc's on a land with held cells and cells
    /// shaped by hand under it and beside it, some of them both (stage 2's ruling R12, S2).
    #[test]
    fn an_areas_outline_is_the_outline_its_regrowth_weighs_it_by() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = grown(false);
        let lake = land.drawing.lakes[0];
        let away = disc(AWAY_AT.0, AWAY_AT.1, DISC_M);
        let mut marked = (*land).clone();
        // Held: a disc over the area's west half and beyond it. By hand: one over its east half and
        // beyond it, and one over its middle, which the held disc partly covers.
        let held = disc(AWAY_AT.0 - DISC_M, AWAY_AT.1, DISC_M);
        let (east, middle) = (disc(AWAY_AT.0 + DISC_M, AWAY_AT.1, DISC_M), disc(AWAY_AT.0 - DISC_M / 2.0, AWAY_AT.1, DISC_M / 2.0));
        marked.marks.by_hand = east.iter().zip(&middle).map(|(&east, &middle)| east || middle).collect();
        marked.marks.held = held;
        let regrown = marked.marks.regrown(&away);
        let cleared = (0..away.len()).any(|cell| marked.marks.by_hand[cell] && !regrown.by_hand[cell]);
        let kept_under = (0..away.len()).any(|cell| away[cell] && regrown.by_hand[cell] && regrown.held[cell]);
        assert!(cleared && kept_under, "the marks clear a hand mark under the area and keep one on a held cell under it");
        let marked = Arc::new(marked);
        let mut outlines = Outlines::default();
        let cases = [
            (&land, lake.x_m + f64::from(lake.radius_m) + BEYOND_RIM_M, lake.z_m, true),
            (&land, AWAY_AT.0, AWAY_AT.1, false),
            (&marked, AWAY_AT.0, AWAY_AT.1, false),
        ];
        for (land, x_m, z_m, takes_the_lake) in cases {
            let area = disc(x_m, z_m, DISC_M);
            let cells = area.iter().filter(|&&on| on).count() as f64;
            outlines.ask(Arc::clone(land), Asked::Area(area.clone()), STATE);
            let outline = the_outline(&mut outlines);
            let edit = pool::run(|| land.regrow_area(&area, &AtomicBool::new(false))).expect("the pool starts").expect("the area regrows");
            let regrown: Vec<u8> = edit.outline.weight.data.iter().map(|&weight| weight_byte(weight)).collect();
            assert!(outline.weights == regrown, "the disc at ({x_m}, {z_m}): its outline is not its regrowth's");
            assert_eq!((outline.inside_m2, outline.fixed_inside_m2), (edit.outline.inside_m2, edit.outline.fixed_inside_m2));
            assert_eq!(outline.inside_m2 > cells, takes_the_lake, "the disc at ({x_m}, {z_m}): {} m² inside, {cells} in the disc", outline.inside_m2);
            assert!(outline.whole.is_none() && !outline.empty);
            assert_eq!(outline.fixed_inside_m2 > 0.0, Arc::ptr_eq(land, &marked), "the disc at ({x_m}, {z_m}): fixed ground inside");
        }
    }

    /// Stage 2's ruling R21, S3 and N3: a drawing's outline is the core's `outline` of its footprint
    /// with the land's own lakes' zones, so on the example at `ZONES_SIZE_M` edit `a` (the brook's
    /// second point 30 m east), whose band reaches the lake, takes the lake's zone whole and is
    /// larger than the early outline without zones, as large as the regrowth's own outline; asked
    /// with the overlay's colours it carries the overlay's picture of it over the land's marks, and
    /// without them, or for an area, none.
    #[test]
    fn a_drawings_outline_takes_the_lands_own_lakes_whole_and_carries_its_overlay() {
        // At `SIZE_M` edit a's outline covers nearly the whole land with or without the zones.
        const ZONES_SIZE_M: u32 = 1024;
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = Arc::new(pool::run(|| Land::grow(&Drawing::example_valley(ZONES_SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false))).expect("the pool starts").expect("the example grows"));
        let edited = example_edit(&land.drawing, "a").expect("the example's edit a");
        let colours = Colours { hand: [0.2, 0.9, 0.2, 0.4], held: [0.0, 0.8, 0.9, 0.4], area: [0.0; 4], band_edge: [1.0, 0.7, 0.0, 1.0], rim: [1.0, 0.0, 0.0, 1.0] };
        let mut outlines = Outlines::default();
        outlines.set_overlay(Some((colours, 3)));
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited.clone()), STATE);
        let asked = the_outline(&mut outlines);
        let (zones, early, edit) = pool::run(|| {
            let own = start_of(&land.drawing, &land.config.water, land.seed).expect("the land's start");
            let new = start_of(&edited, &land.config.water, land.seed).expect("the edit's start");
            let found = footprint(&own, &new, &land.drawing, &edited);
            let zones = own_zones(&land).expect("the land's zones");
            let with = outline(ZONES_SIZE_M, &found, &zones, &land.marks, land.reach_m);
            let early = landform_core::early_outline(ZONES_SIZE_M, &found, &land.marks, land.reach_m);
            let edit = land.edit(&edited, &AtomicBool::new(false)).expect("edit a regrows");
            (with, early, edit)
        })
        .expect("the pool starts");
        let expected: Vec<u8> = zones.weight.data.iter().map(|&weight| weight_byte(weight)).collect();
        assert!(asked.weights == expected, "the drawing's outline is not the core's outline with the land's zones");
        assert_eq!(asked.inside_m2, zones.inside_m2);
        assert!(asked.inside_m2 > early.inside_m2, "the zones added nothing: {} m² against {} m² without them", asked.inside_m2, early.inside_m2);
        println!("edit a at {ZONES_SIZE_M} m: {} m² drawn with the land's zones, {} m² without, {} m² regrown", asked.inside_m2, early.inside_m2, edit.outline.inside_m2);
        assert_eq!(asked.inside_m2, edit.outline.inside_m2, "the outline drawn is not the regrowth's");
        let (pixels, made, edge) = asked.overlay.as_ref().expect("the drawing's outline carries its overlay");
        assert_eq!((*made, *edge), (colours, 3));
        assert_eq!(*pixels, pool::run(|| marks_overlay(ZONES_SIZE_M as usize, &asked.weights, &land.marks, colours, 3)).expect("the pool starts"));
        outlines.set_overlay(None);
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited), STATE);
        assert!(the_outline(&mut outlines).overlay.is_none(), "an outline asked without the colours carries an overlay");
        outlines.set_overlay(Some((colours, 3)));
        let side = ZONES_SIZE_M as usize;
        let area: Vec<bool> = (0..side * side).map(|cell| cell == side * side / 2).collect();
        outlines.ask(Arc::clone(&land), Asked::Area(area), STATE);
        assert!(the_outline(&mut outlines).overlay.is_none(), "an area's outline carries an overlay");
    }

    /// `ready` has the outlines' thread make the land's own start and its lakes' zones ahead,
    /// delivering nothing and never counted by `is_working`; a request asked before the thread takes
    /// it replaces it; it asks nothing while both are kept, nor again under the keys it was worked
    /// out under; and a drawing's outline asked afterwards is the one asked without it.
    #[test]
    fn ready_makes_the_start_and_the_zones_ahead_and_delivers_nothing() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = grown(false);
        let edited = example_edit(&land.drawing, "a").expect("the example's edit a");
        let mut outlines = Outlines::default();
        outlines.ready(Arc::clone(&land), STATE);
        assert!(outlines.post.lock().waiting.as_ref().is_some_and(|ask| !ask.is_outline()), "ready asked for nothing");
        assert!(!outlines.is_working(), "a ready request is counted as an outline");
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited.clone()), STATE);
        assert!(outlines.post.lock().waiting.as_ref().is_some_and(Ask::is_outline), "a request did not replace the ready one");
        let first = the_outline(&mut outlines);
        outlines.ready(Arc::clone(&land), STATE);
        assert!(outlines.post.lock().waiting.is_none(), "ready asked again with the start and the zones kept");
        outlines.forget(true, true);
        outlines.ready(Arc::clone(&land), STATE);
        let start = Instant::now();
        loop {
            assert!(outlines.poll(STATE).is_none(), "a ready request delivered something");
            assert!(!outlines.is_working(), "a ready request is counted as an outline");
            let mail = outlines.post.lock();
            if mail.waiting.is_none() && !mail.worker {
                break;
            }
            drop(mail);
            assert!(start.elapsed() < WAIT_MOST, "the ready request did not end within {WAIT_MOST:?}");
            thread::sleep(POLL_EVERY);
        }
        let keys = outlines.keys;
        {
            let mut mail = outlines.post.lock();
            assert!(mail.start.as_ref().is_some_and(|&(key, _)| key == keys.start), "the land's start was not made ahead");
            assert!(mail.zones.as_ref().is_some_and(|&(key, _)| key == keys.zones), "the land's zones were not made ahead");
            assert_eq!(mail.readied, Some(keys));
            // As though they could not be made: tried under these keys, they are not asked again.
            mail.start = None;
        }
        outlines.ready(Arc::clone(&land), STATE);
        assert!(outlines.post.lock().waiting.is_none(), "ready asked again under the keys it was tried under");
        outlines.ask(Arc::clone(&land), Asked::Drawing(edited), STATE);
        let again = the_outline(&mut outlines);
        assert!(again.weights == first.weights && again.inside_m2 == first.inside_m2, "the outline after ready is another");
    }

    /// An outline asked for while a regrowth of the same land runs is worked out, and the regrowth
    /// ends with its step, neither waiting for the other; `jobs::join_workers` waits for the
    /// outlines' thread, and a request it found waiting gives nothing when it starts afterwards.
    #[test]
    fn an_outline_beside_a_regrowth_waits_for_neither_and_join_workers_stops_both() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = grown(true);
        let regrowth = start_regrow(Arc::clone(&land));
        let clock = Instant::now();
        let mut outlines = Outlines::default();
        outlines.ask(Arc::clone(&land), Asked::Drawing(land.asked.clone()), STATE);
        let outline = the_outline(&mut outlines);
        let outline_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let regrowth_ran_on = !regrowth.wait(Some(Duration::ZERO));
        assert!(regrowth.wait(Some(WAIT_MOST)), "the regrowth did not end");
        let regrowth_ms = clock.elapsed().as_secs_f64() * 1000.0;
        println!("an outline at {SIZE_M} m beside a regrowth: {outline_ms:.1} ms, the regrowth {regrowth_ms:.1} ms, running on after the outline: {regrowth_ran_on}");
        assert!(regrowth_ran_on, "the outline waited for the regrowth: it arrived after {outline_ms:.1} ms, once the regrowth had ended");
        assert!(!outline.empty);
        assert_eq!(regrowth.take().expect("the regrowth's result"), Ok(()));

        // A request waiting when `join_workers` runs, and one being worked out.
        let mut waiting = Outlines::default();
        waiting.ask(Arc::clone(&land), Asked::Drawing(land.asked.clone()), STATE);
        let mut working = Outlines::default();
        working.ask(Arc::clone(&land), Asked::Drawing(land.asked.clone()), STATE);
        assert!(working.poll(STATE).is_none(), "the request was only started");
        let regrowth = start_regrow(Arc::clone(&land));
        jobs::join_workers();
        assert!(!working.post.lock().worker, "join_workers did not wait for the outlines' thread");
        assert!(regrowth.wait(Some(Duration::ZERO)), "join_workers did not wait for the regrowth");
        assert!(delivered(&mut waiting, STATE).is_empty(), "a request whose flag join_workers set gave an outline");
        jobs::join_workers();
        pool::shut_down();
    }
}
