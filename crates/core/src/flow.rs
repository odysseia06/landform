use crate::grid::Grid;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::f64::consts::SQRT_2;

/// How far the priority flood raises a filled cell above its outlet, in metres.
pub(crate) const FILL_STEP_M: f64 = 1e-4;

/// The eight neighbours as column and row offsets, with their distance in cells.
pub(crate) const NEIGHBOURS: [(isize, isize, f64); 8] = [
    (-1, -1, SQRT_2), (0, -1, 1.0), (1, -1, SQRT_2),
    (-1, 0, 1.0), (1, 0, 1.0),
    (-1, 1, SQRT_2), (0, 1, 1.0), (1, 1, SQRT_2),
];

/// Water routed over a coarse grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    /// Ground draining through each cell in square metres, at least the cell's own area.
    pub drained_m2: Grid<f32>,
    /// Cell indices from the highest filled height to the lowest, ties by index.
    pub order: Vec<u32>,
}

/// Index and distance in cells of every neighbour of cell `index` inside a `width` x `height` grid.
pub(crate) fn neighbours(width: usize, height: usize, index: usize) -> impl Iterator<Item = (usize, f64)> {
    let (x, y) = ((index % width) as isize, (index / width) as isize);
    NEIGHBOURS.iter().filter_map(move |&(dx, dy, distance)| {
        let (nx, ny) = (x + dx, y + dy);
        let inside = nx >= 0 && ny >= 0 && (nx as usize) < width && (ny as usize) < height;
        inside.then(|| (ny as usize * width + nx as usize, distance))
    })
}

/// A cell waiting in the priority flood. Ordered so `BinaryHeap` pops the lowest first, ties by
/// index.
pub(crate) struct Waiting {
    pub(crate) height: f64,
    pub(crate) index: usize,
}

impl Ord for Waiting {
    fn cmp(&self, other: &Self) -> Ordering {
        other.height.total_cmp(&self.height).then(other.index.cmp(&self.index))
    }
}

impl PartialOrd for Waiting {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Waiting {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Waiting {}

/// `coarse` with its pits filled by a priority flood inward from the grid's edge: a cell reached
/// from an outlet no lower than itself is raised `FILL_STEP_M` above that outlet. Afterwards every
/// cell off the edge has a lower neighbour, so all water drains to the edge.
fn fill(coarse: &Grid<f32>) -> Vec<f64> {
    let (width, height) = (coarse.width, coarse.height);
    let mut filled: Vec<f64> = coarse.data.iter().map(|&h| f64::from(h)).collect();
    let mut reached = vec![false; filled.len()];
    let mut waiting = BinaryHeap::new();
    for index in 0..filled.len() {
        let (x, y) = (index % width, index / width);
        if x == 0 || y == 0 || x == width - 1 || y == height - 1 {
            reached[index] = true;
            waiting.push(Waiting { height: filled[index], index });
        }
    }
    while let Some(Waiting { height: outlet, index }) = waiting.pop() {
        for (n, _) in neighbours(width, height, index) {
            if !reached[n] {
                reached[n] = true;
                if filled[n] <= outlet {
                    filled[n] = outlet + FILL_STEP_M;
                }
                waiting.push(Waiting { height: filled[n], index: n });
            }
        }
    }
    filled
}

/// Routes water over `coarse`. Pits are filled first; then, visiting cells from high to low, each
/// hands the area drained through it to its lower neighbours in proportion to the slope towards
/// each to the power 4: spread over many directions on open slopes, gathered into one line in
/// valleys. Sequential, so the result never depends on threads.
pub fn route(coarse: &Grid<f32>) -> Flow {
    let (width, height) = (coarse.width, coarse.height);
    let filled = fill(coarse);
    let mut order: Vec<u32> = (0..filled.len() as u32).collect();
    order.sort_unstable_by(|&a, &b| filled[b as usize].total_cmp(&filled[a as usize]).then(a.cmp(&b)));
    let cell = f64::from(coarse.cell_size);
    let mut drained = vec![cell * cell; filled.len()];
    for &c in &order {
        let c = c as usize;
        let weight = |(n, distance): (usize, f64)| {
            let drop = filled[c] - filled[n];
            if drop > 0.0 {
                let slope = drop / (distance * cell);
                (slope * slope) * (slope * slope)
            } else {
                0.0
            }
        };
        let total: f64 = neighbours(width, height, c).map(weight).sum();
        if total > 0.0 {
            let per_weight = drained[c] / total;
            for neighbour in neighbours(width, height, c) {
                drained[neighbour.0] += per_weight * weight(neighbour);
            }
        }
    }
    let data = drained.into_iter().map(|area| area as f32).collect();
    Flow { drained_m2: Grid { width, height, cell_size: coarse.cell_size, data }, order }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::coarsen;
    use crate::analysis::shapes::{at_cell, pit, plain, valley};

    #[test]
    fn water_gathers_along_a_valley_floor() {
        let flow = route(&coarsen(&valley()));
        let drained = |x: usize, z: usize| at_cell(&flow.drained_m2, x, z);
        // The floor is at `x = 128`: the larger of the coarse columns on either side of it.
        let floor = |z: usize| drained(127, z).max(drained(128, z));
        let side = drained(40, 64).max(drained(40, 192));
        let (upstream, downstream) = (floor(64), floor(192));
        assert!(downstream > upstream, "floor {downstream} at z = 192, {upstream} at z = 64");
        assert!(upstream > 20.0 * side, "floor {upstream} at z = 64, side {side}");
        assert!(downstream > 20.0 * side, "floor {downstream} at z = 192, side {side}");
    }

    #[test]
    fn flow_conserves_area_and_orders_downhill() {
        let coarse = coarsen(&valley());
        let flow = route(&coarse);
        let filled = fill(&coarse);
        let (width, height) = (coarse.width, coarse.height);
        // Water leaves the grid only at cells with no lower neighbour, so together they hold all of it.
        let leaving: f64 = (0..filled.len())
            .filter(|&c| neighbours(width, height, c).all(|(n, _)| filled[n] >= filled[c]))
            .map(|c| f64::from(flow.drained_m2.data[c]))
            .sum();
        let whole = (width * height) as f64 * f64::from(coarse.cell_size).powi(2);
        let error = (leaving - whole).abs() / whole;
        assert!(error <= 1e-6, "{leaving} m2 leaves the grid, which holds {whole} m2: relative error {error}");
        let rises = flow.order.windows(2).filter(|pair| filled[pair[1] as usize] > filled[pair[0] as usize]).count();
        assert_eq!(rises, 0, "the filled height rises {rises} times along the order");
    }

    #[test]
    fn a_pit_and_a_dead_flat_plain_still_drain() {
        for (name, heights) in [("pit", pit()), ("plain", plain())] {
            let coarse = coarsen(&heights);
            let flow = route(&coarse);
            let cells = coarse.width * coarse.height;
            assert_eq!(flow.drained_m2.data.len(), cells, "{name}");
            let bad = flow.drained_m2.data.iter().filter(|a| !(a.is_finite() && **a >= 16.0)).count();
            assert_eq!(bad, 0, "{name}: {bad} drained areas are not finite or are below one cell");
            let mut sorted = flow.order.clone();
            sorted.sort_unstable();
            assert!(sorted == (0..cells as u32).collect::<Vec<u32>>(), "{name}: order does not hold every cell once");
            // The fill leaves no pit: a cell with no lower neighbour lies on the grid's edge, and all
            // of the grid's area leaves through those cells.
            let filled = fill(&coarse);
            let (width, height) = (coarse.width, coarse.height);
            let sinks: Vec<usize> = (0..cells)
                .filter(|&c| neighbours(width, height, c).all(|(n, _)| filled[n] >= filled[c]))
                .collect();
            let inside: Vec<usize> =
                sinks.iter().copied().filter(|&c| !(c % width == 0 || c / width == 0 || c % width == width - 1 || c / width == height - 1)).collect();
            assert!(inside.is_empty(), "{name}: {} cells off the edge have no lower neighbour, such as cell {:?}", inside.len(), inside.first());
            let leaving: f64 = sinks.iter().map(|&c| f64::from(flow.drained_m2.data[c])).sum();
            let whole = cells as f64 * f64::from(coarse.cell_size).powi(2);
            let error = (leaving - whole).abs() / whole;
            assert!(error <= 1e-6, "{name}: {leaving} m2 leaves through the edge, the grid holds {whole} m2: relative error {error}");
        }
    }
}
