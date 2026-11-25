use crate::error::LandformError;
use crate::rng::SplitMix64;
use crate::source::{ElevationSource, HeightBlock, RectM};
use std::f64::consts::TAU;

/// A square map window placed inside one source: centred at `(center_x, center_y)` in source
/// metres, turned by `rotation_rad`, and optionally mirrored east-west.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    /// Index of the chosen source in the slice given to `pick_window`.
    pub source: usize,
    pub center_x: f64,
    pub center_y: f64,
    pub rotation_rad: f64,
    pub mirrored: bool,
    pub size_m: f64,
}

impl Window {
    fn mirror_sign(&self) -> f64 {
        if self.mirrored {
            -1.0
        } else {
            1.0
        }
    }

    /// Source coordinates of the window point `(u, v)`, metres from the window's first corner.
    pub fn to_source(&self, u: f64, v: f64) -> (f64, f64) {
        self.source_mapping()(u, v)
    }

    /// `to_source` with the rotation's sine and cosine computed once, for mapping many points.
    pub fn source_mapping(&self) -> impl Fn(f64, f64) -> (f64, f64) + Sync {
        let (window, half) = (*self, self.size_m / 2.0);
        let (sin, cos) = self.rotation_rad.sin_cos();
        move |u, v| {
            let a = window.mirror_sign() * (u - half);
            let b = v - half;
            (window.center_x + a * cos - b * sin, window.center_y + a * sin + b * cos)
        }
    }

    /// The axis-aligned rectangle containing the whole window plus `margin_m` on every side.
    pub fn bounds(&self, margin_m: f64) -> RectM {
        let (sin, cos) = self.rotation_rad.sin_cos();
        let half_extent = self.size_m / 2.0 * (cos.abs() + sin.abs()) + margin_m;
        RectM {
            x: self.center_x - half_extent,
            y: self.center_y - half_extent,
            width: 2.0 * half_extent,
            height: 2.0 * half_extent,
        }
    }

    /// Unit vector in world `(x, z)` pointing to real north.
    pub fn north(&self) -> (f32, f32) {
        let (sin, cos) = self.rotation_rad.sin_cos();
        ((-self.mirror_sign() * sin) as f32, (-cos) as f32)
    }
}

/// When a candidate window is rejected, and how many candidates are tried.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickRules {
    pub max_tries: u32,
    /// Largest accepted share of interior samples that are flat.
    pub max_flat_fraction: f32,
    /// A sample is flat when all four neighbours differ from it by less than this many metres.
    pub flat_epsilon_m: f32,
    /// Extra ground read around the window on every side; `pick_window` raises it to two cells
    /// of a source whose cells are larger than half of it.
    pub margin_m: f64,
}

impl Default for PickRules {
    fn default() -> Self {
        Self { max_tries: 64, max_flat_fraction: 0.5, flat_epsilon_m: 0.01, margin_m: 30.0 }
    }
}

/// True if the block has a missing sample or is mostly flat ground.
fn is_unusable(block: &HeightBlock, rules: &PickRules) -> bool {
    let grid = &block.grid;
    if grid.data.iter().any(|h| h.is_nan()) {
        return true;
    }
    if grid.width < 3 || grid.height < 3 {
        return false;
    }
    let mut flat = 0usize;
    for j in 1..grid.height - 1 {
        for i in 1..grid.width - 1 {
            let h = grid.get(i, j);
            let neighbours = [grid.get(i - 1, j), grid.get(i + 1, j), grid.get(i, j - 1), grid.get(i, j + 1)];
            if neighbours.iter().all(|n| (h - n).abs() < rules.flat_epsilon_m) {
                flat += 1;
            }
        }
    }
    let interior = (grid.width - 2) * (grid.height - 2);
    flat as f32 / interior as f32 > rules.max_flat_fraction
}

/// The ground read around a window in `source`: `rules.margin_m`, but never less than two source
/// cells, since the Catmull-Rom sampler reads up to two cells past the window's edge.
fn margin_for(source: &dyn ElevationSource, rules: &PickRules) -> f64 {
    rules.margin_m.max(2.0 * f64::from(source.info().cell_size_m))
}

/// Chooses a window of `size_m` metres inside one of `sources`, the same one for the same
/// `seed`. Returns it with the block already read for `bounds` of the source's margin, which is
/// `rules.margin_m` raised to at least two source cells.
pub fn pick_window(
    seed: u64,
    sources: &[&dyn ElevationSource],
    size_m: f64,
    rules: &PickRules,
) -> Result<(Window, HeightBlock), LandformError> {
    let first = sources.first().ok_or(LandformError::NoSources)?;
    // Distance from a window's centre to the edge of what is read around it, per source.
    let radius = |source: &dyn ElevationSource| {
        size_m * std::f64::consts::SQRT_2 / 2.0 + margin_for(source, rules)
    };
    let usable: Vec<usize> = (0..sources.len())
        .filter(|&i| {
            let info = sources[i].info();
            info.width_m.min(info.height_m) >= 2.0 * radius(sources[i])
        })
        .collect();
    if usable.is_empty() {
        return Err(LandformError::SourceTooSmall {
            source_id: first.info().id.clone(),
            needed_m: 2.0 * radius(*first),
        });
    }
    let mut rng = SplitMix64::new(seed);
    for _ in 0..rules.max_tries {
        let source = usable[rng.next_below(usable.len() as u64) as usize];
        let info = sources[source].info();
        let margin = margin_for(sources[source], rules);
        let r = radius(sources[source]);
        let center_x = r + rng.next_f64() * (info.width_m - 2.0 * r);
        let center_y = r + rng.next_f64() * (info.height_m - 2.0 * r);
        let rotation_rad = rng.next_f64() * TAU;
        let mirrored = rng.next_u64() & 1 == 1;
        let window = Window { source, center_x, center_y, rotation_rad, mirrored, size_m };
        let block = match sources[source].read(window.bounds(margin)) {
            Ok(block) => block,
            // A rectangle the source cannot read rejects only this candidate.
            Err(LandformError::OutOfBounds) => continue,
            Err(other) => return Err(other),
        };
        if !is_unusable(&block, rules) {
            return Ok((window, block));
        }
    }
    Err(LandformError::NoUsableWindow { tries: rules.max_tries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::source::SourceInfo;
    use crate::synthetic::SyntheticSource;
    use std::f64::consts::{FRAC_PI_2, TAU};

    fn assert_close(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-9 && (actual.1 - expected.1).abs() < 1e-9,
            "{actual:?} is not close to {expected:?}"
        );
    }

    fn assert_close32(actual: (f32, f32), expected: (f32, f32)) {
        assert!(
            (actual.0 - expected.0).abs() < 1e-6 && (actual.1 - expected.1).abs() < 1e-6,
            "{actual:?} is not close to {expected:?}"
        );
    }

    /// A 50 km source whose height is `height(x, y)`, point-sampled on a 10 m grid.
    struct FnSource {
        info: SourceInfo,
        height: fn(f64, f64) -> f32,
    }

    impl FnSource {
        fn new(height: fn(f64, f64) -> f32) -> Self {
            Self {
                info: SourceInfo {
                    id: "test".to_string(),
                    width_m: 50_000.0,
                    height_m: 50_000.0,
                    cell_size_m: 10.0,
                    attribution: String::new(),
                },
                height,
            }
        }
    }

    impl ElevationSource for FnSource {
        fn info(&self) -> &SourceInfo {
            &self.info
        }

        fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError> {
            let inside = rect.x >= 0.0
                && rect.y >= 0.0
                && rect.x + rect.width <= self.info.width_m
                && rect.y + rect.height <= self.info.height_m;
            if !inside {
                return Err(LandformError::OutOfBounds);
            }
            let first_col = (rect.x / 10.0).floor();
            let first_row = (rect.y / 10.0).floor();
            let cols = ((rect.x + rect.width) / 10.0).ceil() - first_col + 1.0;
            let rows = ((rect.y + rect.height) / 10.0).ceil() - first_row + 1.0;
            let mut grid = Grid::new(cols as usize, rows as usize, 10.0);
            for j in 0..grid.height {
                for i in 0..grid.width {
                    let x = (first_col + i as f64) * 10.0;
                    let y = (first_row + j as f64) * 10.0;
                    grid.set(i, j, (self.height)(x, y));
                }
            }
            Ok(HeightBlock { origin_x: first_col * 10.0, origin_y: first_row * 10.0, grid })
        }
    }

    fn w(rot: f64, mirrored: bool) -> Window {
        Window { source: 0, center_x: 1000.0, center_y: 2000.0, rotation_rad: rot, mirrored, size_m: 100.0 }
    }

    #[test]
    fn to_source_maps_corners() {
        assert_close(w(0.0, false).to_source(0.0, 0.0), (950.0, 1950.0));
        assert_close(w(0.0, false).to_source(100.0, 100.0), (1050.0, 2050.0));
        assert_close(w(FRAC_PI_2, false).to_source(0.0, 0.0), (1050.0, 1950.0));
        assert_close(w(0.0, true).to_source(0.0, 0.0), (1050.0, 1950.0));
    }

    #[test]
    fn north_follows_rotation_and_mirror() {
        assert_close32(w(0.0, false).north(), (0.0, -1.0));
        assert_close32(w(FRAC_PI_2, false).north(), (-1.0, 0.0));
        assert_close32(w(FRAC_PI_2, true).north(), (1.0, 0.0));
    }

    #[test]
    fn bounds_contain_every_corner_plus_the_margin() {
        let window = w(0.6, true);
        let r = window.bounds(30.0);
        for (u, v) in [(0.0, 0.0), (100.0, 0.0), (0.0, 100.0), (100.0, 100.0)] {
            let (x, y) = window.to_source(u, v);
            assert!(x >= r.x + 30.0 - 1e-9 && x <= r.x + r.width - 30.0 + 1e-9);
            assert!(y >= r.y + 30.0 - 1e-9 && y <= r.y + r.height - 30.0 + 1e-9);
        }
        assert_close((r.x + r.width / 2.0, r.y + r.height / 2.0), (1000.0, 2000.0));
    }

    #[test]
    fn picks_are_deterministic_and_inside() {
        let s = SyntheticSource::new(1, 50_000.0);
        let rules = PickRules::default();
        let mut mirrored = 0;
        for seed in 0..200u64 {
            let (a, _) = pick_window(seed, &[&s], 2048.0, &rules).unwrap();
            let (b, _) = pick_window(seed, &[&s], 2048.0, &rules).unwrap();
            assert_eq!(
                (a.center_x, a.center_y, a.rotation_rad, a.mirrored),
                (b.center_x, b.center_y, b.rotation_rad, b.mirrored)
            );
            let r = a.bounds(rules.margin_m);
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.x + r.width <= 50_000.0 && r.y + r.height <= 50_000.0);
            assert!((0.0..TAU).contains(&a.rotation_rad));
            if a.mirrored {
                mirrored += 1;
            }
        }
        assert!((70..=130).contains(&mirrored));
    }

    #[test]
    fn windows_avoid_missing_data() {
        let s = FnSource::new(|x, y| if x < 25_000.0 { f32::NAN } else { (0.1 * x + 0.05 * y) as f32 });
        let rules = PickRules::default();
        for seed in 0..200u64 {
            let (window, block) = pick_window(seed, &[&s], 2048.0, &rules).unwrap();
            assert!(window.bounds(rules.margin_m).x >= 25_000.0, "seed {seed}");
            assert!(block.grid.data.iter().all(|h| !h.is_nan()), "seed {seed}");
        }
    }

    /// A source that refuses, as out of bounds, every rectangle starting west of 25 km.
    struct WestUnreadable(FnSource);

    impl ElevationSource for WestUnreadable {
        fn info(&self) -> &SourceInfo {
            self.0.info()
        }

        fn read(&self, rect: RectM) -> Result<HeightBlock, LandformError> {
            if rect.x < 25_000.0 {
                return Err(LandformError::OutOfBounds);
            }
            self.0.read(rect)
        }
    }

    #[test]
    fn an_unreadable_rectangle_rejects_only_that_candidate() {
        let s = WestUnreadable(FnSource::new(|x, y| (0.1 * x + 0.05 * y) as f32));
        let rules = PickRules::default();
        for seed in 0..100u64 {
            let (window, _) = pick_window(seed, &[&s], 2048.0, &rules)
                .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            assert!(window.bounds(rules.margin_m).x >= 25_000.0, "seed {seed}");
        }
    }

    #[test]
    fn all_flat_source_has_no_usable_window() {
        let s = FnSource::new(|_, _| 5.0);
        let err = pick_window(1, &[&s], 2048.0, &PickRules::default()).err();
        assert_eq!(err, Some(LandformError::NoUsableWindow { tries: 64 }));
    }

    #[test]
    fn source_smaller_than_the_map_is_an_error() {
        let s = SyntheticSource::new(1, 1000.0);
        assert!(matches!(
            pick_window(1, &[&s], 2048.0, &PickRules::default()),
            Err(LandformError::SourceTooSmall { .. })
        ));
    }

    #[test]
    fn too_small_error_names_the_first_source_and_needs_twice_r() {
        let s = SyntheticSource::new(1, 1000.0);
        let rules = PickRules::default();
        let needed = 2.0 * (2048.0 * 2f64.sqrt() / 2.0 + rules.margin_m);
        match pick_window(1, &[&s], 2048.0, &rules) {
            Err(LandformError::SourceTooSmall { source_id, needed_m }) => {
                assert_eq!(source_id, "synthetic");
                assert!((needed_m - needed).abs() < 1e-9);
            }
            other => panic!("expected SourceTooSmall, got {:?}", other.map(|(w, _)| w)),
        }
    }

    #[test]
    fn every_usable_source_gets_picked() {
        let a = SyntheticSource::new(1, 50_000.0);
        let b = SyntheticSource::new(2, 50_000.0);
        let small = SyntheticSource::new(3, 1000.0);
        let rules = PickRules::default();
        let mut seen = [false; 3];
        for seed in 0..200u64 {
            let (window, _) = pick_window(seed, &[&a, &small, &b], 2048.0, &rules).unwrap();
            seen[window.source] = true;
        }
        assert_eq!(seen, [true, false, true]);
    }

    #[test]
    fn no_sources_is_an_error() {
        assert!(matches!(pick_window(1, &[], 2048.0, &PickRules::default()), Err(LandformError::NoSources)));
    }
}
