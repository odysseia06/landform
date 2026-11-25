use crate::country::Country;
use crate::error::LandformError;
use crate::grid::Grid;
use crate::world::{MAX_SIZE_CELLS, MIN_SIZE_CELLS};

/// A river drawn as a line of points, upstream first.
#[derive(Debug, Clone, PartialEq)]
pub struct River {
    /// `(x, z)` in world metres, upstream first.
    pub points: Vec<(f64, f64)>,
    /// How wide the river is, in metres.
    pub width_m: f32,
}

/// A lake drawn as a disc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lake {
    /// The centre's `x` in world metres.
    pub x_m: f64,
    /// The centre's `z` in world metres.
    pub z_m: f64,
    /// The disc's radius, in metres.
    pub radius_m: f32,
}

/// What a person draws: high ground, rivers, lakes and the kind of country the land grows into.
/// World metres: `(0, 0)` is the centre, `x` east, `z` south.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    /// Side of the land in metres; the world has this many 1 m cells.
    pub size_m: u32,
    /// 0 = low, 1 = high; any size, covering the world with its first row at the north edge.
    /// `cell_size` is not read. Values outside 0 to 1 are clamped by the model, not refused.
    pub high_ground: Option<Grid<f32>>,
    /// The rivers.
    pub rivers: Vec<River>,
    /// The lakes.
    pub lakes: Vec<Lake>,
    /// The kind of country.
    pub country: Country,
}

/// The most pixels a drawing's picture of high ground may hold: 16,777,216 (4,096 x 4,096), so the
/// picture lies within what a Godot image holds (2^24 pixels a side and 2^28 in all, Godot's
/// `Image::MAX_WIDTH`, `MAX_HEIGHT` and `MAX_PIXELS`), which the extension makes of it
/// (`LandformDrawing`, `LandformLand`), and costs at most 64 MB as `f32`.
pub const PICTURE_PIXELS_MOST: usize = 1 << 24;

/// Side of the example picture, in pixels.
const EXAMPLE_PICTURE_PX: usize = 256;
/// The size the example numbers are written for; other sizes scale them.
const EXAMPLE_SIZE_M: f64 = 2048.0;

/// A blob of high ground in the example: centre and radius in metres at the example size, and its height.
struct Blob {
    x_m: f64,
    z_m: f64,
    radius_m: f64,
    height: f64,
}

const EXAMPLE_BLOBS: [Blob; 6] = [
    Blob { x_m: 496.0, z_m: -544.0, radius_m: 480.0, height: 0.9 },
    Blob { x_m: -64.0, z_m: -784.0, radius_m: 320.0, height: 0.6 },
    Blob { x_m: -464.0, z_m: 496.0, radius_m: 496.0, height: 0.85 },
    Blob { x_m: 336.0, z_m: 696.0, radius_m: 400.0, height: 0.7 },
    Blob { x_m: -864.0, z_m: -144.0, radius_m: 240.0, height: 0.45 },
    Blob { x_m: 864.0, z_m: -64.0, radius_m: 208.0, height: 0.4 },
];

const EXAMPLE_BIG_RIVER: [(f64, f64); 7] = [
    (-704.0, -1024.0),
    (-544.0, -624.0),
    (-264.0, -264.0),
    (96.0, -64.0),
    (456.0, 56.0),
    (776.0, 256.0),
    (1024.0, 336.0),
];

const EXAMPLE_SMALL_RIVER: [(f64, f64); 4] = [(176.0, 776.0), (96.0, 416.0), (112.0, 176.0), (96.0, -48.0)];

impl Drawing {
    /// The size, the picture, every river and lake, then the country, each refused by name:
    /// `size_m` must lie in `MIN_SIZE_CELLS..=MAX_SIZE_CELLS`; `high_ground` must have a width and
    /// height, at most `PICTURE_PIXELS_MOST` pixels, one value per pixel and only finite values; a river needs two points or more, each finite and no further
    /// than `size_m / 2` from the centre on either axis, and a width above 0 and no more than
    /// `size_m` (the water's surface is laid out in buckets over each river's reach, which a width
    /// far beyond the land's would make beyond counting); a lake needs a finite
    /// centre inside the world and a radius above 0 and no more than `size_m / 2`. Rivers and lakes
    /// are numbered from 1. The error is `LandformError::BadConfig`.
    pub fn validate(&self) -> Result<(), LandformError> {
        // Taken apart field by field, so a new field fails to compile until it is checked here.
        let Self { size_m, high_ground, rivers, lakes, country } = self;
        let refuse = |reason: String| Err(LandformError::BadConfig(reason));

        let size = *size_m as usize;
        if !(MIN_SIZE_CELLS..=MAX_SIZE_CELLS).contains(&size) {
            return refuse(format!("size_m must be between {MIN_SIZE_CELLS} and {MAX_SIZE_CELLS}, not {size_m}"));
        }
        let half = f64::from(*size_m) / 2.0;
        let inside = |x: f64, z: f64| x.is_finite() && z.is_finite() && x.abs() <= half && z.abs() <= half;

        if let Some(picture) = high_ground {
            if picture.width == 0 || picture.height == 0 {
                return refuse(format!(
                    "high_ground must have a width and a height, not {} x {}",
                    picture.width, picture.height
                ));
            }
            if picture.width.checked_mul(picture.height).is_none_or(|pixels| pixels > PICTURE_PIXELS_MOST) {
                return refuse(format!(
                    "high_ground must hold at most {PICTURE_PIXELS_MOST} pixels, so that Godot can hold the drawing's picture as an image, not {} x {}",
                    picture.width, picture.height
                ));
            }
            if picture.width.checked_mul(picture.height) != Some(picture.data.len()) {
                return refuse(format!(
                    "high_ground must hold one value per pixel, {} x {}, not {} values",
                    picture.width,
                    picture.height,
                    picture.data.len()
                ));
            }
            if picture.data.iter().any(|value| !value.is_finite()) {
                return refuse("high_ground must hold only finite values".into());
            }
        }
        for (index, river) in rivers.iter().enumerate() {
            let name = format!("river {}", index + 1);
            if river.points.len() < 2 {
                return refuse(format!("{name} needs at least two points, not {}", river.points.len()));
            }
            for (point_index, &(x, z)) in river.points.iter().enumerate() {
                if !inside(x, z) {
                    return refuse(format!(
                        "{name} point {} at ({x}, {z}) must be finite and within {half} m of the centre on both axes",
                        point_index + 1
                    ));
                }
            }
            if !river.width_m.is_finite() || river.width_m <= 0.0 {
                return refuse(format!("{name} width_m must be a finite number above zero, not {}", river.width_m));
            }
            if f64::from(river.width_m) > f64::from(*size_m) {
                return refuse(format!("{name} width_m must be at most {size_m} m, the drawing's size_m, not {}", river.width_m));
            }
        }
        for (index, lake) in lakes.iter().enumerate() {
            let name = format!("lake {}", index + 1);
            if !inside(lake.x_m, lake.z_m) {
                return refuse(format!(
                    "{name} centre ({}, {}) must be finite and within {half} m of the centre on both axes",
                    lake.x_m, lake.z_m
                ));
            }
            if !lake.radius_m.is_finite() || lake.radius_m <= 0.0 {
                return refuse(format!("{name} radius_m must be a finite number above zero, not {}", lake.radius_m));
            }
            if f64::from(lake.radius_m) > half {
                return refuse(format!("{name} radius_m must be at most {half} m, half of size_m, not {}", lake.radius_m));
            }
        }
        country.validate()
    }

    /// The reference drawing, scaled to land `size_m` across (positions, radii and widths scale with
    /// it): hill country, a 256 x 256 picture of six blobs of high ground, two rivers and a lake.
    pub fn example_valley(size_m: u32) -> Drawing {
        let scale = f64::from(size_m) / EXAMPLE_SIZE_M;
        let side = EXAMPLE_PICTURE_PX;
        let mut picture = Grid::<f32>::new(side, side, (f64::from(size_m) / side as f64) as f32);
        let pixel_centre = |index: usize| ((index as f64 + 0.5) / side as f64 - 0.5) * f64::from(size_m);
        for row in 0..side {
            let z = pixel_centre(row);
            for column in 0..side {
                let x = pixel_centre(column);
                let sum: f64 = EXAMPLE_BLOBS
                    .iter()
                    .map(|blob| {
                        let distance = (x - blob.x_m * scale).hypot(z - blob.z_m * scale);
                        blob.height * (-(distance / (blob.radius_m * scale)).powi(2)).exp()
                    })
                    .sum();
                picture.set(column, row, sum.min(1.0) as f32);
            }
        }
        let river = |points: &[(f64, f64)], width_m: f64| River {
            points: points.iter().map(|&(x, z)| (x * scale, z * scale)).collect(),
            width_m: (width_m * scale) as f32,
        };
        Drawing {
            size_m,
            high_ground: Some(picture),
            rivers: vec![river(&EXAMPLE_BIG_RIVER, 16.0), river(&EXAMPLE_SMALL_RIVER, 4.0)],
            lakes: vec![Lake { x_m: -80.0 * scale, z_m: -160.0 * scale, radius_m: (112.0 * scale) as f32 }],
            country: Country::hills(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::Country;
    use crate::error::LandformError;
    use crate::grid::Grid;

    fn message(drawing: &Drawing) -> String {
        match drawing.validate() {
            Err(LandformError::BadConfig(reason)) => reason,
            other => panic!("expected BadConfig, got {other:?}"),
        }
    }

    fn refused_naming(name: &str, spoil: impl FnOnce(&mut Drawing)) {
        let mut drawing = Drawing::example_valley(2048);
        spoil(&mut drawing);
        let reason = message(&drawing);
        assert!(reason.contains(name), "{reason} should name {name}");
    }

    #[test]
    fn the_example_is_valid_at_every_size() {
        for size in [64, 2048, 8192] {
            let drawing = Drawing::example_valley(size);
            assert_eq!(drawing.validate(), Ok(()), "size {size}");
            assert_eq!(drawing.country, Country::hills());
            let picture = drawing.high_ground.as_ref().expect("a picture");
            assert_eq!((picture.width, picture.height), (256, 256));
            assert!(picture.data.iter().all(|v| (0.0..=1.0).contains(v)));
            assert_eq!((drawing.rivers.len(), drawing.lakes.len()), (2, 1));
        }
        let big = Drawing::example_valley(8192);
        assert_eq!(big.rivers[0].points.last(), Some(&(4096.0, 1344.0)));
        assert_eq!(big.rivers[0].width_m, 64.0);
        assert_eq!(big.lakes[0].radius_m, 448.0);
    }

    #[test]
    fn the_example_picture_follows_its_blobs() {
        let drawing = Drawing::example_valley(2048);
        let picture = drawing.high_ground.as_ref().expect("a picture");
        // A pixel's centre is at ((i + 0.5) / 256 - 0.5) * 2048 metres: column 189 is at 492 m east
        // and row 59 at 548 m north of the centre, a few metres from the first blob's centre
        // (496, -544), so the sum is close to that blob's 0.9 plus the others' tails.
        let near_first_blob = picture.get(189, 59);
        assert!(near_first_blob > 0.88 && near_first_blob <= 1.0, "{near_first_blob}");
        // The south-west corner lies far from every blob.
        assert!(picture.get(0, 255) < 0.1);
        // The first row is the north edge: the second blob (-64, -784) is in the north half.
        assert!(picture.get(120, 30) > picture.get(120, 200));
    }

    #[test]
    fn a_bad_drawing_names_its_item() {
        for size in [63, 8193, 0] {
            refused_naming("size_m", |d| d.size_m = size);
        }
        refused_naming("high_ground", |d| d.high_ground = Some(Grid::new(0, 4, 1.0)));
        refused_naming("high_ground", |d| d.high_ground = Some(Grid::new(4, 0, 1.0)));
        // One value short of a 4 x 4 picture, and one over.
        for values in [15, 17] {
            refused_naming("high_ground", |d| d.high_ground = Some(Grid { width: 4, height: 4, cell_size: 1.0, data: vec![0.5; values] }));
        }
        refused_naming("high_ground", |d| {
            let mut picture = Grid::<f32>::new(4, 4, 1.0);
            picture.set(2, 1, f32::NAN);
            d.high_ground = Some(picture);
        });
        // A picture of more pixels than a Godot image of it can hold, whatever values it holds, and
        // one of the most pixels it may hold, which is valid.
        for (width, height) in [(16_777_217, 1), (4096, 4097), (1, usize::MAX)] {
            refused_naming("high_ground", |d| d.high_ground = Some(Grid { width, height, cell_size: 1.0, data: Vec::new() }));
        }
        let mut widest = Drawing::example_valley(2048);
        widest.high_ground = Some(Grid { width: 16_777_217, height: 1, cell_size: 1.0, data: Vec::new() });
        assert_eq!(message(&widest), "high_ground must hold at most 16777216 pixels, so that Godot can hold the drawing's picture as an image, not 16777217 x 1");
        let mut most = Drawing::example_valley(2048);
        most.high_ground = Some(Grid::new(4096, 4096, 0.5));
        assert_eq!(most.validate(), Ok(()));

        // Values outside 0 to 1 are clamped by the model, not refused.
        let mut loud = Drawing::example_valley(2048);
        let mut picture = Grid::<f32>::new(4, 4, 1.0);
        picture.data.fill(7.0);
        loud.high_ground = Some(picture);
        assert_eq!(loud.validate(), Ok(()));

        refused_naming("river 2", |d| d.rivers[1].points.truncate(1));
        refused_naming("river 1", |d| d.rivers[0].points.clear());
        refused_naming("river 1", |d| d.rivers[0].points[3].0 = f64::NAN);
        refused_naming("river 2", |d| d.rivers[1].points[0].1 = f64::INFINITY);
        refused_naming("river 1", |d| d.rivers[0].points[2] = (1024.5, 0.0));
        refused_naming("river 2", |d| d.rivers[1].points[1] = (0.0, -1025.0));
        refused_naming("river 1", |d| d.rivers[0].width_m = 0.0);
        refused_naming("river 2", |d| d.rivers[1].width_m = f32::NAN);
        refused_naming("river 1", |d| d.rivers[0].width_m = -3.0);
        // Wider than the land: the width `generate --drawing` once panicked on, and one just over.
        refused_naming("river 2", |d| d.rivers[1].width_m = 1e30);
        refused_naming("river 1", |d| d.rivers[0].width_m = 2048.5);
        let mut too_wide = Drawing::example_valley(2048);
        too_wide.rivers[1].width_m = 1e30;
        assert_eq!(message(&too_wide), "river 2 width_m must be at most 2048 m, the drawing's size_m, not 1000000000000000000000000000000");

        refused_naming("lake 1", |d| d.lakes[0].x_m = f64::NAN);
        refused_naming("lake 1", |d| d.lakes[0].z_m = 2000.0);
        refused_naming("lake 1", |d| d.lakes[0].radius_m = 0.0);
        refused_naming("lake 1", |d| d.lakes[0].radius_m = f32::INFINITY);
        refused_naming("lake 1", |d| d.lakes[0].radius_m = 1024.5);
        refused_naming("lake 2", |d| d.lakes.push(Lake { x_m: 0.0, z_m: 0.0, radius_m: -1.0 }));

        // The country's own errors come last and name their field.
        refused_naming("rounds", |d| d.country.rounds = 0);
        refused_naming("relief_m", |d| d.country.relief_m = f32::NAN);

        // The edge itself is inside the world, a lake may be as wide as the world, and a river as
        // wide as the land.
        let mut edge = Drawing::example_valley(2048);
        edge.rivers[0].points[0] = (-1024.0, 1024.0);
        edge.lakes[0].radius_m = 1024.0;
        edge.rivers[1].width_m = 2048.0;
        assert_eq!(edge.validate(), Ok(()));
    }

    #[test]
    fn a_drawing_without_a_picture_is_valid() {
        let mut drawing = Drawing::example_valley(512);
        drawing.high_ground = None;
        drawing.rivers.clear();
        drawing.lakes.clear();
        assert_eq!(drawing.validate(), Ok(()));
    }
}
