//! How wet the water makes the ground beside it (`bank_wetness`).

use super::spread::Spread;
use crate::grid::Grid;
use crate::water_rules::WaterRules;

/// How wet the water makes each cell of `heights`, a grid of 1 m cells, 0 to 1; `depth` is the
/// water's depth on the same cells, above 0 on a water cell.
///
/// A water cell is 1. A dry cell within `rules.bank_wet_reach_m` of a water cell is wetted by
/// that cell's surface (its ground plus its depth): 1 where the dry cell's ground lies at most
/// `bank_wet_full_m` above it, 0 where it lies `bank_wet_none_m` or more above it, linear between.
/// Every other cell is 0. The water cell is the one the spread from the water cells hands the dry
/// cell (`Spread`): the nearest, or in a thin sliver between two waters one nearly as near.
/// Sequential and deterministic.
pub(crate) fn bank_wetness(heights: &Grid<f32>, depth: &Grid<f32>, rules: &WaterRules) -> Grid<f32> {
    debug_assert_eq!((heights.width, heights.height), (depth.width, depth.height), "the depth lies on the heights' cells");
    let is_water: Vec<bool> = depth.data.iter().map(|&depth| depth > 0.0).collect();
    let from_water = Spread::new(heights.width, heights.height, &is_water, f64::from(rules.bank_wet_reach_m));
    let (full_m, none_m) = (rules.bank_wet_full_m, rules.bank_wet_none_m);
    let mut wet = Grid::new(heights.width, heights.height, heights.cell_size);
    for (cell, value) in wet.data.iter_mut().enumerate() {
        *value = match from_water.source(cell) {
            _ if is_water[cell] => 1.0,
            None => 0.0,
            Some(water) => {
                let above_m = heights.data[cell] - (heights.data[water] + depth.data[water]);
                ((none_m - above_m) / (none_m - full_m)).clamp(0.0, 1.0)
            }
        };
    }
    wet
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `size` x `size` grid of 1 m cells holding `value(i, j)` at cell `(i, j)`.
    fn grid(size: usize, value: impl Fn(usize, usize) -> f32) -> Grid<f32> {
        let mut grid = Grid::new(size, size, 1.0);
        for j in 0..size {
            for i in 0..size {
                grid.data[j * size + i] = value(i, j);
            }
        }
        grid
    }

    #[test]
    fn a_bank_is_wet_by_its_height_above_the_water() {
        let rules = WaterRules::default();
        // Water 1 m deep in column 0, its surface at 10 m; the ground east of it rises 0.02 m per
        // metre from 10 m at column 1, so column i lies 0.02 (i - 1) m above the surface: fully
        // wet up to column 16, 0.6 wet at column 40, and 0 beyond, out of reach.
        let heights = grid(64, |i, _| if i == 0 { 9.0 } else { 10.0 + 0.02 * (i as f32 - 1.0) });
        let depth = grid(64, |i, _| if i == 0 { 1.0 } else { 0.0 });
        let wet = bank_wetness(&heights, &depth, &rules);
        for i in 0..64 {
            let above_m = 0.02 * (i as f32 - 1.0);
            let expected = if i == 0 {
                1.0
            } else if i > 40 {
                // Beyond the reach.
                0.0
            } else {
                ((1.5 - above_m) / 1.2).clamp(0.0, 1.0)
            };
            for j in [0, 31, 63] {
                let value = wet.get(i, j);
                assert!((value - expected).abs() < 1e-5, "column {i}, {above_m} m above the water: {value}, not {expected}");
            }
        }
    }

    #[test]
    fn a_bank_is_wet_only_within_the_reach() {
        // Flat ground at the water's surface, so every cell the water reaches is fully wet.
        let heights = grid(101, |_, _| 0.0);
        let depth = grid(101, |i, j| if (i, j) == (50, 50) { 0.5 } else { 0.0 });
        for reach_m in [0.0, 10.0, 40.0] {
            let rules = WaterRules { bank_wet_reach_m: reach_m, ..WaterRules::default() };
            let wet = bank_wetness(&heights, &depth, &rules);
            for j in 0..101 {
                for i in 0..101 {
                    let apart_m = (i as f32 - 50.0).hypot(j as f32 - 50.0);
                    let expected = if apart_m <= reach_m { 1.0 } else { 0.0 };
                    assert_eq!(wet.get(i, j), expected, "({i}, {j}), {apart_m} m from the water, reach {reach_m} m");
                }
            }
        }
    }

    #[test]
    fn a_cell_takes_the_nearest_waters_surface() {
        let rules = WaterRules::default();
        // Two waters on flat ground at 10 m: a low one, its surface at 9 m, at column 10, and a high
        // one, its surface at 11 m, at column 30. Columns up to 19 are nearer the low one, so the
        // ground lies 1 m above its surface there (wetness 0.4167); columns from 21 on lie below
        // the high one's surface (1); column 20 is as near both.
        let heights = grid(41, |i, _| match i {
            10 => 8.5,
            30 => 9.5,
            _ => 10.0,
        });
        let depth = grid(41, |i, _| match i {
            10 => 0.5,
            30 => 1.5,
            _ => 0.0,
        });
        let wet = bank_wetness(&heights, &depth, &rules);
        for i in (0..=19).chain(21..=40) {
            let expected = if i <= 19 && i != 10 { (1.5 - 1.0) / 1.2 } else { 1.0 };
            let value = wet.get(i, 20);
            assert!((value - expected).abs() < 1e-5, "column {i}: {value}, not {expected}");
        }
    }
}
