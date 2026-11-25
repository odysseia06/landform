//! The beds at 1 m: the water's surface along each channel, falling only downstream and held below
//! the foot of its banks, and the bed's depth under its deep line (`profiles`); and the beds, bowls
//! between their waterlines, and their banks cut into the 1 m ground, and the water each holds,
//! filled out from its bed (`cut_beds`).

use super::channel::{Channel, ChannelEnd, ChannelPoint};
use super::lake::{segments_beside, LakeShape};
use super::shape::{ends_first, Shape};
use super::smooth::distance_m;
use super::spread::Spread;
use super::{fade, lake_level_at, lake_of_cell};
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{About, Note};
use crate::water_rules::WaterRules;
use crate::world::check_cancel;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::AtomicBool;

/// A bank reaches at most this far beyond the edge of its bed, in metres; its cut fades to nothing
/// there (`fade`).
pub(crate) const BANK_REACH_M: f64 = 12.0;
/// A channel whose surface would reach the point where it enters the water it ends on above the
/// surface there comes down to meet it falling this much more steeply than it would, in metres per
/// metre along it, over the stretch before; and a channel that flows out of a lake comes down from
/// the lake's level, past its stretch beside the lake, falling no faster than this where its banks
/// hold it.
pub(crate) const JUNCTION_GRADE: f64 = 0.05;
/// A channel that begins on dry land grows from no depth at its first point to its full depth this
/// far along it, in metres.
const HEAD_GROWTH_M: f64 = 12.0;
/// A channel whose first point lies within this distance of the world's border, in metres, came in
/// across it rather than beginning on dry land.
pub(crate) const BORDER_START_M: f64 = 8.0;
/// On the 1 m grid a bed's waterlines lie at least this far either side of its line, in metres,
/// however narrow its channel: half a cell's diagonal. A line through any place, at any angle,
/// passes nearer than that to the middle of every cell it runs through, but for a cell whose corner
/// alone it touches, so each of those cells holds water and a channel narrower than a cell shows
/// unbroken; half a cell would leave both rows dry under a line exactly between them.
pub(crate) const HALF_WIDTH_MIN_M: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// A bed's cross-section is a bowl: across it the water's depth is its depth at the deep line
/// times `1 - u` to this power, `u` running from 0 at the deep line to 1 at either waterline
/// (`bowl_share`). A constant, not a rule, until tuning shows a person would want to turn it.
const CROSS_SECTION_POWER: f64 = 3.0;
/// A channel whose surface stands more than this above its floor anywhere (see `profiles`), in
/// metres, is said in a note (`above_floor_notes`).
const ABOVE_FLOOR_SAID_M: f32 = 0.5;
/// What single precision may leave of a height that should be exactly another, in metres: a bank's
/// foot that falls no more than this below its line (`bank_falls`, the chord in double precision
/// against a cell's height in single precision, about 1.5e-6 m on level ground) falls by rounding
/// alone, and does not hold the water.
const ROUNDING_M: f32 = 1e-4;
/// The foot of a channel's bank is read on the cells no more than this beyond the waterline
/// (`bank_falls`), in metres: more than a cell, so that no path from cell to cell across their
/// sides, from the bed out, steps over it.
pub(crate) const BANK_FOOT_M: f64 = 1.5;
/// A channel's wished surface lies at least this far below the foot of its banks: below the
/// line's own ground less the most the foot falls below it (`profiles`), in metres.
pub(crate) const FREEBOARD_M: f32 = 0.1;
/// A lake's rim is held out to this far from its water and from its outflows' beds held beside it,
/// in metres (`hold_rims`): the foot of a bank and a bank's whole reach beyond it, over which the
/// hold falls as a bank does, `BANK_REACH_M` over `bank_run` in all; past it no rim is held.
const RIM_REACH_M: f64 = BANK_FOOT_M + BANK_REACH_M;
/// A tributary whose banks let it arrive more than this below the surface of the channel it ends
/// on, at its entry point, falls short there, and that channel comes down to it (`profiles`), in
/// metres.
pub(crate) const ARRIVAL_ALLOWANCE_M: f32 = 0.3;
/// A channel comes down by no more than this at a junction, below its surface at the tributary's
/// entry point when no junction brings it down (`profiles`), in metres.
const COME_DOWN_MOST_M: f32 = 3.0;
/// The channels' water is worked out no more than this many times over, each pass under the caps
/// the junctions of the passes before ask for (`profiles`).
pub(crate) const PASSES_MOST: usize = 3;
/// A channel a junction brings down by more than this, and a tributary left standing more than
/// this above its banks where it joins, in metres, is said in a note (`junction_notes`).
const DEEPER_NOTE_M: f32 = 1.0;

/// The floor under a point of a channel, in the heights' frame, in metres: the land the model
/// grew, with its lakes filled, read at the point (`water::floor_m`).
pub(crate) type Floor<'a> = dyn Fn(&ChannelPoint) -> f32 + 'a;

/// The level of the lake whose water a cell of the heights holds, by the cell's index, in the
/// heights' frame, in metres; `None` for a cell that holds no lake's water. `bank_falls` reads the
/// water of the earlier channels' beds so too.
type LakeAt<'a> = dyn Fn(usize) -> Option<f32> + 'a;

/// The water along a channel, one value per point of the channel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Profile {
    /// The water's surface, in the heights' frame, in metres; it never rises downstream.
    pub surface_m: Vec<f32>,
    /// The bed's depth under the deep line, after the wander, in metres (see `profiles`); the
    /// water there is `water_fill` of it deep.
    pub depth_m: Vec<f32>,
    /// How far its surface stands at most above its floor, in metres; 0 when it stands above it
    /// nowhere.
    pub above_floor_m: f32,
    /// Whether the channel begins on dry land (see `profiles`): it grows into its bed from its
    /// first point, and none of its water lies straight beyond that point (`cut_beds`).
    pub on_dry_land: bool,
}

impl Profile {
    /// The bed under the deep line at point `k`: `water_fill` of the bed's depth below the surface.
    fn bed_m(&self, k: usize, rules: &WaterRules) -> f32 {
        self.surface_m[k] - rules.water_fill * self.depth_m[k]
    }
}

/// The water along each of `channels`, shaped by `shapes` (one per channel, `shape_channels`), over
/// `heights`, a grid of 1 m cells whose cell `(i, j)` lies at world `(i - width / 2, j - height /
/// 2)`, with `floor` the floor under each point (`Floor`: the land the model grew, its lakes filled;
/// the valley's floor in a valley and the lake's level in a lake, where the 1 m ground may hold a
/// gully the fine shapes cut or a lake's bed), and `lakes` the lakes found (`fill_lakes`): the cells
/// that hold each one's water, at its level, and its held bed, the beds of the channels out of it
/// along their stretch beside it. `lake_levels[k]` is drawn lake k's level in `heights`' frame, or
/// NaN when it holds no water; a channel's end at a lake that holds no water (or whose level is
/// missing or not finite) is taken as no lake, and so is the lake a channel flows out of.
///
/// The wished surface is worked out from each point's width before the wander (its width over its
/// factor). At each point the depth that width gives is `bed_depth_share` of it, kept from
/// `bed_depth_min_m` to `bed_depth_max_m`. A channel that begins on dry land (out of no lake that
/// holds water, its first point more than `BORDER_START_M` inside the world's border, at `±width /
/// 2` and `±height / 2`) grows into that depth: it is that share of it which the distance along the
/// channel is of `HEAD_GROWTH_M`, from none at its first point. So along a channel whose width
/// before the wander never falls, that depth never falls. The wished surface is the ground there
/// (bilinear, the nearest edge's beyond the grid) less `1 - water_fill` of that depth.
///
/// A channel reads the ground as the channels worked out before it have cut it: each of their
/// beds and banks is cut, as `cut_beds` cuts it, into a copy of `heights` as soon as that channel's
/// water is worked out, each lowering worked out from `heights` as they were and the lowest kept.
/// So a tributary, worked out after the channel it ends on, follows that channel's cut bank down
/// at its mouth. `heights` themselves do not change. Before any channel is worked out each lake's
/// rim is held, falling as a bank does (`hold_rims`): a cell that holds no lake's water and lies in
/// no lake's held bed, `d` metres from a lake's water or its held bed (as `Spread` gives the
/// distance, out to `RIM_REACH_M`), is held at the lake's level plus `FREEBOARD_M` less
/// `max(0, d - BANK_FOOT_M) / bank_run`. Once a channel's water is worked out, each cell of the
/// foot of its banks (below) is held at its surface at the cell's nearest point plus
/// `FREEBOARD_M`, and for a channel that begins on dry land so is each cell as far beyond its
/// waterline straight behind its first point (`hold_foot`); a cell held several times is held at
/// the highest. The bank of a channel worked out later lowers no held cell below its hold, nor
/// below its own height where that lies lower; its bed is cut as freely as before, so a tributary's
/// bed still reaches its river through the river's foot (`for_each_lowering`). Nothing is raised.
///
/// Both banks hold the wished surface, measured against the line's own ground. A cell that lies
/// beyond the waterline by no more than `BANK_FOOT_M` at its nearest point of the line (the water's
/// half width there after the wander, no less than `HALF_WIDTH_MIN_M`) falls below the line by the
/// chord there, the straight line between the line's ground at the two points of its stretch at the
/// cell's share along it, less its own ground; a cell that holds a lake's water counts as ground at
/// that lake's level, and one that holds the water of the bed of a channel worked out before it,
/// but for the one it ends on, as ground at that water's surface (`KeptBeds`): the cells `cut_beds`
/// seeds that channel's water in, inside its bed (no farther from its line than its half width, no
/// less than `HALF_WIDTH_MIN_M`, at their nearest point) and below its surface there, none inside
/// what it ends on and none straight beyond the first point of one that begins on dry land, each
/// at the highest such surface. The water beside it, not the floor under that water, is what the
/// foot holds it against; a cell no bed's water lies on, or one standing at or above that water,
/// is ground at its own height. A cell straight beyond the line's first or last point, whose
/// nearest point is that end and which lies beyond it along the line, is no part of the foot.
/// Where the cells whose
/// nearest point lies on a stretch one of whose ends is the point fall below the line
/// (`bank_falls`), the wished surface is also no higher than the ground at the point less the most
/// they fall less `FREEBOARD_M`. As the surface between two points is a straight line too, wherever
/// no later step raises it, it lies at least `FREEBOARD_M` below every cell of the foot at the
/// cell's nearest point, to rounding, on the ground the channel reads. So where the land falls
/// away on one side the surface is lowered and the bed cut deeper until the bank holds, while a
/// channel that falls along its own line is not held by it; and as the waterline is the wandered
/// one, the wander moves a surface here as well as where a channel meets the one it ends on. On
/// the final ground the freeboard holds only against the banks of the channels worked out later,
/// as the hold keeps them from cutting the foot lower, but where a tributary's bed is cut through
/// the foot at its mouth: the channel's own bank, which `cut_beds` cuts as a bank of `bank_run`
/// starting at the surface, cuts its foot down towards the surface, to the surface itself right
/// at the waterline. The cap holds a dry-land head too, whose depth still grows from nothing.
/// Inside the bed of the channel a channel ends on it does not show, as the surface there is
/// that one's (below). The steps after it, the running minimum, the lake's hold and the meeting
/// with what a channel ends on, are as they would be without it, so a surface they raise may
/// stand above a bank's foot: water backing up, which `cut_beds` lets spread beside the bed and
/// the ground then bounds (`water::bound_spread`).
///
/// The bed's depth under the deep line (`Profile::depth_m`) is the depth the width before the
/// wander gives, divided by the factor and kept from `bed_depth_min_m` to `bed_depth_max_m`, then
/// grown over a dry-land head as above: under the same surface, a stretch the wander widens is
/// shallower and one it narrows deeper. With no wander every factor is 1 and the bed's depth is that
/// depth, so the bed never rises where the surface does not.
///
/// Along a channel the surface is the lowest wished surface from its first point to that one, and
/// no higher than the level of the lake it flows out of. Such a channel holds the lake's level for
/// as long as the lake lies beside its bed, and then falls as its banks let it. Its stretch beside
/// the lake is its leading segments with a cell of the lake's water within their half width and
/// `BESIDE_BED_M`, the test the lake settled with (`segments_beside`, on the lake's cells), and
/// `left_m` the distance along it to the far end of the last of them; 0 when there is none. Up to
/// there, a point no farther along than `left_m` when it is above 0, its surface is the level
/// outright. Beyond, each surface is the lowest of the level, what its banks hold there, and the
/// higher of its own surface and the level less `JUNCTION_GRADE` times its distance past `left_m`;
/// then a running minimum keeps them from rising downstream. What its banks hold at a point
/// (`banks_hold`) is read on the cells `bank_falls` reads, on the ground as the channels before
/// have cut it, but only those whose nearest place along the line lies at or past `left_m`: the
/// line's ground there less the most any of them falls below the chord (the true fall, below 0
/// where the whole foot stands above the line, not clipped at 0 as `bank_falls` clips it) less
/// `FREEBOARD_M`; a cell that holds a lake's water holds that lake's level outright; and no bound
/// where there is no such cell. So past the stretch the water falls no faster than the grade where
/// its banks hold it, and as fast as they require where they do not: below a lake on a slope it
/// falls with the land, 0.1 m below the foot of its banks.
///
/// A channel is worked out after the one it ends on, whatever their places in the list, and
/// inside that one's bed, as it wanders, the water is that one's.
/// Its entry point is its first point from which every later point lies inside the bed of the
/// channel it ends on (within half that channel's width after the wander, no less than
/// `HALF_WIDTH_MIN_M`, at that channel's point nearest it, of that channel's line), its last point
/// at least; from the entry point on, each of its surfaces is that channel's surface at that
/// channel's point nearest it, kept from rising downstream by a running minimum. A channel that
/// ends in a lake enters it at its last point, at the lake's level.
/// Upstream of the entry point, with `e` the surface the entry point got, each surface is brought
/// to no more than `e`, plus `JUNCTION_GRADE` times its distance along the channel to the entry
/// point, plus how far it lies above its own surface at the entry point (its banks holding that as
/// they hold every other); and to no less than `e`.
/// So a channel that would reach its entry point above `e` would come down to meet it, falling
/// `JUNCTION_GRADE` more steeply than it would there, until the step is gone; and one that would
/// reach it below `e` is raised to it, so the water never climbs to join. As a tributary reads the
/// ground its river has cut, it follows the river's cut bank down to the river's surface on its
/// own and reaches its entry point at or below `e`, so at a river the grade seldom binds; it binds
/// at a lake's end, where a channel that would reach the lake above its level comes down to it at
/// that grade. Each step keeps the surfaces from rising downstream. The end is met after the
/// lake's hold, so where the lake a channel flows out of lies below what it ends on, or the
/// channel is too short to come down from its lake to what it ends on, the end wins. How far each
/// channel's surface stands at most above its floor is kept with its water
/// (`Profile::above_floor_m`), for the notes (`above_floor_notes`).
///
/// A tributary is raised to `e` only as far as its banks hold it; where they hold it lower, the
/// channel it ends on comes down to it, over passes. What a tributary's banks hold beside a point
/// before its entry point is read on the ground as the channels worked out before it have cut it:
/// of the cells `bank_falls` reads beside the point, those neither inside the bed of the channel
/// it ends on nor in that channel's own bank (no more than `BANK_REACH_M` beyond its waterline, at
/// its line's nearest point, and standing at or above its surface there), a cell that holds a
/// lake's water ground at that lake's level; the lowest of them less `FREEBOARD_M`, and `e` where
/// that is at least `e` less `FREEBOARD_M` or no such cell lies by. Its arrival is the lowest, over
/// its points before its entry point, of the higher of its surface there before it was raised to
/// `e` and the lower of `e` and that hold; it falls short where that lies more than
/// `ARRIVAL_ALLOWANCE_M` below `e`. After a pass the caps are worked out, each tributary before the
/// channel it ends on: its final arrival is the lower of its arrival and the lowest cap on it, and
/// one that falls short by that puts a cap on the channel it ends on (`Cap`), the highest of that
/// arrival, its `e` in the first pass less `COME_DOWN_MOST_M`, and the level of a lake that channel
/// ends in, at the earlier of that channel's point nearest its last point and the first point of
/// that channel's stretch nearest its entry point. The cap is new when it lies below `e` and below
/// the cap the tributary put before, which it replaces; while one is new, the water is worked out
/// again under the caps, in `PASSES_MOST` passes in all, the flag read before each but the first (a
/// set flag ends the work with `LandformError::Cancelled`). In a pass, after the running minimum and
/// the lake's hold, each surface of a capped channel is brought to no more than each cap's line: the
/// cap from its point on, and upstream of it the cap plus `JUNCTION_GRADE` times the distance along
/// to its point plus how far the surface lies above the surface at its point (both as they stood
/// before the caps); but no lower than the level of the lake it flows out of along its stretch
/// beside it; and a channel that begins on dry land is lowered upstream of a cap by no more than the
/// cap's lowering at its point times its distance along over that point's, so its head keeps its
/// ground. Then it meets what it ends on, as above. So a channel comes down to its tributary along
/// its own fall and the grade, never rising downstream; once a pass asks for no new cap the
/// tributary, raised to its new `e`, stands no higher than its banks hold it, to within
/// `ARRIVAL_ALLOWANCE_M`; where the 3 m or a lake stop the cap it still stands above its banks,
/// inside its bed; and where the `PASSES_MOST` passes run out with a cap still asked for (the
/// river's lowering moving the tributary's foot with it) it stands above them by what the last pass
/// left short. A cell of the channel's own bank
/// stands at or above its surface however far it comes down, so the hold reads alike in every pass.
/// The notes on the junctions are `junction_notes`'.
#[allow(clippy::too_many_arguments)]
pub(crate) fn profiles(
    channels: &[Channel],
    shapes: &[Shape],
    heights: &Grid<f32>,
    floor: &Floor,
    lakes: &[LakeShape],
    lake_levels: &[f32],
    rules: &WaterRules,
    cancel: &AtomicBool,
) -> Result<Worked, LandformError> {
    let WorkedOut { profiles, notes, passes, left_short, .. } = worked_out(channels, shapes, heights, floor, Reading::Cut(lakes), lake_levels, rules, cancel)?;
    Ok(Worked { profiles, notes, passes, left_short })
}

/// The channels' water as `profiles` works it out.
pub(crate) struct Worked {
    /// One per channel.
    pub profiles: Vec<Profile>,
    /// What the junctions did, past `DEEPER_NOTE_M` (`junction_notes`).
    pub notes: Vec<Note>,
    /// How many passes worked it out, from 1 to `PASSES_MOST`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub passes: usize,
    /// The tributaries the last pass left short (`WorkedOut::left_short`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub left_short: Vec<(usize, f32)>,
}

/// How `worked_out` has what each channel reads of its ground.
#[derive(Clone, Copy)]
enum Reading<'a> {
    /// Read off the ground as the channels worked out before have cut it (`GroundRead`), beside
    /// these lakes: the water each holds, at its level, and its outflows' beds held beside it.
    Cut(&'a [LakeShape]),
    /// Given, one per channel, with the caps on it: the tests hold a channel's ground fixed so, and
    /// the water is worked out once, under those caps, no junction asking for another.
    #[cfg_attr(not(test), allow(dead_code))]
    Given(&'a [GroundRead]),
}

/// The channels' water, and what each channel read on the way in the last pass, which the tests
/// read.
struct WorkedOut {
    profiles: Vec<Profile>,
    /// Each channel's wished surface as its banks held it (`Wished::surface_m`).
    #[cfg_attr(not(test), allow(dead_code))]
    wished_m: Vec<Vec<f32>>,
    /// What each channel read of its ground, and the caps on it.
    #[cfg_attr(not(test), allow(dead_code))]
    read: Vec<GroundRead>,
    /// The heights as every channel's bed and banks cut them: the copy the channels read, as the
    /// last one worked out left it.
    #[cfg_attr(not(test), allow(dead_code))]
    cut: Grid<f32>,
    /// The notes on the junctions (`junction_notes`).
    notes: Vec<Note>,
    /// How many passes worked the water out.
    passes: usize,
    /// Where the `PASSES_MOST` passes ran out with a cap still asked for, the tributaries whose cap
    /// the last pass asked for anew, each by its index and how far it stands above what its banks
    /// hold where it joins (its `e` less its arrival, `Arrival`), in metres: what the last pass
    /// left short. Empty when the last pass asked for no new cap.
    left_short: Vec<(usize, f32)>,
}

/// `profiles`, with what each channel reads of its ground had as `reading` says, what each channel
/// read in the last pass, and the ground as they cut it then.
#[allow(clippy::too_many_arguments)]
fn worked_out(
    channels: &[Channel],
    shapes: &[Shape],
    heights: &Grid<f32>,
    floor: &Floor,
    reading: Reading,
    lake_levels: &[f32],
    rules: &WaterRules,
    cancel: &AtomicBool,
) -> Result<WorkedOut, LandformError> {
    // The cap each tributary puts on the channel it ends on, by the tributary's index, and each
    // one's arrival in the first pass.
    let mut caps: Vec<Option<Cap>> = vec![None; channels.len()];
    let mut first: Option<Vec<Option<Arrival>>> = None;
    let mut passes = 0;
    loop {
        let pass = one_pass(channels, shapes, heights, floor, reading, lake_levels, rules, &caps);
        passes += 1;
        let first = first.get_or_insert_with(|| pass.arrivals.clone());
        let more = junction_caps(channels, &pass.arrivals, first, lake_levels, &caps);
        if more == caps || passes == PASSES_MOST {
            let notes = junction_notes(channels, first, &pass.arrivals, &caps);
            let asked_anew = (0..channels.len()).filter(|&index| more[index] != caps[index]);
            let left_short = asked_anew.filter_map(|index| pass.arrivals[index].map(|arrival| (index, arrival.entry_m - arrival.arrival_m))).collect();
            let Pass { profiles, wished_m, read, cut, .. } = pass;
            return Ok(WorkedOut { profiles, wished_m, read, cut, notes, passes, left_short });
        }
        check_cancel(cancel)?;
        caps = more;
    }
}

/// One pass of `profiles`: the channels' water, what each read, and how each tributary arrived at
/// the channel it ends on (none for a channel that ends on no other, or whose ground was given).
struct Pass {
    profiles: Vec<Profile>,
    wished_m: Vec<Vec<f32>>,
    read: Vec<GroundRead>,
    cut: Grid<f32>,
    arrivals: Vec<Option<Arrival>>,
}

/// One pass of `profiles`, each channel's ground had as `reading` says, under `caps`, the cap each
/// tributary puts on the channel it ends on, by the tributary's index (a given ground carries its
/// own caps instead).
#[allow(clippy::too_many_arguments)]
fn one_pass(
    channels: &[Channel],
    shapes: &[Shape],
    heights: &Grid<f32>,
    floor: &Floor,
    reading: Reading,
    lake_levels: &[f32],
    rules: &WaterRules,
    caps: &[Option<Cap>],
) -> Pass {
    debug_assert_eq!(channels.len(), shapes.len(), "one shape per channel");
    debug_assert_eq!(channels.len(), caps.len(), "one cap or none per channel");
    let level = |lake: usize| lake_levels.get(lake).copied().filter(|level| level.is_finite());
    let joined = |index: usize| match channels[index].end {
        ChannelEnd::Channel(joined) if joined < channels.len() => Some(joined),
        ChannelEnd::Channel(joined) => {
            debug_assert!(false, "channel {index} ends on channel {joined} of {}", channels.len());
            None
        }
        _ => None,
    };
    let size = (heights.width, heights.height);
    let mut made: Vec<Option<Profile>> = vec![None; channels.len()];
    let mut wished_m = vec![Vec::new(); channels.len()];
    let mut read = vec![GroundRead::default(); channels.len()];
    // The heights as the channels worked out so far have cut them, and what the lakes' rims and
    // the channels' feet hold.
    let mut cut = heights.clone();
    let mut held = vec![f32::NEG_INFINITY; heights.data.len()];
    if let Reading::Cut(lakes) = reading {
        hold_rims(size, lakes, rules, &mut held);
    }
    // The caps on each channel, in the order of the tributaries that put them.
    let mut caps_on: Vec<Vec<Cap>> = vec![Vec::new(); channels.len()];
    for (index, cap) in caps.iter().enumerate() {
        if let (Some(cap), Some(joined)) = (cap, joined(index)) {
            caps_on[joined].push(*cap);
        }
    }
    let mut arrivals: Vec<Option<Arrival>> = vec![None; channels.len()];
    // The water the beds of the channels worked out so far hold, and the cells inside each bed
    // another channel ends on, found once.
    let mut kept = KeptBeds::default();
    let mut beds_of: Vec<Option<Vec<usize>>> = vec![None; channels.len()];
    for index in ends_first(channels) {
        let channel = &channels[index];
        let end = match (channel.end, joined(index)) {
            (_, Some(joined)) => match &made[joined] {
                Some(water) => Some(End::Channel(&channels[joined], water)),
                None => {
                    debug_assert!(false, "a channel is worked out before the channel it ends on");
                    None
                }
            },
            (ChannelEnd::Lake(lake), _) => level(lake).map(End::Lake),
            _ => None,
        };
        let ceiling_m = channel.from_lake.and_then(level);
        let ground = match reading {
            Reading::Cut(lakes) => {
                let lake_at = |cell: usize| lake_level_at(lakes, cell);
                let line_m: Vec<f32> = channel.points.iter().map(|point| ground_m(&cut, point)).collect();
                // A cell under the water of a lake, or of the bed of a channel worked out before
                // but for the one it ends on, is ground at that water's surface for its foot.
                let water_at = |cell: usize| lake_at(cell).or_else(|| kept.water_at(cell, cut.data[cell], joined(index)));
                let falls = bank_falls(channel, &cut, &line_m, &water_at);
                // Out of a lake that holds water, where it leaves the lake, by the lake's cells,
                // and what its banks hold from there on.
                let (banks_hold_m, left_m) = match (ceiling_m, channel.from_lake) {
                    (Some(_), Some(lake)) => {
                        let left_m = left_m(channel, size, |cell| lake_of_cell(lakes, cell) == Some(lake));
                        (banks_hold(channel, &cut, &line_m, &lake_at, left_m), left_m)
                    }
                    _ => (Vec::new(), 0.0),
                };
                debug_assert!(banks_hold_m.is_empty() || banks_hold_m.len() == channel.points.len(), "one bank hold a point, or none");
                GroundRead { line_m, falls, banks_hold_m, left_m, caps: std::mem::take(&mut caps_on[index]) }
            }
            Reading::Given(given) => given[index].clone(),
        };
        let wished = wished(channel, &shapes[index].factor, &Ground { size, read: &ground }, ceiling_m, rules);
        wished_m[index] = wished.surface_m.clone();
        let (water, meeting) = profile(channel, wished, floor, ceiling_m, end, &ground);
        // How it arrives at the channel it ends on, its banks read on the ground as the channels
        // before it cut it.
        if let (Reading::Cut(lakes), Some(meeting), Some(End::Channel(river, river_water))) = (reading, &meeting, end) {
            let lake_at = |cell: usize| lake_level_at(lakes, cell);
            let holds = junction_holds(channel, &cut, &lake_at, &Reach::new(river, river_water), meeting);
            let last = &channel.points[channel.points.len() - 1];
            arrivals[index] = Some(Arrival { at: cap_point(&river.points, last, &channel.points[meeting.entry]), entry_m: meeting.entry_m, arrival_m: arrival_m(meeting, &holds) });
        }
        for_each_lowering(heights, &held, channel, &shapes[index], &water, rules, |cell, lowered_m| {
            cut.data[cell] = cut.data[cell].min(lowered_m);
        });
        hold_foot(size, channel, &water, &mut held);
        if let Reading::Cut(lakes) = reading {
            let end_bed: Option<&[usize]> = joined(index).map(|joined| &beds_of[joined].get_or_insert_with(|| bed_cells(size, &channels[joined]))[..]);
            let inside_end = |cell: usize| match channel.end {
                ChannelEnd::Channel(_) => end_bed.is_some_and(|bed| bed.binary_search(&cell).is_ok()),
                ChannelEnd::Lake(lake) => lake_of_cell(lakes, cell) == Some(lake),
                ChannelEnd::Edge => false,
            };
            kept.record(&cut, channel, &shapes[index], &water, rules, index, inside_end);
        }
        (made[index], read[index]) = (Some(water), ground);
    }
    let profiles = made.into_iter().map(|profile| profile.expect("every channel is worked out")).collect();
    Pass { profiles, wished_m, read, cut, arrivals }
}

/// How a tributary arrives at the channel it ends on in a pass (`profiles`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrival {
    /// The point of the channel it ends on that a cap for it goes on (`cap_point`).
    at: usize,
    /// The surface of the channel it ends on at its entry point, `e`, in metres.
    entry_m: f32,
    /// The lowest its banks let it arrive at (`arrival_m`), in metres.
    arrival_m: f32,
}

/// The caps the junctions ask for after a pass whose tributaries arrived as `arrivals` (`first`
/// those of the first pass), under `caps`, the caps the pass was worked out under, each by the index
/// of the tributary that put it, as `profiles` describes them: tributaries before the channels they
/// end on, each one's final arrival the lower of its arrival and the lowest cap on it; where that
/// lies more than `ARRIVAL_ALLOWANCE_M` below `e`, the cap is the highest of it, its `e` in the first
/// pass less `COME_DOWN_MOST_M` and the level of a lake the channel it ends on ends in, and replaces
/// the tributary's cap when it lies below `e` and below that cap. `caps` again when none is new.
fn junction_caps(channels: &[Channel], arrivals: &[Option<Arrival>], first: &[Option<Arrival>], lake_levels: &[f32], caps: &[Option<Cap>]) -> Vec<Option<Cap>> {
    let mut caps = caps.to_vec();
    let mut tributaries: Vec<Vec<usize>> = vec![Vec::new(); channels.len()];
    for (index, channel) in channels.iter().enumerate() {
        if let ChannelEnd::Channel(joined) = channel.end {
            tributaries[joined].push(index);
        }
    }
    for index in ends_first(channels).into_iter().rev() {
        let (Some(arrival), ChannelEnd::Channel(river)) = (arrivals[index], channels[index].end) else { continue };
        let lowest_m = tributaries[index].iter().filter_map(|&tributary| caps[tributary]).map(|cap| cap.level_m).fold(f32::INFINITY, f32::min);
        let final_m = arrival.arrival_m.min(lowest_m);
        if arrival.entry_m - final_m <= ARRIVAL_ALLOWANCE_M {
            continue;
        }
        let first_m = first[index].map_or(arrival.entry_m, |first| first.entry_m);
        let lake_m = match channels[river].end {
            ChannelEnd::Lake(lake) => lake_levels.get(lake).copied().filter(|level| level.is_finite()),
            _ => None,
        };
        let cap_m = final_m.max(first_m - COME_DOWN_MOST_M).max(lake_m.unwrap_or(f32::NEG_INFINITY));
        if cap_m < arrival.entry_m && caps[index].is_none_or(|old| cap_m < old.level_m) {
            caps[index] = Some(Cap { at: arrival.at, level_m: cap_m });
        }
    }
    caps
}

/// The lowest a tributary arrives at the channel it ends on, its banks holding it, as `profiles`
/// describes it: over its points before its entry point (`meeting`), the higher of its surface
/// before it was raised to `e` and the lower of `e` and what its banks hold there (`holds`, `e`
/// where they hold at least `e` less `FREEBOARD_M`).
fn arrival_m(meeting: &Meeting, holds: &[f32]) -> f32 {
    let entry_m = meeting.entry_m;
    (meeting.own_m.iter().zip(holds))
        .map(|(&own_m, &hold_m)| {
            let hold_m = if hold_m >= entry_m - FREEBOARD_M { entry_m } else { hold_m };
            own_m.max(hold_m.min(entry_m))
        })
        .fold(f32::INFINITY, f32::min)
}

/// What the banks of `channel` hold its water at where it meets the channel it ends on (`river`),
/// beside each of its points, as `profiles` describes it, on `heights` (the ground as the channels
/// worked out before it have cut it): of the cells `bank_falls` reads, those neither inside
/// `river`'s bed nor in its own bank (`Reach::nearest`), a cell that holds a lake's water
/// (`lake_at`) ground at that lake's level; the lowest beside the point less `FREEBOARD_M`.
/// INFINITY beside a point no such cell lies by, and beside every point the arrival does not read
/// it at: one at or past the entry point, or whose surface before it was raised (`meeting`) lies at
/// or above `e`, where the higher of the two is its surface whatever its banks hold.
fn junction_holds(channel: &Channel, heights: &Grid<f32>, lake_at: &LakeAt, river: &Reach, meeting: &Meeting) -> Vec<f32> {
    let (width, height) = (heights.width, heights.height);
    let half = (width as f64 / 2.0, height as f64 / 2.0);
    let read = |k: usize| k < meeting.entry && meeting.own_m[k] < meeting.entry_m;
    let mut holds = vec![f32::INFINITY; channel.points.len()];
    for_each_nearest((width, height), channel, BANK_FOOT_M, |cell, near| {
        let k = near.first;
        if near.distance_m > near.half_width_m && !near.beyond_first && !near.beyond_last && (read(k) || read(k + 1)) {
            let ground_m = lake_at(cell).unwrap_or(heights.data[cell]);
            let at = ((cell % width) as f64 - half.0, (cell / width) as f64 - half.1);
            let river_holds = river.nearest(at).is_some_and(|(apart_m, half_width_m, surface_m)| apart_m <= half_width_m || ground_m >= surface_m);
            if !river_holds {
                for hold in &mut holds[k..=k + 1] {
                    *hold = hold.min(ground_m - FREEBOARD_M);
                }
            }
        }
    });
    holds
}

/// The point of the line through `line` a cap for a tributary goes on, as `profiles` describes it:
/// the earlier of the point nearest the tributary's last point (`last`), and the first point of the
/// stretch nearest its entry point (`entry`); each the first of several as near.
fn cap_point(line: &[ChannelPoint], last: &ChannelPoint, entry: &ChannelPoint) -> usize {
    let at = |point: &ChannelPoint| (point.x_m, point.z_m);
    let nearest = (0..line.len()).fold(0, |best, k| if distance_m(at(&line[k]), at(last)) < distance_m(at(&line[best]), at(last)) { k } else { best });
    let (mut stretch, mut stretch_m) = (0, f64::INFINITY);
    for k in 1..line.len() {
        let (from, to) = (at(&line[k - 1]), at(&line[k]));
        let along = (to.0 - from.0, to.1 - from.1);
        let squared = along.0 * along.0 + along.1 * along.1;
        let share = if squared > 0.0 { (((entry.x_m - from.0) * along.0 + (entry.z_m - from.1) * along.1) / squared).clamp(0.0, 1.0) } else { 0.0 };
        let apart_m = distance_m((from.0 + along.0 * share, from.1 + along.1 * share), at(entry));
        if apart_m < stretch_m {
            (stretch, stretch_m) = (k - 1, apart_m);
        }
    }
    nearest.min(stretch)
}

/// A channel's water, read where its banks reach (`Reach::nearest`): each of its stretches with
/// the box its banks reach from it.
struct Reach<'a> {
    channel: &'a Channel,
    water: &'a Profile,
    /// Each stretch's box, `[west, east, north, south]` in world metres, grown by the channel's
    /// widest half width (no less than `HALF_WIDTH_MIN_M`) and `BANK_REACH_M`: stretch `k - 1` to
    /// `k` at place `k - 1`.
    boxes: Vec<[f64; 4]>,
}

impl<'a> Reach<'a> {
    /// The reach of `channel`'s banks, whose water is `water`.
    fn new(channel: &'a Channel, water: &'a Profile) -> Self {
        let widest_m = channel.points.iter().map(|point| (f64::from(point.width_m) / 2.0).max(HALF_WIDTH_MIN_M)).fold(0.0, f64::max);
        let grow_m = widest_m + BANK_REACH_M;
        let boxes = (channel.points.windows(2))
            .map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                [a.x_m.min(b.x_m) - grow_m, a.x_m.max(b.x_m) + grow_m, a.z_m.min(b.z_m) - grow_m, a.z_m.max(b.z_m) + grow_m]
            })
            .collect();
        Self { channel, water, boxes }
    }

    /// How far world `(x, z)` lies from the nearest point of the channel's line, and the half width
    /// and surface of its water there (`water_nearest`), when it lies within its banks' reach there
    /// (no more than `BANK_REACH_M` beyond its waterline); `None` farther out. Only the stretches
    /// whose box holds `(x, z)` are looked at: any other lies farther from it than any place its
    /// banks reach, so whenever the nearest point of the whole line lies within reach, it is the
    /// one these give, the first of several as near.
    fn nearest(&self, (x_m, z_m): (f64, f64)) -> Option<(f64, f64, f32)> {
        let point = ChannelPoint { x_m, z_m, width_m: 0.0, drained_m2: 0.0 };
        let looked_at = |k: usize| {
            let [west, east, north, south] = self.boxes[k - 1];
            (west..=east).contains(&x_m) && (north..=south).contains(&z_m)
        };
        let (apart_m, half_width_m, surface_m) = nearest_on_line(&self.channel.points, |k| f64::from(self.water.surface_m[k]), &point, looked_at);
        (apart_m - half_width_m <= BANK_REACH_M).then_some((apart_m, half_width_m, surface_m as f32))
    }
}

/// The notes on the junctions after the last pass, as `profiles` describes them. A junction is
/// said where its tributary put a cap on the channel it ends on (`caps`) and brought it down at its
/// entry point by more than `DEEPER_NOTE_M`, from its `e` in the first pass (`first`) to its `e` in
/// the last (`last`); and where the tributary still arrives (`last`) more than `DEEPER_NOTE_M`
/// below its `e`, where the 3 m or a lake stopped the cap, or the passes ran out. Each junction is
/// judged on its own, and then gathered (`gathered`).
///
/// First the channels cut deeper, the drawn rivers by their number N and then the brooks; on each,
/// one note for each drawn tributary, by its number M, and one for all the brooks, X the most of
/// them: "River N was cut X m deeper where river M joins it.", then "River N was cut X m deeper
/// where a brook joins it." or, for K brooks, "River N was cut up to X m deeper where K brooks join
/// it."; on the brooks, "A brook was cut X m deeper where river M joins it.", then, over every
/// junction of a brook on a brook, "A brook was cut X m deeper where a brook joins it." or "K brooks
/// were cut up to X m deeper where brooks join them.". Then the steps left, in the same order:
/// "River M joins river N X m above its own banks; the water steps there.", then "A brook joins
/// river N X m above its own banks; the water steps there." or "K brooks join river N up to X m
/// above their own banks; the water steps there."; "River M joins a brook X m above its own banks;
/// the water steps there.", then "A brook joins a brook X m above its own banks; the water steps
/// there." or "K brooks join brooks up to X m above their own banks; the water steps there.". N
/// and M are drawn numbers counted from 1, as the other water notes name rivers; X is to one
/// decimal. A channel that only one brook joins reads as that junction would alone. Each note is
/// about the first drawn river it names (N where a river is cut deeper, M where a river steps,
/// whichever of the two the note names first), and about the land when it names only brooks.
fn junction_notes(channels: &[Channel], first: &[Option<Arrival>], last: &[Option<Arrival>], caps: &[Option<Cap>]) -> Vec<Note> {
    let (mut deeper, mut steps): (Vec<Said>, Vec<Said>) = (Vec::new(), Vec::new());
    for (index, channel) in channels.iter().enumerate() {
        let (ChannelEnd::Channel(river), Some(arrival)) = (channel.end, last[index]) else { continue };
        let river = channels[river].drawn;
        let deeper_m = first[index].map_or(0.0, |first| first.entry_m - arrival.entry_m);
        if caps[index].is_some() && deeper_m > DEEPER_NOTE_M {
            deeper.push((river, channel.drawn, deeper_m));
        }
        let step_m = arrival.entry_m - arrival.arrival_m;
        if step_m > DEEPER_NOTE_M {
            steps.push((river, channel.drawn, step_m));
        }
    }
    let mut notes = gathered(deeper, |river, joining, x| match (river, joining) {
        (Some(n), Joining::River(m)) => (About::River(n), format!("River {} was cut {x:.1} m deeper where river {} joins it.", n + 1, m + 1)),
        (Some(n), Joining::Brooks(1)) => (About::River(n), format!("River {} was cut {x:.1} m deeper where a brook joins it.", n + 1)),
        (Some(n), Joining::Brooks(k)) => (About::River(n), format!("River {} was cut up to {x:.1} m deeper where {k} brooks join it.", n + 1)),
        (None, Joining::River(m)) => (About::River(m), format!("A brook was cut {x:.1} m deeper where river {} joins it.", m + 1)),
        (None, Joining::Brooks(1)) => (About::Land, format!("A brook was cut {x:.1} m deeper where a brook joins it.")),
        (None, Joining::Brooks(k)) => (About::Land, format!("{k} brooks were cut up to {x:.1} m deeper where brooks join them.")),
    });
    notes.extend(gathered(steps, |river, joining, x| match (river, joining) {
        (Some(n), Joining::River(m)) => (About::River(m), format!("River {} joins river {} {x:.1} m above its own banks; the water steps there.", m + 1, n + 1)),
        (Some(n), Joining::Brooks(1)) => (About::River(n), format!("A brook joins river {} {x:.1} m above its own banks; the water steps there.", n + 1)),
        (Some(n), Joining::Brooks(k)) => (About::River(n), format!("{k} brooks join river {} up to {x:.1} m above their own banks; the water steps there.", n + 1)),
        (None, Joining::River(m)) => (About::River(m), format!("River {} joins a brook {x:.1} m above its own banks; the water steps there.", m + 1)),
        (None, Joining::Brooks(1)) => (About::Land, format!("A brook joins a brook {x:.1} m above its own banks; the water steps there.")),
        (None, Joining::Brooks(k)) => (About::Land, format!("{k} brooks join brooks up to {x:.1} m above their own banks; the water steps there.")),
    }));
    notes
}

/// A junction a note says (`junction_notes`): the drawn number of the channel the tributary ends
/// on and the tributary's own, each `None` for a brook, and how far, in metres.
type Said = (Option<usize>, Option<usize>, f32);

/// Who joins at the junctions one note says (`gathered`): a drawn river, by its drawn number, or
/// this many brooks.
#[derive(Debug, Clone, Copy)]
enum Joining {
    River(usize),
    Brooks(usize),
}

/// The notes on the junctions `said`, gathered as `junction_notes` describes: by the channel they
/// end on, the drawn rivers by number and then the brooks, all of which are one; for each, a note
/// for each drawn tributary by its number, the list's order among the same, and then one for all
/// its brooks at the most of them. `words` says a note, with what it is about: on what (a drawn
/// number, `None` for the brooks), who joins, and how far.
fn gathered(mut said: Vec<Said>, words: impl Fn(Option<usize>, Joining, f32) -> Note) -> Vec<Note> {
    said.sort_by_key(|&(river, tributary, _)| ((river.is_none(), river), (tributary.is_none(), tributary)));
    let mut notes = Vec::new();
    for on in said.chunk_by(|a, b| a.0 == b.0) {
        let river = on[0].0;
        for &(_, tributary, by_m) in on {
            if let Some(drawn) = tributary {
                notes.push(words(river, Joining::River(drawn), by_m));
            }
        }
        let brooks: Vec<f32> = on.iter().filter(|said| said.1.is_none()).map(|said| said.2).collect();
        if let Some(most_m) = brooks.iter().copied().reduce(f32::max) {
            notes.push(words(river, Joining::Brooks(brooks.len()), most_m));
        }
    }
    notes
}

/// A cap a junction puts on the channel a tributary ends on (`profiles`): from the channel's point
/// `at` on its surface stands no higher than `level_m`, and upstream of it no higher than the cap's
/// line.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cap {
    at: usize,
    level_m: f32,
}

/// The water the beds of the channels worked out so far in a pass hold, by cell, and the channel
/// whose water it is: what the foot of a later channel's banks reads a cell under that water at
/// (`bank_falls`, as `profiles` describes it).
#[derive(Debug, Default)]
struct KeptBeds(HashMap<usize, (f32, usize)>);

impl KeptBeds {
    /// Records the water of `channel`, the channel of index `index`, shaped by `shape`, whose water
    /// is `profile`, on `heights` as its bed cut them (laid out as `profiles` describes): the cells
    /// `cut_beds` seeds its water in, by the same test (`reach_of`, `starts_water`), each at the
    /// surface it seeds there. Those are the cells of its bed (no farther from its line than its
    /// half width, no less than `HALF_WIDTH_MIN_M`, at one of their nearest points) whose ground
    /// lies below its surface there, the highest of several points as near, but none inside what it
    /// ends on (`inside_end`) and, for a channel that begins on dry land, none straight beyond its
    /// first point. A cell keeps the highest surface recorded in it, and the channel whose water
    /// that is, the first of several as high.
    ///
    /// Only the ground the test reads differs: `heights` is the ground as the channels worked out
    /// so far in the pass have cut it, this one last, where `cut_beds` reads the ground as every
    /// channel cut it. A cell that a channel worked out later lowers below this one's surface, such
    /// as one on its waterline (which its own bowl leaves at its surface) under a later tributary's
    /// bed, is seeded there and not recorded here.
    #[allow(clippy::too_many_arguments)]
    fn record(&mut self, heights: &Grid<f32>, channel: &Channel, shape: &Shape, profile: &Profile, rules: &WaterRules, index: usize, inside_end: impl Fn(usize) -> bool) {
        for (cell, surface_m, in_bed) in reach_of((heights.width, heights.height), channel, shape, profile, rules, true, inside_end) {
            if starts_water(surface_m, in_bed, heights.data[cell]) {
                let kept = self.0.entry(cell).or_insert((f32::NEG_INFINITY, index));
                if surface_m as f32 > kept.0 {
                    *kept = (surface_m as f32, index);
                }
            }
        }
    }

    /// The surface of the water `cell` holds in the bed of a channel recorded, but for that of the
    /// channel `ends_on`, where it lies above the cell's ground, `ground_m`; `None` where it does
    /// not, the cell then ground at its own height.
    fn water_at(&self, cell: usize, ground_m: f32, ends_on: Option<usize>) -> Option<f32> {
        self.0.get(&cell).filter(|&&(surface_m, of)| Some(of) != ends_on && surface_m > ground_m).map(|&(surface_m, _)| surface_m)
    }
}

/// The cells inside the bed of `channel`, on a grid of `size` (its width and height) laid out as
/// `profiles` describes: no farther from its line than half its width, no less than
/// `HALF_WIDTH_MIN_M`, at their nearest point (`for_each_nearest`), in index order.
fn bed_cells(size: (usize, usize), channel: &Channel) -> Vec<usize> {
    let mut inside = Vec::new();
    for_each_nearest(size, channel, 0.0, |cell, _| inside.push(cell));
    inside.dedup();
    inside
}

/// Where a channel meets the channel it ends on, as `profile` worked it out: its entry point, the
/// surface `e` it got there, and its surfaces before the entry point as they stood before they were
/// raised to `e`.
#[derive(Debug, Clone, PartialEq)]
struct Meeting {
    entry: usize,
    entry_m: f32,
    own_m: Vec<f32>,
}

/// What a channel's water ends on.
#[derive(Debug, Clone, Copy)]
enum End<'a> {
    /// A lake with water, at this level.
    Lake(f32),
    /// Another channel, whose water this is.
    Channel(&'a Channel, &'a Profile),
}

/// What a channel reads of the ground under it, as the channels worked out before it have cut it
/// (`profiles`): the ground at each of its points (bilinear, the nearest edge's beyond the grid),
/// and how far the foot of its banks falls below its line beside each point (`bank_falls`); and,
/// for a channel that flows out of a lake, what its banks hold its water at beside each point
/// (`banks_hold`) and where it leaves the lake (`left_m`); and the caps its tributaries' junctions
/// put on it in the pass that read it.
#[derive(Debug, Clone, PartialEq, Default)]
struct GroundRead {
    line_m: Vec<f32>,
    falls: Vec<f32>,
    /// Out of a lake, the most the banks hold the water at beside each point (`banks_hold`), in
    /// metres; empty for a channel out of no lake, which nothing holds so.
    banks_hold_m: Vec<f32>,
    /// Out of a lake, how far along the channel from its first point its stretch beside the lake
    /// reaches (`left_m`), in metres; 0 for a channel out of no lake.
    left_m: f64,
    /// The caps on its surface (`Cap`), one for each tributary that put one on it, in the list's
    /// order of the tributaries; none where no junction brings it down.
    caps: Vec<Cap>,
}

/// What `wished` reads: a channel's ground (`GroundRead`) on a grid of `size` (its width and
/// height).
#[derive(Clone, Copy)]
struct Ground<'a> {
    size: (usize, usize),
    read: &'a GroundRead,
}

/// What a channel's own ground asks of its water at each point, before the running minimum and
/// the steps after it (`profiles`).
struct Wished {
    /// The point's distance along the channel from its first, in metres.
    along_m: Vec<f64>,
    /// The bed's depth under the deep line, in metres (`Profile::depth_m`).
    depth_m: Vec<f32>,
    /// The wished surface, held by the banks, in metres.
    surface_m: Vec<f32>,
    /// Whether the channel begins on dry land (`Profile::on_dry_land`).
    on_dry_land: bool,
}

/// The water along `channel`, as `profiles` describes it, from what its own ground asks of it
/// (`wished`): `floor` the floor under each point, `ceiling_m` the level of the lake it flows out
/// of, `end` what it ends on, and `read` what it read of its ground (out of a lake, what its banks
/// hold and where it leaves the lake) and the caps on it; and, for a channel that ends on another
/// and has a point before its entry point, where it meets that one (`Meeting`).
fn profile(channel: &Channel, wished: Wished, floor: &Floor, ceiling_m: Option<f32>, end: Option<End>, read: &GroundRead) -> (Profile, Option<Meeting>) {
    let points = &channel.points;
    let Wished { along_m, depth_m, surface_m: wished_m, on_dry_land } = wished;
    // How far each point lies from the line of the channel it ends on, if it ends on one, and the
    // half width and the surface of that channel's water at its point nearest it.
    let against: Vec<(f64, f64, f32)> = match end {
        Some(End::Channel(joined, water)) => points.iter().map(|point| water_nearest(joined, water, point)).collect(),
        Some(End::Lake(_)) | None => Vec::new(),
    };
    let inside_joined = |k: usize| against.get(k).is_some_and(|&(apart_m, half_width_m, _)| apart_m <= half_width_m);
    let mut lowest_m = ceiling_m.unwrap_or(f32::INFINITY);
    let mut surface_m: Vec<f32> = (wished_m.iter())
        .map(|&wished_m| {
            lowest_m = lowest_m.min(wished_m);
            lowest_m
        })
        .collect();
    let floor_m: Vec<f32> = points.iter().map(floor).collect();
    if let Some(level_m) = ceiling_m {
        // Out of a lake: the level outright along the stretch beside the lake; past it the lowest
        // of the level, what the banks hold, and the higher of its own surface and the lake's
        // grade from where it leaves the lake. Then the running minimum.
        let left_m = read.left_m;
        // One hold a point, or none where nothing holds the banks so (a ground the tests give).
        debug_assert!(read.banks_hold_m.is_empty() || read.banks_hold_m.len() == points.len(), "one bank hold a point, or none");
        let mut lowest_m = f32::INFINITY;
        for (k, (surface, &along)) in surface_m.iter_mut().zip(&along_m).enumerate() {
            let held_m = if left_m > 0.0 && along <= left_m {
                level_m
            } else {
                let graded_m = (f64::from(level_m) - JUNCTION_GRADE * (along - left_m)) as f32;
                let banks_m = read.banks_hold_m.get(k).copied().unwrap_or(f32::INFINITY);
                level_m.min(banks_m).min(surface.max(graded_m))
            };
            lowest_m = lowest_m.min(held_m);
            *surface = lowest_m;
        }
    }
    // The caps its tributaries' junctions put on it: each surface no higher than any cap's line,
    // but no lower than the lake's level along its stretch beside the lake it flows out of. Each
    // line and the surfaces before never rise downstream, the lake's level stands above every
    // surface past the stretch, so neither do these.
    if !read.caps.is_empty() {
        let own_m = surface_m.clone();
        let beside_m = ceiling_m.filter(|_| read.left_m > 0.0);
        for (k, surface) in surface_m.iter_mut().enumerate() {
            let capped_m = read.caps.iter().map(|cap| cap_line_m(cap, k, &own_m, &along_m, on_dry_land)).fold(f32::INFINITY, f32::min);
            let capped_m = match beside_m {
                Some(level_m) if along_m[k] <= read.left_m => capped_m.max(level_m),
                _ => capped_m,
            };
            *surface = surface.min(capped_m);
        }
    }
    // The surfaces from the entry point on, upstream first: for a channel that ends on another,
    // from its first point from which every later point lies inside that channel's bed, its last
    // point at least, that channel's surface at its point nearest each.
    let entered: Vec<f32> = match (end, points.len().checked_sub(1)) {
        (None, _) | (Some(End::Channel(..)), None) => Vec::new(),
        (Some(End::Lake(level_m)), _) => vec![level_m],
        (Some(End::Channel(..)), Some(last)) => {
            debug_assert!(against[last].0 == 0.0, "a channel ends on no point of the channel it joins");
            let entry = (0..last).rev().take_while(|&k| inside_joined(k)).last().unwrap_or(last);
            against[entry..].iter().map(|&(_, _, surface)| surface).collect()
        }
    };
    let mut meeting = None;
    if let Some(&entry_m) = entered.first() {
        let entry = points.len() - entered.len();
        let (own_entry_m, along_entry_m) = (f64::from(surface_m[entry]), along_m[entry]);
        // The cap is a sum of two parts that never rise downstream and is `entry_m` at the entry
        // point; the floor is constant; so these surfaces never rise downstream and end at
        // `entry_m` or above.
        let mut own_m = Vec::with_capacity(entry);
        for (surface, &along) in surface_m[..entry].iter_mut().zip(&along_m) {
            let own_fall_m = (f64::from(*surface) - own_entry_m).max(0.0);
            let graded_m = (f64::from(entry_m) + JUNCTION_GRADE * (along_entry_m - along) + own_fall_m) as f32;
            own_m.push(surface.min(graded_m));
            *surface = surface.min(graded_m).max(entry_m);
        }
        if matches!(end, Some(End::Channel(..))) && entry > 0 {
            meeting = Some(Meeting { entry, entry_m, own_m });
        }
        let mut lowest_m = f32::INFINITY;
        for (surface, entered_m) in surface_m[entry..].iter_mut().zip(entered) {
            lowest_m = lowest_m.min(entered_m);
            *surface = lowest_m;
        }
    }
    debug_assert!(surface_m.windows(2).all(|pair| pair[1] <= pair[0]), "a channel's surface rises downstream: {surface_m:?}");
    let above_floor_m = surface_m.iter().zip(&floor_m).map(|(surface, floor)| surface - floor).fold(0.0, f32::max);
    (Profile { surface_m, depth_m, above_floor_m, on_dry_land }, meeting)
}

/// The highest `cap` lets a channel's surface stand at its point `k`, as `profiles` describes it:
/// the cap from its point on; upstream of it the cap plus `JUNCTION_GRADE` times the distance along
/// to its point (`along_m`) plus how far the surface there lies above the surface at its point, both
/// as they stood before the caps (`own_m`); and, for a channel that begins on dry land, no lower
/// than its surface less the cap's lowering at its point times its distance along over that
/// point's, so its head keeps its ground.
fn cap_line_m(cap: &Cap, k: usize, own_m: &[f32], along_m: &[f64], on_dry_land: bool) -> f32 {
    let Cap { at, level_m } = *cap;
    if k >= at {
        return level_m;
    }
    let line_m = (f64::from(level_m) + JUNCTION_GRADE * (along_m[at] - along_m[k]) + f64::from(own_m[k] - own_m[at])) as f32;
    if on_dry_land && along_m[at] > 0.0 {
        let lowered_m = (own_m[at] - level_m).max(0.0);
        line_m.max((f64::from(own_m[k]) - f64::from(lowered_m) * along_m[k] / along_m[at]) as f32)
    } else {
        line_m
    }
}

/// What `channel`'s own ground (`ground`) asks of its water, as `profiles` describes it: each
/// point's distance along it, the bed's depth under its deep line, and the wished surface its
/// banks hold, before the running minimum and the steps after it; `factor` the wander's factor at
/// each point (its width before the wander is its width over it), `ceiling_m` the level of the
/// lake it flows out of.
fn wished(channel: &Channel, factor: &[f32], ground: &Ground, ceiling_m: Option<f32>, rules: &WaterRules) -> Wished {
    let Ground { size: (width, height), read } = *ground;
    let points = &channel.points;
    debug_assert_eq!((points.len(), points.len()), (read.line_m.len(), read.falls.len()), "one ground and one fall per point");
    let along_m = along_m(points);
    let (half_x, half_z) = (width as f64 / 2.0, height as f64 / 2.0);
    let on_dry_land = ceiling_m.is_none()
        && points.first().is_some_and(|first| (half_x - first.x_m.abs()).min(half_z - first.z_m.abs()) > BORDER_START_M);
    // The depth the width before the wander gives, which sets the surface, and the bed's depth
    // under the deep line, each grown over a dry-land head.
    debug_assert_eq!(points.len(), factor.len(), "one factor per point");
    let (unwandered_m, depth_m): (Vec<f32>, Vec<f32>) = (points.iter().zip(&along_m).zip(factor))
        .map(|((point, &along), &factor)| {
            let full = bed_depth_m(point.width_m / factor, rules);
            let deep = (full / factor).clamp(rules.bed_depth_min_m, rules.bed_depth_max_m);
            let grown = if on_dry_land { (along / HEAD_GROWTH_M).min(1.0) as f32 } else { 1.0 };
            (full * grown, deep * grown)
        })
        .unzip();
    let surface_m: Vec<f32> = (read.line_m.iter().zip(&unwandered_m).zip(&read.falls))
        .map(|((&line_m, &depth), &fall_m)| {
            let wished_m = line_m - (1.0 - rules.water_fill) * depth;
            // Its banks hold it where they fall below the line by more than rounding.
            if fall_m > ROUNDING_M {
                wished_m.min(line_m - fall_m - FREEBOARD_M)
            } else {
                wished_m
            }
        })
        .collect();
    Wished { along_m, depth_m, surface_m, on_dry_land }
}

/// Each of `points`' distance along their line from the first, in metres, summed point by point
/// in order, so the same points always give the same distances.
fn along_m(points: &[ChannelPoint]) -> Vec<f64> {
    let at = |point: &ChannelPoint| (point.x_m, point.z_m);
    let mut along_m = Vec::with_capacity(points.len());
    for k in 0..points.len() {
        let before_m = if k == 0 { 0.0 } else { along_m[k - 1] + distance_m(at(&points[k - 1]), at(&points[k])) };
        along_m.push(before_m);
    }
    along_m
}

/// How far along `channel` from its first point its stretch beside a lake reaches, in metres, as
/// `profiles` describes it: to the far end of its last leading segment beside the lake whose water
/// `holds` (by a cell's index, on a grid of `size` laid out as `profiles` describes;
/// `segments_beside`), 0 when its first segment is not beside it.
fn left_m(channel: &Channel, size: (usize, usize), holds: impl Fn(usize) -> bool) -> f64 {
    let leading = segments_beside(channel, holds, size);
    along_m(&channel.points).get(leading).copied().unwrap_or(0.0)
}

/// What the banks of `channel`, which flows out of a lake, hold its water at beside each of its
/// points, on `heights` (as the channels worked out before it have cut them), as `profiles`
/// describes it: of the cells `bank_falls` reads, those whose nearest place along the line lies at
/// or past `left_m`; at each point the line's ground (`line_m`) less the most that any of them on a
/// stretch one of whose ends is the point lies below the chord there, less `FREEBOARD_M`, and no
/// higher than the level of any lake (`lake_at`) one of them holds the water of; INFINITY beside a
/// point no such cell lies by.
fn banks_hold(channel: &Channel, heights: &Grid<f32>, line_m: &[f32], lake_at: &LakeAt, left_m: f64) -> Vec<f32> {
    let along_m = along_m(&channel.points);
    let line_m: Vec<f64> = line_m.iter().map(|&line_m| f64::from(line_m)).collect();
    let mut falls = vec![f64::NEG_INFINITY; channel.points.len()];
    let mut lakes_m = vec![f32::INFINITY; channel.points.len()];
    for_each_nearest((heights.width, heights.height), channel, BANK_FOOT_M, |cell, near| {
        let k = near.first;
        let place_m = along_m[k] + (along_m[k + 1] - along_m[k]) * near.share;
        if near.distance_m > near.half_width_m && !near.beyond_first && !near.beyond_last && place_m >= left_m {
            match lake_at(cell) {
                Some(level_m) => {
                    for lake_m in &mut lakes_m[k..=k + 1] {
                        *lake_m = lake_m.min(level_m);
                    }
                }
                None => {
                    let fall_m = line_m[k] + (line_m[k + 1] - line_m[k]) * near.share - f64::from(heights.data[cell]);
                    for fall in &mut falls[k..=k + 1] {
                        *fall = fall.max(fall_m);
                    }
                }
            }
        }
    });
    (falls.iter().zip(&line_m).zip(&lakes_m))
        .map(|((&fall_m, &line_m), &lake_m)| {
            let banks_m = if fall_m == f64::NEG_INFINITY { f32::INFINITY } else { (line_m - fall_m) as f32 - FREEBOARD_M };
            banks_m.min(lake_m)
        })
        .collect()
}

/// The notes on `channels`, whose water is `profiles`, whose surface stands more than
/// `ABOVE_FLOOR_SAID_M` above its floor somewhere (`Profile::above_floor_m`): for each drawn river
/// so, in the drawing's order, "River N stands up to X m above its valley.", N counting from 1 and X
/// the most of its channels', to one decimal, about that river; then one for all the brooks so, "K
/// brooks stand up to X m above their valleys." ("1 brook stands up to X m above its valley."),
/// about the land. None when no channel stands so high.
pub(crate) fn above_floor_notes(channels: &[Channel], profiles: &[Profile]) -> Vec<Note> {
    let mut rivers: Vec<(usize, f32)> = Vec::new();
    let (mut brooks, mut brooks_m) = (0, 0.0_f32);
    let high = |profile: &&Profile| profile.above_floor_m > ABOVE_FLOOR_SAID_M;
    for (channel, profile) in channels.iter().zip(profiles).filter(|(_, profile)| high(profile)) {
        match channel.drawn {
            Some(drawn) => match rivers.iter_mut().find(|(river, _)| *river == drawn) {
                Some((_, most_m)) => *most_m = most_m.max(profile.above_floor_m),
                None => rivers.push((drawn, profile.above_floor_m)),
            },
            None => (brooks, brooks_m) = (brooks + 1, brooks_m.max(profile.above_floor_m)),
        }
    }
    rivers.sort_by_key(|&(drawn, _)| drawn);
    let mut notes: Vec<Note> = (rivers.iter())
        .map(|&(drawn, most_m)| (About::River(drawn), format!("River {} stands up to {most_m:.1} m above its valley.", drawn + 1)))
        .collect();
    match brooks {
        0 => {}
        1 => notes.push((About::Land, format!("1 brook stands up to {brooks_m:.1} m above its valley."))),
        _ => notes.push((About::Land, format!("{brooks} brooks stand up to {brooks_m:.1} m above their valleys."))),
    }
    notes
}

/// How far `point` lies from the nearest point of `channel`'s line, and the half width (no less
/// than `HALF_WIDTH_MIN_M`) and surface of its water (`water`) there; at a point of the line, that
/// point's own.
fn water_nearest(channel: &Channel, water: &Profile, point: &ChannelPoint) -> (f64, f64, f32) {
    let (apart_m, half_width_m, surface_m) = nearest_on_line(&channel.points, |k| f64::from(water.surface_m[k]), point, |_| true);
    (apart_m, half_width_m, surface_m as f32)
}

/// Whether `point` lies inside the bed of the channel whose points are `line`: no farther from the
/// nearest point of its line than half its width there, no less than `HALF_WIDTH_MIN_M` as the bed
/// is cut, as `profiles` finds the points of a channel inside the bed of the channel it ends on.
pub(super) fn inside_bed(line: &[ChannelPoint], point: &ChannelPoint) -> bool {
    let (apart_m, half_width_m, _) = nearest_on_line(line, |_| 0.0, point, |_| true);
    apart_m <= half_width_m
}

/// How far `point` lies from the nearest point of the line through `line`, the first of several as
/// near; and the half width there, each point's no less than `HALF_WIDTH_MIN_M` as the bed is cut
/// (`for_each_nearest`), and `value` there, both linear along the stretch (at a point of the line,
/// that point's own). Only the stretches `looked_at` takes (stretch `k - 1` to `k` by `k`) are
/// looked at, besides the first point.
fn nearest_on_line(line: &[ChannelPoint], value: impl Fn(usize) -> f64, point: &ChannelPoint, looked_at: impl Fn(usize) -> bool) -> (f64, f64, f64) {
    let at = |point: &ChannelPoint| (point.x_m, point.z_m);
    let half_width_m = |k: usize| (f64::from(line[k].width_m) / 2.0).max(HALF_WIDTH_MIN_M);
    let mut nearest = (distance_m(at(&line[0]), at(point)), half_width_m(0), value(0));
    for k in (1..line.len()).filter(|&k| looked_at(k)) {
        let (from, to) = (at(&line[k - 1]), at(&line[k]));
        let along = (to.0 - from.0, to.1 - from.1);
        let squared = along.0 * along.0 + along.1 * along.1;
        let across = (point.x_m - from.0) * along.0 + (point.z_m - from.1) * along.1;
        let share = if squared > 0.0 { (across / squared).clamp(0.0, 1.0) } else { 0.0 };
        let between = |a: f64, b: f64| {
            if share <= 0.0 {
                a
            } else if share >= 1.0 {
                b
            } else {
                a + (b - a) * share
            }
        };
        let apart_m = distance_m((between(from.0, to.0), between(from.1, to.1)), at(point));
        if apart_m < nearest.0 {
            nearest = (apart_m, between(half_width_m(k - 1), half_width_m(k)), between(value(k - 1), value(k)));
        }
    }
    nearest
}

/// How far the foot of `channel`'s banks falls below its line beside each of its points, on
/// `heights` (as the channels worked out before it have cut them), as `profiles` describes it:
/// the most that any cell beyond the waterline by no more than `BANK_FOOT_M` at its nearest point
/// of the line, on either side, whose nearest point lies on a stretch one of whose ends is the
/// point, lies below the chord there, the straight line between the line's ground at the
/// stretch's two points (`line_m`, one per point, as the wished surface reads it) at the cell's
/// share along it; a cell under water (`water_at`: a lake's, or what the bed of a channel worked
/// out before holds, but for the one it ends on, `KeptBeds`) is ground at that water's surface.
/// A cell straight
/// beyond the line's first or last point (`Near::beyond_first`, `Near::beyond_last`) is no part of
/// the foot. 0 beside a point no such cell lies below the line by.
fn bank_falls(channel: &Channel, heights: &Grid<f32>, line_m: &[f32], water_at: &LakeAt) -> Vec<f32> {
    let line_m: Vec<f64> = line_m.iter().map(|&line_m| f64::from(line_m)).collect();
    let mut falls = vec![0.0_f32; channel.points.len()];
    for_each_nearest((heights.width, heights.height), channel, BANK_FOOT_M, |cell, near| {
        if near.distance_m > near.half_width_m && !near.beyond_first && !near.beyond_last {
            let k = near.first;
            let chord_m = line_m[k] + (line_m[k + 1] - line_m[k]) * near.share;
            let fall_m = (chord_m - f64::from(water_at(cell).unwrap_or(heights.data[cell]))) as f32;
            for fall in &mut falls[k..=k + 1] {
                *fall = fall.max(fall_m);
            }
        }
    });
    falls
}

/// A bed's depth under water `width_m` wide: `bed_depth_share` of the width, kept from
/// `bed_depth_min_m` to `bed_depth_max_m`.
fn bed_depth_m(width_m: f32, rules: &WaterRules) -> f32 {
    (rules.bed_depth_share * width_m).clamp(rules.bed_depth_min_m, rules.bed_depth_max_m)
}

/// World `(x, z)` as `(column, row)` on a `width` x `height` grid laid out as `profiles` describes.
fn on_grid((width, height): (usize, usize), point: &ChannelPoint) -> (f64, f64) {
    (point.x_m + width as f64 / 2.0, point.z_m + height as f64 / 2.0)
}

/// The ground under `point`, bilinear between the cells of `heights` around it, the nearest edge's
/// beyond the grid.
fn ground_m(heights: &Grid<f32>, point: &ChannelPoint) -> f32 {
    let (width, height) = (heights.width, heights.height);
    let (column, row) = on_grid((width, height), point);
    let (column, row) = (column.clamp(0.0, (width - 1) as f64), row.clamp(0.0, (height - 1) as f64));
    let (i, j) = (column.floor() as usize, row.floor() as usize);
    let (next_i, next_j) = ((i + 1).min(width - 1), (j + 1).min(height - 1));
    let (across, down) = (column - i as f64, row - j as f64);
    let value = |i: usize, j: usize| f64::from(heights.data[j * width + i]);
    let top = value(i, j) + (value(next_i, j) - value(i, j)) * across;
    let bottom = value(i, next_j) + (value(next_i, next_j) - value(i, next_j)) * across;
    (top + (bottom - top) * down) as f32
}

/// Cuts each channel's bed and banks into `heights`, and raises `depth` to the water's depth
/// wherever a channel's water reaches; both grids are laid out as `profiles` describes, `shapes`
/// and `profiles` are those of `channels`, and each channel's widths are its widths after the
/// wander.
///
/// Along each stretch between two points of a channel next to each other, the width, the deep
/// line's shift (`Shape::deep_shift_m`), the bed under the deep line (`water_fill` of the bed's
/// depth below the surface) and the surface are linear. For a cell and a channel, take the cell's
/// nearest point of the channel's line (`for_each_near`; each of several points equally near) and
/// there `h`, half the width and no less than `HALF_WIDTH_MIN_M`, `s` the deep line's shift and `D`
/// the water's depth at the deep line (the surface less the bed); and `t` the cell's signed
/// distance across the line, its distance to that point, positive on the side `(dz, -dx)` of the
/// stretch's direction `(dx, dz)`, as `s` is. Within the waterlines, `|t| <= h`, the cell is lowered
/// to the surface less `D` times `1 - u^CROSS_SECTION_POWER`: a bowl, with `u` running from 0 at the
/// deep line to 1 at either waterline (`bowl_share`). For `s` of 0 or more, `u` is `(t - s) / (h -
/// s)` from the deep line out on the side the shift points to (`t >= s`), and `(s - t) / (h + s)`
/// from it out on the other; for a negative `s`, the same with `t` and `s` of the other sign.
/// Beyond, the bank starts at the surface: its target is the surface plus `(|t| - h) / bank_run`,
/// and the cell is lowered by the cut that would make (its height less the target, when above 0)
/// times `fade(|t| - h, BANK_REACH_M)`: all of it at the waterline, fading smoothly to none
/// `BANK_REACH_M` beyond it, so the bank leaves no step there. A cell is never raised. Each
/// lowering is worked out from the heights as they were, and a cell that several channels reach
/// takes the lowest. The lakes' rims (`lakes`, as `profiles` reads them) are held before any
/// channel is cut (`hold_rims`); the channels are taken in the order `profiles` works them out in,
/// each after the one it ends on, and each holds the foot of its banks once it is cut
/// (`hold_foot`), as `profiles` describes: the bank of a channel taken later lowers no held cell
/// below its hold, or below its height where that lies lower, while its bed is cut as freely as
/// any. So the ground cut is the copy `profiles` read, as the last channel worked out left it, cell
/// for cell. Then, on the
/// ground as cut, the water each channel holds, filled out from its
/// bed with the level it was reached at: the cells in its bed (`|t|` no more than `h`) whose ground
/// lies below its surface at their nearest point hold its water at that surface, whatever lies
/// beside them; from a cell it holds at a level, the water reaches each cell beside it across a
/// side within its banks' reach (`|t|` no more than `BANK_REACH_M` beyond `h`) at the lower of that
/// level and the surface at that cell's nearest point, when that lies above the cell's ground, and
/// that is the cell's level for the cells it reaches in turn. A cell reached at several levels
/// keeps the highest; the highest are taken first, by level and then by cell, so what the fill
/// gives does not depend on the order it finds the cells in. So water stands no higher than the
/// level it was reached through: a hollow the water reaches only from downstream holds it at the
/// level there, not at the higher surface beside the hollow's upper end. Each cell its water
/// reaches has its depth raised to its level less its height, so a cell several channels reach
/// keeps the deepest, and no other cell holds water. The water inside what
/// a channel ends on is not that channel's: a channel that ends on another marks no cell inside
/// that one's bed (no farther from its line than its half width, no less than `HALF_WIDTH_MIN_M`,
/// at the nearest point, as `for_each_near` finds the cells), and one that ends in a lake marks no
/// cell of that lake (one of `lakes`); its water neither starts in such a cell nor spreads into one.
/// That water is marked by the other's own pass, the lake's when the lakes are put back
/// (`add_water`). A channel that begins on dry land (`Profile::on_dry_land`) marks no cell
/// straight beyond its first point (whose nearest point is that point, and which lies before it
/// along the first stretch): no water flows back from a spring's head, though the ground behind it
/// may fall away, and those cells are no part of the foot its banks hold its surface below either
/// (`profiles`), so the head is cut no notch; the channels taken after it are held off them all
/// the same. A channel that flows out of a lake marks its cells as any other: it holds the lake's
/// level only along its stretch beside the lake (`profiles`), so a second course out of the lake,
/// level at the lake's level there, meets the first at the level too. The bed's water
/// reaches from waterline to waterline, and beyond only through ground below its surface joined to
/// the bed within the banks' reach: the banks hold the surface below every cell of their foot, read
/// on the ground as the channels worked out before have cut it (`profiles`), and the channels taken
/// later cut no cell of that foot lower but with a bed, so water goes past them where a later step
/// raised the surface above them, as where it backs up from a lake or a junction, or at a mouth,
/// where a tributary's bed is cut through its river's foot. A hollow beyond the foot fills
/// only where such ground joins it to the bed. Only the cells within reach of each stretch are
/// visited; the reach is clipped to the grid.
///
/// Returns each cell's kept level, one value per cell of `heights`: the highest surface of the
/// channels whose water starts in it (the cells each fill is seeded from, in its bed and below its
/// surface there, as single precision), NEG_INFINITY where none does. The water a fill carries
/// beyond those cells is spread water, which the ground then bounds (`water::bound_spread`).
pub(crate) fn cut_beds(
    heights: &mut Grid<f32>,
    depth: &mut Grid<f32>,
    channels: &[Channel],
    shapes: &[Shape],
    profiles: &[Profile],
    lakes: &[LakeShape],
    rules: &WaterRules,
) -> Vec<f32> {
    debug_assert_eq!((heights.width, heights.height), (depth.width, depth.height), "the depth lies on the heights' cells");
    debug_assert_eq!(channels.len(), profiles.len(), "one profile per channel");
    debug_assert_eq!(channels.len(), shapes.len(), "one shape per channel");
    let size = (heights.width, heights.height);
    let lake_of = |cell: usize| lake_of_cell(lakes, cell);
    // The cells inside the bed of each channel another ends on, in index order.
    let beds: Vec<Vec<usize>> = (0..channels.len())
        .map(|index| {
            let ended_on = channels.iter().any(|channel| channel.end == ChannelEnd::Channel(index));
            if ended_on { bed_cells(size, &channels[index]) } else { Vec::new() }
        })
        .collect();
    let inside_end = |channel: &Channel, cell: usize| match channel.end {
        ChannelEnd::Channel(joined) => beds.get(joined).is_some_and(|bed| bed.binary_search(&cell).is_ok()),
        ChannelEnd::Lake(lake) => lake_of(cell) == Some(lake),
        ChannelEnd::Edge => false,
    };
    // Every lowering, worked out from the heights as they were before any, so that two banks that
    // overlap do not cut twice, each bank held off the feet of the channels taken before; then the
    // lowest of each cell's is kept.
    let mut lowered: Vec<(usize, f32)> = Vec::new();
    let mut held = vec![f32::NEG_INFINITY; heights.data.len()];
    hold_rims(size, lakes, rules, &mut held);
    for index in ends_first(channels) {
        let (channel, profile) = (&channels[index], &profiles[index]);
        for_each_lowering(heights, &held, channel, &shapes[index], profile, rules, |cell, lowered_m| lowered.push((cell, lowered_m)));
        hold_foot(size, channel, profile, &mut held);
    }
    for (cell, lowered_m) in lowered {
        heights.data[cell] = heights.data[cell].min(lowered_m);
    }
    // The water over the ground as cut, so that a cell another channel's bank lowered later still
    // holds all its water: each channel's on its own, from its bed out.
    let mut kept_level = vec![f32::NEG_INFINITY; heights.data.len()];
    for ((channel, shape), profile) in channels.iter().zip(shapes).zip(profiles) {
        let reach = reach_of(size, channel, shape, profile, rules, false, |cell| inside_end(channel, cell));
        // Whether water at `level_m` stands above the ground at `place`.
        let holds = |place: usize, level_m: f64| ((level_m - f64::from(heights.data[reach[place].0])) as f32) > 0.0;
        // The level each cell's water stands at, the highest of those it is reached at: its own
        // surface in the bed, and beyond, the lower of its own surface and the level of the cell it
        // is reached from. The highest levels are taken first.
        let mut level_m = vec![f64::NEG_INFINITY; reach.len()];
        let mut waiting = BinaryHeap::new();
        for (place, &(cell, surface_m, in_bed)) in reach.iter().enumerate() {
            if starts_water(surface_m, in_bed, heights.data[cell]) {
                level_m[place] = surface_m;
                waiting.push(Reached { level_m: surface_m, place });
                kept_level[cell] = kept_level[cell].max(surface_m as f32);
            }
        }
        while let Some(Reached { level_m: reached_m, place }) = waiting.pop() {
            if reached_m < level_m[place] {
                continue;
            }
            for cell in beside(reach[place].0, size) {
                if let Ok(next) = reach.binary_search_by_key(&cell, |&(cell, _, _)| cell) {
                    let carried_m = reach[next].1.min(reached_m);
                    if carried_m > level_m[next] && holds(next, carried_m) {
                        level_m[next] = carried_m;
                        waiting.push(Reached { level_m: carried_m, place: next });
                    }
                }
            }
        }
        for (&(cell, _, _), &level_m) in reach.iter().zip(&level_m).filter(|(_, &level_m)| level_m > f64::NEG_INFINITY) {
            let water_m = (level_m - f64::from(heights.data[cell])) as f32;
            if water_m > depth.data[cell] {
                depth.data[cell] = water_m;
            }
        }
    }
    kept_level
}

/// The cells within reach of `channel`'s banks (no more than `BANK_REACH_M` beyond its waterline at
/// their nearest point), on a grid of `size` (its width and height) laid out as `profiles`
/// describes, `shape` and `profile` being the channel's, each once and in index order: the surface
/// at its nearest point, the highest of several points as near, NEG_INFINITY where the cell lies
/// inside what the channel ends on (`inside_end`) or, for a channel that begins on dry land, at a
/// point it lies straight beyond the first point from; and whether it lies in the bed (no farther
/// from the line than the half width, no less than `HALF_WIDTH_MIN_M`) at one of those points.
/// `cut_beds` fills a channel's water out over these cells, and its water starts in those
/// `starts_water` takes.
///
/// With `bed_only`, only the cells no farther from the line than its widest half width, among them
/// every cell of its bed, each with the values it has in the whole reach: every point as near such
/// a cell lies no farther from it than that half width, so a walk of the cells that near finds
/// them all (`for_each_near_walking`). That walk costs what the bed's does, not the reach's.
#[allow(clippy::too_many_arguments)]
fn reach_of(size: (usize, usize), channel: &Channel, shape: &Shape, profile: &Profile, rules: &WaterRules, bed_only: bool, inside_end: impl Fn(usize) -> bool) -> Vec<(usize, f64, bool)> {
    let mut reach: Vec<(usize, f64, bool)> = Vec::new();
    let walk_m = if bed_only { 0.0 } else { BANK_REACH_M };
    for_each_near_walking(size, channel, shape, profile, rules, (walk_m, BANK_REACH_M), |cell, seen| {
        let none = inside_end(cell) || (profile.on_dry_land && seen.beyond_first);
        let surface_m = if none { f64::NEG_INFINITY } else { seen.surface_m };
        let in_bed = seen.distance_m <= seen.half_width_m;
        match reach.last_mut() {
            Some(last) if last.0 == cell => *last = (cell, last.1.max(surface_m), last.2 || in_bed),
            _ => reach.push((cell, surface_m, in_bed)),
        }
    });
    reach
}

/// Whether a channel's water starts in a cell of its reach (`reach_of`) whose surface there is
/// `surface_m` and which lies in its bed when `in_bed` holds: in the bed, its ground (`ground_m`)
/// below that surface, as single precision has it. `cut_beds` seeds each channel's fill from those
/// cells, and `KeptBeds::record` records them.
fn starts_water(surface_m: f64, in_bed: bool, ground_m: f32) -> bool {
    in_bed && ((surface_m - f64::from(ground_m)) as f32) > 0.0
}

/// A cell a channel's water reaches in `cut_beds`, by its place in the channel's reach (the cells in
/// index order), and the level the water reaches it at, in metres: ordered by the level, then by the
/// place, so that the fill takes the highest first, and the same one first whatever order the cells
/// were reached in.
#[derive(Debug, Clone, Copy)]
struct Reached {
    level_m: f64,
    place: usize,
}

impl Ord for Reached {
    fn cmp(&self, other: &Self) -> Ordering {
        self.level_m.total_cmp(&other.level_m).then(self.place.cmp(&other.place))
    }
}

impl PartialOrd for Reached {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Reached {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Reached {}

/// Calls `lower` with each cell of `heights` (laid out as `profiles` describes) that `channel`'s
/// bed and banks lower, and the height they lower it to, worked out from `heights` as they are, as
/// `cut_beds` describes it: within the waterlines the bowl, beyond them the bank, which lowers no
/// cell below what `held` holds it at, one value per cell of `heights` (`hold_foot`), nor below
/// its height where that lies lower; `shape` and `profile` are the channel's.
fn for_each_lowering(
    heights: &Grid<f32>,
    held: &[f32],
    channel: &Channel,
    shape: &Shape,
    profile: &Profile,
    rules: &WaterRules,
    mut lower: impl FnMut(usize, f32),
) {
    debug_assert_eq!(held.len(), heights.data.len(), "one hold per cell");
    let bank_run = f64::from(rules.bank_run);
    for_each_near((heights.width, heights.height), channel, shape, profile, rules, BANK_REACH_M, |cell, seen| {
        let was_m = f64::from(heights.data[cell]);
        let beyond_m = seen.distance_m - seen.half_width_m;
        let lowered_m = if beyond_m <= 0.0 {
            let deep_m = seen.surface_m - seen.bed_m;
            was_m.min(seen.surface_m - deep_m * bowl_share(seen.across_m, seen.half_width_m, seen.deep_shift_m))
        } else {
            let target_m = seen.surface_m + beyond_m / bank_run;
            let cut_m = was_m - (was_m - target_m).max(0.0) * fade(beyond_m, BANK_REACH_M);
            cut_m.max(f64::from(held[cell]).min(was_m))
        };
        if (lowered_m as f32) < heights.data[cell] {
            lower(cell, lowered_m as f32);
        }
    });
}

/// Raises `held`, one value per cell of a grid of `size` (its width and height) laid out as
/// `profiles` describes, at each cell of the foot of `channel`'s banks to the surface of its water
/// (`profile`) at the cell's nearest point of its line, linear along the stretch, plus
/// `FREEBOARD_M`: the cells beyond the waterline by no more than `BANK_FOOT_M` there, as
/// `bank_falls` reads them, none straight beyond its last point; and, for a channel that begins on
/// dry land (`Profile::on_dry_land`), those as far beyond it straight behind its first point too,
/// held at the surface there.
fn hold_foot(size: (usize, usize), channel: &Channel, profile: &Profile, held: &mut [f32]) {
    debug_assert_eq!(held.len(), size.0 * size.1, "one hold per cell");
    for_each_nearest(size, channel, BANK_FOOT_M, |cell, near| {
        if near.distance_m > near.half_width_m && (!near.beyond_first || profile.on_dry_land) && !near.beyond_last {
            let k = near.first;
            let (from_m, to_m) = (f64::from(profile.surface_m[k]), f64::from(profile.surface_m[k + 1]));
            let hold_m = (from_m + (to_m - from_m) * near.share + f64::from(FREEBOARD_M)) as f32;
            held[cell] = held[cell].max(hold_m);
        }
    });
}

/// Raises `held`, one value per cell of a grid of `size` (its width and height) laid out as
/// `profiles` describes, at each lake's rim, falling as a bank does, as `profiles` describes it: of
/// `lakes`, each lake's water and held bed are the sources of a `Spread` out to `RIM_REACH_M`, and
/// each cell `d` metres from them, holding no lake's water and lying in no lake's held bed, is held
/// at the lake's level plus `FREEBOARD_M` less `max(0, d - BANK_FOOT_M) / bank_run`; a cell several
/// lakes hold is held at the highest. The spread runs over the box round the lake's sources grown
/// by the reach: every cell within reach of one lies in it, so the distances are those of a spread
/// over the whole grid. Sequential and deterministic.
fn hold_rims((width, height): (usize, usize), lakes: &[LakeShape], rules: &WaterRules, held: &mut [f32]) {
    debug_assert_eq!(held.len(), width * height, "one hold per cell");
    let bank_run = f64::from(rules.bank_run);
    let beyond = RIM_REACH_M.ceil() as usize;
    let in_any = |cell: usize| lakes.iter().any(|lake| lake.cells.binary_search(&cell).is_ok() || lake.held_bed.binary_search(&cell).is_ok());
    for lake in lakes {
        let sources = || lake.cells.iter().chain(&lake.held_bed).copied();
        let Some(((first_column, last_column), (first_row, last_row))) = sources().fold(None, |bounds, cell| {
            let (column, row) = (cell % width, cell / width);
            Some(match bounds {
                None => ((column, column), (row, row)),
                Some(((left, right), (top, bottom))) => ((column.min(left), column.max(right)), (row.min(top), row.max(bottom))),
            })
        }) else {
            continue;
        };
        let (left, top) = (first_column.saturating_sub(beyond), first_row.saturating_sub(beyond));
        let (right, bottom) = ((last_column + beyond).min(width - 1), (last_row + beyond).min(height - 1));
        let (patch_width, patch_height) = (right - left + 1, bottom - top + 1);
        let mut is_source = vec![false; patch_width * patch_height];
        for cell in sources() {
            is_source[(cell / width - top) * patch_width + cell % width - left] = true;
        }
        let spread = Spread::new(patch_width, patch_height, &is_source, RIM_REACH_M);
        let level_m = f64::from(lake.level_m) + f64::from(FREEBOARD_M);
        for local in (0..patch_width * patch_height).filter(|&local| !is_source[local]) {
            let distance_m = spread.distance_m(local);
            let cell = (top + local / patch_width) * width + left + local % patch_width;
            if distance_m.is_finite() && !in_any(cell) {
                let hold_m = (level_m - (distance_m - BANK_FOOT_M).max(0.0) / bank_run) as f32;
                held[cell] = held[cell].max(hold_m);
            }
        }
    }
}

/// The cells beside `cell` across its sides, on a grid of `size` (its width and height).
fn beside(cell: usize, (width, height): (usize, usize)) -> impl Iterator<Item = usize> {
    let (i, j) = (cell % width, cell / width);
    [(i > 0).then(|| cell - 1), (i + 1 < width).then(|| cell + 1), (j > 0).then(|| cell - width), (j + 1 < height).then(|| cell + width)]
        .into_iter()
        .flatten()
}

/// The share of the water's depth at the deep line that lies over a cell `across_m` from the line,
/// signed as `Shape::deep_shift_m` is, inside waterlines `half_width_m` either side of it, the deep
/// line `shift_m` off the line: `1 - u^CROSS_SECTION_POWER`, as `cut_beds` describes it. 1 at the
/// deep line, 0 at both waterlines and beyond them. The shift lies well inside the half width
/// (`shape_channels` keeps it within half of it).
fn bowl_share(across_m: f64, half_width_m: f64, shift_m: f64) -> f64 {
    debug_assert!(shift_m.abs() < half_width_m, "the deep line {shift_m} m off lies outside the waterlines {half_width_m} m off");
    // Seen from the side the shift points to, where the cell's distance across is positive.
    let (across_m, shift_m) = if shift_m < 0.0 { (-across_m, -shift_m) } else { (across_m, shift_m) };
    let u = if across_m >= shift_m { (across_m - shift_m) / (half_width_m - shift_m) } else { (shift_m - across_m) / (half_width_m + shift_m) };
    1.0 - u.clamp(0.0, 1.0).powf(CROSS_SECTION_POWER)
}

/// Where a cell lies against a channel's line (`for_each_nearest`): its distance to the line's
/// nearest point, signed and not, the water's half width there, no less than `HALF_WIDTH_MIN_M`,
/// in metres, and where that point lies: on the stretch from point `first` to the next, `share`
/// of the way along it.
#[derive(Debug, Clone, Copy)]
struct Near {
    distance_m: f64,
    /// The cell's distance, signed as `Seen::across_m` is.
    across_m: f64,
    half_width_m: f64,
    first: usize,
    share: f64,
    /// Whether the cell lies straight beyond the line's first point: its nearest point is that
    /// point, and it lies before it along the line's first stretch.
    beyond_first: bool,
    /// Whether the cell lies straight beyond the line's last point: its nearest point is that
    /// point, and it lies past it along the line's last stretch.
    beyond_last: bool,
}

/// What a cell sees of a stretch of a channel: its distance to the stretch's nearest point, and
/// the water's half width, the deep line's shift, the bed under the deep line and the surface at
/// that point, in metres.
#[derive(Debug, Clone, Copy)]
struct Seen {
    distance_m: f64,
    /// The cell's distance, signed: positive on the side `(dz, -dx)` of the stretch's direction
    /// `(dx, dz)`, as `Shape::deep_shift_m` is, and negative on the other.
    across_m: f64,
    half_width_m: f64,
    deep_shift_m: f64,
    bed_m: f64,
    surface_m: f64,
    /// Whether the cell lies straight beyond the line's first point (`Near::beyond_first`).
    beyond_first: bool,
}

/// A stretch of a channel's line, between two points of it next to each other, in grid
/// coordinates.
#[derive(Debug, Clone, Copy)]
struct Stretch {
    /// The index of its first point; the other is the next.
    first: usize,
    from: (f64, f64),
    to: (f64, f64),
    /// From its first point to its last.
    along: (f64, f64),
    /// The square of its length, above 0.
    squared: f64,
}

impl Stretch {
    /// Where `cell` lies along the stretch's line, unbounded: below 0 before its first point, above
    /// 1 past its last.
    fn along_share(&self, cell: (f64, f64)) -> f64 {
        ((cell.0 - self.from.0) * self.along.0 + (cell.1 - self.from.1) * self.along.1) / self.squared
    }

    /// Where the stretch's point nearest `cell` lies along it (0 at its first point, 1 at its
    /// last), and how far it lies from `cell`. An end is taken exactly as it is, so two stretches
    /// that meet there find it equally far.
    fn nearest(&self, cell: (f64, f64)) -> (f64, f64) {
        let share = self.along_share(cell).clamp(0.0, 1.0);
        let point = if share <= 0.0 {
            self.from
        } else if share >= 1.0 {
            self.to
        } else {
            (self.from.0 + share * self.along.0, self.from.1 + share * self.along.1)
        };
        (share, (cell.0 - point.0).hypot(cell.1 - point.1))
    }

    /// Which side of the stretch's line `cell` lies on: above 0 on the side `(dz, -dx)` of its
    /// direction `(dx, dz)`, below 0 on the other, 0 on the line.
    fn side(&self, cell: (f64, f64)) -> f64 {
        (cell.0 - self.from.0) * self.along.1 - (cell.1 - self.from.1) * self.along.0
    }
}

/// Calls `visit` with the index of each cell of a grid of `size` (its width and height), laid out
/// as `profiles` describes, whose nearest point of `channel`'s line lies no more than `beyond_m`
/// beyond the waterline there, and with what it sees at that point (`channel`'s shape being `shape`
/// and its water `profile`); the half width it sees is the channel's, no less than
/// `HALF_WIDTH_MIN_M`, and its distance is signed by the side of the stretch it lies on, the side
/// `(dz, -dx)` of the stretch's direction `(dx, dz)` positive (a cell on the line, or straight
/// beyond an end, counts as on that side). Cells are visited once, in the order of their index, or
/// once for each of several points that lie equally near. A stretch of no length is passed over.
///
/// Each stretch walks the cells within reach of it (the channel's widest half width and
/// `beyond_m`) and keeps those for which it lies no farther than the stretches either side of it,
/// so that only the points of the line nearest a cell locally are kept, one or two for most cells;
/// of those, the nearest is the cell's. No grid of the cells' nearest stretches is kept. Where a
/// channel comes back within reach of itself, a cell sees only the nearer pass, so the banks of the
/// two passes meet halfway between them, with a step there when their beds differ.
fn for_each_near(
    size: (usize, usize),
    channel: &Channel,
    shape: &Shape,
    profile: &Profile,
    rules: &WaterRules,
    beyond_m: f64,
    visit: impl FnMut(usize, Seen),
) {
    for_each_near_walking(size, channel, shape, profile, rules, (beyond_m, beyond_m), visit);
}

/// `for_each_near` with the walk and the cells visited apart (`for_each_nearest_walking`): `walk_m`
/// and `beyond_m` of `(walk_m, beyond_m)`.
fn for_each_near_walking(
    size: (usize, usize),
    channel: &Channel,
    shape: &Shape,
    profile: &Profile,
    rules: &WaterRules,
    (walk_m, beyond_m): (f64, f64),
    mut visit: impl FnMut(usize, Seen),
) {
    for_each_nearest_walking(size, channel, (walk_m, beyond_m), |cell, near| {
        let k = near.first;
        let between = |a: f64, b: f64| a + (b - a) * near.share;
        let deep_shift_m = between(f64::from(shape.deep_shift_m[k]), f64::from(shape.deep_shift_m[k + 1]));
        let bed_m = between(f64::from(profile.bed_m(k, rules)), f64::from(profile.bed_m(k + 1, rules)));
        let surface_m = between(f64::from(profile.surface_m[k]), f64::from(profile.surface_m[k + 1]));
        let Near { distance_m, across_m, half_width_m, beyond_first, .. } = near;
        visit(cell, Seen { distance_m, across_m, half_width_m, deep_shift_m, bed_m, surface_m, beyond_first });
    });
}

/// `for_each_near` before anything of the channel's shape or water is read: calls `visit` with the
/// index of each cell whose nearest point of `channel`'s line lies no more than `beyond_m` beyond
/// the waterline there, and where it lies against the line (`Near`), in the same order, as many
/// times and with the same half width and the same signed distance.
fn for_each_nearest(size: (usize, usize), channel: &Channel, beyond_m: f64, visit: impl FnMut(usize, Near)) {
    for_each_nearest_walking(size, channel, (beyond_m, beyond_m), visit);
}

/// `for_each_nearest`, each stretch walking the cells within the channel's widest half width and
/// `walk_m` of it, and visiting each cell of those whose nearest points it visits as
/// `for_each_nearest` with `beyond_m` does: those no more than `beyond_m` beyond the waterline. With
/// a `walk_m` no greater than `beyond_m`, a cell no farther from the line than the walk reaches is
/// visited exactly as `for_each_nearest` visits it, as every point as near it lies within the walk
/// too, and a cell farther out is not visited.
fn for_each_nearest_walking((width, height): (usize, usize), channel: &Channel, (walk_m, beyond_m): (f64, f64), mut visit: impl FnMut(usize, Near)) {
    let at = |k: usize| on_grid((width, height), &channel.points[k]);
    let stretches: Vec<Stretch> = (1..channel.points.len())
        .map(|k| {
            let (from, to) = (at(k - 1), at(k));
            let along = (to.0 - from.0, to.1 - from.1);
            Stretch { first: k - 1, from, to, along, squared: along.0 * along.0 + along.1 * along.1 }
        })
        .filter(|stretch| stretch.squared > 0.0)
        .collect();
    let half_width_m = |k: usize| (f64::from(channel.points[k].width_m) / 2.0).max(HALF_WIDTH_MIN_M);
    let reach_m = (0..channel.points.len()).map(half_width_m).fold(0.0, f64::max) + walk_m;
    // Each cell, the distance to its stretch's nearest point, the stretch's place and the share
    // along it, for every stretch that lies no farther from the cell than those either side of it.
    let mut kept: Vec<(usize, f64, usize, f64)> = Vec::new();
    for (place, stretch) in stretches.iter().enumerate() {
        let (before, after) = (place.checked_sub(1).map(|before| &stretches[before]), stretches.get(place + 1));
        let (a, b) = (stretch.from, stretch.to);
        let cells = |from: f64, to: f64, count: usize| {
            let low = (from.min(to) - reach_m).ceil().max(0.0);
            let high = (from.max(to) + reach_m).floor().min(count as f64 - 1.0);
            (low <= high).then_some(low as usize..=high as usize)
        };
        let (Some(columns), Some(rows)) = (cells(a.0, b.0, width), cells(a.1, b.1, height)) else { continue };
        for j in rows {
            for i in columns.clone() {
                let cell = (i as f64, j as f64);
                let (share, distance_m) = stretch.nearest(cell);
                let nearer = |other: Option<&Stretch>| other.is_some_and(|other| other.nearest(cell).1 < distance_m);
                if distance_m <= reach_m && !nearer(before) && !nearer(after) {
                    kept.push((j * width + i, distance_m, place, share));
                }
            }
        }
    }
    kept.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    for of_cell in kept.chunk_by(|a, b| a.0 == b.0) {
        let nearest_m = of_cell[0].1;
        for &(cell, distance_m, place, share) in of_cell.iter().take_while(|kept| kept.1 == nearest_m) {
            let stretch = &stretches[place];
            let k = stretch.first;
            let between = |a: f64, b: f64| a + (b - a) * share;
            let half_width_m = between(half_width_m(k), half_width_m(k + 1));
            if distance_m - half_width_m > beyond_m {
                continue;
            }
            let cell_xy = ((cell % width) as f64, (cell / width) as f64);
            let across_m = if stretch.side(cell_xy) < 0.0 { -distance_m } else { distance_m };
            let along = stretch.along_share(cell_xy);
            let (beyond_first, beyond_last) = (place == 0 && along < 0.0, place + 1 == stretches.len() && along > 1.0);
            visit(cell, Near { distance_m, across_m, half_width_m, first: k, share, beyond_first, beyond_last });
        }
    }
}

#[cfg(test)]
mod tests;
