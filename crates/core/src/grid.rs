/// A dense 2D grid stored row-major, row 0 first.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid<T> {
    pub width: usize,
    pub height: usize,
    /// Metres between neighbouring cells.
    pub cell_size: f32,
    pub data: Vec<T>,
}

impl<T: Clone + Default> Grid<T> {
    /// Creates a grid filled with `T::default()`.
    pub fn new(width: usize, height: usize, cell_size: f32) -> Self {
        Self { width, height, cell_size, data: vec![T::default(); width * height] }
    }
}

impl<T: Copy> Grid<T> {
    /// Returns the value at column `x`, row `y`. Panics when out of range.
    pub fn get(&self, x: usize, y: usize) -> T {
        assert!(x < self.width && y < self.height, "grid index ({x}, {y}) out of range");
        self.data[y * self.width + x]
    }

    /// Stores `v` at column `x`, row `y`. Panics when out of range.
    pub fn set(&mut self, x: usize, y: usize, v: T) {
        assert!(x < self.width && y < self.height, "grid index ({x}, {y}) out of range");
        self.data[y * self.width + x] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_stores_row_major() {
        let mut g: Grid<f32> = Grid::new(3, 2, 10.0);
        g.set(2, 1, 7.5);
        assert_eq!(g.data[5], 7.5);
        assert_eq!(g.get(2, 1), 7.5);
    }
}
