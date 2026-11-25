//! The drawn lakes, dug into the land model after the last round: each hollow a little below the
//! lowest point of its rim, with an uneven outline, so that the last routing fills it to the height
//! where it spills. Their levels are read from that routing.

use super::{smoothstep, About, GrownLake, Layout, Note};
use crate::drawing::Lake;
use crate::noise::{fbm, widened};
use std::f64::consts::TAU;

/// A lake's water is water deeper than this, in metres: its level is the surface of the largest
/// body of such water within its radius (`lake_levels`), and a lake holding none is not handed on.
const LAKE_WATER_MIN_M: f32 = 0.3;

/// The eight neighbours of a cell, as steps along the columns and the rows.
const AROUND: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];

/// A lake's rim is the lowest of this many points spaced evenly around its circle.
const RIM_POINTS: usize = 64;
/// A lake's outline lies within this share of its radius either way of its circle, by a noise of
/// `WOBBLE_OCTAVES` octaves whose longest wavelength is `WOBBLE_WAVELENGTH_M`.
const WOBBLE: f64 = 0.25;
const WOBBLE_OCTAVES: u32 = 2;
const WOBBLE_WAVELENGTH_M: f64 = 112.0;
/// A hollow is deepest out to this share of the distance to its outline and rises from there to
/// the outline, as a smooth step.
const DEEPEST_SHARE: f64 = 0.35;
/// A hollow's floor lies this far below the rim at the outline, in metres, and
/// `MIDDLE_DEEPER_M` further below it where the hollow is deepest.
const FLOOR_BELOW_RIM_M: f64 = 0.5;
const MIDDLE_DEEPER_M: f64 = 3.5;
/// The ground is brought this many times `w` of the way down to the floor, all the way at most.
const DIG_GAIN: f64 = 1.6;

/// Digs the hollows of `lakes` into `heights` after the last round, in the drawing's order,
/// following the toy's `finish`. A lake's rim is the lowest height among `RIM_POINTS` points evenly
/// spaced on its circle, each read at its nearest cell, after the lakes before it were dug; a lake
/// none of whose points has a cell is skipped with a note about it in `notes`. Each cell whose
/// distance from the centre is under the wobbling radius, `radius * (1 + WOBBLE * widened fbm)` with the
/// noise drawn from `seed` at the cell, is lowered to `min(h, h + (floor - h) * min(1, DIG_GAIN * w))`,
/// with `w = 1 - smoothstep(DEEPEST_SHARE, 1, d)`, `d` the distance as a share of the wobbling
/// radius, and `floor = rim - FLOOR_BELOW_RIM_M - MIDDLE_DEEPER_M * w`. The outlets are not lowered.
/// Returns the lakes dug, as indices into `lakes`, in order.
pub(super) fn dig_lakes(
    lakes: &[Lake],
    layout: &Layout,
    is_outlet: &[bool],
    seed: u64,
    heights: &mut [f64],
    notes: &mut Vec<Note>,
) -> Vec<usize> {
    let mut dug = Vec::with_capacity(lakes.len());
    for (index, lake) in lakes.iter().enumerate() {
        let (centre_x, centre_z, radius_m) = (lake.x_m, lake.z_m, f64::from(lake.radius_m));
        let rim = (0..RIM_POINTS)
            .filter_map(|point| {
                let angle = TAU * point as f64 / RIM_POINTS as f64;
                layout.cell_on_grid((centre_x + radius_m * angle.cos(), centre_z + radius_m * angle.sin()))
            })
            .map(|cell| heights[cell])
            .reduce(f64::min);
        // A guard: `Drawing::validate` keeps a lake's centre in the world and its radius within
        // half the world's size, so a quarter of its circle, 16 points or more, lies in the world,
        // and `grow` never comes here.
        let Some(rim) = rim else {
            notes.push((About::Lake(index), format!("Lake {} lies outside the land.", index + 1)));
            continue;
        };
        let reach_m = radius_m * (1.0 + WOBBLE);
        for row in layout.indices_near(centre_z, reach_m) {
            for column in layout.indices_near(centre_x, reach_m) {
                let cell = row * layout.side + column;
                if is_outlet[cell] {
                    continue;
                }
                let (x, z) = (layout.position_m(column), layout.position_m(row));
                let wobble = f64::from(widened(fbm(seed, x, z, WOBBLE_OCTAVES, WOBBLE_WAVELENGTH_M)));
                let d = (x - centre_x).hypot(z - centre_z) / (radius_m * (1.0 + WOBBLE * wobble));
                if d >= 1.0 {
                    continue;
                }
                let w = 1.0 - smoothstep(DEEPEST_SHARE, 1.0, d);
                let floor_m = rim - FLOOR_BELOW_RIM_M - MIDDLE_DEEPER_M * w;
                let h = heights[cell];
                heights[cell] = h.min(h + (floor_m - h) * (DIG_GAIN * w).min(1.0));
            }
        }
        dug.push(index);
    }
    dug
}

/// The lakes of `dug`, indices into `lakes` in order, that hold water deeper than
/// `LAKE_WATER_MIN_M` on a cell within their radius (its centre no further from the lake's centre
/// than the radius), each with its level: the flooded height of the lake's body of water.
///
/// Those cells make bodies, each connected across its cells' sides and corners. The lake's body is
/// the largest; of several as large, the one holding the cell nearest the lake's centre, and of
/// those the one holding the lowest cell index. Its cells agree to within the flood's tiny steps,
/// and the level is the highest of their flooded heights. So a stray pool higher up within the
/// circle does not set the level. The water's depth is read as `Grown::water_depth_m` holds it,
/// `flooded - heights` in `f32`.
pub(super) fn lake_levels(
    lakes: &[Lake],
    dug: &[usize],
    layout: &Layout,
    heights: &[f64],
    flooded: &[f64],
) -> Vec<GrownLake> {
    dug.iter()
        .filter_map(|&drawn| {
            let Lake { x_m, z_m, radius_m } = lakes[drawn];
            let radius_m = f64::from(radius_m);
            let from_centre_m = |cell: usize| {
                let (x, z) = layout.cell_m(cell);
                (x - x_m).hypot(z - z_m)
            };
            // In index order: the rows run outside, the columns inside.
            let deep: Vec<usize> = (layout.indices_near(z_m, radius_m))
                .flat_map(|row| layout.indices_near(x_m, radius_m).map(move |column| row * layout.side + column))
                .filter(|&cell| from_centre_m(cell) <= radius_m)
                .filter(|&cell| (flooded[cell] - heights[cell]) as f32 > LAKE_WATER_MIN_M)
                .collect();
            let body = largest_body(&deep, layout.side, from_centre_m)?;
            let level_m = body.iter().map(|&cell| flooded[cell]).reduce(f64::max)?;
            Some(GrownLake { drawn, level_m: level_m as f32 })
        })
        .collect()
}

/// Of the bodies `cells` (indices on a grid `side` cells a side, in order) make, each connected
/// across its cells' sides and corners, the largest, as `lake_levels` chooses it, `from_centre_m`
/// giving each cell's distance from the lake's centre; its cells in no particular order. `None`
/// when there are no cells.
fn largest_body(cells: &[usize], side: usize, from_centre_m: impl Fn(usize) -> f64) -> Option<Vec<usize>> {
    let mut taken = vec![false; cells.len()];
    // The body chosen so far, and its nearest cell's distance. The bodies are found in the order
    // of their lowest cells, so a later one as large and as near never replaces it.
    let mut chosen: Option<(Vec<usize>, f64)> = None;
    for start in 0..cells.len() {
        if taken[start] {
            continue;
        }
        let (mut body, mut waiting) = (Vec::new(), vec![start]);
        taken[start] = true;
        while let Some(place) = waiting.pop() {
            let cell = cells[place];
            body.push(cell);
            let (column, row) = (cell % side, cell / side);
            for (across, down) in AROUND {
                let Some(next_column) = column.checked_add_signed(across).filter(|&next| next < side) else { continue };
                let Some(next_row) = row.checked_add_signed(down).filter(|&next| next < side) else { continue };
                if let Ok(next) = cells.binary_search(&(next_row * side + next_column)) {
                    if !taken[next] {
                        taken[next] = true;
                        waiting.push(next);
                    }
                }
            }
        }
        let nearest_m = body.iter().map(|&cell| from_centre_m(cell)).fold(f64::INFINITY, f64::min);
        let better = chosen.as_ref().is_none_or(|(best, best_m)| {
            body.len() > best.len() || (body.len() == best.len() && nearest_m < *best_m)
        });
        if better {
            chosen = Some((body, nearest_m));
        }
    }
    chosen.map(|(body, _)| body)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{example_worlds, grown, largest_drain, on_edge, plain, position_m, wide_lake_worlds, ExampleWorld};
    use super::super::Grown;
    use super::*;
    use crate::drawing::{Drawing, Lake, River};
    use crate::water_rules::WaterRules;
    use std::f64::consts::PI;

    /// The flooded height (ground plus water) of each cell within `lake`'s radius whose water is
    /// deeper than 0.3 m.
    fn lake_surface_m(land: &Grown, size_m: u32, lake: &Lake) -> Vec<f64> {
        let side = land.heights.width;
        (0..side * side)
            .filter(|&cell| {
                let (x, z) = (position_m(size_m, cell % side), position_m(size_m, cell / side));
                (x - lake.x_m).hypot(z - lake.z_m) <= f64::from(lake.radius_m) && land.water_depth_m.data[cell] > 0.3
            })
            .map(|cell| f64::from(land.heights.data[cell]) + f64::from(land.water_depth_m.data[cell]))
            .collect()
    }

    /// The flooded height (ground plus water) at the cell within `lake`'s radius whose water is
    /// deepest, the first by index of several as deep, or `None` when no cell there holds water
    /// deeper than 0.3 m.
    fn flooded_at_the_deepest_m(land: &Grown, size_m: u32, lake: &Lake) -> Option<f64> {
        let side = land.heights.width;
        (0..side * side)
            .filter(|&cell| {
                let (x, z) = (position_m(size_m, cell % side), position_m(size_m, cell / side));
                (x - lake.x_m).hypot(z - lake.z_m) <= f64::from(lake.radius_m) && land.water_depth_m.data[cell] > 0.3
            })
            .reduce(|deepest, cell| if land.water_depth_m.data[cell] > land.water_depth_m.data[deepest] { cell } else { deepest })
            .map(|cell| f64::from(land.heights.data[cell]) + f64::from(land.water_depth_m.data[cell]))
    }

    #[test]
    fn a_wide_lake_takes_the_level_of_its_own_water() {
        let mut failures = Vec::new();
        for ExampleWorld { name, drawing, land, .. } in example_worlds().iter().chain(wide_lake_worlds()) {
            let lake = &drawing.lakes[0];
            let [listed] = land.lakes[..] else { panic!("{name}: one lake, not {:?}", land.lakes) };
            let deepest_m = flooded_at_the_deepest_m(land, drawing.size_m, lake).expect("water deeper than 0.3 m in the lake");
            let off_m = (f64::from(listed.level_m) - deepest_m).abs();
            if off_m > 0.05 {
                failures.push(format!("{name}: the level is {} m, the flooded height at the deepest cell {deepest_m} m", listed.level_m));
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn a_lake_takes_the_level_of_its_largest_body() {
        // Flat ground 10 m high on a 512 m layout, dry but where water is put; each case is the
        // water's cells as (column, row) from the centre of a lake 80 m in radius there, each with
        // its depth over the ground and the ground, and the level expected.
        let layout = Layout::new(512);
        let middle = (layout.side / 2) as i64;
        let cell = |(column, row): (i64, i64)| ((row + middle) * layout.side as i64 + column + middle) as usize;
        let lake = Lake { x_m: layout.position_m(middle as usize), z_m: layout.position_m(middle as usize), radius_m: 80.0 };
        let block = |from: (i64, i64), to: (i64, i64)| -> Vec<(i64, i64)> {
            (from.1..=to.1).flat_map(|row| (from.0..=to.0).map(move |column| (column, row))).collect()
        };
        // (name, [(cells, ground, flooded)], level)
        let cases: Vec<(&str, Vec<(Vec<(i64, i64)>, f64, f64)>, f64)> = vec![
            // The lake, 12 cells at 3 m, and a pool of 2 cells high up the circle's side, at 25.5 m.
            ("a higher pool", vec![(block((-2, -1), (1, 1)), -1.0, 3.0), (vec![(6, 6), (7, 6)], 25.0, 25.5)], 3.0),
            // A pool of 3 cells touching only at their corners is one body, larger than the 2
            // cells side by side at the centre.
            ("a body across corners", vec![(vec![(0, 0), (1, 0)], 0.0, 4.0), (vec![(4, 4), (5, 5), (6, 6)], 0.0, 6.0)], 6.0),
            // Two bodies as large: the one holding the cell nearest the centre.
            ("two as large", vec![(vec![(5, 0), (6, 0)], 0.0, 7.0), (vec![(-1, 2), (-1, 3)], 0.0, 5.0)], 5.0),
            // As large and as near: the one holding the lowest cell index, the northern one.
            ("as near", vec![(vec![(0, 3), (0, 4)], 0.0, 9.0), (vec![(0, -3), (0, -4)], 0.0, 8.0)], 8.0),
        ];
        for (name, bodies, level_m) in cases {
            let mut heights = vec![10.0; layout.cells()];
            let mut flooded = heights.clone();
            for (cells, ground_m, flooded_m) in &bodies {
                for &at in cells {
                    (heights[cell(at)], flooded[cell(at)]) = (*ground_m, *flooded_m);
                }
            }
            let found = lake_levels(&[lake], &[0], &layout, &heights, &flooded);
            assert_eq!(found, [GrownLake { drawn: 0, level_m: level_m as f32 }], "{name}");
        }
    }

    #[test]
    fn a_lake_holds_water_where_it_was_put() {
        for ExampleWorld { name, drawing, land, .. } in example_worlds() {
            let lake = &drawing.lakes[0];
            let wet_m2 = lake_surface_m(land, drawing.size_m, lake).len() as f64 * 64.0;
            let quarter_m2 = PI * f64::from(lake.radius_m).powi(2) / 4.0;
            assert!(
                wet_m2 >= quarter_m2,
                "{name}: water deeper than 0.3 m covers {wet_m2} m2 of the lake's circle, less than a quarter of it ({quarter_m2} m2)"
            );
        }
    }

    #[test]
    fn a_lake_is_level() {
        for ExampleWorld { name, drawing, land, .. } in example_worlds() {
            let surface = lake_surface_m(land, drawing.size_m, &drawing.lakes[0]);
            let (low, high) =
                surface.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), &level| (low.min(level), high.max(level)));
            assert!(
                !surface.is_empty() && high - low < 0.05,
                "{name}: the lake's surface lies from {low} m to {high} m over {} cells",
                surface.len()
            );
        }
    }

    #[test]
    fn awkward_lakes_still_give_land() {
        let example = Drawing::example_valley(512);
        let half_m = f64::from(example.size_m) / 2.0;
        let lake = example.lakes[0];
        let at = |(x_m, z_m): (f64, f64)| Lake { x_m, z_m, ..lake };
        // The centre of the example picture's highest pixel.
        let picture = example.high_ground.as_ref().expect("a picture");
        let (highest, _) = picture.data.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).expect("a pixel");
        let pixel_m = |index: usize, pixels: usize| ((index as f64 + 0.5) / pixels as f64 - 0.5) * 2.0 * half_m;
        let hilltop = (pixel_m(highest % picture.width, picture.width), pixel_m(highest / picture.width, picture.height));
        let cases = [
            ("a lake on the river's line", vec![at(example.rivers[0].points[2])]),
            ("a lake on the picture's highest pixel", vec![at(hilltop)]),
            ("a lake 10 m inside the world's edge", vec![at((half_m - 10.0, 0.0))]),
            ("two lakes that overlap", vec![lake, at((lake.x_m + f64::from(lake.radius_m), lake.z_m))]),
        ];
        for (name, lakes) in cases {
            let drawing = Drawing { lakes, ..example.clone() };
            let land = grown(&drawing, 1);
            for (grid_name, grid) in [("heights", &land.heights), ("drained_m2", &land.drained_m2), ("water_depth_m", &land.water_depth_m)] {
                let bad = grid.data.iter().filter(|value| !value.is_finite()).count();
                assert_eq!(bad, 0, "{name}: {bad} of the {grid_name} are not finite");
            }
            assert!(land.notes.is_empty(), "{name}: {:?}", land.notes);
        }
    }

    #[test]
    fn a_lake_leaves_the_outlets_alone() {
        // No river is drawn, so every cell on the grid's edge is an outlet, and the lake, as wide
        // as the world allows, reaches past the grid's east edge, 248 m from its centre.
        let lake = Lake { x_m: 512.0, z_m: 0.0, radius_m: 512.0 };
        let land = grown(&Drawing { lakes: vec![lake], ..plain(1024) }, 1);
        let side = land.heights.width;
        let dug = (0..side * side).filter(|&cell| on_edge(side, cell) && land.heights.data[cell] != 0.0).count();
        assert_eq!(dug, 0, "{dug} of the outlets on the grid's edge were dug");
    }

    /// Lakes dug by `dig_lakes` alone into flat ground 50 m high on a 512 m layout, whose outlets
    /// are the grid's edge cells, from `seed`; with the notes, each with what it is about, and the
    /// lakes dug.
    fn dug_flat(lakes: &[Lake], seed: u64) -> (Layout, Vec<f64>, Vec<Note>, Vec<usize>) {
        let layout = Layout::new(512);
        let is_outlet: Vec<bool> = (0..layout.cells()).map(|cell| on_edge(layout.side, cell)).collect();
        let (mut heights, mut notes) = (vec![50.0; layout.cells()], Vec::new());
        let dug = dig_lakes(lakes, &layout, &is_outlet, seed, &mut heights, &mut notes);
        (layout, heights, notes, dug)
    }

    #[test]
    fn a_hollow_follows_the_formula() {
        let lake = Lake { x_m: 0.0, z_m: 0.0, radius_m: 80.0 };
        let (layout, first, notes, dug) = dug_flat(&[lake], 1);
        let (_, second, _, _) = dug_flat(&[lake], 2);
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(dug, [0]);
        let cell = |at: (f64, f64)| layout.cell_on_grid(at).expect("a cell on the grid");
        // On flat ground the rim is the ground; the middle lies 0.5 + 3.5 m below it.
        assert_eq!(first[cell((0.0, 0.0))], 50.0 - 4.0, "the lake's centre");
        // The widest outline lies 1.25 radii out.
        assert_eq!(first[cell((104.0, 0.0))], 50.0, "1.3 radii out");
        let near_the_outline = (0..layout.cells()).filter(|&cell| {
            let (x, z) = layout.cell_m(cell);
            (0.8..=1.2).contains(&(x.hypot(z) / 80.0))
        });
        let differing = near_the_outline.filter(|&cell| first[cell] != second[cell]).count();
        assert!(differing > 0, "seeds 1 and 2 dig the same outline between 0.8 and 1.2 radii");
    }

    /// `model_water` counts every drawn lake as dug, as `grow` does: `dig_lakes` skips only a lake
    /// none of whose rim points has a cell, and `Drawing::validate` keeps a lake's centre in the
    /// world and its radius at most half the world's size, so the quarter of its circle facing the
    /// world's centre lies in the world. The widest lakes it allows, at the world's corners, the
    /// middles of two of its edges and its centre, on the smallest world and two larger, are dug.
    #[test]
    fn every_lake_a_drawing_allows_is_dug() {
        for size_m in [64, 512, 2048] {
            let half_m = f64::from(size_m) / 2.0;
            let lakes: Vec<Lake> = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, -1.0), (1.0, 0.0), (0.0, 0.0)]
                .map(|(x, z)| Lake { x_m: x * half_m, z_m: z * half_m, radius_m: half_m as f32 })
                .to_vec();
            let drawing = Drawing { lakes: lakes.clone(), ..plain(size_m) };
            drawing.validate().expect("the drawing allows these lakes");
            let layout = Layout::new(size_m);
            let is_outlet: Vec<bool> = (0..layout.cells()).map(|cell| on_edge(layout.side, cell)).collect();
            let (mut heights, mut notes) = (vec![50.0; layout.cells()], Vec::new());
            let dug = dig_lakes(&lakes, &layout, &is_outlet, 1, &mut heights, &mut notes);
            assert_eq!(dug, (0..lakes.len()).collect::<Vec<usize>>(), "at {size_m} m: {notes:?}");
            assert!(notes.is_empty(), "at {size_m} m: {notes:?}");
        }
    }

    #[test]
    fn a_lake_with_no_rim_on_the_grid_is_noted() {
        // `Drawing::validate` refuses this radius, so the lake is given to `dig_lakes` directly.
        let (_, heights, notes, dug) = dug_flat(&[Lake { x_m: 0.0, z_m: 0.0, radius_m: 1e5 }], 1);
        assert_eq!(notes, [(About::Lake(0), "Lake 1 lies outside the land.".to_string())]);
        assert!(heights.iter().all(|&h| h == 50.0) && dug.is_empty(), "the lake was dug");
    }

    #[test]
    fn a_river_that_ends_in_a_lake_runs_on_through_it() {
        let mut drawing = plain(2048);
        let lake = Lake { x_m: 0.0, z_m: 0.0, radius_m: 112.0 };
        drawing.lakes.push(lake);
        let water = WaterRules::default();
        drawing.rivers.push(River { points: vec![(-1024.0, 0.0), (40.0, 0.0)], width_m: water.river_inflow_width_m });
        let land = grown(&drawing, 1);
        assert_eq!(land.notes, ["River 1 ended in a lake; it was carried on to the edge so the lake has a way out."]);
        assert_eq!(land.notes_about, [About::River(0)]);
        let side = land.drained_m2.width;
        // A river as wide as `river_inflow_width_m` brings `river_inflow_m2`.
        let incoming = f64::from(water.river_inflow_m2);
        // The river's water reaches the lake.
        let in_lake_m2 = (0..side * side)
            .filter(|&cell| {
                let (x, z) = (position_m(drawing.size_m, cell % side), position_m(drawing.size_m, cell / side));
                (x - lake.x_m).hypot(z - lake.z_m) <= f64::from(lake.radius_m)
            })
            .map(|cell| f64::from(land.drained_m2.data[cell]))
            .fold(0.0, f64::max);
        assert!(in_lake_m2 >= incoming, "no cell in the lake drains the river's {incoming} m2; the most is {in_lake_m2} m2");
        // The lake holds water.
        let wet_m2 = lake_surface_m(&land, drawing.size_m, &lake).len() as f64 * 64.0;
        let quarter_m2 = PI * f64::from(lake.radius_m).powi(2) / 4.0;
        assert!(wet_m2 >= quarter_m2, "water deeper than 0.3 m covers {wet_m2} m2 of the lake's circle, less than {quarter_m2} m2");
        // The river carried on is the only way out: all the water leaves at its outlet.
        let whole = (side * side) as f64 * 64.0 + incoming;
        let (at, drained) = largest_drain(&land);
        let error = (drained - whole).abs() / whole;
        assert!(error < 1e-3, "the cell at {at:?} drains {drained} m2 of the {whole} m2 the grid and the river bring: relative error {error}");
    }
}
