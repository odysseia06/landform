use crate::error::LandformError;
use crate::grid::Grid;

/// Source coordinates are metres from the source's north-west corner: `x` east, `y` south.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectM {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// What a source is and how much ground it covers.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub id: String,
    pub width_m: f64,
    pub height_m: f64,
    pub cell_size_m: f32,
    pub attribution: String,
}

/// Point samples of elevation: sample `(i, j)` lies at `(origin_x + i * cell, origin_y + j * cell)`.
#[derive(Debug, Clone, PartialEq)]
pub struct HeightBlock {
    pub origin_x: f64,
    pub origin_y: f64,
    pub grid: Grid<f32>,
}

/// Catmull-Rom interpolation of four equally spaced values at `t` in `[0, 1]` between `p1` and `p2`.
fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

impl HeightBlock {
    /// Elevation at source coordinates `(x, y)` by Catmull-Rom interpolation over the 4 x 4
    /// neighbourhood. `NaN` if any of the 16 taps is `NaN` or outside the block.
    pub fn sample(&self, x: f64, y: f64) -> f32 {
        let cell = f64::from(self.grid.cell_size);
        let fx = (x - self.origin_x) / cell;
        let fy = (y - self.origin_y) / cell;
        if !fx.is_finite() || !fy.is_finite() {
            return f32::NAN;
        }
        let (ix, iy) = (fx.floor(), fy.floor());
        let (i, j) = (ix as i64, iy as i64);
        let (w, h) = (self.grid.width as i64, self.grid.height as i64);
        if i < 1 || j < 1 || i + 2 >= w || j + 2 >= h {
            return f32::NAN;
        }
        let (tx, ty) = (fx - ix, fy - iy);
        let mut column = [0.0f64; 4];
        for (row, slot) in column.iter_mut().enumerate() {
            let jj = (j - 1) as usize + row;
            let tap = |di: usize| f64::from(self.grid.get((i - 1) as usize + di, jj));
            *slot = catmull_rom(tap(0), tap(1), tap(2), tap(3), tx);
        }
        // NaN taps propagate through the arithmetic.
        catmull_rom(column[0], column[1], column[2], column[3], ty) as f32
    }
}

/// Where elevation comes from. Implementations must be deterministic and thread-safe.
pub trait ElevationSource: Send + Sync {
    fn info(&self) -> &SourceInfo;

    /// Native-resolution bare-earth metres covering `rect`, `NaN` where there is no data.
    /// `Err(OutOfBounds)` if the rectangle leaves the source.
    fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError>;
}

/// Panics with a message on any violation of the `ElevationSource` contract. The rectangle
/// is built from `info()`, so it works on a source of any size.
pub fn check_source_contract(source: &dyn ElevationSource) {
    let info = source.info();
    assert!(info.width_m > 0.0 && info.height_m > 0.0, "source size must be positive");
    assert!(info.cell_size_m > 0.0, "source cell size must be positive");
    let rect = RectM {
        x: info.width_m / 3.0,
        y: info.height_m / 3.0,
        width: (info.width_m / 4.0).min(500.0),
        height: (info.height_m / 4.0).min(300.0),
    };
    let block = source.read(rect).expect("a rectangle inside the source must be readable");
    assert_eq!(block.grid.cell_size, info.cell_size_m, "block cell size must match the source");
    assert!(
        block.origin_x <= rect.x && block.origin_y <= rect.y,
        "first sample must be at or before the rectangle's corner"
    );
    let cell = f64::from(block.grid.cell_size);
    let last_x = block.origin_x + (block.grid.width as f64 - 1.0) * cell;
    let last_y = block.origin_y + (block.grid.height as f64 - 1.0) * cell;
    assert!(
        last_x >= rect.x + rect.width && last_y >= rect.y + rect.height,
        "last sample must be at or past the rectangle's far corner"
    );
    let again = source.read(rect).expect("a second read must succeed");
    assert!(
        block.origin_x == again.origin_x
            && block.origin_y == again.origin_y
            && block.grid.width == again.grid.width
            && block.grid.height == again.grid.height
            && block.grid.data.iter().zip(&again.grid.data).all(|(a, b)| a.to_bits() == b.to_bits()),
        "reading the same rectangle twice must give identical data"
    );
    let before_origin = RectM { x: -10.0, y: 0.0, width: 20.0, height: 10.0 };
    assert_eq!(
        source.read(before_origin).err(),
        Some(LandformError::OutOfBounds),
        "a rectangle starting before the origin must be out of bounds"
    );
    let past_edge = RectM { x: info.width_m - 10.0, y: 0.0, width: 20.0, height: 10.0 };
    assert_eq!(
        source.read(past_edge).err(),
        Some(LandformError::OutOfBounds),
        "a rectangle ending past width_m must be out of bounds"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic::SyntheticSource;

    #[test]
    fn sample_reproduces_a_plane() {
        let mut g = Grid::new(8, 8, 10.0);
        for j in 0..8 {
            for i in 0..8 {
                g.set(i, j, 2.0 * (100.0 + i as f32 * 10.0) + 3.0 * (200.0 + j as f32 * 10.0));
            }
        }
        let b = HeightBlock { origin_x: 100.0, origin_y: 200.0, grid: g };
        assert!((b.sample(123.4, 231.7) - (2.0 * 123.4 + 3.0 * 231.7) as f32).abs() < 1e-3);
        assert_eq!(b.sample(120.0, 230.0), b.grid.get(2, 3));
        assert!(b.sample(100.0, 200.0).is_nan()); // no 4 x 4 neighbourhood at the edge
    }

    #[test]
    fn synthetic_source_meets_the_contract() {
        let s = SyntheticSource::new(1, 50_000.0);
        check_source_contract(&s);
        assert_eq!(s.info().id, "synthetic");
        assert_eq!(s.info().cell_size_m, 10.0);
    }
}
