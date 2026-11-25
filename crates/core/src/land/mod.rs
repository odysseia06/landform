//! A land: a world grown from a drawing and kept, which an edit changes in one area only (the
//! editor tool's design, `Specs/2026-11-19 Editor Tool.md` in the vault, its section **Amended after
//! the prototype**). `Land` is what it was grown from (the drawing, the seed, the settings), the
//! drawing asked for, which waits while it differs, the land model's ground, the land as it is and
//! the marks; `Land::edit` changes it for an edited drawing by the prototype's final mechanism
//! (`report2.md`, "The final mechanism", P1 to P12), through the pieces of the modules below, and
//! gives a new land, the given one untouched.
//!
//! An edit is found and weighed in `outline`: its footprint from the land model's starting state,
//! the zones of the lakes, and the outline with the weight by which each place takes the new land.
//! The land model it keeps is mixed in `model`: the kept model and the edited drawing's model grown
//! whole, mixed by that weight, with a way out cut for each pit the mix leaves in the band, or the
//! pit filled. The ground the land keeps is joined in `join`: the kept land's heights, layers and
//! ground map joined to the world of the mixed model by the same weight on the world's cells, and
//! the measure of the band's ground an edit is judged by. The water the land keeps is joined in
//! `water`, after the ground: one course across the outline where a course crosses it, the lakes
//! whole, the band's water bounded by the joined ground and the band's ground wet where that water
//! wets it, and the measure of the water an edit leaves that the ground does not hold. A land is
//! kept as a file by `file` (`Land::write`, `Land::read`): read back field for field and bit for
//! bit, so an edit of a land read is the edit of the land written. What lies between two lands is
//! kept as a step of undo by `undo` (`Change`): swapped with one of them, it makes it the other.

pub mod file;
pub mod join;
pub mod model;
pub mod outline;
pub mod undo;
pub mod water;

use crate::drawing::{Drawing, Lake};
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{grow as grow_model, model_water, start_of, About, Grown, GrownLake, GrownRiver, TRENCH_REACH_M};
use crate::held::{unheld_cells, UnheldCells};
use crate::water::{model_lake_cells, LakeWater, WATER_LAYER};
use crate::world::{check_cancel, check_drawn, finish_drawn, Frame, GenConfig, World};
use file::drawing_bytes;
use join::join_ground;
use model::{mixed_model, Trough, TROUGH_SAID_M};
use outline::{
    area_outline, distances_to_line, early_area_outline, early_outline, footprint, footprint_of_area, lake_partners, lake_water, lake_zones, nearest_fixed_cells, outline,
    to_segment, Covered, Marks, Outline, Whole, BAND_M, LAKE_ZONE_MARGIN_M, REACH_MIN_M,
};
use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use water::{backwater_cells, join_water, river_partners, Backwater, WaterJoin, RISE_LIMIT_M};

/// A share of the ground an edit changes that is held or shaped by hand, in per cent, below which
/// its note says "under" this share rather than one rounded to nothing.
const SHARE_SAID_LEAST_PCT: f64 = 0.1;
/// How many times more an edit runs its outline, mix, fine steps and join with a new lake's
/// backwater taken (`backwater_cells`; stage 2's rulings R23 and R24): once. A round takes about
/// 0.6 s at 2,048 m (its fine steps 0.43 s, 0.55 s in mountains), so an edit that takes one is
/// 2.2 to 2.6 s in the core and up to about 2.8 s on screen with the hand-over to the tool (about
/// 0.25 s), within the design's 3 s, where a second would pass it in mountains; and every edit
/// measured whose backwater was not stopped by fixed ground took one round, none a second (the
/// fixer's 74 lakes along rivers and the final review's 140 edits).
pub const BACKWATER_ROUNDS_MOST: usize = 1;
/// What a held cell is called in a sentence.
const HELD: &str = "held ground";
/// What a hand-shaped cell is called in a sentence.
const BY_HAND: &str = "hand-shaped ground";
/// The note of an edit whose footprint is empty but whose drawing is not the land's own: the land
/// is given back as it was, its own drawing with it, and the drawing given is kept only as the one
/// asked for (`Land::asked`), waiting.
const DRAWING_KEPT: &str = "This edit changes no ground; the land keeps its drawing as it was.";
/// The note of an edit whose footprint is the whole land (the design's **Whole-land edits**), before
/// its reason (`Whole::reason`) and a full stop.
const WHOLE_LAND: &str = "This edit regrows the whole land but held and hand-shaped ground: ";

/// A land a person keeps and edits (the design's **The land file**): what it was grown from, the
/// drawing asked for, the land model's ground, the land as it is and the marks. A land is changed
/// only by making a new one (`Land::edit`, `Land::edit_from_seed`, `Land::reseed`,
/// `Land::regrow_area`), but for its marks (`Land::hold`) and the drawing it asks for (`Land::ask`).
///
/// **The kept model is derived one way, always.** No `Grown` is kept between edits: an edit works
/// the kept model's water out from `model_heights` (`model_water` of the drawing, the seed and the
/// water rules), and takes `first_lake_levels` as its lakes. So an edit gives the same land whether
/// the land was just grown, rebuilt from its fields, or read from a file.
///
/// **What waits.** `asked` is the drawing the person asked for, beside `drawing`, the one the land
/// was grown from: the next regrowth grows toward it (`Land::edit` of `asked`). An edit whose change
/// the footprint does not show gives the land back with its own drawing, since its world's rivers
/// and lakes are counted by that drawing, and keeps the drawing given as `asked`; so a point added
/// where a line already runs waits, and moves as an edit next. The land waits (`Land::waiting`)
/// while the two drawings differ, bit for bit.
#[derive(Debug, Clone, PartialEq)]
pub struct Land {
    /// The drawing the land stands for: the one it was grown from, or that of the last edit whose
    /// footprint was not empty. Its world's rivers and lakes are counted by it.
    pub drawing: Drawing,
    /// The drawing the person asked for, toward which the next regrowth grows: `drawing` while
    /// nothing waits. `Land::grow` sets it to the drawing grown, `Land::edit` and
    /// `Land::edit_from_seed` to the drawing they were given, `Land::ask` to the one it is given;
    /// `Land::reseed`, `Land::regrow_area` and `Land::hold` leave it. Always a drawing
    /// `Drawing::validate` takes, of the land's size. A land file without it (every file but a
    /// waiting land's) is read with `drawing` here, the value under which a land does what it did
    /// before this field: nothing waits.
    pub asked: Drawing,
    /// The seed its land model was grown from, which the model's routing also reads.
    pub seed: u64,
    /// The settings the fine steps run with: the ground rules, the water rules and the detail.
    pub config: GenConfig,
    /// The land model's heights (`Grown::heights`): `grow`'s for a new land, worked out in `f64` and
    /// kept in `f32`; after an edit the mixed model's, whose water `model_water` works out from them
    /// bit for bit.
    pub model_heights: Grid<f32>,
    /// The lakes the land model holds and their levels (`Grown::lakes`): `grow`'s for a new land,
    /// worked out in `f64`, where `model_water` of the kept `f32` heights can give a level a last bit
    /// lower (`wide` at 1,024 m in hills from seed 3); after an edit the mixed model's, which
    /// `model_water` of `model_heights` gives again exactly, but a lake the mix leaves as the kept
    /// model's heights give it, which keeps the kept level (`kept_lake_levels`).
    pub first_lake_levels: Vec<GrownLake>,
    /// The land as it is, in `Frame::Model` (the land model's frame, never shifted after the land is
    /// made), with the `water` layer.
    pub world: World,
    /// The cells an edit leaves as they are.
    pub marks: Marks,
    /// How far an outline reaches beyond its footprint, in metres: read held to `REACH_MIN_M` to
    /// `REACH_MAX_M`.
    pub reach_m: f32,
}

/// How long each step of an edit took, wall-clock milliseconds; informational only.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EditTimings {
    /// The two drawings' starts and the footprint (P1, P2), or the footprint of an area.
    pub footprint_ms: f32,
    /// Growing the edited drawing's model whole (P3); 0 when the footprint is empty.
    pub grow_ms: f32,
    /// The kept model's water worked out from its heights, the lakes' zones, the outline and its
    /// weight, and the test an edit may be refused by (P4, P5); with each round the backwater asks
    /// for (`Land::edit`, **The backwater**), as are the next three.
    pub outline_ms: f32,
    /// The mixed model and its pits (P6 to P8).
    pub mix_ms: f32,
    /// The fine steps from the mixed model (P9).
    pub fine_steps_ms: f32,
    /// The joined ground, the joined water, the test of the water it leaves beside held or
    /// hand-shaped ground (`Land::edit`, **The refusal**) and the notes (P10 to P12).
    pub join_ms: f32,
    /// The whole edit, from the call.
    pub total_ms: f32,
}

/// An edit of a land, as `Land::edit` gives it.
#[derive(Debug, Clone)]
pub struct Edit {
    /// The edited land.
    pub land: Land,
    /// The outline and its weight, 0 wherever the land is as it was.
    pub outline: Outline,
    /// What was done to each pit the mix left in the band (`Mixed::troughs`).
    pub troughs: Vec<Trough>,
    /// The band's pits left when the troughs ran out (`Mixed::pits_left`): 0 but where the passes
    /// ran out, which the edit's notes then say.
    pub pits_left: usize,
    /// What joining the water did.
    pub water: WaterJoin,
    /// What the edit says: first its own notes, then the edited land's notes about what its outline
    /// touches (`Land::edit`, **The notes**), one sentence each.
    pub notes: Vec<String>,
    /// What each of `notes` is about, one for each and in its order.
    pub notes_about: Vec<About>,
    pub timings: EditTimings,
}

/// The world cells a hold or a lift of an area marks (`Land::hold_reach`), worked out once: the area's, and the whole
/// water of every lake whose water meets a cell marked, again until none grows; with each lake's water by its place in
/// the land's drawing. `Land::holds_anything_in` and `Land::hold_reached` read it, so a tool that asks whether a hold
/// changes anything and then holds works it out once (stage 2's ruling R14, N4). It is of the land it was worked out
/// from, as that land stood: a hold or an edit of the land since leaves it stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldReach {
    /// One flag a world cell, in the world's cell order.
    marked: Vec<bool>,
    /// Each lake's water, by the lake's place in the land's drawing and the cells of its water.
    lakes: Vec<(usize, Vec<usize>)>,
}

/// What a hold or a lift did, as `Land::hold_lakes` says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// How many cells' held mark it changed: those it held that were not held, or lifted that were.
    pub cells: usize,
    /// The lakes it held or lifted whole, by their places in the land's drawing (from 0, as a
    /// lake's `drawn` counts them), in that order.
    pub lakes: Vec<usize>,
}

/// The steps of an edit after which it reads its cancel flag, told to a hook in the crate's tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    OldStart,
    NewStart,
    Footprint,
    Grow,
    OldModel,
    Outline,
    Mix,
    FineSteps,
    JoinGround,
    JoinWater,
    WaterBesideFixed,
}

impl Land {
    /// A new land from `drawing`: the land model `grow` gives it from `seed`, its world from the fine
    /// steps in `Frame::Model` (`finish_drawn`), which is `generate_drawn`'s world but for the frame
    /// and the timings, no marks, `REACH_MIN_M` as its reach, and `drawing` as the drawing asked
    /// for, so nothing waits. The drawing and `config` are checked first, as `generate_drawn` checks
    /// them, and the cancel flag is read as it reads it.
    /// The land made is checked last as the land file's reader checks a land it reads, so every land
    /// there is can be written and read again: one the reader would refuse (a river drawn wider than
    /// about 0.8 of the land, or water rules at the ends of their ranges, can make water wider than
    /// the land) is `BadConfig`, its sentence the reader's, naming the course.
    pub fn grow(drawing: &Drawing, seed: u64, config: &GenConfig, cancel: &AtomicBool) -> Result<Land, LandformError> {
        check_drawn(drawing, config)?;
        let grown = grow_model(drawing, &config.water, seed, cancel)?;
        let world = finish_drawn(seed, drawing, config, &grown, Frame::Model, cancel)?;
        let land = Land {
            drawing: drawing.clone(),
            asked: drawing.clone(),
            seed,
            config: *config,
            model_heights: grown.heights,
            first_lake_levels: grown.lakes,
            world,
            marks: Marks::none(drawing.size_m as usize),
            reach_m: REACH_MIN_M,
        };
        check_keepable(&land)?;
        Ok(land)
    }

    /// Sets the drawing the land asks for (`asked`), growing nothing: the next regrowth, `edit` of
    /// it, grows toward it. `BadConfig` for a drawing `Drawing::validate` refuses, naming the item,
    /// and for one of another size, in the words `edit` refuses them with; the land is then as it
    /// was.
    pub fn ask(&mut self, drawing: Drawing) -> Result<(), LandformError> {
        self.check_ask(&drawing)?;
        self.asked = drawing;
        Ok(())
    }

    /// Whether `ask` takes `drawing`, the land unchanged: `ask`'s refusals in its words, so a tool
    /// can refuse a drawing before it does anything else (stage 2's ruling R12, N4).
    pub fn check_ask(&self, drawing: &Drawing) -> Result<(), LandformError> {
        self.check_edited(drawing)
    }

    /// Whether an edit waits to regrow: the drawing asked for is not the one the land was grown
    /// from, compared bit for bit as an edit's empty footprint compares two drawings (the land
    /// file's bytes of each, `file::drawing_bytes`), so the two never disagree. It hashes nothing
    /// and uses no thread pool, but writes both drawings' bytes on every call (about 0.5 MB for the
    /// example's picture of 256 x 256, 128 MB at the largest picture a drawing holds), so it is
    /// called through `pool::run` with the other work of a job, or rarely: never once a frame.
    pub fn waiting(&self) -> bool {
        drawing_bytes(&self.asked) != drawing_bytes(&self.drawing)
    }

    /// The land changed for `drawing`, an edited copy of its drawing of the same size: everything
    /// outside the edit's outline, and every held or hand-shaped cell, as it was, bit for bit, and
    /// inside it the land `drawing` asks for, joined to the kept land across the band. `self` is
    /// never changed, whatever the outcome.
    ///
    /// **The steps** (P1 to P12): the land model's start of both drawings (`start_of`) and the
    /// footprint where they differ (`footprint`). An empty footprint gives the land back bit for bit,
    /// its own drawing with it, without growing (`EditTimings::grow_ms` 0, an outline of weight 0, no
    /// trough, a default `WaterJoin`), but for `asked`, which is `drawing`: so a change the footprint
    /// does not show waits (`Land::waiting`); its one note, about the land, is "This edit changes no
    /// ground; the land keeps its drawing as it was.", but none when `drawing` is the land's own bit
    /// for bit. Otherwise the edited drawing's model grown whole (`grow`); the kept model,
    /// `model_water` of `model_heights` with `first_lake_levels` as its lakes; the lakes' zones and
    /// the outline with the land's marks and reach; the refusal of lakes, below; the mixed model
    /// (`mixed_model`); the fine steps from it in the model's frame (`finish_drawn`); the ground
    /// joined (`join_ground`), then the water (`join_water`, with the land's water rules); the
    /// backwater, below; the refusal by the water left beside fixed ground, below; then the notes.
    /// The edited land holds
    /// the mixed model's heights as `model_heights` and its lakes as `first_lake_levels`, the edit's
    /// drawing, both as the one it stands for and as the one asked for, so nothing waits, and the
    /// land's seed, settings, marks and reach.
    ///
    /// **The backwater** (the design's "the outline follows that river ... as far as it must";
    /// stage 2's rulings R23 and R24). Where the join mends a drawn river down past the band's inner
    /// edge (`backwater_cells` says by how much), a lake of the new world holding its river up above
    /// the kept one past the outline or across the band, the kept river's cells over which the new
    /// world's river stands so, from where it enters the outline upstream as far as that holds, are
    /// taken as a lake's zone, with the band beyond them, and the outline, its refusal, the mix,
    /// the fine steps and the join run again, `BACKWATER_ROUNDS_MOST` times more at most and only
    /// where the cells change the outline's weight; the edit's times (`EditTimings`) are the sums
    /// of its rounds. Where fixed ground stops the backwater (`Backwater::stopped`), the band
    /// cannot move past it and the edit is refused, after any round, with one sentence: "Lake 2
    /// holds river 1 up to held ground at (49, -90); the water there would not be held: move the
    /// lake down the river, or lift the hold there." (`regrow that ground as drawn` for ground
    /// shaped by hand), naming the lake that holds it up where the edit adds, moves or resizes it,
    /// or else the lake the edit so changes whose rim lies nearest, or else the item it changes
    /// nearest, as refusal 4 names it. An area regrown as drawn is its own outline and takes none.
    /// What a tool draws before the edit runs (`early_outline`) cannot show it, nor the refusal.
    ///
    /// **The refusal** (the design's amendment 10): `Refused`, one sentence naming the river or lake
    /// by its place in the drawing as the notes count it (from 1), or the ground, the cell that must
    /// not change, its place in whole metres and the distance, and what to do (`refusal_sentence`).
    /// A cell is *lowered by fixed ground* where its weight is below the weight the same outline
    /// gives it without the marks (the outline itself is the same, since a zone is taken before
    /// fixed cells are left out of it): so exactly where a held or hand-shaped cell lies nearer it
    /// than the outline's rim, within `BAND_M`. After the outline, before anything is mixed, the
    /// lakes are asked, in this order:
    ///
    /// 1. a lake the edit moves, resizes or takes away has a held or hand-shaped cell of its water
    ///    (`Land::hold` holds a lake whole): that water would stay, naming no lake;
    /// 2. a lake the edit adds or changes has a world cell where the edited drawing's model holds its
    ///    water (`model_lake_cells`) in the footprint lowered by fixed ground;
    /// 3. a lake whose zone the outline takes has a cell of its zone lowered by fixed ground: it would
    ///    not be taken whole (the design's W4). A kept lake's own fixed cells (of its water in the
    ///    land, `outline::lake_water`) are not asked, and a lake whose water is fixed whole, as
    ///    `Land::hold` holds a lake, is not asked at all: it lies wholly at weight 0 and the edit is
    ///    taken round it, the lake kept bit for bit (`join_water` keeps a kept lake whose water lies
    ///    wholly at weight 0), and the water beside it is asked by 4.
    ///
    /// The place named is the fixed cell nearest the item's lightest such cell (for 1, the fixed
    /// cell of its water nearest its drawn centre), with its distance from that cell. Then, after
    /// the water is joined, the water itself is asked (the controller's ruling R17: no line or
    /// distance tells a harmless join beside held ground from a harmful one, only the water it
    /// leaves):
    ///
    /// 4. a cell lowered by fixed ground is unheld in the edited land (`unheld_cells`: a dry cell
    ///    below the water beside it, a lake cell over lower water, or either cell of a step) and is
    ///    not unheld in the same way at the same place in the kept land: the water there would not be
    ///    held. The place named is the fixed cell nearest the first such cell in the world's cell
    ///    order, and the item what the edit changes nearest that cell: of the drawn rivers whose
    ///    trench the footprint touches, by their new lines, and the rivers the edit removes, by their
    ///    lines in the kept model and their numbers in the land's drawing ("River 2, which this edit
    ///    removes, ran within 150 m of held ground at ..."; the controller's ruling R24), the one
    ///    whose line lies nearest, with the distance from that line to the fixed cell; else the lake
    ///    the edit adds or changes whose drawn rim lies nearest, with the rim's distance; else "the
    ///    painted ground" where the picture of high ground changed, or "the regrown ground", with the
    ///    distance from the footprint (the controller's ruling R18). An
    ///    edit of the land's own drawing (`reseed`, `regrow_area`) changes no drawn river or lake and
    ///    names the regrown ground, and so does an edit whose footprint is the whole land (another
    ///    seed or country, `Footprint::whole`), whose drawn lines may not have moved at all. Where
    ///    the edit took a backwater and the cell lies within `BAND_M` and `LAKE_ZONE_MARGIN_M` of its
    ///    cells, the item is the lake that holds it up, as **The backwater** names it, with the
    ///    cell's distance from the fixed cell ("Lake 2 holds river 1 up to held ground at ..., 4 m
    ///    from it"; stage 2's ruling R24), not a river line the edit did not move.
    ///
    /// **A tight join.** An edit beside fixed ground that is not refused is said (the design's "A
    /// tight join", marked and not refused, beside a hill as beside a river): for each piece of fixed
    /// ground (joined across sides and corners) that lowers a world cell of the footprint, being the
    /// fixed ground nearest it, a note at the piece's middle, in the order of the pieces' first
    /// cells. A hill painted beside held ground is not refused but by the water it would leave.
    ///
    /// **The notes.** `Edit::notes` are the edit's own, each about the land (`About::Land`) but where
    /// said: for a footprint that is the whole land, first, "This edit regrows the whole land but
    /// held and hand-shaped ground: " and why (`Whole::reason`: "the seed changed", "the kind of
    /// country changed", "a number of the country changed: relief_m"); every trough cut deeper than
    /// `TROUGH_SAID_M` with its place and depth, and every pit filled by more than that, with its
    /// place and the most it raised a cell, in the order done; the pits left when the troughs ran
    /// out (`Mixed::pits_left`), with their count; every place a course with no partner was narrowed
    /// to nothing or cut off at the outline (`WaterJoin`'s lists, a drawn river named as a river);
    /// every joined course that rose more than `RISE_LIMIT_M` going down it before it was mended,
    /// at its largest rise (`CourseJoin::rise_at`), and every end the join left out
    /// (`WaterJoin::ends_lost_at`); each of these four about its drawn river where it is one
    /// (`About::River`, the controller's ruling R23) and else about the land, as every note that
    /// names a drawn river is about it; each tight join beside fixed ground, above; the share of the footprint's
    /// world cells that is held, and the share shaped by hand, when there is any. Then the mixed model's world's notes whose subject the outline
    /// touches, with their subjects: a river any of whose course points, in the kept land or the
    /// mixed model's world, lies at weight above 0, or whose trench in either model the footprint
    /// touches; a lake whose zone the outline takes, or whose middle (as drawn in either drawing, or
    /// of its water in either world) lies at weight above 0; the land. The edited land's own notes
    /// are the mixed model's world's, in its order, but for each river or lake the outline does not
    /// touch and that has the same place in both drawings (paired as `join_water` pairs them): its
    /// notes are the kept land's, in their own order, standing one for one in the mixed world's
    /// places for it, those left over after its last such place, or all of them after all the
    /// others when the mixed world says nothing of it.
    ///
    /// **Errors.** `BadConfig` for a drawing `Drawing::validate` refuses, naming the item, for a
    /// drawing of another size (a land keeps its size), for a land whose world is not its
    /// drawing's size in the model's frame with the `water` layer, and for an edited land the land
    /// file's reader would refuse, checked last as `Land::grow` checks a land it makes, its sentence
    /// naming the course; `Cancelled` when `cancel` is set before the
    /// call or between its steps (it is read before and after every step that does not read it
    /// itself, and `grow` and `finish_drawn` read it within); `Refused`, above; the steps' own errors.
    pub fn edit(&self, drawing: &Drawing, cancel: &AtomicBool) -> Result<Edit, LandformError> {
        self.edit_from_seed(drawing, self.seed, cancel)
    }

    /// The land changed for `drawing` and grown from `seed`: `edit` from another seed, whose
    /// footprint is the whole land when `seed` is not the land's (`footprint` gives
    /// `Footprint::whole` for starts of different seeds), so a seed changed while an edit waits
    /// regrows the drawing asked for in the same growing. The edited land stands on `seed`, and
    /// `drawing` is its `asked`, whether or not its footprint is empty. With the land's own seed it
    /// is `edit` exactly. Refused, cancelled and failing as `edit` and `reseed` are.
    pub fn edit_from_seed(&self, drawing: &Drawing, seed: u64, cancel: &AtomicBool) -> Result<Edit, LandformError> {
        self.change(drawing, seed, None, drawing, cancel, &mut |_| {})
    }

    /// The land regrown from another seed: an edit of the land's own drawing whose footprint is the
    /// whole land (`footprint` gives `Footprint::whole` for starts of different seeds), so everything
    /// but held and hand-shaped ground is taken, with a band round that ground only, and the edited
    /// land stands on `seed`. It is `edit_from_seed` of the land's own drawing, but that the drawing
    /// asked for stays as it was, so what waits still waits. The same seed gives the land back bit
    /// for bit. Refused, cancelled and failing as `edit` is: held ground within `BAND_M` of a lake's
    /// zone is refused, and held ground beside which the regrown land's water would not be held
    /// ("The regrown ground runs over held ground at ..."); the join beside each piece of held ground
    /// is tight, and said.
    pub fn reseed(&self, seed: u64, cancel: &AtomicBool) -> Result<Edit, LandformError> {
        self.change(&self.drawing, seed, None, &self.asked, cancel, &mut |_| {})
    }

    /// The land's own drawing regrown as drawn on `area` (one flag a world cell, in the world's cell
    /// order; a flag beyond the world is not read), inside the area (the stage 2 plan's question 1,
    /// answer (a)): the drawing grown whole as an edit grows it, weighed by `area_outline`, so the
    /// outline is the area and every lake zone it touches, taken whole with its band, and the band
    /// lies inside the area from its edge inward. Outside that outline the land is as it was, bit for
    /// bit (heights, ground map, every layer, courses and lakes); the land is the drawing grown whole
    /// only farther than `BAND_M` inside the edge, and held ground lowers the weight round it as
    /// anywhere. The hand marks on the area's cells that are not held are cleared first, so the
    /// hand-shaped ground there becomes grown ground again; a held cell's ground does not change, so
    /// it stays held and keeps its hand mark. What the edit covers, for its refusal and its notes,
    /// is the area's own cells (`Covered::of_area`). The drawing asked for stays as it was, so what
    /// waits still waits. An area of no world cell gives the land back bit for bit, and so does one
    /// whose outline weighs 0 everywhere (held whole), with the edit's own notes. An area that
    /// reaches the world's border takes the land model's margin beyond it (`area_outline`), so it is
    /// regrown fully up to the border. Refused, cancelled and failing as `edit` is.
    pub fn regrow_area(&self, area: &[bool], cancel: &AtomicBool) -> Result<Edit, LandformError> {
        self.change(&self.drawing, self.seed, Some(area), &self.asked, cancel, &mut |_| {})
    }

    /// Holds (`on`) or lifts the hold on the world cells of `area` (one flag a world cell, in the
    /// world's cell order; a flag beyond the world is not read). A lake is held whole: holding any
    /// cell of a lake's water holds every cell of it, and lifting the hold on any cell of it lifts it
    /// on all of them (so, of lakes whose water meets, on each of them). A lake's water is its cells
    /// in the land's `water` layer at its level within where `fill_lakes` looks for it, the cells
    /// `lake_zones` takes a land's water by (`outline::lake_water`, reading a lake's level as
    /// `held.rs` does). A tool says so when it holds or lifts a lake. Nothing else of the land
    /// changes, the drawing asked for among it. Returns how many cells' mark it changed: those it
    /// held that were not held, or lifted that were.
    pub fn hold(&mut self, area: &[bool], on: bool) -> usize {
        self.hold_lakes(area, on).cells
    }

    /// `hold`, saying also which lakes it held or lifted whole (`Held`): the lakes whose water the
    /// call marked, through the area or through water that meets water the area reaches, and of
    /// whose cells it changed the mark of one at least. A lake already held whole that a hold
    /// passes over again is not among them, nor one a lift meets that was not held.
    pub fn hold_lakes(&mut self, area: &[bool], on: bool) -> Held {
        let reach = self.hold_reach(area);
        self.hold_reached(reach, on)
    }

    /// `hold_lakes` of the cells `reach` marks, worked out before from this land as it stands (`hold_reach`).
    pub fn hold_reached(&mut self, reach: HoldReach, on: bool) -> Held {
        let cells = self.world.heights.data.len();
        let HoldReach { marked, lakes } = reach;
        assert_eq!(marked.len(), cells, "a hold's reach is of a land of another size");
        if self.marks.held.len() < cells {
            self.marks.held.resize(cells, false);
        }
        let mut whole: Vec<usize> = (lakes.iter())
            .filter(|(_, water)| water.iter().any(|&cell| marked[cell] && self.marks.held[cell] != on))
            .map(|&(drawn, _)| drawn)
            .collect();
        whole.sort_unstable();
        whole.dedup();
        let mut changed = 0;
        for (held, _) in self.marks.held.iter_mut().zip(&marked).filter(|(_, &marked)| marked) {
            changed += usize::from(*held != on);
            *held = on;
        }
        Held { cells: changed, lakes: whole }
    }

    /// Whether `hold_lakes(area, on)` would change the held mark of a cell (`Held::cells` above 0),
    /// the land unchanged, so that a tool leaves what it keeps for the land as it is when a hold or a
    /// lift changes nothing (stage 2's ruling R12, N4). It works out the cells the call would mark
    /// as `hold_lakes` does.
    pub fn holds_anything(&self, area: &[bool], on: bool) -> bool {
        self.holds_anything_in(&self.hold_reach(area), on)
    }

    /// `holds_anything` of the cells `reach` marks, worked out before from this land as it stands (`hold_reach`).
    pub fn holds_anything_in(&self, reach: &HoldReach, on: bool) -> bool {
        (reach.marked.iter().enumerate()).any(|(cell, &marked)| marked && self.marks.held.get(cell).copied().unwrap_or(false) != on)
    }

    /// The world cells a hold or a lift of `area` marks (one flag a world cell in the world's order; a flag of `area`
    /// beyond the world is not read): the area's, and the whole water of every lake whose water meets a cell marked,
    /// again until none grows; with each lake's water by its place in the land's drawing (`HoldReach`).
    pub fn hold_reach(&self, area: &[bool]) -> HoldReach {
        let cells = self.world.heights.data.len();
        let mut marked: Vec<bool> = (0..cells).map(|cell| area.get(cell) == Some(&true)).collect();
        let lakes: Vec<(usize, Vec<usize>)> = (self.world.water.lakes.iter())
            .filter_map(|water| Some((water.drawn, lake_water(&self.world, self.drawing.lakes.get(water.drawn)?, water.drawn))))
            .collect();
        loop {
            let mut grew = false;
            for (_, water) in &lakes {
                if water.iter().any(|&cell| marked[cell]) && !water.iter().all(|&cell| marked[cell]) {
                    water.iter().for_each(|&cell| marked[cell] = true);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        HoldReach { marked, lakes }
    }

    /// `BadConfig` unless `drawing` is one an edit of the land takes, and one the land may ask for:
    /// a drawing `Drawing::validate` takes (its sentence naming the item), of the land's size.
    fn check_edited(&self, drawing: &Drawing) -> Result<(), LandformError> {
        drawing.validate()?;
        if drawing.size_m != self.drawing.size_m {
            return Err(LandformError::BadConfig(format!(
                "an edit keeps the land's size: the drawing is {} m across and the land {} m",
                drawing.size_m, self.drawing.size_m
            )));
        }
        Ok(())
    }

    /// `BadConfig` unless the land's world is as an edit reads it, and as `Land::grow` and every edit
    /// make it: its heights, ground map and layers of its drawing's size in cells of 1 m, the `water`
    /// layer among them, in the land model's frame (`base_elevation_m` 0).
    fn check_world(&self) -> Result<(), LandformError> {
        let size = self.drawing.size_m as usize;
        let world = &self.world;
        let fits = |width: usize, height: usize, cell_size: f32, cells: usize| (width, height, cells) == (size, size, size * size) && cell_size == 1.0;
        let grids = fits(world.heights.width, world.heights.height, world.heights.cell_size, world.heights.data.len())
            && fits(world.ground.width, world.ground.height, world.ground.cell_size, world.ground.data.len())
            && world.layers.values().all(|layer| fits(layer.width, layer.height, layer.cell_size, layer.data.len()));
        if !grids || !world.layers.contains_key(WATER_LAYER) || world.meta.base_elevation_m != 0.0 {
            return Err(LandformError::BadConfig(format!(
                "the land's world must be its drawing's {size} x {size} cells of 1 m, with the {WATER_LAYER} layer, in the land model's frame"
            )));
        }
        Ok(())
    }

    /// The kept land model: `model_water` of `model_heights` for the land's drawing, seed and water
    /// rules, with `first_lake_levels` as its lakes, the one way an edit derives it (the struct's
    /// "The kept model is derived one way, always"). Public so that a tool reads the land's own lakes'
    /// zones by it (`lake_zones`) before a regrowth of an area runs. Uses no thread pool.
    pub fn kept_model(&self) -> Result<Grown, LandformError> {
        Ok(self.kept_model_and_routed()?.0)
    }

    /// The sentence an edit toward `drawing` would be refused with before it grows anything: a lake
    /// of the land's drawing that `drawing` moves, resizes or takes away while some of its water is
    /// held or shaped by hand (`edit`'s refusal 1, in its words; stage 2's ruling R18, N2), so a tool
    /// says it while a person drags, before Regrow. The refusals that need the drawing grown (a lake
    /// added or changed beside fixed ground, a lake not taken whole, the water left beside fixed
    /// ground: 2 to 4) are not seen. `None` when the edit is not refused so, and for a drawing of
    /// another size, which `edit` refuses in its own words. Plain Rust, no thread pool.
    pub fn held_lake_refusal(&self, drawing: &Drawing) -> Option<String> {
        if drawing.size_m != self.drawing.size_m {
            return None;
        }
        lake_fixed(self, &self.marks, &lake_partners(&self.drawing, drawing))
    }

    /// The sentence a regrowth of `area` as drawn would be refused with by a lake its outline takes
    /// but not whole (`regrow_area`'s refusal 3, in its words; stage 2's ruling R18, N2), worked out
    /// from `zones`, the land's own lakes' zones (`lake_zones` of its world with its kept model as
    /// both models, as a tool keeps them), and `shape`, the area's outline with the marks the
    /// regrowth weighs it by (`area_outline` with `Marks::regrown` of the area): the area is weighed
    /// once more without the marks, as the regrowth weighs it. So a tool says it while a person
    /// paints, before Regrow. Not seen: the cells only the drawing grown whole adds to a lake's zone
    /// (the regrowth takes those too, and may then name another fixed cell, or refuse where this
    /// does not), and the water refusal (4), which needs the joined land. `None` without fixed
    /// ground, or when the regrowth is not refused so. Uses rayon's current pool, as `area_outline`
    /// does.
    pub fn area_lake_refusal(&self, area: &[bool], zones: &[Vec<usize>], shape: &Outline) -> Option<String> {
        let marks = self.marks.regrown(area);
        if !marks.held.contains(&true) && !marks.by_hand.contains(&true) {
            return None;
        }
        let free = area_outline(self.drawing.size_m, area, zones, &Marks::default());
        lake_not_whole(self, &marks, zones, shape, &free, |zone| zone + 1)
    }

    /// `kept_model`, and the lakes `model_water` gives those heights itself (`kept_lake_levels`
    /// reads them).
    fn kept_model_and_routed(&self) -> Result<(Grown, Vec<GrownLake>), LandformError> {
        let mut model = model_water(&self.drawing, &self.config.water, self.seed, &self.model_heights)?;
        let routed = std::mem::replace(&mut model.lakes, self.first_lake_levels.clone());
        Ok((model, routed))
    }

    /// The edit of `edit`, `edit_from_seed`, `reseed` and `regrow_area`: to `drawing` from `seed`,
    /// its footprint found from the two starts and its outline reaching beyond it, or, for an
    /// `area`, the area its own outline (`area_outline`), the edited land asking for `asked`;
    /// `after` is told each step done, before the cancel flag is read after it.
    pub(crate) fn change(
        &self,
        drawing: &Drawing,
        seed: u64,
        area: Option<&[bool]>,
        asked: &Drawing,
        cancel: &AtomicBool,
        after: &mut dyn FnMut(Step),
    ) -> Result<Edit, LandformError> {
        let clock = Instant::now();
        self.check_edited(drawing)?;
        self.check_world()?;
        check_cancel(cancel)?;
        let mut step = |done: Step| -> Result<(), LandformError> {
            after(done);
            check_cancel(cancel)
        };
        let (size_m, water) = (self.drawing.size_m, &self.config.water);
        let mut timings = EditTimings::default();
        let mut stage = Instant::now();

        // P1, P2: the footprint, from the land model's starts, or the area's.
        let marks = match area {
            Some(area) => self.marks.regrown(area),
            None => self.marks.clone(),
        };
        let found = match area {
            Some(area) => footprint_of_area(size_m, area),
            None => {
                let old_start = start_of(&self.drawing, water, self.seed)?;
                step(Step::OldStart)?;
                let new_start = start_of(drawing, water, seed)?;
                step(Step::NewStart)?;
                footprint(&old_start, &new_start, &self.drawing, drawing)
            }
        };
        step(Step::Footprint)?;
        timings.footprint_ms = lap(&mut stage);
        if found.is_empty() {
            // The land as it was, its own drawing with it, asking for `asked`; said when the drawing
            // given is another.
            let kept = drawing_bytes(drawing) != drawing_bytes(&self.drawing);
            let (notes, notes_about) = if kept { (vec![DRAWING_KEPT.to_string()], vec![About::Land]) } else { (Vec::new(), Vec::new()) };
            timings.total_ms = since(clock);
            let outline = match area {
                Some(area) => early_area_outline(size_m, area, &self.marks),
                None => early_outline(size_m, &found, &self.marks, self.reach_m),
            };
            let land = Land { asked: asked.clone(), ..self.clone() };
            return Ok(Edit { land, outline, troughs: Vec::new(), pits_left: 0, water: WaterJoin::default(), notes, notes_about, timings });
        }

        // P3: the edited drawing's model, grown whole.
        let new_model = grow_model(drawing, water, seed, cancel)?;
        step(Step::Grow)?;
        timings.grow_ms = lap(&mut stage);

        // P4, P5: the kept model, the lakes' zones, the outline and its weight; then the refusal.
        let (old_model, routed_lakes) = self.kept_model_and_routed()?;
        step(Step::OldModel)?;
        let zones = lake_zones(&self.world, &old_model, &new_model, &self.drawing, drawing);
        let covered = match area {
            Some(area) => Covered::of_area(size_m, area),
            None => Covered::new(size_m, &found),
        };
        let whole = found.whole.as_ref();
        let marked = marks.held.contains(&true) || marks.by_hand.contains(&true);
        // The outline, the mix, the fine steps and the join, again with a new lake's backwater taken
        // as a lake's zone where the join finds one past the outline that changes the outline's
        // weight (`backwater_cells`; stage 2's rulings R23 and R24), up to `BACKWATER_ROUNDS_MOST`
        // times more; refused where fixed ground stops it.
        // An area regrown as drawn is its own outline, with the lakes' zones it touches, and covers
        // its own cells (the stage 2 plan's question 1, answer (a)); an edit's outline reaches beyond
        // its footprint.
        let weigh = |marks: &Marks, backwater: &[usize]| {
            let with_backwater: Vec<Vec<usize>>;
            let taken: &[Vec<usize>] = if backwater.is_empty() {
                &zones
            } else {
                with_backwater = zones.iter().cloned().chain([backwater.to_vec()]).collect();
                &with_backwater
            };
            match area {
                Some(area) => area_outline(size_m, area, taken, marks),
                None => outline(size_m, &found, taken, marks, self.reach_m),
            }
        };
        let mut backwater: Vec<usize> = Vec::new();
        let mut taken_backwater: Option<Backwater> = None;
        let mut next_shape: Option<Outline> = None;
        let mut rounds = 0;
        let (shape, unmarked, mixed, mixed_world, mut world, joined_water) = loop {
            let shape = next_shape.take().unwrap_or_else(|| weigh(&marks, &backwater));
            let unmarked = marked.then(|| weigh(&Marks::default(), &backwater));
            let seen = Seen::new(self, drawing, &marks, &covered, (&old_model, &new_model), &zones, &shape, unmarked.as_ref(), whole);
            if let Some(sentence) = seen.refusal() {
                return Err(LandformError::Refused(sentence));
            }
            step(Step::Outline)?;
            timings.outline_ms += lap(&mut stage);
            if area.is_some() && shape.weight.data.iter().chain(&shape.model_weight).all(|&weight| weight == 0.0) {
                // An area whose outline weighs 0 everywhere, held whole (its cells all held, so none
                // of its hand marks cleared), regrows no ground: the land as it was, with the edit's
                // own notes (its tight joins and the share held), the land's own drawing and seed
                // unchanged.
                let own = own_notes(None, &[], 0, &WaterJoin::default(), &seen.tight_joins(), &covered, &marks);
                let (notes, notes_about) = own.into_iter().unzip();
                timings.total_ms = since(clock);
                let land = Land { asked: asked.clone(), marks, ..self.clone() };
                return Ok(Edit { land, outline: shape, troughs: Vec::new(), pits_left: 0, water: WaterJoin::default(), notes, notes_about, timings });
            }

            // P6 to P8: the mixed model.
            let mut mixed = mixed_model(&old_model, &new_model, drawing, water, seed, &shape.model_weight)?;
            kept_lake_levels(&mut mixed.model.lakes, &routed_lakes, &old_model.lakes, &lake_partners(&self.drawing, drawing));
            step(Step::Mix)?;
            timings.mix_ms += lap(&mut stage);

            // P9: the fine steps, once a round, from the mixed model.
            let mixed_world = finish_drawn(seed, drawing, &self.config, &mixed.model, Frame::Model, cancel)?;
            step(Step::FineSteps)?;
            timings.fine_steps_ms += lap(&mut stage);

            // P10, P11: the ground, then the water.
            let mut world = join_ground(&self.world, &mixed_world, &shape.weight);
            step(Step::JoinGround)?;
            let joined_water = join_water(&mut world, &self.world, &mixed_world, &shape.weight, &self.drawing, drawing, water);
            step(Step::JoinWater)?;
            if area.is_none() {
                let found_now = backwater_cells(&self.world, &mixed_world, &shape.weight, &joined_water, &self.drawing, drawing, |cell| marks.fixed(cell));
                if let Some(cell) = found_now.stopped {
                    return Err(LandformError::Refused(seen.backwater_refusal(&found_now, world_place(size_m as usize, cell), cell)));
                }
                let mut grown: Vec<usize> = backwater.iter().copied().chain(found_now.cells).collect();
                grown.sort_unstable();
                grown.dedup();
                if rounds < BACKWATER_ROUNDS_MOST && grown.len() > backwater.len() {
                    let grown_shape = weigh(&marks, &grown);
                    if grown_shape.weight.data != shape.weight.data || grown_shape.model_weight != shape.model_weight {
                        (backwater, next_shape) = (grown, Some(grown_shape));
                        taken_backwater = Some(Backwater { cells: backwater.clone(), ..found_now });
                        rounds += 1;
                        timings.join_ms += lap(&mut stage);
                        continue;
                    }
                }
            }
            break (shape, unmarked, mixed, mixed_world, world, joined_water);
        };
        let seen = Seen::new(self, drawing, &marks, &covered, (&old_model, &new_model), &zones, &shape, unmarked.as_ref(), whole);
        if let Some(sentence) = seen.water_refusal(&world, taken_backwater.as_ref()) {
            return Err(LandformError::Refused(sentence));
        }
        step(Step::WaterBesideFixed)?;
        let touched = seen.touched(&mixed_world);
        let own = own_notes(whole, &mixed.troughs, mixed.pits_left, &joined_water, &seen.tight_joins(), &covered, &marks);
        let (notes, notes_about) = edit_notes(own, &mixed_world, &touched);
        (world.notes, world.notes_about) = land_notes(&self.world, &mixed_world, &touched);
        timings.join_ms += lap(&mut stage);

        let land = Land {
            drawing: drawing.clone(),
            asked: asked.clone(),
            seed,
            config: self.config,
            model_heights: mixed.model.heights,
            first_lake_levels: mixed.model.lakes,
            world,
            marks,
            reach_m: self.reach_m,
        };
        check_keepable(&land)?;
        timings.total_ms = since(clock);
        Ok(Edit { land, outline: shape, troughs: mixed.troughs, pits_left: mixed.pits_left, water: joined_water, notes, notes_about, timings })
    }
}

/// The mixed model's lakes (`lakes`, as `model_water` gives them) with each lake the mix leaves as
/// the kept model's heights give it at the kept model's own level: where a lake's level is the one
/// `model_water` gives the kept model's heights (`routed`) for its partner in the kept drawing
/// (`partners`, `lake_partners` of the kept drawing and the edited one), bit for bit, it takes the
/// level the kept model holds (`kept`, the land's `first_lake_levels`). A land just grown keeps
/// `grow`'s levels, worked out in `f64`, where `model_water` of its `f32` heights may give a lake a
/// last bit lower (`wide` at 1,024 m in hills from seed 3, 4.908266 against 4.9082656); so an edit
/// that leaves such a lake as it was keeps its level, and the fine steps lay it as the land holds
/// it, and a land regrown as it was is that land (stage 2's ruling R8, M1).
pub(crate) fn kept_lake_levels(lakes: &mut [GrownLake], routed: &[GrownLake], kept: &[GrownLake], partners: &[Option<usize>]) {
    for lake in lakes.iter_mut() {
        let Some(old) = partners.iter().position(|&partner| partner == Some(lake.drawn)) else { continue };
        let level_of = |lakes: &[GrownLake]| lakes.iter().find(|other| other.drawn == old).map(|other| other.level_m);
        if let (Some(routed_m), Some(kept_m)) = (level_of(routed), level_of(kept)) {
            if routed_m.to_bits() == lake.level_m.to_bits() {
                lake.level_m = kept_m;
            }
        }
    }
}

/// `BadConfig` unless `land`, one `Land::grow` or an edit made, is a land the land file's reader
/// reads (`file::check`, called and not copied, so the two cannot differ): the reader's sentence,
/// which names the course whose water it would refuse.
fn check_keepable(land: &Land) -> Result<(), LandformError> {
    file::check(land).map_err(|reason| LandformError::BadConfig(format!("the land it makes could not be written and read again as a land file, since {reason}")))
}

/// The milliseconds since `clock`, and the clock started again.
fn lap(clock: &mut Instant) -> f32 {
    let ms = since(*clock);
    *clock = Instant::now();
    ms
}

/// The milliseconds since `clock`.
fn since(clock: Instant) -> f32 {
    clock.elapsed().as_secs_f32() * 1000.0
}

/// What an edit has found by its outline, which its refusal and its notes read.
struct Seen<'a> {
    land: &'a Land,
    /// The edited drawing.
    drawing: &'a Drawing,
    /// The marks the edit runs with: the land's, with an area's hand marks cleared.
    marks: &'a Marks,
    covered: &'a Covered,
    old_model: &'a Grown,
    new_model: &'a Grown,
    zones: &'a [Vec<usize>],
    /// The outline with the marks, and the same outline weighed without them.
    shape: &'a Outline,
    free: &'a Outline,
    /// The world cells lowered by fixed ground (`Land::edit`, **The refusal**), one flag a cell;
    /// `None` when the land has no fixed cell, and so the outline was weighed once.
    lowered: Option<Vec<bool>>,
    /// Why the footprint is the whole land, where it is so (`Footprint::whole`).
    whole: Option<&'a Whole>,
}

/// What an edit's refusal names (`Land::edit`, **The refusal**), each lake and river by its number
/// (its place in the drawing, from 1), which gives the sentence its words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// 1: a lake the edit moves, resizes or takes away, some of whose water is fixed.
    LakeFixed(usize),
    /// 2: a lake the edit adds or changes, its new water lowered by fixed ground.
    NewLake(usize),
    /// 3: a lake whose zone the outline takes, some of it lowered by fixed ground.
    LakeNotWhole(usize),
    /// 4, the water left beside fixed ground, by what the edit changes nearest it: a drawn river
    /// whose trench the footprint touches, by its new line;
    River(usize),
    /// a drawn river the edit removes, by its line in the kept model, numbered in the land's drawing;
    RemovedRiver(usize),
    /// a lake the edit adds or changes, by its drawn rim;
    LakeWater(usize),
    /// the picture of high ground, by the footprint;
    Painted,
    /// or, with the picture the same, the ground the edit regrows, by the footprint;
    Regrown,
    /// or a lake the edit adds, moves or resizes whose backwater reaches fixed ground (stage 2's
    /// ruling R24), with the drawn river it holds up (`None` for a brook).
    Backwater(usize, Option<usize>),
}

/// The sentence an edit is refused with (`Land::edit`, **The refusal**): `refusal` names the item;
/// the fixed cell named lies at world `at`, held, or shaped by hand where `by_hand`; and the item
/// lies `apart_m` from it. Said in whole metres: "within 150 m of held ground at (x, z), d m from
/// it", "over held ground at (x, z)" where it rounds to 0, and "d m from held ground at (x, z)"
/// where it rounds to `BAND_M` or more (which only the water's sentences can give, their distance
/// being the item's own and not that of a lowered cell); a lake with fixed water names only its
/// fixed cell nearest its drawn centre. One sentence each.
fn refusal_sentence(refusal: Refusal, by_hand: bool, at: (f64, f64), apart_m: f64) -> String {
    let (kind, lift) = if by_hand { (BY_HAND, "regrow that ground as drawn") } else { (HELD, "lift the hold") };
    let there = place(at);
    let (what, water, mend) = match refusal {
        Refusal::LakeFixed(number) if by_hand => {
            return format!("Lake {number} lies on {BY_HAND} at {there}, so the edit cannot move, resize or take it away; regrow that ground as drawn first.");
        }
        Refusal::LakeFixed(number) => return format!("Lake {number} is held at {there}, so the edit cannot move, resize or take it away; lift the hold on it first."),
        Refusal::Backwater(lake, river) => {
            let river = river.map_or_else(|| "a brook".to_string(), |river| format!("river {river}"));
            let apart_m = apart_m.round() as i64;
            let from = if apart_m == 0 { String::new() } else { format!(", {apart_m} m from it") };
            return format!("Lake {lake} holds {river} up to {kind} at {there}{from}; the water there would not be held: move the lake down the river, or {lift} there.");
        }
        Refusal::NewLake(number) => (format!("Lake {number}'s new water lies"), false, "move the lake"),
        Refusal::LakeNotWhole(number) => (format!("Lake {number} would not be taken whole: its water and shore lie"), false, "keep the edit farther from the lake"),
        Refusal::River(number) => (format!("River {number}'s new line runs"), true, "move the line"),
        Refusal::RemovedRiver(number) => (format!("River {number}, which this edit removes, ran"), true, "keep the river"),
        Refusal::LakeWater(number) => (format!("Lake {number}'s new water lies"), true, "move the lake"),
        Refusal::Painted => ("The painted ground runs".to_string(), true, "paint farther from it"),
        Refusal::Regrown => ("The regrown ground runs".to_string(), true, "keep the edit farther from it"),
    };
    let apart_m = apart_m.round() as i64;
    let near = if apart_m == 0 {
        format!("over {kind} at {there}")
    } else if apart_m < BAND_M as i64 {
        format!("within {BAND_M} m of {kind} at {there}, {apart_m} m from it")
    } else {
        format!("{apart_m} m from {kind} at {there}")
    };
    let why = if water { "; the water there would not be held: " } else { "; " };
    format!("{what} {near}{why}{mend}, or {lift} there.")
}

/// Which rivers and lakes an edit's outline touches, as the notes ask it (`Land::edit`, **The
/// notes**).
struct Touched {
    /// For each river and each lake of the edited drawing, whether the outline touches it.
    rivers: Vec<bool>,
    lakes: Vec<bool>,
    /// The rivers and lakes whose kept notes stand: untouched, at the same place in both drawings.
    kept_rivers: BTreeSet<usize>,
    kept_lakes: BTreeSet<usize>,
}

impl<'a> Seen<'a> {
    /// What an edit of `land` to `drawing` has found by its outline `shape`, weighed with `marks`,
    /// and `unmarked`, the same outline weighed without them where the land has fixed cells (`None`
    /// where it has none, and `shape` is weighed once); the two models, the kept one first.
    #[allow(clippy::too_many_arguments)]
    fn new(
        land: &'a Land,
        drawing: &'a Drawing,
        marks: &'a Marks,
        covered: &'a Covered,
        (old_model, new_model): (&'a Grown, &'a Grown),
        zones: &'a [Vec<usize>],
        shape: &'a Outline,
        unmarked: Option<&'a Outline>,
        whole: Option<&'a Whole>,
    ) -> Self {
        let lowered = unmarked.map(|free| shape.weight.data.iter().zip(&free.weight.data).map(|(shape, free)| shape < free).collect());
        Seen { land, drawing, marks, covered, old_model, new_model, zones, shape, free: unmarked.unwrap_or(shape), lowered, whole }
    }
}

impl Seen<'_> {
    /// The land's cells a side.
    fn size(&self) -> usize {
        self.land.drawing.size_m as usize
    }

    /// The sentence the edit is refused with by its lakes, or `None` (`Land::edit`, **The
    /// refusal**, 1 to 3).
    fn refusal(&self) -> Option<String> {
        let (old, new, size) = (&self.land.drawing, self.drawing, self.size());
        let lake_partner = lake_partners(old, new);

        // 1. A held lake moved, resized or taken away.
        if let Some(sentence) = lake_fixed(self.land, self.marks, &lake_partner) {
            return Some(sentence);
        }

        // 2. A lake added or changed, partly taken beside fixed ground.
        let unpaired = unpaired_lakes(&lake_partner, new);
        for (drawn, lake) in unpaired.iter().map(|&drawn| (drawn, &new.lakes[drawn])) {
            let Some(grown) = self.new_model.lakes.iter().find(|grown| grown.drawn == drawn) else { continue };
            let cells = model_lake_cells(self.new_model, lake, grown.level_m, (size, size));
            let own = cells.into_iter().filter(|&cell| self.covered.world_cell(cell));
            if let Some(cell) = lowered(own, &self.shape.weight.data, &self.free.weight.data) {
                return Some(self.beside(Refusal::NewLake(drawn + 1), world_place(size, cell)));
            }
        }

        // 3. A lake whose zone the outline takes, not taken whole: a kept lake by its place in the
        // edited drawing, or in the land's where it has none; an added one by its place.
        let number = |zone: usize| match zone.checked_sub(old.lakes.len()) {
            None => lake_partner[zone].unwrap_or(zone) + 1,
            Some(added) => unpaired[added] + 1,
        };
        lake_not_whole(self.land, self.marks, self.zones, self.shape, self.free, number)
    }

    /// The sentence the edit is refused with by the water `joined`, the edited land, would leave
    /// beside fixed ground, or `None` (`Land::edit`, **The refusal**, 4): the fixed cell named is
    /// the one nearest the first new unheld cell, and the item named what the edit changes nearest
    /// that cell (`changed_nearest`), with its distance from the fixed cell; or, where the cell lies
    /// within reach of `backwater`, the backwater the edit took, the lake that holds it up
    /// (`backwater_lake`), with the cell's distance from the fixed cell.
    fn water_refusal(&self, joined: &World, backwater: Option<&Backwater>) -> Option<String> {
        let lowered = self.lowered.as_deref()?;
        if !lowered.contains(&true) {
            return None;
        }
        let size = self.size();
        let (now, before) = (unheld_cells(joined)?, unheld_cells(&self.land.world)?);
        let cell = first_new_unheld(&now, &before, lowered, size)?;
        let at = world_place(size, cell);
        let (fixed, _) = nearest_fixed(self.marks, size, at).expect("a cell lowered by fixed ground has fixed ground to be lowered by");
        let fixed_at = world_place(size, fixed);
        // Within the reach of a backwater the edit took, the lake that holds it up (stage 2's
        // ruling R24), the water's distance from the fixed cell.
        let reach_m = f64::from(BAND_M) + LAKE_ZONE_MARGIN_M;
        let near_backwater = backwater.filter(|found| found.cells.iter().any(|&taken| distance(world_place(size, taken), at) <= reach_m));
        let lake = near_backwater.and_then(|found| self.backwater_lake(found, at).map(|lake| (lake, found.river)));
        let (refusal, apart_m) = match lake {
            Some((lake, river)) => (Refusal::Backwater(lake + 1, river.map(|river| river + 1)), distance(at, fixed_at)),
            None => self.changed_nearest(at, fixed_at),
        };
        Some(refusal_sentence(refusal, !self.is_held(fixed), fixed_at, apart_m))
    }

    /// The sentence the edit is refused with where fixed ground stops a new lake's backwater
    /// (`Land::edit`, **The backwater**; stage 2's ruling R24): at world `at`, the fixed cell `cell`
    /// the backwater holds its river up over (`Backwater::stopped`), the lake that holds it, where
    /// the edit adds, moves or resizes it (`backwater_lake`); failing one, the item the edit changes
    /// nearest (`changed_nearest`), with its distance.
    fn backwater_refusal(&self, found: &Backwater, at: (f64, f64), cell: usize) -> String {
        let by_hand = !self.is_held(cell);
        match self.backwater_lake(found, at) {
            Some(lake) => refusal_sentence(Refusal::Backwater(lake + 1, found.river.map(|river| river + 1)), by_hand, at, 0.0),
            None => {
                let (refusal, apart_m) = self.changed_nearest(at, at);
                refusal_sentence(refusal, by_hand, at, apart_m)
            }
        }
    }

    /// The lake of the edited drawing that holds a backwater up (`Backwater::lake`) where the edit
    /// adds, moves or resizes it (a lake with no partner in the land's drawing, `lake_partners`);
    /// failing that, of the lakes it adds, moves or resizes, the one whose drawn rim lies nearest
    /// world `at`, the first of several as near; `None` where it changes no lake.
    fn backwater_lake(&self, found: &Backwater, at: (f64, f64)) -> Option<usize> {
        let new = self.drawing;
        let changed = unpaired_lakes(&lake_partners(&self.land.drawing, new), new);
        if let Some(lake) = found.lake.filter(|lake| changed.contains(lake)) {
            return Some(lake);
        }
        let rim = |lake: usize| (distance(at, (new.lakes[lake].x_m, new.lakes[lake].z_m)) - f64::from(new.lakes[lake].radius_m)).max(0.0);
        changed.into_iter().min_by(|&a, &b| rim(a).total_cmp(&rim(b)))
    }

    /// What the edit changes nearest world `at` (`Land::edit`, **The refusal**, 4), and how far it
    /// lies from world `fixed_at`: of the edited drawing's rivers whose trench the footprint touches,
    /// each by its new line (its smoothed line in the edited drawing's model), and the land's
    /// drawn rivers the edit removes (those with no partner in the edited drawing, `river_partners`),
    /// each by its line in the kept model, the one whose line lies nearest `at`, with that line's
    /// distance (the controller's ruling R24); failing one, of the lakes the edit adds or changes,
    /// the one whose drawn rim lies nearest, with the rim's distance (0 inside it); failing one, the
    /// painted ground where the picture of high ground differs, or else the regrown ground, with the
    /// distance of the footprint's nearest world cell. The first in the drawing of several as near,
    /// a river of the edited drawing before one it removes. An edit of the land's own drawing
    /// (`reseed`, `regrow_area`) changes no drawn river or lake, though its footprint touches their
    /// trenches: it names the regrown ground; and so does an edit whose footprint is the whole land,
    /// which regrows every river whether its line moved or not.
    fn changed_nearest(&self, at: (f64, f64), fixed_at: (f64, f64)) -> (Refusal, f64) {
        let (old, new) = (&self.land.drawing, self.drawing);
        let drawn = drawing_bytes(new) != drawing_bytes(old) && self.whole.is_none();
        let changed = (self.new_model.rivers.iter())
            .filter(|river| drawn && self.trench_touched(river))
            .map(|river| (Refusal::River(river.drawn + 1), &river.line));
        let partner = if drawn { river_partners(old, new) } else { Vec::new() };
        let removed = (self.old_model.rivers.iter())
            .filter(|river| partner.get(river.drawn) == Some(&None))
            .map(|river| (Refusal::RemovedRiver(river.drawn + 1), &river.line));
        if let Some((refusal, line)) = changed.chain(removed).min_by(|a, b| to_line(at, a.1).total_cmp(&to_line(at, b.1))) {
            return (refusal, to_line(fixed_at, line));
        }
        let rim = |lake: &Lake, p: (f64, f64)| (distance(p, (lake.x_m, lake.z_m)) - f64::from(lake.radius_m)).max(0.0);
        let lakes = if drawn { unpaired_lakes(&lake_partners(old, new), new) } else { Vec::new() };
        if let Some(drawn) = lakes.into_iter().min_by(|&a, &b| rim(&new.lakes[a], at).total_cmp(&rim(&new.lakes[b], at))) {
            return (Refusal::LakeWater(drawn + 1), rim(&new.lakes[drawn], fixed_at));
        }
        let size = self.size();
        let apart_m = self.covered.world_cells().map(|cell| distance(world_place(size, cell), fixed_at)).fold(f64::INFINITY, f64::min);
        (if drawn && old.high_ground != new.high_ground { Refusal::Painted } else { Refusal::Regrown }, apart_m)
    }

    /// Whether world cell `cell` is held (where it is fixed but not held, it is shaped by hand).
    fn is_held(&self, cell: usize) -> bool {
        self.marks.held.get(cell) == Some(&true)
    }

    /// Whether the footprint touches `river`'s trench: a model cell it covers lies within
    /// `TRENCH_REACH_M` of the river's smoothed line.
    fn trench_touched(&self, river: &GrownRiver) -> bool {
        let near = distances_to_line(&river.line, self.size() as u32, TRENCH_REACH_M);
        (0..near.len()).any(|cell| near[cell] < TRENCH_REACH_M && self.covered.model_cell(cell))
    }

    /// The edit's notes of tight joins beside fixed ground (`Land::edit`, **A tight join**): one for
    /// each piece of fixed ground, joined across sides and corners, that lowers a world cell of the
    /// footprint (holds the fixed cell nearest it, `nearest_fixed_cells`), in the order of the
    /// pieces' first cells, each at the middle of its cells. What it costs grows with the cells, not
    /// with the cells times the fixed cells: one distance transform over the lowered cells' box,
    /// then each piece named followed once over its own cells.
    fn tight_joins(&self) -> Vec<String> {
        let Some(lowered) = self.lowered.as_deref() else { return Vec::new() };
        let size = self.size();
        let tight: Vec<usize> = self.covered.world_cells().filter(|&cell| lowered[cell]).collect();
        let mut seen = vec![false; if tight.is_empty() { 0 } else { size * size }];
        let mut pieces = Vec::new();
        for fixed in nearest_fixed_cells(self.land.drawing.size_m, self.marks, &tight, f64::from(BAND_M)).into_iter().flatten() {
            if !seen[fixed] {
                pieces.push(piece_from(self.marks, size, fixed, &mut seen));
            }
        }
        pieces.sort_by_key(|piece| piece.first);
        let half = size as f64 / 2.0;
        (pieces.iter())
            .map(|piece| {
                let middle = (piece.columns as f64 / piece.cells as f64 - half, piece.rows as f64 / piece.cells as f64 - half);
                let kind = if self.is_held(piece.first) { HELD } else { BY_HAND };
                format!("The join beside {kind} at {} is tight: the new land is only partly taken within {BAND_M} m of it.", place(middle))
            })
            .collect()
    }

    /// Whether the outline takes the zone of `cells`: one of them lies inside it.
    fn zone_taken(&self, cells: &[usize]) -> bool {
        cells.iter().any(|&cell| self.free.weight.data.get(cell).is_some_and(|&w| w > 0.0))
    }

    /// The sentence of lake refusal 2: `at` the place of the item's cell weighing least, the fixed
    /// cell named the one nearest it, with its distance.
    fn beside(&self, refusal: Refusal, at: (f64, f64)) -> String {
        beside(self.marks, self.size(), refusal, at)
    }

    /// Which rivers and lakes the outline touches (`Land::edit`, **The notes**), `mixed` the mixed
    /// model's world.
    fn touched(&self, mixed: &World) -> Touched {
        let (old, new) = (&self.land.drawing, self.drawing);
        let weight = &self.shape.weight;
        let inside = |(x, z): (f64, f64)| weight_at(weight, x, z) > 0.0;
        let courses_touch = |world: &World, drawn: usize| {
            (world.water.rivers.iter())
                .filter(|course| course.drawn == Some(drawn))
                .any(|course| course.points.iter().any(|point| inside((f64::from(point.x_m), f64::from(point.z_m)))))
        };
        let model_river = |model: &Grown, drawn: usize| model.rivers.iter().find(|river| river.drawn == drawn).is_some_and(|river| self.trench_touched(river));
        let river_partner = river_partners(old, new);
        let old_touched: Vec<bool> = (0..old.rivers.len()).map(|drawn| courses_touch(&self.land.world, drawn) || model_river(self.old_model, drawn)).collect();
        let mut rivers: Vec<bool> = (0..new.rivers.len()).map(|drawn| courses_touch(mixed, drawn) || model_river(self.new_model, drawn)).collect();
        for (drawn, partner) in river_partner.iter().enumerate() {
            if let Some(partner) = *partner {
                rivers[partner] |= old_touched[drawn];
            }
        }
        let kept_rivers = (0..old.rivers.len()).filter(|&drawn| river_partner[drawn] == Some(drawn) && !rivers[drawn]).collect();

        let middle_inside = |water: &[LakeWater], drawn: usize| water.iter().filter(|lake| lake.drawn == drawn).any(|lake| inside((f64::from(lake.x_m), f64::from(lake.z_m))));
        let lake_partner = lake_partners(old, new);
        // A lake the edit adds, moves or resizes lies in the footprint (its disc): touched.
        let mut lakes = vec![true; new.lakes.len()];
        for (drawn, partner) in lake_partner.iter().enumerate() {
            let Some(partner) = *partner else { continue };
            lakes[partner] = self.zones.get(drawn).is_some_and(|cells| self.zone_taken(cells))
                || middle_inside(&self.land.world.water.lakes, drawn)
                || middle_inside(&mixed.water.lakes, partner)
                || inside((old.lakes[drawn].x_m, old.lakes[drawn].z_m))
                || inside((new.lakes[partner].x_m, new.lakes[partner].z_m));
        }
        let kept_lakes = (0..old.lakes.len()).filter(|&drawn| lake_partner[drawn] == Some(drawn) && !lakes[drawn]).collect();
        Touched { rivers, lakes, kept_rivers, kept_lakes }
    }
}

/// The world cell a place of `unheld_cells` lies on, in a land `size` cells a side.
fn cell_at(size: usize, (x, z): (f64, f64)) -> usize {
    let half = size as f64 / 2.0;
    let index = |at: f64| ((at + half).round().max(0.0) as usize).min(size - 1);
    index(z) * size + index(x)
}

/// The first world cell, in the world's cell order, of the unheld places of `now` that lie on a cell
/// `lowered` marks and that `before` has not at the same place in the same way (a dry cell, a lake
/// cell over lower water, or a step between the same two cells, of which the first cell `lowered`
/// marks): `None` when there is none.
fn first_new_unheld(now: &UnheldCells, before: &UnheldCells, lowered: &[bool], size: usize) -> Option<usize> {
    let dry: BTreeSet<usize> = before.dry.iter().map(|cell| cell_at(size, cell.at)).collect();
    let over: BTreeSet<usize> = before.lake_over_water.iter().map(|cell| cell_at(size, cell.at)).collect();
    let steps: BTreeSet<(usize, usize)> = before.steps.iter().map(|step| (cell_at(size, step.a), cell_at(size, step.b))).collect();
    let new_dry = now.dry.iter().map(|cell| cell_at(size, cell.at)).filter(|cell| lowered[*cell] && !dry.contains(cell));
    let new_over = now.lake_over_water.iter().map(|cell| cell_at(size, cell.at)).filter(|cell| lowered[*cell] && !over.contains(cell));
    let new_steps = (now.steps.iter())
        .map(|step| (cell_at(size, step.a), cell_at(size, step.b)))
        .filter(|pair| !steps.contains(pair))
        .filter_map(|(a, b)| [a, b].into_iter().find(|&cell| lowered[cell]));
    new_dry.chain(new_over).chain(new_steps).min()
}

/// The lakes of `new` the edit adds or changes: those no lake of the land's drawing pairs with
/// (`partners`, `lake_partners`' list), in `new`'s order.
fn unpaired_lakes(partners: &[Option<usize>], new: &Drawing) -> Vec<usize> {
    let mut paired = vec![false; new.lakes.len()];
    partners.iter().flatten().for_each(|&drawn| paired[drawn] = true);
    (0..new.lakes.len()).filter(|&drawn| !paired[drawn]).collect()
}

/// The distance from world `at` to `line` (world metres): to its nearest stretch, or to its one
/// point; infinite for no point.
fn to_line(at: (f64, f64), line: &[(f64, f64)]) -> f64 {
    match line {
        [] => f64::INFINITY,
        [only] => distance(*only, at),
        _ => line.windows(2).map(|pair| to_segment(at, pair[0], pair[1])).fold(f64::INFINITY, f64::min),
    }
}

/// A piece of fixed ground, its cells joined across sides and corners: its first world cell in the
/// world's cell order, how many cells it holds, and the sums of their columns and of their rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Piece {
    first: usize,
    cells: u64,
    columns: u64,
    rows: u64,
}

/// The piece of fixed ground (`marks`) of a land `size` cells a side that holds fixed world cell
/// `start`, each of its cells marked in `seen` (one flag a world cell), which none of them may be
/// before: followed once over its own cells.
fn piece_from(marks: &Marks, size: usize, start: usize, seen: &mut [bool]) -> Piece {
    let mut piece = Piece { first: start, cells: 0, columns: 0, rows: 0 };
    seen[start] = true;
    let mut open = vec![start];
    while let Some(cell) = open.pop() {
        let (i, j) = (cell % size, cell / size);
        piece.first = piece.first.min(cell);
        piece.cells += 1;
        piece.columns += i as u64;
        piece.rows += j as u64;
        for nj in j.saturating_sub(1)..=(j + 1).min(size - 1) {
            for ni in i.saturating_sub(1)..=(i + 1).min(size - 1) {
                let next = nj * size + ni;
                if !seen[next] && marks.fixed(next) {
                    seen[next] = true;
                    open.push(next);
                }
            }
        }
    }
    piece
}

/// Lake refusal 1 (`Land::edit`, **The refusal**): a lake of the land's drawing that the edited
/// drawing moves, resizes or takes away (`lake_partner`, by `lake_partners`, gives it none) while
/// some of its water (`lake_water`) is fixed in `marks`, named by its fixed cell nearest its drawn
/// centre, held or shaped by hand.
fn lake_fixed(land: &Land, marks: &Marks, lake_partner: &[Option<usize>]) -> Option<String> {
    let size = land.drawing.size_m as usize;
    for (drawn, lake) in land.drawing.lakes.iter().enumerate().filter(|&(drawn, _)| lake_partner[drawn].is_none()) {
        let centre = (lake.x_m, lake.z_m);
        let nearest = (lake_water(&land.world, lake, drawn).into_iter())
            .filter(|&cell| marks.fixed(cell))
            .min_by(|&a, &b| distance(world_place(size, a), centre).total_cmp(&distance(world_place(size, b), centre)));
        if let Some(cell) = nearest {
            return Some(refusal_sentence(Refusal::LakeFixed(drawn + 1), marks.held.get(cell) != Some(&true), world_place(size, cell), 0.0));
        }
    }
    None
}

/// Lake refusal 3 (`Land::edit`, **The refusal**): of the lakes' `zones` that the outline takes (a
/// cell of the zone weighs above 0 in `free`, the outline weighed without the marks), the first one
/// some cell of which `shape` (the outline with `marks`) weighs below `free`, named by
/// `number(zone)` at its cell weighing least. A kept lake's own fixed cells are not asked, nor a
/// lake whose water is fixed whole, which lies wholly at weight 0.
fn lake_not_whole(land: &Land, marks: &Marks, zones: &[Vec<usize>], shape: &Outline, free: &Outline, number: impl Fn(usize) -> usize) -> Option<String> {
    let size = land.drawing.size_m as usize;
    let taken = |cells: &[usize]| cells.iter().any(|&cell| free.weight.data.get(cell).is_some_and(|&w| w > 0.0));
    for (zone, cells) in zones.iter().enumerate().filter(|(_, cells)| taken(cells)) {
        let own: Vec<usize> = land.drawing.lakes.get(zone).map_or_else(Vec::new, |lake| lake_water(&land.world, lake, zone));
        let own_fixed: BTreeSet<usize> = own.iter().copied().filter(|&cell| marks.fixed(cell)).collect();
        if !own.is_empty() && own_fixed.len() == own.len() {
            continue;
        }
        let asked = cells.iter().copied().filter(|cell| !own_fixed.contains(cell));
        if let Some(cell) = lowered(asked, &shape.weight.data, &free.weight.data) {
            return Some(beside(marks, size, Refusal::LakeNotWhole(number(zone)), world_place(size, cell)));
        }
    }
    None
}

/// The sentence of lake refusal 2 or 3: `at` the place of the item's cell weighing least, the
/// fixed cell of `marks` named the one nearest it, with its distance, held or shaped by hand.
fn beside(marks: &Marks, size: usize, refusal: Refusal, at: (f64, f64)) -> String {
    let (cell, apart_m) = nearest_fixed(marks, size, at).expect("a weight lowered by fixed ground has fixed ground to be lowered by");
    refusal_sentence(refusal, marks.held.get(cell) != Some(&true), world_place(size, cell), apart_m)
}

/// Of `cells`, the one whose weight in `shape` lies below its weight in `free` and is least (the
/// first in the order given of several as low), or `None` when none lies below. `shape` and `free`
/// are one outline weighed with and without the marks, so a cell lies below exactly where fixed
/// ground lies nearer it than the rim, and the least weight lies nearest that ground.
fn lowered(cells: impl Iterator<Item = usize>, shape: &[f32], free: &[f32]) -> Option<usize> {
    let mut lowest: Option<usize> = None;
    for cell in cells.filter(|&cell| shape[cell] < free[cell]) {
        if lowest.is_none_or(|lowest| shape[cell] < shape[lowest]) {
            lowest = Some(cell);
        }
    }
    lowest
}

/// The fixed world cell nearest world `at`, in metres (the first in the world's cell order of
/// several as near), with its distance; `None` when no cell is fixed.
fn nearest_fixed(marks: &Marks, size: usize, at: (f64, f64)) -> Option<(usize, f64)> {
    let mut nearest: Option<(usize, f64)> = None;
    for cell in (0..size * size).filter(|&cell| marks.fixed(cell)) {
        let apart_m = distance(world_place(size, cell), at);
        if nearest.is_none_or(|(_, best)| apart_m < best) {
            nearest = Some((cell, apart_m));
        }
    }
    nearest
}

/// World `(x, z)` of world cell `cell` of a land `size` cells a side, in metres, the half taken
/// exactly (`outline`'s grids).
fn world_place(size: usize, cell: usize) -> (f64, f64) {
    let half = size as f64 / 2.0;
    ((cell % size) as f64 - half, (cell / size) as f64 - half)
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// A place in a sentence: world `(x, z)` in whole metres.
fn place((x, z): (f64, f64)) -> String {
    format!("({}, {})", x.round() as i64, z.round() as i64)
}

/// The weight `weight` gives world `(x, z)`: that of the world cell it lies on, held to the grid, as
/// `join_water` reads it.
fn weight_at(weight: &Grid<f32>, x: f64, z: f64) -> f32 {
    let cell = |at: f64, count: usize| ((at + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
    weight.data[cell(z, weight.height) * weight.width + cell(x, weight.width)]
}

/// The edit's own notes and what each is about (`Land::edit`, **The notes**): the land, but for a
/// note that names a drawn river (its course narrowed to nothing or cut off at the outline, risen
/// before it was mended, or its end lost), which is about that river (the controller's ruling R23).
fn own_notes(whole: Option<&Whole>, troughs: &[Trough], pits_left: usize, water: &WaterJoin, tight: &[String], covered: &Covered, marks: &Marks) -> Vec<(String, About)> {
    let mut notes: Vec<(String, About)> = Vec::new();
    let mut say = |note: String| notes.push((note, About::Land));
    if let Some(whole) = whole {
        say(format!("{WHOLE_LAND}{}.", whole.reason()));
    }
    for trough in troughs.iter().filter(|trough| trough.depth_m > TROUGH_SAID_M) {
        if trough.filled {
            say(format!("A pit in the band at {} was filled, the ground raised by up to {:.1} m, as no way out could be cut for it.", place(trough.at), trough.depth_m));
        } else {
            say(format!("A trough {:.1} m deep was cut at {} to give a pit in the band a way out.", trough.depth_m, place(trough.at)));
        }
    }
    if pits_left > 0 {
        let pits = if pits_left == 1 { "1 pit was".to_string() } else { format!("{pits_left} pits were") };
        say(format!("{pits} left in the band when the troughs ran out."));
    }
    let course = |river: Option<usize>| river.map_or_else(|| "A brook".to_string(), |drawn| format!("River {}", drawn + 1));
    let about = |river: Option<usize>| river.map_or(About::Land, About::River);
    for (&at, &river) in water.brooks_narrowed.iter().zip(&water.narrowed_rivers) {
        notes.push((format!("{} was narrowed to nothing at the outline at {}.", course(river), place(at)), about(river)));
    }
    for (&at, &river) in water.brooks_ended.iter().zip(&water.ended_rivers) {
        notes.push((format!("{} was cut off at the outline at {}.", course(river), place(at)), about(river)));
    }
    for join in water.joins.iter().filter(|join| join.rise_before_m > RISE_LIMIT_M) {
        let at = join.rise_at.map_or_else(String::new, |at| format!(" at {}", place(at)));
        let note = format!("{} rose {:.2} m going downstream where it was joined across the outline{at}, and was mended so it no longer rises.", course(join.drawn), join.rise_before_m);
        notes.push((note, about(join.drawn)));
    }
    for (&at, &river) in water.ends_lost_at.iter().zip(&water.ends_lost_rivers) {
        notes.push((format!("{}'s end at {} met water the join left out, so it ends there now.", course(river), place(at)), about(river)));
    }
    let mut say = |note: String| notes.push((note, About::Land));
    tight.iter().cloned().for_each(&mut say);
    let (mut changed, mut held, mut by_hand) = (0usize, 0usize, 0usize);
    for cell in covered.world_cells() {
        changed += 1;
        if marks.held.get(cell) == Some(&true) {
            held += 1;
        } else if marks.by_hand.get(cell) == Some(&true) {
            by_hand += 1;
        }
    }
    for (count, how) in [(held, "is held"), (by_hand, "was shaped by hand")] {
        if count > 0 {
            let share_pct = 100.0 * count as f64 / changed as f64;
            let share = if share_pct < SHARE_SAID_LEAST_PCT { format!("Under {SHARE_SAID_LEAST_PCT} %") } else { format!("{share_pct:.1} %") };
            say(format!("{share} of the ground the edited drawing changes {how}, and was kept as it was."));
        }
    }
    notes
}

/// Whether the outline touches `about`, as `touched` says: the land always.
fn is_touched(touched: &Touched, about: About) -> bool {
    match about {
        About::Land => true,
        About::River(drawn) => touched.rivers.get(drawn).copied().unwrap_or(true),
        About::Lake(drawn) => touched.lakes.get(drawn).copied().unwrap_or(true),
    }
}

/// Whether the kept land's notes about `about` stand in the edited land.
fn is_kept(touched: &Touched, about: About) -> bool {
    match about {
        About::Land => false,
        About::River(drawn) => touched.kept_rivers.contains(&drawn),
        About::Lake(drawn) => touched.kept_lakes.contains(&drawn),
    }
}

/// `Edit::notes` and their subjects: `own`, with theirs, then the mixed world's notes about what the
/// outline touches.
fn edit_notes(own: Vec<(String, About)>, mixed: &World, touched: &Touched) -> (Vec<String>, Vec<About>) {
    let (mut notes, mut about): (Vec<String>, Vec<About>) = own.into_iter().unzip();
    for (note, &subject) in mixed.notes.iter().zip(&mixed.notes_about).filter(|(_, &subject)| is_touched(touched, subject)) {
        notes.push(note.clone());
        about.push(subject);
    }
    (notes, about)
}

/// The edited land's notes and their subjects (`Land::edit`, **The notes**): the mixed world's, in
/// its order, but for the kept rivers and lakes, whose notes are the kept land's, in their own order,
/// standing one for one in the mixed world's places for each (the fine steps write a river's notes
/// among others' by kind, so a land regrown as it was keeps its order); those left over follow its
/// last such place, and all of them follow the others when the mixed world says nothing of it.
fn land_notes(old: &World, mixed: &World, touched: &Touched) -> (Vec<String>, Vec<About>) {
    let old_notes = || old.notes.iter().cloned().zip(old.notes_about.iter().copied());
    // The kept land's notes about each kept subject the mixed world names, still to be placed.
    let mut waiting: Vec<(About, VecDeque<String>)> = Vec::new();
    for &subject in mixed.notes_about.iter().filter(|&&subject| is_kept(touched, subject)) {
        if !waiting.iter().any(|(about, _)| *about == subject) {
            waiting.push((subject, old_notes().filter(|&(_, about)| about == subject).map(|(note, _)| note).collect()));
        }
    }
    let last_place = |subject: About| mixed.notes_about.iter().rposition(|&about| about == subject);
    let mut notes: Vec<(String, About)> = Vec::new();
    for (place, (note, &subject)) in mixed.notes.iter().zip(&mixed.notes_about).enumerate() {
        if !is_kept(touched, subject) {
            notes.push((note.clone(), subject));
            continue;
        }
        let Some((_, kept)) = waiting.iter_mut().find(|(about, _)| *about == subject) else { continue };
        let taken = if last_place(subject) == Some(place) { kept.len() } else { kept.len().min(1) };
        notes.extend(kept.drain(..taken).map(|note| (note, subject)));
    }
    let named = |about: About| waiting.iter().any(|(subject, _)| *subject == about);
    notes.extend(old_notes().filter(|&(_, about)| is_kept(touched, about) && !named(about)));
    notes.into_iter().unzip()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Country;
    use crate::error::is_one_sentence;
    use crate::grow::{model_side, GROW_CELL_M, GROW_MARGIN_M};
    use crate::trial::{example_edit, standard_edit, trial_drawing};
    use std::sync::atomic::Ordering;

    /// The land model's grid of a land `size` metres across: its cells a side, the world `x` (and
    /// `z`) of its first cell, and its cells' size, in metres.
    fn model_grid(size: usize) -> (usize, f64, f64) {
        (model_side(size as u32), -(size as f64) / 2.0 - f64::from(GROW_MARGIN_M), f64::from(GROW_CELL_M))
    }

    /// The land the cancel's tests edit: the example at 512 m in hill country from seed 1, small so
    /// that each edit is quick, and its edit `a`, the brook's second point moved 30 m.
    fn small_land() -> (Land, Drawing) {
        let land = Land::grow(&Drawing::example_valley(512), 1, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
        let edited = example_edit(&land.drawing, "a").expect("an edit of the example");
        (land, edited)
    }

    /// Every step of an edit, in the order an edit takes them.
    const STEPS: [Step; 11] = [
        Step::OldStart,
        Step::NewStart,
        Step::Footprint,
        Step::Grow,
        Step::OldModel,
        Step::Outline,
        Step::Mix,
        Step::FineSteps,
        Step::JoinGround,
        Step::JoinWater,
        Step::WaterBesideFixed,
    ];

    /// The cancel flag is read after every step, the join's two halves among them (Review Focus 4,
    /// the join as well as the growing and the fine steps), so a flag set by a hook between two steps,
    /// which needs no clock, ends the edit `Cancelled` before the next begins; set before the call it
    /// ends it before the first; and the land given is untouched each time.
    #[test]
    fn a_cancel_set_between_any_two_steps_ends_the_edit_there() {
        let (land, edited) = small_land();
        let before = land.clone();
        let mut seen = Vec::new();
        let done = land.change(&edited, land.seed, None, &edited, &AtomicBool::new(false), &mut |step| seen.push(step));
        assert!(done.is_ok(), "{:?}", done.err());
        assert_eq!(seen, STEPS, "the steps an edit takes");
        assert!(!land.marks.held.contains(&true), "the test of the water beside fixed ground has nothing to read, and its step is still told");

        let set = AtomicBool::new(true);
        let mut seen = Vec::new();
        assert_eq!(land.change(&edited, land.seed, None, &edited, &set, &mut |step| seen.push(step)).err(), Some(LandformError::Cancelled));
        assert!(seen.is_empty(), "set before the call: no step runs, {seen:?}");

        for (at, &cancelled_after) in STEPS.iter().enumerate() {
            let cancel = AtomicBool::new(false);
            let mut seen = Vec::new();
            let result = land.change(&edited, land.seed, None, &edited, &cancel, &mut |step| {
                seen.push(step);
                if step == cancelled_after {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            assert_eq!(result.err(), Some(LandformError::Cancelled), "the flag set after {cancelled_after:?}");
            assert_eq!(seen, STEPS[..=at], "the flag set after {cancelled_after:?}: no later step runs");
            assert_eq!(land, before, "the flag set after {cancelled_after:?}: the land");
        }
    }

    /// The edits the test of a backwater stopped by held ground makes on the example at 2,048 m in
    /// hill country from seed 11 (the final review of stage 2's rulings R20 and R23, M1 and S1):
    /// its lake held by a disc of 10 m at its centre, which holds the lake's water whole, and a lake
    /// of 40 m added on the main river 550 m below it; and a disc of 30 m held on the main river at
    /// 0.24 of its length, and the lake added at 0.35.
    const HELD_LAKE_M: f64 = 10.0;
    const BELOW_THE_HELD_LAKE: Lake = Lake { x_m: 427.0, z_m: 46.0, radius_m: 40.0 };
    const HELD_ON_THE_RIVER: ((f64, f64), f64) = ((-467.0, -525.0), 30.0);
    const BELOW_THE_HELD_RIVER: Lake = Lake { x_m: -311.0, z_m: -324.0, radius_m: 40.0 };

    /// Stage 2's ruling R24: a backwater that held ground stops is refused, naming the lake that
    /// holds it up, never taken with water nothing holds; and the edit runs another round only
    /// where the backwater's cells change the outline. Before it, the lake below the held lake took
    /// two rounds more, the third run giving what the second gave, about 2.9 s, then was refused as
    /// "River 1's new line" (in gentle country from seed 42, taken with 20 new unheld cells
    /// inside); the lake below the disc held on the river was refused as "River 1's new line" too.
    /// Now the first runs its outline, mix, fine steps and join twice and is refused with the
    /// lake's sentence, and the second, whose backwater reaches the held disc in the first round,
    /// runs them once and is refused the same; the land given is as it was.
    #[test]
    fn a_backwater_that_held_ground_stops_is_refused_by_its_lake() {
        let land = Land::grow(&Drawing::example_valley(2048), 11, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
        let size = 2048usize;
        let disc = |(x, z): (f64, f64), radius_m: f64| -> Vec<bool> { (0..size * size).map(|cell| distance(world_place(size, cell), (x, z)) <= radius_m).collect() };
        let kept = land.drawing.lakes[0];
        let cases = [(((kept.x_m, kept.z_m), HELD_LAKE_M), BELOW_THE_HELD_LAKE, 2), (HELD_ON_THE_RIVER, BELOW_THE_HELD_RIVER, 1)];
        for ((at, radius_m), lake, runs) in cases {
            let mut held = land.clone();
            assert!(held.hold(&disc(at, radius_m), true) > 0, "a hold that holds something");
            let before = held.clone();
            let mut edited = land.drawing.clone();
            edited.lakes.push(lake);
            let mut seen = Vec::new();
            let result = held.change(&edited, held.seed, None, &edited, &AtomicBool::new(false), &mut |step| seen.push(step));
            let Err(LandformError::Refused(sentence)) = result else { panic!("held at {at:?}: refused, not {result:?}") };
            assert!(sentence.starts_with("Lake 2 holds river 1 up to held ground at (") && sentence.ends_with("; the water there would not be held: move the lake down the river, or lift the hold there."), "{sentence}");
            assert!(is_one_sentence(&sentence), "{sentence}");
            let rounds = seen.iter().filter(|&&step| step == Step::JoinWater).count();
            assert_eq!(rounds, runs, "held at {at:?}: how many times the outline, mix, fine steps and join run: {seen:?}");
            assert_eq!(held, before, "held at {at:?}: the land given");
        }
    }

    /// The distances to a line are those of a search over every cell, within the reach and only
    /// there, for a line of many points, of one point, and one running off the grid.
    #[test]
    fn a_lines_distances_are_the_nearest_cells_within_the_reach() {
        let size = 512;
        let (side, origin, cell_m) = model_grid(size);
        let lines: [Vec<(f64, f64)>; 3] = [vec![(-100.0, -40.0), (20.0, 35.5), (90.0, 60.0), (91.0, 200.0)], vec![(10.5, -3.0)], vec![(200.0, 200.0), (700.0, 650.0)]];
        for line in &lines {
            let near = distances_to_line(line, size as u32, TRENCH_REACH_M);
            assert_eq!(near.len(), side * side);
            for (cell, &found) in near.iter().enumerate() {
                let at = (origin + (cell % side) as f64 * cell_m, origin + (cell / side) as f64 * cell_m);
                let searched = if line.len() == 1 { distance(at, line[0]) } else { line.windows(2).map(|pair| to_segment(at, pair[0], pair[1])).fold(f64::INFINITY, f64::min) };
                if searched <= TRENCH_REACH_M {
                    assert_eq!(found, searched, "cell {cell} of {line:?}");
                } else {
                    assert_eq!(found, f64::INFINITY, "cell {cell} of {line:?}");
                }
            }
        }
    }

    /// Every kind of refusal, a lake's or a river's as number 2, for the tests of its sentences.
    const REFUSALS: [Refusal; 8] = [
        Refusal::LakeFixed(2),
        Refusal::NewLake(2),
        Refusal::LakeNotWhole(2),
        Refusal::River(2),
        Refusal::RemovedRiver(2),
        Refusal::LakeWater(2),
        Refusal::Painted,
        Refusal::Regrown,
    ];

    /// Every sentence a refusal can be (`refusal_sentence`; the controller's ruling N10): each kind
    /// of refusal, beside held and beside hand-shaped ground, over it, within the band and beyond it,
    /// is one sentence by `LandformError`'s own check, starts with a capital, names the place, and
    /// says the distance as the band reads it.
    #[test]
    fn every_refusal_sentence_is_one_sentence() {
        const AT: (f64, f64) = (-4.6, 425.5);
        let mut forms = BTreeSet::new();
        for refusal in REFUSALS {
            for by_hand in [false, true] {
                for (apart_m, said) in [(0.3, "over "), (67.6, "within 150 m of "), (149.4, "within 150 m of "), (149.6, "150 m from "), (212.0, "212 m from ")] {
                    let sentence = refusal_sentence(refusal, by_hand, AT, apart_m);
                    assert!(is_one_sentence(&sentence) && sentence.starts_with(char::is_uppercase), "{sentence}");
                    assert!(sentence.contains("(-5, 426)"), "{sentence}");
                    assert_eq!(sentence.contains(BY_HAND), by_hand, "{sentence}");
                    if refusal == Refusal::LakeFixed(2) {
                        assert!(sentence.starts_with("Lake 2 "), "{sentence}");
                    } else {
                        assert!(sentence.contains(&format!("{said}{}", if by_hand { BY_HAND } else { HELD })), "{sentence}");
                        assert!(sentence.ends_with(if by_hand { "or regrow that ground as drawn there." } else { "or lift the hold there." }), "{sentence}");
                    }
                    if apart_m == 67.6 && refusal != Refusal::LakeFixed(2) {
                        assert!(sentence.contains("at (-5, 426), 68 m from it; "), "{sentence}");
                    }
                    forms.insert(sentence);
                }
            }
        }
        // A lake with fixed water has one form beside each kind of ground; every other refusal five.
        let water = forms.iter().filter(|sentence| sentence.contains("; the water there would not be held: ")).count();
        assert_eq!((forms.len(), water), (2 + 7 * 2 * 5, 5 * 2 * 5), "the forms: {forms:#?}");
        let river = refusal_sentence(Refusal::River(2), false, AT, 124.3);
        assert_eq!(river, "River 2's new line runs within 150 m of held ground at (-5, 426), 124 m from it; the water there would not be held: move the line, or lift the hold there.");
        // A river the edit removes (the controller's ruling R24), in its three forms.
        let removed = |apart_m: f64, by_hand: bool| refusal_sentence(Refusal::RemovedRiver(2), by_hand, AT, apart_m);
        assert_eq!(
            removed(124.3, false),
            "River 2, which this edit removes, ran within 150 m of held ground at (-5, 426), 124 m from it; the water there would not be held: keep the river, or lift the hold there."
        );
        assert_eq!(removed(212.0, false), "River 2, which this edit removes, ran 212 m from held ground at (-5, 426); the water there would not be held: keep the river, or lift the hold there.");
        assert_eq!(
            removed(0.3, true),
            "River 2, which this edit removes, ran over hand-shaped ground at (-5, 426); the water there would not be held: keep the river, or regrow that ground as drawn there."
        );
        let painted = refusal_sentence(Refusal::Painted, false, AT, 0.0);
        assert_eq!(painted, "The painted ground runs over held ground at (-5, 426); the water there would not be held: paint farther from it, or lift the hold there.");
    }

    /// A piece of fixed ground is joined across sides and corners and followed once: cells meeting
    /// at a corner are one piece, a cell two away is another, held and hand-shaped cells join, and
    /// the piece's first cell, count and sums are its own.
    #[test]
    fn a_piece_of_fixed_ground_is_its_cells_joined_across_sides_and_corners() {
        const SIZE: usize = 6;
        let mut marks = Marks::none(SIZE);
        for cell in [7, 14, 20, 21, 17] {
            marks.held[cell] = true;
        }
        marks.by_hand[28] = true;
        let mut seen = vec![false; SIZE * SIZE];
        let piece = piece_from(&marks, SIZE, 21, &mut seen);
        // Cells 7 (1, 1), 14 (2, 2), 20 (2, 3), 21 (3, 3) and 28 (4, 4), corner to corner and side to side.
        assert_eq!(piece, Piece { first: 7, cells: 5, columns: 1 + 2 + 2 + 3 + 4, rows: 1 + 2 + 3 + 3 + 4 });
        assert_eq!((0..SIZE * SIZE).filter(|&cell| seen[cell]).collect::<Vec<_>>(), [7, 14, 20, 21, 28]);
        assert_eq!(piece_from(&marks, SIZE, 17, &mut seen), Piece { first: 17, cells: 1, columns: 5, rows: 2 }, "cell 17 (5, 2) lies two from 14 and 21");
    }

    /// The controller's ruling 1 pinned (its review's N1) on the trial `wide` at 1,024 m in hills
    /// from seed 3, the one trial whose lake level `model_water` of the kept `f32` heights gives a
    /// last bit lower than `grow`'s: the kept model an edit derives holds `grow`'s heights and lake
    /// levels (`first_lake_levels`), not `model_water`'s; the standard edit through the land is the
    /// edit worked out step by step from that kept model, its mixed model's lakes given the kept
    /// levels where the mix leaves them (`kept_lake_levels`, stage 2's ruling R11, N3, as the edit
    /// does); and the same steps from `grow`'s own model
    /// (the report's comparison by hand) give the same edit: outline, mixed model and edited world
    /// (heights, ground, every layer, the water) bit for bit. The kept model and `grow`'s differ only
    /// in what `model_water` works out again in `f32` from the heights and `grow` worked out in `f64`:
    /// the water's depth, on 157 cells by at most 3.3e-6 m; their heights, drained areas, receivers,
    /// rivers and notes are one.
    #[test]
    fn the_kept_model_holds_grows_levels_and_gives_grows_edit() {
        const SEED: u64 = 3;
        // The water's depth the kept model works out again in `f32` against `grow`'s, measured on
        // `wide` from seed 3: 157 cells, at most 3.3e-6 m apart.
        const DEPTH_CELLS_MOST: usize = 157;
        const DEPTH_APART_MOST_M: f32 = 4e-6;
        let config = GenConfig::default();
        let never = AtomicBool::new(false);
        let drawing = trial_drawing("wide", 1024, Country::hills()).expect("a trial");
        let land = Land::grow(&drawing, SEED, &config, &never).expect("the trial grows");
        let grown = grow_model(&drawing, &config.water, SEED, &never).expect("the trial grows");
        let (kept, _) = land.kept_model_and_routed().expect("the kept model");
        let levels = |lakes: &[GrownLake]| -> Vec<(usize, u32)> { lakes.iter().map(|lake| (lake.drawn, lake.level_m.to_bits())).collect() };
        let routed = model_water(&drawing, &config.water, SEED, &land.model_heights).expect("the kept heights routed");
        assert_ne!(levels(&routed.lakes), levels(&grown.lakes), "model_water gives wide's lake another last bit");
        assert_eq!(levels(&kept.lakes), levels(&grown.lakes), "the kept model holds grow's levels");
        let bits = |data: &[f32]| -> Vec<u32> { data.iter().map(|value| value.to_bits()).collect() };
        assert_eq!(bits(&kept.heights.data), bits(&grown.heights.data));
        assert_eq!(bits(&kept.drained_m2.data), bits(&grown.drained_m2.data));
        assert!(kept.receivers == grown.receivers && kept.rivers == grown.rivers, "the kept model's routing and rivers are grow's");
        assert!(kept.notes == grown.notes && kept.notes_about == grown.notes_about);
        let depth_cells = (kept.water_depth_m.data.iter().zip(&grown.water_depth_m.data)).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
        let worst_m = (kept.water_depth_m.data.iter().zip(&grown.water_depth_m.data)).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        println!("wide s3: the kept model's water depth differs from grow's on {depth_cells} cells, at most {worst_m:e} m");
        assert!(depth_cells <= DEPTH_CELLS_MOST && worst_m <= DEPTH_APART_MOST_M, "the water's depth differs on {depth_cells} cells, by up to {worst_m:e} m");

        let edited = standard_edit(&drawing).expect("the trial's edit");
        let edit = land.edit(&edited, &never).expect("the standard edit is taken");
        for (name, model) in [("the kept model", &kept), ("grow's model", &grown)] {
            let water = &config.water;
            let found = footprint(&start_of(&drawing, water, SEED).expect("a start"), &start_of(&edited, water, SEED).expect("a start"), &drawing, &edited);
            let new_model = grow_model(&edited, water, SEED, &never).expect("the edited trial grows");
            let zones = lake_zones(&land.world, model, &new_model, &drawing, &edited);
            let shape = outline(drawing.size_m, &found, &zones, &land.marks, land.reach_m);
            let mut mixed = mixed_model(model, &new_model, &edited, water, SEED, &shape.model_weight).expect("the mixed model");
            let routed = model_water(&drawing, water, SEED, &model.heights).expect("the model's heights routed");
            kept_lake_levels(&mut mixed.model.lakes, &routed.lakes, &model.lakes, &lake_partners(&drawing, &edited));
            let mixed_world = finish_drawn(SEED, &edited, &config, &mixed.model, Frame::Model, &never).expect("its world");
            let mut joined = join_ground(&land.world, &mixed_world, &shape.weight);
            let join = join_water(&mut joined, &land.world, &mixed_world, &shape.weight, &drawing, &edited, water);
            let after = &edit.land.world;
            assert_eq!((bits(&shape.weight.data), bits(&shape.model_weight)), (bits(&edit.outline.weight.data), bits(&edit.outline.model_weight)), "{name}: the outline");
            assert_eq!(bits(&mixed.model.heights.data), bits(&edit.land.model_heights.data), "{name}: the mixed model's heights");
            assert_eq!(levels(&mixed.model.lakes), levels(&edit.land.first_lake_levels), "{name}: the mixed model's lakes");
            assert_eq!(bits(&joined.heights.data), bits(&after.heights.data), "{name}: the heights");
            assert!(joined.ground == after.ground, "{name}: the ground map");
            assert!(joined.layers.keys().eq(after.layers.keys()), "{name}: the layers' names");
            for (layer, grid) in &joined.layers {
                assert_eq!(bits(&grid.data), bits(&after.layers[layer].data), "{name}: the {layer} layer");
            }
            assert_eq!(joined.water, after.water, "{name}: the water");
            assert_eq!(join, edit.water, "{name}: what joining the water did");
        }
    }

    /// The cell named is the one weighing least of those the marks lower (the first of several), and
    /// a cell the marks do not lower is never named.
    #[test]
    fn the_cell_named_weighs_least_of_those_fixed_ground_lowers() {
        let shape = [1.0, 0.4, 0.2, 0.2, 0.0, 0.5];
        let free = [1.0, 0.9, 0.9, 0.9, 0.0, 0.5];
        assert_eq!(lowered(0..6, &shape, &free), Some(2));
        assert_eq!(lowered([0, 4, 5].into_iter(), &shape, &free), None);
        assert_eq!(lowered([3, 2].into_iter(), &shape, &free), Some(3));
    }

    /// The edited land's notes: the mixed world's in its order, but a kept river's and a kept lake's
    /// are the kept land's, in their own order, standing one for one in the mixed world's places for
    /// each, those left over after its last place, or after the others when the mixed world says
    /// nothing of it; `Edit::notes` are the edit's own, about the land, then the mixed world's about
    /// what the outline touches and about the land. A kept river whose notes stand apart, another's
    /// between them, as the fine steps write `headjoin`'s (stage 2's ruling R8, M1), keeps
    /// their order where the mixed world has them so, and the mixed world's places beyond the kept
    /// notes' count are left empty.
    #[test]
    fn kept_notes_stand_in_the_mixed_worlds_place_and_the_edits_name_what_it_touches() {
        let (land, _) = small_land();
        let world = |notes: &[(&str, About)]| World {
            notes: notes.iter().map(|(note, _)| note.to_string()).collect(),
            notes_about: notes.iter().map(|&(_, about)| about).collect(),
            ..land.world.clone()
        };
        let old = world(&[("old river 1", About::River(0)), ("old lake 2", About::Lake(1)), ("old land", About::Land), ("old river 2 again", About::River(1)), ("old river 2", About::River(1))]);
        let mixed = world(&[("new land", About::Land), ("new river 2", About::River(1)), ("new river 1", About::River(0)), ("new lake 1", About::Lake(0))]);
        let touched = Touched { rivers: vec![true, false], lakes: vec![true, false], kept_rivers: BTreeSet::from([1]), kept_lakes: BTreeSet::from([1]) };
        let (notes, about) = land_notes(&old, &mixed, &touched);
        assert_eq!(notes, ["new land", "old river 2 again", "old river 2", "new river 1", "new lake 1", "old lake 2"]);
        assert_eq!(about, [About::Land, About::River(1), About::River(1), About::River(0), About::Lake(0), About::Lake(1)]);
        let (notes, about) = edit_notes(vec![("own".to_string(), About::Land)], &mixed, &touched);
        assert_eq!(notes, ["own", "new land", "new river 1", "new lake 1"]);
        assert_eq!(about, [About::Land, About::Land, About::River(0), About::Lake(0)]);

        let apart = [("river 1 ended", About::River(0)), ("river 1 leaves", About::River(0)), ("river 2 short", About::River(1)), ("river 1 cut", About::River(0))];
        let both_kept = Touched { rivers: vec![false, false], lakes: Vec::new(), kept_rivers: BTreeSet::from([0, 1]), kept_lakes: BTreeSet::new() };
        let (notes, about) = land_notes(&world(&apart), &world(&apart), &both_kept);
        assert_eq!(notes, apart.map(|(note, _)| note), "the kept land's order, the notes about river 1 apart");
        assert_eq!(about, apart.map(|(_, about)| about));
        let fewer = world(&[("new river 1", About::River(0)), ("new river 2", About::River(1)), ("new river 1 again", About::River(0)), ("new river 1 a third", About::River(0)), ("new river 1 a fourth", About::River(0))]);
        let (notes, _) = land_notes(&world(&apart), &fewer, &both_kept);
        assert_eq!(notes, ["river 1 ended", "river 2 short", "river 1 leaves", "river 1 cut"], "one for one in the mixed world's places, the last ones empty");
        let one_place = world(&[("new river 2", About::River(1)), ("new river 1", About::River(0))]);
        let (notes, _) = land_notes(&world(&apart), &one_place, &both_kept);
        assert_eq!(notes, ["river 2 short", "river 1 ended", "river 1 leaves", "river 1 cut"], "those left over after the last place");
    }

    /// A `CourseJoin` of a course of `drawn` that rose `rise_m` before it was mended, at `at`.
    fn rose(drawn: Option<usize>, rise_m: f32, at: (f64, f64)) -> water::CourseJoin {
        water::CourseJoin {
            course: 0,
            old: 0,
            new: 0,
            drawn,
            rise_before_m: rise_m,
            rise_m: 0.0,
            rise_at: Some(at),
            lowered_most_m: rise_m,
            raised_most_m: 0.0,
            mended_at_weight_1: 0,
            width_step_m: 0.0,
            width_step_own_m: 0.0,
            gap_m: 0.0,
        }
    }

    /// The edit's own notes, in their order and with their subjects: a footprint that is the whole
    /// land says why first (the whole-branch review's S2); a pit filled by more than `TROUGH_SAID_M`
    /// is said with its place and the most it raised a cell, as a trough cut so deep is, in the
    /// order done, and a shallower one is not; a course narrowed to nothing or cut off at the
    /// outline is said at its place, a joined course that rose more than `RISE_LIMIT_M` before it
    /// was mended at its largest rise, and an end the join left out at the course's last point,
    /// each about its drawn river where it is one (the controller's ruling R23) and else about the
    /// land; each note is one sentence.
    #[test]
    fn an_edits_own_notes_say_the_whole_land_deep_fills_rises_and_lost_ends() {
        const SIZE_M: u32 = 64;
        let troughs = [
            Trough { at: (-584.4, -520.0), depth_m: 14.18, filled: false },
            Trough { at: (10.0, 20.0), depth_m: 6.34, filled: true },
            Trough { at: (0.0, 0.0), depth_m: 0.35, filled: true },
            Trough { at: (5.0, 5.0), depth_m: 4.9, filled: false },
        ];
        let joins = vec![rose(Some(1), 0.123, (-36.0, 4.5)), rose(None, 0.25, (100.2, -7.0)), rose(Some(0), RISE_LIMIT_M, (0.0, 0.0))];
        let water = WaterJoin {
            joins,
            ends_lost: 2,
            ends_lost_at: vec![(1.0, 2.0), (3.4, -5.6)],
            ends_lost_rivers: vec![Some(1), None],
            brooks_narrowed: vec![(187.2, -308.0), (40.0, 60.4)],
            narrowed_rivers: vec![None, Some(2)],
            brooks_ended: vec![(495.0, -53.0), (-7.0, 8.0)],
            ended_rivers: vec![Some(0), None],
            ..WaterJoin::default()
        };
        let side = model_side(SIZE_M);
        let covered = Covered::new(SIZE_M, &outline::Footprint { model_cells: vec![false; side * side], lake_discs: Vec::new(), whole: None });
        let marks = Marks::none(SIZE_M as usize);
        let said = |whole: Option<&Whole>| own_notes(whole, &troughs, 0, &water, &[], &covered, &marks);
        let expected = [
            ("A trough 14.2 m deep was cut at (-584, -520) to give a pit in the band a way out.", About::Land),
            ("A pit in the band at (10, 20) was filled, the ground raised by up to 6.3 m, as no way out could be cut for it.", About::Land),
            ("A brook was narrowed to nothing at the outline at (187, -308).", About::Land),
            ("River 3 was narrowed to nothing at the outline at (40, 60).", About::River(2)),
            ("River 1 was cut off at the outline at (495, -53).", About::River(0)),
            ("A brook was cut off at the outline at (-7, 8).", About::Land),
            ("River 2 rose 0.12 m going downstream where it was joined across the outline at (-36, 5), and was mended so it no longer rises.", About::River(1)),
            ("A brook rose 0.25 m going downstream where it was joined across the outline at (100, -7), and was mended so it no longer rises.", About::Land),
            ("River 2's end at (1, 2) met water the join left out, so it ends there now.", About::River(1)),
            ("A brook's end at (3, -6) met water the join left out, so it ends there now.", About::Land),
        ];
        let notes = said(None);
        assert_eq!(notes, expected.iter().map(|&(note, about)| (note.to_string(), about)).collect::<Vec<_>>());
        for (whole, why) in [(Whole::Seed, "the seed changed"), (Whole::Kind, "the kind of country changed"), (Whole::Numbers(vec!["relief_m"]), "a number of the country changed: relief_m")] {
            let notes = said(Some(&whole));
            assert_eq!(notes[0], (format!("This edit regrows the whole land but held and hand-shaped ground: {why}."), About::Land));
            assert_eq!(notes.len(), expected.len() + 1);
        }
        assert!(said(Some(&Whole::Numbers(vec!["relief_m", "rounds"]))).iter().all(|(note, _)| is_one_sentence(note)));
    }
}
