use crate::grid::Grid;
use crate::source::HeightBlock;
use crate::window::Window;
use rayon::prelude::*;

/// Resamples `block` onto a `size_cells` x `size_cells` grid of `cell_size_m` metres laid out
/// in the window's orientation: cell `(i, j)` is the block sampled at
/// `window.to_source(i * cell, j * cell)`. Each cell depends only on its own coordinates.
pub fn resample(block: &HeightBlock, window: &Window, size_cells: usize, cell_size_m: f32) -> Grid<f32> {
    let mut grid = Grid::new(size_cells, size_cells, cell_size_m);
    if size_cells == 0 {
        return grid;
    }
    let cell = f64::from(cell_size_m);
    let to_source = window.source_mapping();
    grid.data.par_chunks_mut(size_cells).enumerate().for_each(|(j, row)| {
        for (i, h) in row.iter_mut().enumerate() {
            let (x, y) = to_source(i as f64 * cell, j as f64 * cell);
            *h = block.sample(x, y);
        }
    });
    grid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::HeightBlock;
    use crate::window::Window;
    use crate::grid::Grid;

    #[test]
    fn resample_follows_the_window_transform() {
        // Plane h = 2x + 3y sampled on a 10 m grid covering 4000..6000 m on both axes.
        let mut g = Grid::new(200, 200, 10.0);
        for j in 0..200 {
            for i in 0..200 {
                let (x, y) = (4000.0 + i as f32 * 10.0, 4000.0 + j as f32 * 10.0);
                g.set(i, j, 2.0 * x + 3.0 * y);
            }
        }
        let block = HeightBlock { origin_x: 4000.0, origin_y: 4000.0, grid: g };
        let window = Window { source: 0, center_x: 5000.0, center_y: 5000.0, rotation_rad: 0.7, mirrored: true, size_m: 256.0 };
        let grid = resample(&block, &window, 256, 1.0);
        assert_eq!((grid.width, grid.height, grid.cell_size), (256, 256, 1.0));
        for (i, j) in [(0, 0), (17, 200), (255, 255)] {
            let (sx, sy) = window.to_source(i as f64, j as f64);
            let expected = (2.0 * sx + 3.0 * sy) as f32;
            assert!((grid.get(i, j) - expected).abs() < 1e-2, "({i}, {j}): {} vs {expected}", grid.get(i, j));
        }
    }
}
