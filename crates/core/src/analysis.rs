use crate::flow::{neighbours, route};
use crate::grid::Grid;
use crate::rules::GroundRules;
use rayon::prelude::*;

/// Side of a coarse analysis cell in metres. A coarse cell is the mean of a square block of world
/// cells this wide, rounded to whole cells and at least one: 4 x 4 cells of the 1 m world grid.
pub const ANALYSIS_CELL_M: f32 = 4.0;

/// Radius of the neighbourhood ridge height is measured over, in metres.
pub const RIDGE_RADIUS_M: f32 = 48.0;

/// The wetness index, `ln` of the area drained per metre of contour over the slope's tangent, at
/// which ground counts as fully dry and as fully wet.
const WETNESS_INDEX_DRY: f32 = 5.0;
const WETNESS_INDEX_WET: f32 = 13.0;
/// The smallest slope tangent the wetness index divides by, so a dead-flat plain is not wet.
const WETNESS_MIN_TAN: f32 = 0.02;
/// Loose material begins to come to rest this many degrees below `scree_rest_slope_deg`.
const SCREE_REST_FADE_DEG: f32 = 8.0;

/// The ground of a world analysed on the coarse grid: every field has `ceil(size / 4)` cells a
/// side and `cell_size` 4.0 on a world of 1 m cells.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// Mean height of each block of world cells.
    pub coarse: Grid<f32>,
    /// Ground draining through the cell in square metres, at least one cell's area. This is the
    /// raw routing result, a line one cell wide for a stream; the deposit and the wetness are
    /// computed from it. The cover reads `flow_log10` instead.
    pub drained_m2: Grid<f32>,
    /// `log10` of `drained_m2` blurred `SMOOTHING_PASSES` times, so a diagonal stream has no
    /// staircase edge. The blur runs on the area itself, not on its logarithm: it keeps the total
    /// water and spreads a stream's peak over its neighbours, while a blur in log space would pull
    /// a one-cell stream's peak below the wet-flow rules.
    pub flow_log10: Grid<f32>,
    /// 0 dry to 1 wet: the wetness computed from the raw `drained_m2`, then blurred
    /// `SMOOTHING_PASSES` times like `flow_log10`.
    pub wetness: Grid<f32>,
    /// Height in metres above the mean of the square neighbourhood of radius `RIDGE_RADIUS_M`,
    /// narrowed near the grid's edge so it stays centred on the cell; negative in hollows.
    pub ridge_m: Grid<f32>,
    /// 0 to 1: loose material resting here.
    pub deposit: Grid<f32>,
}

/// Analyses `heights`: coarse heights, where water runs and how wet the ground is, ridges and
/// hollows, and where loose material from steep ground comes to rest.
pub fn analyse(heights: &Grid<f32>, rules: &GroundRules) -> Analysis {
    let coarse = coarsen(heights);
    let slope = slope_deg(&coarse);
    let flow = route(&coarse);
    let wetness = smooth(&wetness(&flow.drained_m2, &slope));
    let flow_log10 = log10_grid(&smooth(&flow.drained_m2));
    let deposit = deposit(&coarse, &slope, &flow.order, rules);
    let table = summed_area(&coarse);
    let ridge_m = ridge_height(&coarse, &table, RIDGE_RADIUS_M);
    Analysis { coarse, drained_m2: flow.drained_m2, flow_log10, wetness, ridge_m, deposit }
}

/// How many times the flow-derived fields (the drained area and the wetness) are blurred with
/// `[1, 2, 1] / 4` along x and along z (about a one-cell Gaussian) before the cover reads them. Water routing gathers a stream into a
/// line one coarse cell wide, so a stream that runs diagonally is a staircase of 4 m cells and
/// bilinear sampling would keep the steps as a saw-tooth edge on the wet ground along it.
pub(crate) const SMOOTHING_PASSES: usize = 2;

/// `grid` blurred `SMOOTHING_PASSES` times with `[1, 2, 1] / 4` along x, then along z. At the
/// grid's edge the weights are renormalised over the cells that exist, so a constant field stays
/// exactly constant (the sums run in `f64`, where they are exact for `f32` values) and, away from
/// the edge, the total is kept. Each output cell reads shared inputs only, so rows run in parallel
/// and the result does not depend on threads.
pub(crate) fn smooth(grid: &Grid<f32>) -> Grid<f32> {
    let (width, height) = (grid.width, grid.height);
    let mut current = grid.clone();
    let mut across = Grid::new(width, height, grid.cell_size);
    for _ in 0..SMOOTHING_PASSES {
        let source = &current.data;
        across.data.par_chunks_mut(width.max(1)).zip(source.par_chunks(width.max(1))).for_each(|(out, row)| {
            for (i, value) in out.iter_mut().enumerate() {
                let (before, after) = (i > 0, i + 1 < width);
                let mut sum = 2.0 * f64::from(row[i]);
                let mut weight = 2.0;
                if before {
                    sum += f64::from(row[i - 1]);
                    weight += 1.0;
                }
                if after {
                    sum += f64::from(row[i + 1]);
                    weight += 1.0;
                }
                *value = (sum / weight) as f32;
            }
        });
        let source = &across.data;
        current.data.par_chunks_mut(width.max(1)).enumerate().for_each(|(j, out)| {
            let row = |j: usize| &source[j * width..(j + 1) * width];
            for (i, value) in out.iter_mut().enumerate() {
                let mut sum = 2.0 * f64::from(row(j)[i]);
                let mut weight = 2.0;
                if j > 0 {
                    sum += f64::from(row(j - 1)[i]);
                    weight += 1.0;
                }
                if j + 1 < height {
                    sum += f64::from(row(j + 1)[i]);
                    weight += 1.0;
                }
                *value = (sum / weight) as f32;
            }
        });
    }
    current
}

/// Rise per metre along x and along z at `(i, j)`: central differences inside, one-sided
/// differences on the border.
pub(crate) fn gradient_at(heights: &Grid<f32>, i: usize, j: usize) -> (f32, f32) {
    let (w, h) = (heights.width, heights.height);
    let (x0, x1) = (i.saturating_sub(1), (i + 1).min(w - 1));
    let (y0, y1) = (j.saturating_sub(1), (j + 1).min(h - 1));
    let gx = if x1 > x0 { (heights.get(x1, j) - heights.get(x0, j)) / ((x1 - x0) as f32 * heights.cell_size) } else { 0.0 };
    let gy = if y1 > y0 { (heights.get(i, y1) - heights.get(i, y0)) / ((y1 - y0) as f32 * heights.cell_size) } else { 0.0 };
    (gx, gy)
}

/// Rise over run at `(i, j)`: the length of `gradient_at`.
pub(crate) fn slope_at(heights: &Grid<f32>, i: usize, j: usize) -> f32 {
    let (gx, gy) = gradient_at(heights, i, j);
    (gx * gx + gy * gy).sqrt()
}

/// Slope of every cell in degrees: central differences, one-sided on the border.
pub fn slope_deg(heights: &Grid<f32>) -> Grid<f32> {
    let mut slope = Grid::new(heights.width, heights.height, heights.cell_size);
    slope.data.par_chunks_mut(heights.width.max(1)).enumerate().for_each(|(j, row)| {
        for (i, value) in row.iter_mut().enumerate() {
            *value = slope_at(heights, i, j).atan().to_degrees();
        }
    });
    slope
}

/// Bilinear sample of a coarse grid at a position in metres, clamped at the edges. Coarse cell
/// `(i, j)` covers `i * cell_size` to `(i + 1) * cell_size` metres along x, and the same along z
/// for `j`; its value stands at its middle. So world cell `(i, j)` of `c` metres is read at
/// `((i + 0.5) * c, (j + 0.5) * c)`.
pub fn sample(coarse: &Grid<f32>, x_m: f32, z_m: f32) -> f32 {
    let u = (x_m / coarse.cell_size - 0.5).clamp(0.0, (coarse.width - 1) as f32);
    let v = (z_m / coarse.cell_size - 0.5).clamp(0.0, (coarse.height - 1) as f32);
    let (i0, j0) = (u as usize, v as usize);
    let (i1, j1) = ((i0 + 1).min(coarse.width - 1), (j0 + 1).min(coarse.height - 1));
    let (fu, fv) = (u - i0 as f32, v - j0 as f32);
    let top = coarse.get(i0, j0) + fu * (coarse.get(i1, j0) - coarse.get(i0, j0));
    let bottom = coarse.get(i0, j1) + fu * (coarse.get(i1, j1) - coarse.get(i0, j1));
    top + fv * (bottom - top)
}

impl Analysis {
    /// `log10` of the drained area, smoothed, at a position in metres: the one reader the cover and
    /// the `flow` layer share.
    pub fn flow_log10_at(&self, x_m: f32, z_m: f32) -> f32 {
        sample(&self.flow_log10, x_m, z_m)
    }

    /// The smoothed wetness at a position in metres: the one reader the cover and the `wetness`
    /// layer share.
    pub fn wetness_at(&self, x_m: f32, z_m: f32) -> f32 {
        sample(&self.wetness, x_m, z_m)
    }

    /// The loose material resting at a position in metres: the one reader the cover and the
    /// detail share.
    pub fn deposit_at(&self, x_m: f32, z_m: f32) -> f32 {
        sample(&self.deposit, x_m, z_m)
    }
}

/// The middle of world cell `(i, j)` of `cell_size` metres, where the coarse grids are read.
pub(crate) fn cell_middle(i: usize, j: usize, cell_size: f32) -> (f32, f32) {
    ((i as f32 + 0.5) * cell_size, (j as f32 + 0.5) * cell_size)
}

/// A smooth step from 0 at `start` to 1 at `full`; a plain step when they are equal, 1 from
/// `full` on.
pub fn ramp(start: f32, full: f32, x: f32) -> f32 {
    if start == full {
        return if x < full { 0.0 } else { 1.0 };
    }
    let t = ((x - start) / (full - start)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `log10` of an area in square metres, read as at least the smallest positive `f32`, so a rule
/// of 0 m² gives a finite threshold.
pub(crate) fn log10_m2(area_m2: f32) -> f32 {
    area_m2.max(f32::MIN_POSITIVE).log10()
}

/// World cells along each side of the block one coarse cell averages.
fn block_cells(cell_size: f32) -> usize {
    (ANALYSIS_CELL_M / cell_size).round().max(1.0) as usize
}

/// The mean of each square block of world cells; edge blocks average the cells that exist.
pub(crate) fn coarsen(heights: &Grid<f32>) -> Grid<f32> {
    let block = block_cells(heights.cell_size);
    let width = heights.width.div_ceil(block);
    let mut coarse = Grid::new(width, heights.height.div_ceil(block), block as f32 * heights.cell_size);
    coarse.data.par_chunks_mut(width.max(1)).enumerate().for_each(|(cj, row)| {
        let rows = cj * block..((cj + 1) * block).min(heights.height);
        for (ci, value) in row.iter_mut().enumerate() {
            let columns = ci * block..((ci + 1) * block).min(heights.width);
            let mut sum = 0.0;
            for j in rows.clone() {
                for i in columns.clone() {
                    sum += f64::from(heights.get(i, j));
                }
            }
            *value = (sum / (rows.len() * columns.len()) as f64) as f32;
        }
    });
    coarse
}

/// `log10` of every cell of `drained_m2`, which is at least one cell's area and so above 0.
fn log10_grid(drained_m2: &Grid<f32>) -> Grid<f32> {
    let data = drained_m2.data.par_iter().map(|&area| area.log10()).collect();
    Grid { width: drained_m2.width, height: drained_m2.height, cell_size: drained_m2.cell_size, data }
}

/// `ln((drained / cell) / max(tan slope, WETNESS_MIN_TAN))`, mapped from dry to wet onto 0..1.
fn wetness(drained_m2: &Grid<f32>, slope_deg: &Grid<f32>) -> Grid<f32> {
    let cell = drained_m2.cell_size;
    let data = drained_m2
        .data
        .par_iter()
        .zip(slope_deg.data.par_iter())
        .map(|(&area, &slope)| {
            let index = (area / cell / slope.to_radians().tan().max(WETNESS_MIN_TAN)).ln();
            ramp(WETNESS_INDEX_DRY, WETNESS_INDEX_WET, index)
        })
        .collect();
    Grid { width: drained_m2.width, height: drained_m2.height, cell_size: cell, data }
}

/// Loose material resting on each coarse cell, from 0 to 1. Steep ground starts with its
/// steepness as `carry`; visiting cells in `order`, each hands every lower neighbour its carry
/// fading over `scree_reach_m`, and the neighbour keeps the largest it is handed. The material
/// rests only where the ground is gentle enough, and never on fully steep ground.
fn deposit(coarse: &Grid<f32>, slope_deg: &Grid<f32>, order: &[u32], rules: &GroundRules) -> Grid<f32> {
    let steep: Vec<f32> =
        slope_deg.data.iter().map(|&s| ramp(rules.rock_slope_start_deg, rules.rock_slope_full_deg, s)).collect();
    let mut rank = vec![0; order.len()];
    for (position, &cell) in order.iter().enumerate() {
        rank[cell as usize] = position;
    }
    let reach_cells = rules.scree_reach_m / coarse.cell_size;
    let mut carry = steep.clone();
    for &cell in order {
        let c = cell as usize;
        if carry[c] == 0.0 {
            continue;
        }
        for (n, distance_cells) in neighbours(coarse.width, coarse.height, c) {
            // Lower on the ground and later in the order, so it has not handed its carry on yet.
            if coarse.data[n] < coarse.data[c] && rank[n] > rank[c] {
                let handed = carry[c] * (-(distance_cells as f32) / reach_cells).exp();
                carry[n] = carry[n].max(handed);
            }
        }
    }
    let rest_start = rules.scree_rest_slope_deg - SCREE_REST_FADE_DEG;
    let data = carry
        .iter()
        .zip(&steep)
        .zip(&slope_deg.data)
        .map(|((&carried, &steepness), &slope)| {
            if steepness >= 1.0 {
                0.0
            } else {
                carried * (1.0 - ramp(rest_start, rules.scree_rest_slope_deg, slope))
            }
        })
        .collect();
    Grid { width: coarse.width, height: coarse.height, cell_size: coarse.cell_size, data }
}

/// Summed-area table of `grid` in `f64`, `width + 1` entries a row and `height + 1` rows: entry
/// `(i, j)` is the sum of every cell left of column `i` and above row `j`.
fn summed_area(grid: &Grid<f32>) -> Vec<f64> {
    let stride = grid.width + 1;
    let mut table = vec![0.0; stride * (grid.height + 1)];
    for j in 0..grid.height {
        let mut row_sum = 0.0;
        for i in 0..grid.width {
            row_sum += f64::from(grid.get(i, j));
            table[(j + 1) * stride + i + 1] = table[j * stride + i + 1] + row_sum;
        }
    }
    table
}

/// Each cell minus the mean of the neighbourhood within `radius_m` of it; `table` is the grid's
/// `summed_area`. Near the grid's edge the neighbourhood shrinks on both sides of the cell along
/// that axis, so it stays centred on the cell and any plane reads 0 everywhere: ground rising
/// toward an edge is not a ridge.
fn ridge_height(coarse: &Grid<f32>, table: &[f64], radius_m: f32) -> Grid<f32> {
    let (width, height) = (coarse.width, coarse.height);
    let (stride, radius) = (width + 1, (radius_m / coarse.cell_size).round() as usize);
    // Columns or rows the neighbourhood reaches on each side of `index` along an axis of `len`.
    let reach = |index: usize, len: usize| radius.min(index).min(len - 1 - index);
    let mut ridge = Grid::new(width, height, coarse.cell_size);
    ridge.data.par_chunks_mut(width.max(1)).enumerate().for_each(|(j, row)| {
        let (y0, y1) = (j - reach(j, height), j + reach(j, height) + 1);
        for (i, value) in row.iter_mut().enumerate() {
            let (x0, x1) = (i - reach(i, width), i + reach(i, width) + 1);
            let sum = table[y1 * stride + x1] - table[y0 * stride + x1] - table[y1 * stride + x0]
                + table[y0 * stride + x0];
            let mean = sum / ((x1 - x0) * (y1 - y0)) as f64;
            *value = (f64::from(coarse.get(i, j)) - mean) as f32;
        }
    });
    ridge
}

/// Constructed ground for the analysis, flow and cover tests: square grids of 1 m cells.
#[cfg(test)]
pub(crate) mod shapes {
    use crate::grid::Grid;

    /// A `size` x `size` grid of 1 m cells whose height at column `x`, row `z` is `height(x, z)`.
    pub(crate) fn shape(size: usize, height: impl Fn(f32, f32) -> f32) -> Grid<f32> {
        let mut g = Grid::new(size, size, 1.0);
        for z in 0..size {
            for x in 0..size {
                g.set(x, z, height(x as f32, z as f32));
            }
        }
        g
    }

    /// The value of the coarse cell holding world cell `(x, z)` of a 1 m world.
    pub(crate) fn at_cell(coarse: &Grid<f32>, x: usize, z: usize) -> f32 {
        coarse.get(x / 4, z / 4)
    }

    /// Dead flat at 5 m.
    pub(crate) fn plain() -> Grid<f32> {
        shape(256, |_, _| 5.0)
    }

    /// Rising at 60 degrees toward larger `x`.
    pub(crate) fn ramp_60() -> Grid<f32> {
        shape(256, |x, _| 1.732_050_8 * x)
    }

    /// Rising at 8 degrees toward larger `x`.
    pub(crate) fn ramp_8() -> Grid<f32> {
        let rise = 8f32.to_radians().tan();
        shape(256, |x, _| rise * x)
    }

    /// A `size` x `size` slope rising linearly from 20 degrees at `x = 0` to 50 degrees at the last
    /// column, the same in every row.
    pub(crate) fn steepening(size: usize) -> Grid<f32> {
        let (start, per_cell) = (20f32.to_radians(), (30.0 / (size - 1) as f32).to_radians());
        shape(size, |x, _| (start.cos().ln() - (start + per_cell * x).cos().ln()) / per_cell)
    }

    /// A ridge along `x = 128`, falling 0.12 m per metre on both sides.
    pub(crate) fn tent() -> Grid<f32> {
        shape(256, |x, _| 30.0 - 0.12 * (x - 128.0).abs())
    }

    /// The tent upside down: a trench along `x = 128`.
    pub(crate) fn trench() -> Grid<f32> {
        shape(256, |x, _| 0.12 * (x - 128.0).abs())
    }

    /// A valley along `x = 128` whose floor falls toward larger `z`.
    pub(crate) fn valley() -> Grid<f32> {
        shape(256, |x, z| 0.15 * (x - 128.0).abs() + 0.02 * (255.0 - z))
    }

    /// A V-shaped valley along the diagonal `x = z`, falling toward larger `x` and `z`.
    pub(crate) fn diagonal_valley() -> Grid<f32> {
        let root_2 = std::f32::consts::SQRT_2;
        shape(256, |x, z| 0.15 * (x - z).abs() / root_2 + 0.02 * (510.0 - x - z) / root_2)
    }

    /// A bowl lowest in the middle: a pit with no way out but over its rim.
    pub(crate) fn pit() -> Grid<f32> {
        shape(256, |x, z| 0.002 * ((x - 128.0).powi(2) + (z - 128.0).powi(2)))
    }

    /// A plateau at 60 m for `x < 96`, a 60 degree face down to 0 m by `x = 131`, then an apron
    /// falling at 8 degrees.
    pub(crate) fn cliff() -> Grid<f32> {
        cliff_from(96.0)
    }

    /// The same cliff with the plateau ending at `top`, so a longer apron fits the 256 cells.
    pub(crate) fn cliff_from(top: f32) -> Grid<f32> {
        let face = 60f32.to_radians().tan();
        let apron = 8f32.to_radians().tan();
        let foot = top + 60.0 / face;
        shape(256, |x, _| {
            if x < top {
                60.0
            } else if x < foot {
                60.0 - face * (x - top)
            } else {
                -apron * (x - foot)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::shapes::*;
    use super::*;

    /// Mean of `coarse` over every world cell of a 256 x 256 world whose column is in `columns`.
    fn mean_over_columns(coarse: &Grid<f32>, columns: std::ops::Range<usize>) -> f32 {
        let values: Vec<f32> =
            columns.flat_map(|x| (0..256).map(move |z| (x, z))).map(|(x, z)| at_cell(coarse, x, z)).collect();
        values.iter().sum::<f32>() / values.len() as f32
    }

    #[test]
    fn edge_blocks_average_the_cells_that_exist() {
        let coarse = coarsen(&shape(6, |x, z| x + 10.0 * z));
        assert_eq!((coarse.width, coarse.height, coarse.cell_size), (2, 2, ANALYSIS_CELL_M));
        assert_eq!(coarse.data, vec![16.5, 19.5, 46.5, 49.5]);
    }

    #[test]
    fn sample_reads_cell_middles_and_clamps_at_the_edges() {
        let coarse = Grid { width: 2, height: 1, cell_size: 4.0, data: vec![10.0, 20.0] };
        assert_eq!(sample(&coarse, 2.0, 2.0), 10.0);
        assert_eq!(sample(&coarse, 4.0, 2.0), 15.0);
        assert_eq!(sample(&coarse, 6.0, 2.0), 20.0);
        assert_eq!(sample(&coarse, -50.0, -50.0), 10.0);
        assert_eq!(sample(&coarse, 50.0, 50.0), 20.0);
    }

    #[test]
    fn a_60_degree_ramp_has_a_60_degree_slope() {
        let slope = slope_deg(&ramp_60());
        for z in 1..255 {
            for x in 1..255 {
                let s = slope.get(x, z);
                assert!((s - 60.0).abs() < 0.1, "slope {s} at ({x}, {z})");
            }
        }
    }

    #[test]
    fn a_tent_ridge_is_above_its_neighbourhood() {
        let a = analyse(&tent(), &GroundRules::default());
        let crest = at_cell(&a.ridge_m, 128, 128);
        let flank = at_cell(&a.ridge_m, 64, 128);
        assert!((2.0..=4.0).contains(&crest), "crest {crest}");
        assert!(flank.abs() < 0.05, "flank {flank}");
    }

    #[test]
    fn a_plain_slope_has_no_ridge_at_the_grid_edge() {
        let a = analyse(&ramp_8(), &GroundRules::default());
        let ridge = &a.ridge_m;
        let (index, worst) = ridge
            .data
            .iter()
            .enumerate()
            .fold((0, 0.0f32), |(i, w), (j, &v)| if v.abs() > w { (j, v.abs()) } else { (i, w) });
        let at = (index % ridge.width, index / ridge.width);
        assert!(worst <= 0.01, "ridge height {worst} at coarse cell {at:?} over {RIDGE_RADIUS_M} m");
    }

    #[test]
    fn a_trench_is_below_its_neighbourhood() {
        let a = analyse(&trench(), &GroundRules::default());
        let bottom = at_cell(&a.ridge_m, 128, 128);
        assert!(bottom < 0.0, "{bottom} at the bottom over {RIDGE_RADIUS_M} m");
    }

    #[test]
    fn a_flat_plain_is_not_wet() {
        let a = analyse(&plain(), &GroundRules::default());
        let wet_start = GroundRules::default().wet_start;
        let wet = a.wetness.data.iter().filter(|w| w.is_nan() || **w >= wet_start).count();
        let wettest = a.wetness.data.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert_eq!(wet, 0, "{wet} cells are wet; the wettest is {wettest}");
    }

    #[test]
    fn smoothing_keeps_a_constant_field_exactly_constant() {
        for (width, height) in [(1, 1), (1, 7), (9, 1), (2, 2), (17, 11)] {
            for value in [0.0, 1.0, 0.3, 4.605_170_2, 123_456.79] {
                let mut grid = Grid::new(width, height, 4.0);
                grid.data.fill(value);
                let smoothed = smooth(&grid);
                let wrong = smoothed.data.iter().filter(|&&v| v != value).count();
                assert_eq!(wrong, 0, "{wrong} of {width} x {height} cells differ from {value}, the first {:?}", &smoothed.data[..3.min(smoothed.data.len())]);
            }
        }
    }

    #[test]
    fn smoothing_spreads_a_spike_and_keeps_its_total() {
        let mut grid = Grid::new(41, 41, 4.0);
        grid.set(20, 20, 1000.0);
        let smoothed = smooth(&grid);
        let total: f64 = smoothed.data.iter().map(|&v| f64::from(v)).sum();
        assert!((total - 1000.0).abs() <= 1000.0 * 1e-4, "total {total}");
        assert!(smoothed.get(20, 20) < 1000.0 && smoothed.get(20, 20) > 0.0, "centre {}", smoothed.get(20, 20));
        for (x, z) in [(19, 20), (21, 20), (20, 19), (20, 21), (19, 19), (21, 21), (18, 20), (20, 22)] {
            assert!(smoothed.get(x, z) > 0.0, "nothing reached ({x}, {z})");
        }
        assert_eq!(smoothed.get(20 + SMOOTHING_PASSES + 1, 20), 0.0, "reached further than the passes allow");
    }

    #[test]
    fn loose_material_rests_below_a_cliff() {
        // The plateau ends at column 16, so the face's foot is at 50.6 and columns 221..256 lie
        // 170 to 205 m below it: 4.3 to 5.1 reaches at the default `scree_reach_m`, as the old
        // layout put them 3.6 to 5.0 reaches below at 25 m.
        let a = analyse(&cliff_from(16.0), &GroundRules::default());
        let below = mean_over_columns(&a.deposit, 56..64);
        let plateau = mean_over_columns(&a.deposit, 0..16);
        let far = mean_over_columns(&a.deposit, 221..256);
        assert!(below > 0.5, "below the face {below}");
        assert!(plateau < 0.05, "plateau {plateau}");
        assert!(far < 0.05, "far down the apron {far}");
    }
}
