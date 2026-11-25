//! The water's way over the land model's heights: a flood from the outlets, each cell's receiver
//! and the ground drained.

use super::{Land, CELL_AREA_M2, CELL_M};
use crate::flow::{Waiting, FILL_STEP_M, NEIGHBOURS};
use std::collections::BinaryHeap;

/// The index of the cell `(dx, dy)` from column `x`, row `y`, when it lies on the `side` x `side` grid.
fn neighbour(side: usize, x: usize, y: usize, dx: isize, dy: isize) -> Option<usize> {
    let (nx, ny) = (x.wrapping_add_signed(dx), y.wrapping_add_signed(dy));
    (nx < side && ny < side).then(|| ny * side + nx)
}

/// The water's way over the heights. The buffers are kept from round to round.
pub(super) struct Routing {
    /// The flooded surface: the ground, with every hollow filled to just above where it spills.
    pub(super) flooded: Vec<f64>,
    /// The cell each cell's water runs to; an outlet's is itself.
    pub(super) receiver: Vec<usize>,
    /// The length of the link to the receiver, in metres, without its factor.
    pub(super) link_m: Vec<f64>,
    /// Every cell once, downstream before upstream: the order in which the flood reached them.
    pub(super) order: Vec<usize>,
    /// Ground draining through each cell, in square metres.
    pub(super) drained_m2: Vec<f64>,
    /// Whether the flood has reached each cell.
    reached: Vec<bool>,
    /// The flood's edge.
    waiting: BinaryHeap<Waiting>,
}

impl Routing {
    pub(super) fn new(cells: usize) -> Self {
        Self {
            flooded: vec![0.0; cells],
            receiver: vec![0; cells],
            link_m: vec![0.0; cells],
            order: Vec::with_capacity(cells),
            drained_m2: vec![0.0; cells],
            reached: vec![false; cells],
            waiting: BinaryHeap::with_capacity(cells),
        }
    }

    /// Routes the rain over `heights`. A flood from the outlets, lowest first and ties by index,
    /// gives each cell it reaches the flooded height `max(own, neighbour + FILL_STEP_M)` and the
    /// order of the cells. Each cell but an outlet then drains to the neighbour with the steepest
    /// drop on the flooded surface over the links' jittered lengths, and the ground drained is
    /// summed from upstream to downstream, starting at each cell's own area and incoming water.
    pub(super) fn route(&mut self, heights: &[f64], land: &Land) {
        let side = land.side;
        self.reached.fill(false);
        self.order.clear();
        for &outlet in &land.outlets {
            self.reached[outlet] = true;
            self.flooded[outlet] = heights[outlet];
            self.receiver[outlet] = outlet;
            self.waiting.push(Waiting { height: heights[outlet], index: outlet });
        }
        while let Some(Waiting { height: level, index: cell }) = self.waiting.pop() {
            self.order.push(cell);
            let (x, y) = (cell % side, cell / side);
            for &(dx, dy, cells) in &NEIGHBOURS {
                let Some(next) = neighbour(side, x, y, dx, dy) else { continue };
                if self.reached[next] {
                    continue;
                }
                self.reached[next] = true;
                self.flooded[next] = heights[next].max(level + FILL_STEP_M);
                self.receiver[next] = cell;
                self.link_m[next] = cells * CELL_M;
                self.waiting.push(Waiting { height: self.flooded[next], index: next });
            }
        }

        // A cell with no lower neighbour on the flooded surface keeps the link the flood reached it by.
        for cell in 0..side * side {
            if land.is_outlet[cell] {
                continue;
            }
            let (x, y) = (cell % side, cell / side);
            let mut steepest = 0.0;
            for (direction, &(dx, dy, cells)) in NEIGHBOURS.iter().enumerate() {
                let Some(next) = neighbour(side, x, y, dx, dy) else { continue };
                let per_length_m = f64::from(land.per_jittered_link_m[cell * NEIGHBOURS.len() + direction]);
                let slope = (self.flooded[cell] - self.flooded[next]) * per_length_m;
                if slope > steepest {
                    steepest = slope;
                    self.receiver[cell] = next;
                    self.link_m[cell] = cells * CELL_M;
                }
            }
        }

        for (drained, incoming) in self.drained_m2.iter_mut().zip(&land.incoming_m2) {
            *drained = CELL_AREA_M2 + incoming;
        }
        for &cell in self.order.iter().rev() {
            let receiver = self.receiver[cell];
            if receiver != cell {
                self.drained_m2[receiver] += self.drained_m2[cell];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{edge_cells, unit, Land, Layout};
    use super::*;
    use crate::country::Country;
    use crate::rng::mix64;

    #[test]
    fn every_cell_drains_to_a_neighbour_routed_before_it() {
        let layout = Layout::new(512);
        let (side, cells) = (layout.side, layout.cells());
        // Pits everywhere, and a dead flat grid where every tie is broken by index.
        let noisy: Vec<f64> = (0..cells).map(|cell| 50.0 * unit(mix64(cell as u64))).collect();
        let flat = vec![0.0; cells];
        let one_outlet = vec![(side / 2) * side + side - 1];
        for (outlets_name, outlets) in [("every edge cell", edge_cells(side)), ("one cell", one_outlet)] {
            let land = Land::new(&layout, &Country::hills(), 1, outlets, vec![0.0; cells], vec![0.0; cells]);
            for (heights_name, heights) in [("noisy", &noisy), ("flat", &flat)] {
                let name = format!("{heights_name} heights, outlets at {outlets_name}");
                let mut routing = Routing::new(cells);
                routing.route(heights, &land);

                let mut place = vec![usize::MAX; cells];
                for (position, &cell) in routing.order.iter().enumerate() {
                    assert_eq!(place[cell], usize::MAX, "{name}: cell {cell} comes twice in the order");
                    place[cell] = position;
                }
                assert_eq!(routing.order.len(), cells, "{name}: the order misses cells");
                for cell in 0..cells {
                    let receiver = routing.receiver[cell];
                    if land.is_outlet[cell] {
                        assert_eq!(receiver, cell, "{name}: the outlet {cell} drains elsewhere");
                        continue;
                    }
                    let (dx, dy) = ((cell % side).abs_diff(receiver % side), (cell / side).abs_diff(receiver / side));
                    assert!(receiver != cell && dx <= 1 && dy <= 1, "{name}: cell {cell} drains to {receiver}");
                    assert!(place[receiver] < place[cell], "{name}: cell {cell} comes before its receiver {receiver}");
                }
            }
        }
    }
}
