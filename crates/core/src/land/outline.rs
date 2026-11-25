//! An edit's footprint, its outline and its weight: what an edit of a land changes, found from the
//! land model's starting state before anything is grown (the footprint, step P2 of the editor
//! tool's prototype, `report2.md`, "The final mechanism"), where each drawn lake's water may lie
//! (its zone, P4), and the ground the edit may change round them, with the share of the new land
//! each place takes (the outline and its weight, P5). An area regrown as drawn is its own outline,
//! with the lakes' zones it touches (`area_outline`, the stage 2 plan's question 1, answer (a)):
//! its band lies inside it.
//!
//! Three grids meet here. Each is square and laid over world metres, `x` east and `z` south, row by
//! row from the north-west, and a land `size` metres across has all three:
//!
//! - **The world's cells:** `size` x `size` cells of 1 m. Cell `(i, j)`, index `j * size + i`, lies
//!   at world `(i - size / 2, j - size / 2)`, the half taken exactly (half a metre for an odd size),
//!   where the fine steps put it (`world.rs`). The marks, the lake zones and `Outline::weight` are
//!   on these cells.
//! - **The land model's cells:** `side` x `side` cells of `GROW_CELL_M` over the world and
//!   `GROW_MARGIN_M` beyond it on every side. Cell `(i, j)`, index `j * side + i`, lies at world
//!   `(8 i - size / 2 - 256, 8 j - size / 2 - 256)` (`Grown`). `Footprint::model_cells` and
//!   `Outline::model_weight` are on these cells.
//! - **The lattice:** the points 1 m apart from the model's first cell to its last on both axes,
//!   `8 (side - 1) + 1` a side. Point `(a, b)` lies at world `(a - size / 2 - 256, b - size / 2 -
//!   256)`. Model cell `(i, j)` is point `(8 i, 8 j)` and world cell `(i, j)` is point
//!   `(i + 256, j + 256)`, so each cell of either grid is one point of it; the points of the margin
//!   are no world cell's, and carry no mark. The outline is decided on it, and both weights are read
//!   from it.
//!
//! A distance is the exact Euclidean one, in metres, from a point of the lattice to the nearest
//! point of a set, found by the two passes of Felzenszwalb and Huttenlocher's transform: down each
//! column, then along each row. Each transform runs over a box of the lattice no larger than the
//! farthest distance it is asked about needs (the footprint's box grown by the reach, a zone's box
//! grown by the band and the zone's margin, the outline's box grown by one point), clipped to the
//! lattice, and gives there what it would give over the whole lattice. The columns, and then the
//! rows, are taken in parallel, each written only by itself, so every value is the same on any
//! thread count. Nothing beyond the lattice is a point of any set: an outline that reaches the
//! lattice's edge has no ground outside it there.

use crate::country::{Country, COUNTRY_NAMES};
use crate::drawing::{Drawing, Lake};
use crate::grid::Grid;
use crate::grow::{model_side, smoothed_line, Grown, Start, GROW_CELL_M, GROW_MARGIN_M};
use crate::held::LAKE_LEVEL_WITHIN_M;
use crate::water::{model_lake_cells, LAKE_REACH_RADII, POCKET_MARGIN_M, WATER_LAYER};
use crate::world::World;
use rayon::prelude::*;
use std::ops::{Range, RangeInclusive};

/// A model cell is in an edit's footprint when its starting height differs between the two
/// drawings by more than this many metres, or its lift over all the rounds does (P2). The
/// prototype's choice: at 0.5 m a trial drawing (`zigzag`) left water unheld in the band, and at
/// 0.1 m the outlines grew with nothing gained.
pub const FOOTPRINT_TOLERANCE_M: f64 = 0.25;
/// A lake added, removed or changed puts a disc of this many of its radii round its centre into the
/// footprint: as far as a dug lake's outline reaches.
pub const LAKE_DISC_RADII: f64 = 1.25;
/// An outline that touches a lake's zone takes the zone and every point within the band and this
/// many metres more of it (P4, P5), so the lake lies whole where the weight is 1, when no held or
/// hand-shaped cell lies within `BAND_M` of the zone: fixed ground lowers the weight round it inside
/// a zone as anywhere else, to `s(d / BAND_M)` at `d` metres from it.
pub const LAKE_ZONE_MARGIN_M: f64 = 16.0;
/// The reach, how far an outline lies beyond its footprint in metres, is held between this and
/// `REACH_MAX_M`: the land's setting runs from the one to the other. At half this (and half the
/// band) the prototype's band bent 67 % more sharply than either land.
pub const REACH_MIN_M: f32 = 300.0;
/// The most the reach is: a setting above it, or an infinite one, is read as this.
pub const REACH_MAX_M: f32 = 600.0;
/// The band: inside an outline the weight rises from 0 to 1 over this many metres from the nearest
/// point that must not change (outside the outline, held or shaped by hand).
pub const BAND_M: f32 = 150.0;
/// A drawn river whose width the edit changes puts every model cell within this many metres of its
/// smoothed line, in either drawing, into the footprint: one model cell (`GROW_CELL_M`). The land
/// model's start does not show a width (`lay_rivers` reads one only for the water a river brings in
/// at the world's edge), but the fine steps cut the river's bed by it.
pub const WIDTH_CHANGE_REACH_M: f64 = GROW_CELL_M as f64;

/// Points of the lattice per model cell along an axis.
const CELL_POINTS: usize = GROW_CELL_M as usize;
/// Points of the lattice in the model's margin along an axis: world cell `(i, j)` is point
/// `(i + MARGIN_POINTS, j + MARGIN_POINTS)`.
const MARGIN_POINTS: usize = GROW_MARGIN_M as usize;
/// A model cell of a footprint covers the points within this many of its own along each axis: its
/// 8 m square, edges included, shared with its neighbours (9 x 9 points).
const CELL_HALF_POINTS: usize = CELL_POINTS / 2;
/// The side of a world's cell, in metres: a drawn world's cells are 1 m, so a cell is 1 m².
const WORLD_CELL_M: f32 = 1.0;
/// Marks a point whose column, within the box a transform runs over, holds no point of the set. A
/// lattice is at most 8,697 points a side (`MAX_SIZE_CELLS`), so every distance along a column is
/// less.
const NO_SITE: u16 = u16::MAX;

/// Where a land must not change: one flag a world cell each (`size` x `size`, in the world's cell
/// order). The margin beyond the world has no marks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Marks {
    /// Held by the person: nothing changes these cells.
    pub held: Vec<bool>,
    /// Shaped by hand: no regrowth changes these cells.
    pub by_hand: Vec<bool>,
}

impl Marks {
    /// No cell marked, for a world `size` cells a side.
    pub fn none(size: usize) -> Self {
        Self { held: vec![false; size * size], by_hand: vec![false; size * size] }
    }

    /// Whether world cell `cell` is held or shaped by hand, either of which an edit leaves as it is.
    /// A cell beyond the end of a list is not marked by that list.
    pub fn fixed(&self, cell: usize) -> bool {
        self.held.get(cell) == Some(&true) || self.by_hand.get(cell) == Some(&true)
    }

    /// The marks a regrowth of `area` as drawn weighs the area by and leaves the land with
    /// (`Land::regrow_area`), `area` one flag a world cell in the world's cell order (a cell beyond
    /// its end not in it): the hand marks cleared on the area's cells that are not held, since the
    /// regrowth gives that ground back to the drawing, while a held cell's ground does not change
    /// and it keeps its hand mark (stage 2's ruling R4, N1); the held marks as they are. The one rule
    /// the core's regrowth and the editor tool's outline of an area (stage 2's ruling R12, S2) both
    /// read.
    pub fn regrown(&self, area: &[bool]) -> Marks {
        let Marks { held, by_hand } = self;
        let by_hand = (by_hand.iter().enumerate())
            .map(|(cell, &by_hand)| by_hand && !(area.get(cell) == Some(&true) && held.get(cell) != Some(&true)))
            .collect();
        Marks { held: held.clone(), by_hand }
    }
}

/// What an edit changes, found before anything is grown (P2).
#[derive(Debug, Clone, PartialEq)]
pub struct Footprint {
    /// One flag per cell of the land model's grid, in its cell order. A cell in the footprint covers
    /// the lattice's points within `GROW_CELL_M / 2` of its own on both axes.
    pub model_cells: Vec<bool>,
    /// A disc for each lake that changed, as world `(x, z)` of its centre and its radius, in
    /// metres. It covers the lattice's points that lie within the radius of the centre.
    pub lake_discs: Vec<(f64, f64, f64)>,
    /// Why the footprint is the whole land, where `footprint` gives the whole land because the two
    /// starts' seeds or the two drawings' countries differ, so that a tool can say so before the edit
    /// runs, and while a person drags (the design's **Whole-land edits**); `None` for every other
    /// footprint.
    pub whole: Option<Whole>,
}

/// Why an edit's footprint is the whole land (`Footprint::whole`): the land model's rounds read the
/// seed and every number of the country, so the land grows otherwise everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Whole {
    /// The two starts are of different seeds (`Land::reseed`).
    Seed,
    /// The two countries are each a kind `Country::named` gives, and not the same kind.
    Kind,
    /// The two countries differ in these numbers, each named by its field in `Country`, in the
    /// struct's order.
    Numbers(Vec<&'static str>),
}

impl Whole {
    /// Why the footprint of an edit from a drawing of country `old` to one of country `new`, of one
    /// seed, is the whole land: `Kind` when both are kinds `Country::named` gives, else the numbers
    /// in which they differ. `None` when they are the same country.
    pub fn of_countries(old: &Country, new: &Country) -> Option<Whole> {
        let numbers = country_numbers_changed(old, new);
        if numbers.is_empty() {
            return None;
        }
        let kind = |country: &Country| COUNTRY_NAMES.iter().position(|name| Country::named(name).as_ref() == Some(country));
        Some(match (kind(old), kind(new)) {
            (Some(_), Some(_)) => Whole::Kind,
            _ => Whole::Numbers(numbers),
        })
    }

    /// Why, in the words an edit's note says it with: "the seed changed", "the kind of country
    /// changed", "a number of the country changed: relief_m", or for more than one "numbers of the
    /// country changed: relief_m, rough_m and rounds".
    pub fn reason(&self) -> String {
        match self {
            Whole::Seed => "the seed changed".to_string(),
            Whole::Kind => "the kind of country changed".to_string(),
            Whole::Numbers(numbers) => match numbers.as_slice() {
                [] => "the country changed".to_string(),
                [one] => format!("a number of the country changed: {one}"),
                [first @ .., last] => format!("numbers of the country changed: {} and {last}", first.join(", ")),
            },
        }
    }
}

/// The fields in which countries `old` and `new` differ, by name, in `Country`'s order. Taken apart
/// field by field, so a new field of `Country` fails to compile until it is named here.
fn country_numbers_changed(old: &Country, new: &Country) -> Vec<&'static str> {
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
    } = *old;
    let numbers = [
        ("floor_m", floor_m != new.floor_m),
        ("relief_m", relief_m != new.relief_m),
        ("rough_m", rough_m != new.rough_m),
        ("rough_size_m", rough_size_m != new.rough_size_m),
        ("lift_low_m", lift_low_m != new.lift_low_m),
        ("lift_high_m", lift_high_m != new.lift_high_m),
        ("cut_strength", cut_strength != new.cut_strength),
        ("cut_area_power", cut_area_power != new.cut_area_power),
        ("slump_m2", slump_m2 != new.slump_m2),
        ("rounds", rounds != new.rounds),
        ("hardness_variation", hardness_variation != new.hardness_variation),
        ("hardness_patch_m", hardness_patch_m != new.hardness_patch_m),
    ];
    numbers.into_iter().filter(|&(_, differs)| differs).map(|(name, _)| name).collect()
}

impl Footprint {
    /// The whole land of a model grid `side` cells a side: every model cell, no disc, and no reason
    /// given (`whole` is `None`).
    pub fn whole(side: usize) -> Self {
        Self { model_cells: vec![true; side * side], lake_discs: Vec::new(), whole: None }
    }

    /// Whether the footprint holds nothing: no model cell and no disc.
    pub fn is_empty(&self) -> bool {
        !self.model_cells.contains(&true) && self.lake_discs.is_empty()
    }
}

/// An edit's outline and the weight by which each place takes the new land (P5).
#[derive(Debug, Clone, PartialEq)]
pub struct Outline {
    /// The weight on the world's cells, `size` x `size` cells of 1 m: 0 outside the outline and on
    /// every held or hand-shaped cell, and inside `s(d / BAND_M)`, `s(t) = t² (3 - 2 t)` with `t`
    /// held to 0..1 and `d` the distance from the cell's point to the nearest point that is outside
    /// the outline or fixed; 1 where there is no such point on the lattice.
    pub weight: Grid<f32>,
    /// The weight at each cell of the land model's grid, in its cell order, read at the cell's point
    /// of the lattice: the margin's cells by the same rule as the world's.
    pub model_weight: Vec<f32>,
    /// The world cells inside the outline, held and hand-shaped ones among them, in square metres;
    /// the margin is not counted.
    pub inside_m2: f64,
    /// The world cells inside the outline that are held or shaped by hand, in square metres.
    pub fixed_inside_m2: f64,
}

/// The footprint of an edit from `old_drawing` to `new_drawing` (P2), from the land model's start
/// for each (`start_of`, from the land's seed and water rules).
///
/// A model cell is in it when its starting height differs by more than `FOOTPRINT_TOLERANCE_M`, or
/// its lift per round does by more than that over all the rounds (the difference times the
/// country's `rounds`), or the water arriving there from beyond the grid differs at all, or whether
/// it is an outlet. Each lake of either drawing that the other does not hold unchanged (the same
/// place and radius) adds a disc of `LAKE_DISC_RADII` of its radius round its centre. A drawn river
/// at the same place in both drawings whose `width_m` differs adds every model cell whose centre lies
/// within `WIDTH_CHANGE_REACH_M` of its smoothed line in either drawing (the line `lay_rivers`
/// smooths and `GrownRiver::line` holds, without the stretches it adds beyond the ends), which the
/// start does not show.
///
/// Two starts from different seeds, or two drawings of different countries (any field), give the
/// whole land (`Footprint::whole`), with a disc for every lake of either drawing (each lake once),
/// so the lakes' zones still apply: the rounds read the seed and every number of the country, so
/// the land grows otherwise everywhere, though the starts may agree on many cells (a river's trench
/// and an outlet do not depend on the seed) or on all (a change of `cut_strength` does not touch
/// the start). So `rounds` is only ever one country's. Its `whole` says why: `Whole::Seed` for the
/// seeds, before the countries, and else `Whole::of_countries`.
///
/// An edit keeps the land's size: the two drawings must have one `size_m`, and the two starts one
/// grid, which a debug build asserts. A release build given two grids reads nothing beyond either
/// and gives the whole of `new`'s.
pub fn footprint(old: &Start, new: &Start, old_drawing: &Drawing, new_drawing: &Drawing) -> Footprint {
    debug_assert_eq!(old_drawing.size_m, new_drawing.size_m, "an edit keeps the land's size: a footprint is found between two drawings of one size");
    debug_assert_eq!(old.side, new.side, "an edit keeps the land's size: a footprint is found between two starts of one grid");
    let disc = |lake: &Lake| (lake.x_m, lake.z_m, LAKE_DISC_RADII * f64::from(lake.radius_m));
    let cells = new.side * new.side;
    let one_grid = [&old.heights, &old.lift_m, &old.incoming_m2, &new.heights, &new.lift_m, &new.incoming_m2]
        .iter()
        .all(|values| values.len() == cells)
        && old.side == new.side;
    let whole = if old.seed != new.seed { Some(Whole::Seed) } else { Whole::of_countries(&old_drawing.country, &new_drawing.country) };
    if whole.is_some() || !one_grid {
        let mut lakes: Vec<&Lake> = Vec::new();
        for lake in old_drawing.lakes.iter().chain(&new_drawing.lakes) {
            if !lakes.contains(&lake) {
                lakes.push(lake);
            }
        }
        return Footprint { lake_discs: lakes.into_iter().map(disc).collect(), whole, ..Footprint::whole(new.side) };
    }
    let mut model_cells = starts_differ(old, new, new_drawing.country.rounds);
    for (before, after) in old_drawing.rivers.iter().zip(&new_drawing.rivers).filter(|(before, after)| before.width_m != after.width_m) {
        for line in [&before.points, &after.points].into_iter().filter_map(|points| smoothed_line(points)) {
            let near = distances_to_line(&line, new_drawing.size_m, WIDTH_CHANGE_REACH_M);
            for (flag, _) in model_cells.iter_mut().zip(&near).filter(|(_, &apart_m)| apart_m <= WIDTH_CHANGE_REACH_M) {
                *flag = true;
            }
        }
    }
    let removed = old_drawing.lakes.iter().filter(|lake| !new_drawing.lakes.contains(lake));
    let added = new_drawing.lakes.iter().filter(|lake| !old_drawing.lakes.contains(lake));
    Footprint { model_cells, lake_discs: removed.chain(added).map(disc).collect(), whole: None }
}

/// P2's four tests, cell by cell, on two starts of one grid (each of their lists one value a cell)
/// grown over `rounds` rounds: whether the starting height differs by more than
/// `FOOTPRINT_TOLERANCE_M`, or the lift does over the rounds, or the water arriving differs at all,
/// or whether the cell is an outlet.
fn starts_differ(old: &Start, new: &Start, rounds: u32) -> Vec<bool> {
    let (cells, rounds) = (new.side * new.side, f64::from(rounds));
    let outlets = |start: &Start| {
        let mut is_outlet = vec![false; cells];
        for &outlet in &start.outlets {
            if let Some(flag) = is_outlet.get_mut(outlet) {
                *flag = true;
            }
        }
        is_outlet
    };
    let (old_outlets, new_outlets) = (outlets(old), outlets(new));
    (0..cells)
        .map(|cell| {
            (old.heights[cell] - new.heights[cell]).abs() > FOOTPRINT_TOLERANCE_M
                || (old.lift_m[cell] - new.lift_m[cell]).abs() * rounds > FOOTPRINT_TOLERANCE_M
                || old.incoming_m2[cell] != new.incoming_m2[cell]
                || old_outlets[cell] != new_outlets[cell]
        })
        .collect()
}

/// The footprint of an area: every model cell that covers a point of a world cell of `area` (one
/// flag a world cell of a land `size_m` across, in the world's cell order; a flag beyond the world's
/// cells is not read), so the footprint covers the whole area. No disc. Stage 1 regrew an area as
/// the edit of this footprint, its outline reaching beyond the area; an area regrown as drawn is
/// now its own outline (`area_outline`), and `Land::regrow_area` reads this footprint only to tell
/// an area of no cell.
pub fn footprint_of_area(size_m: u32, area: &[bool]) -> Footprint {
    let lattice = Lattice::new(size_m);
    let side = lattice.model_side;
    let mut model_cells = vec![false; side * side];
    for (cell, _) in area.iter().enumerate().take(lattice.world_cells()).filter(|(_, &on)| on) {
        let (a, b) = lattice.world_point(cell);
        for j in lattice.model_cells_covering(b) {
            for i in lattice.model_cells_covering(a) {
                model_cells[j * side + i] = true;
            }
        }
    }
    Footprint { model_cells, lake_discs: Vec::new(), whole: None }
}

/// The outline of `footprint` alone, before anything is grown and so without the lakes' zones: what
/// a tool draws while a person drags. `outline` with no zones.
pub fn early_outline(size_m: u32, footprint: &Footprint, marks: &Marks, reach_m: f32) -> Outline {
    outline(size_m, footprint, &[], marks, reach_m)
}

/// The outline of regrowing an area as drawn (the stage 2 plan's question 1, answer (a)): the area
/// itself, its world cells (`area`, one flag a world cell of a land `size_m` across, in the world's
/// cell order; a flag beyond the world's cells is not read), with, beyond each of them on the
/// world's border, the points of the land model's margin whose nearest world cell it is (on each
/// axis the cell's own column or row, held to the world), so the world's border is no edge of the
/// area; and every lake zone it touches taken whole with every point within `BAND_M` and
/// `LAKE_ZONE_MARGIN_M` of it, as `outline` takes one (a zone that a taken zone's ground touches
/// likewise, whatever the order of `zones`). The weight is `outline`'s: 0 outside and on every held
/// or hand-shaped cell, and `s(d / BAND_M)` with `d` the distance from the cell's point to the
/// nearest point outside the outline or fixed. So the band lies inside the area, from its edge
/// inward, but not along the world's border: nothing beyond the area changes but round a lake it
/// touches and the model's margin beyond it, and the land is the drawing grown whole only farther
/// than `BAND_M` inside the edge (an area less than 300 m across nowhere), up to the border itself
/// (the whole land painted is the drawing grown whole on every cell farther than `BAND_M` from fixed
/// ground). No reach is read. An area of no world cell gives an empty outline, weight 0 everywhere.
pub fn area_outline(size_m: u32, area: &[bool], zones: &[Vec<usize>], marks: &Marks) -> Outline {
    weigh(size_m, Taken::Area(area), zones, marks, Boxes::Bounded)
}

/// The outline of regrowing `area` as drawn before the lakes' zones are known: what a tool draws
/// while a person paints the area, the area itself weighed as `area_outline` weighs it (with no
/// zones).
pub fn early_area_outline(size_m: u32, area: &[bool], marks: &Marks) -> Outline {
    area_outline(size_m, area, &[], marks)
}

/// The outline of an edit of a land `size_m` across and its weight (P5).
///
/// The outline holds every point of the lattice within the reach of `footprint` (`reach_m`, held to
/// `REACH_MIN_M..=REACH_MAX_M`, a NaN taken as `REACH_MIN_M`). Then, while it touches a lake's zone
/// it has not taken (one of `zones`' world cells is a point of it), it takes every point within
/// `BAND_M` and `LAKE_ZONE_MARGIN_M` of that zone; so a zone that a taken zone's ground reaches is
/// taken too, whatever the order of `zones`. Every point outside it, and every held or hand-shaped
/// world cell (`marks`) in it, must not change; the weight at every other point is `s(d / BAND_M)`
/// with `d` its distance to the nearest of those (`Outline` says how both weights are read). An
/// empty footprint gives an empty outline, weight 0 everywhere.
///
/// `zones` are lists of world cells (`lake_zones`); an index beyond the world's cells is not read.
///
/// # Panics
///
/// When `footprint` does not have one flag per model cell of a land `size_m` across.
pub fn outline(size_m: u32, footprint: &Footprint, zones: &[Vec<usize>], marks: &Marks, reach_m: f32) -> Outline {
    weigh(size_m, Taken::Reach(footprint, reach_m), zones, marks, Boxes::Bounded)
}

/// The world cells where each drawn lake's water may lie before and after an edit from
/// `old_drawing` to `new_drawing` (P4): one zone a lake, each a sorted list of distinct indices of
/// `old`'s world cells, in its cell order.
///
/// A lake of the old drawing and one of the new drawing at the same place with the same radius are
/// one lake (the first such of the new drawing not already paired). Its zone is the union of:
///
/// - the cells of `old`, the land as it is, whose water stands at the lake's level there (its
///   surface, ground plus the `water` layer's depth, within `LAKE_LEVEL_WITHIN_M` of it) and that lie
///   within `LAKE_REACH_RADII` of its radii and `POCKET_MARGIN_M` more of its centre, where
///   `fill_lakes` looks for its water;
/// - the cells where `old_model` holds the lake's water at the level it gives the lake, and the
///   cells where `new_model` does (`model_lake_cells`, the cells `fill_lakes` looks among).
///
/// The old drawing's lakes come first, in its order, then the new drawing's that were not paired,
/// in its order, each with the cells where `new_model` holds its water. A lake that neither land
/// nor model holds gives an empty zone, which no outline touches. `old` without the `water` layer
/// gives only the models' cells.
pub fn lake_zones(old: &World, old_model: &Grown, new_model: &Grown, old_drawing: &Drawing, new_drawing: &Drawing) -> Vec<Vec<usize>> {
    let size = (old.heights.width, old.heights.height);
    let model_cells = |model: &Grown, lake: &Lake, drawn: usize| -> Vec<usize> {
        model.lakes.iter().find(|grown| grown.drawn == drawn).map_or_else(Vec::new, |grown| model_lake_cells(model, lake, grown.level_m, size))
    };
    let partners = lake_partners(old_drawing, new_drawing);
    let mut paired = vec![false; new_drawing.lakes.len()];
    let mut zones = Vec::new();
    for ((drawn, lake), partner) in old_drawing.lakes.iter().enumerate().zip(partners) {
        let mut zone = lake_water(old, lake, drawn);
        zone.extend(model_cells(old_model, lake, drawn));
        if let Some(new_drawn) = partner {
            paired[new_drawn] = true;
            zone.extend(model_cells(new_model, &new_drawing.lakes[new_drawn], new_drawn));
        }
        zone.sort_unstable();
        zone.dedup();
        zones.push(zone);
    }
    for (drawn, lake) in new_drawing.lakes.iter().enumerate().filter(|&(drawn, _)| !paired[drawn]) {
        let mut zone = model_cells(new_model, lake, drawn);
        zone.sort_unstable();
        zone.dedup();
        zones.push(zone);
    }
    zones
}

/// Which lake of `new_drawing` each lake of `old_drawing` is, in the old drawing's order: the first
/// lake of the new drawing not already paired that lies at the same place with the same radius
/// (`Lake ==`), or `None` for a lake the edit moved, resized or took away. An edit's lake zones
/// (`lake_zones`) and its joined water (`land::water::join_water`) pair lakes so.
pub(crate) fn lake_partners(old_drawing: &Drawing, new_drawing: &Drawing) -> Vec<Option<usize>> {
    let mut paired = vec![false; new_drawing.lakes.len()];
    (old_drawing.lakes.iter())
        .map(|lake| {
            let partner = (0..new_drawing.lakes.len()).find(|&n| !paired[n] && new_drawing.lakes[n] == *lake)?;
            paired[partner] = true;
            Some(partner)
        })
        .collect()
}

/// The cells of `world` that hold drawn lake `drawn`'s water (`lake`), as `lake_zones` reads them:
/// in index order, the cells whose water's surface lies within `LAKE_LEVEL_WITHIN_M` of the lake's
/// level (`held.rs` tells a lake's water so) and that lie within `LAKE_REACH_RADII` of its radii
/// and `POCKET_MARGIN_M` more of its drawn centre, where `fill_lakes` looks for it. A land holds a
/// lake's water whole by these cells (`Land::hold`).
pub(crate) fn lake_water(world: &World, lake: &Lake, drawn: usize) -> Vec<usize> {
    let (Some(depth), Some(water)) = (world.layers.get(WATER_LAYER), world.water.lakes.iter().find(|water| water.drawn == drawn)) else {
        return Vec::new();
    };
    let (width, height) = (world.heights.width, world.heights.height);
    let reach_m = LAKE_REACH_RADII * f64::from(lake.radius_m) + POCKET_MARGIN_M;
    let (half_width, half_height) = (width as f64 / 2.0, height as f64 / 2.0);
    // The cells whose middle lies within `reach_m` of the centre on one axis (none when the first
    // lies past the end).
    let near = |at_m: f64, half: f64, count: usize| -> Range<usize> {
        let first = (at_m - reach_m + half).ceil().max(0.0);
        let end = ((at_m + reach_m + half).floor() + 1.0).clamp(0.0, count as f64);
        first as usize..end as usize
    };
    let mut cells = Vec::new();
    for j in near(lake.z_m, half_height, height) {
        for i in near(lake.x_m, half_width, width) {
            let cell = j * width + i;
            let (x, z) = (i as f64 - half_width, j as f64 - half_height);
            let depth_m = depth.data[cell];
            let surface_m = world.heights.data[cell] + depth_m;
            if depth_m > 0.0
                && (x - lake.x_m).hypot(z - lake.z_m) <= reach_m
                && (f64::from(surface_m) - f64::from(water.level_m)).abs() < LAKE_LEVEL_WITHIN_M
            {
                cells.push(cell);
            }
        }
    }
    cells
}

/// How far the boxes the distances are found over reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Boxes {
    /// As far as each distance asked about needs: what `outline` runs.
    Bounded,
    /// The whole lattice, every time: what the boxes must give, for the tests.
    #[cfg_attr(not(test), allow(dead_code))]
    Whole,
}

/// What an outline takes before the lakes' zones.
#[derive(Debug, Clone, Copy)]
enum Taken<'a> {
    /// Every point within a reach (in metres, held to `REACH_MIN_M..=REACH_MAX_M`) of a footprint:
    /// an edit's (`outline`).
    Reach(&'a Footprint, f32),
    /// The points of an area's world cells, one flag a world cell: an area regrown as drawn
    /// (`area_outline`).
    Area(&'a [bool]),
}

/// `outline` and `area_outline`, their distances found over `boxes`.
fn weigh(size_m: u32, taken: Taken, zones: &[Vec<usize>], marks: &Marks, boxes: Boxes) -> Outline {
    let lattice = Lattice::new(size_m);
    let model_cells = lattice.model_side * lattice.model_side;
    let around = |found: Rect, by_m: f64| match boxes {
        Boxes::Bounded => found.grown(by_m.floor() as usize, lattice.side),
        Boxes::Whole => lattice.all(),
    };

    // Every point within the reach of the footprint, or every point of the area.
    let mut inside = vec![false; lattice.points()];
    let mut region: Option<Rect> = None;
    match taken {
        Taken::Reach(footprint, reach_m) => {
            assert_eq!(
                footprint.model_cells.len(),
                model_cells,
                "a footprint of a land {size_m} m across needs one flag per model cell, {model_cells}"
            );
            let reach = f64::from(if reach_m.is_nan() { REACH_MIN_M } else { reach_m.clamp(REACH_MIN_M, REACH_MAX_M) });
            if let Some(found) = footprint_box(&lattice, footprint) {
                let rect = around(found, reach);
                let sites = footprint_sites(&lattice, footprint, rect);
                take_within(&lattice, &mut inside, rect, &sites, reach);
                region = Some(rect);
            }
        }
        Taken::Area(area) => {
            let cells = || area.iter().enumerate().take(lattice.world_cells()).filter(|(_, &on)| on).map(|(cell, _)| lattice.world_point(cell));
            for point in cells() {
                inside[lattice.index(point)] = true;
            }
            region = bounds(cells());
            // The model's margin beyond the area's cells on the world's border: a point of the
            // margin is the area's where the world cell nearest it is, so the border is no edge.
            for cell in lattice.border_cells().filter(|&cell| area.get(cell) == Some(&true)) {
                let beyond = lattice.nearest_to(cell);
                for b in beyond.b0..=beyond.b1 {
                    inside[lattice.index((beyond.a0, b))..=lattice.index((beyond.a1, b))].fill(true);
                }
                region = Some(region.map_or(beyond, |region| region.union(beyond)));
            }
        }
    }

    // Every zone it touches, with the band and the zone's margin round it, until none is left.
    let zone_reach = f64::from(BAND_M) + LAKE_ZONE_MARGIN_M;
    let zone_points: Vec<Vec<(usize, usize)>> = zones
        .iter()
        .map(|zone| zone.iter().filter(|&&cell| cell < lattice.world_cells()).map(|&cell| lattice.world_point(cell)).collect())
        .collect();
    let mut taken = vec![false; zones.len()];
    loop {
        let mut grew = false;
        for (zone, points) in zone_points.iter().enumerate() {
            if taken[zone] || !points.iter().any(|&point| inside[lattice.index(point)]) {
                continue;
            }
            taken[zone] = true;
            grew = true;
            let Some(found) = bounds(points.iter().copied()) else { continue };
            let rect = around(found, zone_reach);
            let mut sites = vec![false; rect.len()];
            for &point in points {
                sites[rect.local(point)] = true;
            }
            take_within(&lattice, &mut inside, rect, &sites, zone_reach);
            region = Some(region.map_or(rect, |region| region.union(rect)));
        }
        if !grew {
            break;
        }
    }

    let size = lattice.size;
    let mut weight = Grid::new(size, size, WORLD_CELL_M);
    let mut model_weight = vec![0.0; model_cells];
    let (mut inside_cells, mut fixed_cells) = (0usize, 0usize);
    if let Some(region) = region {
        for cell in lattice.world_cells_in(region) {
            if inside[lattice.index(lattice.world_point(cell))] {
                inside_cells += 1;
                fixed_cells += usize::from(marks.fixed(cell));
            }
        }
        // The distance to the nearest point that must not change. `region` holds every point of the
        // outline, so the ring round it holds only such points, and the nearest one to any point of
        // `region` lies in `region` or on the ring.
        let rect = around(region, 1.0);
        let mut sites = vec![false; rect.len()];
        sites.par_chunks_mut(rect.width()).enumerate().for_each(|(row, sites)| {
            let b = rect.b0 + row;
            for (column, site) in sites.iter_mut().enumerate() {
                let point = (rect.a0 + column, b);
                *site = !inside[lattice.index(point)] || lattice.world_cell(point).is_some_and(|cell| marks.fixed(cell));
            }
        });
        let near = column_pass(rect, &sites);
        let mut weights = vec![0.0; rect.len()];
        weights.par_chunks_mut(rect.width()).enumerate().for_each(|(row, weights)| {
            row_pass(rect, &near, row, |column, squared| weights[column] = weight_of(squared));
        });
        let read = |point: (usize, usize)| if rect.contains(point) { weights[rect.local(point)] } else { 0.0 };
        weight.data.par_chunks_mut(size).enumerate().for_each(|(j, row)| {
            for (i, value) in row.iter_mut().enumerate() {
                *value = read(lattice.world_point(j * size + i));
            }
        });
        for (cell, value) in model_weight.iter_mut().enumerate() {
            *value = read(lattice.model_point(cell));
        }
    }
    let cell_m2 = f64::from(WORLD_CELL_M * WORLD_CELL_M);
    Outline { weight, model_weight, inside_m2: inside_cells as f64 * cell_m2, fixed_inside_m2: fixed_cells as f64 * cell_m2 }
}

/// The weight at a point whose nearest point that must not change lies `distance_m` away:
/// `s(distance_m / BAND_M)`, `s(t) = t² (3 - 2 t)` with `t` held to 0..1.
fn weight_at(distance_m: f64) -> f32 {
    let t = (distance_m / f64::from(BAND_M)).clamp(0.0, 1.0);
    (t * t * (3.0 - 2.0 * t)) as f32
}

/// `weight_at` the square root of `squared`, a squared distance in square metres; 1 for `None`,
/// where no point must stay on the lattice at all.
fn weight_of(squared: Option<u64>) -> f32 {
    squared.map_or(1.0, |squared| weight_at((squared as f64).sqrt()))
}

/// Sets `inside` (one flag a point of the lattice) at every point of `rect` within `radius_m` of a
/// point of `rect` that `sites` (one flag a point of `rect`, row by row) marks.
fn take_within(lattice: &Lattice, inside: &mut [bool], rect: Rect, sites: &[bool], radius_m: f64) {
    let near = column_pass(rect, sites);
    let limit = radius_m * radius_m;
    inside.par_chunks_mut(lattice.side).enumerate().skip(rect.b0).take(rect.height()).for_each(|(b, row)| {
        row_pass(rect, &near, b - rect.b0, |column, squared| {
            if squared.is_some_and(|squared| squared as f64 <= limit) {
                row[rect.a0 + column] = true;
            }
        });
    });
}

/// The first pass of the distance transform over `rect`: for each of its points, how many points up
/// or down its column, within `rect`, the nearest point that `sites` marks (one flag a point of
/// `rect`, row by row) lies; `NO_SITE` where its column has none. Column by column, in parallel,
/// each written only by itself; the result is column by column, `rect.height()` values each.
fn column_pass(rect: Rect, sites: &[bool]) -> Vec<u16> {
    let (width, height) = (rect.width(), rect.height());
    let mut near = vec![NO_SITE; width * height];
    near.par_chunks_mut(height).enumerate().for_each(|(column, near)| {
        let mut last: Option<usize> = None;
        for (row, value) in near.iter_mut().enumerate() {
            if sites[row * width + column] {
                last = Some(row);
            }
            if let Some(last) = last {
                *value = (row - last) as u16;
            }
        }
        let mut next: Option<usize> = None;
        for (row, value) in near.iter_mut().enumerate().rev() {
            if sites[row * width + column] {
                next = Some(row);
            }
            if let Some(next) = next {
                *value = (*value).min((next - row) as u16);
            }
        }
    });
    near
}

/// The second pass of the distance transform along row `row` of `rect` (counting from its top),
/// from the first pass's `near`: the squared distance in square metres from each point of the row to
/// the nearest marked point of `rect`, handed to `put` with the point's column counting from the
/// left of `rect`, or `None` when `rect` has no marked point. It is the lower envelope of the
/// parabolas `(x - q)² + near(q)²` of the columns `q` that see a marked point; every value is a sum
/// of two squares of whole numbers, exact.
fn row_pass(rect: Rect, near: &[u16], row: usize, mut put: impl FnMut(usize, Option<u64>)) {
    row_pass_from(rect, near, row, |column, found| put(column, found.map(|(squared, _)| squared)));
}

/// `row_pass`, each squared distance handed to `put` with the column (counting from the left of
/// `rect`) of the marked point it was measured to: that point lies `near` of that column and this row
/// up or down it.
fn row_pass_from(rect: Rect, near: &[u16], row: usize, mut put: impl FnMut(usize, Option<(u64, usize)>)) {
    let (width, height) = (rect.width(), rect.height());
    let along = |column: usize| near[column * height + row];
    let lifted = |column: usize| {
        let (across, along) = (column as u64, u64::from(along(column)));
        (across * across + along * along) as f64
    };
    // The columns whose parabolas make the envelope, left to right, and where each begins.
    let mut parabolas: Vec<usize> = Vec::new();
    let mut begins: Vec<f64> = Vec::new();
    for column in (0..width).filter(|&column| along(column) != NO_SITE) {
        while let (Some(&last), Some(&start)) = (parabolas.last(), begins.last()) {
            let meets = (lifted(column) - lifted(last)) / (2.0 * (column - last) as f64);
            if meets > start {
                parabolas.push(column);
                begins.push(meets);
                break;
            }
            parabolas.pop();
            begins.pop();
        }
        if parabolas.is_empty() {
            parabolas.push(column);
            begins.push(f64::NEG_INFINITY);
        }
    }
    if parabolas.is_empty() {
        for column in 0..width {
            put(column, None);
        }
        return;
    }
    let mut k = 0;
    for column in 0..width {
        while k + 1 < parabolas.len() && begins[k + 1] < column as f64 {
            k += 1;
        }
        let (across, along) = (column.abs_diff(parabolas[k]) as u64, u64::from(along(parabolas[k])));
        put(column, Some((across * across + along * along, parabolas[k])));
    }
}

/// For each of `cells` (world cells of a land `size_m` across, in any order; a cell beyond the world
/// gets `None`), the held or hand-shaped world cell (`marks`) nearest it by the exact Euclidean
/// distance, where one lies within `reach_m` of it, or `None`: of several as near, the one the
/// distance transform's two passes keep, the same on any thread count. One transform over the box
/// round `cells` grown by `reach_m`, so what it costs is that box's points, whatever the number of
/// fixed cells.
pub(crate) fn nearest_fixed_cells(size_m: u32, marks: &Marks, cells: &[usize], reach_m: f64) -> Vec<Option<usize>> {
    let lattice = Lattice::new(size_m);
    let points = || cells.iter().filter(|&&cell| cell < lattice.world_cells()).map(|&cell| lattice.world_point(cell));
    let Some(found) = bounds(points()) else {
        return vec![None; cells.len()];
    };
    let rect = found.grown(reach_m.floor() as usize, lattice.side);
    let mut sites = vec![false; rect.len()];
    sites.par_chunks_mut(rect.width()).enumerate().for_each(|(row, sites)| {
        for (column, site) in sites.iter_mut().enumerate() {
            *site = lattice.world_cell((rect.a0 + column, rect.b0 + row)).is_some_and(|cell| marks.fixed(cell));
        }
    });
    let near = column_pass(rect, &sites);
    // The rows of the box that hold a cell asked about, each with those cells' columns and places in
    // `cells`.
    let mut asked: Vec<Vec<(usize, usize)>> = vec![Vec::new(); rect.height()];
    for (at, &cell) in cells.iter().enumerate().filter(|&(_, &cell)| cell < lattice.world_cells()) {
        let (a, b) = lattice.world_point(cell);
        asked[b - rect.b0].push((a - rect.a0, at));
    }
    let limit = reach_m * reach_m;
    let rows: Vec<Vec<(usize, Option<usize>)>> = (asked.par_iter().enumerate())
        .map(|(row, asked)| {
            if asked.is_empty() {
                return Vec::new();
            }
            let mut nearest = vec![None; rect.width()];
            row_pass_from(rect, &near, row, |column, found| {
                let Some((squared, from)) = found.filter(|&(squared, _)| squared as f64 <= limit) else { return };
                let along = usize::from(near[from * rect.height() + row]);
                let site_row = if row >= along && sites[(row - along) * rect.width() + from] { row - along } else { row + along };
                debug_assert!(squared == (column.abs_diff(from).pow(2) + along * along) as u64);
                nearest[column] = lattice.world_cell((rect.a0 + from, rect.b0 + site_row));
            });
            asked.iter().map(|&(column, at)| (at, nearest[column])).collect()
        })
        .collect();
    let mut nearest = vec![None; cells.len()];
    for (at, cell) in rows.into_iter().flatten() {
        nearest[at] = cell;
    }
    nearest
}

/// For each cell of the land model's grid of a land `size_m` across, in its cell order, the distance
/// in metres from its centre to `line` (world metres) where that is at most `reach_m`, and
/// `f64::INFINITY` elsewhere; a line of one point is that point.
pub(crate) fn distances_to_line(line: &[(f64, f64)], size_m: u32, reach_m: f64) -> Vec<f64> {
    let lattice = Lattice::new(size_m);
    let (side, origin, cell_m) = (lattice.model_side, lattice.origin_m, f64::from(GROW_CELL_M));
    let mut near = vec![f64::INFINITY; side * side];
    // The cells whose centre lies within the reach of `low_m..=high_m` on one axis.
    let span = |low_m: f64, high_m: f64| {
        let first = ((low_m - reach_m - origin) / cell_m).ceil().clamp(0.0, side as f64) as usize;
        let end = (((high_m + reach_m - origin) / cell_m).floor() + 1.0).clamp(0.0, side as f64) as usize;
        first..end
    };
    let segments: Vec<((f64, f64), (f64, f64))> = match line {
        [] => Vec::new(),
        [only] => vec![(*only, *only)],
        _ => line.windows(2).map(|pair| (pair[0], pair[1])).collect(),
    };
    for (a, b) in segments {
        for j in span(a.1.min(b.1), a.1.max(b.1)) {
            for i in span(a.0.min(b.0), a.0.max(b.0)) {
                let apart_m = to_segment((origin + i as f64 * cell_m, origin + j as f64 * cell_m), a, b);
                let value = &mut near[j * side + i];
                if apart_m <= reach_m && apart_m < *value {
                    *value = apart_m;
                }
            }
        }
    }
    near
}

/// The distance from `p` to the segment from `a` to `b`, in metres.
pub(crate) fn to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let at = nearest_on_segment(p, a, b);
    (p.0 - at.0).hypot(p.1 - at.1)
}

/// The place on the segment from `a` to `b` nearest `p`.
pub(crate) fn nearest_on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let squared = dx * dx + dz * dz;
    let t = if squared > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / squared).clamp(0.0, 1.0) } else { 0.0 };
    (a.0 + dx * t, a.1 + dz * t)
}

/// The box round every point of `footprint`, or `None` when it has none.
fn footprint_box(lattice: &Lattice, footprint: &Footprint) -> Option<Rect> {
    let squares = footprint.model_cells.iter().enumerate().filter(|(_, &on)| on).map(|(cell, _)| lattice.square(cell));
    let discs = footprint.lake_discs.iter().filter_map(|&disc| lattice.disc_box(disc));
    squares.chain(discs).reduce(Rect::union)
}

/// One flag a point of `rect`, row by row: whether the point is one of `footprint`'s. `rect` holds
/// the footprint's box.
fn footprint_sites(lattice: &Lattice, footprint: &Footprint, rect: Rect) -> Vec<bool> {
    let mut sites = vec![false; rect.len()];
    for (cell, _) in footprint.model_cells.iter().enumerate().filter(|(_, &on)| on) {
        let square = lattice.square(cell);
        for b in square.b0..=square.b1 {
            for a in square.a0..=square.a1 {
                sites[rect.local((a, b))] = true;
            }
        }
    }
    for &(x_m, z_m, radius_m) in &footprint.lake_discs {
        let Some(disc) = lattice.disc_box((x_m, z_m, radius_m)) else { continue };
        for b in disc.b0..=disc.b1 {
            for a in disc.a0..=disc.a1 {
                if (lattice.at_m(a) - x_m).hypot(lattice.at_m(b) - z_m) <= radius_m {
                    sites[rect.local((a, b))] = true;
                }
            }
        }
    }
    sites
}

/// The points of the lattice a footprint covers (the flagged model cells' squares and the lakes'
/// discs, as `outline` reads them), or an area regrown as drawn (its world cells' points, as
/// `area_outline` reads them), read at the world's cells and at the land model's cells (a model
/// cell's own point, where its weight is read).
pub(crate) struct Covered {
    lattice: Lattice,
    /// The footprint's box, `None` for an empty footprint.
    rect: Option<Rect>,
    /// One flag a point of `rect`, row by row.
    sites: Vec<bool>,
}

impl Covered {
    /// What `footprint` covers on the lattice of a land `size_m` across.
    pub(crate) fn new(size_m: u32, footprint: &Footprint) -> Self {
        let lattice = Lattice::new(size_m);
        let rect = footprint_box(&lattice, footprint);
        let sites = rect.map_or_else(Vec::new, |rect| footprint_sites(&lattice, footprint, rect));
        Self { lattice, rect, sites }
    }

    /// What regrowing `area` as drawn covers on the lattice of a land `size_m` across: the points of
    /// its world cells (one flag a world cell; a flag beyond the world is not read), so a model cell
    /// is covered where its own point is one of them.
    pub(crate) fn of_area(size_m: u32, area: &[bool]) -> Self {
        let lattice = Lattice::new(size_m);
        let points = || area.iter().enumerate().take(lattice.world_cells()).filter(|(_, &on)| on).map(|(cell, _)| lattice.world_point(cell));
        let rect = bounds(points());
        let sites = rect.map_or_else(Vec::new, |rect| {
            let mut sites = vec![false; rect.len()];
            for point in points() {
                sites[rect.local(point)] = true;
            }
            sites
        });
        Self { lattice, rect, sites }
    }

    fn at(&self, point: (usize, usize)) -> bool {
        self.rect.is_some_and(|rect| rect.contains(point) && self.sites[rect.local(point)])
    }

    /// Whether world cell `cell` (in the world's cell order) is covered; a cell beyond the world is not.
    pub(crate) fn world_cell(&self, cell: usize) -> bool {
        cell < self.lattice.world_cells() && self.at(self.lattice.world_point(cell))
    }

    /// Whether model cell `cell` (in the land model's cell order) is covered at its own point; a cell
    /// beyond the grid is not.
    pub(crate) fn model_cell(&self, cell: usize) -> bool {
        cell < self.lattice.model_side * self.lattice.model_side && self.at(self.lattice.model_point(cell))
    }

    /// The world cells covered, in the world's cell order.
    pub(crate) fn world_cells(&self) -> impl Iterator<Item = usize> + '_ {
        let cells = self.rect.into_iter().flat_map(|rect| self.lattice.world_cells_in(rect));
        cells.filter(|&cell| self.at(self.lattice.world_point(cell)))
    }
}

/// The lattice of a land (the module's doc says where its points lie).
#[derive(Debug, Clone, Copy)]
struct Lattice {
    /// Points per side.
    side: usize,
    /// Cells per side of the land model's grid.
    model_side: usize,
    /// Cells per side of the world.
    size: usize,
    /// World `x` of the first column, and world `z` of the first row, in metres.
    origin_m: f64,
}

impl Lattice {
    fn new(size_m: u32) -> Self {
        let model_side = model_side(size_m);
        Self {
            side: CELL_POINTS * (model_side - 1) + 1,
            model_side,
            size: size_m as usize,
            origin_m: -f64::from(size_m) / 2.0 - f64::from(GROW_MARGIN_M),
        }
    }

    fn points(&self) -> usize {
        self.side * self.side
    }

    fn world_cells(&self) -> usize {
        self.size * self.size
    }

    /// The index of point `(a, b)`.
    fn index(&self, (a, b): (usize, usize)) -> usize {
        b * self.side + a
    }

    /// World `x` of column `index`, or world `z` of row `index`, in metres.
    fn at_m(&self, index: usize) -> f64 {
        self.origin_m + index as f64
    }

    /// The point of world cell `cell`.
    fn world_point(&self, cell: usize) -> (usize, usize) {
        (cell % self.size + MARGIN_POINTS, cell / self.size + MARGIN_POINTS)
    }

    /// The world cell at point `(a, b)`, or `None` for a point of the margin.
    fn world_cell(&self, (a, b): (usize, usize)) -> Option<usize> {
        let (i, j) = (a.checked_sub(MARGIN_POINTS)?, b.checked_sub(MARGIN_POINTS)?);
        (i < self.size && j < self.size).then_some(j * self.size + i)
    }

    /// The world cells on the world's border, each once: the first and last rows, then the first
    /// and last columns between them.
    fn border_cells(&self) -> impl Iterator<Item = usize> + '_ {
        let last = self.size - 1;
        let rows = (0..self.size).flat_map(move |i| [i, last * self.size + i]);
        let columns = (1..last).flat_map(move |j| [j * self.size, j * self.size + last]);
        rows.chain(columns)
    }

    /// The points whose nearest world cell is `cell`, on each axis the cell's own column (or row)
    /// held to the world: its own point, and for a cell on the world's border the points of the
    /// model's margin beyond it (at a corner, the margin's corner).
    fn nearest_to(&self, cell: usize) -> Rect {
        let span = |at: usize| match at {
            0 => (0, MARGIN_POINTS),
            at if at + 1 == self.size => (at + MARGIN_POINTS, self.side - 1),
            at => (at + MARGIN_POINTS, at + MARGIN_POINTS),
        };
        let ((a0, a1), (b0, b1)) = (span(cell % self.size), span(cell / self.size));
        Rect { a0, a1, b0, b1 }
    }

    /// The world cells whose points lie in `rect`.
    fn world_cells_in(&self, rect: Rect) -> impl Iterator<Item = usize> + '_ {
        let span = |first: usize, last: usize| first.max(MARGIN_POINTS) - MARGIN_POINTS..(last + 1).saturating_sub(MARGIN_POINTS).min(self.size);
        let columns = span(rect.a0, rect.a1);
        span(rect.b0, rect.b1).flat_map(move |j| columns.clone().map(move |i| j * self.size + i))
    }

    /// The point of model cell `cell`.
    fn model_point(&self, cell: usize) -> (usize, usize) {
        (cell % self.model_side * CELL_POINTS, cell / self.model_side * CELL_POINTS)
    }

    /// The points model cell `cell` covers in a footprint: those within `CELL_HALF_POINTS` of its
    /// own on both axes, on the lattice.
    fn square(&self, cell: usize) -> Rect {
        let (a, b) = self.model_point(cell);
        Rect::at((a, b)).grown(CELL_HALF_POINTS, self.side)
    }

    /// The columns (or rows) of the model cells that cover column (or row) `at` in a footprint.
    fn model_cells_covering(&self, at: usize) -> RangeInclusive<usize> {
        at.saturating_sub(CELL_HALF_POINTS).div_ceil(CELL_POINTS)..=((at + CELL_HALF_POINTS) / CELL_POINTS).min(self.model_side - 1)
    }

    /// The box of the points whose world `x` and `z` each lie within `radius_m` of a disc's centre,
    /// or `None` when none lies on the lattice.
    fn disc_box(&self, (x_m, z_m, radius_m): (f64, f64, f64)) -> Option<Rect> {
        let (a0, a1) = self.indices_near(x_m, radius_m)?;
        let (b0, b1) = self.indices_near(z_m, radius_m)?;
        Some(Rect { a0, a1, b0, b1 })
    }

    /// The first and last columns whose `x` (or rows whose `z`) lies within `reach_m` of `at_m`, or
    /// `None` when none does.
    fn indices_near(&self, at_m: f64, reach_m: f64) -> Option<(usize, usize)> {
        let first = (at_m - reach_m - self.origin_m).ceil().max(0.0);
        let last = (at_m + reach_m - self.origin_m).floor().min((self.side - 1) as f64);
        (first <= last).then_some((first as usize, last as usize))
    }

    fn all(&self) -> Rect {
        Rect { a0: 0, a1: self.side - 1, b0: 0, b1: self.side - 1 }
    }
}

/// A box of the lattice's points, its first and last columns and rows included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    a0: usize,
    a1: usize,
    b0: usize,
    b1: usize,
}

impl Rect {
    fn at((a, b): (usize, usize)) -> Self {
        Self { a0: a, a1: a, b0: b, b1: b }
    }

    fn width(&self) -> usize {
        self.a1 - self.a0 + 1
    }

    fn height(&self) -> usize {
        self.b1 - self.b0 + 1
    }

    fn len(&self) -> usize {
        self.width() * self.height()
    }

    fn union(self, other: Self) -> Self {
        Self { a0: self.a0.min(other.a0), a1: self.a1.max(other.a1), b0: self.b0.min(other.b0), b1: self.b1.max(other.b1) }
    }

    /// The box `by` points wider on every side, held to a lattice `side` points a side.
    fn grown(self, by: usize, side: usize) -> Self {
        Self { a0: self.a0.saturating_sub(by), a1: (self.a1 + by).min(side - 1), b0: self.b0.saturating_sub(by), b1: (self.b1 + by).min(side - 1) }
    }

    fn contains(&self, (a, b): (usize, usize)) -> bool {
        (self.a0..=self.a1).contains(&a) && (self.b0..=self.b1).contains(&b)
    }

    /// The place of point `(a, b)`, which lies in the box, among its points row by row.
    fn local(&self, (a, b): (usize, usize)) -> usize {
        (b - self.b0) * self.width() + (a - self.a0)
    }
}

/// The box round `points`, or `None` for none.
fn bounds(points: impl IntoIterator<Item = (usize, usize)>) -> Option<Rect> {
    points.into_iter().map(Rect::at).reduce(Rect::union)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::country::Country;
    use crate::grow::{grow, start_of};
    use crate::rng::SplitMix64;
    use crate::trial::{example_edit, EDIT_NAMES};
    use crate::water_rules::WaterRules;
    use crate::world::{finish_drawn, Frame, GenConfig};
    use std::sync::atomic::AtomicBool;
    use std::sync::OnceLock;
    use std::time::Instant;

    /// A regrowth of an area clears the hand marks on its cells that are not held, keeps those on
    /// its held cells and beyond it, and leaves the held marks as they are; a cell beyond the end of
    /// a list is not marked by it.
    #[test]
    fn a_regrown_area_clears_the_hand_marks_on_its_cells_that_are_not_held() {
        // Cells 0 to 5: in the area 1 to 4 (and 6, beyond the marks); held 2 and 4; by hand all but 5.
        let marks = Marks { held: vec![false, false, true, false, true, false], by_hand: vec![true, true, true, true, true, false] };
        let area = [false, true, true, true, true, false, true];
        let regrown = marks.regrown(&area);
        assert_eq!(regrown.held, marks.held);
        assert_eq!(regrown.by_hand, [true, false, true, false, true, false]);
        assert_eq!(marks.regrown(&[true]).by_hand, [false, true, true, true, true, false], "an area shorter than the marks");
        let short = Marks { held: vec![true], by_hand: vec![true, true] };
        assert_eq!(short.regrown(&[true, true]).by_hand, [true, false], "held marks shorter than the hand marks");
    }

    /// Hill country `size_m` across with no picture, no river and no lake.
    fn plain(size_m: u32) -> Drawing {
        Drawing { size_m, high_ground: None, rivers: Vec::new(), lakes: Vec::new(), country: Country::hills() }
    }

    /// The footprint of the one model cell whose point is `(a, b)` (each a multiple of 8), on a land
    /// `size_m` across.
    fn one_cell(size_m: u32, (a, b): (usize, usize)) -> Footprint {
        let side = model_side(size_m);
        let mut model_cells = vec![false; side * side];
        model_cells[(b / CELL_POINTS) * side + a / CELL_POINTS] = true;
        Footprint { model_cells, lake_discs: Vec::new(), whole: None }
    }

    /// The squared distance, in square metres, from point `point` to the square that the model cell
    /// at point `centre` covers in a footprint.
    fn from_square(point: (usize, usize), centre: (usize, usize)) -> u64 {
        let beyond = |at: usize, middle: usize| at.abs_diff(middle).saturating_sub(CELL_HALF_POINTS) as u64;
        let (across, down) = (beyond(point.0, centre.0), beyond(point.1, centre.1));
        across * across + down * down
    }

    /// The world cells within `radius_m` of world `(x, z)` on a land `size_m` across.
    fn disc_of_cells(size_m: u32, (x_m, z_m): (f64, f64), radius_m: f64) -> Vec<usize> {
        let size = size_m as usize;
        let half = f64::from(size_m) / 2.0;
        (0..size * size).filter(|&cell| ((cell % size) as f64 - half - x_m).hypot((cell / size) as f64 - half - z_m) <= radius_m).collect()
    }

    /// The distance in metres from `point` to the nearest point of the lattice within `look` of it
    /// on both axes for which `outside` holds, found point by point.
    fn nearest_by_search(lattice: &Lattice, point: (usize, usize), look: usize, outside: impl Fn((usize, usize)) -> bool) -> f64 {
        let span = |at: usize| at.saturating_sub(look)..=(at + look).min(lattice.side - 1);
        let mut nearest = u64::MAX;
        for b in span(point.1) {
            for a in span(point.0) {
                if outside((a, b)) {
                    let (across, down) = (a.abs_diff(point.0) as u64, b.abs_diff(point.1) as u64);
                    nearest = nearest.min(across * across + down * down);
                }
            }
        }
        (nearest as f64).sqrt()
    }

    /// `outline`'s outline and the one found over the whole lattice are equal bit for bit.
    fn assert_bounded_is_whole(size_m: u32, footprint: &Footprint, zones: &[Vec<usize>], marks: &Marks, reach_m: f32, case: &str) -> Outline {
        assert_taken_bounded_is_whole(size_m, Taken::Reach(footprint, reach_m), zones, marks, case)
    }

    /// The outline of what `taken` takes and the one found over the whole lattice are equal bit for
    /// bit.
    fn assert_taken_bounded_is_whole(size_m: u32, taken: Taken, zones: &[Vec<usize>], marks: &Marks, case: &str) -> Outline {
        let bounded = weigh(size_m, taken, zones, marks, Boxes::Bounded);
        let whole = weigh(size_m, taken, zones, marks, Boxes::Whole);
        let bits = |values: &[f32]| values.iter().map(|value| value.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&bounded.weight.data), bits(&whole.weight.data), "{case}: the world's weights");
        assert_eq!(bits(&bounded.model_weight), bits(&whole.model_weight), "{case}: the model's weights");
        assert_eq!((bounded.inside_m2, bounded.fixed_inside_m2), (whole.inside_m2, whole.fixed_inside_m2), "{case}: the areas");
        bounded
    }

    #[test]
    fn the_distances_are_exact_on_any_box() {
        let mut random = SplitMix64::new(7);
        for case in 0..300 {
            let (width, height) = (1 + random.next_below(40) as usize, 1 + random.next_below(40) as usize);
            let rect = Rect { a0: 3, a1: 2 + width, b0: 5, b1: 4 + height };
            let density = [0.0, 0.002, 0.02, 0.2, 0.9][case % 5];
            let sites: Vec<bool> = (0..rect.len()).map(|_| random.next_f64() < density).collect();
            let near = column_pass(rect, &sites);
            for row in 0..height {
                let mut found = vec![Some(u64::MAX); width];
                row_pass(rect, &near, row, |column, squared| found[column] = squared);
                for (column, &found) in found.iter().enumerate() {
                    let searched = (0..rect.len())
                        .filter(|&site| sites[site])
                        .map(|site| {
                            let (across, down) = ((site % width).abs_diff(column) as u64, (site / width).abs_diff(row) as u64);
                            across * across + down * down
                        })
                        .min();
                    assert_eq!(found, searched, "case {case}, {width} x {height} with {density} of sites, at ({column}, {row})");
                }
            }
        }
    }

    /// `nearest_fixed_cells` gives, for each cell asked about, a fixed cell as near as a search over
    /// every fixed cell finds, held or shaped by hand, where one lies within the reach, and `None`
    /// where none does or the cell lies beyond the world; on lands of few and of many fixed cells,
    /// with the cells asked about near the world's edges too.
    #[test]
    fn the_nearest_fixed_cell_is_as_near_as_a_search_over_every_fixed_cell_finds() {
        const SIZE_M: u32 = 96;
        const REACH_M: f64 = 30.0;
        let size = SIZE_M as usize;
        let place = |cell: usize| ((cell % size) as f64, (cell / size) as f64);
        let mut random = SplitMix64::new(11);
        for (case, density) in [0.0005, 0.003, 0.02, 0.3].into_iter().enumerate() {
            let mut marks = Marks::none(size);
            for cell in 0..size * size {
                if random.next_f64() < density {
                    if random.next_below(3) == 0 {
                        marks.by_hand[cell] = true;
                    } else {
                        marks.held[cell] = true;
                    }
                }
            }
            let mut asked: Vec<usize> = (0..200).map(|_| random.next_below((size * size) as u64) as usize).collect();
            asked.extend([0, size - 1, size * size - 1, size * size + 5]);
            let found = nearest_fixed_cells(SIZE_M, &marks, &asked, REACH_M);
            assert_eq!(found.len(), asked.len());
            for (&cell, &nearest) in asked.iter().zip(&found) {
                let searched = (0..size * size)
                    .filter(|&fixed| marks.fixed(fixed) && cell < size * size)
                    .map(|fixed| (place(fixed).0 - place(cell).0).hypot(place(fixed).1 - place(cell).1))
                    .fold(f64::INFINITY, f64::min);
                match nearest {
                    Some(fixed) => {
                        assert!(marks.fixed(fixed), "case {case}: cell {cell} names {fixed}, not fixed");
                        let apart_m = (place(fixed).0 - place(cell).0).hypot(place(fixed).1 - place(cell).1);
                        assert_eq!(apart_m, searched, "case {case}: cell {cell}");
                        assert!(apart_m <= REACH_M);
                    }
                    None => assert!(searched > REACH_M, "case {case}: cell {cell} has fixed ground {searched} m from it"),
                }
            }
        }
        assert!(nearest_fixed_cells(SIZE_M, &Marks::none(size), &[], REACH_M).is_empty());
    }

    #[test]
    fn a_start_that_differs_by_more_than_the_tolerance_gives_its_cells() {
        let drawing = Drawing::example_valley(512);
        let start = start_of(&drawing, &WaterRules::default(), 1).expect("the example is valid");
        assert!(footprint(&start, &start, &drawing, &drawing).is_empty(), "equal starts give a footprint");
        let cell = start.side * (start.side / 2) + start.side / 3;
        assert!(!start.outlets.contains(&cell));
        let rounds = f64::from(drawing.country.rounds);
        let cells_of = |change: &dyn Fn(&mut Start)| -> Vec<usize> {
            let mut changed = start.clone();
            change(&mut changed);
            let found = footprint(&start, &changed, &drawing, &drawing);
            assert!(found.lake_discs.is_empty(), "the lakes are the same");
            found.model_cells.iter().enumerate().filter(|(_, &on)| on).map(|(cell, _)| cell).collect()
        };
        let none: Vec<usize> = Vec::new();
        assert_eq!(cells_of(&|start| start.heights[cell] += 0.3), vec![cell], "0.3 m higher");
        assert_eq!(cells_of(&|start| start.heights[cell] -= 0.3), vec![cell], "0.3 m lower");
        assert_eq!(cells_of(&|start| start.heights[cell] += 0.2), none, "0.2 m higher");
        assert_eq!(cells_of(&|start| start.lift_m[cell] += 0.3 / rounds), vec![cell], "0.3 m more lift over the rounds");
        assert_eq!(cells_of(&|start| start.lift_m[cell] -= 0.2 / rounds), none, "0.2 m less lift over the rounds");
        assert_eq!(cells_of(&|start| start.incoming_m2[cell] += 1.0), vec![cell], "water arriving");
        assert_eq!(
            cells_of(&|start| {
                start.outlets.push(cell);
                start.outlets.sort_unstable();
            }),
            vec![cell],
            "an outlet"
        );
    }

    /// A drawn river whose width changes, which its start does not show, gives every model cell
    /// whose centre lies within `WIDTH_CHANGE_REACH_M` of its smoothed line, found here by a search
    /// over every cell, and nothing else; the same width gives nothing.
    #[test]
    fn a_rivers_width_changed_gives_the_cells_along_its_line() {
        let drawing = Drawing::example_valley(512);
        let water = WaterRules::default();
        let start = start_of(&drawing, &water, 1).expect("the example is valid");
        let mut wider = drawing.clone();
        wider.rivers[1].width_m *= 2.0;
        assert_eq!(start_of(&wider, &water, 1).expect("valid"), start, "the start does not show the width");
        let found = footprint(&start, &start, &drawing, &wider);
        let line = smoothed_line(&drawing.rivers[1].points).expect("a line");
        let lattice = Lattice::new(drawing.size_m);
        let expected: Vec<bool> = (0..start.side * start.side)
            .map(|cell| {
                let (a, b) = lattice.model_point(cell);
                let at = (lattice.at_m(a), lattice.at_m(b));
                line.windows(2).map(|pair| to_segment(at, pair[0], pair[1])).fold(f64::INFINITY, f64::min) <= WIDTH_CHANGE_REACH_M
            })
            .collect();
        assert!(expected.iter().filter(|&&on| on).count() > 20, "cells along the brook");
        assert_eq!(found.model_cells, expected);
        assert!(found.lake_discs.is_empty());
        assert!(footprint(&start, &start, &wider, &wider).is_empty(), "the same width again");
    }

    #[test]
    fn a_moved_lake_gives_a_disc_at_each_place() {
        let water = WaterRules::default();
        let mut old = plain(512);
        old.lakes.push(Lake { x_m: -100.0, z_m: 50.0, radius_m: 40.0 });
        let old_start = start_of(&old, &water, 1).expect("a valid drawing");
        let mut moved = old.clone();
        moved.lakes[0].x_m = 120.0;
        let found = footprint(&old_start, &start_of(&moved, &water, 1).expect("a valid drawing"), &old, &moved);
        assert!(!found.model_cells.contains(&true), "a lake does not change the start of a drawing without rivers");
        assert_eq!(found.lake_discs, vec![(-100.0, 50.0, 50.0), (120.0, 50.0, 50.0)]);
        // A lake of another radius is one lake removed and another added; the same lake is neither.
        let mut wider = old.clone();
        wider.lakes[0].radius_m = 60.0;
        let found = footprint(&old_start, &start_of(&wider, &water, 1).expect("a valid drawing"), &old, &wider);
        assert_eq!(found.lake_discs, vec![(-100.0, 50.0, 50.0), (-100.0, 50.0, 75.0)]);
        assert!(footprint(&old_start, &old_start, &old, &old).is_empty());
        // The disc covers the points within its radius.
        let size_m = 512;
        let lattice = Lattice::new(size_m);
        let disc = Footprint { model_cells: vec![false; lattice.model_side * lattice.model_side], lake_discs: vec![(-100.0, 50.0, 50.0)], whole: None };
        let rect = footprint_box(&lattice, &disc).expect("a disc on the lattice");
        let sites = footprint_sites(&lattice, &disc, rect);
        let covered = (rect.b0..=rect.b1).flat_map(|b| (rect.a0..=rect.a1).map(move |a| (a, b))).filter(|&point| sites[rect.local(point)]);
        let within = |(a, b): (usize, usize)| (lattice.at_m(a) + 100.0).hypot(lattice.at_m(b) - 50.0) <= 50.0;
        assert!(covered.clone().all(within));
        assert_eq!(covered.count(), disc_of_cells(size_m, (-100.0, 50.0), 50.0).len(), "the disc lies in the world, a point a cell");
    }

    #[test]
    fn an_early_outline_of_one_cell_is_a_disc_of_the_reach() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let centre = (768, 768);
        let found = early_outline(size_m, &one_cell(size_m, centre), &Marks::none(lattice.size), REACH_MIN_M);
        assert_eq!((found.weight.width, found.weight.height, found.weight.cell_size), (lattice.size, lattice.size, 1.0));
        assert_eq!(found.model_weight.len(), lattice.model_side * lattice.model_side);
        let (reach, band) = (300 * 300, 150 * 150);
        let mut inside = 0;
        for cell in 0..lattice.world_cells() {
            let point = lattice.world_point(cell);
            let (squared, weight) = (from_square(point, centre), found.weight.data[cell]);
            if squared <= reach {
                inside += 1;
                assert!(weight > 0.0, "{point:?}, {squared} m² from the cell, has weight {weight}");
            } else {
                assert_eq!(weight, 0.0, "{point:?}, {squared} m² from the cell, lies beyond the reach");
            }
            if squared <= band {
                assert_eq!(weight, 1.0, "{point:?} lies more than the band inside the rim");
            }
            let beside_rim = [(1, 0), (0, 1), (-1, 0), (0, -1)].iter().any(|&(across, down): &(isize, isize)| {
                from_square((point.0.saturating_add_signed(across), point.1.saturating_add_signed(down)), centre) > reach
            });
            if squared <= reach && beside_rim {
                assert!(weight <= weight_at(1.0), "{point:?} beside the rim has weight {weight}");
            }
        }
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (f64::from(inside), 0.0));
        // Going inward along each of eight rays from the cell, the weight never falls.
        for (across, down) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let weights: Vec<f32> = (0..=400)
                .map(|step: isize| {
                    let point = (centre.0.saturating_add_signed(across * step), centre.1.saturating_add_signed(down * step));
                    found.weight.data[lattice.world_cell(point).expect("the ray stays in the world")]
                })
                .collect();
            assert!(weights.windows(2).all(|pair| pair[0] >= pair[1]), "along ({across}, {down}): {weights:?}");
            assert_eq!((weights[0], weights[400]), (1.0, 0.0));
        }
        // A model cell takes the weight at its point; the outline lies inside the world.
        for (cell, &weight) in found.model_weight.iter().enumerate() {
            let point = lattice.model_point(cell);
            let at_world = lattice.world_cell(point).map_or(0.0, |world| found.weight.data[world]);
            assert_eq!(weight.to_bits(), at_world.to_bits(), "model cell {cell} at {point:?}");
        }
        assert!(found.model_weight.contains(&1.0) && found.model_weight.iter().any(|&weight| weight > 0.0 && weight < 1.0));
    }

    #[test]
    fn a_held_cell_has_weight_0_and_the_weight_rises_from_it_as_from_the_rim() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let (centre, held_at) = ((768, 768), (768, 868));
        let held_cell = lattice.world_cell(held_at).expect("in the world");
        let mut marks = Marks::none(lattice.size);
        marks.held[held_cell] = true;
        let cell = one_cell(size_m, centre);
        let open = early_outline(size_m, &cell, &Marks::none(lattice.size), REACH_MIN_M);
        let held = assert_bounded_is_whole(size_m, &cell, &[], &marks, REACH_MIN_M, "a held cell");
        assert_eq!(held.weight.data[held_cell], 0.0);
        assert!(open.weight.data[held_cell] > 0.0);
        assert_eq!((held.inside_m2, held.fixed_inside_m2), (open.inside_m2, 1.0));
        let outside = |point: (usize, usize)| from_square(point, centre) > 300 * 300;
        for step in 1..=40 {
            // Away from the held cell, the weight is that of the distance to it...
            let below = lattice.world_cell((held_at.0, held_at.1 + step)).expect("in the world");
            assert_eq!(held.weight.data[below], weight_at(step as f64), "{step} m below the held cell");
            // ...as it is that of the distance to the nearest point outside, in from the rim.
            let point = (1073 - step, 768);
            let from_rim = nearest_by_search(&lattice, point, 60, outside);
            assert!(from_rim <= step as f64);
            assert_eq!(open.weight.data[lattice.world_cell(point).expect("in the world")], weight_at(from_rim), "{step} m in from the rim");
        }
        // Beyond the band from the held cell, its mark changes nothing.
        for cell in 0..lattice.world_cells() {
            let (a, b) = lattice.world_point(cell);
            if (a.abs_diff(held_at.0) as f64).hypot(b.abs_diff(held_at.1) as f64) > f64::from(BAND_M) {
                assert_eq!(held.weight.data[cell].to_bits(), open.weight.data[cell].to_bits(), "{:?}", (a, b));
            }
        }
        // Shaped by hand is held as far as an outline goes.
        let by_hand = Marks { held: Vec::new(), by_hand: marks.held.clone() };
        assert_eq!(early_outline(size_m, &cell, &by_hand, REACH_MIN_M), held);
    }

    #[test]
    fn an_outline_that_touches_a_lake_zone_takes_it_whole() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        // The footprint lies 304 m west of the centre; zone A, round the centre, lies within its
        // reach; zone B lies beyond the reach but within the band and margin of A; zone C beyond both.
        let cell = one_cell(size_m, (464, 768));
        let (a, b, c) = ((0.0, 0.0, 30.0), (200.0, 0.0, 20.0), (0.0, 400.0, 20.0));
        let zone = |(x, z, r): (f64, f64, f64)| disc_of_cells(size_m, (x, z), r);
        let zones = vec![zone(a), zone(b), zone(c)];
        let found = assert_bounded_is_whole(size_m, &cell, &zones, &Marks::none(lattice.size), REACH_MIN_M, "three zones");
        let early = early_outline(size_m, &cell, &Marks::none(lattice.size), REACH_MIN_M);
        for &zone_cell in &zones[0] {
            assert_eq!(found.weight.data[zone_cell], 1.0, "zone A's cell {zone_cell}");
        }
        for &zone_cell in &zones[1] {
            assert_eq!(found.weight.data[zone_cell], 1.0, "zone B's cell {zone_cell}");
            assert_eq!(early.weight.data[zone_cell], 0.0, "zone B lies beyond the reach");
        }
        for &zone_cell in &zones[2] {
            assert_eq!(found.weight.data[zone_cell], 0.0, "zone C's cell {zone_cell}");
        }
        // The band and the margin round each zone taken lie inside the outline.
        let around = f64::from(BAND_M) + LAKE_ZONE_MARGIN_M - 1.0;
        for (x, z, r) in [a, b] {
            for zone_cell in disc_of_cells(size_m, (x, z), r + around) {
                assert!(found.weight.data[zone_cell] > 0.0, "{zone_cell} lies within the band and margin of a zone");
            }
        }
        assert!(found.inside_m2 > early.inside_m2);
        // The order of the zones does not matter.
        let reversed: Vec<Vec<usize>> = zones.iter().rev().cloned().collect();
        assert_eq!(outline(size_m, &cell, &reversed, &Marks::none(lattice.size), REACH_MIN_M), found);
        // A zone's index beyond the world's cells is not read.
        let mut beyond = zones.clone();
        beyond[2].push(lattice.world_cells() + 5);
        assert_eq!(outline(size_m, &cell, &beyond, &Marks::none(lattice.size), REACH_MIN_M), found);
    }

    /// Review Focus 1: an edit at the land's edge.
    #[test]
    fn an_outline_at_the_lands_edge_is_clipped_to_the_land_and_the_margin_takes_the_same_rule() {
        let size_m = 512;
        let lattice = Lattice::new(size_m);
        assert_eq!((lattice.model_side, lattice.side), (128, 1017));
        // The world's north-west corner, the lattice's two far corners, and beside the world's east
        // border in the margin.
        for centre in [(256, 256), (0, 0), (1016, 1016), (768, 480)] {
            let cell = one_cell(size_m, centre);
            let found = assert_bounded_is_whole(size_m, &cell, &[], &Marks::none(lattice.size), REACH_MIN_M, &format!("a cell at {centre:?}"));
            let within = |point: (usize, usize)| from_square(point, centre) <= 300 * 300;
            let mut inside = 0;
            for world in 0..lattice.world_cells() {
                let point = lattice.world_point(world);
                assert_eq!(found.weight.data[world] > 0.0, within(point), "{centre:?}: world cell at {point:?}");
                inside += usize::from(within(point));
            }
            assert_eq!(found.inside_m2, inside as f64, "{centre:?}: the world's cells inside");
            for (model, &weight) in found.model_weight.iter().enumerate() {
                let point = lattice.model_point(model);
                assert_eq!(weight > 0.0, within(point), "{centre:?}: model cell at {point:?}");
            }
            // In the margin as in the world, the weight is that of the distance to the nearest point
            // outside the outline on the lattice.
            for model in (0..found.model_weight.len()).step_by(7) {
                let point = lattice.model_point(model);
                if within(point) {
                    let nearest = nearest_by_search(&lattice, point, 160, |point| !within(point));
                    assert_eq!(found.model_weight[model], weight_at(nearest), "{centre:?}: model cell at {point:?}");
                }
            }
        }
        let corner = early_outline(size_m, &one_cell(size_m, (256, 256)), &Marks::none(lattice.size), REACH_MIN_M);
        assert!(corner.inside_m2 < std::f64::consts::PI * 304.0 * 304.0 / 2.0, "a quarter of the disc and the margin's part left out: {}", corner.inside_m2);
        // A disc of a lake whose centre lies in the margin, and one half off the lattice.
        for disc in [(-300.0, -300.0, 60.0), (-512.0, 0.0, 100.0)] {
            let footprint = Footprint { model_cells: vec![false; 128 * 128], lake_discs: vec![disc], whole: None };
            let found = assert_bounded_is_whole(size_m, &footprint, &[], &Marks::none(lattice.size), REACH_MIN_M, &format!("a disc {disc:?}"));
            assert!(found.inside_m2 > 0.0 && found.weight.data.iter().filter(|&&weight| weight > 0.0).count() as f64 == found.inside_m2);
        }
    }

    /// Review Focus 6: held ground in the way.
    #[test]
    fn a_footprint_wholly_on_held_cells_says_how_much_was_held() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let cell = one_cell(size_m, (768, 768));
        let held: Vec<usize> = disc_of_cells(size_m, (0.0, 0.0), 20.0);
        let mut marks = Marks::none(lattice.size);
        for &cell in &held {
            marks.held[cell] = true;
        }
        // The footprint's square lies on the held disc.
        let square = lattice.square(lattice.model_side * 96 + 96);
        assert!((square.b0..=square.b1).all(|b| (square.a0..=square.a1).all(|a| marks.fixed(lattice.world_cell((a, b)).expect("in the world")))));
        let open = early_outline(size_m, &cell, &Marks::none(lattice.size), REACH_MIN_M);
        let found = assert_bounded_is_whole(size_m, &cell, &[], &marks, REACH_MIN_M, "a held footprint");
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (open.inside_m2, held.len() as f64));
        for world in 0..lattice.world_cells() {
            if marks.fixed(world) {
                assert_eq!(found.weight.data[world], 0.0, "held cell {world}");
            } else {
                assert_eq!(found.weight.data[world] > 0.0, open.weight.data[world] > 0.0, "cell {world} round the held disc");
            }
        }
    }

    /// Review Focus 2, by hand: the whole land as the footprint.
    #[test]
    fn the_whole_land_is_taken_with_a_band_round_fixed_ground_only() {
        let size_m = 512;
        let lattice = Lattice::new(size_m);
        let whole = Footprint::whole(lattice.model_side);
        let found = early_outline(size_m, &whole, &Marks::none(lattice.size), REACH_MIN_M);
        assert!(found.weight.data.iter().chain(&found.model_weight).all(|&weight| weight == 1.0));
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (f64::from(size_m * size_m), 0.0));
        let held = disc_of_cells(size_m, (0.0, 0.0), 30.0);
        let mut marks = Marks::none(lattice.size);
        for &cell in &held {
            marks.by_hand[cell] = true;
        }
        let found = assert_bounded_is_whole(size_m, &whole, &[], &marks, REACH_MIN_M, "the whole land round a disc");
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (f64::from(size_m * size_m), held.len() as f64));
        assert!(found.model_weight.iter().filter(|&&weight| weight < 1.0).count() > 0);
        let half = f64::from(size_m) / 2.0;
        for cell in 0..lattice.world_cells() {
            let from_centre = ((cell % lattice.size) as f64 - half).hypot((cell / lattice.size) as f64 - half);
            let weight = found.weight.data[cell];
            if from_centre <= 30.0 {
                assert_eq!(weight, 0.0, "{cell} is shaped by hand");
            } else if from_centre < 170.0 {
                assert!(weight > 0.0 && weight < 1.0, "{cell}, {from_centre} m from the centre, has weight {weight}");
            } else if from_centre > 182.0 {
                assert_eq!(weight, 1.0, "{cell}, {from_centre} m from the centre");
            }
        }
    }

    #[test]
    fn bounded_boxes_give_what_the_whole_lattice_gives() {
        let size_m = 512;
        let lattice = Lattice::new(size_m);
        let mut random = SplitMix64::new(3);
        for case in 0..6 {
            let count = lattice.model_side * lattice.model_side;
            let mut model_cells = vec![false; count];
            for _ in 0..1 + random.next_below(5) {
                model_cells[random.next_below(count as u64) as usize] = true;
            }
            let place = |random: &mut SplitMix64| (random.next_f64() * 900.0 - 450.0, random.next_f64() * 900.0 - 450.0);
            let (x, z) = place(&mut random);
            let footprint = Footprint { model_cells, lake_discs: vec![(x, z, 10.0 + random.next_f64() * 90.0)], whole: None };
            let zones: Vec<Vec<usize>> = (0..3).map(|_| disc_of_cells(size_m, place(&mut random), 10.0 + random.next_f64() * 40.0)).collect();
            let mut marks = Marks::none(lattice.size);
            for cell in disc_of_cells(size_m, place(&mut random), 5.0 + random.next_f64() * 40.0) {
                marks.held[cell] = true;
            }
            let reach_m = [REACH_MIN_M, 450.5, REACH_MAX_M][case % 3];
            assert_bounded_is_whole(size_m, &footprint, &zones, &marks, reach_m, &format!("case {case}"));
        }
        let empty = Footprint { model_cells: vec![false; lattice.model_side * lattice.model_side], lake_discs: Vec::new(), whole: None };
        let found = assert_bounded_is_whole(size_m, &empty, &[disc_of_cells(size_m, (0.0, 0.0), 30.0)], &Marks::none(lattice.size), REACH_MIN_M, "empty");
        assert!(found.weight.data.iter().chain(&found.model_weight).all(|&weight| weight == 0.0));
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (0.0, 0.0));
    }

    #[test]
    fn an_outline_is_the_same_on_any_thread_count() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let mut model_cells = vec![false; lattice.model_side * lattice.model_side];
        for cell in [40 * lattice.model_side + 50, 41 * lattice.model_side + 50, 120 * lattice.model_side + 130] {
            model_cells[cell] = true;
        }
        let footprint = Footprint { model_cells, lake_discs: vec![(200.0, -150.0, 75.0)], whole: None };
        let zones = vec![disc_of_cells(size_m, (100.0, -300.0), 40.0), disc_of_cells(size_m, (-400.0, 400.0), 30.0)];
        let mut marks = Marks::none(lattice.size);
        for cell in disc_of_cells(size_m, (-150.0, -200.0), 25.0) {
            marks.held[cell] = true;
        }
        let on = |threads: usize| {
            let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
            pool.install(|| outline(size_m, &footprint, &zones, &marks, 420.0))
        };
        let one = on(1);
        assert!(one.inside_m2 > 0.0 && one.fixed_inside_m2 > 0.0);
        let bits = |found: &Outline| (found.weight.data.iter().chain(&found.model_weight).map(|w| w.to_bits()).collect::<Vec<_>>(), found.inside_m2, found.fixed_inside_m2);
        for threads in [4, 16] {
            assert!(bits(&on(threads)) == bits(&one), "{threads} threads");
        }
    }

    #[test]
    fn the_reach_is_held_to_the_lands_setting() {
        let size_m = 1024;
        let cell = one_cell(size_m, (768, 768));
        let none = Marks::none(size_m as usize);
        let at = |reach_m: f32| early_outline(size_m, &cell, &none, reach_m);
        assert_eq!(at(100.0), at(REACH_MIN_M));
        assert_eq!(at(f32::NAN), at(REACH_MIN_M));
        assert_eq!(at(1000.0), at(REACH_MAX_M));
        assert!(at(REACH_MAX_M).inside_m2 > at(450.0).inside_m2 && at(450.0).inside_m2 > at(REACH_MIN_M).inside_m2);
    }

    #[test]
    fn the_footprint_of_an_area_covers_it_with_the_model_cells_it_touches() {
        let size_m = 512;
        let lattice = Lattice::new(size_m);
        let mut area = vec![false; lattice.world_cells()];
        for cell in disc_of_cells(size_m, (100.0, -50.0), 30.0).into_iter().chain([0, lattice.world_cells() - 1, 7 * lattice.size + 300]) {
            area[cell] = true;
        }
        let found = footprint_of_area(size_m, &area);
        assert!(found.lake_discs.is_empty());
        let squares: Vec<Rect> = (0..found.model_cells.len()).filter(|&cell| found.model_cells[cell]).map(|cell| lattice.square(cell)).collect();
        let points: Vec<(usize, usize)> = (0..area.len()).filter(|&cell| area[cell]).map(|cell| lattice.world_point(cell)).collect();
        assert!(points.iter().all(|&point| squares.iter().any(|square| square.contains(point))), "every cell of the area is covered");
        assert!(squares.iter().all(|square| points.iter().any(|&point| square.contains(point))), "every model cell touches the area");
        assert!(footprint_of_area(size_m, &vec![false; lattice.world_cells()]).is_empty());
        let mut longer = area.clone();
        longer.extend([true; 10]);
        assert_eq!(footprint_of_area(size_m, &longer), found, "a flag beyond the world is not read");
    }

    /// One flag a world cell of a land `size_m` across: the cells within `radius_m` of `(x, z)`.
    fn area_of(size_m: u32, at: (f64, f64), radius_m: f64) -> Vec<bool> {
        let mut area = vec![false; size_m as usize * size_m as usize];
        disc_of_cells(size_m, at, radius_m).into_iter().for_each(|cell| area[cell] = true);
        area
    }

    /// Stage 2's Task 3 (question 1, answer (a), Review Focus 8): the early outline of a disc of 100 m
    /// regrown as drawn is the disc: weight 0 on every cell beside it and beyond, so its rim lies on
    /// the disc's edge, where each of its cells beside one outside weighs at most what 1 m weighs; above
    /// 0 on every cell of it, each the weight of its distance to the nearest point outside (found by a
    /// search); nowhere 1, a disc 200 m across lying nowhere more than the band inside its edge. The
    /// boxes give what the whole lattice gives, a model cell takes the weight at its point, a flag
    /// beyond the world is not read, and an area of no cell gives an empty outline.
    #[test]
    fn an_early_area_outline_of_a_disc_is_the_disc_its_band_inside_it_and_nowhere_1() {
        const RADIUS_M: f64 = 100.0;
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let none = Marks::none(lattice.size);
        let area = area_of(size_m, (100.0, -50.0), RADIUS_M);
        let cells = area.iter().filter(|&&on| on).count();
        let found = assert_taken_bounded_is_whole(size_m, Taken::Area(&area), &[], &none, "a disc of 100 m");
        assert_eq!(found, early_area_outline(size_m, &area, &none));
        assert_eq!(found, area_outline(size_m, &area, &[], &none));
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (cells as f64, 0.0));
        let outside = |point: (usize, usize)| lattice.world_cell(point).is_none_or(|cell| !area[cell]);
        let beside_outside = |(a, b): (usize, usize)| [(a + 1, b), (a - 1, b), (a, b + 1), (a, b - 1)].into_iter().any(outside);
        let mut rim = 0;
        for (cell, &weight) in found.weight.data.iter().enumerate() {
            let point = lattice.world_point(cell);
            if outside(point) {
                assert_eq!(weight, 0.0, "{point:?} lies outside the area");
                continue;
            }
            assert!(weight > 0.0 && weight < 1.0, "{point:?} in the area weighs {weight}");
            if beside_outside(point) {
                rim += 1;
                assert!(weight <= weight_at(1.0), "{point:?} on the area's edge weighs {weight}");
            }
        }
        assert!(rim > 0, "the disc has an edge");
        for cell in (0..area.len()).filter(|&cell| area[cell]).step_by(9) {
            let point = lattice.world_point(cell);
            let nearest = nearest_by_search(&lattice, point, RADIUS_M as usize + 2, outside);
            assert_eq!(found.weight.data[cell], weight_at(nearest), "{point:?}, {nearest} m from the nearest point outside");
        }
        let most = found.weight.data.iter().copied().fold(0.0, f32::max);
        assert!(most > weight_at(RADIUS_M) && most < weight_at(RADIUS_M + 1.0), "the most is at the disc's middle: {most}");
        for (cell, &weight) in found.model_weight.iter().enumerate() {
            let point = lattice.model_point(cell);
            let at_world = lattice.world_cell(point).map_or(0.0, |world| found.weight.data[world]);
            assert_eq!(weight.to_bits(), at_world.to_bits(), "model cell {cell} at {point:?}");
        }
        let mut longer = area.clone();
        longer.extend([true; 10]);
        assert_eq!(early_area_outline(size_m, &longer, &none), found, "a flag beyond the world is not read");
        let empty = early_area_outline(size_m, &vec![false; area.len()], &none);
        assert!(empty.weight.data.iter().chain(&empty.model_weight).all(|&weight| weight == 0.0));
        assert_eq!((empty.inside_m2, empty.fixed_inside_m2), (0.0, 0.0));
        println!("a disc of {RADIUS_M} m: {cells} cells inside, {rim} on its edge, the most weight {most}");
    }

    /// Stage 2's ruling R4 (S2): an area that reaches the world's border takes the points of
    /// the model's margin whose nearest world cell is its own, so its band does not run along the
    /// border. A strip on the west border and a square at the south-east corner: the boxes give what
    /// the whole lattice gives; every sampled cell of the area weighs `weight_at` its distance to the
    /// nearest point whose nearest world cell lies outside the area, found by search, and every other
    /// cell 0; the model's margin beyond the strip and beyond the corner weighs 1, and beyond a
    /// border cell outside the area 0.
    #[test]
    fn an_area_at_the_worlds_border_takes_the_models_margin_beyond_it() {
        /// The strip's width and the square's side, in metres, and one cell in this many sampled.
        const STRIP_M: usize = 40;
        const SQUARE_M: usize = 30;
        const SAMPLED_EVERY: usize = 7;
        /// A column of the world outside both, whose border cell's margin the area does not take.
        const OUTSIDE_COLUMN: usize = 104;
        let size_m = 256;
        let lattice = Lattice::new(size_m);
        let size = lattice.size;
        let area: Vec<bool> = (0..size * size)
            .map(|cell| {
                let (i, j) = (cell % size, cell / size);
                i < STRIP_M || (i >= size - SQUARE_M && j >= size - SQUARE_M)
            })
            .collect();
        let none = Marks::none(size);
        let found = assert_taken_bounded_is_whole(size_m, Taken::Area(&area), &[], &none, "an area at the border");
        assert_eq!(found.inside_m2, area.iter().filter(|&&on| on).count() as f64, "the area's world cells");
        let nearest_cell = |(a, b): (usize, usize)| {
            let held = |at: usize| at.saturating_sub(MARGIN_POINTS).min(size - 1);
            held(b) * size + held(a)
        };
        let outside = |point: (usize, usize)| !area[nearest_cell(point)];
        for cell in (0..size * size).step_by(SAMPLED_EVERY) {
            let point = lattice.world_point(cell);
            let expected = if area[cell] { weight_at(nearest_by_search(&lattice, point, BAND_M as usize + 2, outside)) } else { 0.0 };
            assert_eq!(found.weight.data[cell], expected, "{point:?}");
        }
        assert_eq!(found.weight.data[size / 2 * size], weight_at(STRIP_M as f64), "the strip's border cell");
        let model = |(a, b): (usize, usize)| found.model_weight[b / CELL_POINTS * lattice.model_side + a / CELL_POINTS];
        let last = lattice.side - 1;
        assert_eq!(model((0, last / 2 / CELL_POINTS * CELL_POINTS)), 1.0, "the margin west of the strip");
        assert_eq!(model((last, last)), 1.0, "the margin's corner beyond the square");
        let north_of_outside = (OUTSIDE_COLUMN + MARGIN_POINTS, 0);
        assert_eq!(north_of_outside.0 % CELL_POINTS, 0, "a model cell's point");
        assert_eq!(model(north_of_outside), 0.0, "the margin beyond a border cell outside the area");
    }

    /// Stage 2's Task 3: a held cell in an area weighs 0 and the weight rises from it as from the
    /// area's edge; shaped by hand is held as far as an area's outline goes.
    #[test]
    fn a_held_cell_in_an_area_weighs_0_and_the_weight_rises_from_it() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let area = area_of(size_m, (0.0, 0.0), 100.0);
        let centre = lattice.world_point(lattice.size / 2 * lattice.size + lattice.size / 2);
        let held_cell = lattice.world_cell(centre).expect("in the world");
        assert!(area[held_cell]);
        let mut marks = Marks::none(lattice.size);
        marks.held[held_cell] = true;
        let found = assert_taken_bounded_is_whole(size_m, Taken::Area(&area), &[], &marks, "a held cell in an area");
        let open = early_area_outline(size_m, &area, &Marks::none(lattice.size));
        assert_eq!(found.weight.data[held_cell], 0.0);
        assert_eq!((found.inside_m2, found.fixed_inside_m2), (open.inside_m2, 1.0));
        for step in 1..=40 {
            let east = lattice.world_cell((centre.0 + step, centre.1)).expect("in the world");
            assert_eq!(found.weight.data[east], weight_at(step as f64), "{step} m east of the held cell");
        }
        let by_hand = Marks { held: Vec::new(), by_hand: marks.held.clone() };
        assert_eq!(early_area_outline(size_m, &area, &by_hand), found);
    }

    /// Stage 2's Task 3: an area's outline takes every lake zone the area touches whole, with the
    /// band and the zone's margin round it, and a zone that ground reaches; one beyond both is not
    /// taken. A taken zone's cells weigh 1, the order of the zones does not matter, and a zone's index
    /// beyond the world's cells is not read.
    #[test]
    fn an_areas_outline_takes_the_lake_zones_it_touches_whole_with_their_band() {
        let size_m = 1024;
        let lattice = Lattice::new(size_m);
        let none = Marks::none(lattice.size);
        // Zone A overlaps the area; zone B lies beyond the area but within the band and margin of A;
        // zone C beyond both.
        let area = area_of(size_m, (0.0, 0.0), 30.0);
        let (a, b, c) = ((40.0, 0.0, 20.0), (240.0, 0.0, 20.0), (0.0, 400.0, 20.0));
        let zone = |(x, z, r): (f64, f64, f64)| disc_of_cells(size_m, (x, z), r);
        let zones = vec![zone(a), zone(b), zone(c)];
        let found = assert_taken_bounded_is_whole(size_m, Taken::Area(&area), &zones, &none, "three zones round an area");
        let early = early_area_outline(size_m, &area, &none);
        for &zone_cell in zones[0].iter().chain(&zones[1]) {
            assert_eq!(found.weight.data[zone_cell], 1.0, "zone A's or B's cell {zone_cell}");
        }
        for &zone_cell in &zones[1] {
            assert_eq!(early.weight.data[zone_cell], 0.0, "zone B lies beyond the area");
        }
        for &zone_cell in &zones[2] {
            assert_eq!(found.weight.data[zone_cell], 0.0, "zone C's cell {zone_cell}");
        }
        let around = f64::from(BAND_M) + LAKE_ZONE_MARGIN_M - 1.0;
        for (x, z, r) in [a, b] {
            for zone_cell in disc_of_cells(size_m, (x, z), r + around) {
                assert!(found.weight.data[zone_cell] > 0.0, "{zone_cell} lies within the band and margin of a zone");
            }
        }
        assert!(found.inside_m2 > early.inside_m2 && early.inside_m2 == area.iter().filter(|&&on| on).count() as f64);
        let reversed: Vec<Vec<usize>> = zones.iter().rev().cloned().collect();
        assert_eq!(area_outline(size_m, &area, &reversed, &none), found);
        let mut beyond = zones.clone();
        beyond[2].push(lattice.world_cells() + 5);
        assert_eq!(area_outline(size_m, &area, &beyond, &none), found);
        // Without zone A the area touches none.
        assert_eq!(area_outline(size_m, &area, &zones[1..], &none), early);
    }

    /// The seed the example land of these tests is grown from.
    pub(crate) const EXAMPLE_SEED: u64 = 11;

    /// The land the example's edits are made to: the example at 2,048 m in hill country from
    /// `EXAMPLE_SEED`, its start, its land model and its world in the model's frame, as a land keeps
    /// them; grown once for every test that reads it.
    pub(crate) struct OldLand {
        pub(crate) drawing: Drawing,
        pub(crate) start: Start,
        pub(crate) model: Grown,
        pub(crate) world: World,
    }

    pub(crate) fn old_land() -> &'static OldLand {
        static LAND: OnceLock<OldLand> = OnceLock::new();
        LAND.get_or_init(|| {
            let (drawing, config) = (Drawing::example_valley(2048), GenConfig::default());
            let start = start_of(&drawing, &config.water, EXAMPLE_SEED).expect("the example is valid");
            let model = grow(&drawing, &config.water, EXAMPLE_SEED, &AtomicBool::new(false)).expect("the example grows");
            let world = finish_drawn(EXAMPLE_SEED, &drawing, &config, &model, Frame::Model, &AtomicBool::new(false)).expect("the example's world");
            OldLand { drawing, start, model, world }
        })
    }

    /// The milliseconds since `clock`, and the clock started again.
    fn lap(clock: &mut Instant) -> f64 {
        let ms = clock.elapsed().as_secs_f64() * 1000.0;
        *clock = Instant::now();
        ms
    }

    /// The prototype's areas inside the outline at a tolerance of 0.25 m, in hectares of the world,
    /// and its footprints' model cells (`report2.md`, table 1), for each of `EDIT_NAMES`.
    const PROTOTYPE_HA: [(&str, f64, usize); 9] = [
        ("a", 96.0, 1278),
        ("b", 102.0, 1978),
        ("c", 102.0, 1913),
        ("d", 51.0, 923),
        ("e1", 44.0, 0),
        ("e2", 64.0, 0),
        ("f", 67.0, 851),
        ("g", 100.0, 1561),
        ("h", 143.0, 4428),
    ];
    /// The table gives whole hectares; an outline's area is asked to lie within this many of it.
    const PROTOTYPE_WITHIN_HA: f64 = 2.0;
    const M2_PER_HA: f64 = 10_000.0;

    #[test]
    fn the_examples_edits_have_the_prototypes_outlines() {
        let old = old_land();
        let water = WaterRules::default();
        let none = Marks::none(old.world.heights.width);
        assert_eq!(PROTOTYPE_HA.map(|(name, _, _)| name), EDIT_NAMES);
        // The edits' models first, all grown at once.
        let models: Vec<(Drawing, Grown)> = std::thread::scope(|scope| {
            let growing: Vec<_> = EDIT_NAMES
                .iter()
                .map(|&name| {
                    scope.spawn(move || {
                        let drawing = example_edit(&old.drawing, name).expect("an edit of the example");
                        let model = grow(&drawing, &water, EXAMPLE_SEED, &AtomicBool::new(false)).expect("the edited drawing grows");
                        (drawing, model)
                    })
                })
                .collect();
            growing.into_iter().map(|model| model.join().expect("an edit grows")).collect()
        });
        for ((name, prototype_ha, prototype_cells), (drawing, model)) in PROTOTYPE_HA.iter().zip(&models) {
            let mut clock = Instant::now();
            let new_start = start_of(drawing, &water, EXAMPLE_SEED).expect("the edited drawing is valid");
            let found = footprint(&old.start, &new_start, &old.drawing, drawing);
            let footprint_ms = lap(&mut clock);
            let early = early_outline(drawing.size_m, &found, &none, REACH_MIN_M);
            let early_ms = lap(&mut clock);
            let zones = lake_zones(&old.world, &old.model, model, &old.drawing, drawing);
            let zones_ms = lap(&mut clock);
            let shape = outline(drawing.size_m, &found, &zones, &none, REACH_MIN_M);
            let outline_ms = lap(&mut clock);
            let cells = found.model_cells.iter().filter(|&&on| on).count();
            let (ha, early_ha) = (shape.inside_m2 / M2_PER_HA, early.inside_m2 / M2_PER_HA);
            println!(
                "edit {name}: footprint {cells} model cells and {} discs; outline {ha:.2} ha (prototype {prototype_ha} ha), early outline {early_ha:.2} ha; zones {:?} cells; start and footprint {footprint_ms:.1} ms, early outline {early_ms:.1} ms, lake zones {zones_ms:.1} ms, outline {outline_ms:.1} ms",
                found.lake_discs.len(),
                zones.iter().map(Vec::len).collect::<Vec<_>>()
            );
            assert!((ha - prototype_ha).abs() <= PROTOTYPE_WITHIN_HA, "edit {name}: {ha} ha inside the outline, the prototype {prototype_ha} ha");
            assert_eq!(cells, *prototype_cells, "edit {name}: the footprint's model cells");
            assert_eq!(found.lake_discs.len(), usize::from(name.starts_with('e')), "edit {name}: the lakes' discs");
            assert!(shape.inside_m2 >= early.inside_m2 && shape.fixed_inside_m2 == 0.0);
            if *name == "a" {
                assert_bounded_is_whole(drawing.size_m, &found, &zones, &none, REACH_MIN_M, "edit a");
            }
        }
    }

    /// Review Focus 2, on the example: an edit that changes nothing, and edits that change
    /// everything, each saying why (the whole-branch review's S2): another seed, a number of the
    /// country, two numbers, another kind of country.
    #[test]
    fn the_same_drawing_gives_nothing_and_another_seed_or_country_gives_everything() {
        let old = old_land();
        let water = WaterRules::default();
        let again = start_of(&old.drawing, &water, EXAMPLE_SEED).expect("the example is valid");
        let same = footprint(&old.start, &again, &old.drawing, &old.drawing);
        assert!(same.is_empty(), "the same drawing");
        let none = Marks::none(old.world.heights.width);
        let shape = outline(2048, &same, &lake_zones(&old.world, &old.model, &old.model, &old.drawing, &old.drawing), &none, REACH_MIN_M);
        assert_eq!((shape.inside_m2, shape.fixed_inside_m2), (0.0, 0.0));
        assert!(shape.weight.data.iter().chain(&shape.model_weight).all(|&weight| weight == 0.0));

        let lake = old.drawing.lakes[0];
        let whole = |why: Whole| Footprint { lake_discs: vec![(lake.x_m, lake.z_m, LAKE_DISC_RADII * f64::from(lake.radius_m))], whole: Some(why), ..Footprint::whole(old.start.side) };
        // The cells the start alone would leave out of the footprint, comparing over the more rounds.
        let left_out = |start: &Start, drawing: &Drawing| {
            let rounds = old.drawing.country.rounds.max(drawing.country.rounds);
            starts_differ(&old.start, start, rounds).iter().filter(|&&differs| !differs).count()
        };
        let cells = old.start.side * old.start.side;
        for seed in [12, 23, 42] {
            let other = start_of(&old.drawing, &water, seed).expect("the example is valid");
            assert_eq!(other.seed, seed);
            assert_eq!(footprint(&old.start, &other, &old.drawing, &old.drawing), whole(Whole::Seed), "seed {seed}");
            let left = left_out(&other, &old.drawing);
            println!("seed {seed}: comparing the starts alone leaves {left} of {cells} model cells out");
            assert!(left > 0, "seed {seed}");
        }
        // A country number the start does not read: the starts are equal, the land is not.
        let cut = Drawing { country: Country { cut_strength: old.drawing.country.cut_strength * 2.0, ..old.drawing.country }, ..old.drawing.clone() };
        let cut_start = start_of(&cut, &water, EXAMPLE_SEED).expect("a valid drawing");
        assert_eq!(cut_start, old.start, "doubling the cut leaves the start as it is");
        assert_eq!(left_out(&cut_start, &cut), cells);
        assert_eq!(footprint(&old.start, &cut_start, &old.drawing, &cut), whole(Whole::Numbers(vec!["cut_strength"])), "the cut doubled");
        assert_eq!(Whole::Numbers(vec!["cut_strength"]).reason(), "a number of the country changed: cut_strength");
        let two = Drawing { country: Country { rounds: 91, ..cut.country }, ..cut.clone() };
        let two_start = start_of(&two, &water, EXAMPLE_SEED).expect("a valid drawing");
        let numbers = Whole::Numbers(vec!["cut_strength", "rounds"]);
        assert_eq!(footprint(&old.start, &two_start, &old.drawing, &two), whole(numbers.clone()), "the cut doubled and a round more");
        assert_eq!(numbers.reason(), "numbers of the country changed: cut_strength and rounds");
        assert_eq!(Whole::Numbers(vec!["floor_m", "relief_m", "rounds"]).reason(), "numbers of the country changed: floor_m, relief_m and rounds");
        // The seed is named before the country.
        let reseeded = start_of(&cut, &water, 23).expect("a valid drawing");
        assert_eq!(footprint(&old.start, &reseeded, &old.drawing, &cut).whole, Some(Whole::Seed));
        for name in ["gentle", "mountains"] {
            let other = Drawing { country: Country::named(name).expect("a known kind"), ..old.drawing.clone() };
            let start = start_of(&other, &water, EXAMPLE_SEED).expect("a valid drawing");
            assert_eq!(footprint(&old.start, &start, &old.drawing, &other), whole(Whole::Kind), "{name}");
            assert_eq!(Whole::Kind.reason(), "the kind of country changed");
            let tuned = Drawing { country: Country { relief_m: other.country.relief_m + 1.0, ..other.country }, ..other.clone() };
            assert_eq!(Whole::of_countries(&old.drawing.country, &tuned.country), Some(Whole::Numbers(country_numbers_changed(&old.drawing.country, &tuned.country))), "{name} tuned is not a kind");
            println!("{name}: comparing the starts alone leaves {} of {cells} model cells out", left_out(&start, &other));
        }
        // Each lake of either drawing once: an added lake joins the example's.
        let added = example_edit(&old.drawing, "e1").expect("an edit of the example");
        let other = start_of(&added, &water, 23).expect("a valid drawing");
        let discs = footprint(&old.start, &other, &old.drawing, &added).lake_discs;
        assert_eq!(discs, vec![whole(Whole::Seed).lake_discs[0], (-500.0, 300.0, 75.0)]);
        assert_eq!(Whole::Seed.reason(), "the seed changed");
        assert_eq!(same.whole, None, "the same drawing");
        let shape = early_outline(2048, &whole(Whole::Seed), &none, REACH_MIN_M);
        assert_eq!(shape.inside_m2, 2048.0 * 2048.0);
    }

    #[test]
    fn a_lakes_zone_holds_its_water_in_the_land_and_in_both_models() {
        let old = old_land();
        let water = WaterRules::default();
        let size = old.world.heights.width;
        let half = size as f64 / 2.0;
        let lake = old.drawing.lakes[0];
        let area_m2 = old.world.water.lakes.iter().find(|water| water.drawn == 0).expect("the example's lake holds water").area_m2;
        for name in ["e1", "e2"] {
            let drawing = example_edit(&old.drawing, name).expect("an edit of the example");
            let model = grow(&drawing, &water, EXAMPLE_SEED, &AtomicBool::new(false)).expect("the edited drawing grows");
            let zones = lake_zones(&old.world, &old.model, &model, &old.drawing, &drawing);
            assert_eq!(zones.len(), 2, "{name}: the example's lake and the new one");
            for (index, zone) in zones.iter().enumerate() {
                assert!(zone.windows(2).all(|pair| pair[0] < pair[1]) && zone.iter().all(|&cell| cell < size * size), "{name}: zone {index}");
            }
            let at = |cell: usize| ((cell % size) as f64 - half, (cell / size) as f64 - half);
            let reach = LAKE_REACH_RADII * f64::from(lake.radius_m) + POCKET_MARGIN_M;
            assert!(zones[0].len() as f32 >= area_m2, "{name}: the zone holds the lake's {area_m2} m² of water and more: {}", zones[0].len());
            assert!(zones[0].iter().all(|&cell| (at(cell).0 - lake.x_m).hypot(at(cell).1 - lake.z_m) <= reach), "{name}: zone 0 lies round its lake");
            let added = drawing.lakes[1];
            let reach = LAKE_REACH_RADII * f64::from(added.radius_m);
            assert!(model.lakes.iter().any(|grown| grown.drawn == 1) == !zones[1].is_empty(), "{name}: the new lake's zone is where its model holds it");
            assert!(zones[1].iter().all(|&cell| (at(cell).0 - added.x_m).hypot(at(cell).1 - added.z_m) <= reach), "{name}: zone 1 lies round its lake");
            println!("{name}: zones of {} and {} cells; the example's lake holds {area_m2} m²", zones[0].len(), zones[1].len());
        }
        // A lake moved is two lakes, each with its own zone.
        let mut moved = old.drawing.clone();
        moved.lakes[0].x_m += 300.0;
        let zones = lake_zones(&old.world, &old.model, &old.model, &old.drawing, &moved);
        assert_eq!(zones.len(), 2);
        assert!(!zones[0].is_empty());
    }
}
