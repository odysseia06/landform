use landform_core::{cell_rgb, layer_range, value_grey, Grid, GroundCell};

/// Direction toward the light as a unit vector in (east, south, up): from the north-west,
/// 45 degrees above the horizon. Row 0 of a grid is north, as in an image.
const LIGHT: (f32, f32, f32) = (-0.5, -0.5, std::f32::consts::FRAC_1_SQRT_2);

/// Shaded relief of `heights` as one 8-bit grey value per cell, row-major. A flat surface is
/// mid-bright; slopes facing the light are brighter and slopes facing away are darker.
pub fn hillshade(heights: &Grid<f32>) -> Vec<u8> {
    let (w, h) = (heights.width, heights.height);
    let at = |x: usize, y: usize| heights.data[y * w + x];
    let mut shade = Vec::with_capacity(w * h);
    for y in 0..h {
        let (north, south) = (y.saturating_sub(1), (y + 1).min(h - 1));
        for x in 0..w {
            let (west, east) = (x.saturating_sub(1), (x + 1).min(w - 1));
            // Central differences, one-sided at the edges.
            let dx = (at(east, y) - at(west, y)) / ((east - west).max(1) as f32 * heights.cell_size);
            let dy = (at(x, south) - at(x, north)) / ((south - north).max(1) as f32 * heights.cell_size);
            // The surface normal is (-dx, -dy, 1) before normalising; brightness is its dot
            // product with the light.
            let length = (dx * dx + dy * dy + 1.0).sqrt();
            let lit = (-dx * LIGHT.0 - dy * LIGHT.1 + LIGHT.2) / length;
            shade.push((lit.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    shade
}

/// The ground map as one RGB triple per cell, row-major, each cell coloured by `cell_rgb`.
pub fn ground_rgb(ground: &Grid<GroundCell>) -> Vec<u8> {
    ground.data.iter().flat_map(|&cell| cell_rgb(cell)).collect()
}

/// A layer as one 8-bit grey value per cell, row-major, each by `value_grey` over
/// `layer_range(name)`. Panics on a name that is not a layer's.
pub fn layer_grey(name: &str, layer: &Grid<f32>) -> Vec<u8> {
    let range = layer_range(name).unwrap_or_else(|| panic!("{name} is not a layer"));
    layer.data.iter().map(|&value| value_grey(value, range)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use landform_core::Material;

    /// A 5 by 5 grid whose height rises by `slope_x` per cell eastward and `slope_y` southward.
    fn ramp(slope_x: f32, slope_y: f32) -> Grid<f32> {
        let mut g: Grid<f32> = Grid::new(5, 5, 1.0);
        for y in 0..5 {
            for x in 0..5 {
                g.set(x, y, slope_x * x as f32 + slope_y * y as f32);
            }
        }
        g
    }

    #[test]
    fn slopes_facing_the_light_are_brighter() {
        // A falls toward the south-east, away from the north-west light.
        let away = hillshade(&ramp(-0.5, -0.5));
        // B falls toward the north-west, toward the light.
        let toward = hillshade(&ramp(0.5, 0.5));
        assert!(toward[2 * 5 + 2] > away[2 * 5 + 2]);
    }

    #[test]
    fn a_flat_grid_is_one_uniform_value() {
        let shade = hillshade(&ramp(0.0, 0.0));
        assert_eq!(shade.len(), 25);
        assert!(shade.iter().all(|&v| v == shade[0]));
    }

    #[test]
    fn a_pure_cell_gets_its_materials_colour() {
        let mut ground: Grid<GroundCell> = Grid::new(2, 1, 1.0);
        ground.set(0, 0, GroundCell { strongest: Material::Rock as u8, second: Material::Scree as u8, blend: 0 });
        ground.set(1, 0, GroundCell { strongest: Material::Rock as u8, second: Material::Scree as u8, blend: 127 });
        let rgb = ground_rgb(&ground);
        assert_eq!(rgb.len(), 6);
        assert_eq!(rgb[0..3], Material::Rock.rgb());
        // A blended cell lies between its two materials, nearer the strongest.
        let (rock, scree) = (Material::Rock.rgb(), Material::Scree.rgb());
        for channel in 0..3 {
            let (low, high) = (rock[channel].min(scree[channel]), rock[channel].max(scree[channel]));
            assert!((low..=high).contains(&rgb[3 + channel]), "channel {channel}");
        }
        assert_ne!(rgb[3..6], rock);
    }

    #[test]
    fn a_layer_value_at_the_top_of_its_range_is_white() {
        let (low, high) = layer_range("flow").unwrap();
        let mut layer: Grid<f32> = Grid::new(5, 1, 1.0);
        layer.data = vec![low - 1.0, low, high, high + 1.0, f32::NAN];
        assert_eq!(layer_grey("flow", &layer), vec![0, 0, 255, 255, 0]);
        let mut half: Grid<f32> = Grid::new(1, 1, 1.0);
        half.data = vec![30.0];
        assert_eq!(layer_grey("slope", &half), vec![128]);
        let mut top: Grid<f32> = Grid::new(1, 1, 1.0);
        top.data = vec![1.0];
        assert_eq!(layer_grey("wetness", &top), vec![255]);
    }
}
