use crate::error::LandformError;
use crate::grid::Grid;
use crate::noise::fbm;
use crate::source::{ElevationSource, HeightBlock, RectM, SourceInfo};

/// The seed of the standard synthetic source: one fixed terrain, from which the world seed picks a window.
pub const SYNTHETIC_SEED: u64 = 0;
/// Side length in metres of the standard synthetic source.
pub const SYNTHETIC_SIZE_M: f64 = 50_000.0;

const CELL_SIZE_M: f32 = 10.0;
const BASE_HEIGHT_M: f64 = 400.0;
const RELIEF_M: f64 = 300.0;
const OCTAVES: u32 = 6;
const WAVELENGTH_M: f64 = 4000.0;

/// Procedural terrain evaluated directly at each sample point; a square of `size_m` metres.
pub struct SyntheticSource {
    seed: u64,
    info: SourceInfo,
}

impl SyntheticSource {
    pub fn new(seed: u64, size_m: f64) -> Self {
        Self {
            seed,
            info: SourceInfo {
                id: "synthetic".to_string(),
                width_m: size_m,
                height_m: size_m,
                cell_size_m: CELL_SIZE_M,
                attribution: "Synthetic noise terrain".to_string(),
            },
        }
    }

    /// The synthetic source both front ends use: `SYNTHETIC_SEED`, `SYNTHETIC_SIZE_M` a side.
    pub fn standard() -> Self {
        Self::new(SYNTHETIC_SEED, SYNTHETIC_SIZE_M)
    }

    fn height_at(&self, x: f64, y: f64) -> f32 {
        (BASE_HEIGHT_M + RELIEF_M * f64::from(fbm(self.seed, x, y, OCTAVES, WAVELENGTH_M))) as f32
    }
}

impl ElevationSource for SyntheticSource {
    fn info(&self) -> &SourceInfo {
        &self.info
    }

    fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError> {
        let inside = rect.x >= 0.0
            && rect.y >= 0.0
            && rect.width >= 0.0
            && rect.height >= 0.0
            && rect.x + rect.width <= self.info.width_m
            && rect.y + rect.height <= self.info.height_m;
        if !inside {
            return Err(LandformError::OutOfBounds);
        }
        let cell = f64::from(self.info.cell_size_m);
        // Snap the origin down to a whole cell so repeated reads line up.
        let first_col = (rect.x / cell).floor();
        let first_row = (rect.y / cell).floor();
        let cols = ((rect.x + rect.width) / cell).ceil() - first_col + 1.0;
        let rows = ((rect.y + rect.height) / cell).ceil() - first_row + 1.0;
        let mut grid = Grid::new(cols as usize, rows as usize, self.info.cell_size_m);
        for j in 0..grid.height {
            let y = (first_row + j as f64) * cell;
            for i in 0..grid.width {
                grid.set(i, j, self.height_at((first_col + i as f64) * cell, y));
            }
        }
        Ok(HeightBlock { origin_x: first_col * cell, origin_y: first_row * cell, grid })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_source_uses_the_shared_seed_and_size() {
        let s = SyntheticSource::standard();
        assert_eq!(s.seed, SYNTHETIC_SEED);
        assert_eq!((s.info.width_m, s.info.height_m), (SYNTHETIC_SIZE_M, SYNTHETIC_SIZE_M));
    }
}
