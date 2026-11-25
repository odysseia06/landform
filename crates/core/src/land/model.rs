//! The mixed model: the land model a land keeps after an edit, made from the model it kept and the
//! edited drawing's model grown whole (steps P6, P7 and P8 of the editor tool's prototype,
//! `report2.md`, "The final mechanism").
//!
//! The two models' heights are mixed by the outline's weight on the model's cells (P6). Where two
//! valleys that do not line up meet in the band, the mix can close a hollow that neither model
//! holds water in: a pit. Each pit is given a way out, a trough cut along the way its water would
//! take, or filled to its level where that way runs into ground that must not change (P7). The
//! heights are rounded to `f32` after the mix and after each trough or fill, before the next
//! routing, and the model kept is routed once more from them (P8), so its water can be worked out
//! again from its `f32` heights alone (`model_water` gives the same receivers, ground drained,
//! depths and lakes' levels): a land's file keeps only those heights.

use crate::drawing::Drawing;
use crate::error::LandformError;
use crate::grid::Grid;
use crate::grow::{Grown, ModelRouting, GROW_CELL_M};
use crate::water::LAKE_REACH_RADII;
use crate::water_rules::WaterRules;

/// A band cell (weight above 0 and below 1) where the mixed model's standing water is deeper than
/// this, in metres, while the kept model and the new model each hold this much or less there, is a
/// pit: water the mix alone holds (P7). As deep as the land model takes a lake's water to be
/// (`lake_levels` in `grow/lake.rs`).
pub const PIT_DEPTH_M: f64 = 0.3;
/// A trough lowers the model cells whose centre lies less than this many metres from a cell of its
/// way (P7).
pub const TROUGH_REACH_M: f64 = 96.0;
/// A trough lies nowhere below a V whose sides rise this much in 1 from its way's bed, measured
/// from the cell of its way nearest each cell (P7): a bound on how deep it cuts, not on how steep
/// its sides are. Two neighbouring cells that both stand on the V lie at most `TROUGH_SIDE +
/// TROUGH_FALL √2` in 1 apart (their nearest cells of the way may be neighbours along it, whose
/// beds differ by the fall over a diagonal link); where the shoulder's fade binds instead, a side
/// is the ground's own slope plus up to the fade's, `1.5 c / (SHOULDER_M + SHOULDER_PER_CUT c)` in
/// 1 for a cut of `c` m (on the trial `shoulder`, cut 14.18 m deep, 882 sides between neighbouring
/// model cells, each counted once, are steeper than 0.3028 in 1 where they were no steeper than
/// 0.3 before, the steepest 0.683 in 1: the prototype's "stepped trough"). A V alone, with no bound
/// on its depth, cut 15 to 30 m into steep bowls (`report2.md`, "What failed first", 2).
pub const TROUGH_SIDE: f64 = 0.3;
/// A trough's bed falls this many metres per metre along its way from the pit (P7). Laid instead
/// in a straight line down to the ground of the way's last cell, which can lie far below the pit,
/// the bed cut a 17 m trench (`report2.md`, "What failed first", 1).
pub const TROUGH_FALL: f64 = 0.002;
/// The mixed model is routed, and its deepest pit opened or filled, at most this many times (P7);
/// a pit still left then stays, and is counted (`Mixed::pits_left`).
pub const TROUGH_PASSES: usize = 80;
/// A trough cut deeper than this, in metres, is said in the edit's notes (the design's amendment 11),
/// and so is a pit filled by more than this (`Trough::depth_m`, the most it raised a cell).
pub const TROUGH_SAID_M: f32 = 5.0;

/// A trough's bed starts this many metres below the pit's floor, so the pit's own lowest cell
/// drains along it.
const BED_BELOW_FLOOR_M: f64 = 0.01;
/// No cell within this many of a drawn lake's radii of its centre is a pit, nor is a trough cut for
/// one: a drawn lake's water lies there (`LAKE_REACH_RADII`, as far as `fill_lakes` looks for it).
const PIT_CLEAR_OF_LAKE_RADII: f64 = LAKE_REACH_RADII;
/// A pit filled is filled with its water body: the cells joined to it across their sides and
/// corners, inside the outline, that hold standing water whose surface lies within this many metres
/// of the pit's level.
const BODY_WITHIN_M: f64 = 0.05;
/// A trough's cut fades by a smooth step from its full depth at its way to nothing at
/// `SHOULDER_M` plus `SHOULDER_PER_CUT` times that depth, in metres (8 m, one model cell, and three
/// metres more for each metre cut): no deeper anywhere than the way itself needs. The land engine's
/// trench shoulder, full depth within 8 m and gone by 64 m, tried first, cut flat-floored,
/// steep-walled trenches up to 17 m deep beside a pit's way (`report2.md`, "What failed first", 2).
const SHOULDER_M: f64 = 8.0;
const SHOULDER_PER_CUT: f64 = 3.0;

/// The side of a model cell, in metres.
const CELL_M: f64 = GROW_CELL_M as f64;
/// The eight neighbours of a cell, as steps along the columns and the rows.
const AROUND: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// A pit the mix left in the band, and what was done to it (P7).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trough {
    /// World `(x, z)` of the pit's deepest model cell, in metres, where its way out begins.
    pub at: (f64, f64),
    /// For a trough cut, how deep: the most any model cell was lowered, in metres, which is the cut
    /// its way needed at its highest cell above its bed. For a pit filled, the most any cell was
    /// raised.
    pub depth_m: f32,
    /// Whether the pit was filled to its level instead of given a way out.
    pub filled: bool,
}

/// An edit's mixed model and what was done to give its band's pits a way out (`mixed_model`).
#[derive(Debug, Clone)]
pub struct Mixed {
    /// The land model a land keeps after the edit: `model_water` of its own heights, bit for bit.
    pub model: Grown,
    /// Each pit opened or filled, in the order it was done; at most `TROUGH_PASSES`.
    pub troughs: Vec<Trough>,
    /// The band's pits still standing when the `TROUGH_PASSES` passes ran out, by the same test as
    /// the passes' (`mixed_model`, **The pits**), as a count of model cells; 0 when the passes left
    /// none.
    pub pits_left: usize,
}

/// The mixed model of an edit (P6, P7, P8): `old`, the land model the land kept, and `new`, the
/// edited drawing's model grown whole, mixed by `model_weight` (`Outline::model_weight`, one value
/// a model cell from 0 to 1), with the band's pits given a way out or filled, and routed with
/// `new_drawing`'s land (its outlets, its links' lengths from `seed` and the water arriving where its
/// rivers enter, as `model_water` routes it). Returns the model, what was done to each pit in the
/// order it was done, and how many of the band's pits were left when the passes ran out (`Mixed`).
///
/// **The mix.** Each cell's height is `old`'s, bit for bit, where its weight is 0, `new`'s, bit for
/// bit, where it is 1, and between `old + w (new - old)`, worked in `f64` and rounded to `f32`.
///
/// **The pits.** The heights are routed (`model_water`'s routing). A pit is a band cell (weight
/// above 0 and below 1) whose centre lies more than `LAKE_REACH_RADII` (1.5) of its radii from the
/// centre of every lake of `new_drawing`, where the routed model's water is deeper than
/// `PIT_DEPTH_M` and `old`'s and `new`'s each `PIT_DEPTH_M` or less: a drawn lake is never a pit.
/// The deepest pit is taken (the first in cell order of several as deep). Its way out follows the
/// receivers from its cell, the bed falling from the pit's floor (its ground) less
/// `BED_BELOW_FLOOR_M` by `TROUGH_FALL` per metre along the links, to the first cell whose flooded
/// surface (ground plus water) lies below the bed there, or to an outlet.
///
/// **A trough.** Every cell of weight above 0 whose centre lies less than `TROUGH_REACH_M` from a
/// cell of the way (the pit's cell first, then the way's, the first of several as near), but for the
/// outlets, which the model holds at 0, is lowered by `min(max(0, h - (b + TROUGH_SIDE d)), c (1 -
/// s(d / (SHOULDER_M + SHOULDER_PER_CUT c))))`, `s(t) = t² (3 - 2 t)` with `t` held to 0..1: `d`
/// the distance to its nearest cell of the way, `b` that cell's bed and `c = max(0, h_way - b)` the
/// cut that cell needs, every height read before the trough. So the trough is no deeper anywhere
/// than its way and lies nowhere below a V whose sides rise `TROUGH_SIDE` in 1: a bound on its
/// depth, not on its sides' steepness (`TROUGH_SIDE` says how steep they may be).
///
/// **A fill.** Where the way meets a cell of weight 0 before it ends, where it reaches no cell
/// beyond the pit, or where a trough was already cut from the same cell, the pit's water body is
/// filled instead: the cells of weight above 0 joined to the pit across their sides and corners
/// whose standing water's surface lies within `BODY_WITHIN_M` of the pit's level are raised to that
/// level, rounded to `f32`, where they lie below it.
///
/// Each trough or fill is followed by a routing and the next deepest pit, up to `TROUGH_PASSES`
/// times; should the passes run out, the model is routed once more after the last, and a pit left
/// then stays, counted in `Mixed::pits_left` by the same test. Every height is rounded to `f32`
/// after the mix and after each trough and fill, before the next routing, so the model returned is
/// `model_water(new_drawing, water, seed, &its heights)`, bit for bit. A cell of weight 0 keeps
/// `old`'s height bit for bit throughout. The rivers and notes are those `model_water` gives (the
/// rivers `new_drawing` lays, and the notes laying them writes); the lakes are `new_drawing`'s that hold water deeper than 0.3 m within
/// their radius in the mixed model. No note is written for a lake the mixed model does not hold:
/// the fine steps say it ("Lake N holds no water; it was left out.", `fill_lakes`), as for a world
/// grown whole.
///
/// `new_drawing` and `water` are checked first, as `model_water` checks them; then the models'
/// heights and depths, which must lie on `new_drawing`'s model grid, and the weight, one value from
/// 0 to 1 a model cell: each failure is `BadConfig` naming what is wrong. Sequential and
/// deterministic. It reads no cancel flag: at 2,048 m in a release build it took about 22 ms with
/// one routing and about 8 ms more for each further pass (the example's edits `g` and `h`), so 80
/// passes would take about 0.7 s.
pub fn mixed_model(
    old: &Grown,
    new: &Grown,
    new_drawing: &Drawing,
    water: &WaterRules,
    seed: u64,
    model_weight: &[f32],
) -> Result<Mixed, LandformError> {
    let mut routing = ModelRouting::new(new_drawing, water, seed)?;
    let side = routing.side();
    let grids = [
        ("kept model's heights", &old.heights),
        ("kept model's water's depth", &old.water_depth_m),
        ("new model's heights", &new.heights),
        ("new model's water's depth", &new.water_depth_m),
    ];
    for (name, grid) in grids {
        if !routing.on_grid(grid) {
            return Err(LandformError::BadConfig(format!(
                "the {name} must be {side} x {side} cells of {GROW_CELL_M} m, the edited drawing's model grid, not {} x {} cells of {} m",
                grid.width, grid.height, grid.cell_size
            )));
        }
    }
    if model_weight.len() != side * side || !model_weight.iter().all(|w| (0.0..=1.0).contains(w)) {
        return Err(LandformError::BadConfig(format!(
            "the model's weight must hold one value from 0 to 1 for each of the model's {} cells",
            side * side
        )));
    }

    let mut heights = mix(old, new, model_weight);
    let mut troughs = Vec::new();
    // The pits a trough was cut from: one that comes back is filled instead.
    let mut opened: Vec<usize> = Vec::new();
    for _ in 0..TROUGH_PASSES {
        let model = routing.water(&heights)?;
        let Some(pit) = deepest(&model, &band_pits(&model, old, new, new_drawing, model_weight, &routing)) else {
            return Ok(Mixed { model, troughs, pits_left: 0 });
        };
        let (depth_m, filled) = match way_out(&model, model_weight, pit) {
            Some(way) if !opened.contains(&pit) => {
                opened.push(pit);
                (cut_trough(&mut heights.data, &way, model_weight, &routing), false)
            }
            _ => (fill(&mut heights.data, &model, model_weight, pit), true),
        };
        troughs.push(Trough { at: routing.cell_m(pit), depth_m, filled });
    }
    let model = routing.water(&heights)?;
    let pits_left = band_pits(&model, old, new, new_drawing, model_weight, &routing).len();
    Ok(Mixed { model, troughs, pits_left })
}

/// The heights of `old` and `new` mixed by `weight`, one value a model cell (P6): `old`'s bit for
/// bit where the weight is 0, `new`'s bit for bit where it is 1, and `old + w (new - old)` between,
/// worked in `f64` and rounded to `f32`.
fn mix(old: &Grown, new: &Grown, weight: &[f32]) -> Grid<f32> {
    let data = (old.heights.data.iter().zip(&new.heights.data).zip(weight)).map(|((&old, &new), &w)| mixed(old, new, w)).collect();
    Grid { width: old.heights.width, height: old.heights.height, cell_size: old.heights.cell_size, data }
}

/// One value of an edit's mix by weight `w`: `old`, bit for bit, at weight 0, `new`, bit for bit,
/// at weight 1, and between `old + w (new - old)`, worked in `f64` and rounded to `f32`. The mixed
/// model's heights are mixed so (P6), and the joined ground's heights and layers (P10,
/// `join_ground`).
pub(crate) fn mixed(old: f32, new: f32, w: f32) -> f32 {
    if w == 0.0 {
        old
    } else if w == 1.0 {
        new
    } else {
        (f64::from(old) + f64::from(w) * (f64::from(new) - f64::from(old))) as f32
    }
}

/// The pits of `model`, the mix of `old` and `new` by `weight` routed, in cell order: the cells of
/// weight above 0 and below 1, more than `PIT_CLEAR_OF_LAKE_RADII` of its radii from every lake of
/// `drawing`, where `model`'s water is deeper than `PIT_DEPTH_M` and `old`'s and `new`'s each
/// `PIT_DEPTH_M` or less.
fn band_pits(model: &Grown, old: &Grown, new: &Grown, drawing: &Drawing, weight: &[f32], routing: &ModelRouting) -> Vec<usize> {
    let deeper = |grown: &Grown, cell: usize| f64::from(grown.water_depth_m.data[cell]) > PIT_DEPTH_M;
    (0..weight.len())
        .filter(|&cell| weight[cell] > 0.0 && weight[cell] < 1.0)
        .filter(|&cell| deeper(model, cell) && !deeper(old, cell) && !deeper(new, cell))
        .filter(|&cell| {
            let (x, z) = routing.cell_m(cell);
            drawing.lakes.iter().all(|lake| (x - lake.x_m).hypot(z - lake.z_m) > PIT_CLEAR_OF_LAKE_RADII * f64::from(lake.radius_m))
        })
        .collect()
}

/// Of `pits`, cells of `model` in order, the one whose water is deepest; the first of several as
/// deep. `None` when there are none.
fn deepest(model: &Grown, pits: &[usize]) -> Option<usize> {
    let depth = |cell: usize| model.water_depth_m.data[cell];
    pits.iter().copied().reduce(|best, cell| if depth(cell) > depth(best) { cell } else { best })
}

/// The way out of `pit`, a cell of `model`, with each cell's bed: the pit itself, its bed its
/// floor less `BED_BELOW_FLOOR_M`, then the cells its receivers lead to, each bed `TROUGH_FALL` per
/// metre lower along the links, up to and with the first whose flooded surface (ground plus water)
/// lies below its bed, or an outlet. `None` when a cell of weight 0 comes first, or no cell lies
/// beyond the pit.
fn way_out(model: &Grown, weight: &[f32], pit: usize) -> Option<Vec<(usize, f64)>> {
    let side = model.heights.width;
    let surface = |cell: usize| f64::from(model.heights.data[cell]) + f64::from(model.water_depth_m.data[cell]);
    let floor = f64::from(model.heights.data[pit]);
    let bed = |along_m: f64| floor - BED_BELOW_FLOOR_M - TROUGH_FALL * along_m;
    let mut way = vec![(pit, bed(0.0))];
    let (mut cell, mut along_m) = (pit, 0.0);
    loop {
        let next = model.receivers[cell] as usize;
        if next == cell {
            break;
        }
        along_m += link_m(side, cell, next);
        cell = next;
        if weight[cell] == 0.0 {
            return None;
        }
        way.push((cell, bed(along_m)));
        if surface(cell) < bed(along_m) {
            break;
        }
    }
    (way.len() > 1).then_some(way)
}

/// Cuts the trough along `way` (cells of the model's grid with their beds, `way_out`) into
/// `heights`, as `mixed_model` says, and gives its depth: the most any cell was lowered, in metres.
fn cut_trough(heights: &mut [f32], way: &[(usize, f64)], weight: &[f32], routing: &ModelRouting) -> f32 {
    let side = routing.side();
    let reach = (TROUGH_REACH_M / CELL_M).floor() as usize;
    let cuts: Vec<f64> = way.iter().map(|&(cell, bed)| (f64::from(heights[cell]) - bed).max(0.0)).collect();
    // The box of cells within the reach of the way, clipped to the grid.
    let (columns, rows) = (way.iter().map(|&(cell, _)| cell % side), way.iter().map(|&(cell, _)| cell / side));
    let (a0, a1) = (columns.clone().min().unwrap_or(0).saturating_sub(reach), (columns.max().unwrap_or(0) + reach).min(side - 1));
    let (b0, b1) = (rows.clone().min().unwrap_or(0).saturating_sub(reach), (rows.max().unwrap_or(0) + reach).min(side - 1));
    let width = a1 - a0 + 1;
    // Each cell's nearest cell of the way, as its distance and its place in the way.
    let mut nearest: Vec<Option<(f64, usize)>> = vec![None; width * (b1 - b0 + 1)];
    for (place, &(cell, _)) in way.iter().enumerate() {
        let (column, row) = (cell % side, cell / side);
        for b in row.saturating_sub(reach)..=(row + reach).min(side - 1) {
            for a in column.saturating_sub(reach)..=(column + reach).min(side - 1) {
                let near = b * side + a;
                let d_m = CELL_M * (a.abs_diff(column) as f64).hypot(b.abs_diff(row) as f64);
                if d_m >= TROUGH_REACH_M || weight[near] == 0.0 || routing.is_outlet(near) {
                    continue;
                }
                let found = &mut nearest[(b - b0) * width + (a - a0)];
                if found.is_none_or(|(nearest_m, _)| d_m < nearest_m) {
                    *found = Some((d_m, place));
                }
            }
        }
    }
    let mut depth_m = 0.0f32;
    for (index, found) in nearest.iter().enumerate() {
        let Some((d_m, place)) = *found else { continue };
        let cell = (b0 + index / width) * side + a0 + index % width;
        let (bed, cut) = (way[place].1, cuts[place]);
        let t = (d_m / (SHOULDER_M + SHOULDER_PER_CUT * cut)).clamp(0.0, 1.0);
        let most = cut * (1.0 - t * t * (3.0 - 2.0 * t));
        let h = f64::from(heights[cell]);
        let laid = (h - (h - (bed + TROUGH_SIDE * d_m)).max(0.0).min(most)) as f32;
        if laid < heights[cell] {
            depth_m = depth_m.max(heights[cell] - laid);
            heights[cell] = laid;
        }
    }
    depth_m
}

/// Fills `pit`'s water body in `heights`, the heights `model` was routed from, as `mixed_model`
/// says, and gives the most it raised a cell, in metres.
fn fill(heights: &mut [f32], model: &Grown, weight: &[f32], pit: usize) -> f32 {
    let side = model.heights.width;
    let surface = |cell: usize| f64::from(model.heights.data[cell]) + f64::from(model.water_depth_m.data[cell]);
    let level = surface(pit);
    let holds = |cell: usize| weight[cell] > 0.0 && model.water_depth_m.data[cell] > 0.0 && (surface(cell) - level).abs() < BODY_WITHIN_M;
    let mut body = vec![false; side * side];
    body[pit] = true;
    let mut waiting = vec![pit];
    while let Some(cell) = waiting.pop() {
        for next in around(side, cell) {
            if !body[next] && holds(next) {
                body[next] = true;
                waiting.push(next);
            }
        }
    }
    let level = level as f32;
    let mut depth_m = 0.0f32;
    for (height, &in_body) in heights.iter_mut().zip(&body) {
        if in_body && *height < level {
            depth_m = depth_m.max(level - *height);
            *height = level;
        }
    }
    depth_m
}

/// The length of the link between `from` and `to`, neighbouring cells of a grid `side` cells a
/// side, in metres.
fn link_m(side: usize, from: usize, to: usize) -> f64 {
    CELL_M * ((from % side).abs_diff(to % side) as f64).hypot((from / side).abs_diff(to / side) as f64)
}

/// The cells around `cell` on a grid `side` cells a side, across its sides and corners.
fn around(side: usize, cell: usize) -> impl Iterator<Item = usize> {
    let (column, row) = (cell % side, cell / side);
    AROUND.iter().filter_map(move |&(across, down)| {
        let column = column.checked_add_signed(across).filter(|&next| next < side)?;
        let row = row.checked_add_signed(down).filter(|&next| next < side)?;
        Some(row * side + column)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Country;
    use crate::drawing::Lake;
    use crate::grow::{grow, model_side, model_water, start_of, Start};
    use crate::land::outline::tests::{old_land, EXAMPLE_SEED};
    use crate::land::outline::{footprint, lake_zones, outline, Marks, REACH_MIN_M};
    use crate::trial::{example_edit, standard_edit, trial_drawing};
    use crate::world::{finish_drawn, Frame, GenConfig, World};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// The hand-made land of the bowl's tests: hill country this many metres across, with no
    /// river (every cell on the grid's edge an outlet), routed from this seed.
    const HAND_M: u32 = 512;
    const HAND_SEED: u64 = 1;
    /// The bowl's middle, a model cell as (column, row): world (0, 0).
    const BOWL: (usize, usize) = (64, 64);
    /// Its floor reaches this many cells from its middle, its wall this many.
    const FLOOR_CELLS: f64 = 3.0;
    const WALL_CELLS: f64 = 5.0;
    /// The notch in its wall, its lowest way out (two cells, on the row of its middle), stands this
    /// high, in metres; the floor this many metres below it, the rest of the wall this many above.
    const NOTCH_M: f64 = 50.0;
    const BOWL_DEPTH_M: f64 = 2.0;
    const WALL_ABOVE_M: f64 = 1.0;
    /// Beyond the wall the ground falls toward the notch's side by this many metres a cell (0.05 in
    /// 1), from this far below the notch beside it.
    const PLANE_FALL_M: f64 = 0.4;
    const PLANE_BELOW_NOTCH_M: f64 = 0.1;
    /// The band: the weight is 0 up to this column, rises by a smooth step and is 1 from this one.
    const BAND_FROM: usize = 58;
    const BAND_TO: usize = 90;
    /// The ground outside the band that one model or the other holds higher, by this many metres,
    /// where the mix takes the other's.
    const ASIDE_M: f32 = 3.0;
    /// A lake drawn on the bowl: its floor lies within 1.5 of these radii of its middle.
    const BOWL_LAKE_M: f32 = 20.0;
    /// The water a model is made to hold on the bowl's floor, in metres, more than `PIT_DEPTH_M`.
    const HELD_ON_FLOOR_M: f32 = 1.0;
    /// The cells round a trough's way that it may lower, as a count of cells along a row.
    const REACH_CELLS: f64 = TROUGH_REACH_M / CELL_M;
    /// The plane of the test where the passes run out: it stands this high at the grid's west edge,
    /// in metres, and falls east by this many metres a cell (0.05 in 1).
    const PLANE_WEST_M: f64 = 100.0;
    const PLANE_EAST_FALL_M: f64 = 0.4;
    /// Its one-cell holes, this many metres deep, in the band on these columns and on every
    /// `HOLE_ROW_STEP`th row from `HOLE_ROW_FROM`, `HOLE_ROWS` rows: 120 pits, more than
    /// `TROUGH_PASSES`, far enough apart that no trough reaches another's hole.
    const HOLE_DEPTH_M: f64 = 1.0;
    const HOLE_COLUMNS: [usize; 4] = [64, 68, 72, 76];
    const HOLE_ROW_FROM: usize = 10;
    const HOLE_ROW_STEP: usize = 3;
    const HOLE_ROWS: usize = 30;

    /// Which way the bowl's notch faces: toward the band's inside, where the weight rises (east), or
    /// toward its outside (west), where the ground must not change.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Notch {
        Inward,
        Outward,
    }

    /// Plain hill country `HAND_M` across with `lakes`.
    fn plain(lakes: Vec<Lake>) -> Drawing {
        Drawing { size_m: HAND_M, high_ground: None, rivers: Vec::new(), lakes, country: Country::hills() }
    }

    /// The column and row of model cell `cell` from the bowl's middle.
    fn from_bowl(cell: usize) -> (f64, f64) {
        let side = model_side(HAND_M);
        ((cell % side) as f64 - BOWL.0 as f64, (cell / side) as f64 - BOWL.1 as f64)
    }

    /// Whether `cell` is on the bowl's floor.
    fn on_floor(cell: usize) -> bool {
        let (across, down) = from_bowl(cell);
        across.hypot(down) <= FLOOR_CELLS
    }

    /// The hand-made land's heights: a bowl `BOWL_DEPTH_M` deep below the notch in its wall, which
    /// faces `notch`, on ground falling toward that side.
    fn bowl(notch: Notch) -> Grid<f32> {
        let side = model_side(HAND_M);
        let toward = if notch == Notch::Inward { 1.0 } else { -1.0 };
        let data = (0..side * side)
            .map(|cell| {
                let (across, down) = from_bowl(cell);
                let (from_middle, toward_notch) = (across.hypot(down), toward * across);
                let height = if from_middle <= FLOOR_CELLS {
                    NOTCH_M - BOWL_DEPTH_M
                } else if from_middle <= WALL_CELLS {
                    if down == 0.0 && toward_notch > 0.0 {
                        NOTCH_M
                    } else {
                        NOTCH_M + WALL_ABOVE_M
                    }
                } else {
                    NOTCH_M - PLANE_BELOW_NOTCH_M - PLANE_FALL_M * (toward_notch - WALL_CELLS)
                };
                height as f32
            })
            .collect();
        Grid { width: side, height: side, cell_size: GROW_CELL_M, data }
    }

    /// The band's weight, one value a model cell: 0 up to `BAND_FROM`, 1 from `BAND_TO`, a smooth
    /// step between, by column.
    fn band() -> Vec<f32> {
        let side = model_side(HAND_M);
        (0..side * side)
            .map(|cell| {
                let t = (((cell % side) as f64 - BAND_FROM as f64) / (BAND_TO - BAND_FROM) as f64).clamp(0.0, 1.0);
                (t * t * (3.0 - 2.0 * t)) as f32
            })
            .collect()
    }

    /// A land model made by hand on `heights`, holding no standing water: the kept and the new model
    /// of the bowl's tests hold none at the bowl, as two models whose valleys do not line up hold none
    /// where their mix does. Its other parts are not read by `mixed_model`.
    fn by_hand(heights: Grid<f32>) -> Grown {
        let cells = heights.data.len();
        let none = Grid { data: vec![0.0; cells], ..heights.clone() };
        Grown {
            drained_m2: none.clone(),
            water_depth_m: none,
            receivers: (0..cells as u32).collect(),
            rivers: Vec::new(),
            lakes: Vec::new(),
            notes: Vec::new(),
            notes_about: Vec::new(),
            heights,
        }
    }

    /// The kept and the new model of the bowl's tests: `heights` where the mix takes each, the kept
    /// one `ASIDE_M` higher where the weight is 1 and the new one where it is 0, so a mix that took
    /// the wrong model there would show it.
    fn models(heights: &Grid<f32>) -> (Grown, Grown) {
        let side = heights.width;
        let raised = |beside: fn(usize) -> bool| {
            let mut raised = heights.clone();
            for (cell, height) in raised.data.iter_mut().enumerate() {
                if beside(cell % side) {
                    *height += ASIDE_M;
                }
            }
            by_hand(raised)
        };
        (raised(|column| column >= BAND_TO), raised(|column| column <= BAND_FROM))
    }

    fn bits(values: &[f32]) -> Vec<u32> {
        values.iter().map(|value| value.to_bits()).collect()
    }

    /// The band's pits of `model`, the mix of `old` and `new` by `weight` for `drawing` routed: how
    /// many, and the deepest's water.
    fn pits_in(model: &Grown, old: &Grown, new: &Grown, drawing: &Drawing, weight: &[f32], seed: u64) -> (usize, f32) {
        let routing = ModelRouting::new(drawing, &WaterRules::default(), seed).expect("a valid drawing");
        let pits = band_pits(model, old, new, drawing, weight, &routing);
        (pits.len(), pits.iter().map(|&cell| model.water_depth_m.data[cell]).fold(0.0, f32::max))
    }

    /// The model `mixed_model` returned is the one `model_water` works out from its heights, bit for
    /// bit: its receivers, ground drained, water's depth and lakes' levels, its rivers and notes
    /// with what they are about.
    fn assert_worked_out_again(mixed: &Grown, drawing: &Drawing, seed: u64, case: &str) {
        let again = model_water(drawing, &WaterRules::default(), seed, &mixed.heights).expect("the mixed model's heights");
        assert!(again.receivers == mixed.receivers, "{case}: the receivers");
        assert!(bits(&again.drained_m2.data) == bits(&mixed.drained_m2.data), "{case}: the ground drained");
        assert!(bits(&again.water_depth_m.data) == bits(&mixed.water_depth_m.data), "{case}: the water's depth");
        assert_eq!((&again.lakes, &again.rivers, &again.notes, &again.notes_about), (&mixed.lakes, &mixed.rivers, &mixed.notes, &mixed.notes_about), "{case}: the lakes, rivers and notes");
    }

    /// Every cell of weight 0 keeps `old`'s height bit for bit.
    fn assert_kept_where_weight_0(mixed: &Grown, old: &Grown, weight: &[f32], case: &str) {
        let changed = (0..weight.len()).filter(|&cell| weight[cell] == 0.0 && mixed.heights.data[cell].to_bits() != old.heights.data[cell].to_bits()).count();
        assert_eq!(changed, 0, "{case}: cells of weight 0 changed");
        assert!(weight.contains(&0.0), "{case}: no cell has weight 0");
    }

    /// P7's trough along `way` (cells with their beds) cut into `heights`, restated here apart from
    /// `cut_trough`: every cell of weight above 0, not an outlet, whose centre lies less than
    /// `TROUGH_REACH_M` from a cell of the way is lowered by `min(max(0, h - (b + TROUGH_SIDE d)), c
    /// (1 - s(d / (SHOULDER_M + SHOULDER_PER_CUT c))))`, `d` its distance to the first of the way's
    /// nearest cells, `b` that cell's bed and `c` its cut above its bed (never below 0), the result
    /// rounded to `f32` and kept only where it lies lower. Returns the heights and how many cells lie
    /// within the reach.
    fn trough_by_p7(heights: &Grid<f32>, way: &[(usize, f64)], weight: &[f32], routing: &ModelRouting) -> (Vec<f32>, usize) {
        let side = heights.width;
        let mut cut = heights.data.clone();
        let mut in_reach = 0;
        for (cell, laid) in cut.iter_mut().enumerate() {
            if weight[cell] == 0.0 || routing.is_outlet(cell) {
                continue;
            }
            let apart_m = |at: usize| CELL_M * ((at % side) as f64 - (cell % side) as f64).hypot((at / side) as f64 - (cell / side) as f64);
            let (d_m, &(at, bed)) = way.iter().map(|step| (apart_m(step.0), step)).reduce(|best, next| if next.0 < best.0 { next } else { best }).expect("a way");
            if d_m >= TROUGH_REACH_M {
                continue;
            }
            in_reach += 1;
            let h = f64::from(heights.data[cell]);
            let c = (f64::from(heights.data[at]) - bed).max(0.0);
            let t = (d_m / (SHOULDER_M + SHOULDER_PER_CUT * c)).clamp(0.0, 1.0);
            let lower = (h - (bed + TROUGH_SIDE * d_m)).max(0.0).min(c * (1.0 - t * t * (3.0 - 2.0 * t)));
            *laid = laid.min((h - lower) as f32);
        }
        (cut, in_reach)
    }

    /// The deepest cell of the bowl's floor in `routed`, the first of several as deep.
    fn deepest_on_floor(routed: &Grown) -> usize {
        let depth = |cell: usize| routed.water_depth_m.data[cell];
        (0..routed.water_depth_m.data.len()).filter(|&cell| on_floor(cell)).reduce(|best, cell| if depth(cell) > depth(best) { cell } else { best }).expect("a floor")
    }

    #[test]
    fn the_mix_takes_the_old_at_weight_0_and_the_new_at_weight_1_bit_for_bit() {
        let grid = |data: Vec<f32>| Grid { width: 2, height: 2, cell_size: GROW_CELL_M, data };
        let old = by_hand(grid(vec![-0.0, 7.0, 1.0, 1000.5]));
        let new = by_hand(grid(vec![5.0, 1e-7, 3.0, 2.0]));
        let mixed = mix(&old, &new, &[0.0, 1.0, 0.5, 0.25]);
        let expected = [-0.0, 1e-7, 2.0, (1000.5 + 0.25 * (2.0 - 1000.5)) as f32];
        assert_eq!(bits(&mixed.data), bits(&expected));
        assert_eq!((mixed.width, mixed.height, mixed.cell_size), (2, 2, GROW_CELL_M));
    }

    #[test]
    fn weight_0_everywhere_gives_the_old_heights_and_weight_1_the_new() {
        let water = WaterRules::default();
        let drawing = Drawing::example_valley(512);
        let edited = example_edit(&drawing, "a").expect("an edit of the example");
        let old = grow(&drawing, &water, HAND_SEED, &AtomicBool::new(false)).expect("the example grows");
        let new = grow(&edited, &water, HAND_SEED, &AtomicBool::new(false)).expect("the edit grows");
        let cells = old.heights.data.len();
        assert!(bits(&old.heights.data) != bits(&new.heights.data), "the edit changes the model");

        let Mixed { model: kept, troughs, pits_left } = mixed_model(&old, &new, &edited, &water, HAND_SEED, &vec![0.0; cells]).expect("a mixed model");
        assert_eq!(pits_left, 0, "weight 0");
        assert!(bits(&kept.heights.data) == bits(&old.heights.data), "weight 0: the old model's heights");
        assert!(troughs.is_empty(), "weight 0: {troughs:?}");
        assert_worked_out_again(&kept, &edited, HAND_SEED, "weight 0");

        let Mixed { model: taken, troughs, pits_left } = mixed_model(&old, &new, &edited, &water, HAND_SEED, &vec![1.0; cells]).expect("a mixed model");
        assert_eq!(pits_left, 0, "weight 1");
        assert!(bits(&taken.heights.data) == bits(&new.heights.data), "weight 1: the new model's heights");
        assert!(troughs.is_empty(), "weight 1: {troughs:?}");
        assert_eq!((&taken.rivers, &taken.notes, &taken.notes_about), (&new.rivers, &new.notes, &new.notes_about), "weight 1: the edited drawing's rivers and notes");
        assert_worked_out_again(&taken, &edited, HAND_SEED, "weight 1");
    }

    /// The bowl, its notch facing the band's inside, is a pit: the kept and the new model hold no
    /// water there and the mix holds its 2 m. It is opened by one trough along its way out, the
    /// bowl's floor to the notch and on down the ground beyond until the ground lies below the bed.
    #[test]
    fn a_pit_in_the_band_whose_way_leads_inward_is_opened_by_one_trough() {
        let (water, drawing, weight) = (WaterRules::default(), plain(Vec::new()), band());
        let heights = bowl(Notch::Inward);
        let (old, new) = models(&heights);
        let side = heights.width;
        let Mixed { model: mixed, troughs, pits_left } = mixed_model(&old, &new, &drawing, &water, HAND_SEED, &weight).expect("a mixed model");
        assert_eq!(pits_left, 0);

        // The way, worked out here from the bowl routed: from the deepest cell of the floor along
        // the receivers, the bed falling from the floor less 1 cm, to the first cell below its bed.
        let routed = model_water(&drawing, &water, HAND_SEED, &heights).expect("the bowl's heights");
        let depth = |cell: usize| routed.water_depth_m.data[cell];
        let start = deepest_on_floor(&routed);
        assert!(f64::from(depth(start)) > BOWL_DEPTH_M && f64::from(depth(start)) < BOWL_DEPTH_M + 0.01, "the bowl holds {} m", depth(start));
        let floor = f64::from(heights.data[start]);
        let mut way = vec![(start, floor - BED_BELOW_FLOOR_M)];
        let (mut cell, mut along_m) = (start, 0.0);
        while routed.receivers[cell] as usize != cell {
            let next = routed.receivers[cell] as usize;
            along_m += link_m(side, cell, next);
            cell = next;
            let bed = floor - BED_BELOW_FLOOR_M - TROUGH_FALL * along_m;
            way.push((cell, bed));
            if f64::from(heights.data[cell]) + f64::from(depth(cell)) < bed {
                break;
            }
        }
        // The way's highest cell above its bed is in the notch: its cut is the bowl's depth, 1 cm,
        // and the bed's fall from the floor to there.
        let cut = |&(cell, bed): &(usize, f64)| f64::from(heights.data[cell]) - bed;
        let highest = *way.iter().max_by(|a, b| cut(a).total_cmp(&cut(b))).expect("a way");
        assert_eq!(f64::from(heights.data[highest.0]), NOTCH_M, "the way's highest cell above its bed is not in the notch");

        println!("the bowl's trough: {troughs:?}; its way {} cells, the cut at the notch {:.4} m", way.len(), cut(&highest));
        assert_eq!(troughs.len(), 1, "{troughs:?}");
        let trough = troughs[0];
        assert!(!trough.filled);
        let routing = ModelRouting::new(&drawing, &water, HAND_SEED).expect("a valid drawing");
        assert_eq!(trough.at, routing.cell_m(start));
        assert!((f64::from(trough.depth_m) - cut(&highest)).abs() < 1e-4, "{} m deep, the cut at the notch {} m", trough.depth_m, cut(&highest));
        assert!(f64::from(trough.depth_m) > BOWL_DEPTH_M + BED_BELOW_FLOOR_M);

        let place = |cell: usize| ((cell % side) as f64, (cell / side) as f64);
        let apart_m = |a: usize, b: usize| CELL_M * (place(a).0 - place(b).0).hypot(place(a).1 - place(b).1);
        let lowered: Vec<usize> = (0..side * side).filter(|&cell| mixed.heights.data[cell] < heights.data[cell]).collect();
        assert!(lowered.len() > way.len(), "{} cells lowered", lowered.len());
        assert!((0..side * side).all(|cell| mixed.heights.data[cell] <= heights.data[cell]), "a trough raises nothing");
        let mut steepest = 0.0f64;
        for &cell in &lowered {
            // Nowhere below a V whose sides rise 0.3 in 1 from the bed along the way, and within
            // the trough's reach of it.
            let v = way.iter().map(|&(at, bed)| bed + TROUGH_SIDE * apart_m(at, cell)).fold(f64::INFINITY, f64::min);
            assert!(f64::from(mixed.heights.data[cell]) >= v - 1e-4, "cell {cell} lies at {} m, below the V's {v} m", mixed.heights.data[cell]);
            assert!(weight[cell] > 0.0 && way.iter().any(|&(at, _)| apart_m(at, cell) < TROUGH_REACH_M), "cell {cell}");
            // P7 bounds the trough's depth, not its sides (`TROUGH_SIDE`): two neighbours that both
            // stand on the V lie at most `TROUGH_SIDE + TROUGH_FALL √2` in 1 apart, their nearest
            // cells of the way being neighbours along it whose beds differ by the fall over a
            // diagonal link; where the shoulder's fade binds, a side is the ground's own slope plus
            // up to 1.5 c / (8 + 3 c) in 1, steeper on deep cuts (the trial `shoulder`: 0.683 in 1).
            // The bowl's cut of 2.12 m leaves its steepest side, among the sides no steeper than
            // 0.3 before, at the V's bound: measured, exactly 0.3 + 0.002 √2.
            for next in around(side, cell) {
                let slope = |grid: &Grid<f32>| (f64::from(grid.data[cell]) - f64::from(grid.data[next])).abs() / apart_m(cell, next);
                if slope(&heights) <= TROUGH_SIDE {
                    steepest = steepest.max(slope(&mixed.heights));
                }
            }
        }
        println!("{} cells lowered; the steepest side {steepest:.6} in 1", lowered.len());
        assert!(steepest <= TROUGH_SIDE + TROUGH_FALL * std::f64::consts::SQRT_2 + 1e-5, "a side of {steepest} in 1");
        // The trough's shape, cell for cell: P7's formula worked out here from the way, on every
        // cell, equals the heights `mixed_model` left, bit for bit.
        let (expected, in_reach) = trough_by_p7(&heights, &way, &weight, &routing);
        let differ: Vec<usize> = (0..side * side).filter(|&cell| expected[cell].to_bits() != mixed.heights.data[cell].to_bits()).collect();
        println!("P7's trough worked out again: {in_reach} cells in reach, {} differ", differ.len());
        assert!(differ.is_empty(), "{} cells differ from P7's trough, the first {:?}", differ.len(), differ.first().map(|&cell| (cell, expected[cell], mixed.heights.data[cell])));
        assert!(in_reach > lowered.len(), "{in_reach} cells in reach, {} lowered", lowered.len());
        assert_kept_where_weight_0(&mixed, &old, &weight, "the bowl");
        let weight_0_in_reach = (0..side * side).filter(|&cell| weight[cell] == 0.0 && way.iter().any(|&(at, _)| apart_m(at, cell) < TROUGH_REACH_M)).count();
        assert!(weight_0_in_reach > 0, "no cell of weight 0 lies within the trough's reach, so the test shows nothing");
        assert!(way.iter().all(|&(cell, _)| (place(cell).0 - place(start).0).abs() < REACH_CELLS + WALL_CELLS * 2.0));
        // The mix took the right model on each side of the band.
        assert!((0..side * side).filter(|&cell| weight[cell] == 1.0).all(|cell| mixed.heights.data[cell].to_bits() == new.heights.data[cell].to_bits()));
        assert_eq!(pits_in(&mixed, &old, &new, &drawing, &weight, HAND_SEED).0, 0, "a pit is left");
        assert_worked_out_again(&mixed, &drawing, HAND_SEED, "the bowl");
    }

    /// The same bowl with its notch facing out of the band: its way out meets a cell of weight 0
    /// beyond the notch, so its water is filled to its level instead, and nothing else changes.
    #[test]
    fn a_pit_whose_way_out_meets_ground_that_must_not_change_is_filled_to_its_level() {
        let (water, drawing, weight) = (WaterRules::default(), plain(Vec::new()), band());
        let heights = bowl(Notch::Outward);
        let (old, new) = models(&heights);
        let side = heights.width;
        let Mixed { model: mixed, troughs, pits_left } = mixed_model(&old, &new, &drawing, &water, HAND_SEED, &weight).expect("a mixed model");
        assert_eq!(pits_left, 0);
        let routed = model_water(&drawing, &water, HAND_SEED, &heights).expect("the bowl's heights");
        let start = deepest_on_floor(&routed);
        let level = (f64::from(heights.data[start]) + f64::from(routed.water_depth_m.data[start])) as f32;
        println!("the bowl filled: {troughs:?}, to {level} m");
        assert_eq!(troughs.len(), 1, "{troughs:?}");
        assert!(troughs[0].filled);
        assert_eq!(troughs[0].depth_m, level - (NOTCH_M - BOWL_DEPTH_M) as f32);
        assert!(f64::from(troughs[0].depth_m) >= BOWL_DEPTH_M);
        for cell in 0..side * side {
            let expected = if on_floor(cell) { level } else { heights.data[cell] };
            assert_eq!(mixed.heights.data[cell].to_bits(), expected.to_bits(), "cell {cell}");
        }
        assert_kept_where_weight_0(&mixed, &old, &weight, "the bowl filled");
        assert_eq!(pits_in(&mixed, &old, &new, &drawing, &weight, HAND_SEED).0, 0, "a pit is left");
        assert_worked_out_again(&mixed, &drawing, HAND_SEED, "the bowl filled");
    }

    /// A plane falling east across the band with 120 one-cell holes in the band, each a pit, more
    /// than `TROUGH_PASSES`: the passes open the 80 first, one trough each, and run out; the 40
    /// pits left are counted, every cell of weight 0 is kept, and the model is still the one
    /// `model_water` works out from its heights.
    #[test]
    fn the_pits_left_when_the_passes_run_out_are_counted() {
        let (water, drawing, weight) = (WaterRules::default(), plain(Vec::new()), band());
        let side = model_side(HAND_M);
        let holes: Vec<usize> = (0..HOLE_ROWS).flat_map(|row| HOLE_COLUMNS.map(|column| (HOLE_ROW_FROM + HOLE_ROW_STEP * row) * side + column)).collect();
        let data = (0..side * side)
            .map(|cell| {
                let plane = PLANE_WEST_M - PLANE_EAST_FALL_M * (cell % side) as f64;
                (if holes.contains(&cell) { plane - HOLE_DEPTH_M } else { plane }) as f32
            })
            .collect();
        let heights = Grid { width: side, height: side, cell_size: GROW_CELL_M, data };
        let (old, new) = models(&heights);
        let before = model_water(&drawing, &water, HAND_SEED, &heights).expect("the plane's heights");
        let (pits_before, _) = pits_in(&before, &old, &new, &drawing, &weight, HAND_SEED);
        let Mixed { model: mixed, troughs, pits_left } = mixed_model(&old, &new, &drawing, &water, HAND_SEED, &weight).expect("a mixed model");
        let (pits_after, deepest_left) = pits_in(&mixed, &old, &new, &drawing, &weight, HAND_SEED);
        println!(
            "the plane's {} holes: {pits_before} pits; {} troughs ({} filled), {pits_left} pits left, the deepest {deepest_left:.2} m",
            holes.len(),
            troughs.len(),
            troughs.iter().filter(|trough| trough.filled).count()
        );
        assert_eq!(pits_before, holes.len(), "each hole is a pit");
        assert_eq!(troughs.len(), TROUGH_PASSES);
        assert!(troughs.iter().all(|trough| !trough.filled), "{troughs:?}");
        assert_eq!(pits_left, holes.len() - TROUGH_PASSES);
        assert_eq!(pits_left, pits_after, "the pits left are those the passes' test finds");
        assert_kept_where_weight_0(&mixed, &old, &weight, "the plane");
        assert_worked_out_again(&mixed, &drawing, HAND_SEED, "the plane");
    }

    /// The bowl of the trough's test under a lake of the new drawing is no pit, nor where the kept
    /// model or the new one holds its water: the mix stays as it is.
    #[test]
    fn a_drawn_lake_is_never_a_pit_and_water_either_model_holds_is_none() {
        let water = WaterRules::default();
        let weight = band();
        let heights = bowl(Notch::Inward);
        let (old, new) = models(&heights);
        let on_lake = plain(vec![Lake { x_m: 0.0, z_m: 0.0, radius_m: BOWL_LAKE_M }]);
        let Mixed { model: mixed, troughs, .. } = mixed_model(&old, &new, &on_lake, &water, HAND_SEED, &weight).expect("a mixed model");
        assert!(troughs.is_empty(), "a drawn lake: {troughs:?}");
        assert!(bits(&mixed.heights.data) == bits(&heights.data), "a drawn lake: the mix changed");
        assert_eq!(mixed.lakes.len(), 1, "the lake holds the bowl's water");

        let holding = |model: &Grown| {
            let mut holding = model.clone();
            for (cell, depth) in holding.water_depth_m.data.iter_mut().enumerate() {
                if on_floor(cell) {
                    *depth = HELD_ON_FLOOR_M;
                }
            }
            holding
        };
        for (case, old, new) in [("the kept model", holding(&old), new.clone()), ("the new model", old.clone(), holding(&new))] {
            let Mixed { model: mixed, troughs, .. } = mixed_model(&old, &new, &plain(Vec::new()), &water, HAND_SEED, &weight).expect("a mixed model");
            assert!(troughs.is_empty(), "{case} holds the water: {troughs:?}");
            assert!(bits(&mixed.heights.data) == bits(&heights.data), "{case} holds the water: the mix changed");
        }
    }

    #[test]
    fn a_mixed_model_refuses_what_does_not_fit_the_edited_drawing() {
        let water = WaterRules::default();
        let heights = bowl(Notch::Inward);
        let (old, new) = models(&heights);
        let weight = band();
        let small = by_hand(Grid::<f32>::new(3, 3, GROW_CELL_M));
        let mut one_point = plain(Vec::new());
        one_point.rivers.push(crate::drawing::River { points: vec![(0.0, 0.0)], width_m: 4.0 });
        let mut no_depth = new.clone();
        no_depth.water_depth_m = Grid::new(3, 3, GROW_CELL_M);
        let (mut beyond, mut not_a_number) = (weight.clone(), weight.clone());
        beyond[7] = 1.5;
        not_a_number[7] = f32::NAN;
        let cases = [
            ("a drawing with a river of one point", &old, &new, &one_point, &weight[..], "river 1"),
            ("a kept model of another size", &small, &new, &plain(Vec::new()), &weight[..], "kept model's heights"),
            ("a new model without its depth", &old, &no_depth, &plain(Vec::new()), &weight[..], "new model's water's depth"),
            ("a weight too short", &old, &new, &plain(Vec::new()), &weight[1..], "weight"),
            ("a weight above 1", &old, &new, &plain(Vec::new()), &beyond[..], "weight"),
            ("a weight that is not a number", &old, &new, &plain(Vec::new()), &not_a_number[..], "weight"),
        ];
        for (case, old, new, drawing, weight, name) in cases {
            match mixed_model(old, new, drawing, &water, HAND_SEED, weight) {
                Err(error @ LandformError::BadConfig(_)) => assert!(error.to_string().contains(name), "{case}: {error} should name the {name}"),
                other => panic!("{case}: expected BadConfig naming the {name}, got {:?}", other.map(|mixed| mixed.troughs)),
            }
        }
    }

    /// One edit of a land, up to its mixed model: the edited drawing's model grown whole, the
    /// weight of Task 3's outline from the kept land (its footprint from the two starts, its lakes'
    /// zones from the land and both models, no marks, the least reach), the mixed model with its
    /// troughs and how long `mixed_model` took, and the band's pits before the troughs and after,
    /// each as a count and the deepest's water.
    struct MixedEdit {
        new_model: Grown,
        weight: Vec<f32>,
        unopened: Grown,
        mixed: Grown,
        troughs: Vec<Trough>,
        pits_left: usize,
        before: (usize, f32),
        after: (usize, f32),
        ms: f64,
    }

    fn mixed_edit(old_drawing: &Drawing, old_start: &Start, old_model: &Grown, old_world: &World, drawing: &Drawing, seed: u64) -> MixedEdit {
        let water = WaterRules::default();
        let new_model = grow(drawing, &water, seed, &AtomicBool::new(false)).expect("the edited drawing grows");
        let new_start = start_of(drawing, &water, seed).expect("the edited drawing is valid");
        let found = footprint(old_start, &new_start, old_drawing, drawing);
        let zones = lake_zones(old_world, old_model, &new_model, old_drawing, drawing);
        let weight = outline(drawing.size_m, &found, &zones, &Marks::none(old_world.heights.width), REACH_MIN_M).model_weight;
        let mut routing = ModelRouting::new(drawing, &water, seed).expect("the edited drawing is valid");
        let unopened = routing.water(&mix(old_model, &new_model, &weight)).expect("the mix's heights");
        let before = pits_in(&unopened, old_model, &new_model, drawing, &weight, seed);
        let clock = Instant::now();
        let Mixed { model: mixed, troughs, pits_left } = mixed_model(old_model, &new_model, drawing, &water, seed, &weight).expect("a mixed model");
        let ms = clock.elapsed().as_secs_f64() * 1000.0;
        let after = pits_in(&mixed, old_model, &new_model, drawing, &weight, seed);
        MixedEdit { new_model, weight, unopened, mixed, troughs, pits_left, before, after, ms }
    }

    /// Prints what `mixed_model` did to `edit`, and asserts what holds of every edit: the cells of
    /// weight 0 kept bit for bit, the model worked out again from its heights alone, and no pit
    /// left in the band, none counted as left. Returns how many model cells the troughs lowered.
    fn report_and_check(name: &str, old_model: &Grown, edit: &MixedEdit, drawing: &Drawing, seed: u64) -> usize {
        let (before, after) = (&edit.unopened.heights.data, &edit.mixed.heights.data);
        let lowered = before.iter().zip(after).filter(|(before, after)| after < before).count();
        let raised = before.iter().zip(after).filter(|(before, after)| after > before).count();
        println!(
            "{name}: mixed_model {:.1} ms; band pits before the troughs {} (deepest {:.2} m), after {} (deepest {:.2} m); {} troughs, {lowered} cells lowered and {raised} raised",
            edit.ms,
            edit.before.0,
            edit.before.1,
            edit.after.0,
            edit.after.1,
            edit.troughs.len()
        );
        for trough in &edit.troughs {
            println!("  at ({:.0}, {:.0}) m: {:.2} m {}", trough.at.0, trough.at.1, trough.depth_m, if trough.filled { "filled" } else { "cut" });
        }
        assert_kept_where_weight_0(&edit.mixed, old_model, &edit.weight, name);
        assert_worked_out_again(&edit.mixed, drawing, seed, name);
        assert_eq!(edit.after.0, 0, "{name}: band cells away from a drawn lake hold more than {PIT_DEPTH_M} m of water neither model holds");
        assert_eq!(edit.pits_left, 0, "{name}: pits left");
        lowered
    }

    /// The example at 2,048 m in hills from seed 11, with its brook removed (edit `g`) and its main
    /// river's second point moved 250 m (edit `h`): no pit is left, and no trough is deeper than
    /// `TROUGH_SAID_M` (the prototype: none in `g`, one pit filled in `h`).
    #[test]
    fn the_examples_brook_removed_and_main_river_moved_leave_no_pit() {
        let old = old_land();
        for name in ["g", "h"] {
            let drawing = example_edit(&old.drawing, name).expect("an edit of the example");
            let edit = mixed_edit(&old.drawing, &old.start, &old.model, &old.world, &drawing, EXAMPLE_SEED);
            report_and_check(&format!("edit {name}"), &old.model, &edit, &drawing, EXAMPLE_SEED);
            assert!(edit.troughs.iter().all(|trough| trough.depth_m <= TROUGH_SAID_M), "edit {name}: {:?}", edit.troughs);
        }
    }

    /// The trial `shoulder` at 1,024 m in hills from seed 3, with its standard edit: the land grown
    /// whole has a brook where the kept land has a ridge at the band's edge, and the trough that lets
    /// its water out is the deepest of every land measured (the prototype: 14.2 m, 623 model cells
    /// lowered). Its fade, not the V, holds up most of the cells it lowers, so their sides are
    /// steeper than the V's (`TROUGH_SIDE`): the count of sides steeper than the V's bound that were
    /// no steeper than `TROUGH_SIDE` before, and the steepest, are printed.
    #[test]
    fn the_shoulders_new_valley_is_let_out_by_a_trough_between_10_and_16_m_deep() {
        const DEEPEST_FROM_M: f32 = 10.0;
        const DEEPEST_TO_M: f32 = 16.0;
        const PROTOTYPE_LOWERED: usize = 623;
        const SEED: u64 = 3;
        let config = GenConfig::default();
        let drawing = trial_drawing("shoulder", 1024, Country::hills()).expect("a trial drawing");
        let start = start_of(&drawing, &config.water, SEED).expect("the trial is valid");
        let model = grow(&drawing, &config.water, SEED, &AtomicBool::new(false)).expect("the trial grows");
        let world = finish_drawn(SEED, &drawing, &config, &model, Frame::Model, &AtomicBool::new(false)).expect("the trial's world");
        let edited = standard_edit(&drawing).expect("the trial's edit");
        let edit = mixed_edit(&drawing, &start, &model, &world, &edited, SEED);
        let lowered = report_and_check("shoulder", &model, &edit, &edited, SEED);
        assert!(edit.before.0 > 0, "no pit before the troughs");
        assert_eq!(lowered, PROTOTYPE_LOWERED, "model cells lowered");
        // The sides, each pair of neighbours once: those no steeper than the V's side before and
        // steeper than the V's bound after.
        let (side, (before, after)) = (edit.mixed.heights.width, (&edit.unopened.heights, &edit.mixed.heights));
        let v_bound = TROUGH_SIDE + TROUGH_FALL * std::f64::consts::SQRT_2;
        let (mut steeper, mut steepest) = (0, (0.0f64, 0, 0));
        for cell in 0..side * side {
            for next in around(side, cell).filter(|&next| next > cell) {
                let slope = |grid: &Grid<f32>| (f64::from(grid.data[cell]) - f64::from(grid.data[next])).abs() / link_m(side, cell, next);
                if slope(before) <= TROUGH_SIDE && slope(after) > v_bound {
                    steeper += 1;
                    if slope(after) > steepest.0 {
                        steepest = (slope(after), cell, next);
                    }
                }
            }
        }
        let routing = ModelRouting::new(&edited, &config.water, SEED).expect("a valid drawing");
        println!(
            "shoulder: {steeper} sides no steeper than {TROUGH_SIDE} before are steeper than {v_bound:.6} after; the steepest {:.3} in 1, between {:?} and {:?}",
            steepest.0,
            routing.cell_m(steepest.1),
            routing.cell_m(steepest.2)
        );
        let deepest = edit.troughs.iter().filter(|trough| !trough.filled).map(|trough| trough.depth_m).fold(0.0, f32::max);
        assert!((DEEPEST_FROM_M..=DEEPEST_TO_M).contains(&deepest), "the deepest trough cut is {deepest} m: {:?}", edit.troughs);
    }

    /// Review Focus 3: an edit of the example that adds a lake of 10 m on a ridge 220 m east of the
    /// brook, which the engine leaves out. Its centre lies between four model cells, one of them the
    /// one at (312, 416), which stands 7.6 m above the cells 24 m from it on either side along one
    /// line; the hollow `dig_lakes` digs into those four drains over the ridge's flank, so the
    /// model holds no water deeper than 0.3 m within the lake's radius. The mixed model holds none
    /// there either; no trough is cut for it; and its world says so in the engine's own words
    /// (`fill_lakes`), as the world grown whole does. Such a lake is rare: of the example's 16
    /// highest crests, 3 lose a lake of 10 m centred between cells, and none loses one of 14 m so
    /// placed or one of 6, 8, 10, 14 or 40 m centred on a cell; nor do its 32 steepest flanks lose
    /// one of 12 or 24 m, and no lake of the 153 trial worlds is left out.
    #[test]
    fn a_lake_the_ground_cannot_hold_gets_no_trough_and_the_engines_note() {
        const RIDGE_LAKE: Lake = Lake { x_m: 316.0, z_m: 420.0, radius_m: 10.0 };
        const NOTE: &str = "Lake 2 holds no water; it was left out.";
        let old = old_land();
        let config = GenConfig::default();
        let mut drawing = old.drawing.clone();
        drawing.lakes.push(RIDGE_LAKE);
        let edit = mixed_edit(&old.drawing, &old.start, &old.model, &old.world, &drawing, EXAMPLE_SEED);
        report_and_check("a lake on a ridge", &old.model, &edit, &drawing, EXAMPLE_SEED);
        assert!(edit.troughs.is_empty(), "{:?}", edit.troughs);
        let routing = ModelRouting::new(&drawing, &config.water, EXAMPLE_SEED).expect("a valid drawing");
        let by_lake: Vec<usize> = (0..edit.weight.len())
            .filter(|&cell| {
                let (x, z) = routing.cell_m(cell);
                (x - RIDGE_LAKE.x_m).hypot(z - RIDGE_LAKE.z_m) <= PIT_CLEAR_OF_LAKE_RADII * f64::from(RIDGE_LAKE.radius_m)
            })
            .collect();
        assert!(!by_lake.is_empty());
        for (name, model) in [("the new model", &edit.new_model), ("the mixed model", &edit.mixed)] {
            assert!(model.lakes.iter().all(|lake| lake.drawn != 1) && model.lakes.iter().any(|lake| lake.drawn == 0), "{name}: {:?}", model.lakes);
            let deep = by_lake.iter().filter(|&&cell| f64::from(model.water_depth_m.data[cell]) > PIT_DEPTH_M).count();
            assert_eq!(deep, 0, "{name} holds water deeper than {PIT_DEPTH_M} m by the lake");
        }
        // No zone is taken for its water, which is not there: its zone, the new model's cells, is empty.
        let zones = lake_zones(&old.world, &old.model, &edit.new_model, &old.drawing, &drawing);
        assert!(zones.len() == 2 && !zones[0].is_empty() && zones[1].is_empty(), "the zones' sizes {:?}", zones.iter().map(Vec::len).collect::<Vec<_>>());
        // The lake's hollow, the only ground the new model holds otherwise, lies where the mix takes it.
        let dug: Vec<usize> = (0..edit.weight.len()).filter(|&cell| edit.new_model.heights.data[cell].to_bits() != old.model.heights.data[cell].to_bits()).collect();
        assert!(!dug.is_empty() && dug.iter().all(|&cell| edit.weight[cell] == 1.0), "the hollow's {} cells", dug.len());
        let world = finish_drawn(EXAMPLE_SEED, &drawing, &config, &edit.mixed, Frame::Model, &AtomicBool::new(false)).expect("the mixed model's world");
        let whole = finish_drawn(EXAMPLE_SEED, &drawing, &config, &edit.new_model, Frame::Model, &AtomicBool::new(false)).expect("the world grown whole");
        println!("a lake on a ridge: the hollow dug on {} model cells; the mixed model's world says {:?}; grown whole, {:?}", dug.len(), world.notes, whole.notes);
        for (name, world) in [("the mixed model's world", &world), ("the world grown whole", &whole)] {
            assert_eq!(world.notes.iter().filter(|note| note.contains("Lake 2")).collect::<Vec<_>>(), [NOTE], "{name}");
            assert!(world.water.lakes.iter().all(|lake| lake.drawn != 1) && world.water.lakes.iter().any(|lake| lake.drawn == 0), "{name}: its lakes");
        }
    }
}
