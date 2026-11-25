//! The joined water: the rivers, brooks and lakes of an edited land and its `water` layer, the
//! water of the land it kept joined inside the edit's outline to the water of the world the fine
//! steps made from the edit's mixed model (steps P11 and P12 of the editor tool's prototype,
//! `report2.md`, "The final mechanism", with the design's amendments 7 and 8), after the ground is
//! joined (`join_ground`).
//!
//! A lake is the kept land's where its middle, or all of its water, lies outside the outline (a
//! lake held whole), and the new world's where its middle lies inside, where the outline holds it
//! whole. A course that crosses the outline is one course: the kept land's runs of it outside the
//! outline and its partner's runs inside, in the kept course's order, its points in the band moved
//! from the kept line toward the new one by the weight, and mended so that going downstream no new
//! point stands above the point before it; a course with no partner keeps its runs on its own side,
//! each a course of its own, so no course bridges the outline. The band's water is made from the
//! joined courses' beds, by the fine steps' rule that a course holds no water inside the bed of the
//! one it ends on, and from the water either world holds there, bounded by the joined ground as the
//! fine steps bound the water beyond a bed (`bound_spread`), a lake's cells kept as the fine steps
//! keep them; where a cell, its ground and the course beside it are as both worlds hold them, the
//! cell keeps the water both hold, and so does a cell off the beds whose ground and water both
//! worlds hold alike at a lake's level, so a world joined with itself is that world, bit for bit, by
//! any weight. Inside the outline the water stands no higher than a course the mending lowered, but a
//! lake's water, which keeps its level; and the band's ground takes the engine's own rules for
//! ground under water and wet ground beside it from that water.
//!
//! `new_unheld` counts, by place, the water the ground does not hold that an edited land has and
//! neither the kept land nor the edited drawing grown whole has at that place: a measure that
//! changes nothing.

use crate::cover::{ground_under_water, LAYER_NAMES};
use crate::drawing::{Drawing, Lake};
use crate::grid::Grid;
use crate::grow::{smoothed_line, GROW_CELL_M};
use crate::held::{unheld_cells, UnheldCells, LAKE_LEVEL_WITHIN_M};
use crate::land::outline::{lake_partners, lake_water, to_segment, BAND_M};
use crate::water::{bank_wetness, bound_spread, CourseEnd, CoursePoint, LakeWater, RiverCourse, Water, HALF_WIDTH_MIN_M, WATER_LAYER};
use crate::water_rules::WaterRules;
use crate::world::World;
use std::collections::{BTreeSet, HashSet};

/// A brook of the new world and one of the kept land, both touching the outline, are one brook
/// when each, at its median point in the band, lies within the wider one's width (each brook's
/// widest) and this many metres more of the other's line (amendment 7, the prototype's test), and
/// the new one's head, where it lies in the band, lies within that and `HEAD_SLACK_M` more of the
/// kept one's line. The join drops the kept brook's band points and moves the new one's toward the
/// kept line, so only a new head far from that line can put the joined course where neither land
/// has a valley: the median alone joined two brooks whose new head lies 46.6 m from the kept line,
/// which left 21 cells of water the ground does not hold in the band (the example's brook point
/// moved 150 m, mountains, seed 23). Asked of every band point, as the plan first said, the test
/// left apart brooks the median joins cleanly, that swing apart mid-course or start a cell apart
/// (17 such cells on the example, 116 on the trials); asked of the kept brook's head too, it left
/// apart brooks whose kept head lies further up the same valley, which goes with the kept brook's
/// dropped points (274 such cells on the 27 runs).
pub const BROOK_PAIR_M: f64 = 8.0;
/// A brook's head is found on the model's cells, so it is uncertain by one cell's diagonal: a new
/// brook's head in the band may lie this many metres further from the kept brook's line than the
/// pairing allows at the brooks' middles.
pub const HEAD_SLACK_M: f64 = GROW_CELL_M as f64 * std::f64::consts::SQRT_2;
/// A band cell whose middle lies within half the water's width (at least `HALF_WIDTH_MIN_M`) and
/// this many metres more of a joined course's line is on its bed, and keeps that course's surface
/// there (P12).
pub const BED_MARGIN_M: f64 = 0.5;
/// A band cell whose nearest course within half the water's width and this many metres more is
/// one the join dropped takes none of that world's water: a course the join dropped takes its
/// water with it (P12). And a wet cell inside the outline whose nearest joined course within that
/// reach lies there on a stretch the mending lowered stands no higher than that course
/// (`join_water`, **The `water` layer**).
pub const DROPPED_REACH_M: f64 = 12.0;
/// A joined course whose surface would rise downstream across the outline by more than this, in
/// metres, before it is mended (the design's bound for a crossing, "Water across the outline") is
/// counted in `WaterJoin::rises_over_limit`, and the edit's notes say where; the mending lowers it
/// either way.
pub const RISE_LIMIT_M: f32 = 0.1;
/// Two drawn rivers, one of each drawing, that are not equal are one river, moved or changed by the
/// edit, only when their smoothed lines lie no farther apart than this on the mean, in metres
/// (`river_partners`): the band, across which the join moves a course's new points onto its kept
/// line by the weight. Lines farther apart than the band on the mean are two rivers, the kept one's
/// courses ending at the outline and the new one's narrowing there. Measured: the example's edits
/// move a river's line at most 53.9 m on the mean (its brook's point moved 150 m, 52.7 m at
/// 2,048 m), the trials' standard edits at most 20.0 m; two rivers of one drawing lie 206 m or more
/// apart on the mean on the example and the trials, but `parallel`'s two, 25.6 m apart.
pub const RIVER_PARTNER_REACH_M: f64 = BAND_M as f64;

/// A new lake's backwater (`backwater_cells`; stage 2's ruling R23): the join lowering a point of a
/// drawn course by more than this, in metres, where it moved one at the band's inner edge, says
/// the new world's river stands above the kept one where it meets it, and the kept river's points
/// over which the edit's world stands more than this above them are its backwater. `RISE_LIMIT_M`, the rise the join
/// says. Measured: on the three sets the join mends a drawn course down at weight 1 by at most
/// 0.030 m (the example in gentle country from seed 23, edit `c`) and the new world's river stands
/// at most 0.047 m above the kept one beyond the outline (`wide` and `thin`, within 24 m of where
/// they enter it), on the 40 lake edits at most 0.009 m; a lake added on the example's main river
/// whose course into it crosses the outline mends it down by 0.30 to 1.58 m and stands 0.33 to
/// 1.59 m above the kept river beyond the outline, and its lake moved upstream 0.16 m.
pub const BACKWATER_ABOVE_M: f32 = RISE_LIMIT_M;

/// The courses are found near a place through buckets of this many metres a side.
const BUCKET_M: f64 = 32.0;

/// One course of the kept land joined across the outline with its partner in the new world, and
/// what joining did to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CourseJoin {
    /// Its index in the joined land's `Water::rivers`: the course that holds the partner's points,
    /// where the join leaves runs of the kept course apart as courses of their own.
    pub course: usize,
    /// The kept land's course and the new world's, by their indices in their worlds; for a run of
    /// a kept course cut at a lake (`join_water`, **Partners**), the course it was cut from.
    pub old: usize,
    pub new: usize,
    /// The drawn river it is a course of, by its place in the edited drawing; `None` for a brook.
    pub drawn: Option<usize>,
    /// The most its surface rose from one point to the next downstream before it was mended, and
    /// after, in metres; 0 when it never rose.
    pub rise_before_m: f32,
    pub rise_m: f32,
    /// World `(x, z)` of where it rose most before it was mended, the middle of the two points, in
    /// metres; `None` when it never rose.
    pub rise_at: Option<(f64, f64)>,
    /// The most the mending lowered a new point, and the most it raised one, in metres; and how
    /// many of the points it moved lie inside the band's inner edge, where the weight is 1.
    pub lowered_most_m: f32,
    pub raised_most_m: f32,
    pub mended_at_weight_1: usize,
    /// The largest step in width where a kept point and a new one follow each other, in metres, and
    /// the kept course's own largest step between neighbouring points.
    pub width_step_m: f32,
    pub width_step_own_m: f32,
    /// The longest stretch between a kept point and a new one that follow each other, in metres.
    pub gap_m: f32,
}

/// What joining the water did (`join_water`): for the edit's notes and its counts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaterJoin {
    /// How many courses of the kept land that cross the outline (a point outside it and one inside)
    /// were joined with a partner; `joins` holds them, and the courses of `joined_inside`. A course
    /// cut at a lake (`join_water`, **Partners**) is counted by its runs, here or in `joined_inside`.
    pub crossings: usize,
    /// How many courses of the kept land that lie wholly inside the outline (no point outside it)
    /// were joined with a partner; they are not counted in `crossings`.
    pub joined_inside: usize,
    /// The most the mending lowered a point of a joined course, in metres.
    pub lowered_most_m: f32,
    /// The most a joined course's surface rises from one point to the next downstream after the
    /// mending, in metres: 0, as the mending makes it (`join_water`, **Courses**).
    pub rise_most_m: f32,
    /// The most the mending raised a new point to the kept point after it, in metres.
    pub raised_most_m: f32,
    /// The largest step in width where a kept point and a new one follow each other on a joined
    /// course, in metres (`CourseJoin::width_step_m`).
    pub width_step_most_m: f32,
    /// World `(x, z)` of every place where a course of the new world with no partner crosses the
    /// outline, in the courses' order and each course's from upstream: the middle between its last
    /// point on one side and its first on the other; then, in the same way, every place where a run
    /// of a joined course's partner that the join leaves apart crosses it (`join_water`,
    /// **Courses**). Each such course or run is narrowed to nothing across the band. Mostly brooks,
    /// but a drawn river with no partner is listed too (a river the kept land has no course of
    /// there; none on the measured sets), so a note made from these places should not call each of
    /// them a brook.
    pub brooks_narrowed: Vec<(f64, f64)>,
    /// World `(x, z)` of every place where a course of the kept land with no partner crosses the
    /// outline, as `brooks_narrowed` gives them: such a course ends at the outline there, or starts
    /// there again where it comes back out of the outline. A drawn river with no partner is listed
    /// too.
    pub brooks_ended: Vec<(f64, f64)>,
    /// For each place of `brooks_narrowed`, in its order, the drawn river whose course it is, by its
    /// place in `new_drawing`; `None` for a brook.
    pub narrowed_rivers: Vec<Option<usize>>,
    /// For each place of `brooks_ended`, in its order, the drawn river whose course it is, by its
    /// place in `new_drawing` (the river of `old_drawing` it pairs with); `None` for a brook, and
    /// for a river `new_drawing` does not have, whose kept course the joined land holds as a brook.
    pub ended_rivers: Vec<Option<usize>>,
    /// The brooks that cross the outline: the kept land's and the new world's.
    pub brooks_crossing: (usize, usize),
    /// How many of the kept land's brooks that cross the outline were paired with a brook of the new
    /// world (`BROOK_PAIR_M`).
    pub brooks_paired: usize,
    /// Each joined course, in the kept land's order.
    pub joins: Vec<CourseJoin>,
    /// How many joined courses rose by more than `RISE_LIMIT_M` before they were mended (each one's
    /// `CourseJoin::rise_at` says where).
    pub rises_over_limit: usize,
    /// How many courses' ends named a course or lake the join left out, and end at the border
    /// instead.
    pub ends_lost: usize,
    /// For each end lost, in the order of the joined land's courses: world `(x, z)` of the course's
    /// last point, where it ends now, and the drawn river whose course it is, by its place in the
    /// edited drawing (`None` for a brook).
    pub ends_lost_at: Vec<(f64, f64)>,
    pub ends_lost_rivers: Vec<Option<usize>>,
    /// The band's cells whose wetness was raised to how wet the joined water makes them, and those
    /// whose ground became wet ground alone under the joined water (`join_water`).
    pub band_wetness_raised: usize,
    pub band_ground_made_wet: usize,
    /// The cells of weight 1 whose water was lowered to a mended course's surface (`join_water`,
    /// **The `water` layer**): the only cells of weight 1 whose water is not the new world's. And
    /// the most one was lowered, in metres.
    pub capped_at_weight_1: usize,
    pub capped_most_m: f32,
}

/// The water of an edited land (P11, P12): `joined` is `join_ground`'s world, the kept land's
/// ground joined to `new`'s by `weight` (`Outline::weight`) and still holding `old`'s water; `old`
/// is the world the land kept and `new` the world the fine steps made from the edit's mixed model,
/// both in the land model's frame; `old_drawing` and `new_drawing` are the drawings they were grown
/// from, and `water` the rules the fine steps ran with. Replaces `joined`'s rivers, lakes and
/// `water` layer, raises its band's wetness and wets its band's ground where the joined water asks
/// it, and says what joining did.
///
/// **Lakes.** A lake of `old` whose middle lies outside the outline (weight 0), or whose water
/// (`outline::lake_water`) lies wholly outside it, as a lake held whole does though its middle (the
/// mean of its cells' places) may lie on ground beside it that weighs more, is kept bit for bit,
/// its `drawn` naming its place in `new_drawing` (the lake of `new_drawing` at the same place with
/// the same radius; one with none, a lake taken away whose middle is held, which the edit refuses,
/// is left out); a lake of `new` whose middle lies inside is taken bit for bit. They are listed
/// in `new_drawing`'s order. The outline holds a lake whole, so its water lies wholly at weight 0
/// or wholly at weight 1, but beside held or hand-shaped ground, which the edit refuses, or round
/// a lake held whole, which lies wholly at weight 0.
///
/// **Partners.** A course touches the outline when one of its points lies at weight above 0
/// (each point reads the weight of the world cell it lies on). A course of `old` and one of `new`
/// that both touch it are partners: for a drawn river's courses, the same river (`river_partners`:
/// the river of `new_drawing` equal to `old_drawing`'s, or else the one whose line lies nearest
/// within `RIVER_PARTNER_REACH_M`) out of the same lake or of none, the n-th such of each in order;
/// then a drawn river's courses that no lake partner pairs (a lake moved or resized pairs with none,
/// `lake_partners`) by their order among the river's courses, from upstream, where the river has as
/// many courses in both worlds and, at every place in its chain, the two courses start from no
/// lake, from one lake (paired by `lake_partners`, or the same lake resized where it was), or from
/// two lakes neither of which is another's, so a moved or resized lake's outflow stays one course
/// across the outline (the design's W3; stage 2's ruling R9), and an outflow wholly inside the
/// outline is joined, not taken whole; a river whose count of courses changes (a lake added on it or
/// taken away), or whose lakes the edit reorders along it (stage 2's ruling R11, S1), is not paired
/// so; then a drawn river's kept course that runs on past a lake its partner ends in, where a
/// lake of `new` splits the river the kept land runs as one course, is cut at that lake: where the
/// partner ends in a lake the kept course does not end in, a course of the river out of that lake
/// has no partner, and the kept course's water reaches the lake's disc (a point no farther from the
/// disc than half the water's width there) on points inside the outline and the course leaves the
/// outline below it, its points from the first such to the last are left out, its run below joined
/// with the course out of the lake and its run above with the partner, or, where the run above
/// lies wholly inside the outline, left out with the partner taken as `new` has it (a run below is
/// cut again at the next such lake; stage 2's ruling R20); then two courses of one drawn river,
/// however paired, that meet nowhere at the outline (one or both cross it, and every place either
/// crosses it lies farther from the other's line than `RIVER_PARTNER_REACH_M`) are left apart, the
/// river's water gone two ways (stage 2's ruling R13); and last a kept course and a new one of a
/// drawn river, each with no partner, that both cross the outline at places within
/// `RIVER_PARTNER_REACH_M` of each other are paired, the two whose places lie nearest first (stage
/// 2's ruling R20); so a lake that splits a river leaves it one course on each side of the lake,
/// neither bridged from the lake to where the kept course leaves the outline nor cut off there
/// beside the course out of the lake (the design's W3);
/// for brooks, first a brook of `old` and one of `new` whose points are equal bit for bit, the first
/// such of each in order, wherever they lie (so a brook wholly inside the band's inner edge, which
/// has no point in the band, keeps its place among the courses where the edit leaves it as it was);
/// then of the brooks left, a brook of `old` and
/// one of `new` that each, at its median point in the band (weight above 0 and below 1), lie within
/// the wider brook's widest width and `BROOK_PAIR_M` of the other's line, and whose new brook's
/// head, where it lies in the band, lies within that and `HEAD_SLACK_M` more of the kept brook's
/// line (`BROOK_PAIR_M` says why), the pairs nearest by the mean of the two median distances taken
/// first.
///
/// **Courses.** A course of `old` that does not touch the outline is kept bit for bit; one cut at a
/// lake (**Partners**) is joined run by run, its runs in its place among the kept courses, from
/// upstream, and a course that ended on it ends on the run nearest its last point. One with a
/// partner is joined: its runs outside the outline (weight 0), bit for bit, and its partner's runs
/// inside (weight above 0), in its own order, the n-th run inside of each together; when the two
/// have other numbers of runs inside, the kept course's span inside (from its first run inside to
/// its last) is one run, its runs outside between left apart, each a course of its own, and the
/// partner's run inside whose first and last points lie nearest the span's (the sum of the two
/// distances) takes the span's place, its other runs inside left apart, each a course of its own
/// narrowed across the band as a course of `new` with no partner is, every place where they cross
/// the outline listed in `brooks_narrowed`. So a kept course that leaves the outline and comes
/// back, joined with a partner that does not, is not bridged across the outline, and none of a
/// partner's points outside the outline is taken, which the mix would lay on the kept line at
/// weight 0, running back up it; the joined course flows out of the partner's lake, or keeps its
/// end, where the kept course does not, only when that run holds the partner's first or last point.
/// Each of the partner's points in the band takes its place,
/// surface, width and depth as `o + w (n - o)`, `o` read at the nearest place on the kept course's
/// line (linear along the stretch), `n` its own, `w` its weight, worked in `f64` and rounded to
/// `f32`. Then it is mended, and a kept point is never changed: going downstream, a new point that
/// stands above the point before it is lowered to it; then, where a kept point follows new points
/// (the course leaving the outline) and stands above the last of them, which only lowering the kept
/// point could otherwise cure, the new points before it are raised to it, going upstream, up to the
/// first that stands at or above it (the kept point before the band stands above it, being
/// upstream on the same kept course). So the surface never rises downstream, across the outline
/// too; the mending may move a new point inside the band's inner edge as well
/// (`CourseJoin::mended_at_weight_1`), so there a course is the new world's but for that. A course of
/// `old` with no partner that has a point outside keeps its runs outside, each a course of its own
/// that ends at the outline, or starts there again where the course comes back out of it
/// (`brooks_ended`); one wholly inside is dropped. A course of `new` with no partner that touches
/// the outline keeps its runs inside, each a course of its own, each point's width and depth times
/// its weight, so it narrows to nothing across the band (`brooks_narrowed`), or is taken whole if it
/// lies wholly inside; one that does not touch it is dropped. So no course of the joined land
/// bridges the outline with a straight stretch. Of the courses one course is cut into, the one that
/// holds its first point flows out of its lake and the one that holds its last point keeps its end;
/// the others flow out of no lake and end at the border, and a course that ended on the course cut
/// ends on the one of them with a point nearest its own last point. The courses are listed drawn
/// rivers first, in `new_drawing`'s order, then the brooks, each kind in the order made (the kept
/// land's courses in its order, then the new world's), the courses cut from one course from
/// upstream, and their ends and lakes name the joined courses and lakes (a lake by its drawn lake,
/// the joined land's lake of that drawn lake from whichever world it takes it, so a course of one
/// world names a lake taken from the other); an end the join left out ends at the border
/// (`ends_lost`, each at its course's last point in `ends_lost_at`).
///
/// **The `water` layer.** A cell of weight 0 holds `old`'s water bit for bit, and one of weight 1
/// `new`'s, but along a mended course. *Along a mended course*: a wet cell of weight above 0 whose
/// nearest joined course within half the water's width (at least `HALF_WIDTH_MIN_M`) and
/// `DROPPED_REACH_M` lies there on a stretch one of whose points the forward pass of the mending
/// lowered stands no higher than that course's mended surface at that place (linear along the
/// stretch), and is dry where that lies at or below its ground; but a cell where `new` holds a
/// lake's water at its level (`outline::lake_water`, as `Land::hold` and the lakes' zones tell a
/// lake's water) keeps it, so a lake keeps its level and is found whole by it. Where that nearest
/// course ends on another (`CourseEnd::River`), the place on its last stretch lying nearer its last
/// point than the point before, the cell is as near the other course's point that last point lies
/// on, and stands no higher than the other course's mended surface there if the mending lowered it:
/// so the river's bed beside a tributary's mouth is not left at the river's unmended surface. So
/// the water stands no higher than a course the mending lowered, also past the band's inner edge,
/// and does not rise there going down it (`capped_at_weight_1` counts the cells of weight 1 this
/// lowers, the only ones whose water is not `new`'s). Where the mending raised points inside the
/// band's inner edge, the water there stays `new`'s, below the course. A band cell within half the
/// water's width and `BED_MARGIN_M` of a joined course's line is on its bed and holds that course's
/// surface there, at the nearest of those lines, where it stands above the cell's ground; but a
/// course that ends on another holds no water inside that one's bed (no farther from its line than
/// its half width, at least `HALF_WIDTH_MIN_M`), as the fine steps fill the beds (`add_water`), so
/// a cell there is on the nearest of the other beds, the one it ends on among them (stage 2's
/// ruling R4: the brook's surface, read along its last stretch, stood up to 0.197 m above the
/// river's in the river's bed at its mouth). Where the cell lies there on a stretch both worlds hold,
/// bit for bit, and both worlds and the joined land hold its ground alike and both worlds its water,
/// it holds that water itself, which the fine steps made from that stretch: worked out again from
/// the course's points, kept in single precision, it would move by single precision's step, and by
/// more where the fine steps fill a bed by their own rule (in its margin, and where two beds meet).
/// Off every bed, a band cell whose ground both worlds and the joined land hold alike, and its
/// water both worlds, holds that water and is kept, capped as above, where it stands within
/// `held::LAKE_LEVEL_WITHIN_M` of a joined lake's level, as a bed cell on a stretch both worlds
/// hold is: a lake's water spread off the beds beyond the reach `lake_water` gives the lake (up a
/// river's valley), which the fine steps bounded once; bounded again by rims worked out again in
/// single precision it would come down a few steps (stage 2's ruling R11: `lake3` at 1,024 m in
/// mountains from seed 3, a disc of 200 m regrown, 3,212 cells by at most 5.96e-8 m). Any other
/// band cell that `old` or `new` wets, unless its
/// nearest course in that world within its half width and `DROPPED_REACH_M` is one the join
/// dropped, holds the higher of the two worlds' surfaces there where that stands above its ground:
/// that world's own water where the joined ground is that world's ground, so its depth is not worked
/// out again from the surface. A band cell of a joined lake's water (`outline::lake_water`, in the
/// world the lake is taken from) is kept at the lake's level, or at its own surface where that
/// stands higher, as the fine steps keep a lake's cells. Then the fine steps' bound
/// (`bound_spread`) keeps every wet cell outside the band, every wet bed cell, every lake cell and
/// every cell kept so at its surface, and lets the rest stand only where the joined ground closes it in, no higher than
/// its rim, joined through water to a kept cell. So a world joined with itself (`old` and `new` one
/// world, with its drawing) is that world, bit for bit, by any weight: its courses, lakes and water
/// layer, and the band's wetness and ground taken from that water below.
///
/// **The band's ground.** On the band's cells alone, the engine's two rules for the ground beside
/// and under water are taken from the joined water, as the fine steps take them from a world's
/// own: a cell's wetness is no less than how wet the water makes it (`bank_wetness`, with `water`),
/// and a cell under more than 0.05 m of water is wet ground alone (`ground_under_water`). Cells of
/// weight 0 and 1 are untouched, so near the band they may break the first rule by the band's new
/// water.
///
/// Sequential, in an order that depends only on the worlds, so the same worlds give the same water
/// on any thread count.
///
/// # Panics
///
/// When the four grids are not of one size, or `old` or `new` holds no `water` layer: the edit makes
/// all of them from one land in the model's frame.
pub fn join_water(joined: &mut World, old: &World, new: &World, weight: &Grid<f32>, old_drawing: &Drawing, new_drawing: &Drawing, water: &WaterRules) -> WaterJoin {
    let shape = |grid: &Grid<f32>| (grid.width, grid.height);
    assert!(
        shape(&joined.heights) == shape(weight) && shape(&old.heights) == shape(weight) && shape(&new.heights) == shape(weight),
        "the joined land, the two worlds and the weight are of one size: {:?}, {:?}, {:?} and {:?} cells",
        shape(&joined.heights),
        shape(&old.heights),
        shape(&new.heights),
        shape(weight)
    );
    assert!(old.layers.contains_key(WATER_LAYER) && new.layers.contains_key(WATER_LAYER), "both worlds hold the {WATER_LAYER} layer");
    let lake_partner = lake_partners(old_drawing, new_drawing);
    let lakes = join_lakes(old, new, weight, old_drawing, new_drawing, &lake_partner);
    let partners = pair_courses(old, new, weight, old_drawing, new_drawing, &lake_partner);
    let mut report = WaterJoin { brooks_crossing: partners.brooks_crossing, brooks_paired: partners.brooks_paired, ..WaterJoin::default() };
    let made = what_is_made(new, weight, &partners, &mut report);
    let JoinedCourses { rivers, lowered } = join_courses(new, weight, &made, &partners, &lakes, &mut report);
    report.joined_inside = made.iter().filter(|how| matches!(how, Made::Joined(j, _) if !reaches_outside(&partners.kept[*j], weight))).count();
    report.crossings = report.joins.len() - report.joined_inside;
    for join in &report.joins {
        report.lowered_most_m = report.lowered_most_m.max(join.lowered_most_m);
        report.raised_most_m = report.raised_most_m.max(join.raised_most_m);
        report.rise_most_m = report.rise_most_m.max(join.rise_m);
        report.width_step_most_m = report.width_step_most_m.max(join.width_step_m);
        report.rises_over_limit += usize::from(join.rise_before_m > RISE_LIMIT_M);
    }
    // The cells of each joined lake's water in the world it is taken from, with its level.
    let lake_cells_of = |world: &World, drawing: &Drawing, to: &[Option<usize>]| -> Vec<(f32, Vec<usize>)> {
        (world.water.lakes.iter().zip(to))
            .filter(|(_, to)| to.is_some())
            .filter_map(|(lake, _)| Some((lake.level_m, lake_water(world, drawing.lakes.get(lake.drawn)?, lake.drawn))))
            .collect()
    };
    let mut lake_cells = lake_cells_of(old, old_drawing, &lakes.old_to);
    lake_cells.extend(lake_cells_of(new, new_drawing, &lakes.new_to));
    joined.water = Water { rivers, lakes: lakes.lakes };

    let old_dropped: Vec<bool> = (0..old.water.rivers.len()).map(|j| !made.iter().any(|made| made.old_course().map(|piece| partners.kept_from[piece]) == Some(j))).collect();
    let new_dropped: Vec<bool> = (0..new.water.rivers.len()).map(|k| !made.iter().any(|made| made.new_course() == Some(k))).collect();
    let depth = band_water(joined, old, new, weight, new_drawing, &lake_cells, &old_dropped, &new_dropped, &lowered, &mut report);
    (report.band_wetness_raised, report.band_ground_made_wet) = wet_the_band(joined, &depth, weight, water);
    joined.layers.insert(WATER_LAYER.to_string(), depth);
    report
}

/// What `backwater_cells` finds of a new lake's backwater that an edit's outline leaves out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Backwater {
    /// The world cells to take, sorted and distinct; none where there is none.
    pub cells: Vec<usize>,
    /// The river it holds up and the lake that holds it, where a kept course was walked: the
    /// first such course's drawn river, by its place in the edited drawing, and the lake of the
    /// edited drawing that the mixed world's river runs down into from the course's place where
    /// it enters the outline (following the courses' ends), if it reaches one.
    pub river: Option<usize>,
    pub lake: Option<usize>,
    /// Where fixed ground stops it, if it does: the first fixed world cell of the walks
    /// (`backwater_cells` says which).
    pub stopped: Option<usize>,
}

/// The backwater of an edit's new water that its outline leaves out (stage 2's ruling R23, the
/// design's "the outline follows that river ... as far as it must"). `old` is the kept land,
/// `mixed` the world the fine steps made from the edit's mixed model, `weight` the outline's
/// weight, `join` what joining the two did (`join_water`), `old_drawing` and `new_drawing` the
/// drawings they were grown from, and `fixed` whether a world cell is held or shaped by hand.
///
/// Where the join mended a drawn river's course with a point of weight 1 among those it moved
/// (`CourseJoin::mended_at_weight_1`) and lowered a point by more than `BACKWATER_ABOVE_M`
/// (`CourseJoin::lowered_most_m`), the new world's river stands above the kept one where it meets
/// it: a lake it flows into holds it up past the outline or across the band. Then, at each place
/// where the kept course of that join enters the outline (a point of weight 0 followed by one
/// above 0), the cells are taken of its point there, of its points in the band (weight below 1)
/// over which `mixed` stands more than `BACKWATER_ABOVE_M` above it, and of its points upstream of
/// that place, from the nearest, as long as `mixed` stands more than that above them. And a kept
/// course that ends on a course so taken, held up by the river it flows into, is walked from its
/// mouth upstream as long as `mixed` stands more than that above it, its points below weight 1
/// taken, and so on up the courses that end on it. `mixed` is read at the nearest place on its
/// courses of the same drawn river (`river_partners`), or on its brooks for a brook, within half
/// the water's width there and `DROPPED_REACH_M`; a point with no such place is not taken and ends
/// the walk. The edit takes the cells as a lake's zone (`outline`), with the band beyond, so the
/// band lies where the two worlds' rivers meet.
///
/// Fixed ground stops it (stage 2's ruling R24) where a point a walk upstream takes lies on a
/// fixed cell, or where a kept course enters the outline from fixed ground (its point of weight 0
/// there on a fixed cell) with points of the band held up beyond it: the band cannot move past
/// fixed ground, so the edit cannot take the backwater.
pub fn backwater_cells(old: &World, mixed: &World, weight: &Grid<f32>, join: &WaterJoin, old_drawing: &Drawing, new_drawing: &Drawing, fixed: impl Fn(usize) -> bool) -> Backwater {
    let (width, height) = (weight.width, weight.height);
    let cell_of = |point: &CoursePoint| {
        let index = |at: f32, count: usize| ((f64::from(at) + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
        index(point.z_m, height) * width + index(point.x_m, width)
    };
    let river_partner = river_partners(old_drawing, new_drawing);
    // The river of `mixed` a kept course is read on: its drawn river's partner, or the brooks
    // (`None`) for a brook; none at all for a drawn river the edited drawing does not have.
    let river_of = |course: &RiverCourse| -> Option<Option<usize>> {
        match course.drawn {
            Some(drawn) => river_partner.get(drawn).copied().flatten().map(Some),
            None => Some(None),
        }
    };
    // The nearest place to a point on `mixed`'s courses of a river, within reach: the course and
    // the place.
    let near_of = |river: Option<usize>, point: &CoursePoint| -> Option<(usize, Near)> {
        let (x, z) = (f64::from(point.x_m), f64::from(point.z_m));
        (mixed.water.rivers.iter().enumerate())
            .filter(|(_, other)| other.drawn == river)
            .filter_map(|(index, other)| nearest_on_line(other, x, z).map(|near| (index, near)))
            .filter(|(_, near)| near.apart_m <= near.half_width_m() + DROPPED_REACH_M)
            .reduce(|best, next| if next.1.apart_m < best.1.apart_m { next } else { best })
    };
    // Whether `mixed` stands more than `BACKWATER_ABOVE_M` above a point of a kept course.
    let held_up = |course: &RiverCourse, point: &CoursePoint| -> bool {
        let Some(river) = river_of(course) else { return false };
        near_of(river, point).is_some_and(|(_, near)| near.surface_m - f64::from(point.surface_m) > f64::from(BACKWATER_ABOVE_M))
    };
    // The lake the mixed world's river runs down into from a course, following the courses' ends.
    let lake_below = |mut course: usize| -> Option<usize> {
        for _ in 0..mixed.water.rivers.len() {
            match mixed.water.rivers.get(course)?.end {
                CourseEnd::Lake(lake) => return mixed.water.lakes.get(lake).map(|lake| lake.drawn),
                CourseEnd::River(next) => course = next,
                CourseEnd::Edge => return None,
            }
        }
        None
    };
    let mut found = Backwater::default();
    let stop_at = |found: &mut Backwater, cell: usize| {
        found.stopped = found.stopped.or(Some(cell));
    };
    let mut walked = vec![false; old.water.rivers.len()];
    let mut taken_courses = Vec::new();
    for join in join.joins.iter().filter(|join| join.drawn.is_some() && join.mended_at_weight_1 > 0 && join.lowered_most_m > BACKWATER_ABOVE_M) {
        let Some(kept) = old.water.rivers.get(join.old).filter(|_| !walked[join.old]) else { continue };
        walked[join.old] = true;
        let points = &kept.points;
        let mut taken = false;
        for k in (1..points.len()).filter(|&k| weight_of(weight, &points[k - 1]) == 0.0 && weight_of(weight, &points[k]) > 0.0) {
            let band: Vec<usize> = (k..points.len()).take_while(|&i| weight_of(weight, &points[i]) < 1.0).filter(|&i| held_up(kept, &points[i])).collect();
            let upstream: Vec<usize> = (0..k).rev().take_while(|&i| held_up(kept, &points[i])).collect();
            if band.is_empty() && upstream.is_empty() {
                continue;
            }
            if found.river.is_none() {
                found.river = join.drawn;
                found.lake = near_of(join.drawn, &points[k]).and_then(|(course, _)| lake_below(course));
            }
            if !band.is_empty() && fixed(cell_of(&points[k - 1])) {
                stop_at(&mut found, cell_of(&points[k - 1]));
            }
            if let Some(&i) = upstream.iter().find(|&&i| fixed(cell_of(&points[i]))) {
                stop_at(&mut found, cell_of(&points[i]));
            }
            taken = true;
            found.cells.push(cell_of(&points[k]));
            found.cells.extend(band.iter().chain(&upstream).map(|&i| cell_of(&points[i])));
        }
        if taken {
            taken_courses.push(join.old);
        }
    }
    // The courses held up by a course so taken, from their mouths upstream.
    while let Some(river) = taken_courses.pop() {
        for (index, course) in old.water.rivers.iter().enumerate() {
            if walked[index] || course.end != CourseEnd::River(river) {
                continue;
            }
            walked[index] = true;
            let up: Vec<usize> = (0..course.points.len()).rev().take_while(|&i| held_up(course, &course.points[i])).collect();
            if up.is_empty() {
                continue;
            }
            if let Some(&i) = up.iter().find(|&&i| weight_of(weight, &course.points[i]) == 0.0 && fixed(cell_of(&course.points[i]))) {
                stop_at(&mut found, cell_of(&course.points[i]));
            }
            found.cells.extend(up.iter().filter(|&&i| weight_of(weight, &course.points[i]) < 1.0).map(|&i| cell_of(&course.points[i])));
            taken_courses.push(index);
        }
    }
    found.cells.sort_unstable();
    found.cells.dedup();
    found
}

/// Where the unheld water of an edited land lies that neither the kept land nor the edited drawing
/// grown whole has at the same place (`new_unheld`), each kind counted outside the outline (weight
/// 0), in the band and inside its inner edge (weight 1), in that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NewUnheld {
    /// Dry cells below the water beside them (`Unheld::dry_lake` and `dry_course`).
    pub dry: [usize; 3],
    /// Lake cells over lower water (`Unheld::lake_over_water`).
    pub lake_over_water: [usize; 3],
    /// Steps between water cells, at a mouth or not (`Unheld::water_steps` and `mouth_steps`): a pair
    /// is outside when both its cells lie at weight 0, inside when both lie at weight 1, and in the
    /// band otherwise.
    pub steps: [usize; 3],
}

impl NewUnheld {
    /// All kinds outside the outline.
    pub fn outside(&self) -> usize {
        self.dry[0] + self.lake_over_water[0] + self.steps[0]
    }

    /// All kinds in the band.
    pub fn band(&self) -> usize {
        self.dry[1] + self.lake_over_water[1] + self.steps[1]
    }

    /// All kinds inside the band's inner edge.
    pub fn inside(&self) -> usize {
        self.dry[2] + self.lake_over_water[2] + self.steps[2]
    }
}

/// The unheld water (`unheld_cells`) of `edited`, an edited land, that neither `old`, the land it
/// was made from, nor `whole`, the edited drawing grown whole, has at the same place (the same cell,
/// or the same pair of cells for a step, of the same kind), counted by where it lies under `weight`
/// (`Outline::weight`). The prototype's `edit_probe.rs` compared the edited land with the kept land
/// alone (its `new_unheld`, keyed on the place to the half metre) and judged the counts against the
/// land grown whole apart. A measure that changes nothing; `None` when a world holds no `water`
/// layer.
pub fn new_unheld(edited: &World, old: &World, whole: &World, weight: &Grid<f32>) -> Option<NewUnheld> {
    let (edited_cells, old_cells, whole_cells) = (unheld_cells(edited)?, unheld_cells(old)?, unheld_cells(whole)?);
    let key = |(x, z): (f64, f64)| (x.round() as i64, z.round() as i64);
    let dry = |cells: &UnheldCells| cells.dry.iter().map(|cell| key(cell.at)).collect::<BTreeSet<_>>();
    let over = |cells: &UnheldCells| cells.lake_over_water.iter().map(|cell| key(cell.at)).collect::<BTreeSet<_>>();
    let steps = |cells: &UnheldCells| cells.steps.iter().map(|step| (key(step.a), key(step.b))).collect::<BTreeSet<_>>();
    let place = |at: (f64, f64)| match weight_at(weight, at.0, at.1) {
        0.0 => 0,
        1.0 => 2,
        _ => 1,
    };
    let mut found = NewUnheld::default();
    let (old_dry, whole_dry) = (dry(&old_cells), dry(&whole_cells));
    for cell in edited_cells.dry.iter().filter(|cell| !old_dry.contains(&key(cell.at)) && !whole_dry.contains(&key(cell.at))) {
        found.dry[place(cell.at)] += 1;
    }
    let (old_over, whole_over) = (over(&old_cells), over(&whole_cells));
    for cell in edited_cells.lake_over_water.iter().filter(|cell| !old_over.contains(&key(cell.at)) && !whole_over.contains(&key(cell.at))) {
        found.lake_over_water[place(cell.at)] += 1;
    }
    let (old_steps, whole_steps) = (steps(&old_cells), steps(&whole_cells));
    for step in edited_cells.steps.iter().filter(|step| !old_steps.contains(&(key(step.a), key(step.b))) && !whole_steps.contains(&(key(step.a), key(step.b)))) {
        let places = (place(step.a), place(step.b));
        found.steps[if places.0 == places.1 { places.0 } else { 1 }] += 1;
    }
    Some(found)
}

/// The weight `weight` gives world `(x, z)`: that of the world cell it lies on, the nearest cell's
/// middle, held to the grid.
fn weight_at(weight: &Grid<f32>, x: f64, z: f64) -> f32 {
    let cell = |at: f64, count: usize| ((at + count as f64 / 2.0).round().max(0.0) as usize).min(count - 1);
    weight.data[cell(z, weight.height) * weight.width + cell(x, weight.width)]
}

/// The weight at a course point's place.
fn weight_of(weight: &Grid<f32>, point: &CoursePoint) -> f32 {
    weight_at(weight, f64::from(point.x_m), f64::from(point.z_m))
}

/// A course point's values as their bits, so that two points compare bit for bit.
fn point_bits(point: &CoursePoint) -> [u32; 5] {
    [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m].map(f32::to_bits)
}

/// The nearest place on a course's line to a place, and the water's surface, width and depth there,
/// linear along the stretch, in `f64`.
#[derive(Debug, Clone, Copy)]
struct Near {
    apart_m: f64,
    x_m: f64,
    z_m: f64,
    surface_m: f64,
    width_m: f64,
    depth_m: f64,
}

impl Near {
    /// Half the water's width here, held to at least `HALF_WIDTH_MIN_M`, as a bed is cut.
    fn half_width_m(&self) -> f64 {
        (self.width_m / 2.0).max(HALF_WIDTH_MIN_M)
    }
}

/// The nearest place to `(x, z)` on the stretch from `a` to `b`.
fn near_stretch(a: &CoursePoint, b: &CoursePoint, x: f64, z: f64) -> Near {
    let (ax, az, bx, bz) = (f64::from(a.x_m), f64::from(a.z_m), f64::from(b.x_m), f64::from(b.z_m));
    let (dx, dz) = (bx - ax, bz - az);
    let squared = dx * dx + dz * dz;
    let t = if squared > 0.0 { (((x - ax) * dx + (z - az) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
    let (px, pz) = (ax + dx * t, az + dz * t);
    let between = |u: f32, v: f32| f64::from(u) + (f64::from(v) - f64::from(u)) * t;
    Near { apart_m: (px - x).hypot(pz - z), x_m: px, z_m: pz, surface_m: between(a.surface_m, b.surface_m), width_m: between(a.width_m, b.width_m), depth_m: between(a.depth_m, b.depth_m) }
}

/// The nearest place to `(x, z)` on `course`'s line, the first stretch of several as near; a course
/// of one point is that point. `None` for a course of no point.
fn nearest_on_line(course: &RiverCourse, x: f64, z: f64) -> Option<Near> {
    match course.points.as_slice() {
        [] => None,
        [only] => Some(near_stretch(only, only, x, z)),
        points => points.windows(2).map(|pair| near_stretch(&pair[0], &pair[1], x, z)).reduce(|best, next| if next.apart_m < best.apart_m { next } else { best }),
    }
}

/// A run of a course's points all inside the outline (weight above 0) or all outside: its first and
/// last point, the last included.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    inside: bool,
    first: usize,
    last: usize,
}

/// The runs of `course` under `weight`, in its order.
fn runs(course: &RiverCourse, weight: &Grid<f32>) -> Vec<Run> {
    let mut found: Vec<Run> = Vec::new();
    for (index, point) in course.points.iter().enumerate() {
        let inside = weight_of(weight, point) > 0.0;
        match found.last_mut() {
            Some(run) if run.inside == inside => run.last = index,
            _ => found.push(Run { inside, first: index, last: index }),
        }
    }
    found
}

/// Whether some point of `course` lies inside the outline, and whether some lies outside.
fn touches(course: &RiverCourse, weight: &Grid<f32>) -> bool {
    course.points.iter().any(|point| weight_of(weight, point) > 0.0)
}

fn reaches_outside(course: &RiverCourse, weight: &Grid<f32>) -> bool {
    course.points.iter().any(|point| weight_of(weight, point) == 0.0)
}

/// Where a course crosses the outline, from upstream: the middle between the last point of each run
/// and the first of the next.
fn crossings_of(course: &RiverCourse, weight: &Grid<f32>) -> Vec<(f64, f64)> {
    let middle = |a: &CoursePoint, b: &CoursePoint| ((f64::from(a.x_m) + f64::from(b.x_m)) / 2.0, (f64::from(a.z_m) + f64::from(b.z_m)) / 2.0);
    runs(course, weight).windows(2).map(|pair| middle(&course.points[pair[0].last], &course.points[pair[1].first])).collect()
}

/// The joined land's lakes, and which of them each lake of the two worlds became.
struct JoinedLakes {
    lakes: Vec<LakeWater>,
    /// For each lake of `old.water.lakes`, and of `new.water.lakes`, its index in `lakes` where the
    /// joined land takes it.
    old_to: Vec<Option<usize>>,
    new_to: Vec<Option<usize>>,
    /// For each lake of `old.water.lakes`, and of `new.water.lakes`, the index in `lakes` of the lake
    /// it is, the same drawn lake (`lake_partners`), from whichever world the joined land takes it:
    /// the lake a course flowing out of it or into it names.
    old_is: Vec<Option<usize>>,
    new_is: Vec<Option<usize>>,
}

/// The lakes of the joined land, as `join_water` says, in `new_drawing`'s order.
fn join_lakes(old: &World, new: &World, weight: &Grid<f32>, old_drawing: &Drawing, new_drawing: &Drawing, lake_partner: &[Option<usize>]) -> JoinedLakes {
    let middle = |lake: &LakeWater| weight_at(weight, f64::from(lake.x_m), f64::from(lake.z_m));
    // A kept lake held whole lies wholly at weight 0, though its middle, the mean of its cells'
    // places, may lie on ground beside its water (an island, a bay's far shore) that weighs more.
    let wholly_outside = |lake: &LakeWater| {
        let cells = old_drawing.lakes.get(lake.drawn).map_or_else(Vec::new, |drawn| lake_water(old, drawn, lake.drawn));
        !cells.is_empty() && cells.iter().all(|&cell| weight.data[cell] == 0.0)
    };
    let mut joined = JoinedLakes {
        lakes: Vec::new(),
        old_to: vec![None; old.water.lakes.len()],
        new_to: vec![None; new.water.lakes.len()],
        old_is: Vec::new(),
        new_is: Vec::new(),
    };
    for drawn in 0..new_drawing.lakes.len() {
        let kept = (old.water.lakes.iter().enumerate())
            .find(|(_, lake)| lake_partner.get(lake.drawn).copied().flatten() == Some(drawn) && (middle(lake) == 0.0 || wholly_outside(lake)));
        let taken = (new.water.lakes.iter().enumerate()).find(|(_, lake)| lake.drawn == drawn && middle(lake) > 0.0);
        if let Some((index, lake)) = kept {
            joined.old_to[index] = Some(joined.lakes.len());
            joined.lakes.push(LakeWater { drawn, ..*lake });
        } else if let Some((index, lake)) = taken {
            joined.new_to[index] = Some(joined.lakes.len());
            joined.lakes.push(*lake);
        }
    }
    let of_drawn = |drawn: Option<usize>| drawn.and_then(|drawn| joined.lakes.iter().position(|lake| lake.drawn == drawn));
    joined.old_is = old.water.lakes.iter().map(|lake| of_drawn(lake_partner.get(lake.drawn).copied().flatten())).collect();
    joined.new_is = new.water.lakes.iter().map(|lake| of_drawn(Some(lake.drawn))).collect();
    joined
}

/// Which course of the other world each course is joined with, and the brooks' counts.
struct Partners {
    /// For each drawn river of `old_drawing`, the river of `new_drawing` it is.
    river_partner: Vec<Option<usize>>,
    /// The kept land's courses as the join pairs them: `old`'s, but a course cut at a lake of the
    /// new world (`join_water`, **Partners**), whose run above the lake is in its place and whose
    /// run below follows the others; and for each, the course of `old` it is or is cut from.
    kept: Vec<RiverCourse>,
    kept_from: Vec<usize>,
    /// For each of `kept`, and of the new world's courses, its partner.
    of_old: Vec<Option<usize>>,
    of_new: Vec<Option<usize>>,
    brooks_crossing: (usize, usize),
    brooks_paired: usize,
}

/// Which drawn river of `new_drawing` each of `old_drawing`'s is: the first equal one not already
/// taken; then, of the rivers of either drawing left, the two whose smoothed lines (`smoothed_line`,
/// the line the land model lays) lie nearest on the mean (`mean_apart_m`) are one, then the next
/// nearest two, and so on (the first in the old drawing's order, then the new's, of pairs as near),
/// but never two farther apart than `RIVER_PARTNER_REACH_M`. So a river added or taken away in the
/// same edit as a change to another pairs with neither, whatever their order in the drawings, and
/// a river whose line moves farther than the band on the mean is a river taken away and another
/// added.
pub(crate) fn river_partners(old_drawing: &Drawing, new_drawing: &Drawing) -> Vec<Option<usize>> {
    let mut paired = vec![false; new_drawing.rivers.len()];
    let mut partners: Vec<Option<usize>> = (old_drawing.rivers.iter())
        .map(|river| {
            let partner = (0..new_drawing.rivers.len()).find(|&n| !paired[n] && new_drawing.rivers[n] == *river)?;
            paired[partner] = true;
            Some(partner)
        })
        .collect();
    if partners.iter().all(Option::is_some) || paired.iter().all(|&taken| taken) {
        return partners;
    }
    let lines = |drawing: &Drawing| -> Vec<Option<Vec<(f64, f64)>>> { drawing.rivers.iter().map(|river| smoothed_line(&river.points)).collect() };
    let (old_lines, new_lines) = (lines(old_drawing), lines(new_drawing));
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (o, old_line) in old_lines.iter().enumerate().filter(|&(o, _)| partners[o].is_none()) {
        for (n, new_line) in new_lines.iter().enumerate().filter(|&(n, _)| !paired[n]) {
            if let (Some(old_line), Some(new_line)) = (old_line, new_line) {
                let apart_m = mean_apart_m(old_line, new_line);
                if apart_m <= RIVER_PARTNER_REACH_M {
                    pairs.push((apart_m, o, n));
                }
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, o, n) in pairs {
        if partners[o].is_none() && !paired[n] {
            partners[o] = Some(n);
            paired[n] = true;
        }
    }
    partners
}

/// How far apart two lines lie on the mean, in metres: the mean over each line's points of their
/// distance to the other line, the two means averaged, so the same both ways. A line of one point
/// is that point; a line of none lies infinitely far.
pub(crate) fn mean_apart_m(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let to_line = |p: (f64, f64), line: &[(f64, f64)]| match line {
        [] => f64::INFINITY,
        [only] => (p.0 - only.0).hypot(p.1 - only.1),
        _ => line.windows(2).map(|pair| to_segment(p, pair[0], pair[1])).fold(f64::INFINITY, f64::min),
    };
    let mean = |from: &[(f64, f64)], to: &[(f64, f64)]| {
        if from.is_empty() {
            return f64::INFINITY;
        }
        from.iter().map(|&p| to_line(p, to)).sum::<f64>() / from.len() as f64
    };
    (mean(a, b) + mean(b, a)) / 2.0
}

/// The partners of the two worlds' courses (`join_water`, **Partners**).
fn pair_courses(old: &World, new: &World, weight: &Grid<f32>, old_drawing: &Drawing, new_drawing: &Drawing, lake_partner: &[Option<usize>]) -> Partners {
    let river_partner = river_partners(old_drawing, new_drawing);
    // A drawn course's river and the lake it flows out of, both as `new_drawing` places them; `None`
    // for a brook, or for a course of `old` whose river or lake `new_drawing` does not have.
    let old_key = |course: &RiverCourse| -> Option<(usize, Option<usize>)> {
        let river = river_partner.get(course.drawn?).copied().flatten()?;
        let lake = match course.from_lake {
            None => None,
            Some(lake) => Some(lake_partner.get(old.water.lakes.get(lake)?.drawn).copied().flatten()?),
        };
        Some((river, lake))
    };
    let new_key = |course: &RiverCourse| -> Option<(usize, Option<usize>)> {
        let lake = match course.from_lake {
            None => None,
            Some(lake) => Some(new.water.lakes.get(lake)?.drawn),
        };
        Some((course.drawn?, lake))
    };
    let old_touched: Vec<usize> = (0..old.water.rivers.len()).filter(|&j| touches(&old.water.rivers[j], weight)).collect();
    let new_touched: Vec<usize> = (0..new.water.rivers.len()).filter(|&k| touches(&new.water.rivers[k], weight)).collect();
    let mut of_old = vec![None; old.water.rivers.len()];
    let mut of_new = vec![None; new.water.rivers.len()];
    for &j in &old_touched {
        let Some(key) = old_key(&old.water.rivers[j]) else { continue };
        let ordinal = old_touched.iter().filter(|&&i| i < j && old_key(&old.water.rivers[i]) == Some(key)).count();
        if let Some(k) = new_touched.iter().copied().filter(|&k| new_key(&new.water.rivers[k]) == Some(key)).nth(ordinal) {
            of_old[j] = Some(k);
            of_new[k] = Some(j);
        }
    }
    // Then a drawn river's courses no lake partner paired, by their order among the river's courses
    // (each river's from upstream, as the fine steps list them), only where the order names the same
    // stretch of it: the river has as many courses in both worlds, and at every place in its chain
    // the two courses start from no lake, or from one lake, or from two lakes neither of which is
    // another's. A lake is another's when `lake_partners` pairs them or, of the lakes it leaves, the
    // two lie at the same place (a lake resized where it was), so a chain whose lakes the edit
    // reorders, the lake that starts a stretch now another's, pairs as before.
    let resized = lakes_resized(old_drawing, new_drawing, lake_partner);
    let is_of = |old_lake: usize| lake_partner.get(old_lake).copied().flatten().or(resized.get(old_lake).copied().flatten());
    let starts = |world: &World, course: usize| world.water.rivers[course].from_lake.and_then(|lake| world.water.lakes.get(lake)).map(|lake| lake.drawn);
    let one_place = |j: usize, k: usize| match (starts(old, j), starts(new, k)) {
        (None, None) => true,
        (Some(kept), Some(fresh)) => is_of(kept) == Some(fresh) || (is_of(kept).is_none() && !(0..old_drawing.lakes.len()).any(|other| is_of(other) == Some(fresh))),
        _ => false,
    };
    let courses_of = |world: &World, drawn: usize| (0..world.water.rivers.len()).filter(|&i| world.water.rivers[i].drawn == Some(drawn)).collect::<Vec<_>>();
    for (old_drawn, new_drawn) in river_partner.iter().enumerate().filter_map(|(old_drawn, &new_drawn)| Some((old_drawn, new_drawn?))) {
        let (kept, fresh) = (courses_of(old, old_drawn), courses_of(new, new_drawn));
        if kept.len() != fresh.len() || !kept.iter().zip(&fresh).all(|(&j, &k)| one_place(j, k)) {
            continue;
        }
        for (&j, &k) in kept.iter().zip(&fresh) {
            if of_old[j].is_none() && of_new[k].is_none() && old_touched.contains(&j) && new_touched.contains(&k) {
                of_old[j] = Some(k);
                of_new[k] = Some(j);
            }
        }
    }

    // Then a drawn river's kept course that runs on past a lake its partner ends in is cut at that
    // lake, as the new world's water is: where the partner ends in a lake of the new world that the
    // kept course does not end in, a course of the same river out of that lake has no partner, and
    // the kept course's water (`cut_at_lake`) reaches the lake's disc inside the outline and the
    // course leaves the outline below it, its run below the lake is joined with the course out of
    // the lake, and its run above with the partner where that run leaves the outline too; a run
    // above wholly inside the outline is left out, as the new world's water replaces it, and the
    // partner is taken as the new world has it. So the river is neither bridged in a straight
    // stretch from the lake to where it leaves the outline nor cut off there beside the course out
    // of the lake (the design's W3; stage 2's ruling R20). A run below is cut again where it runs
    // past the next such lake.
    let mut kept = old.water.rivers.clone();
    let mut kept_from: Vec<usize> = (0..kept.len()).collect();
    let mut j = 0;
    while j < kept.len() {
        let cut = of_old[j].and_then(|k| {
            let fresh = &new.water.rivers[k];
            let CourseEnd::Lake(lake) = fresh.end else { return None };
            let drawn = new.water.lakes.get(lake)?.drawn;
            let ends_in_it = |course: &RiverCourse| matches!(course.end, CourseEnd::Lake(at) if old.water.lakes.get(at).and_then(|lake| is_of(lake.drawn)) == Some(drawn));
            if kept[j].drawn.is_none() || ends_in_it(&kept[j]) {
                return None;
            }
            let out_of_it = (0..new.water.rivers.len()).find(|&n| {
                let course = &new.water.rivers[n];
                of_new[n].is_none() && course.drawn == fresh.drawn && course.from_lake == Some(lake) && new_touched.contains(&n)
            })?;
            let (above, below) = cut_at_lake(&kept[j], new_drawing.lakes.get(drawn)?, weight)?;
            Some((above, below, out_of_it))
        });
        if let Some((above, below, out_of_it)) = cut {
            let course = kept[j].clone();
            let run_below = RiverCourse { drawn: course.drawn, from_lake: None, points: course.points[below..].to_vec(), end: course.end };
            kept[j] = RiverCourse { drawn: course.drawn, from_lake: course.from_lake, points: course.points[..=above].to_vec(), end: CourseEnd::Edge };
            kept.push(run_below);
            kept_from.push(kept_from[j]);
            of_old.push(Some(out_of_it));
            of_new[out_of_it] = Some(kept.len() - 1);
            if !reaches_outside(&kept[j], weight) {
                if let Some(k) = of_old[j].take() {
                    of_new[k] = None;
                }
            }
        }
        j += 1;
    }

    // Of a drawn river's courses, however paired, those that meet nowhere at the outline are its
    // water gone two ways, not one course: where one or both cross the outline (`crossings_of`) and
    // every place either crosses it lies farther from the other's line than
    // `RIVER_PARTNER_REACH_M`, the reach at which two lines are one river (`river_partners`), they
    // are left apart, the kept course ended at the outline or dropped and the new one narrowed
    // there, as courses with no partner are; so the band's points of a course are never drawn
    // toward a line that crosses the outline nowhere near them.
    let near_the_other = |course: &RiverCourse, line: &RiverCourse| -> (usize, bool) {
        let crossings = crossings_of(course, weight);
        let near = crossings.iter().any(|&(x, z)| nearest_on_line(line, x, z).is_some_and(|near| near.apart_m <= RIVER_PARTNER_REACH_M));
        (crossings.len(), near)
    };
    for j in 0..kept.len() {
        let Some(k) = of_old[j] else { continue };
        let (kept, fresh) = (&kept[j], &new.water.rivers[k]);
        let ((kept_crossings, kept_near), (fresh_crossings, fresh_near)) = (near_the_other(kept, fresh), near_the_other(fresh, kept));
        if kept.drawn.is_some() && kept_crossings + fresh_crossings > 0 && !kept_near && !fresh_near {
            of_old[j] = None;
            of_new[k] = None;
        }
    }

    // And a drawn river's courses left with no partner that meet at the outline are one course: a
    // kept course and a new one of the same river (`river_partners`) that both cross the outline,
    // a place where one crosses it lying within `RIVER_PARTNER_REACH_M` of a place where the other
    // does, are paired, the two whose places lie nearest first (the first in the kept land's order,
    // then the new world's, of pairs as near); so a river the new world runs through a lake by
    // another way than the kept land's, its course out of the lake leaving the outline where the
    // kept course does, is not cut off there beside it (stage 2's ruling R20).
    let unpaired_crossings = |course: &RiverCourse, paired: bool| -> Vec<(f64, f64)> { if paired || course.drawn.is_none() { Vec::new() } else { crossings_of(course, weight) } };
    let fresh_crossings: Vec<Vec<(f64, f64)>> = (0..new.water.rivers.len()).map(|k| unpaired_crossings(&new.water.rivers[k], of_new[k].is_some())).collect();
    let mut meetings: Vec<(f64, usize, usize)> = Vec::new();
    for j in 0..kept.len() {
        let kept_crossings = unpaired_crossings(&kept[j], of_old[j].is_some());
        let river = kept[j].drawn.and_then(|drawn| river_partner.get(drawn).copied().flatten());
        for k in (0..new.water.rivers.len()).filter(|&k| river.is_some() && new.water.rivers[k].drawn == river) {
            let apart_m = (kept_crossings.iter()).flat_map(|&(x, z)| fresh_crossings[k].iter().map(move |&(u, v)| (x - u).hypot(z - v))).fold(f64::INFINITY, f64::min);
            if apart_m <= RIVER_PARTNER_REACH_M {
                meetings.push((apart_m, j, k));
            }
        }
    }
    meetings.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, j, k) in meetings {
        if of_old[j].is_none() && of_new[k].is_none() {
            of_old[j] = Some(k);
            of_new[k] = Some(j);
        }
    }

    // Brooks equal bit for bit, first: the edit left such a brook as it was, wherever it lies.
    let crossing_brook = |course: &RiverCourse| course.drawn.is_none() && reaches_outside(course, weight);
    let mut brooks_paired = 0;
    let bits = |course: &RiverCourse| course.points.iter().map(point_bits).collect::<Vec<_>>();
    for &j in old_touched.iter().filter(|&&j| old.water.rivers[j].drawn.is_none()) {
        let kept = bits(&old.water.rivers[j]);
        let equal = new_touched.iter().copied().find(|&k| new.water.rivers[k].drawn.is_none() && of_new[k].is_none() && bits(&new.water.rivers[k]) == kept);
        if let Some(k) = equal {
            of_old[j] = Some(k);
            of_new[k] = Some(j);
            brooks_paired += usize::from(crossing_brook(&old.water.rivers[j]));
        }
    }

    // Brooks, by course.
    let band_points = |course: &RiverCourse| -> Vec<(f64, f64)> {
        (course.points.iter())
            .filter(|point| {
                let w = weight_of(weight, point);
                w > 0.0 && w < 1.0
            })
            .map(|point| (f64::from(point.x_m), f64::from(point.z_m)))
            .collect()
    };
    let widest = |course: &RiverCourse| course.points.iter().map(|point| f64::from(point.width_m)).fold(0.0, f64::max);
    let apart = |points: &[(f64, f64)], line: &RiverCourse| -> Vec<f64> { points.iter().map(|&(x, z)| nearest_on_line(line, x, z).map_or(f64::INFINITY, |near| near.apart_m)).collect() };
    let median = |mut values: Vec<f64>| -> f64 {
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let old_brooks: Vec<(usize, Vec<(f64, f64)>)> = (old_touched.iter()).filter(|&&j| old.water.rivers[j].drawn.is_none()).map(|&j| (j, band_points(&old.water.rivers[j]))).filter(|(_, points)| !points.is_empty()).collect();
    let new_brooks: Vec<(usize, Vec<(f64, f64)>)> = (new_touched.iter()).filter(|&&k| new.water.rivers[k].drawn.is_none()).map(|&k| (k, band_points(&new.water.rivers[k]))).filter(|(_, points)| !points.is_empty()).collect();
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (j, old_points) in &old_brooks {
        let kept = &old.water.rivers[*j];
        for (k, new_points) in &new_brooks {
            let fresh = &new.water.rivers[*k];
            let limit_m = widest(kept).max(widest(fresh)) + BROOK_PAIR_M;
            // Each near the other at its middle in the band, and the new brook's head, where it lies
            // in the band, near the kept line: the join drops the kept brook's band points and
            // moves the new one's toward the kept line, so a new head far from it would put the
            // joined course where neither land has a valley; a kept head further up goes with its
            // points.
            let by_middle = median(apart(old_points, fresh)) <= limit_m && median(apart(new_points, kept)) <= limit_m;
            let head_in_band = fresh.points.first().filter(|head| {
                let w = weight_of(weight, head);
                w > 0.0 && w < 1.0
            });
            let head_near = head_in_band.is_none_or(|head| nearest_on_line(kept, f64::from(head.x_m), f64::from(head.z_m)).is_some_and(|near| near.apart_m <= limit_m + HEAD_SLACK_M));
            if by_middle && head_near {
                pairs.push(((median(apart(old_points, fresh)) + median(apart(new_points, kept))) / 2.0, *j, *k));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (_, j, k) in pairs {
        if of_old[j].is_none() && of_new[k].is_none() {
            of_old[j] = Some(k);
            of_new[k] = Some(j);
            brooks_paired += usize::from(crossing_brook(&old.water.rivers[j]));
        }
    }
    let brooks_crossing = (
        old_touched.iter().filter(|&&j| crossing_brook(&old.water.rivers[j])).count(),
        new_touched.iter().filter(|&&k| crossing_brook(&new.water.rivers[k])).count(),
    );
    Partners { river_partner, kept, kept_from, of_old, of_new, brooks_crossing, brooks_paired }
}

/// Where `course`, a course of the kept land, is cut at `lake`, a lake of the edited drawing: the
/// last of its points before its first point whose water reaches the lake's disc (the point no
/// farther from the disc than half its width) and the first after its last such point, its points
/// between left out. `None` where no point's water reaches the disc, where the course starts or
/// ends there, where one of the points from the one before to the one after lies outside the
/// outline (weight 0), whose points the join keeps bit for bit, or where the course does not leave
/// the outline below the lake.
fn cut_at_lake(course: &RiverCourse, lake: &Lake, weight: &Grid<f32>) -> Option<(usize, usize)> {
    let within = |point: &CoursePoint| (f64::from(point.x_m) - lake.x_m).hypot(f64::from(point.z_m) - lake.z_m) <= f64::from(lake.radius_m) + f64::from(point.width_m) / 2.0;
    let (first, last) = (course.points.iter().position(within)?, course.points.iter().rposition(within)?);
    let (above, below) = (first.checked_sub(1)?, last + 1);
    let points = &course.points;
    if below >= points.len() || points[above..=below].iter().any(|point| weight_of(weight, point) == 0.0) || !points[below..].iter().any(|point| weight_of(weight, point) == 0.0) {
        return None;
    }
    Some((above, below))
}

/// For each lake of `old_drawing` that `lake_partners` (`partner`) leaves apart, the lake of
/// `new_drawing` it is resized into: the first one at the same place (its centre the same, bit for
/// bit) that no other lake is, or `None`.
fn lakes_resized(old_drawing: &Drawing, new_drawing: &Drawing, partner: &[Option<usize>]) -> Vec<Option<usize>> {
    let mut taken: Vec<bool> = (0..new_drawing.lakes.len()).map(|new| partner.contains(&Some(new))).collect();
    (old_drawing.lakes.iter().enumerate())
        .map(|(old, lake)| {
            if partner.get(old).copied().flatten().is_some() {
                return None;
            }
            let place = |other: &Lake| other.x_m.to_bits() == lake.x_m.to_bits() && other.z_m.to_bits() == lake.z_m.to_bits();
            let found = (0..new_drawing.lakes.len()).find(|&new| !taken[new] && place(&new_drawing.lakes[new]))?;
            taken[found] = true;
            Some(found)
        })
        .collect()
}

/// How a course of the joined land is made.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Made {
    /// A course of the kept land that does not touch the outline.
    Kept(usize),
    /// A course of the kept land joined with its partner in the new world.
    Joined(usize, usize),
    /// A course of the kept land with no partner: its runs outside, each a course ended at the
    /// outline.
    Ended(usize),
    /// A course of the new world with no partner, wholly inside the outline.
    New(usize),
    /// A course of the new world with no partner that crosses the outline: its runs inside, each a
    /// course narrowed across the band.
    Narrowed(usize),
}

impl Made {
    /// The course of the kept land it is made from, if any.
    fn old_course(self) -> Option<usize> {
        match self {
            Made::Kept(j) | Made::Joined(j, _) | Made::Ended(j) => Some(j),
            Made::New(_) | Made::Narrowed(_) => None,
        }
    }

    /// The course of the new world it is made from, if any.
    fn new_course(self) -> Option<usize> {
        match self {
            Made::Joined(_, k) | Made::New(k) | Made::Narrowed(k) => Some(k),
            Made::Kept(_) | Made::Ended(_) => None,
        }
    }
}

/// The joined land's courses, as what each is made from, in the order made: the kept land's in its
/// order, the runs of a course cut at a lake (`Partners::kept`) in its place from upstream, then the
/// new world's; with the places brooks were ended and narrowed in `report`.
fn what_is_made(new: &World, weight: &Grid<f32>, partners: &Partners, report: &mut WaterJoin) -> Vec<Made> {
    let mut made = Vec::new();
    let mut order: Vec<usize> = (0..partners.kept.len()).collect();
    order.sort_by_key(|&j| (partners.kept_from[j], j));
    for j in order {
        let course = &partners.kept[j];
        if !touches(course, weight) {
            made.push(Made::Kept(j));
        } else if let Some(k) = partners.of_old[j] {
            made.push(Made::Joined(j, k));
        } else if reaches_outside(course, weight) {
            made.push(Made::Ended(j));
            let places = crossings_of(course, weight);
            let river = course.drawn.and_then(|drawn| partners.river_partner.get(drawn).copied().flatten());
            report.ended_rivers.extend(std::iter::repeat_n(river, places.len()));
            report.brooks_ended.extend(places);
        }
    }
    for (k, course) in new.water.rivers.iter().enumerate() {
        if !touches(course, weight) || partners.of_new[k].is_some() {
            continue;
        }
        if reaches_outside(course, weight) {
            made.push(Made::Narrowed(k));
            let places = crossings_of(course, weight);
            report.narrowed_rivers.extend(std::iter::repeat_n(course.drawn, places.len()));
            report.brooks_narrowed.extend(places);
        } else {
            made.push(Made::New(k));
        }
    }
    made
}

/// A course of the joined land as it is made, before its end names a course or lake of the joined
/// land: the course, its end left at the border; the end it keeps from the world it came from, if
/// any, and whether that world is the kept land; which of its points the mending lowered; and the
/// entry of `made` it comes from.
struct Piece {
    course: RiverCourse,
    end: Option<CourseEnd>,
    from_the_kept_land: bool,
    lowered: Vec<bool>,
    made: usize,
}

/// The joined land's courses, and for each, which of its points the mending lowered.
struct JoinedCourses {
    rivers: Vec<RiverCourse>,
    lowered: Vec<Vec<bool>>,
}

/// The joined land's courses (`join_water`, **Courses**), with each joined course's `CourseJoin`
/// and the ends lost in `report`.
fn join_courses(new: &World, weight: &Grid<f32>, made: &[Made], partners: &Partners, lakes: &JoinedLakes, report: &mut WaterJoin) -> JoinedCourses {
    let (river_partner, kept_courses) = (&partners.river_partner, &partners.kept);
    let old_drawn = |course: &RiverCourse| course.drawn.and_then(|drawn| river_partner.get(drawn).copied().flatten());
    let from_old = |course: &RiverCourse| course.from_lake.and_then(|lake| lakes.old_is.get(lake).copied().flatten());
    let from_new = |course: &RiverCourse| course.from_lake.and_then(|lake| lakes.new_is.get(lake).copied().flatten());
    let mut pieces: Vec<Piece> = Vec::new();
    let mut joins = Vec::new();
    for (index, &how) in made.iter().enumerate() {
        // A course of these points, none of them lowered, its lake an index of the joined lakes.
        let piece = |drawn: Option<usize>, from_lake: Option<usize>, points: Vec<CoursePoint>, end: Option<CourseEnd>, from_the_kept_land: bool| Piece {
            lowered: vec![false; points.len()],
            course: RiverCourse { drawn, from_lake, points, end: CourseEnd::Edge },
            end,
            from_the_kept_land,
            made: index,
        };
        match how {
            Made::Kept(j) => {
                let course = &kept_courses[j];
                pieces.push(piece(old_drawn(course), from_old(course), course.points.clone(), Some(course.end), true));
            }
            Made::New(k) => {
                let course = &new.water.rivers[k];
                pieces.push(piece(course.drawn, from_new(course), course.points.clone(), Some(course.end), false));
            }
            Made::Ended(j) => {
                // Each run outside a course of its own: out of the course's lake if it is where the
                // course starts, into the course's end if it is where the course ends.
                let course = &kept_courses[j];
                let last = course.points.len() - 1;
                for run in runs(course, weight).into_iter().filter(|run| !run.inside) {
                    let from_lake = if run.first == 0 { from_old(course) } else { None };
                    pieces.push(piece(old_drawn(course), from_lake, course.points[run.first..=run.last].to_vec(), (run.last == last).then_some(course.end), true));
                }
            }
            Made::Narrowed(k) => {
                let course = &new.water.rivers[k];
                let last = course.points.len() - 1;
                for run in runs(course, weight).into_iter().filter(|run| run.inside) {
                    let from_lake = if run.first == 0 { from_new(course) } else { None };
                    pieces.push(piece(course.drawn, from_lake, narrowed(course, run, weight), (run.last == last).then_some(course.end), false));
                }
            }
            Made::Joined(j, k) => {
                let (kept, fresh) = (&kept_courses[j], &new.water.rivers[k]);
                let JoinedCourse { points, lowered, join, apart, fresh_apart, fresh_start, fresh_end } = joined_course(kept, fresh, weight);
                let kept_runs = runs(kept, weight);
                let starts_outside = kept_runs.first().is_some_and(|run| !run.inside);
                let ends_outside = kept_runs.last().is_some_and(|run| !run.inside);
                let from_lake = if starts_outside { from_old(kept) } else if fresh_start { from_new(fresh) } else { None };
                let (end, end_is_old) = if ends_outside { (Some(kept.end), true) } else { (fresh_end.then_some(fresh.end), false) };
                joins.push(CourseJoin { course: pieces.len(), old: partners.kept_from[j], new: k, drawn: fresh.drawn, ..join });
                let course = RiverCourse { drawn: fresh.drawn, from_lake, points, end: CourseEnd::Edge };
                pieces.push(Piece { course, end, from_the_kept_land: end_is_old, lowered, made: index });
                // The kept course's runs outside that the join leaves apart, each a course of its own.
                for run in apart {
                    pieces.push(piece(fresh.drawn, None, kept.points[run.first..=run.last].to_vec(), None, true));
                }
                // The partner's runs inside that the join leaves apart, each a course of its own
                // narrowed across the band, as a course of the new world with no partner is.
                let last = fresh.points.len() - 1;
                let middle = |a: &CoursePoint, b: &CoursePoint| ((f64::from(a.x_m) + f64::from(b.x_m)) / 2.0, (f64::from(a.z_m) + f64::from(b.z_m)) / 2.0);
                for run in fresh_apart {
                    let from_lake = if run.first == 0 { from_new(fresh) } else { None };
                    pieces.push(piece(fresh.drawn, from_lake, narrowed(fresh, run, weight), (run.last == last).then_some(fresh.end), false));
                    let entering = (run.first > 0).then(|| middle(&fresh.points[run.first - 1], &fresh.points[run.first]));
                    let leaving = (run.last < last).then(|| middle(&fresh.points[run.last], &fresh.points[run.last + 1]));
                    for place in entering.into_iter().chain(leaving) {
                        report.brooks_narrowed.push(place);
                        report.narrowed_rivers.push(fresh.drawn);
                    }
                }
            }
        }
    }

    // Drawn rivers first, in the new drawing's order, then the brooks, each in the order made, the
    // courses made from one course from upstream.
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by_key(|&index| (pieces[index].course.drawn.is_none(), pieces[index].course.drawn.unwrap_or(0)));
    let mut position = vec![0; pieces.len()];
    for (at, &index) in order.iter().enumerate() {
        position[index] = at;
    }
    // The joined course a course ends on that ended on course `river` of its world: of the courses
    // made from that one (from its runs, where it was cut at a lake), the one with a point nearest
    // its last point, the first of several as near.
    let ends_on = |river: usize, from_the_kept_land: bool, last: &CoursePoint| -> Option<usize> {
        let of = |how: &Made| if from_the_kept_land { how.old_course().map(|j| partners.kept_from[j]) } else { how.new_course() };
        let sources: Vec<usize> = (0..made.len()).filter(|&index| of(&made[index]) == Some(river)).collect();
        let apart_m = |piece: &Piece| {
            let apart = |point: &CoursePoint| (f64::from(point.x_m) - f64::from(last.x_m)).hypot(f64::from(point.z_m) - f64::from(last.z_m));
            piece.course.points.iter().map(apart).fold(f64::INFINITY, f64::min)
        };
        let made_from = (0..pieces.len()).filter(|&index| sources.contains(&pieces[index].made));
        made_from.map(|index| (apart_m(&pieces[index]), index)).reduce(|best, next| if next.0 < best.0 { next } else { best }).map(|(_, index)| position[index])
    };
    let (mut ends_lost, mut ends_lost_at, mut ends_lost_rivers) = (0, Vec::new(), Vec::new());
    let mut joined = JoinedCourses { rivers: Vec::with_capacity(pieces.len()), lowered: Vec::with_capacity(pieces.len()) };
    for &index in &order {
        let piece = &pieces[index];
        let mut course = piece.course.clone();
        let to = match (piece.end, course.points.last()) {
            (None | Some(CourseEnd::Edge), _) | (_, None) => Some(CourseEnd::Edge),
            (Some(CourseEnd::River(river)), Some(last)) => ends_on(river, piece.from_the_kept_land, last).map(CourseEnd::River),
            (Some(CourseEnd::Lake(lake)), Some(_)) => {
                let to = if piece.from_the_kept_land { lakes.old_is.get(lake) } else { lakes.new_is.get(lake) };
                to.copied().flatten().map(CourseEnd::Lake)
            }
        };
        course.end = to.unwrap_or_else(|| {
            ends_lost += 1;
            if let Some(last) = course.points.last() {
                ends_lost_at.push((f64::from(last.x_m), f64::from(last.z_m)));
                ends_lost_rivers.push(course.drawn);
            }
            CourseEnd::Edge
        });
        joined.rivers.push(course);
        joined.lowered.push(piece.lowered.clone());
    }
    for join in &mut joins {
        join.course = position[join.course];
    }
    report.joins = joins;
    (report.ends_lost, report.ends_lost_at, report.ends_lost_rivers) = (ends_lost, ends_lost_at, ends_lost_rivers);
    joined
}

/// The points of `run`, a run of `course` inside the outline, each one's width and depth times its
/// weight, so the run narrows to nothing toward the outline.
fn narrowed(course: &RiverCourse, run: Run, weight: &Grid<f32>) -> Vec<CoursePoint> {
    (course.points[run.first..=run.last].iter())
        .map(|&point| {
            let w = weight_of(weight, &point);
            CoursePoint { width_m: point.width_m * w, depth_m: point.depth_m * w, ..point }
        })
        .collect()
}

/// A course of the kept land joined with its partner (`joined_course`): its points, mended, which
/// of them the mending lowered, what joining did (its `course`, `old`, `new` and `drawn` left for
/// the caller), the kept course's runs outside and the partner's runs inside that the join leaves
/// apart, each a course of its own, and whether the joined course may take the partner's lake and
/// its end where the kept course does not start or end outside the outline: always, but where the
/// join chose one of the partner's runs, which must hold the partner's first or last point.
struct JoinedCourse {
    points: Vec<CoursePoint>,
    lowered: Vec<bool>,
    join: CourseJoin,
    apart: Vec<Run>,
    fresh_apart: Vec<Run>,
    fresh_start: bool,
    fresh_end: bool,
}

/// `kept`, a course of the kept land, joined with `fresh`, its partner in the new world
/// (`join_water`, **Courses**), and mended.
fn joined_course(kept: &RiverCourse, fresh: &RiverCourse, weight: &Grid<f32>) -> JoinedCourse {
    let kept_runs = runs(kept, weight);
    let mut fresh_inside: Vec<Run> = runs(fresh, weight).into_iter().filter(|run| run.inside).collect();
    // The kept course's runs as the join lays them and its runs outside it leaves apart, the
    // partner's runs inside it leaves apart, and the partner's run it chose for the kept course's
    // span inside where the two have other numbers of runs inside.
    let mut laid = kept_runs.clone();
    let (mut apart, mut fresh_apart, mut chosen) = (Vec::new(), Vec::new(), None);
    if fresh_inside.len() != kept_runs.iter().filter(|run| run.inside).count() {
        // The kept course's span inside as one run, whose runs outside between its first and last
        // run inside are left apart; in its place the partner's run inside whose first and last
        // points lie nearest the span's (the sum of the two distances, the first of several as near),
        // its other runs inside left apart. So none of the partner's points outside the outline is
        // taken, which the mix would lay on the kept line at weight 0.
        if let (Some(first), Some(last)) = (kept_runs.iter().position(|run| run.inside), kept_runs.iter().rposition(|run| run.inside)) {
            apart = kept_runs[first..=last].iter().filter(|run| !run.inside).copied().collect();
            let span = Run { inside: true, first: kept_runs[first].first, last: kept_runs[last].last };
            laid = (kept_runs[..first].iter().copied()).chain([span]).chain(kept_runs[last + 1..].iter().copied()).collect();
            let apart_m = |a: &CoursePoint, b: &CoursePoint| (f64::from(a.x_m) - f64::from(b.x_m)).hypot(f64::from(a.z_m) - f64::from(b.z_m));
            let ends_apart_m = |run: &Run| apart_m(&fresh.points[run.first], &kept.points[span.first]) + apart_m(&fresh.points[run.last], &kept.points[span.last]);
            let nearest = (0..fresh_inside.len()).reduce(|best, next| if ends_apart_m(&fresh_inside[next]) < ends_apart_m(&fresh_inside[best]) { next } else { best });
            if let Some(best) = nearest {
                fresh_apart = (fresh_inside.iter().enumerate()).filter(|&(at, _)| at != best).map(|(_, run)| *run).collect();
                fresh_inside = vec![fresh_inside[best]];
                chosen = Some(fresh_inside[0]);
            }
        }
    }
    // Where the join chose one run, the joined course holds the partner's first point, or its last,
    // only where that run does.
    let (fresh_start, fresh_end) = chosen.map_or((true, true), |run: Run| (run.first == 0, run.last + 1 == fresh.points.len()));
    // The points in order, each marked kept (true) or new.
    let mut points: Vec<(CoursePoint, bool)> = Vec::new();
    let mut taken = fresh_inside.iter();
    for run in &laid {
        if !run.inside {
            points.extend(kept.points[run.first..=run.last].iter().map(|&point| (point, true)));
        } else if let Some(fresh_run) = taken.next() {
            for &point in &fresh.points[fresh_run.first..=fresh_run.last] {
                let w = f64::from(weight_of(weight, &point));
                let moved = match nearest_on_line(kept, f64::from(point.x_m), f64::from(point.z_m)) {
                    Some(near) if w < 1.0 => {
                        let mix = |o: f64, n: f32| (o + w * (f64::from(n) - o)) as f32;
                        CoursePoint {
                            x_m: mix(near.x_m, point.x_m),
                            z_m: mix(near.z_m, point.z_m),
                            surface_m: mix(near.surface_m, point.surface_m),
                            width_m: mix(near.width_m, point.width_m),
                            depth_m: mix(near.depth_m, point.depth_m),
                        }
                    }
                    _ => point,
                };
                points.push((moved, false));
            }
        }
    }
    let rise = |points: &[(CoursePoint, bool)]| points.windows(2).map(|pair| pair[1].0.surface_m - pair[0].0.surface_m).fold(0.0f32, f32::max);
    // Where it rises most before it is mended: the middle of the first pair of several that rise as
    // much.
    let rise_at = (points.windows(2))
        .map(|pair| (pair[1].0.surface_m - pair[0].0.surface_m, pair))
        .filter(|&(up_m, _)| up_m > 0.0)
        .reduce(|best, next| if next.0 > best.0 { next } else { best })
        .map(|(_, pair)| ((f64::from(pair[0].0.x_m) + f64::from(pair[1].0.x_m)) / 2.0, (f64::from(pair[0].0.z_m) + f64::from(pair[1].0.z_m)) / 2.0));
    let mut join = CourseJoin {
        course: 0,
        old: 0,
        new: 0,
        drawn: None,
        rise_before_m: rise(&points),
        rise_m: 0.0,
        rise_at,
        lowered_most_m: 0.0,
        raised_most_m: 0.0,
        mended_at_weight_1: 0,
        width_step_m: 0.0,
        width_step_own_m: kept.points.windows(2).map(|pair| (pair[1].width_m - pair[0].width_m).abs()).fold(0.0, f32::max),
        gap_m: 0.0,
    };
    // Going downstream, no new point stands above the point before it.
    let mut lowered = vec![false; points.len()];
    let mut previous = f32::INFINITY;
    for ((point, is_kept), lowered) in points.iter_mut().zip(&mut lowered) {
        if !*is_kept && point.surface_m > previous {
            join.lowered_most_m = join.lowered_most_m.max(point.surface_m - previous);
            join.mended_at_weight_1 += usize::from(weight_of(weight, point) == 1.0);
            point.surface_m = previous;
            *lowered = true;
        }
        previous = point.surface_m;
    }
    // Where a kept point follows new points and stands above them, they are raised to it, going
    // upstream, up to the first that stands at or above it.
    for at in 1..points.len() {
        if !points[at].1 || points[at - 1].1 {
            continue;
        }
        let kept_m = points[at].0.surface_m;
        for (point, _) in points[..at].iter_mut().rev().take_while(|(point, is_kept)| !*is_kept && point.surface_m < kept_m) {
            join.raised_most_m = join.raised_most_m.max(kept_m - point.surface_m);
            join.mended_at_weight_1 += usize::from(weight_of(weight, point) == 1.0);
            point.surface_m = kept_m;
        }
    }
    join.rise_m = rise(&points);
    for pair in points.windows(2).filter(|pair| pair[0].1 != pair[1].1) {
        let (a, b) = (pair[0].0, pair[1].0);
        join.width_step_m = join.width_step_m.max((b.width_m - a.width_m).abs());
        join.gap_m = join.gap_m.max((b.x_m - a.x_m).hypot(b.z_m - a.z_m));
    }
    JoinedCourse { points: points.into_iter().map(|(point, _)| point).collect(), lowered, join, apart, fresh_apart, fresh_start, fresh_end }
}

/// The stretches of a world's courses, in buckets of `BUCKET_M` over its cells, each in every bucket
/// its line comes within its half width and `reach_m` of, so the courses within reach of a place
/// are found in its bucket.
struct Stretches<'a> {
    courses: &'a [RiverCourse],
    reach_m: f64,
    half: (f64, f64),
    across: usize,
    down: usize,
    /// Each bucket's stretches, as a course and its first point, in the courses' order.
    buckets: Vec<Vec<(usize, usize)>>,
}

impl<'a> Stretches<'a> {
    fn new(courses: &'a [RiverCourse], (width, height): (usize, usize), reach_m: f64) -> Self {
        let half = (width as f64 / 2.0, height as f64 / 2.0);
        let (across, down) = ((width as f64 / BUCKET_M).ceil() as usize + 1, (height as f64 / BUCKET_M).ceil() as usize + 1);
        let mut buckets = vec![Vec::new(); across * down];
        let bucket = |at: f64, count: usize| ((at / BUCKET_M).floor().max(0.0) as usize).min(count - 1);
        for (index, course) in courses.iter().enumerate() {
            // A course of one point is one stretch from it to itself.
            let stretches = match course.points.len() {
                0 => 0,
                1 => 1,
                points => points - 1,
            };
            for first in 0..stretches {
                let (a, b) = (course.points[first], course.points[(first + 1).min(course.points.len() - 1)]);
                let reach = (f64::from(a.width_m.max(b.width_m)) / 2.0).max(HALF_WIDTH_MIN_M) + reach_m;
                let (x0, x1) = (f64::from(a.x_m.min(b.x_m)) - reach + half.0, f64::from(a.x_m.max(b.x_m)) + reach + half.0);
                let (z0, z1) = (f64::from(a.z_m.min(b.z_m)) - reach + half.1, f64::from(a.z_m.max(b.z_m)) + reach + half.1);
                for row in bucket(z0, down)..=bucket(z1, down) {
                    for column in bucket(x0, across)..=bucket(x1, across) {
                        buckets[row * across + column].push((index, first));
                    }
                }
            }
        }
        Self { courses, reach_m, half, across, down, buckets }
    }

    /// Of the courses whose line comes within half the water's width there (at least
    /// `HALF_WIDTH_MIN_M`) and the reach of world `(x, z)`, the nearest: its index, the first point
    /// of the stretch the nearest place lies on, and the place.
    fn nearest(&self, x: f64, z: f64) -> Option<(usize, usize, Near)> {
        self.nearest_of(x, z, |_| true)
    }

    /// `nearest` among the courses `keep` takes, by index.
    fn nearest_of(&self, x: f64, z: f64, keep: impl Fn(usize) -> bool) -> Option<(usize, usize, Near)> {
        let bucket = |at: f64, count: usize| ((at / BUCKET_M).floor().max(0.0) as usize).min(count - 1);
        let found = &self.buckets[bucket(z + self.half.1, self.down) * self.across + bucket(x + self.half.0, self.across)];
        let mut nearest: Option<(usize, usize, Near)> = None;
        for &(index, first) in found.iter().filter(|&&(index, _)| keep(index)) {
            let points = &self.courses[index].points;
            let near = near_stretch(&points[first], &points[(first + 1).min(points.len() - 1)], x, z);
            if near.apart_m <= near.half_width_m() + self.reach_m && nearest.is_none_or(|(_, _, best)| near.apart_m < best.apart_m) {
                nearest = Some((index, first, near));
            }
        }
        nearest
    }
}

/// The joined land's `water` layer (`join_water`, **The `water` layer**): `joined` holds the joined
/// ground and courses, and `lowered` which points of each of its courses the mending lowered; a
/// course of `old` or `new` marked in `old_dropped` or `new_dropped` is one the join dropped; `new`
/// was grown from `new_drawing`; `lake_cells` holds each joined lake's level and the cells of its
/// water in the world it is taken from. The cells of weight 1 whose water is lowered to a mended
/// course are counted in `report`.
#[allow(clippy::too_many_arguments)]
fn band_water(
    joined: &World,
    old: &World,
    new: &World,
    weight: &Grid<f32>,
    new_drawing: &Drawing,
    lake_cells: &[(f32, Vec<usize>)],
    old_dropped: &[bool],
    new_dropped: &[bool],
    lowered: &[Vec<bool>],
    report: &mut WaterJoin,
) -> Grid<f32> {
    let heights = &joined.heights;
    let size = (heights.width, heights.height);
    let (old_depth, new_depth) = (&old.layers[WATER_LAYER], &new.layers[WATER_LAYER]);
    let beds = Stretches::new(&joined.water.rivers, size, BED_MARGIN_M);
    let old_courses = Stretches::new(&old.water.rivers, size, DROPPED_REACH_M);
    let new_courses = Stretches::new(&new.water.rivers, size, DROPPED_REACH_M);
    // A course that ends on another holds no water inside that one's bed (no farther from its line
    // than its half width), as the fine steps fill the beds (`add_water`): that water is the other
    // course's.
    let inside_what_it_ends_on = |course: usize, x: f64, z: f64| match joined.water.rivers[course].end {
        CourseEnd::River(river) => beds.nearest_of(x, z, |other| other == river).is_some_and(|(_, _, near)| near.apart_m <= near.half_width_m()),
        CourseEnd::Edge | CourseEnd::Lake(_) => false,
    };
    // The stretches of each world's courses, by their points' bits; a joined course's stretch both
    // worlds hold is one the edit left as it was.
    let stretches_of = |world: &World| -> HashSet<([u32; 5], [u32; 5])> {
        (world.water.rivers.iter())
            .flat_map(|course| match course.points.as_slice() {
                [only] => vec![(point_bits(only), point_bits(only))],
                points => points.windows(2).map(|pair| (point_bits(&pair[0]), point_bits(&pair[1]))).collect(),
            })
            .collect()
    };
    let (old_stretches, new_stretches) = (stretches_of(old), stretches_of(new));
    let unchanged = |course: usize, first: usize| {
        let points = &joined.water.rivers[course].points;
        let stretch = (point_bits(&points[first]), point_bits(&points[(first + 1).min(points.len() - 1)]));
        old_stretches.contains(&stretch) && new_stretches.contains(&stretch)
    };
    // The surface a cell inside the outline stands no higher than: where its nearest joined course
    // within half the water's width and `DROPPED_REACH_M` lies on a stretch one of whose points the
    // mending lowered, that course's mended surface there. A course that ends on another ends on one
    // of its points, so where the cell's nearest course is such a one, its nearest place on that
    // course's last stretch lying nearer the last point, the other course's at that point stands for
    // it: the cell is as near the other course's point as the tributary's.
    let lowered_stretch = |course: usize, first: usize| {
        let points = &lowered[course];
        points[first] || points[(first + 1).min(points.len() - 1)]
    };
    let mended = lowered.iter().any(|points| points.contains(&true)).then(|| Stretches::new(&joined.water.rivers, size, DROPPED_REACH_M));
    // The cells where `new` holds a lake's water at its level (`lake_water`, as `Land::hold` and the
    // lakes' zones tell a lake's water), which no cap touches: a lake's water stays at its level, so
    // the lake is found whole again by its level.
    let on_a_lake: Vec<bool> = match mended {
        None => Vec::new(),
        Some(_) => {
            let mut on = vec![false; heights.data.len()];
            for lake in &new.water.lakes {
                let Some(drawn) = new_drawing.lakes.get(lake.drawn) else { continue };
                lake_water(new, drawn, lake.drawn).into_iter().for_each(|cell| on[cell] = true);
            }
            on
        }
    };
    let cap_at = |cell: usize, x: f64, z: f64| -> Option<f32> {
        let courses = mended.as_ref()?;
        if on_a_lake[cell] {
            return None;
        }
        let (index, first, near) = courses.nearest(x, z)?;
        if lowered_stretch(index, first) {
            return Some(near.surface_m as f32);
        }
        let course = &joined.water.rivers[index];
        let (CourseEnd::River(river), [.., before, last]) = (course.end, course.points.as_slice()) else { return None };
        let apart = |point: &CoursePoint| (f64::from(point.x_m) - x).hypot(f64::from(point.z_m) - z);
        if first + 2 != course.points.len() || apart(last) > apart(before) {
            return None;
        }
        let (on, at, mouth) = courses.nearest_of(f64::from(last.x_m), f64::from(last.z_m), |other| other == river)?;
        lowered_stretch(on, at).then_some(mouth.surface_m as f32)
    };
    let mut depth = Grid::new(size.0, size.1, heights.cell_size);
    let mut kept_level = vec![f32::NEG_INFINITY; heights.data.len()];
    for (cell, kept_m) in kept_level.iter_mut().enumerate() {
        let (w, ground_m) = (weight.data[cell], heights.data[cell]);
        if w == 0.0 {
            depth.data[cell] = old_depth.data[cell];
            if depth.data[cell] > 0.0 {
                *kept_m = ground_m + depth.data[cell];
            }
            continue;
        }
        let (x, z) = ((cell % size.0) as f64 - size.0 as f64 / 2.0, (cell / size.0) as f64 - size.1 as f64 / 2.0);
        // The cap on a surface of the cell's, where the surface stands above its ground and the cap.
        let cap_below = |surface_m: f32| if surface_m > ground_m { cap_at(cell, x, z).filter(|&cap_m| cap_m < surface_m) } else { None };
        let capped = |surface_m: f32| cap_below(surface_m).unwrap_or(surface_m);
        if w == 1.0 {
            // The new world's water, or less where the cap lowers it below that depth (a cap within
            // single precision's step of the surface may give the same depth again).
            let own_m = new_depth.data[cell];
            depth.data[cell] = own_m;
            if let Some(capped_m) = cap_below(ground_m + own_m).map(|cap_m| (cap_m - ground_m).max(0.0)).filter(|&capped_m| capped_m < own_m) {
                depth.data[cell] = capped_m;
                report.capped_at_weight_1 += 1;
                report.capped_most_m = report.capped_most_m.max(own_m - capped_m);
            }
            if depth.data[cell] > 0.0 {
                *kept_m = ground_m + depth.data[cell];
            }
            continue;
        }
        // The water both worlds hold on the cell, where they and the joined land hold its ground
        // alike, bit for bit.
        let alike = old.heights.data[cell].to_bits() == ground_m.to_bits()
            && new.heights.data[cell].to_bits() == ground_m.to_bits()
            && old_depth.data[cell].to_bits() == new_depth.data[cell].to_bits();
        if let Some((index, first, near)) = beds.nearest_of(x, z, |index| !inside_what_it_ends_on(index, x, z)) {
            match (alike && unchanged(index, first)).then_some(old_depth.data[cell]) {
                // On a stretch the edit left as it was, the water the fine steps made from it.
                Some(own_m) => match cap_below(ground_m + own_m) {
                    Some(cap_m) if cap_m > ground_m => {
                        depth.data[cell] = cap_m - ground_m;
                        *kept_m = cap_m;
                    }
                    Some(_) => {}
                    None if own_m > 0.0 => {
                        depth.data[cell] = own_m;
                        *kept_m = ground_m + own_m;
                    }
                    None => {}
                },
                None => {
                    let surface_m = capped(near.surface_m as f32);
                    if surface_m > ground_m {
                        depth.data[cell] = surface_m - ground_m;
                        *kept_m = surface_m;
                    }
                }
            }
            continue;
        }
        // Off every bed, the water both worlds hold where it stands at a joined lake's level and they
        // and the joined land hold its ground alike: kept, as the bed's keep rule keeps a bed cell's.
        // The fine steps bounded that spread water once; bounded again by rims worked out again in
        // single precision, it would come down a few steps below the lake's level.
        let own_m = old_depth.data[cell];
        let at_a_lake = |surface_m: f32| joined.water.lakes.iter().any(|lake| (f64::from(surface_m) - f64::from(lake.level_m)).abs() < LAKE_LEVEL_WITHIN_M);
        if alike && own_m > 0.0 && at_a_lake(ground_m + own_m) {
            match cap_below(ground_m + own_m) {
                Some(cap_m) if cap_m > ground_m => {
                    depth.data[cell] = cap_m - ground_m;
                    *kept_m = cap_m;
                }
                Some(_) => {}
                None => {
                    depth.data[cell] = own_m;
                    *kept_m = ground_m + own_m;
                }
            }
            continue;
        }
        let dropped = |courses: &Stretches, dropped: &[bool]| courses.nearest(x, z).is_some_and(|(index, _, _)| dropped[index]);
        // The higher of the two worlds' surfaces, and that world's own water where the joined ground
        // is its ground: the same surface, its depth not worked out again from it.
        let mut higher: Option<(f32, Option<f32>)> = None;
        for (world, own, courses, dropped_of) in [(old, old_depth, &old_courses, old_dropped), (new, new_depth, &new_courses, new_dropped)] {
            let own_m = own.data[cell];
            if own_m > 0.0 && !dropped(courses, dropped_of) {
                let surface_m = world.heights.data[cell] + own_m;
                let own_m = (world.heights.data[cell].to_bits() == ground_m.to_bits()).then_some(own_m);
                if higher.is_none_or(|(best_m, best_own)| surface_m > best_m || (surface_m == best_m && best_own.is_none())) {
                    higher = Some((surface_m, own_m));
                }
            }
        }
        let Some((surface_m, own_m)) = higher else { continue };
        match (cap_below(surface_m), own_m) {
            (Some(cap_m), _) => {
                if cap_m > ground_m {
                    depth.data[cell] = cap_m - ground_m;
                }
            }
            (None, Some(own_m)) => depth.data[cell] = own_m,
            (None, None) => {
                if surface_m > ground_m {
                    depth.data[cell] = surface_m - ground_m;
                }
            }
        }
    }
    // A band cell of a lake's water is kept at the lake's level, or at its own surface where that
    // stands higher, as the fine steps keep a lake's cells (`add_water`), so the ground's bound
    // leaves a lake's water as it is.
    for (level_m, cells) in lake_cells {
        for &cell in cells.iter().filter(|&&cell| weight.data[cell] > 0.0 && weight.data[cell] < 1.0 && depth.data[cell] > 0.0) {
            kept_level[cell] = kept_level[cell].max(*level_m).max(heights.data[cell] + depth.data[cell]);
        }
    }
    bound_spread(heights, &mut depth, &kept_level);
    depth
}

/// The engine's rules for the ground beside and under water, taken on the band's cells alone from
/// `depth`, the joined land's water (`join_water`, **The band's ground**). Returns how many band
/// cells' wetness was raised and how many became wet ground alone.
fn wet_the_band(joined: &mut World, depth: &Grid<f32>, weight: &Grid<f32>, water: &WaterRules) -> (usize, usize) {
    let [_, _, _, wetness_layer] = LAYER_NAMES;
    let band = |cell: usize| weight.data[cell] > 0.0 && weight.data[cell] < 1.0;
    let mut raised = 0;
    if let Some(wetness) = joined.layers.get_mut(wetness_layer) {
        let wet = bank_wetness(&joined.heights, depth, water);
        for cell in (0..wetness.data.len()).filter(|&cell| band(cell)) {
            if wet.data[cell] > wetness.data[cell] {
                wetness.data[cell] = wet.data[cell];
                raised += 1;
            }
        }
    }
    let mut made_wet = 0;
    for cell in (0..joined.ground.data.len()).filter(|&cell| band(cell)) {
        if let Some(wet_alone) = ground_under_water(depth.data[cell]).filter(|&wet_alone| joined.ground.data[cell] != wet_alone) {
            joined.ground.data[cell] = wet_alone;
            made_wet += 1;
        }
    }
    (raised, made_wet)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::country::Country;
    use crate::cover::GroundCell;
    use crate::drawing::{Lake, River};
    use crate::grow::{grow, start_of, Grown, Start};
    use crate::held::unheld;
    use crate::land::join::join_ground;
    use crate::land::model::mixed_model;
    use crate::land::outline::tests::{old_land, EXAMPLE_SEED};
    use crate::land::outline::{footprint, lake_zones, outline, Marks, REACH_MIN_M};
    use crate::trial::{example_edit, standard_edit, trial_drawing, EDIT_NAMES};
    use crate::window::Window;
    use crate::world::{finish_drawn, Frame, GenConfig, Timings, WorldMeta};
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// The hand-made worlds are this many 1 m cells a side; cell `(i, j)` lies at world
    /// `(i - HAND / 2, j - HAND / 2)`.
    const HAND: usize = 256;
    /// Their straight outline: the weight is 0 west of `OUTLINE_X_M`, rises by a smooth step across
    /// the band and is 1 from `INNER_X_M` east.
    const OUTLINE_X_M: f64 = -40.0;
    const INNER_X_M: f64 = 20.0;
    /// The hand-made courses' points lie this many metres apart, as a world's do at most.
    const POINT_SPACING_M: f64 = 4.0;
    /// The hand-made ground stands this high, above every hand-made course's water, but where said.
    const HIGH_GROUND_M: f32 = 30.0;
    /// A hand-made river: its surface where it starts, how much it falls a metre along (little, so
    /// a mix that rises across the band shows), its width and its depth at the deep line.
    const SOURCE_M: f64 = 20.0;
    const FALL_PER_M: f64 = 1e-4;
    const RIVER_WIDTH_M: f32 = 4.0;
    const RIVER_DEPTH_M: f32 = 0.5;
    /// The U the hand-made river runs in, east into the outline along one side and back west out of
    /// it along the other; the new world's runs 6 m inside the kept one's.
    const U_FAR_X_M: f64 = 60.0;
    const U_SIDE_M: f64 = 60.0;
    const U_FROM_X_M: f64 = -110.0;
    const APART_M: f64 = 6.0;
    /// How far above the kept river's surface the new one stands in the mending's tests.
    const ABOVE_M: f64 = 0.05;
    /// The hand-made brooks: their width, and how far the second new brook lies from the kept one.
    const BROOK_WIDTH_M: f32 = 2.0;
    const BROOK_FAR_M: f64 = 40.0;
    /// The ground of the hand-made worlds whose water is given cell by cell, below all of it.
    const LOW_GROUND_M: f32 = 19.0;
    /// How much the new river of the test of the water beside a mended course falls a metre along
    /// inside the outline: enough to stand above the kept river where it enters until some 40 m
    /// past the band's inner edge, from `ABOVE_M` over it there.
    const STEEP_FALL_PER_M: f64 = 5e-4;
    /// That test's pools of water inside the band's inner edge, `x` from `POOL_FROM_X_M` to
    /// `POOL_TO_X_M`: one beside the river, `z` from `SIDE_POOL_Z_M`, standing at the new river's
    /// surface, and one far from it, from `FAR_POOL_Z_M`, at `FAR_POOL_M`; each `POOL_ACROSS_M`
    /// across.
    const POOL_FROM_X_M: f64 = 30.0;
    const POOL_TO_X_M: f64 = 50.0;
    const SIDE_POOL_Z_M: f64 = 3.0;
    const FAR_POOL_Z_M: f64 = 40.0;
    const POOL_ACROSS_M: f64 = 5.0;
    const FAR_POOL_M: f32 = 20.5;
    /// That test's tributary: it flows north into the new river at `TRIBUTARY_X_M`, one of the
    /// river's points the mending lowers, from `TRIBUTARY_FROM_Z_M`, its surface rising this much a
    /// metre upstream from the new river's at its mouth.
    const TRIBUTARY_X_M: f64 = 26.0;
    const TRIBUTARY_FROM_Z_M: f64 = -32.0;
    const TRIBUTARY_RISE_PER_M: f64 = 1e-3;
    /// The lakes the hand-made U brook flows out of and into, beyond its ends, and their radius.
    const U_LAKE_X_M: f64 = -118.0;
    const U_LAKE_RADIUS_M: f32 = 6.0;
    /// A surface is said to have been lowered when it stands more than this below what it was, in
    /// metres: above single precision's step at the hand-made rivers' heights.
    const LOWERED_M: f32 = 1e-5;

    /// The smooth step `t² (3 - 2 t)` of `t` held to 0..1, as the outline's weight rises.
    fn smooth(t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// The world place of hand-made cell `cell`.
    fn place(cell: usize) -> (f64, f64) {
        ((cell % HAND) as f64 - (HAND / 2) as f64, (cell / HAND) as f64 - (HAND / 2) as f64)
    }

    /// The straight outline's weight on the hand-made world's cells.
    fn straight_weight() -> Grid<f32> {
        let data = (0..HAND * HAND).map(|cell| smooth((place(cell).0 - OUTLINE_X_M) / (INNER_X_M - OUTLINE_X_M)) as f32).collect();
        Grid { width: HAND, height: HAND, cell_size: 1.0, data }
    }

    /// A hand-made world in the model's frame: ground `height` and water `depth` at each place,
    /// every layer of a drawn world (all but the water's 0), and `water`.
    fn hand_world(height: impl Fn(f64, f64) -> f32, depth: impl Fn(f64, f64) -> f32, water: Water) -> World {
        let grid = |value: &dyn Fn(f64, f64) -> f32| Grid {
            width: HAND,
            height: HAND,
            cell_size: 1.0,
            data: (0..HAND * HAND).map(|cell| value(place(cell).0, place(cell).1)).collect(),
        };
        let mut layers: BTreeMap<String, Grid<f32>> = LAYER_NAMES.iter().map(|name| (name.to_string(), grid(&|_, _| 0.0))).collect();
        layers.insert(WATER_LAYER.to_string(), grid(&depth));
        let meta = WorldMeta {
            seed: 0,
            source_id: "by hand".to_string(),
            window: Window { source: 0, center_x: 0.0, center_y: 0.0, rotation_rad: 0.0, mirrored: false, size_m: HAND as f64 },
            base_elevation_m: 0.0,
            north: (0.0, -1.0),
            timings: Timings::default(),
        };
        let ground = Grid { width: HAND, height: HAND, cell_size: 1.0, data: vec![GroundCell::default(); HAND * HAND] };
        World { heights: grid(&height), layers, ground, meta, notes: Vec::new(), notes_about: Vec::new(), water }
    }

    /// A course along `corners`, its points every `POINT_SPACING_M` and at each corner, its surface
    /// `surface(along)` at each, `along` the metres from its first point.
    fn course(corners: &[(f64, f64)], surface: impl Fn(f64) -> f64, width_m: f32, drawn: Option<usize>, end: CourseEnd) -> RiverCourse {
        let mut points = Vec::new();
        let mut along_m = 0.0;
        for pair in corners.windows(2) {
            let ((ax, az), (bx, bz)) = (pair[0], pair[1]);
            let length_m = (bx - ax).hypot(bz - az);
            let steps = (length_m / POINT_SPACING_M).ceil() as usize;
            for step in 0..steps {
                let t = step as f64 / steps as f64;
                let here_m = along_m + t * length_m;
                points.push(CoursePoint { x_m: (ax + t * (bx - ax)) as f32, z_m: (az + t * (bz - az)) as f32, surface_m: surface(here_m) as f32, width_m, depth_m: RIVER_DEPTH_M });
            }
            along_m += length_m;
        }
        let &(x, z) = corners.last().expect("a corner");
        points.push(CoursePoint { x_m: x as f32, z_m: z as f32, surface_m: surface(along_m) as f32, width_m, depth_m: RIVER_DEPTH_M });
        RiverCourse { drawn, from_lake: None, points, end }
    }

    /// The U, `inset` metres inside the kept one, its surface `above` metres over the kept one's at
    /// the same place along.
    fn u_river(inset_m: f64, above: impl Fn(f64) -> f64) -> RiverCourse {
        let (far, side) = (U_FAR_X_M - inset_m, U_SIDE_M - inset_m);
        let corners = [(U_FROM_X_M, -side), (far, -side), (far, side), (U_FROM_X_M, side)];
        course(&corners, |along_m| SOURCE_M - FALL_PER_M * along_m + above(along_m), RIVER_WIDTH_M, Some(0), CourseEnd::Edge)
    }

    /// A drawing of the hand-made worlds' size with these rivers and lakes.
    fn hand_drawing(rivers: Vec<River>, lakes: Vec<Lake>) -> Drawing {
        Drawing { size_m: HAND as u32, high_ground: None, rivers, lakes, country: Country::hills() }
    }

    /// A drawn river of two points, the second at `x`: the kept and the edited drawing differ only
    /// in it, so their rivers are one river.
    fn drawn_river(x: f64) -> River {
        River { points: vec![(-100.0, 0.0), (x, 0.0)], width_m: RIVER_WIDTH_M }
    }

    /// `join_water` on a kept world and a new one, the ground joined first.
    fn joined(old: &World, new: &World, weight: &Grid<f32>, old_drawing: &Drawing, new_drawing: &Drawing) -> (World, WaterJoin) {
        let mut joined = join_ground(old, new, weight);
        let join = join_water(&mut joined, old, new, weight, old_drawing, new_drawing, &WaterRules::default());
        (joined, join)
    }

    /// The largest rise of `course`'s surface from one point to the next downstream.
    fn rise_of(course: &RiverCourse) -> f32 {
        course.points.windows(2).map(|pair| pair[1].surface_m - pair[0].surface_m).fold(0.0, f32::max)
    }

    #[test]
    fn a_course_crossing_the_outline_twice_comes_out_one_course_its_kept_points_kept() {
        let weight = straight_weight();
        let (kept_river, new_river) = (u_river(0.0, |_| 0.0), u_river(APART_M, |_| 0.0));
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept_river.clone()], lakes: Vec::new() });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![new_river.clone()], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &hand_drawing(vec![drawn_river(50.0)], Vec::new()), &hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        assert_eq!(world.water.rivers.len(), 1, "one course: {:?}", world.water.rivers.len());
        let course = &world.water.rivers[0];
        assert_eq!((course.drawn, course.from_lake, course.end), (Some(0), None, CourseEnd::Edge));
        // Outside, the kept points bit for bit, in order, first and last included.
        let outside = |river: &RiverCourse| river.points.iter().filter(|point| weight_of(&weight, point) == 0.0).map(point_bits).collect::<Vec<_>>();
        assert_eq!(outside(course), outside(&kept_river));
        assert_eq!(point_bits(&course.points[0]), point_bits(&kept_river.points[0]));
        assert_eq!(point_bits(course.points.last().expect("points")), point_bits(kept_river.points.last().expect("points")));
        // In the band, each point lies its weight's share of the way from the kept line to the new
        // one, which run `APART_M` apart there.
        let mut band_points = 0;
        for point in &course.points {
            let w = f64::from(weight_of(&weight, point));
            let from_kept = nearest_on_line(&kept_river, f64::from(point.x_m), f64::from(point.z_m)).expect("a line").apart_m;
            if w > 0.0 && w < 1.0 {
                band_points += 1;
                assert!((from_kept - APART_M * w).abs() < 0.05, "a band point at weight {w} lies {from_kept} m from the kept line");
            } else if w == 1.0 {
                let on_new = new_river.points.iter().any(|new_point| point_bits(new_point) == point_bits(point));
                assert!(on_new, "a point inside is the new course's own");
            }
        }
        assert!(band_points > 20, "{band_points} band points");
        assert_eq!(rise_of(course), 0.0, "its surface rises");
        assert_eq!(join.crossings, 1);
        assert_eq!((join.rise_most_m, join.lowered_most_m, join.raised_most_m), (0.0, 0.0, 0.0), "{join:?}");
        assert!(join.joins[0].gap_m <= 2.0 * POINT_SPACING_M as f32, "{join:?}");
        assert!(join.brooks_ended.is_empty() && join.brooks_narrowed.is_empty());
    }

    /// The mending. Where the new course stands `ABOVE_M` over the kept one where it enters the
    /// outline, its band points would rise above the kept point before them, which is never raised:
    /// they are lowered to it. Where it stands `ABOVE_M` below the kept one where it leaves, its band
    /// points, rising toward the kept surface, are first lowered to the point before them, and the
    /// kept point after them, which is never lowered, would stand above them: they are raised to it.
    /// Either way the course never rises.
    #[test]
    fn the_band_points_are_mended_and_the_kept_points_are_not() {
        let weight = straight_weight();
        let kept_river = u_river(0.0, |_| 0.0);
        let drawings = (hand_drawing(vec![drawn_river(50.0)], Vec::new()), hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept_river.clone()], lakes: Vec::new() });
        let along_u = 2.0 * (U_FAR_X_M - U_FROM_X_M) + 2.0 * U_SIDE_M;
        for (case, above) in [("above where it enters", 1.0), ("below where it leaves", -1.0)] {
            // Above by `ABOVE_M` all along, or below by it on the way out, nothing on the way in.
            let offset = move |along_m: f64| if above > 0.0 { ABOVE_M } else { -ABOVE_M * smooth((along_m - along_u / 2.0) / 40.0) };
            let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![u_river(APART_M, offset)], lakes: Vec::new() });
            let (world, join) = joined(&old, &new, &weight, &drawings.0, &drawings.1);
            let course = &world.water.rivers[0];
            println!("{case}: {join:?}");
            assert_eq!(rise_of(course), 0.0, "{case}: the surface rises");
            let outside = |river: &RiverCourse| river.points.iter().filter(|point| weight_of(&weight, point) == 0.0).map(point_bits).collect::<Vec<_>>();
            assert_eq!(outside(course), outside(&kept_river), "{case}: the kept points");
            assert_eq!(join.rise_most_m, 0.0, "{case}");
            if above > 0.0 {
                assert!(join.lowered_most_m > 0.8 * ABOVE_M as f32 && join.lowered_most_m <= ABOVE_M as f32, "{case}: lowered {} m", join.lowered_most_m);
                assert!(join.joins[0].rise_before_m > 0.0, "{case}");
            } else {
                assert!(join.raised_most_m > 0.0 && join.raised_most_m <= ABOVE_M as f32, "{case}: raised {} m", join.raised_most_m);
                // The last point before the course leaves stands at the kept point after it.
                let leaving = (1..course.points.len()).find(|&at| weight_of(&weight, &course.points[at]) == 0.0 && weight_of(&weight, &course.points[at - 1]) > 0.0 && at > course.points.len() / 2).expect("the course leaves");
                assert_eq!(course.points[leaving - 1].surface_m, course.points[leaving].surface_m, "{case}");
            }
        }
    }

    /// A lake whose middle lies outside the outline is the kept land's bit for bit, its cells' water
    /// too; one whose middle lies inside is the new world's.
    #[test]
    fn a_lake_outside_is_kept_and_a_lake_inside_is_the_new_one() {
        let weight = straight_weight();
        const WEST: (f64, f64) = (-90.0, 0.0);
        const EAST: (f64, f64) = (70.0, 0.0);
        const RADIUS_M: f64 = 15.0;
        let on = |centre: (f64, f64), x: f64, z: f64| (x - centre.0).hypot(z - centre.1) <= RADIUS_M;
        let lake = |drawn: usize, (x, z): (f64, f64), level_m: f32| LakeWater { drawn, level_m, x_m: x as f32, z_m: z as f32, area_m2: 700.0 };
        let world = |levels: (f32, f32)| {
            hand_world(
                |_, _| 10.0,
                move |x, z| if on(WEST, x, z) { levels.0 - 10.0 } else if on(EAST, x, z) { levels.1 - 10.0 } else { 0.0 },
                Water { rivers: Vec::new(), lakes: vec![lake(0, WEST, levels.0), lake(1, EAST, levels.1)] },
            )
        };
        let (old, new) = (world((11.0, 12.0)), world((11.5, 12.5)));
        let lakes = vec![Lake { x_m: WEST.0, z_m: WEST.1, radius_m: RADIUS_M as f32 }, Lake { x_m: EAST.0, z_m: EAST.1, radius_m: RADIUS_M as f32 }];
        let drawing = hand_drawing(Vec::new(), lakes);
        let (joined, _) = joined(&old, &new, &weight, &drawing, &drawing);
        assert_eq!(joined.water.lakes, vec![old.water.lakes[0], new.water.lakes[1]]);
        for cell in 0..HAND * HAND {
            let (x, z) = place(cell);
            let layer = |world: &World| world.layers[WATER_LAYER].data[cell].to_bits();
            if on(WEST, x, z) {
                assert_eq!(layer(&joined), layer(&old), "the kept lake's cell {cell}");
            } else if on(EAST, x, z) {
                assert_eq!(layer(&joined), layer(&new), "the new lake's cell {cell}");
            }
        }
    }

    /// Two brooks crossing the outline, a kept one along the middle row and a new one `apart_m` from
    /// it, each 2 m wide: their worlds and the join.
    fn two_brooks(apart_m: f64) -> (World, World, WaterJoin) {
        let weight = straight_weight();
        let brook = |z: f64| course(&[(U_FROM_X_M, z), (U_FAR_X_M, z)], |along_m| SOURCE_M - FALL_PER_M * along_m, BROOK_WIDTH_M, None, CourseEnd::Edge);
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![brook(0.0)], lakes: Vec::new() });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![brook(apart_m)], lakes: Vec::new() });
        let drawing = hand_drawing(Vec::new(), Vec::new());
        let (world, join) = joined(&old, &new, &weight, &drawing, &drawing);
        (old, world, join)
    }

    #[test]
    fn brooks_near_in_the_band_are_one_and_brooks_far_apart_are_two() {
        let weight = straight_weight();
        let (_, near, join) = two_brooks(APART_M);
        assert_eq!(near.water.rivers.len(), 1, "two brooks 6 m apart are one");
        assert_eq!((join.brooks_crossing, join.brooks_paired, join.crossings), ((1, 1), 1, 1), "{join:?}");
        assert!(join.brooks_narrowed.is_empty() && join.brooks_ended.is_empty());

        let (old, far, join) = two_brooks(BROOK_FAR_M);
        assert_eq!(far.water.rivers.len(), 2, "two brooks 40 m apart are two");
        assert_eq!((join.brooks_crossing, join.brooks_paired, join.crossings), ((1, 1), 0, 0), "{join:?}");
        // The kept one ends at the outline, its points outside kept; the new one, its points inside
        // alone, narrows to nothing toward the outline.
        let ended = &far.water.rivers[0];
        assert!(ended.points.iter().all(|point| weight_of(&weight, point) == 0.0));
        assert_eq!(ended.points.len(), old.water.rivers[0].points.iter().filter(|point| weight_of(&weight, point) == 0.0).count());
        let narrowed = &far.water.rivers[1];
        assert!(narrowed.points.iter().all(|point| weight_of(&weight, point) > 0.0));
        for point in &narrowed.points {
            assert_eq!(point.width_m, BROOK_WIDTH_M * weight_of(&weight, point), "a narrowed point's width");
        }
        assert!(narrowed.points[0].width_m < 0.01, "the new brook starts {} m wide at the outline", narrowed.points[0].width_m);
        let at_outline = |(x, _): (f64, f64)| (x - OUTLINE_X_M).abs() <= POINT_SPACING_M;
        assert!(join.brooks_ended.len() == 1 && at_outline(join.brooks_ended[0]), "{:?}", join.brooks_ended);
        assert!(join.brooks_narrowed.len() == 1 && at_outline(join.brooks_narrowed[0]), "{:?}", join.brooks_narrowed);
        assert!((join.brooks_narrowed[0].1 - BROOK_FAR_M).abs() < 1e-9 && join.brooks_ended[0].1.abs() < 1e-9);
    }

    /// Brooks flowing west out of the outline along the middle row: a straight one from a head
    /// inside the outline, and one whose head lies in the band at `HOOK_X_M`, `offset` metres off
    /// the row, from where it turns west along the row. At their middles in the band both lie on the
    /// row. A new hooked brook whose head lies 25 m off the kept straight line, beyond its width,
    /// `BROOK_PAIR_M` and `HEAD_SLACK_M` (21.3 m), is not one with it: the kept one ends at the
    /// outline and the new one narrows. One whose head lies 15 m off, within the slack, is one with
    /// it; and a kept hooked brook is one with a new straight one, its head 25 m off or not, since the
    /// join drops the kept brook's band points.
    #[test]
    fn a_new_brooks_head_in_the_band_must_lie_near_the_kept_line_and_a_kept_head_need_not() {
        const STRAIGHT_HEAD_X_M: f64 = 40.0;
        const HOOK_X_M: f64 = 10.0;
        const FAR_M: f64 = 25.0;
        const NEAR_M: f64 = 15.0;
        let weight = straight_weight();
        let w_hook = weight_at(&weight, HOOK_X_M, 0.0);
        assert!(w_hook > 0.0 && w_hook < 1.0, "the hook's head lies in the band");
        let flowing = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let straight = || course(&[(STRAIGHT_HEAD_X_M, 0.0), (U_FROM_X_M, 0.0)], flowing, BROOK_WIDTH_M, None, CourseEnd::Edge);
        let hooked = |offset_m: f64| course(&[(HOOK_X_M, offset_m), (HOOK_X_M, 0.0), (U_FROM_X_M, 0.0)], flowing, BROOK_WIDTH_M, None, CourseEnd::Edge);
        let drawing = hand_drawing(Vec::new(), Vec::new());
        let cases = [
            ("a new head far off the kept line", straight(), hooked(FAR_M), false),
            ("a new head within the slack", straight(), hooked(NEAR_M), true),
            ("a kept head far off the new line", hooked(FAR_M), straight(), true),
        ];
        for (case, kept, fresh, one) in cases {
            let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept], lakes: Vec::new() });
            let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![fresh], lakes: Vec::new() });
            let (world, join) = joined(&old, &new, &weight, &drawing, &drawing);
            assert_eq!(join.brooks_crossing, (1, 1), "{case}");
            assert_eq!((join.brooks_paired, world.water.rivers.len()), if one { (1, 1) } else { (0, 2) }, "{case}: {join:?}");
            assert_eq!((join.brooks_ended.len(), join.brooks_narrowed.len()), if one { (0, 0) } else { (1, 1) }, "{case}");
        }
    }

    /// A brook in the band ends in a lake inside the outline, its water in a trench the ground holds
    /// and joined to the lake's. The kept land has it and the new world does not, so the join drops
    /// it, and its water with it; where the new world has it too, its water stays.
    #[test]
    fn a_course_the_join_drops_takes_its_water_with_it() {
        let weight = straight_weight();
        const LAKE_LEVEL_M: f32 = 10.0;
        const TRENCH_M: f32 = 9.0;
        const LAKE_FLOOR_M: f32 = 8.0;
        const BANK_M: f32 = 12.0;
        let in_lake = |x: f64, z: f64| (INNER_X_M + 2.0..=60.0).contains(&x) && z.abs() <= 20.0;
        let in_trench = |x: f64, z: f64| (-30.0..INNER_X_M + 2.0).contains(&x) && z.abs() <= 1.0;
        let height = move |x: f64, z: f64| if in_lake(x, z) { LAKE_FLOOR_M } else if in_trench(x, z) { TRENCH_M } else { BANK_M };
        let lake = LakeWater { drawn: 0, level_m: LAKE_LEVEL_M, x_m: 41.0, z_m: 0.0, area_m2: 1600.0 };
        let brook = course(&[(-30.0, 0.0), (INNER_X_M + 2.0, 0.0)], |_| f64::from(LAKE_LEVEL_M), BROOK_WIDTH_M, None, CourseEnd::Lake(0));
        let depth = move |with_brook: bool| move |x: f64, z: f64| if in_lake(x, z) { LAKE_LEVEL_M - LAKE_FLOOR_M } else if with_brook && in_trench(x, z) { LAKE_LEVEL_M - TRENCH_M } else { 0.0 };
        let old = hand_world(height, depth(true), Water { rivers: vec![brook.clone()], lakes: vec![lake] });
        let drawing = hand_drawing(Vec::new(), vec![Lake { x_m: 41.0, z_m: 0.0, radius_m: 20.0 }]);
        let trench: Vec<usize> = (0..HAND * HAND).filter(|&cell| in_trench(place(cell).0, place(cell).1) && weight.data[cell] < 1.0).collect();
        let wet = |world: &World| trench.iter().filter(|&&cell| world.layers[WATER_LAYER].data[cell] > 0.0).count();

        let without = hand_world(height, depth(false), Water { rivers: Vec::new(), lakes: vec![lake] });
        let (dropped, join) = joined(&old, &without, &weight, &drawing, &drawing);
        assert!(dropped.water.rivers.is_empty() && join.crossings == 0, "{join:?}");
        assert_eq!(wet(&dropped), 0, "the dropped brook's water stays in the trench");
        assert_eq!(dropped.water.lakes, vec![lake]);

        let with = hand_world(height, depth(true), Water { rivers: vec![brook], lakes: vec![lake] });
        let (joined_brook, join) = joined(&old, &with, &weight, &drawing, &drawing);
        assert_eq!((joined_brook.water.rivers.len(), join.crossings, join.joined_inside), (1, 0, 1), "{join:?}");
        assert_eq!(joined_brook.water.rivers[0].end, CourseEnd::Lake(0));
        assert_eq!(wet(&joined_brook), trench.len(), "the joined brook's water");
    }

    /// The water beside a mended course. A drawn river runs east along the middle row, and the new
    /// one along the same line stands `ABOVE_M` over the kept one where it enters the outline and
    /// falls `STEEP_FALL_PER_M` from there, so the mending lowers it to the kept point before the
    /// band until some 40 m past the band's inner edge, where the new world's water would stand above
    /// the course and rise going out of the band. Each world's water lies on its own river's bed at
    /// its surface, and the new world's also in a pool beside its river, in one far from it, and in
    /// the bed of a tributary that ends on a lowered point of its river, its last stretch running
    /// into the river's bed. Inside the band's inner edge, the water within reach of the stretches
    /// the mending lowered stands no higher than the mended course, on the river's bed beside the
    /// tributary's mouth too, so going down the course the water's surface never rises; every other
    /// cell there holds the new world's water bit for bit.
    #[test]
    fn the_water_inside_stands_no_higher_than_a_course_the_mending_lowered() {
        let weight = straight_weight();
        let half_m = f64::from(RIVER_WIDTH_M) / 2.0;
        let kept_surface = |x: f64| SOURCE_M - FALL_PER_M * (x - U_FROM_X_M);
        let new_surface = move |x: f64| kept_surface(OUTLINE_X_M) + ABOVE_M - STEEP_FALL_PER_M * (x - OUTLINE_X_M);
        let river = |surface: &dyn Fn(f64) -> f64| course(&[(U_FROM_X_M, 0.0), (-U_FROM_X_M, 0.0)], |along_m| surface(U_FROM_X_M + along_m), RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let on_bed = move |z: f64| z.abs() <= half_m;
        let in_pool = |x: f64, z: f64, from_z: f64| (POOL_FROM_X_M..=POOL_TO_X_M).contains(&x) && (from_z..=from_z + POOL_ACROSS_M).contains(&z);
        let above_ground = |surface_m: f64| (surface_m - f64::from(LOW_GROUND_M)) as f32;
        let tributary_surface = move |z: f64| new_surface(TRIBUTARY_X_M) - TRIBUTARY_RISE_PER_M * z;
        let in_tributary = |x: f64, z: f64| (x - TRIBUTARY_X_M).abs() <= f64::from(BROOK_WIDTH_M) / 2.0 && (TRIBUTARY_FROM_Z_M..=0.0).contains(&z);
        let tributary = course(&[(TRIBUTARY_X_M, TRIBUTARY_FROM_Z_M), (TRIBUTARY_X_M, 0.0)], |along_m| tributary_surface(TRIBUTARY_FROM_Z_M + along_m), BROOK_WIDTH_M, None, CourseEnd::River(0));
        let old_depth = move |x: f64, z: f64| if on_bed(z) { above_ground(kept_surface(x)) } else { 0.0 };
        let new_depth = move |x: f64, z: f64| {
            if on_bed(z) || in_pool(x, z, SIDE_POOL_Z_M) {
                above_ground(new_surface(x))
            } else if in_tributary(x, z) {
                above_ground(tributary_surface(z))
            } else if in_pool(x, z, FAR_POOL_Z_M) {
                FAR_POOL_M - LOW_GROUND_M
            } else {
                0.0
            }
        };
        let old = hand_world(|_, _| LOW_GROUND_M, old_depth, Water { rivers: vec![river(&kept_surface)], lakes: Vec::new() });
        let new = hand_world(|_, _| LOW_GROUND_M, new_depth, Water { rivers: vec![river(&new_surface), tributary], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &hand_drawing(vec![drawn_river(50.0)], Vec::new()), &hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        assert_eq!(world.water.rivers.len(), 2, "the river and its tributary");
        assert_eq!(world.water.rivers[1].end, CourseEnd::River(0));
        let course = &world.water.rivers[0];
        assert!(join.joins[0].mended_at_weight_1 > 0, "the mending reaches past the band's inner edge: {join:?}");
        assert!(join.capped_at_weight_1 > 0 && join.capped_most_m > 0.0, "{join:?}");

        // The points the mending lowered: those standing below the mix `o + w (n - o)` of the two
        // rivers' surfaces at their place, which the join gives a point of the band before mending.
        let lowered: Vec<&CoursePoint> = (course.points.iter())
            .filter(|point| {
                let (x, w) = (f64::from(point.x_m), f64::from(weight_of(&weight, point)));
                w > 0.0 && point.surface_m < (kept_surface(x) + w * (new_surface(x) - kept_surface(x))) as f32 - LOWERED_M
            })
            .collect();
        assert!(lowered.iter().any(|point| weight_of(&weight, point) == 1.0), "a point inside the band's inner edge is lowered");
        assert!(lowered.iter().any(|point| f64::from(point.x_m) == TRIBUTARY_X_M), "the tributary's mouth is lowered");
        let near_lowered = |x: f64, z: f64| lowered.iter().any(|point| (f64::from(point.x_m) - x).hypot(f64::from(point.z_m) - z) <= half_m + DROPPED_REACH_M + POINT_SPACING_M);

        // Inside the band's inner edge, away from the lowered points the new world's water bit for
        // bit, the far pool among it; near them, water lowered to the course and no higher.
        let (joined_depth, new_layer) = (&world.layers[WATER_LAYER], &new.layers[WATER_LAYER]);
        let (mut kept_away, mut capped) = (0, 0);
        for cell in (0..HAND * HAND).filter(|&cell| weight.data[cell] == 1.0) {
            let (x, z) = place(cell);
            let (now, was) = (joined_depth.data[cell], new_layer.data[cell]);
            if !near_lowered(x, z) {
                assert_eq!(now.to_bits(), was.to_bits(), "cell {cell} at ({x}, {z}), away from the lowered points");
                kept_away += usize::from(was > 0.0);
            } else if now.to_bits() != was.to_bits() {
                capped += 1;
                let course_m = nearest_on_line(course, x, z).expect("a line").surface_m as f32;
                let surface_m = world.heights.data[cell] + now;
                assert!(now < was && surface_m <= course_m + LOWERED_M, "cell {cell} at ({x}, {z}) stands at {surface_m} m, the course beside it at {course_m} m");
            }
            if in_pool(x, z, FAR_POOL_Z_M) {
                assert!(now > 0.0 && now.to_bits() == was.to_bits(), "the far pool's cell {cell} at ({x}, {z})");
            }
            if in_pool(x, z, SIDE_POOL_Z_M) {
                assert!(now < was, "the side pool's cell {cell} at ({x}, {z}) is lowered");
            }
            if on_bed(z) && now > 0.0 && near_lowered(x, z) {
                let course_m = nearest_on_line(course, x, z).expect("a line").surface_m as f32;
                let surface_m = world.heights.data[cell] + now;
                assert!(surface_m <= course_m + LOWERED_M, "the river's bed at cell {cell} at ({x}, {z}) stands at {surface_m} m, the river at {course_m} m");
            }
        }
        assert_eq!(capped, join.capped_at_weight_1, "the cells of weight 1 lowered");
        assert!(kept_away > 0, "water away from the lowered points");

        // Going down the course, the water's surface never rises: read at each of its points' cells,
        // and cell by cell along its row from where the weight is above 0.
        let surface_at = |cell: usize| (joined_depth.data[cell] > 0.0).then(|| world.heights.data[cell] + joined_depth.data[cell]);
        let row = HAND / 2 * HAND;
        let along_row: Vec<f32> = (row..row + HAND).filter(|&cell| weight.data[cell] > 0.0).filter_map(surface_at).collect();
        let cell_of = |point: &CoursePoint| row + (f64::from(point.x_m) + (HAND / 2) as f64).round() as usize;
        let at_points: Vec<f32> = course.points.iter().map(cell_of).filter_map(surface_at).collect();
        for (reading, surfaces) in [("along the row", &along_row), ("at the points", &at_points)] {
            let rise = surfaces.windows(2).map(|pair| pair[1] - pair[0]).fold(0.0f32, f32::max);
            assert_eq!(rise, 0.0, "the water's surface rises {rise} m {reading}");
        }
    }

    /// The runs of `course` whose points' weight `keep` takes, each its points in order.
    fn runs_where(course: &RiverCourse, weight: &Grid<f32>, keep: impl Fn(f32) -> bool) -> Vec<Vec<CoursePoint>> {
        let mut found: Vec<Vec<CoursePoint>> = vec![Vec::new()];
        for point in &course.points {
            if keep(weight_of(weight, point)) {
                found.last_mut().expect("a run").push(*point);
            } else if found.last().is_some_and(|run| !run.is_empty()) {
                found.push(Vec::new());
            }
        }
        found.into_iter().filter(|run| !run.is_empty()).collect()
    }

    /// A course that leaves the outline and comes back is a course for each run, so none bridges the
    /// outline with a straight stretch (the U brook, the review's case). The kept U brook with no
    /// partner, its water in its bed, ends at the outline as two courses: the first out of the lake
    /// it flows out of, the second into the lake it flows into. A new brook that leaves the outline
    /// and comes back in, with no partner, is narrowed as two courses. A kept river that leaves the
    /// outline and comes back in, joined with a partner wholly inside the outline, is that partner's
    /// course, and its run outside is a course of its own after it.
    #[test]
    fn a_course_that_leaves_the_outline_and_comes_back_is_split_at_each_run() {
        let weight = straight_weight();
        let longest = |world: &World| (world.water.rivers.iter()).flat_map(|course| course.points.windows(2)).map(|pair| (pair[1].x_m - pair[0].x_m).hypot(pair[1].z_m - pair[0].z_m)).fold(0.0f32, f32::max);
        const STRETCH_MOST_M: f32 = POINT_SPACING_M as f32 + 1e-3;
        let at_outline = |(x, _): (f64, f64)| (x - OUTLINE_X_M).abs() <= POINT_SPACING_M;
        let bed = |river: &RiverCourse| {
            let river = river.clone();
            move |x: f64, z: f64| if nearest_on_line(&river, x, z).is_some_and(|near| near.apart_m <= near.half_width_m()) { 1.0 } else { 0.0 }
        };
        let bits_of = |courses: &[RiverCourse]| courses.iter().map(|course| course.points.iter().map(point_bits).collect::<Vec<_>>()).collect::<Vec<_>>();
        let runs_bits = |runs: Vec<Vec<CoursePoint>>| runs.iter().map(|run| run.iter().map(point_bits).collect::<Vec<_>>()).collect::<Vec<_>>();
        let flowing = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        // In from the band's far side, out west to `U_FROM_X_M` and back.
        let inverted_u = |drawn: Option<usize>, width_m: f32| {
            let corners = [(U_FAR_X_M, -U_SIDE_M), (U_FROM_X_M, -U_SIDE_M), (U_FROM_X_M, U_SIDE_M), (U_FAR_X_M, U_SIDE_M)];
            course(&corners, flowing, width_m, drawn, CourseEnd::Edge)
        };

        // The kept U brook, with no partner.
        let lake = |drawn: usize, z_m: f64| LakeWater { drawn, level_m: SOURCE_M as f32, x_m: U_LAKE_X_M as f32, z_m: z_m as f32, area_m2: 100.0 };
        let lakes = vec![lake(0, U_SIDE_M), lake(1, -U_SIDE_M)];
        let drawn_lake = |z_m: f64| Lake { x_m: U_LAKE_X_M, z_m, radius_m: U_LAKE_RADIUS_M };
        let drawing = hand_drawing(Vec::new(), vec![drawn_lake(U_SIDE_M), drawn_lake(-U_SIDE_M)]);
        let u_brook = RiverCourse { drawn: None, from_lake: Some(1), end: CourseEnd::Lake(0), ..u_river(0.0, |_| 0.0) };
        let old = hand_world(|_, _| LOW_GROUND_M, bed(&u_brook), Water { rivers: vec![u_brook.clone()], lakes: lakes.clone() });
        let new = hand_world(|_, _| LOW_GROUND_M, |_, _| 0.0, Water { rivers: Vec::new(), lakes });
        let (world, join) = joined(&old, &new, &weight, &drawing, &drawing);
        let rivers = &world.water.rivers;
        assert_eq!(bits_of(rivers), runs_bits(runs_where(&u_brook, &weight, |w| w == 0.0)), "the kept brook's two runs outside, each a course");
        assert_eq!(rivers.len(), 2);
        assert_eq!([(rivers[0].from_lake, rivers[0].end), (rivers[1].from_lake, rivers[1].end)], [(Some(1), CourseEnd::Edge), (None, CourseEnd::Lake(0))]);
        assert!(longest(&world) <= STRETCH_MOST_M, "a stretch {} m long", longest(&world));
        assert!(join.brooks_ended.len() == 2 && join.brooks_ended.iter().copied().all(at_outline), "{:?}", join.brooks_ended);
        assert!(join.brooks_ended[0].1 < 0.0 && join.brooks_ended[1].1 > 0.0, "from upstream: {:?}", join.brooks_ended);

        // A new brook that leaves the outline and comes back in, with no partner.
        let brook = inverted_u(None, BROOK_WIDTH_M);
        let drawing = hand_drawing(Vec::new(), Vec::new());
        let old = hand_world(|_, _| LOW_GROUND_M, |_, _| 0.0, Water::default());
        let new = hand_world(|_, _| LOW_GROUND_M, bed(&brook), Water { rivers: vec![brook.clone()], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &drawing, &drawing);
        let narrowed = |run: Vec<CoursePoint>| {
            let at = |point: &CoursePoint| weight_of(&weight, point);
            run.iter().map(|point| CoursePoint { width_m: point.width_m * at(point), depth_m: point.depth_m * at(point), ..*point }).collect()
        };
        let expected: Vec<Vec<CoursePoint>> = runs_where(&brook, &weight, |w| w > 0.0).into_iter().map(narrowed).collect();
        assert_eq!(expected.len(), 2);
        assert_eq!(bits_of(&world.water.rivers), runs_bits(expected), "the new brook's two runs inside, each a course, narrowed");
        assert!(world.water.rivers.iter().all(|course| course.from_lake.is_none() && course.end == CourseEnd::Edge));
        assert!(longest(&world) <= STRETCH_MOST_M, "a stretch {} m long", longest(&world));
        assert!(join.brooks_narrowed.len() == 2 && join.brooks_narrowed.iter().copied().all(at_outline), "{:?}", join.brooks_narrowed);

        // A kept river that leaves the outline and comes back in, joined with a partner wholly inside.
        let kept_river = inverted_u(Some(0), RIVER_WIDTH_M);
        let partner = course(&[(U_FAR_X_M, -U_SIDE_M), (U_FAR_X_M, U_SIDE_M)], flowing, RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept_river.clone()], lakes: Vec::new() });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![partner.clone()], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &hand_drawing(vec![drawn_river(50.0)], Vec::new()), &hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        let mut expected = vec![partner.points.clone()];
        expected.extend(runs_where(&kept_river, &weight, |w| w == 0.0));
        assert_eq!(bits_of(&world.water.rivers), runs_bits(expected), "the partner, then the kept river's run outside");
        assert_eq!((world.water.rivers[1].drawn, world.water.rivers[1].from_lake, world.water.rivers[1].end), (Some(0), None, CourseEnd::Edge));
        assert_eq!((join.crossings, join.joined_inside, join.joins[0].course), (1, 0, 0), "{join:?}");
        assert!(longest(&world) <= STRETCH_MOST_M, "a stretch {} m long", longest(&world));
    }

    /// A kept land as a land keeps it: its drawing, start, model and world in the model's frame.
    pub(crate) struct Kept {
        pub(crate) drawing: Drawing,
        pub(crate) start: Start,
        pub(crate) model: Grown,
        pub(crate) world: World,
    }

    pub(crate) fn kept_land(drawing: Drawing, seed: u64) -> Kept {
        let config = GenConfig::default();
        let start = start_of(&drawing, &config.water, seed).expect("a valid drawing");
        let model = grow(&drawing, &config.water, seed, &AtomicBool::new(false)).expect("the drawing grows");
        let world = finish_drawn(seed, &drawing, &config, &model, Frame::Model, &AtomicBool::new(false)).expect("its world");
        Kept { drawing, start, model, world }
    }

    /// One edit of a kept land through the whole chain: the edited drawing's model grown whole, the
    /// outline with its lakes' zones (no marks, the least reach), the mixed model and its world in
    /// the model's frame, the joined ground and then the joined water; and the edited drawing grown
    /// whole, to judge by.
    pub(crate) struct Chain {
        pub(crate) weight: Grid<f32>,
        pub(crate) mixed: World,
        pub(crate) ground_joined: World,
        pub(crate) joined: World,
        pub(crate) whole: World,
        pub(crate) join: WaterJoin,
        pub(crate) join_ms: f64,
    }

    pub(crate) fn chain(drawing: &Drawing, start: &Start, model: &Grown, world: &World, edited: &Drawing, seed: u64) -> Chain {
        let config = GenConfig::default();
        let cancel = AtomicBool::new(false);
        let new_model = grow(edited, &config.water, seed, &cancel).expect("the edited drawing grows");
        let new_start = start_of(edited, &config.water, seed).expect("the edited drawing is valid");
        let found = footprint(start, &new_start, drawing, edited);
        let zones = lake_zones(world, model, &new_model, drawing, edited);
        let shape = outline(edited.size_m, &found, &zones, &Marks::none(world.heights.width), REACH_MIN_M);
        let mixed_model = mixed_model(model, &new_model, edited, &config.water, seed, &shape.model_weight).expect("the mixed model");
        let mixed = finish_drawn(seed, edited, &config, &mixed_model.model, Frame::Model, &cancel).expect("the mixed model's world");
        let ground_joined = join_ground(world, &mixed, &shape.weight);
        let mut joined = ground_joined.clone();
        let clock = Instant::now();
        let join = join_water(&mut joined, world, &mixed, &shape.weight, drawing, edited, &config.water);
        let join_ms = clock.elapsed().as_secs_f64() * 1000.0;
        let whole = finish_drawn(seed, edited, &config, &new_model, Frame::Model, &cancel).expect("the world grown whole");
        Chain { weight: shape.weight, mixed, ground_joined, joined, whole, join, join_ms }
    }

    /// The most points of a joined land's courses `assert_kept_outside` allows at weight 0 that are
    /// not the kept land's: the one class W1 read both ways still finds (this module's review, N1; the
    /// land's tests bound it the same, `tests/land.rs`). Where the join gives a course of the band its
    /// kept line, a point of the band next to the outline may be moved onto a cell of weight 0 just
    /// outside it, beside its course's next point in the band. Measured: one point, on the trial
    /// `repeated` at 1,024 m in hills from seed 3, and none on this module's chains.
    const NEW_POINTS_AT_WEIGHT_0_MOST: usize = 1;

    /// W1 and amendment 6 on a chain from the kept land `old` (of `drawing`) to `edited`: every
    /// course point and lake of the kept land outside the outline is the joined land's bit for bit
    /// (a lake's `drawn` naming its place in `edited`), and every course point of the joined land
    /// outside it the kept land's, but at most `NEW_POINTS_AT_WEIGHT_0_MOST` of its class (each
    /// beside a point of its course at weight above 0); every `water` cell of weight 0 the kept
    /// land's, every lake taken the mixed world's, and every height, ground cell and layer of weight
    /// 0 the kept land's; and every `water` cell of weight 1 the mixed world's but those lowered to
    /// a mended course, each of which stands below the mixed world's water within half the water's
    /// width and `DROPPED_REACH_M` of a stretch of a joined course the mending lowered, as many as
    /// `WaterJoin::capped_at_weight_1` says. Returns how many kept points were checked.
    pub(crate) fn assert_kept_outside(name: &str, drawing: &Drawing, old: &World, edited: &Drawing, chain: &Chain) -> usize {
        let weight = &chain.weight;
        let joined_points: BTreeSet<[u32; 5]> = chain.joined.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
        let outside: Vec<&CoursePoint> = old.water.rivers.iter().flat_map(|course| &course.points).filter(|point| weight_of(weight, point) == 0.0).collect();
        let missing = outside.iter().filter(|point| !joined_points.contains(&point_bits(point))).count();
        assert_eq!(missing, 0, "{name}: {missing} of the kept land's {} course points outside the outline are missing", outside.len());
        let kept_points: BTreeSet<[u32; 5]> = old.water.rivers.iter().flat_map(|course| course.points.iter().map(point_bits)).collect();
        let mut moved = 0;
        for course in &chain.joined.water.rivers {
            let at_0 = |point: &CoursePoint| weight_of(weight, point) == 0.0;
            for (k, point) in course.points.iter().enumerate().filter(|&(_, point)| at_0(point) && !kept_points.contains(&point_bits(point))) {
                moved += 1;
                let beside_the_band = [k.wrapping_sub(1), k + 1].into_iter().filter_map(|n| course.points.get(n)).any(|next| !at_0(next));
                assert!(beside_the_band, "{name}: a joined point of weight 0 not the kept land's, at ({}, {}), away from the band", point.x_m, point.z_m);
            }
        }
        assert!(moved <= NEW_POINTS_AT_WEIGHT_0_MOST, "{name}: {moved} joined course points at weight 0 are not the kept land's");
        let partner = lake_partners(drawing, edited);
        for lake in old.water.lakes.iter().filter(|lake| weight_at(weight, f64::from(lake.x_m), f64::from(lake.z_m)) == 0.0) {
            let as_kept = LakeWater { drawn: partner[lake.drawn].expect("a lake outside the outline is in the edited drawing"), ..*lake };
            assert!(chain.joined.water.lakes.contains(&as_kept), "{name}: the lake {lake:?} outside the outline");
        }
        for lake in chain.mixed.water.lakes.iter().filter(|lake| weight_at(weight, f64::from(lake.x_m), f64::from(lake.z_m)) > 0.0) {
            assert!(chain.joined.water.lakes.contains(lake), "{name}: the lake {lake:?} inside the outline");
        }
        let layer = |world: &World, name: &str| world.layers[name].data.clone();
        let (joined, kept, mixed) = (layer(&chain.joined, WATER_LAYER), layer(old, WATER_LAYER), layer(&chain.mixed, WATER_LAYER));
        let lowered: Vec<&RiverCourse> = (chain.join.joins.iter()).filter(|join| join.lowered_most_m > 0.0).map(|join| &chain.joined.water.rivers[join.course]).collect();
        let size = weight.width;
        let mut capped = 0;
        for (cell, &w) in weight.data.iter().enumerate() {
            if w == 0.0 {
                assert_eq!(joined[cell].to_bits(), kept[cell].to_bits(), "{name}: the water of cell {cell}, of weight 0");
                assert_eq!(chain.joined.heights.data[cell].to_bits(), old.heights.data[cell].to_bits(), "{name}: cell {cell}'s height");
                assert_eq!(chain.joined.ground.data[cell], old.ground.data[cell], "{name}: cell {cell}'s ground");
                for (layer, grid) in &old.layers {
                    assert_eq!(chain.joined.layers[layer].data[cell].to_bits(), grid.data[cell].to_bits(), "{name}: cell {cell}'s {layer}");
                }
            } else if w == 1.0 && joined[cell].to_bits() != mixed[cell].to_bits() {
                capped += 1;
                assert!(joined[cell] < mixed[cell], "{name}: the water of cell {cell}, of weight 1, stands higher than the mixed world's");
                let (x, z) = ((cell % size) as f64 - size as f64 / 2.0, (cell / size) as f64 - size as f64 / 2.0);
                let reach = |pair: &[CoursePoint]| {
                    let near = near_stretch(&pair[0], &pair[1], x, z);
                    near.apart_m <= near.half_width_m() + DROPPED_REACH_M
                };
                let near = lowered.iter().any(|course| course.points.windows(2).any(reach));
                assert!(near, "{name}: the water of cell {cell}, of weight 1, is lowered beyond the reach of every course the mending lowered");
            }
        }
        assert_eq!(capped, chain.join.capped_at_weight_1, "{name}: the cells of weight 1 lowered to a mended course");
        outside.len()
    }

    /// How many cells break the engine's two rules for ground beside and under water in `world`: in
    /// the band, then elsewhere, the wetness below how wet the water makes the cell, and the ground
    /// under more than 0.05 m of water not wet ground alone.
    pub(crate) fn ground_rules_broken(world: &World, weight: &Grid<f32>) -> ([usize; 2], [usize; 2]) {
        let [_, _, _, wetness_layer] = LAYER_NAMES;
        let depth = &world.layers[WATER_LAYER];
        let wet = bank_wetness(&world.heights, depth, &WaterRules::default());
        let wetness = &world.layers[wetness_layer];
        let (mut band, mut elsewhere) = ([0, 0], [0, 0]);
        for cell in 0..depth.data.len() {
            let w = weight.data[cell];
            let counts = if w > 0.0 && w < 1.0 { &mut band } else { &mut elsewhere };
            counts[0] += usize::from(wetness.data[cell] < wet.data[cell]);
            counts[1] += usize::from(ground_under_water(depth.data[cell]).is_some_and(|wet_alone| world.ground.data[cell] != wet_alone));
        }
        (band, elsewhere)
    }

    /// The measures of one chain's water join, printed on one line and returned.
    pub(crate) struct Measured {
        pub(crate) width_step_over_own_m: f32,
        pub(crate) new_unheld: NewUnheld,
        pub(crate) band_unheld: usize,
    }

    pub(crate) fn measure(name: &str, old: &World, chain: &Chain) -> Measured {
        let join = &chain.join;
        let rise_before_m = join.joins.iter().map(|join| join.rise_before_m).fold(0.0, f32::max);
        let own = join.joins.iter().map(|join| join.width_step_own_m).fold(0.0, f32::max);
        let width_step_over_own_m = join.joins.iter().map(|join| join.width_step_m - join.width_step_own_m).fold(f32::NEG_INFINITY, f32::max);
        let gap = join.joins.iter().map(|join| join.gap_m).fold(0.0, f32::max);
        let at_1: usize = join.joins.iter().map(|join| join.mended_at_weight_1).sum();
        let new_unheld = new_unheld(&chain.joined, old, &chain.whole, &chain.weight).expect("worlds with water");
        let cells = unheld_cells(&chain.joined).expect("a world with water");
        let band = |at: (f64, f64)| {
            let w = weight_at(&chain.weight, at.0, at.1);
            w > 0.0 && w < 1.0
        };
        let band_unheld = cells.dry.iter().filter(|cell| band(cell.at)).count()
            + cells.lake_over_water.iter().filter(|cell| band(cell.at)).count()
            + cells.steps.iter().filter(|step| band(step.a) || band(step.b)).count();
        let mut unfloored = chain.joined.clone();
        let [_, _, _, wetness_layer] = LAYER_NAMES;
        unfloored.layers.insert(wetness_layer.to_string(), chain.ground_joined.layers[wetness_layer].clone());
        unfloored.ground = chain.ground_joined.ground.clone();
        let (before, after) = (ground_rules_broken(&unfloored, &chain.weight), ground_rules_broken(&chain.joined, &chain.weight));
        println!(
            "{name}: crossings {} (and {} wholly inside); rise before {rise_before_m:.3} after {:.3} m (over {RISE_LIMIT_M} m: {}); lowered {:.3} raised {:.4} m ({at_1} points at weight 1 moved); width step at a join {:.2} m (own {own:.2}, most over own {width_step_over_own_m:.2}); gap {gap:.1} m; brooks old/new {:?} paired {} narrowed {} ended {}; ends lost {}; new unheld out/band/in {}/{}/{} (dry {:?} lake {:?} steps {:?}), all unheld in the band {band_unheld}; water lowered at weight 1 {} cells, at most {:.4} m; band wetness raised {} ground made wet {}; broken before: band {:?} elsewhere {:?}, after: band {:?} elsewhere {:?}; join {:.0} ms",
            join.crossings,
            join.joined_inside,
            join.rise_most_m,
            join.rises_over_limit,
            join.lowered_most_m,
            join.raised_most_m,
            join.width_step_most_m,
            join.brooks_crossing,
            join.brooks_paired,
            join.brooks_narrowed.len(),
            join.brooks_ended.len(),
            join.ends_lost,
            new_unheld.outside(),
            new_unheld.band(),
            new_unheld.inside(),
            new_unheld.dry,
            new_unheld.lake_over_water,
            new_unheld.steps,
            join.capped_at_weight_1,
            join.capped_most_m,
            join.band_wetness_raised,
            join.band_ground_made_wet,
            before.0,
            before.1,
            after.0,
            after.1,
            chain.join_ms
        );
        assert_eq!(after.0, [0, 0], "{name}: band cells breaking the ground's rules after the join");
        assert_eq!((before.0[0], before.0[1]), (join.band_wetness_raised, join.band_ground_made_wet), "{name}: the counts said");
        Measured { width_step_over_own_m, new_unheld, band_unheld }
    }

    /// Step 5 on the example at 2,048 m in hills from seed 11, its nine edits each through the whole
    /// chain: W1 first (`assert_kept_outside`), then no new unheld cell outside the outline, in the
    /// band or inside its inner edge, no unheld cell in the band at all, no rise, and no width step at
    /// a join larger than the kept course's own largest (the prototype: 0 / 0 / 0, a rise after
    /// mending of at most 0.001 m, width steps at a join at most 0.70 m against 0.74 m).
    #[test]
    fn the_examples_nine_edits_join_their_water_with_nothing_new_unheld() {
        let old = old_land();
        for name in EDIT_NAMES {
            let edited = example_edit(&old.drawing, name).expect("an edit of the example");
            let chain = chain(&old.drawing, &old.start, &old.model, &old.world, &edited, EXAMPLE_SEED);
            let points = assert_kept_outside(name, &old.drawing, &old.world, &edited, &chain);
            let measured = measure(&format!("edit {name} ({points} kept points outside)"), &old.world, &chain);
            assert_eq!(measured.new_unheld, NewUnheld::default(), "edit {name}: new unheld water");
            assert_eq!(measured.band_unheld, 0, "edit {name}: unheld water in the band");
            assert_eq!(chain.join.rise_most_m, 0.0, "edit {name}: a rise");
            assert!(measured.width_step_over_own_m <= 0.0, "edit {name}: a width step {} m above the kept course's own", measured.width_step_over_own_m);
        }
    }

    /// Step 5 on the trial `wide` at 1,024 m in hills from seed 3 with its standard edit: W1, no new
    /// unheld cell in the band, and inside at most one step and no cell of the other two kinds (the
    /// prototype: one step inside).
    #[test]
    fn the_trial_wides_water_joins_with_nothing_new_unheld_in_the_band() {
        const SEED: u64 = 3;
        const STEPS_INSIDE_MOST: usize = 1;
        let kept = kept_land(trial_drawing("wide", 1024, Country::hills()).expect("a trial"), SEED);
        let edited = standard_edit(&kept.drawing).expect("the trial's edit");
        let chain = chain(&kept.drawing, &kept.start, &kept.model, &kept.world, &edited, SEED);
        assert_kept_outside("wide", &kept.drawing, &kept.world, &edited, &chain);
        let measured = measure("wide", &kept.world, &chain);
        assert_eq!(measured.new_unheld.outside(), 0, "wide: new unheld water outside the outline");
        assert_eq!(measured.new_unheld.band(), 0, "wide: new unheld water in the band");
        let inside = &measured.new_unheld;
        assert!(inside.steps[2] <= STEPS_INSIDE_MOST && inside.dry[2] == 0 && inside.lake_over_water[2] == 0, "wide: {inside:?} inside");
        assert_eq!(chain.join.rise_most_m, 0.0);
    }

    /// Step 4: the run whose brooks the prototype paired 47 m apart (the example in mountains from
    /// seed 23, the brook's point moved 150 m): the new brook's head lies 46.6 m from the kept line,
    /// beyond `BROOK_PAIR_M`, its width and `HEAD_SLACK_M`, so the two are not one, and the band holds
    /// no unheld cell (the prototype: 21 in the band, 1 inside).
    #[test]
    fn brooks_far_apart_at_their_ends_are_not_joined_and_leave_nothing_unheld_in_the_band() {
        const SEED: u64 = 23;
        let drawing = Drawing { country: Country::named("mountains").expect("a kind"), ..Drawing::example_valley(2048) };
        let kept = kept_land(drawing, SEED);
        let edited = example_edit(&kept.drawing, "b").expect("an edit of the example");
        let chain = chain(&kept.drawing, &kept.start, &kept.model, &kept.world, &edited, SEED);
        assert_kept_outside("b mountains 23", &kept.drawing, &kept.world, &edited, &chain);
        let measured = measure("b mountains 23", &kept.world, &chain);
        assert_eq!(measured.new_unheld.outside(), 0);
        assert_eq!(measured.new_unheld.band(), 0, "b mountains 23: new unheld water in the band");
        assert_eq!(measured.band_unheld, 0, "b mountains 23: unheld water in the band");
        assert_eq!(chain.join.rise_most_m, 0.0);
        println!("b mountains 23: unheld in the joined land {:?}, the kept land {:?}, grown whole {:?}", unheld(&chain.joined), unheld(&kept.world), unheld(&chain.whole));
    }

    /// The whole-branch review's S3: a kept river crossing a straight outline once, east into it,
    /// joined with a partner that crosses it, leaves it west and comes back (two runs inside against
    /// one). The partner's run whose ends lie nearest those of the kept course's run inside takes its
    /// place, the second, which ends where the kept course does; the partner's other run inside is a
    /// course of its own, narrowed to nothing where it enters and leaves the outline, and both places
    /// are listed. So no point of the joined land at weight 0 is not the kept land's, no stretch runs
    /// back upstream along the kept line, and none is longer than a join's gap (the review's probe:
    /// 25 points at weight 0 that were not the kept land's, 10 stretches back upstream).
    #[test]
    fn a_partner_with_more_runs_inside_than_its_kept_course_leaves_the_others_apart() {
        const LOOP_Z_M: [f64; 3] = [6.0, 20.0, 34.0];
        const LOOP_EAST_X_M: f64 = 30.0;
        const LOOP_WEST_X_M: f64 = -80.0;
        const END_X_M: f64 = 110.0;
        let weight = straight_weight();
        let flowing = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let kept_river = course(&[(U_FROM_X_M, 0.0), (END_X_M, 0.0)], flowing, RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let corners = [
            (U_FROM_X_M, LOOP_Z_M[0]),
            (LOOP_EAST_X_M, LOOP_Z_M[0]),
            (LOOP_EAST_X_M, LOOP_Z_M[1]),
            (LOOP_WEST_X_M, LOOP_Z_M[1]),
            (LOOP_WEST_X_M, LOOP_Z_M[2]),
            (END_X_M, LOOP_Z_M[2]),
        ];
        let fresh = course(&corners, flowing, RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept_river.clone()], lakes: Vec::new() });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![fresh.clone()], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &hand_drawing(vec![drawn_river(50.0)], Vec::new()), &hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        let at_0 = |point: &CoursePoint| weight_of(&weight, point) == 0.0;
        let kept_bits: BTreeSet<[u32; 5]> = kept_river.points.iter().map(point_bits).collect();
        let (mut not_kept_at_0, mut back) = (0, 0);
        for course in &world.water.rivers {
            not_kept_at_0 += course.points.iter().filter(|point| at_0(point) && !kept_bits.contains(&point_bits(point))).count();
            back += course.points.windows(2).filter(|pair| pair[1].x_m < pair[0].x_m - 0.5 && pair[0].z_m.abs() < 0.01 && pair[1].z_m.abs() < 0.01).count();
        }
        assert_eq!((not_kept_at_0, back), (0, 0), "points at weight 0 that are not the kept land's, and stretches back upstream along the kept line");
        assert_eq!((world.water.rivers.len(), join.crossings, join.joins[0].course), (2, 1, 0), "{join:?}");

        // The joined course: the kept run outside, then the partner's second run inside, to its end.
        let runs_inside = runs_where(&fresh, &weight, |w| w > 0.0);
        assert_eq!(runs_inside.len(), 2);
        let joined_course = &world.water.rivers[0];
        let outside = |river: &RiverCourse| river.points.iter().filter(|point| at_0(point)).map(point_bits).collect::<Vec<_>>();
        assert_eq!(outside(joined_course), outside(&kept_river), "the kept points");
        assert_eq!(joined_course.points.len(), outside(&kept_river).len() + runs_inside[1].len(), "the kept run and the partner's second run");
        assert_eq!(point_bits(joined_course.points.last().expect("points")), point_bits(fresh.points.last().expect("points")), "it ends where the partner ends");
        assert_eq!((joined_course.drawn, joined_course.end), (Some(0), CourseEnd::Edge));
        assert!(join.joins[0].gap_m <= 2.0 * POINT_SPACING_M as f32, "{join:?}");

        // The partner's first run inside, a course of its own narrowed across the band.
        let narrowed: Vec<CoursePoint> = runs_inside[0].iter().map(|point| CoursePoint { width_m: point.width_m * weight_of(&weight, point), depth_m: point.depth_m * weight_of(&weight, point), ..*point }).collect();
        let apart = &world.water.rivers[1];
        assert_eq!(apart.points.iter().map(point_bits).collect::<Vec<_>>(), narrowed.iter().map(point_bits).collect::<Vec<_>>(), "the partner's first run, narrowed");
        assert_eq!((apart.drawn, apart.from_lake, apart.end), (Some(0), None, CourseEnd::Edge));
        let longest = apart.points.windows(2).map(|pair| (pair[1].x_m - pair[0].x_m).hypot(pair[1].z_m - pair[0].z_m)).fold(0.0f32, f32::max);
        assert!(longest <= POINT_SPACING_M as f32 + 1e-3, "a stretch {longest} m long");
        let at_outline = |(x, _): (f64, f64)| (x - OUTLINE_X_M).abs() <= POINT_SPACING_M;
        assert_eq!(join.narrowed_rivers, [Some(0), Some(0)], "{join:?}");
        assert!(join.brooks_narrowed.iter().copied().all(at_outline), "{:?}", join.brooks_narrowed);
        assert_eq!([join.brooks_narrowed[0].1, join.brooks_narrowed[1].1], [LOOP_Z_M[0], LOOP_Z_M[1]], "where it enters and where it leaves");
    }

    /// The whole-branch review's N7: drawn rivers pair equal first, then by their nearest lines, so
    /// a river added or taken away in the same edit as a change to another pairs with neither,
    /// whatever their order; a river whose line moves farther than `RIVER_PARTNER_REACH_M` on the
    /// mean pairs with nothing.
    #[test]
    fn rivers_pair_equal_first_then_by_their_nearest_lines() {
        let river = |z_from: f64, z_to: f64| River { points: vec![(-100.0, z_from), (100.0, z_to)], width_m: RIVER_WIDTH_M };
        let (a, b, c) = (river(-60.0, -60.0), river(0.0, 0.0), river(60.0, 60.0));
        let old = hand_drawing(vec![a.clone(), b.clone(), c.clone()], Vec::new());
        // A river inserted before B, and B's end moved 10 m: B pairs with the moved B (2), not with
        // the new river 30 m away (1), which order alone gave it.
        let inserted = hand_drawing(vec![a.clone(), river(30.0, 30.0), river(0.0, 10.0), c.clone()], Vec::new());
        assert_eq!(river_partners(&old, &inserted), [Some(0), Some(2), Some(3)]);
        // B taken away and C's end moved 10 m: C pairs with the moved C (1), B with nothing; order
        // alone gave B the moved C and C nothing.
        let removed = hand_drawing(vec![a.clone(), river(60.0, 70.0)], Vec::new());
        assert_eq!(river_partners(&old, &removed), [Some(0), None, Some(1)]);
        // C moved 160 m, beyond the reach: another river.
        let far = hand_drawing(vec![a.clone(), b.clone(), river(-100.0, -100.0)], Vec::new());
        assert_eq!(river_partners(&old, &far), [Some(0), Some(1), None]);
        let (line, moved) = (smoothed_line(&c.points).expect("a line"), smoothed_line(&river(60.0, 70.0).points).expect("a line"));
        assert_eq!(mean_apart_m(&line, &moved), mean_apart_m(&moved, &line), "the same both ways");
        assert!((mean_apart_m(&line, &moved) - 5.0).abs() < 0.1, "{} m", mean_apart_m(&line, &moved));
    }

    /// A joined course whose surface rises more than `RISE_LIMIT_M` from one point to the next before
    /// it is mended (a new river standing 2 m above the kept one, on its line) is counted, with the
    /// place it rose most, in the band on the line, and its drawn river; mended, it rises nowhere.
    #[test]
    fn a_rise_past_the_limit_before_mending_is_counted_with_its_place() {
        const HIGHER_M: f64 = 2.0;
        let weight = straight_weight();
        let flowing = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let line = [(U_FROM_X_M, 0.0), (-U_FROM_X_M, 0.0)];
        let kept_river = course(&line, flowing, RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let fresh = course(&line, |along_m| flowing(along_m) + HIGHER_M, RIVER_WIDTH_M, Some(0), CourseEnd::Edge);
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![kept_river], lakes: Vec::new() });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![fresh], lakes: Vec::new() });
        let (world, join) = joined(&old, &new, &weight, &hand_drawing(vec![drawn_river(50.0)], Vec::new()), &hand_drawing(vec![drawn_river(44.0)], Vec::new()));
        assert_eq!(join.rises_over_limit, 1, "{join:?}");
        let course = &join.joins[0];
        assert!(course.rise_before_m > RISE_LIMIT_M && course.rise_m == 0.0 && rise_of(&world.water.rivers[0]) == 0.0, "{course:?}");
        let (x, z) = course.rise_at.expect("where it rose");
        assert!(x > OUTLINE_X_M && x < INNER_X_M && z == 0.0, "it rose most at ({x}, {z})");
        assert_eq!(course.drawn, Some(0));
    }

    /// An end the join leaves out (a kept brook with no partner, ending in a lake whose middle lies
    /// inside the outline, where the new world has no lake) ends at the border, counted with its
    /// place, the brook's last point.
    #[test]
    fn an_end_the_join_leaves_out_ends_at_the_border_and_is_listed_with_its_place() {
        const LAKE_X_M: f64 = -30.0;
        let weight = straight_weight();
        let flowing = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let corners = [(U_FROM_X_M, 20.0), (-20.0, 20.0), (-20.0, 0.0), (-60.0, 0.0)];
        let brook = course(&corners, flowing, BROOK_WIDTH_M, None, CourseEnd::Lake(0));
        let lake = LakeWater { drawn: 0, level_m: SOURCE_M as f32, x_m: LAKE_X_M as f32, z_m: 0.0, area_m2: 100.0 };
        let old = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water { rivers: vec![brook.clone()], lakes: vec![lake] });
        let new = hand_world(|_, _| HIGH_GROUND_M, |_, _| 0.0, Water::default());
        let drawing = hand_drawing(Vec::new(), vec![Lake { x_m: LAKE_X_M, z_m: 0.0, radius_m: U_LAKE_RADIUS_M }]);
        let (world, join) = joined(&old, &new, &weight, &drawing, &drawing);
        assert!(world.water.lakes.is_empty(), "the lake, its middle inside, is not kept");
        let last = brook.points.last().expect("points");
        assert_eq!((join.ends_lost, join.ends_lost_at.clone(), join.ends_lost_rivers.clone()), (1, vec![(f64::from(last.x_m), f64::from(last.z_m))], vec![None]), "{join:?}");
        let ending = world.water.rivers.iter().find(|course| course.points.last().map(point_bits) == Some(point_bits(last))).expect("the run that ends where the brook ends");
        assert_eq!(ending.end, CourseEnd::Edge);
    }

    /// A kept lake whose water lies wholly at weight 0 (as a lake held whole does) is kept bit for
    /// bit, its cells' water too, though its middle, the mean of its cells' places, lies on ground
    /// beside it that weighs more (here a bay's far shore inside the outline); and a lake whose
    /// water reaches inside is not.
    #[test]
    fn a_kept_lake_whose_water_lies_wholly_outside_is_kept_though_its_middle_does_not() {
        const LAKE_LEVEL_M: f32 = 11.0;
        const RADIUS_M: f32 = 30.0;
        let weight = straight_weight();
        let lake_at = |x_m: f64| Lake { x_m, z_m: 0.0, radius_m: RADIUS_M };
        let world = |water_to_x_m: f64, middle_x_m: f32| {
            let wet = move |x: f64, z: f64| x <= water_to_x_m && (x - (OUTLINE_X_M - 10.0)).hypot(z) <= f64::from(RADIUS_M);
            let lake = LakeWater { drawn: 0, level_m: LAKE_LEVEL_M, x_m: middle_x_m, z_m: 0.0, area_m2: 1000.0 };
            hand_world(|_, _| 10.0, move |x, z| if wet(x, z) { LAKE_LEVEL_M - 10.0 } else { 0.0 }, Water { rivers: Vec::new(), lakes: vec![lake] })
        };
        let drawing = hand_drawing(Vec::new(), vec![lake_at(OUTLINE_X_M - 10.0)]);
        let new = hand_world(|_, _| 10.0, |_, _| 0.0, Water::default());
        // Its water ends 1 m short of the outline; its middle is said to lie 5 m inside it.
        let outside = world(OUTLINE_X_M - 1.0, (OUTLINE_X_M + 5.0) as f32);
        assert!(weight_at(&weight, OUTLINE_X_M + 5.0, 0.0) > 0.0, "the middle weighs more than 0");
        let (joined_world, _) = joined(&outside, &new, &weight, &drawing, &drawing);
        assert_eq!(joined_world.water.lakes, outside.water.lakes, "kept, bit for bit");
        // Its water reaching 5 m inside the outline: not wholly outside, and not kept.
        let reaching = world(OUTLINE_X_M + 5.0, (OUTLINE_X_M + 5.0) as f32);
        let (joined_world, _) = joined(&reaching, &new, &weight, &drawing, &drawing);
        assert!(joined_world.water.lakes.is_empty(), "{:?}", joined_world.water.lakes);
    }

    /// A band cell inside the bed of the river a brook ends on holds the river's water, as the fine
    /// steps fill it (`add_water`: a channel marks no cell inside the bed of the one it ends on),
    /// though the brook's last stretch, falling to the river's surface at its mouth, lies nearer it;
    /// a cell of the brook's bed outside the river's holds the brook's. The two worlds' ground
    /// differs, so the band's water is made from the beds, not kept.
    #[test]
    fn a_band_cell_inside_the_bed_of_the_river_a_brook_ends_on_holds_the_rivers_water() {
        /// The river runs east along `z = 0` across the world, this wide, so its bed reaches 4 m
        /// either side of its line.
        const WIDE_RIVER_M: f32 = 8.0;
        const RIVER_FROM_X_M: f64 = -110.0;
        /// The brook flows north into it from `BROOK_FROM_Z_M`, along `x = 0`, standing
        /// `BROOK_ABOVE_M` over the river until its last stretch of `LAST_STRETCH_M`, which falls
        /// to the river's surface at its mouth, as a tributary meets the river it ends on.
        const BROOK_FROM_Z_M: f64 = -60.0;
        const BROOK_ABOVE_M: f64 = 0.3;
        const LAST_STRETCH_M: f64 = 4.0;
        /// The new world's ground lies this much below the kept world's, so no cell is alike.
        const LOWER_M: f32 = 0.01;
        /// The places asked: inside the river's bed on the brook's line, and in the brook's bed
        /// beyond the river's.
        const IN_THE_RIVERS_BED: (f64, f64) = (0.0, -2.0);
        const IN_THE_BROOKS_BED: (f64, f64) = (0.0, -6.0);
        let river_surface = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let river = course(&[(RIVER_FROM_X_M, 0.0), (-RIVER_FROM_X_M, 0.0)], river_surface, WIDE_RIVER_M, Some(0), CourseEnd::Edge);
        let mouth_m = river_surface(-RIVER_FROM_X_M);
        let brook_length_m = -BROOK_FROM_Z_M;
        let brook_surface = |along_m: f64| {
            let to_mouth_m = brook_length_m - along_m;
            mouth_m + BROOK_ABOVE_M * (to_mouth_m / LAST_STRETCH_M).min(1.0) + FALL_PER_M * (to_mouth_m - LAST_STRETCH_M).max(0.0)
        };
        let brook = course(&[(0.0, BROOK_FROM_Z_M), (0.0, -LAST_STRETCH_M), (0.0, 0.0)], brook_surface, BROOK_WIDTH_M, None, CourseEnd::River(0));
        let water = Water { rivers: vec![river, brook], lakes: Vec::new() };
        let old = hand_world(|_, _| LOW_GROUND_M, |_, _| 0.0, water.clone());
        let new = hand_world(|_, _| LOW_GROUND_M - LOWER_M, |_, _| 0.0, water);
        let weight = Grid { width: HAND, height: HAND, cell_size: 1.0, data: vec![0.5; HAND * HAND] };
        let drawing = hand_drawing(vec![drawn_river(-RIVER_FROM_X_M)], Vec::new());
        let (joined_world, _) = joined(&old, &new, &weight, &drawing, &drawing);
        let surface_at = |(x, z): (f64, f64)| {
            let cell = (z as i64 + (HAND / 2) as i64) as usize * HAND + (x as i64 + (HAND / 2) as i64) as usize;
            joined_world.heights.data[cell] + joined_world.layers[WATER_LAYER].data[cell]
        };
        let line_surface = |course: &RiverCourse, (x, z): (f64, f64)| nearest_on_line(course, x, z).expect("a line").surface_m;
        let (river, brook) = (&joined_world.water.rivers[0], &joined_world.water.rivers[1]);
        let rivers_m = line_surface(river, IN_THE_RIVERS_BED);
        assert!(line_surface(brook, IN_THE_RIVERS_BED) > rivers_m + BROOK_ABOVE_M / 4.0, "the brook stands above the river there");
        assert!((f64::from(surface_at(IN_THE_RIVERS_BED)) - rivers_m).abs() < f64::from(LOWERED_M), "inside the river's bed: {} against the river's {rivers_m}", surface_at(IN_THE_RIVERS_BED));
        let brooks_m = line_surface(brook, IN_THE_BROOKS_BED);
        assert!((f64::from(surface_at(IN_THE_BROOKS_BED)) - brooks_m).abs() < f64::from(LOWERED_M), "in the brook's bed: {} against the brook's {brooks_m}", surface_at(IN_THE_BROOKS_BED));
    }

    /// A course that ends in a lake the join takes from the other world (its middle outside the
    /// outline, the course's last points inside it), and one that flows out of it, name that lake
    /// in the joined land: the lake is one drawn lake, from whichever world it is taken.
    #[test]
    fn a_course_names_its_lake_from_whichever_world_the_lake_is_taken() {
        const LAKE_LEVEL_M: f32 = 11.0;
        const RADIUS_M: f32 = 12.0;
        /// The lake lies just outside the outline, its water reaching into the band.
        const LAKE_X_M: f64 = OUTLINE_X_M - 4.0;
        let weight = straight_weight();
        let drawing = hand_drawing(Vec::new(), vec![Lake { x_m: LAKE_X_M, z_m: 0.0, radius_m: RADIUS_M }]);
        let wet = |x: f64, z: f64| (x - LAKE_X_M).hypot(z) <= f64::from(RADIUS_M);
        let lake = LakeWater { drawn: 0, level_m: LAKE_LEVEL_M, x_m: LAKE_X_M as f32, z_m: 0.0, area_m2: 400.0 };
        let level = |_: f64| f64::from(LAKE_LEVEL_M);
        let into = course(&[(LAKE_X_M + 60.0, 30.0), (LAKE_X_M + 6.0, 0.0)], level, BROOK_WIDTH_M, None, CourseEnd::Lake(0));
        let mut out = course(&[(LAKE_X_M + 6.0, -2.0), (LAKE_X_M + 60.0, -30.0)], level, BROOK_WIDTH_M, None, CourseEnd::Edge);
        out.from_lake = Some(0);
        let world = |ground_m: f32| {
            let water = Water { rivers: vec![into.clone(), out.clone()], lakes: vec![lake] };
            hand_world(move |_, _| ground_m, move |x, z| if wet(x, z) { LAKE_LEVEL_M - ground_m } else { 0.0 }, water)
        };
        let (old, new) = (world(10.0), world(10.0 - 0.01));
        assert_eq!(weight_at(&weight, LAKE_X_M, 0.0), 0.0, "the lake's middle lies outside");
        let (joined_world, _) = joined(&old, &new, &weight, &drawing, &drawing);
        assert_eq!(joined_world.water.lakes, old.water.lakes, "the kept lake");
        let ends: Vec<(Option<usize>, CourseEnd)> = joined_world.water.rivers.iter().map(|course| (course.from_lake, course.end)).collect();
        assert_eq!(ends, vec![(None, CourseEnd::Lake(0)), (Some(0), CourseEnd::Edge)]);
    }

    /// Both sides of the band's keep rule (stage 2's ruling R8, S1). A river runs east along
    /// `z = 0`; both worlds hold its bed's water `KEPT_ABOVE_M` above the stretch's linear surface, as
    /// the fine steps' own fill may, and the band is 0.5 everywhere. A cell of its bed keeps that
    /// water where both worlds hold the stretch beside it, the cell's ground and its water alike, the
    /// new world differing only far away; it takes the joined course's surface where the new world's
    /// stretch there lies `MOVED_M` lower, or where the two worlds' ground at the cell differs.
    #[test]
    fn a_band_cell_keeps_the_worlds_water_only_where_its_stretch_ground_and_water_are_both_worlds() {
        /// The river's width and its first point's `x`.
        const WIDE_RIVER_M: f32 = 8.0;
        const RIVER_FROM_X_M: f64 = -110.0;
        /// How far both worlds' water in the bed stands above the stretch's linear surface.
        const KEPT_ABOVE_M: f32 = 0.003;
        /// How much lower the new world's river point at `MOVED_X_M` lies in the second case.
        const MOVED_M: f32 = 0.03;
        const MOVED_X_M: f32 = 2.0;
        /// How much higher the new world's ground lies in the third case, and in the first beyond
        /// `FAR_X_M` alone.
        const RAISED_M: f32 = 0.02;
        const FAR_X_M: f64 = 60.0;
        /// The cell asked, in the river's bed, on the stretch from `x = -2` to `x = 2`.
        const IN_THE_BED: (f64, f64) = (0.0, -2.0);
        /// Single precision's step at these heights, about 20 m, held to: what the cell is said to
        /// hold it holds within this.
        const WITHIN_M: f64 = 1e-5;
        let river_surface = |along_m: f64| SOURCE_M - FALL_PER_M * along_m;
        let river = course(&[(RIVER_FROM_X_M, 0.0), (-RIVER_FROM_X_M, 0.0)], river_surface, WIDE_RIVER_M, Some(0), CourseEnd::Edge);
        let water_above = |x: f64, z: f64| {
            let in_bed = z.abs() <= f64::from(WIDE_RIVER_M) / 2.0;
            if in_bed { (river_surface(x - RIVER_FROM_X_M) as f32 + KEPT_ABOVE_M) - LOW_GROUND_M } else { 0.0 }
        };
        let world = |ground: &dyn Fn(f64, f64) -> f32, course: RiverCourse| {
            let ground_of = |x: f64, z: f64| ground(x, z);
            hand_world(ground_of, |x, z| water_above(x, z), Water { rivers: vec![course], lakes: Vec::new() })
        };
        let weight = Grid { width: HAND, height: HAND, cell_size: 1.0, data: vec![0.5; HAND * HAND] };
        let drawing = hand_drawing(vec![drawn_river(-RIVER_FROM_X_M)], Vec::new());
        let old = world(&|_, _| LOW_GROUND_M, river.clone());
        let cell = (IN_THE_BED.1 as i64 + (HAND / 2) as i64) as usize * HAND + (IN_THE_BED.0 as i64 + (HAND / 2) as i64) as usize;
        let surface = |joined: &World| f64::from(joined.heights.data[cell] + joined.layers[WATER_LAYER].data[cell]);
        let line = |joined: &World| nearest_on_line(&joined.water.rivers[0], IN_THE_BED.0, IN_THE_BED.1).expect("a line").surface_m;
        let kept_m = f64::from(old.heights.data[cell] + old.layers[WATER_LAYER].data[cell]);

        // Alike at the cell and along its stretch, the new world's ground higher far away: kept.
        let far_off = world(&|x, _| if x > FAR_X_M { LOW_GROUND_M + RAISED_M } else { LOW_GROUND_M }, river.clone());
        let (joined_world, _) = joined(&old, &far_off, &weight, &drawing, &drawing);
        assert_eq!(joined_world.layers[WATER_LAYER].data[cell].to_bits(), old.layers[WATER_LAYER].data[cell].to_bits(), "both worlds' water, kept bit for bit");
        assert!(kept_m - line(&joined_world) > f64::from(KEPT_ABOVE_M) / 2.0, "the kept water stands above the stretch's line");

        // The new world's stretch beside it lower: the joined course's surface there.
        let mut moved = river.clone();
        let point = moved.points.iter_mut().find(|point| point.x_m == MOVED_X_M).expect("a point at x = 2");
        point.surface_m -= MOVED_M;
        let (joined_world, _) = joined(&old, &world(&|_, _| LOW_GROUND_M, moved), &weight, &drawing, &drawing);
        assert!((surface(&joined_world) - line(&joined_world)).abs() < WITHIN_M, "the joined course's surface, {} against {}", surface(&joined_world), line(&joined_world));
        assert!(kept_m - surface(&joined_world) > f64::from(MOVED_M) / 4.0, "not the kept water: {} against {kept_m}", surface(&joined_world));

        // The two worlds' ground at the cell apart: the joined course's surface there.
        let raised = world(&|_, _| LOW_GROUND_M + RAISED_M, river.clone());
        let (joined_world, _) = joined(&old, &raised, &weight, &drawing, &drawing);
        assert!((surface(&joined_world) - line(&joined_world)).abs() < WITHIN_M, "the joined course's surface, {} against {}", surface(&joined_world), line(&joined_world));
        assert!(kept_m - surface(&joined_world) > f64::from(KEPT_ABOVE_M) / 2.0, "not the kept water");
    }
}
