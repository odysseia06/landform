//! The breadth-first spread from source cells that a lake's bed and shore and the wet banks share
//! (`Spread`).

use std::collections::VecDeque;

/// The eight neighbours of a cell, as steps along the columns and the rows.
const AROUND: [(isize, isize); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
/// No source: a cell beyond the reach of every one.
const NONE: u32 = u32::MAX;

/// Which source each cell of a `width` x `height` grid of 1 m cells takes, where one lies within
/// reach, and how far it lies. Each caller reads what the source carries for it: a lake only the
/// distance, a bank also the source's surface.
///
/// A breadth-first spread from the sources, in index order, across the eight neighbours, in a
/// fixed order: each cell takes, of the sources its neighbours took, the nearest it has been
/// offered, and passes it on whenever that brings it nearer, as long as it lies within the reach.
/// So each distance is to one of the sources and is never less than the nearest's. It is the
/// nearest's unless the cells nearest a source reach it only across cells that took another, a
/// thin sliver between other sources; on random sources, few and far apart to many close
/// together, out to 12 m and 64 m, every distance is the nearest's
/// (`the_spread_finds_the_nearest_source`). Sequential and deterministic.
pub(super) struct Spread {
    /// Each cell's source, as an index on the grid, or `NONE`.
    nearest: Vec<u32>,
    /// The square of each cell's distance to its source, in square cells.
    apart: Vec<u32>,
}

impl Spread {
    /// The spread from the cells `is_source` marks, out to `reach_m`. Indices and squared
    /// distances are kept as `u32`, 8 bytes a cell: the largest world, 8192 cells a side, has
    /// fewer cells than `u32::MAX`, and a squared distance is kept only within the reach.
    pub(super) fn new(width: usize, height: usize, is_source: &[bool], reach_m: f64) -> Self {
        let count = width * height;
        debug_assert!(count < NONE as usize, "a grid of {count} cells");
        debug_assert_eq!(is_source.len(), count, "one mark per cell");
        let mut nearest = vec![NONE; count];
        let mut apart = vec![u32::MAX; count];
        let mut waiting = VecDeque::new();
        for cell in (0..count).filter(|&cell| is_source[cell]) {
            (nearest[cell], apart[cell]) = (cell as u32, 0);
            waiting.push_back(cell as u32);
        }
        let reach_squared = reach_m * reach_m;
        while let Some(cell) = waiting.pop_front() {
            let (cell, source) = (cell as usize, nearest[cell as usize]);
            let (column, row) = (cell % width, cell / width);
            let (source_column, source_row) = (source as usize % width, source as usize / width);
            for (across, down) in AROUND {
                let Some(next_column) = column.checked_add_signed(across).filter(|&next| next < width) else { continue };
                let Some(next_row) = row.checked_add_signed(down).filter(|&next| next < height) else { continue };
                let next = next_row * width + next_column;
                let (across, down) = (next_column.abs_diff(source_column), next_row.abs_diff(source_row));
                let squared = across * across + down * down;
                if squared < apart[next] as usize && squared as f64 <= reach_squared {
                    (nearest[next], apart[next]) = (source, squared as u32);
                    waiting.push_back(next as u32);
                }
            }
        }
        Self { nearest, apart }
    }

    /// The source `cell` took, as an index on the grid; its own at a source, and `None` where no
    /// source lies within reach.
    pub(super) fn source(&self, cell: usize) -> Option<usize> {
        (self.nearest[cell] != NONE).then_some(self.nearest[cell] as usize)
    }

    /// The distance in metres from `cell` to the source it took: 0 at a source, and INFINITY
    /// where no source lies within reach.
    pub(super) fn distance_m(&self, cell: usize) -> f64 {
        match self.nearest[cell] {
            NONE => f64::INFINITY,
            _ => f64::from(self.apart[cell]).sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::SplitMix64;

    #[test]
    fn the_spread_finds_the_nearest_source() {
        let mut random = SplitMix64::new(11);
        let (width, height) = (90, 90);
        for reach_m in [12.0, 64.0] {
            for trial in 0..24 {
                // From a few sources, far apart, to many close together.
                let share = [0.0005, 0.002, 0.01, 0.05][trial % 4];
                let is_source: Vec<bool> = (0..width * height).map(|_| random.next_f64() < share).collect();
                let sources: Vec<usize> = (0..width * height).filter(|&cell| is_source[cell]).collect();
                let spread = Spread::new(width, height, &is_source, reach_m);
                for cell in 0..width * height {
                    let apart_m = |source: usize| {
                        let (across, down) = ((cell % width).abs_diff(source % width), (cell / width).abs_diff(source / width));
                        ((across * across + down * down) as f64).sqrt()
                    };
                    let nearest_m = sources.iter().map(|&source| apart_m(source)).fold(f64::INFINITY, f64::min);
                    let expected = if nearest_m <= reach_m { nearest_m } else { f64::INFINITY };
                    assert_eq!(spread.distance_m(cell), expected, "reach {reach_m} m, trial {trial}: cell {cell}");
                    // The source taken lies at that distance.
                    let taken_m = spread.source(cell).map_or(f64::INFINITY, apart_m);
                    assert_eq!(taken_m, expected, "reach {reach_m} m, trial {trial}: cell {cell} took {:?}", spread.source(cell));
                }
            }
        }
    }
}
