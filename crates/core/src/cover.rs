use crate::analysis::{cell_middle, gradient_at, log10_m2, ramp, sample, Analysis};
use crate::grid::Grid;
use crate::noise::fbm;
use crate::rng::mix64;
use crate::rules::GroundRules;
use crate::water::{WaterField, WATER_LAYER};
use rayon::prelude::*;
use std::collections::BTreeMap;

/// How far the three patch fields move their inputs at `patch_strength` 1: the slope in degrees,
/// the ridge height in metres and the wetness.
const PATCH_SLOPE_DEG: f32 = 12.0;
const PATCH_RIDGE_M: f32 = 3.0;
const PATCH_WETNESS: f32 = 0.5;
/// Wavelength of the finest patch field; the others are `patch_size_m` and a third of it.
const PATCH_FINE_M: f64 = 6.0;
/// Octaves of the large, medium and fine patch fields.
const PATCH_OCTAVES: [u32; 3] = [3, 2, 1];
/// Xored into the cover seed to give each patch field its own seed.
const PATCH_TAGS: [u64; 3] = [1, 2, 3];
/// Wet ground fades out over this many degrees up to `wet_slope_limit_deg`.
const WET_SLOPE_FADE_DEG: f32 = 4.0;
/// Vigour is `VIGOUR_BASE`, raised by wetness, lowered on ridges (raised in hollows) and on steep
/// slopes facing the sun, plus the large patch field; each term at most its weight here.
const VIGOUR_BASE: f32 = 0.5;
const VIGOUR_WET: f32 = 0.5;
const VIGOUR_RIDGE: f32 = 0.25;
const VIGOUR_SUN: f32 = 0.25;
const VIGOUR_PATCH: f32 = 0.3;
/// The largest `blend`: the second material's share stays below a half, even when the two
/// weights are equal.
const MAX_BLEND: f32 = 127.0;
/// A cell under more water than this, in metres, is wet ground alone.
const UNDER_WATER_M: f32 = 0.05;

/// The ground of a cell under `depth_m` of water where the water alone decides it: wet ground
/// alone under more than `UNDER_WATER_M`, and `None` elsewhere, where the cover's weights decide.
/// The cover reads it on every cell of a drawn world, and an edit's joined water on its band's
/// cells (`land::water::join_water`).
pub(crate) fn ground_under_water(depth_m: f32) -> Option<GroundCell> {
    (depth_m > UNDER_WATER_M).then_some(GroundCell { strongest: Material::WetGround as u8, second: Material::WetGround as u8, blend: 0 })
}

/// What lies on the ground of a cell; the discriminant is the index a `GroundCell` stores.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    MeadowGrass = 0,
    DryGrass = 1,
    BareSoil = 2,
    Rock = 3,
    Scree = 4,
    WetGround = 5,
}

impl Material {
    /// Every material, in index order.
    pub const ALL: [Material; 6] =
        [Material::MeadowGrass, Material::DryGrass, Material::BareSoil, Material::Rock, Material::Scree, Material::WetGround];

    /// "meadow_grass", "dry_grass", "bare_soil", "rock", "scree", "wet_ground"
    pub fn name(self) -> &'static str {
        match self {
            Material::MeadowGrass => "meadow_grass",
            Material::DryGrass => "dry_grass",
            Material::BareSoil => "bare_soil",
            Material::Rock => "rock",
            Material::Scree => "scree",
            Material::WetGround => "wet_ground",
        }
    }

    /// False colour for pictures: (86,125,70) (160,150,90) (120,85,60) (130,130,130) (170,165,150) (50,80,90)
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Material::MeadowGrass => [86, 125, 70],
            Material::DryGrass => [160, 150, 90],
            Material::BareSoil => [120, 85, 60],
            Material::Rock => [130, 130, 130],
            Material::Scree => [170, 165, 150],
            Material::WetGround => [50, 80, 90],
        }
    }
}

/// One cell of the ground map: the indices of its strongest and second strongest `Material`.
/// `blend / 255` is the second material's share, so `blend` is at most 127; 0 means pure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GroundCell {
    pub strongest: u8,
    pub second: u8,
    pub blend: u8,
}

/// The cover of a world, one value per world cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cover {
    pub ground: Grid<GroundCell>,
    /// 0 dry and sparse to 1 lush.
    pub vigour: Grid<f32>,
}

/// The three shared patch fields, each `fbm` in -1..1 with its own seed: large (`patch_size_m`)
/// moves the slope, medium (`patch_size_m / 3`) the ridge height, fine (`PATCH_FINE_M`) the
/// wetness. Vigour also reads the large one.
struct PatchFields {
    seeds: [u64; 3],
    wavelengths_m: [f64; 3],
}

impl PatchFields {
    fn new(seed: u64, patch_size_m: f32) -> Self {
        let size = f64::from(patch_size_m);
        Self { seeds: PATCH_TAGS.map(|tag| mix64(seed ^ tag)), wavelengths_m: [size, size / 3.0, PATCH_FINE_M] }
    }

    /// The large, medium and fine fields at a point in metres.
    fn at(&self, x_m: f64, z_m: f64) -> [f32; 3] {
        [0, 1, 2].map(|k| fbm(self.seeds[k], x_m, z_m, PATCH_OCTAVES[k], self.wavelengths_m[k]))
    }
}

/// What one cell's weights and vigour are taken from, after patch noise.
struct Inputs {
    slope_deg: f32,
    /// Above the 48 m neighbourhood mean, `Analysis::ridge_m`.
    ridge_m: f32,
    /// 0 to 1.
    wetness: f32,
    flow_log10: f32,
    deposit: f32,
    /// The downhill direction's component pointing away from north, -1 to 1; 0 on flat ground.
    sun: f32,
}

/// The downhill direction at `(i, j)`, as its component pointing away from `north`.
fn sun_at(heights: &Grid<f32>, i: usize, j: usize, north: (f32, f32)) -> f32 {
    // Downhill is minus the gradient and away from north is minus north, so the signs cancel.
    let (gx, gz) = gradient_at(heights, i, j);
    let length = (gx * gx + gz * gz).sqrt();
    if length > 0.0 {
        ((gx * north.0 + gz * north.1) / length).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Each material's weight, in index order.
fn weights(here: &Inputs, rules: &GroundRules) -> [f32; 6] {
    let s = here.slope_deg;
    let rock = ramp(rules.rock_slope_start_deg, rules.rock_slope_full_deg, s);
    let soil = ramp(rules.soil_slope_start_deg, rules.soil_slope_full_deg, s) * (1.0 - rock);
    let scree = here.deposit * (1.0 - rock);
    let flow = ramp(log10_m2(rules.wet_flow_start_m2), log10_m2(rules.wet_flow_full_m2), here.flow_log10);
    let wet_slope = ramp(rules.wet_slope_limit_deg - WET_SLOPE_FADE_DEG, rules.wet_slope_limit_deg, s);
    let wet = flow.max(ramp(rules.wet_start, rules.wet_full, here.wetness)) * (1.0 - wet_slope);
    let ridge = ramp(rules.dry_ridge_start_m, rules.dry_ridge_full_m, here.ridge_m);
    let sunny = here.sun.max(0.0) * ramp(rules.dry_sun_slope_start_deg, rules.dry_sun_slope_full_deg, s);
    // Thin grass yields to rock, as soil and scree do.
    let dry = ridge.max(sunny) * (1.0 - here.wetness) * (1.0 - rock);
    // Meadow yields to bare soil, so the soil to rock border blends soil with rock, not meadow.
    let meadow = rules.meadow_weight * (1.0 - ramp(rules.soil_slope_start_deg, rules.soil_slope_full_deg, s));
    [meadow, dry, soil, rock, scree, wet]
}

/// The strongest and second strongest weight, ties to the lower index, and the blend: the second
/// starts to blend in once its weight comes within `transition_width` of the strongest's. A width
/// of 0 never blends.
fn ground_cell(weights: &[f32; 6], transition_width: f32) -> GroundCell {
    let mut strongest = 0;
    for m in 1..weights.len() {
        if weights[m] > weights[strongest] {
            strongest = m;
        }
    }
    let mut second = usize::from(strongest == 0);
    for m in 0..weights.len() {
        if m != strongest && weights[m] > weights[second] {
            second = m;
        }
    }
    let (w1, w2) = (weights[strongest], weights[second]);
    let blend = if transition_width > 0.0 && w1 > 0.0 {
        let share = 0.5 * ((w2 / w1 - (1.0 - transition_width)) / transition_width).clamp(0.0, 1.0);
        (share * 255.0).round().min(MAX_BLEND) as u8
    } else {
        0
    };
    GroundCell { strongest: strongest as u8, second: second as u8, blend }
}

/// High in hollows and on wet ground, low on ridges and on steep slopes facing the sun, moved by
/// the large patch field so a plain is not uniform; 0 to 1.
fn vigour_at(here: &Inputs, large_patch: f32, rules: &GroundRules) -> f32 {
    let (start, full) = (rules.dry_ridge_start_m, rules.dry_ridge_full_m);
    // 1 on a ridge, -1 in a hollow as deep as a ridge is high.
    let ridge = ramp(start, full, here.ridge_m) - ramp(start, full, -here.ridge_m);
    let sunny = here.sun.max(0.0) * ramp(rules.dry_sun_slope_start_deg, rules.dry_sun_slope_full_deg, here.slope_deg);
    let vigour = VIGOUR_BASE + VIGOUR_WET * here.wetness - VIGOUR_RIDGE * ridge - VIGOUR_SUN * sunny
        + VIGOUR_PATCH * large_patch;
    vigour.clamp(0.0, 1.0)
}

/// The ground map and vigour of every world cell. Each material's weight comes from the final
/// slope and the analysis read at the cell's middle, after the shared patch fields of `seed` have
/// moved them; the cell keeps the two strongest. Rows run in parallel and a cell reads only shared
/// inputs, so the result does not depend on threads.
///
/// `heights` gives the downhill direction, `slope_deg` its steepness; both are of the final surface.
/// With `water`, of a drawn world, a cell under more than `UNDER_WATER_M` of water is wet ground
/// alone, and elsewhere the wetness read from the analysis is no less than how wet the water makes
/// the cell (`WaterField::bank_wet`).
pub(crate) fn cover(
    heights: &Grid<f32>,
    slope_deg: &Grid<f32>,
    analysis: &Analysis,
    north: (f32, f32),
    seed: u64,
    rules: &GroundRules,
    water: Option<&WaterField>,
) -> Cover {
    let (width, height, cell) = (heights.width, heights.height, heights.cell_size);
    let fields = PatchFields::new(seed, rules.patch_size_m);
    let strength = rules.patch_strength;
    let mut ground = Grid::new(width, height, cell);
    let mut vigour = Grid::new(width, height, cell);
    let rows = ground.data.par_chunks_mut(width.max(1)).zip(vigour.data.par_chunks_mut(width.max(1)));
    rows.enumerate().for_each(|(j, (ground_row, vigour_row))| {
        for (i, (ground_cell_out, vigour_out)) in ground_row.iter_mut().zip(vigour_row).enumerate() {
            let (x_m, z_m) = cell_middle(i, j, cell);
            let [large, medium, fine] = fields.at(f64::from(x_m), f64::from(z_m));
            let read = analysis.wetness_at(x_m, z_m);
            let wetness = water.map_or(read, |water| read.max(water.bank_wet.get(i, j))) + strength * PATCH_WETNESS * fine;
            let here = Inputs {
                slope_deg: slope_deg.get(i, j) + strength * PATCH_SLOPE_DEG * large,
                ridge_m: sample(&analysis.ridge_m, x_m, z_m) + strength * PATCH_RIDGE_M * medium,
                wetness: wetness.clamp(0.0, 1.0),
                flow_log10: analysis.flow_log10_at(x_m, z_m),
                deposit: analysis.deposit_at(x_m, z_m),
                sun: sun_at(heights, i, j, north),
            };
            *ground_cell_out = match water.and_then(|water| ground_under_water(water.depth_m.get(i, j))) {
                Some(wet_alone) => wet_alone,
                None => ground_cell(&weights(&here, rules), rules.transition_width),
            };
            *vigour_out = vigour_at(&here, large, rules);
        }
    });
    Cover { ground, vigour }
}

/// Sorted: "flow", "slope", "vigour", "wetness".
pub const LAYER_NAMES: [&str; 4] = ["flow", "slope", "vigour", "wetness"];

/// The value range a layer's picture is drawn over: flow 1.2..6.5, slope 0..60, vigour and wetness 0..1.
pub fn layer_range(name: &str) -> Option<(f32, f32)> {
    match name {
        "flow" => Some((1.2, 6.5)),
        "slope" => Some((0.0, 60.0)),
        "vigour" | "wetness" => Some((0.0, 1.0)),
        _ => None,
    }
}

/// A ground cell's picture colour: its two materials' `Material::rgb` mixed by the cell's share
/// of the second, `blend / 255`, as `round(a * (1 - share) + b * share)` per channel.
pub fn cell_rgb(cell: GroundCell) -> [u8; 3] {
    let strongest = Material::ALL[usize::from(cell.strongest)].rgb();
    let second = Material::ALL[usize::from(cell.second)].rgb();
    let share = f32::from(cell.blend) / 255.0;
    [0, 1, 2].map(|c| (f32::from(strongest[c]) * (1.0 - share) + f32::from(second[c]) * share).round() as u8)
}

/// The picture colour of a depth of the water layer, in metres: land, 0 or less (or not a number),
/// is (214, 208, 190); water goes from (120, 170, 220) just above 0 to (20, 60, 140) at 2 m and
/// deeper, linear in the depth and rounded per channel.
pub fn water_rgb(depth_m: f32) -> [u8; 3] {
    const LAND: [u8; 3] = [214, 208, 190];
    const SHALLOW: [u8; 3] = [120, 170, 220];
    const DEEP: [u8; 3] = [20, 60, 140];
    const DEEP_M: f32 = 2.0;
    if depth_m.is_nan() || depth_m <= 0.0 {
        return LAND;
    }
    let share = (depth_m / DEEP_M).min(1.0);
    [0, 1, 2].map(|c| (f32::from(SHALLOW[c]) + (f32::from(DEEP[c]) - f32::from(SHALLOW[c])) * share).round() as u8)
}

/// A layer value's picture grey over `(low, high)`, as `layer_range` gives it: `low` and below
/// are 0, `high` and above 255, linear between and rounded; a value that is not finite is 0.
pub fn value_grey(value: f32, (low, high): (f32, f32)) -> u8 {
    if value.is_finite() {
        (((value - low) / (high - low)).clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        0
    }
}

/// A grid the size of `like` holding `read` taken at each cell's middle.
fn at_cell_middles(like: &Grid<f32>, read: impl Fn(f32, f32) -> f32 + Sync) -> Grid<f32> {
    let mut grid = Grid::new(like.width, like.height, like.cell_size);
    grid.data.par_chunks_mut(like.width.max(1)).enumerate().for_each(|(j, row)| {
        for (i, value) in row.iter_mut().enumerate() {
            let (x_m, z_m) = cell_middle(i, j, like.cell_size);
            *value = read(x_m, z_m);
        }
    });
    grid
}

/// A world's layers at full size, named as in `LAYER_NAMES`: `slope_deg` and `vigour` as given,
/// `flow` as `log10` of the drained area and `wetness`, both the smoothed fields read from the
/// analysis at each cell's middle with the same readers as the cover stage. With `water`, of a
/// drawn world, `wetness` is no less than how wet the water makes the cell, as in the cover, and
/// a fifth layer, `WATER_LAYER`, holds the water's depth: the field's own grid, not a copy.
pub(crate) fn layers(
    slope_deg: Grid<f32>,
    analysis: &Analysis,
    vigour: Grid<f32>,
    water: Option<WaterField>,
) -> BTreeMap<String, Grid<f32>> {
    let flow = at_cell_middles(&slope_deg, |x_m, z_m| analysis.flow_log10_at(x_m, z_m));
    let mut wetness = at_cell_middles(&slope_deg, |x_m, z_m| analysis.wetness_at(x_m, z_m));
    // The bank's wetness is read here and dropped; the depth becomes the water layer.
    let depth_m = water.map(|WaterField { depth_m, bank_wet }| {
        for (value, &bank_wet) in wetness.data.iter_mut().zip(&bank_wet.data) {
            *value = value.max(bank_wet);
        }
        depth_m
    });
    let [flow_name, slope_name, vigour_name, wetness_name] = LAYER_NAMES;
    let mut layers = BTreeMap::from([
        (flow_name.to_string(), flow),
        (slope_name.to_string(), slope_deg),
        (vigour_name.to_string(), vigour),
        (wetness_name.to_string(), wetness),
    ]);
    if let Some(depth_m) = depth_m {
        layers.insert(WATER_LAYER.to_string(), depth_m);
    }
    layers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::shapes::*;
    use crate::analysis::{analyse, slope_deg};
    use crate::detail::add_detail;

    #[test]
    fn a_pure_cell_has_its_materials_colour() {
        let cell = GroundCell { strongest: Material::Rock as u8, second: Material::Scree as u8, blend: 0 };
        assert_eq!(cell_rgb(cell), [130, 130, 130]);
    }

    #[test]
    fn a_mixed_cell_is_the_rounded_per_channel_mix() {
        // Rock (130,130,130) and scree (170,165,150), blend 127: share = 127 / 255 = 0.498039.
        // red 130 * 0.501961 + 170 * 0.498039 = 149.92 -> 150
        // green 130 * 0.501961 + 165 * 0.498039 = 147.43 -> 147
        // blue 130 * 0.501961 + 150 * 0.498039 = 139.96 -> 140
        let cell = GroundCell { strongest: Material::Rock as u8, second: Material::Scree as u8, blend: 127 };
        assert_eq!(cell_rgb(cell), [150, 147, 140]);
    }

    #[test]
    fn a_layer_value_maps_linearly_over_its_range_and_clamps() {
        let range = (1.0, 5.0);
        // Both ends, the middle (0.5 * 255 = 127.5 rounds up), a quarter (63.75 -> 64), outside both ends and NaN.
        assert_eq!(value_grey(1.0, range), 0);
        assert_eq!(value_grey(5.0, range), 255);
        assert_eq!(value_grey(3.0, range), 128);
        assert_eq!(value_grey(2.0, range), 64);
        assert_eq!(value_grey(0.0, range), 0);
        assert_eq!(value_grey(9.0, range), 255);
        assert_eq!(value_grey(f32::NAN, range), 0);
        assert_eq!(value_grey(f32::INFINITY, range), 0);
    }

    #[test]
    fn water_is_blue_by_its_depth_and_land_is_pale() {
        for land in [0.0, -1.0, f32::NAN] {
            assert_eq!(water_rgb(land), [214, 208, 190], "depth {land}");
        }
        assert_eq!(water_rgb(f32::MIN_POSITIVE), [120, 170, 220]);
        // Halfway: (120 + 20) / 2, (170 + 60) / 2, (220 + 140) / 2.
        assert_eq!(water_rgb(1.0), [70, 115, 180]);
        // A quarter: 120 - 25 = 95, 170 - 27.5 = 142.5 rounds up, 220 - 20 = 200.
        assert_eq!(water_rgb(0.5), [95, 143, 200]);
        for deep in [2.0, 5.0, f32::INFINITY] {
            assert_eq!(water_rgb(deep), [20, 60, 140], "depth {deep}");
        }
    }

    const NORTH: (f32, f32) = (0.0, -1.0);
    /// Cells left out at each edge of the interior.
    const EDGE: usize = 16;

    /// Analysis, final slope and cover of `heights`, with cover seed 1.
    fn ground_of(heights: &Grid<f32>, north: (f32, f32), rules: &GroundRules) -> Cover {
        let analysis = analyse(heights, rules);
        let slope = slope_deg(heights);
        cover(heights, &slope, &analysis, north, 1, rules, None)
    }

    /// The default rules without patch noise, so the borders follow the rules exactly.
    fn exact() -> GroundRules {
        GroundRules { patch_strength: 0.0, ..Default::default() }
    }

    /// Every cell of a `size` x `size` grid but the `EDGE` cells at each edge.
    fn interior(size: usize) -> impl Iterator<Item = (usize, usize)> {
        (EDGE..size - EDGE).flat_map(move |z| (EDGE..size - EDGE).map(move |x| (x, z)))
    }

    /// Every cell of a 256 x 256 grid whose `x` is in `columns`.
    fn columns(columns: impl Iterator<Item = usize> + Clone) -> impl Iterator<Item = (usize, usize)> {
        (0..256).flat_map(move |z| columns.clone().map(move |x| (x, z)))
    }

    fn is(c: &Cover, (x, z): (usize, usize), m: Material) -> bool {
        c.ground.get(x, z).strongest == m as u8
    }

    fn name_of(index: u8) -> &'static str {
        Material::ALL.get(usize::from(index)).map_or("no material", |m| m.name())
    }

    /// The cells of `cells` whose strongest material is not `m`, with what they are instead.
    fn others(c: &Cover, cells: impl Iterator<Item = (usize, usize)>, m: Material) -> Vec<((usize, usize), &'static str)> {
        cells.filter(|&cell| !is(c, cell, m)).map(|cell| (cell, name_of(c.ground.get(cell.0, cell.1).strongest))).collect()
    }

    fn assert_all(c: &Cover, cells: impl Iterator<Item = (usize, usize)>, m: Material, what: &str) {
        let others = others(c, cells, m);
        assert!(others.is_empty(), "{} {what} cells are not {}, the first {:?}", others.len(), m.name(), &others[..others.len().min(5)]);
    }

    #[test]
    fn a_flat_plain_is_meadow() {
        let c = ground_of(&plain(), NORTH, &GroundRules::default());
        assert_all(&c, interior(256), Material::MeadowGrass, "interior");
    }

    #[test]
    fn a_60_degree_ramp_is_rock() {
        let c = ground_of(&ramp_60(), NORTH, &GroundRules::default());
        assert_all(&c, interior(256), Material::Rock, "interior");
    }

    #[test]
    fn a_valley_floor_is_wet_ground() {
        let c = ground_of(&valley(), NORTH, &exact());
        let floor = (136..224).flat_map(|z| (128..=131).map(move |x| (x, z)));
        assert_all(&c, floor, Material::WetGround, "valley floor");
        let side: Vec<usize> = (0..256).filter(|&z| is(&c, (40, z), Material::WetGround)).collect();
        assert!(side.is_empty(), "{} cells at x = 40 are wet ground, at z {side:?}", side.len());
    }

    #[test]
    fn a_gullied_valley_floor_is_still_wet() {
        // The stages as `generate` runs them: analysis, detail on the surface, the final slope,
        // the cover. The gully's banks may push the cells beside it past the wet slope limit, so
        // only the stream's bed is asked to stay wet: for each row, the lowest cell among the
        // columns 124 to 135 (the lower `x` on a tie).
        let rules = GroundRules::default();
        let mut heights = valley();
        let analysis = analyse(&heights, &rules);
        add_detail(&mut heights, &analysis, 1, 0.25, &rules);
        let slope = slope_deg(&heights);
        let c = cover(&heights, &slope, &analysis, NORTH, 1, &rules, None);
        let bed = |z: usize| (124..=135).min_by(|&a, &b| heights.get(a, z).total_cmp(&heights.get(b, z))).unwrap();
        let rows: Vec<usize> = (136..224).collect();
        let dry: Vec<usize> = rows.iter().copied().filter(|&z| !is(&c, (bed(z), z), Material::WetGround)).collect();
        let share = 1.0 - dry.len() as f64 / rows.len() as f64;
        let what: Vec<String> = dry
            .iter()
            .take(5)
            .map(|&z| {
                let x = bed(z);
                let (x_m, z_m) = ((x as f32 + 0.5), (z as f32 + 0.5));
                format!(
                    "z {z}: x {x} {} slope {:.1} wetness {:.2} flow {:.2}",
                    name_of(c.ground.get(x, z).strongest),
                    slope.get(x, z),
                    analysis.wetness_at(x_m, z_m),
                    analysis.flow_log10_at(x_m, z_m)
                )
            })
            .collect();
        assert!(share >= 0.9, "{share} of the bed cells are wet ground; the first dry ones: {what:?}");
    }

    #[test]
    fn gentle_ground_under_a_cliff_is_scree() {
        let c = ground_of(&cliff(), NORTH, &exact());
        let below: Vec<(usize, usize)> = columns(136..144).collect();
        let scree = below.iter().filter(|&&cell| is(&c, cell, Material::Scree)).count();
        assert!(scree as f32 >= 0.8 * below.len() as f32, "{scree} of {} cells below the face are scree", below.len());
        let far = columns(221..256).filter(|&cell| is(&c, cell, Material::Scree)).count();
        assert_eq!(far, 0, "{far} cells far down the apron are scree");
    }

    #[test]
    fn a_bare_ridge_top_is_dry_grass() {
        let c = ground_of(&tent(), NORTH, &exact());
        assert_all(&c, columns(126..=130), Material::DryGrass, "crest");
        assert_all(&c, columns(64..=64), Material::MeadowGrass, "flank");
    }

    #[test]
    fn slopes_facing_the_sun_are_drier() {
        // 25 degrees, falling toward smaller `z`.
        let slope = shape(256, |_, z| 0.46631 * z);
        let dry_share = |north: (f32, f32)| {
            let c = ground_of(&slope, north, &exact());
            let cells: Vec<(usize, usize)> = interior(256).collect();
            cells.iter().filter(|&&cell| is(&c, cell, Material::DryGrass)).count() as f32 / cells.len() as f32
        };
        let (facing, away) = (dry_share((0.0, 1.0)), dry_share((0.0, -1.0)));
        assert!(facing - away >= 0.5, "dry grass share {facing} facing the sun, {away} facing away");
    }

    /// The range over the interior rows of the first `x` whose strongest material is rock.
    fn first_rock_range(rules: &GroundRules) -> usize {
        let c = ground_of(&steepening(256), NORTH, rules);
        let firsts: Vec<usize> = (EDGE..256 - EDGE)
            .map(|z| (0..256).find(|&x| is(&c, (x, z), Material::Rock)).unwrap_or_else(|| panic!("row {z} has no rock")))
            .collect();
        firsts.iter().max().unwrap() - firsts.iter().min().unwrap()
    }

    #[test]
    fn borders_wander_instead_of_following_the_rule() {
        let wandering = first_rock_range(&GroundRules::default());
        assert!((4..=120).contains(&wandering), "the first rock moves over {wandering} cells");
        assert_eq!(first_rock_range(&exact()), 0, "without patch noise");
    }

    #[test]
    fn vigour_follows_water_and_varies_on_a_plain() {
        let rules = GroundRules::default();
        let mean = |c: &Cover, cells: Vec<(usize, usize)>| {
            cells.iter().map(|&(x, z)| c.vigour.get(x, z)).sum::<f32>() / cells.len() as f32
        };
        let floor = mean(&ground_of(&valley(), NORTH, &rules), (136..224).flat_map(|z| (128..=131).map(move |x| (x, z))).collect());
        let crest = mean(&ground_of(&tent(), NORTH, &rules), (EDGE..256 - EDGE).flat_map(|z| (126..=130).map(move |x| (x, z))).collect());
        assert!(floor - crest >= 0.2, "mean vigour {floor} on the valley floor, {crest} on the crest");

        let c = ground_of(&plain(), NORTH, &rules);
        let values: Vec<f32> = interior(256).map(|(x, z)| c.vigour.get(x, z)).collect();
        let average = values.iter().sum::<f32>() / values.len() as f32;
        let deviation = (values.iter().map(|v| (v - average).powi(2)).sum::<f32>() / values.len() as f32).sqrt();
        assert!(deviation >= 0.05, "vigour on the plain has a standard deviation of {deviation}");
    }

    /// Cells wide of the run of wet ground that holds `(x, z)`, or `None` when that cell is not wet.
    fn wet_run_width(c: &Cover, x: usize, z: usize) -> Option<usize> {
        if !is(c, (x, z), Material::WetGround) {
            return None;
        }
        let wet = |x: usize| is(c, (x, z), Material::WetGround);
        let left = (0..x).rev().take_while(|&x| wet(x)).count();
        let right = (x + 1..256).take_while(|&x| wet(x)).count();
        Some(left + 1 + right)
    }

    #[test]
    fn a_diagonal_stream_has_smooth_edges() {
        // The valley floor falls 0.02 m per metre of diagonal and the diagonal starts at the
        // grid's corner, so water takes a while to gather: wet ground holds on the diagonal from
        // row 141 on (row 96 before the flow was smoothed). Above the tested range the band is
        // still forming: its width moves by up to two cells between rows (5 to 7 at row 152, 7
        // to 9 at row 164) and, even at rows 176 to 191, wobbles between 9 and 11 cells, with a
        // drop from 11 at row 182 to 9 at row 183. The smoothed flow at the stream's centre keeps
        // a small ripple with the staircase's period of four rows (log10 4.16 at row 182, 4.13 at
        // row 183), enough to move the border by a cell where wet ground's weight is close to
        // meadow's. From row 192 to the far end of the interior the width changes by at most one
        // cell between neighbouring rows, which the unsmoothed flow fails: its staircase swings
        // the width between 9 and 7 cells every second row (9 7 7 9 9 7 7 9).
        let c = ground_of(&diagonal_valley(), NORTH, &exact());
        let rows = 192..256 - EDGE;
        let widths: Vec<Option<usize>> = rows.clone().map(|z| wet_run_width(&c, z, z)).collect();
        let dry: Vec<usize> = rows.clone().zip(&widths).filter(|(_, w)| w.is_none()).map(|(z, _)| z).collect();
        assert!(dry.is_empty(), "rows {dry:?} have no wet ground on the diagonal");
        let widths: Vec<usize> = widths.into_iter().flatten().collect();
        let jumps: Vec<(usize, usize, usize)> = widths
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| pair[0].abs_diff(pair[1]) > 1)
            .map(|(k, pair)| (rows.start + k, pair[0], pair[1]))
            .collect();
        assert!(jumps.is_empty(), "widths change by more than 1 at (row, width, next width) {jumps:?}; all widths {widths:?}");
    }

    /// Indices below 6, `blend` at most 127, two different materials wherever they blend, and vigour within 0..1.
    fn assert_valid(name: &str, c: &Cover) {
        for (index, (cell, &vigour)) in c.ground.data.iter().zip(&c.vigour.data).enumerate() {
            let at = (index % c.ground.width, index / c.ground.width);
            assert!(cell.strongest < 6 && cell.second < 6, "{name}: {cell:?} at {at:?}");
            assert!(cell.blend <= 127, "{name}: {cell:?} at {at:?}");
            assert!(cell.blend == 0 || cell.second != cell.strongest, "{name}: {cell:?} at {at:?}");
            assert!((0.0..=1.0).contains(&vigour), "{name}: vigour {vigour} at {at:?}");
        }
    }

    #[test]
    fn every_shape_gives_a_valid_ground_map() {
        let shapes = [("plain", plain()), ("pit", pit()), ("60 degree ramp", ramp_60()), ("66 x 66 steepening", steepening(66))];
        for (name, heights) in shapes {
            let c = ground_of(&heights, NORTH, &GroundRules::default());
            let size = (heights.width, heights.height);
            assert_eq!((c.ground.width, c.ground.height), size, "{name}: ground");
            assert_eq!((c.vigour.width, c.vigour.height), size, "{name}: vigour");
            assert_valid(name, &c);
        }
        // The 66-cell shape must hold several materials and blends, or the checks above prove little.
        let small = ground_of(&steepening(66), NORTH, &GroundRules::default());
        let mut strongest: Vec<u8> = small.ground.data.iter().map(|cell| cell.strongest).collect();
        strongest.sort_unstable();
        strongest.dedup();
        let blended = small.ground.data.iter().filter(|cell| cell.blend > 0).count();
        assert!(strongest.len() >= 3 && blended > 0, "66 x 66: strongest materials {strongest:?}, {blended} cells blend");
    }

    #[test]
    fn a_soil_to_rock_border_blends_soil_with_rock() {
        let rules = exact();
        let heights = steepening(256);
        let slope = slope_deg(&heights);
        let c = ground_of(&heights, NORTH, &rules);
        let (soil, rock, meadow) = (Material::BareSoil as u8, Material::Rock as u8, Material::MeadowGrass as u8);
        let cells = || c.ground.data.iter().zip(&slope.data).enumerate();
        let meadow_behind: Vec<(usize, GroundCell)> = cells()
            .filter(|(_, (cell, &s))| {
                s > rules.soil_slope_full_deg
                    && (cell.strongest == soil || cell.strongest == rock)
                    && cell.second == meadow
                    && cell.blend > 0
            })
            .map(|(index, (cell, _))| (index % 256, *cell))
            .collect();
        assert!(meadow_behind.is_empty(), "{} steep cells blend in meadow, the first (x, cell) {:?}", meadow_behind.len(), &meadow_behind[..meadow_behind.len().min(5)]);
        let soil_with_rock = cells()
            .filter(|(_, (cell, _))| {
                let pair = (cell.strongest, cell.second);
                (pair == (soil, rock) || pair == (rock, soil)) && cell.blend > 0
            })
            .count();
        assert!(soil_with_rock > 0, "no cell blends bare soil with rock");
    }

    #[test]
    fn a_gentle_slope_is_meadow_right_up_to_the_edge() {
        let c = ground_of(&ramp_8(), NORTH, &exact());
        assert_all(&c, columns(0..256), Material::MeadowGrass, "8 degree slope");
    }

    #[test]
    fn equal_weights_go_to_the_lower_index_and_the_second_takes_less_than_half() {
        let tie = ground_cell(&[0.5, 0.0, 0.0, 1.0, 0.0, 1.0], 0.25);
        assert_eq!(tie, GroundCell { strongest: Material::Rock as u8, second: Material::WetGround as u8, blend: 127 });
        let pure = ground_cell(&[0.5, 0.0, 0.0, 0.0, 0.0, 0.0], 0.25);
        assert_eq!(pure, GroundCell { strongest: Material::MeadowGrass as u8, second: Material::DryGrass as u8, blend: 0 });
    }

    #[test]
    fn a_zero_transition_width_never_blends() {
        let mixed = ground_of(&steepening(256), NORTH, &GroundRules::default());
        assert!(mixed.ground.data.iter().any(|cell| cell.blend > 0), "the default width blends nowhere on this shape");
        let c = ground_of(&steepening(256), NORTH, &GroundRules { transition_width: 0.0, ..Default::default() });
        let blended = c.ground.data.iter().filter(|cell| cell.blend > 0).count();
        assert_eq!(blended, 0, "{blended} cells blend");
        let bad = c.ground.data.iter().filter(|cell| cell.strongest >= 6 || cell.second >= 6).count();
        assert_eq!(bad, 0, "{bad} cells hold an index of no material");
        let not_finite = c.vigour.data.iter().filter(|v| !v.is_finite()).count();
        assert_eq!(not_finite, 0, "{not_finite} vigour values are not finite");
    }
}
