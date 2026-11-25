use crate::analysis::{cell_middle, log10_m2, ramp, slope_at, Analysis};
use crate::grid::Grid;
use crate::noise::{fbm, widened};
use crate::rng::mix64;
use crate::rules::GroundRules;
use rayon::prelude::*;
use std::ops::Range;

/// The hummocks: `fbm` of this many octaves and this wavelength.
const OCTAVES: u32 = 4;
const WAVELENGTH_M: f64 = 12.0;
/// The hummocks fade in between these two slopes (rise over run), so flat ground stays flat.
const FADE_START: f32 = 0.02;
const FADE_END: f32 = 0.15;
/// Octaves of the roughness, whose wavelength is `rough_size_m`: with more, the finest would be
/// shorter than two cells and a 1 m grid could not hold it.
const ROUGH_OCTAVES: u32 = 2;
/// Octaves of the strata, a widened noise of the height alone whose wavelength is
/// `ledge_spacing_m`, so layers lie at irregular heights and differ in strength.
const STRATA_OCTAVES: u32 = 3;
/// The field that moves the strata up and down along a slope, by up to `ledge_spacing_m`: its
/// wavelength and octaves.
const WARP_WAVELENGTH_M: f64 = 40.0;
const WARP_OCTAVES: u32 = 2;
/// The field that lets ledges crop out in patches: its wavelength and octaves, and the value from
/// which the ledges are complete. Where it is 0 or below, about half of the steep ground, there
/// are none.
const MASK_WAVELENGTH_M: f64 = 90.0;
const MASK_OCTAVES: u32 = 2;
const MASK_FULL: f32 = 0.15;
/// A gully's depth varies along a stream as `GULLY_DEPTH_MEAN + GULLY_DEPTH_SWING` times a widened
/// noise, from 0.7 to 1 of its value around a mean of 0.85; the wavelength and octaves of that
/// noise. The depth varies slowly along a stream so that its bed keeps falling: with a short
/// wavelength (25 m) the cut shallowed fast enough downstream for the bed to step uphill.
const GULLY_DEPTH_MEAN: f32 = 0.85;
const GULLY_DEPTH_SWING: f32 = 0.15;
const GULLY_DEPTH_WAVELENGTH_M: f64 = 200.0;
const GULLY_DEPTH_OCTAVES: u32 = 2;
/// Side, in cells, of the square around a cell over which the slope gullies need is averaged: a
/// valley's centre line is flat across, so its own slope says little.
const MEAN_SLOPE_CELLS: usize = 13;
/// Xored into the detail seed for each noise field but the hummocks', which take the seed itself,
/// as they did before the other shapes.
const ROUGH_TAG: u64 = 1;
const WARP_TAG: u64 = 2;
const STRATA_TAG: u64 = 3;
const MASK_TAG: u64 = 4;
const GULLY_DEPTH_TAG: u64 = 5;

/// Adds four shapes, each an offset computed from the surface as it was on entry, summed and
/// clamped to `rules.detail_limit_m`:
///
/// - hummocks: `amplitude_m * fbm(seed)` wherever the ground slopes, fading in between rises of
///   0.02 and 0.15;
/// - roughness on rock and on resting loose material: `rough_amplitude_m * r * fbm`, where `r` is
///   the larger of the rock's steepness ramp and the deposit;
/// - ledges on steep ground: irregular strata, a noise of the height moved up and down along the
///   slope, cropping out in patches; at most `ledge_height_m`, fading in from
///   `ledge_slope_start_deg` to `ledge_slope_full_deg`;
/// - gullies along the water lines: down by up to `gully_depth_m`, varying along the stream, as
///   the smoothed flow rises from `gully_flow_start_m2` to `gully_flow_full_m2`, and fading out as
///   the mean slope of the 13 x 13 cells around the cell falls from `gully_min_slope_deg` to 0
///   (a `gully_min_slope_deg` of 0, the default, switches that fade off).
///
/// `amplitude_m` 0 leaves `heights` untouched, whatever the rules; a shape whose amount is 0
/// costs nothing. The analysis is read at each cell's middle. Rows run in parallel and a cell
/// reads only shared inputs, so the result does not depend on threads. `rules` must be valid
/// (`GroundRules::validate`), as `generate` makes sure. A drawn world fades its gullies out near its
/// lakes (`add_detail_faded`).
pub fn add_detail(heights: &mut Grid<f32>, analysis: &Analysis, seed: u64, amplitude_m: f32, rules: &GroundRules) {
    add_detail_faded(heights, analysis, seed, amplitude_m, rules, None);
}

/// `add_detail`, with each cell's gully cut multiplied by its value in `gully_factor` when that is
/// given: one value a cell of `heights`, in index order, from 0 (no gully, as with a
/// `gully_depth_m` of 0) to 1 (the whole cut). The hummocks, the roughness and the ledges are not
/// touched. Without it, exactly `add_detail`.
pub(crate) fn add_detail_faded(
    heights: &mut Grid<f32>,
    analysis: &Analysis,
    seed: u64,
    amplitude_m: f32,
    rules: &GroundRules,
    gully_factor: Option<&[f32]>,
) {
    let width = heights.width;
    if amplitude_m == 0.0 || width == 0 || heights.height == 0 {
        return;
    }
    debug_assert!(gully_factor.is_none_or(|factor| factor.len() == heights.data.len()), "one gully factor a cell");
    let shapes = Shapes::new(heights, analysis, seed, amplitude_m, rules);
    heights.data.par_chunks_mut(width).enumerate().for_each(|(j, row)| {
        for (i, h) in row.iter_mut().enumerate() {
            let factor = gully_factor.map(|factor| factor[j * width + i]);
            if let Some(offset) = shapes.offset(i, j, *h, factor) {
                *h += offset;
            }
        }
    });
}

/// Whether the detail cuts gullies under `amplitude_m` and `rules`: not with an `amplitude_m` of 0,
/// which leaves the heights untouched, nor with a `gully_depth_m` of 0. Where it does not, a gully
/// factor changes nothing (`add_detail_faded`), so none need be built.
pub(crate) fn cuts_gullies(amplitude_m: f32, rules: &GroundRules) -> bool {
    amplitude_m != 0.0 && rules.gully_depth_m != 0.0
}

/// The ledges' patch field at a point in metres, from 0 (no ledges) to 1.
fn ledge_mask(mask_seed: u64, x: f64, z: f64) -> f32 {
    ramp(0.0, MASK_FULL, fbm(mask_seed, x, z, MASK_OCTAVES, MASK_WAVELENGTH_M))
}

/// The cells of `0..len` within `MEAN_SLOPE_CELLS / 2` of `index`.
fn window(index: usize, len: usize) -> Range<usize> {
    let half = MEAN_SLOPE_CELLS / 2;
    index.saturating_sub(half)..(index + half + 1).min(len)
}

/// Each cell's slope in degrees summed over the cells of its row in its `window`: the first of
/// the two passes of the mean slope. The sums run in `f64` and are kept as `f32`.
fn row_slope_sums(rise: &Grid<f32>) -> Grid<f32> {
    let width = rise.width;
    let mut sums = Grid::new(width, rise.height, rise.cell_size);
    sums.data.par_chunks_mut(width).zip(rise.data.par_chunks(width)).for_each(|(out, rise_row)| {
        // As `slope_deg` computes it.
        let slope: Vec<f64> = rise_row.iter().map(|r| f64::from(r.atan().to_degrees())).collect();
        for (i, sum) in out.iter_mut().enumerate() {
            *sum = slope[window(i, width)].iter().sum::<f64>() as f32;
        }
    });
    sums
}

/// The mean slope in degrees of the `MEAN_SLOPE_CELLS` x `MEAN_SLOPE_CELLS` cells centred on
/// `(i, j)`, over those on the grid: the second pass, down the rows of `row_sums`.
fn mean_slope_deg(row_sums: &Grid<f32>, i: usize, j: usize) -> f32 {
    let (width, height) = (row_sums.width, row_sums.height);
    let (columns, rows) = (window(i, width), window(j, height));
    let count = (columns.len() * rows.len()) as f64;
    let total: f64 = rows.map(|y| f64::from(row_sums.data[y * width + i])).sum();
    (total / count) as f32
}

/// Everything the four shapes read, taken from the heights as they were on entry.
struct Shapes<'a> {
    analysis: &'a Analysis,
    rules: &'a GroundRules,
    amplitude_m: f32,
    seed: u64,
    rough_seed: u64,
    warp_seed: u64,
    strata_seed: u64,
    mask_seed: u64,
    gully_depth_seed: u64,
    /// Each cell's rise over run.
    rise: Grid<f32>,
    /// `row_slope_sums` of `rise`; `None` when gullies are off or `gully_min_slope_deg` is 0, which
    /// switches the fade off.
    row_slope_sums: Option<Grid<f32>>,
    /// `log10` of `gully_flow_start_m2` and of `gully_flow_full_m2`.
    gully_flow_log10: (f32, f32),
}

impl<'a> Shapes<'a> {
    fn new(heights: &Grid<f32>, analysis: &'a Analysis, seed: u64, amplitude_m: f32, rules: &'a GroundRules) -> Self {
        let (width, cell) = (heights.width, heights.cell_size);
        let mut rise = Grid::new(width, heights.height, cell);
        rise.data.par_chunks_mut(width).enumerate().for_each(|(j, row)| {
            for (i, value) in row.iter_mut().enumerate() {
                *value = slope_at(heights, i, j);
            }
        });
        let row_slope_sums = (rules.gully_depth_m > 0.0 && rules.gully_min_slope_deg > 0.0).then(|| row_slope_sums(&rise));
        Self {
            analysis,
            rules,
            amplitude_m,
            seed,
            rough_seed: mix64(seed ^ ROUGH_TAG),
            warp_seed: mix64(seed ^ WARP_TAG),
            strata_seed: mix64(seed ^ STRATA_TAG),
            mask_seed: mix64(seed ^ MASK_TAG),
            gully_depth_seed: mix64(seed ^ GULLY_DEPTH_TAG),
            rise,
            row_slope_sums,
            gully_flow_log10: (log10_m2(rules.gully_flow_start_m2), log10_m2(rules.gully_flow_full_m2)),
        }
    }

    /// The clamped sum of the shapes at cell `(i, j)`, whose height on entry is `h`, the gully's
    /// cut multiplied by `gully_factor` when it is given; `None` when no shape touches the cell.
    fn offset(&self, i: usize, j: usize, h: f32, gully_factor: Option<f32>) -> Option<f32> {
        let rules = self.rules;
        let cell = self.rise.cell_size;
        // The noise is laid on cell corners, as the hummocks always were; the coarse analysis is
        // read at cell middles, as the cover reads it.
        let (x, z) = (i as f64 * f64::from(cell), j as f64 * f64::from(cell));
        let middle = cell_middle(i, j, cell);
        let rise = self.rise.data[j * self.rise.width + i];
        // The slope in degrees, as `slope_deg` computes it, only when a shape reads it.
        let reads_slope = rules.rough_amplitude_m > 0.0 || rules.ledge_height_m > 0.0;
        let s = if reads_slope { rise.atan().to_degrees() } else { 0.0 };
        let shapes = [
            self.hummock(rise, x, z),
            self.roughness(s, x, z, middle),
            self.ledge(s, h, x, z),
            self.gully(i, j, x, z, middle, gully_factor),
        ];
        let total = shapes.into_iter().flatten().reduce(|total, offset| total + offset)?;
        Some(total.clamp(-rules.detail_limit_m, rules.detail_limit_m))
    }

    /// The hummocks, exactly as the detail stage computed them before the other shapes.
    fn hummock(&self, rise: f32, x: f64, z: f64) -> Option<f32> {
        let fade = ramp(FADE_START, FADE_END, rise);
        (fade > 0.0).then(|| self.amplitude_m * fbm(self.seed, x, z, OCTAVES, WAVELENGTH_M) * fade)
    }

    fn roughness(&self, s: f32, x: f64, z: f64, (x_m, z_m): (f32, f32)) -> Option<f32> {
        let rules = self.rules;
        if rules.rough_amplitude_m == 0.0 {
            return None;
        }
        let r = ramp(rules.rock_slope_start_deg, rules.rock_slope_full_deg, s).max(self.analysis.deposit_at(x_m, z_m));
        (r > 0.0).then(|| {
            rules.rough_amplitude_m * r * fbm(self.rough_seed, x, z, ROUGH_OCTAVES, f64::from(rules.rough_size_m))
        })
    }

    fn ledge(&self, s: f32, h: f32, x: f64, z: f64) -> Option<f32> {
        let rules = self.rules;
        if rules.ledge_height_m == 0.0 {
            return None;
        }
        let t = ramp(rules.ledge_slope_start_deg, rules.ledge_slope_full_deg, s);
        if t == 0.0 {
            return None;
        }
        let mask = ledge_mask(self.mask_seed, x, z);
        if mask == 0.0 {
            return None;
        }
        let spacing = rules.ledge_spacing_m;
        let warp = spacing * fbm(self.warp_seed, x, z, WARP_OCTAVES, WARP_WAVELENGTH_M);
        let level = f64::from(h) + f64::from(warp);
        let strata = widened(fbm(self.strata_seed, level, 0.0, STRATA_OCTAVES, f64::from(spacing)));
        Some(rules.ledge_height_m * t * mask * strata)
    }

    /// The gully's cut, times `factor` when it is given; a factor of 0 cuts nothing, as a
    /// `gully_depth_m` of 0.
    fn gully(&self, i: usize, j: usize, x: f64, z: f64, (x_m, z_m): (f32, f32), factor: Option<f32>) -> Option<f32> {
        let rules = self.rules;
        if rules.gully_depth_m == 0.0 || factor == Some(0.0) {
            return None;
        }
        // The flow first: it is 0 on most cells, and the mean slope is read only where water runs.
        let (start, full) = self.gully_flow_log10;
        let g = ramp(start, full, self.analysis.flow_log10_at(x_m, z_m));
        if g == 0.0 {
            return None;
        }
        // With no fade asked for, `k` is 1 everywhere, as `ramp(0.0, 0.0, mean)` would give.
        let k = self.row_slope_sums.as_ref().map_or(1.0, |sums| ramp(0.0, rules.gully_min_slope_deg, mean_slope_deg(sums, i, j)));
        if k == 0.0 {
            return None;
        }
        let noise = fbm(self.gully_depth_seed, x, z, GULLY_DEPTH_OCTAVES, GULLY_DEPTH_WAVELENGTH_M);
        let v = GULLY_DEPTH_MEAN + GULLY_DEPTH_SWING * widened(noise);
        let cut = -rules.gully_depth_m * g * k * v;
        Some(factor.map_or(cut, |factor| cut * factor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::shapes::*;
    use crate::analysis::{analyse, slope_deg};

    /// The detail stage as it was before the gullies, ledges and roughness: the hummocks alone.
    fn old_add_detail(heights: &mut Grid<f32>, seed: u64, amplitude_m: f32) {
        let width = heights.width;
        if width == 0 || heights.height == 0 {
            return;
        }
        let cell = f64::from(heights.cell_size);
        let fades: Vec<f32> = {
            let source = &*heights;
            (0..source.height)
                .into_par_iter()
                .flat_map_iter(|j| (0..width).map(move |i| ramp(0.02, 0.15, slope_at(source, i, j))))
                .collect()
        };
        heights.data.par_chunks_mut(width).enumerate().for_each(|(j, row)| {
            for (i, h) in row.iter_mut().enumerate() {
                let fade = fades[j * width + i];
                if fade > 0.0 {
                    let noise = fbm(seed, i as f64 * cell, j as f64 * cell, 4, 12.0);
                    *h += amplitude_m * noise * fade;
                }
            }
        });
    }

    /// Cells left out at each edge of the interior.
    const EDGE: usize = 16;
    /// A hummock amplitude small enough to leave the other shapes alone in a test.
    const NEGLIGIBLE: f32 = 1e-6;

    /// The heights after `add_detail` with seed 1, analysed with `rules` first.
    fn detailed(heights: &Grid<f32>, amplitude: f32, rules: &GroundRules) -> Grid<f32> {
        let analysis = analyse(heights, rules);
        let mut after = heights.clone();
        add_detail(&mut after, &analysis, 1, amplitude, rules);
        after
    }

    /// What `add_detail` with seed 1 moves each cell by: new height minus old.
    fn detail_of(heights: &Grid<f32>, amplitude: f32, rules: &GroundRules) -> Grid<f32> {
        let mut offset = detailed(heights, amplitude, rules);
        for (new, old) in offset.data.iter_mut().zip(&heights.data) {
            *new -= old;
        }
        offset
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Shape {
        Gullies,
        Ledges,
        Roughness,
    }

    /// The default rules with every shape's amount but `shape`'s set to 0; used with an amplitude
    /// of `NEGLIGIBLE`, so the hummocks are negligible too.
    fn only(shape: Shape) -> GroundRules {
        let defaults = GroundRules::default();
        let amount = |of: Shape, value: f32| if of == shape { value } else { 0.0 };
        GroundRules {
            gully_depth_m: amount(Shape::Gullies, defaults.gully_depth_m),
            ledge_height_m: amount(Shape::Ledges, defaults.ledge_height_m),
            rough_amplitude_m: amount(Shape::Roughness, defaults.rough_amplitude_m),
            ..defaults
        }
    }

    /// Rising at 35 degrees toward larger `x`: tan 35 = 0.70021.
    fn ramp_35() -> Grid<f32> {
        shape(256, |x, _| 0.700_21 * x)
    }

    /// Every cell of a 256 x 256 grid but the `EDGE` cells at each edge.
    fn interior() -> impl Iterator<Item = (usize, usize)> {
        (EDGE..256 - EDGE).flat_map(|z| (EDGE..256 - EDGE).map(move |x| (x, z)))
    }

    /// Every cell of a 256 x 256 grid whose `x` is in `columns`.
    fn columns(columns: impl Iterator<Item = usize> + Clone) -> impl Iterator<Item = (usize, usize)> {
        (0..256).flat_map(move |z| columns.clone().map(move |x| (x, z)))
    }

    /// The mean and the standard deviation of `grid` over `cells`.
    fn mean_and_deviation(grid: &Grid<f32>, cells: impl Iterator<Item = (usize, usize)>) -> (f64, f64) {
        let values: Vec<f64> = cells.map(|(x, z)| f64::from(grid.get(x, z))).collect();
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
        (mean, variance.sqrt())
    }

    fn bits(grid: &Grid<f32>) -> Vec<u32> {
        grid.data.iter().map(|h| h.to_bits()).collect()
    }

    #[test]
    fn zero_amplitude_leaves_the_ground_alone() {
        for (name, heights) in [("valley", valley()), ("cliff", cliff())] {
            let after = detailed(&heights, 0.0, &GroundRules::default());
            let differing = bits(&after).iter().zip(bits(&heights)).filter(|(a, b)| *a != b).count();
            assert_eq!(differing, 0, "{name}: {differing} heights changed");
        }
    }

    #[test]
    fn with_the_new_shapes_at_zero_the_detail_is_the_old_noise() {
        let rules = GroundRules { gully_depth_m: 0.0, ledge_height_m: 0.0, rough_amplitude_m: 0.0, ..Default::default() };
        for (name, heights) in [("valley", valley()), ("cliff", cliff()), ("60 degree ramp", ramp_60())] {
            let after = detailed(&heights, 0.25, &rules);
            let mut old = heights.clone();
            old_add_detail(&mut old, 1, 0.25);
            let differing = bits(&after).iter().zip(bits(&old)).filter(|(a, b)| *a != b).count();
            assert_eq!(differing, 0, "{name}: {differing} heights differ from the old noise");
            assert!(bits(&old) != bits(&heights), "{name}: the old noise moved nothing");
        }
    }

    #[test]
    fn a_flat_plain_stays_flat() {
        let offset = detail_of(&plain(), 0.25, &GroundRules::default());
        let moved: Vec<(usize, f32)> = offset.data.iter().copied().enumerate().filter(|(_, o)| *o != 0.0).collect();
        assert!(moved.is_empty(), "{} cells moved, the first (index, offset) {:?}", moved.len(), &moved[..moved.len().min(5)]);
    }

    #[test]
    fn no_cell_moves_further_than_the_limit() {
        let rules = GroundRules {
            detail_limit_m: 0.5,
            gully_depth_m: 5.0,
            ledge_height_m: 5.0,
            rough_amplitude_m: 5.0,
            ..Default::default()
        };
        for (name, heights) in [("valley", valley()), ("cliff", cliff()), ("60 degree ramp", ramp_60())] {
            let offset = detail_of(&heights, 2.0, &rules);
            let furthest = offset.data.iter().map(|o| o.abs()).fold(0.0f32, f32::max);
            assert!(furthest <= 0.5 + 1e-4, "{name}: a cell moved {furthest}");
            assert!(furthest > 0.45, "{name}: the furthest a cell moved is {furthest}, so the limit was never reached");
        }
    }

    #[test]
    fn a_stream_cuts_a_gully() {
        let rules = only(Shape::Gullies);
        let offset = detail_of(&valley(), NEGLIGIBLE, &rules);
        let floor = (160..224).flat_map(|z| (126..=133).map(move |x| (x, z)));
        let (cut, _) = mean_and_deviation(&offset, floor);
        // The floor's smoothed flow, about 15,000 to 21,000 m², is far below `gully_flow_full_m2`:
        // the cut measures -0.36 m, under a third of the depth; the bound is half of that.
        assert!(cut < -0.18, "the valley floor moved {cut} on average");
        assert!(cut >= -f64::from(rules.gully_depth_m), "the valley floor moved {cut} on average, deeper than the gully depth");
        let side = (0..256).map(|z| f64::from(offset.get(40, z).abs())).sum::<f64>() / 256.0;
        assert!(side < 0.01, "the valley side at x = 40 moved {side} on average");
    }

    /// The bed of `valley()` after `add_detail`, row by row for `rows`: the lowest height among the
    /// columns 124 to 135.
    fn valley_bed(rules: &GroundRules, rows: Range<usize>) -> Vec<f32> {
        let after = detailed(&valley(), NEGLIGIBLE, rules);
        rows.map(|z| (124..=135).map(|x| after.get(x, z)).fold(f32::INFINITY, f32::min)).collect()
    }

    /// The steps from one row of `bed` to the next that rise by more than `rise_m`, and the
    /// steps that fall by less than `fall_m`; a rise counts as a fall of less than any `fall_m`.
    fn weak_steps(bed: &[f32], rise_m: f32, fall_m: f32) -> (usize, usize) {
        let steps = || bed.windows(2).map(|pair| pair[1] - pair[0]);
        (steps().filter(|&step| step > rise_m).count(), steps().filter(|&step| step > -fall_m).count())
    }

    #[test]
    fn a_gully_keeps_the_streams_fall() {
        // The valley's floor falls 0.02 m per metre toward larger z, and the bed with it. Going
        // downstream the bed may not climb by more than that from one row to the next, and it
        // must keep falling by at least 0.008 m a row, 0.4 of the floor's fall: a gully whose
        // depth varies too much along the stream almost levels the bed where it shallows.
        let bed = valley_bed(&only(Shape::Gullies), 100..224);
        let (rising, slow) = weak_steps(&bed, 0.02, 0.008);
        assert_eq!(rising, 0, "the bed rises by more than 0.02 m at {rising} of {} steps", bed.len() - 1);
        assert_eq!(slow, 0, "the bed falls by less than 0.008 m at {slow} of {} steps", bed.len() - 1);
    }

    #[test]
    fn the_gully_fade_still_works_when_asked_for() {
        // The valley's floor lies in ground whose mean slope is about 7 degrees: a fade up to 30
        // degrees cuts it much less than no fade.
        let cut = |min_slope: f32| {
            let rules = GroundRules { gully_min_slope_deg: min_slope, ..only(Shape::Gullies) };
            let floor = (160..224).flat_map(|z| (126..=133).map(move |x| (x, z)));
            mean_and_deviation(&detail_of(&valley(), NEGLIGIBLE, &rules), floor).0
        };
        let (faded, unfaded) = (cut(30.0), cut(0.0));
        assert!(unfaded < -0.18, "with no fade the floor moved {unfaded} on average");
        assert!(faded > unfaded * 0.75, "with a fade up to 30 degrees the floor moved {faded} on average, against {unfaded} with none");
    }

    /// The heights after `add_detail_faded` with seed 1 and `gully_factor`, analysed with `rules`
    /// first.
    fn faded(heights: &Grid<f32>, amplitude: f32, rules: &GroundRules, gully_factor: &[f32]) -> Grid<f32> {
        let analysis = analyse(heights, rules);
        let mut after = heights.clone();
        add_detail_faded(&mut after, &analysis, 1, amplitude, rules, Some(gully_factor));
        after
    }

    /// A valley along `x = 128` whose sides rise at 40 degrees (tan 40 = 0.83910) and whose floor
    /// falls 0.02 m per metre toward larger `z`: beside its stream the ground is steep enough for
    /// ledges and rock.
    fn steep_valley() -> Grid<f32> {
        shape(256, |x, z| 0.839_1 * (x - 128.0).abs() + 0.02 * (255.0 - z))
    }

    #[test]
    fn a_gully_factor_of_one_everywhere_is_no_factor() {
        let rules = GroundRules::default();
        for (name, heights) in [("valley", valley()), ("steep valley", steep_valley()), ("cliff", cliff()), ("60 degree ramp", ramp_60())] {
            let ones = vec![1.0; heights.data.len()];
            let (with, without) = (faded(&heights, 0.25, &rules, &ones), detailed(&heights, 0.25, &rules));
            let differing = bits(&with).iter().zip(bits(&without)).filter(|(a, b)| **a != *b).count();
            assert_eq!(differing, 0, "{name}: {differing} heights differ from the detail without a factor");
            assert!(bits(&without) != bits(&heights), "{name}: the detail moved nothing");
        }
    }

    /// Where the detail cuts no gully, with an amplitude of 0 or a `gully_depth_m` of 0, a gully
    /// factor changes nothing: the heights are those without one, bit for bit, whatever its values,
    /// so `cuts_gullies` says none need be built; with the default rules and an amplitude it says
    /// one does.
    #[test]
    fn where_no_gully_is_cut_a_gully_factor_changes_nothing() {
        let rules = GroundRules::default();
        let ungullied = GroundRules { gully_depth_m: 0.0, ..rules };
        assert!(cuts_gullies(0.25, &rules));
        for (name, heights) in [("valley", valley()), ("steep valley", steep_valley())] {
            let zeros = vec![0.0; heights.data.len()];
            for (case, amplitude, rules) in [("no detail", 0.0, &rules), ("no gully", 0.25, &ungullied)] {
                assert!(!cuts_gullies(amplitude, rules), "{name}, {case}: gullies are cut");
                let (with, without) = (faded(&heights, amplitude, rules, &zeros), detailed(&heights, amplitude, rules));
                let differing = bits(&with).iter().zip(bits(&without)).filter(|(a, b)| **a != *b).count();
                assert_eq!(differing, 0, "{name}, {case}: {differing} heights differ from the detail without a factor");
            }
            let gullied = (faded(&heights, 0.25, &rules, &zeros), detailed(&heights, 0.25, &rules));
            assert!(bits(&gullied.0) != bits(&gullied.1), "{name}: with gullies cut, a factor of 0 changes nothing");
        }
    }

    #[test]
    fn a_gully_factor_of_0_leaves_a_stream_uncut_and_the_other_shapes_as_they_were() {
        let rules = GroundRules::default();
        let heights = steep_valley();
        // The stream's cells: those the gully alone cuts by more than a millimetre.
        let stream: Vec<bool> = detail_of(&heights, NEGLIGIBLE, &only(Shape::Gullies)).data.iter().map(|&cut| cut < -1e-3).collect();
        let streams = stream.iter().filter(|&&on| on).count();
        assert!(streams >= 1000, "only {streams} cells of the stream");
        let factor: Vec<f32> = stream.iter().map(|&on| if on { 0.0 } else { 1.0 }).collect();
        let after = faded(&heights, 0.25, &rules, &factor);
        let uncut = detailed(&heights, 0.25, &GroundRules { gully_depth_m: 0.0, ..rules });
        let whole = detailed(&heights, 0.25, &rules);
        // On the stream the ground is the other three shapes' alone, bit for bit, as with no gullies
        // at all; elsewhere it is the whole detail's.
        let (mut differing, mut cut) = (Vec::new(), 0);
        for (cell, &on) in stream.iter().enumerate() {
            let expected = if on { uncut.data[cell] } else { whole.data[cell] };
            if after.data[cell].to_bits() != expected.to_bits() {
                differing.push((cell % 256, cell / 256, after.data[cell], expected));
            }
            cut += usize::from(on && whole.data[cell] < uncut.data[cell]);
        }
        assert!(differing.is_empty(), "{} cells differ, the first (x, z, height, expected) {:?}", differing.len(), &differing[..differing.len().min(5)]);
        assert!(cut * 2 >= streams, "the whole detail cuts only {cut} of the {streams} cells of the stream");
        // Each of the other three shapes moves cells of the stream, so the test sees them kept.
        let hummocks = GroundRules { gully_depth_m: 0.0, ledge_height_m: 0.0, rough_amplitude_m: 0.0, ..rules };
        for (name, offset) in [
            ("hummocks", detail_of(&heights, 0.25, &hummocks)),
            ("roughness", detail_of(&heights, NEGLIGIBLE, &only(Shape::Roughness))),
            ("ledges", detail_of(&heights, NEGLIGIBLE, &only(Shape::Ledges))),
        ] {
            let moved = (0..stream.len()).filter(|&cell| stream[cell] && offset.data[cell].abs() > 1e-3).count();
            assert!(moved * 10 >= streams, "the {name} move only {moved} of the {streams} cells of the stream");
        }
    }

    #[test]
    fn steep_ground_gets_ledges() {
        let rules = only(Shape::Ledges);
        let after = detailed(&ramp_35(), NEGLIGIBLE, &rules);
        let (_, spread) = mean_and_deviation(&slope_deg(&after), interior());
        assert!(spread >= 2.0, "the interior's slopes have a standard deviation of {spread} degrees");
        let offset = detail_of(&ramp_35(), NEGLIGIBLE, &rules);
        let sizes: Vec<f32> = interior().map(|(x, z)| offset.get(x, z).abs()).collect();
        let share = |keep: &dyn Fn(f32) -> bool| sizes.iter().filter(|&&o| keep(o)).count() as f64 / sizes.len() as f64;
        let (bare, ledged) = (share(&|o| o < 0.02), share(&|o| o > 0.1));
        assert!(bare >= 0.25, "{bare} of the interior moved less than 0.02 m: the ledges are not patchy");
        assert!(ledged >= 0.15, "{ledged} of the interior moved more than 0.1 m");
        // Where the strata are clamped and the mask is full a ledge moves the ground by exactly
        // `ledge_height_m`; the offset, read back as new height minus old near 180 m, carries the
        // rounding of `f32` there (a few millionths), as in `no_cell_moves_further_than_the_limit`.
        let furthest = offset.data.iter().map(|o| o.abs()).fold(0.0f32, f32::max);
        assert!(furthest <= rules.ledge_height_m + 1e-4, "a cell moved {furthest}, more than the ledge height");
        let (mean, _) = mean_and_deviation(&offset, interior());
        assert!(mean.abs() <= 0.1, "the interior moved {mean} on average");

        let gentle_offset = detail_of(&ramp_8(), NEGLIGIBLE, &rules);
        let furthest = gentle_offset.data.iter().map(|o| o.abs()).fold(0.0f32, f32::max);
        assert!(furthest < 1e-3, "a cell of the 8 degree ramp moved {furthest}");
    }

    /// The ledges count as on where their mask is at least this.
    const MASK_ON: f32 = 0.5;
    /// Offsets closer than this, in metres, count as level when a crest is found.
    const LEVEL_M: f32 = 1e-3;

    /// The crests of the offsets along `row`, as positions in cells. A crest is a run of one or
    /// more cells within `LEVEL_M` of the run's first cell, higher by more than `LEVEL_M` than the
    /// cells just before and just after it; it stands at the run's middle. So a flat top, where
    /// the strata are clamped and only rounding moves the offset, is one crest, not several.
    fn crests(row: &[f32]) -> Vec<f64> {
        let mut crests = Vec::new();
        let mut k = 1;
        while k + 1 < row.len() {
            if row[k] <= row[k - 1] + LEVEL_M {
                k += 1;
                continue;
            }
            let mut end = k;
            while end + 1 < row.len() && (row[end + 1] - row[k]).abs() <= LEVEL_M {
                end += 1;
            }
            if end + 1 < row.len() && row[end + 1] < row[k] - LEVEL_M {
                crests.push((k + end) as f64 / 2.0);
            }
            k = end + 1;
        }
        crests
    }

    #[test]
    fn ledges_are_not_evenly_spaced() {
        // Pooled over every stretch of an interior row where the ledge mask of stage seed 1 is at
        // least `MASK_ON`: one row holds too few ledges for a stable figure.
        let offset = detail_of(&ramp_35(), NEGLIGIBLE, &only(Shape::Ledges));
        let mask_seed = mix64(1 ^ MASK_TAG);
        let columns: Vec<usize> = (EDGE..256 - EDGE).collect();
        let mut gaps: Vec<f64> = Vec::new();
        for z in EDGE..256 - EDGE {
            for stretch in columns.split(|&x| ledge_mask(mask_seed, x as f64, z as f64) < MASK_ON) {
                let row: Vec<f32> = stretch.iter().map(|&x| offset.get(x, z)).collect();
                gaps.extend(crests(&row).windows(2).map(|pair| pair[1] - pair[0]));
            }
        }
        assert!(gaps.len() >= 100, "only {} gaps between ledges where the mask is on", gaps.len());
        let mean = gaps.iter().sum::<f64>() / gaps.len() as f64;
        let deviation = (gaps.iter().map(|g| (g - mean).powi(2)).sum::<f64>() / gaps.len() as f64).sqrt();
        let variation = deviation / mean;
        assert!(variation >= 0.25, "{} gaps between ledges, {mean} cells on average, vary by only {variation} of their mean", gaps.len());
    }

    #[test]
    fn rock_and_loose_ground_are_rough() {
        let rules = only(Shape::Roughness);
        let (_, below_face) = mean_and_deviation(&detail_of(&cliff(), NEGLIGIBLE, &rules), columns(136..144));
        assert!(below_face > 0.03, "the loose ground below the face has a standard deviation of {below_face}");
        let (_, rock) = mean_and_deviation(&detail_of(&ramp_60(), NEGLIGIBLE, &rules), interior());
        assert!(rock > 0.05, "the 60 degree ramp has a standard deviation of {rock}");
        let (_, meadow) = mean_and_deviation(&detail_of(&ramp_8(), NEGLIGIBLE, &rules), columns(0..256));
        assert!(meadow < 1e-4, "the 8 degree ramp has a standard deviation of {meadow}");
    }

    #[test]
    fn a_world_of_66_cells_takes_detail() {
        let heights = steepening(66);
        let after = detailed(&heights, 0.25, &GroundRules::default());
        let bad = after.data.iter().filter(|h| !h.is_finite()).count();
        assert_eq!(bad, 0, "{bad} heights are not finite");
        assert!(after.data != heights.data, "nothing moved");
    }

    #[test]
    fn detail_is_bounded_on_slopes() {
        let g = shape(128, |x, _| 0.3 * x);
        let offset = detail_of(&g, 0.25, &GroundRules::default());
        let max_change = offset.data.iter().map(|o| o.abs()).fold(0.0f32, f32::max);
        assert!(max_change <= 0.25, "max change {max_change}");
        assert!(max_change > 0.025, "max change {max_change}");
    }
}
