//! The `pack` subcommand: project a square of USGS tiles onto a metric grid and write a pack.

use crate::geotiff::{read_geotiff, GeoRaster};
use crate::recipe::Recipe;
use landform_core::{write_pack, Grid};
use proj4rs::proj::Proj;
use proj4rs::transform::transform;
use std::fs;
use std::path::Path;

/// The geographic system of the USGS tiles: NAD83 coordinates on the GRS80 ellipsoid.
const SOURCE_GEOGRAPHIC: &str = "+proj=longlat +ellps=GRS80 +no_defs";
/// A pack whose missing share is above this many percent is not written.
const MAX_NODATA_PERCENT: f64 = 20.0;

/// Cuts the recipe's square region out of `rasters` on a metric grid, north row first.
/// Cells outside every raster, or touching no-data, are `NaN`.
///
/// Cell `(i, j)` lies at projected `(cx + (i - n/2) * cell, cy - (j - n/2) * cell)`, where
/// `(cx, cy)` is the projected centre and `n = size_m / cell_size_m`; it is taken from the
/// first raster that contains it.
pub fn project_region(rasters: &[GeoRaster], recipe: &Recipe) -> Result<Grid<f32>, String> {
    let cell = f64::from(recipe.cell_size_m);
    if !(recipe.size_m.is_finite() && recipe.size_m > 0.0) {
        return Err(format!("The region size must be a positive number of metres, not {}.", recipe.size_m));
    }
    if !(cell.is_finite() && cell > 0.0) {
        return Err(format!("The cell size must be a positive number of metres, not {cell}."));
    }
    let cells = (recipe.size_m / cell).round();
    if !(1.0..=65_535.0).contains(&cells) {
        return Err(format!(
            "A region of {} m at {cell} m per cell has {cells} cells a side; it must be 1 to 65535.",
            recipe.size_m
        ));
    }
    let n = cells as usize;

    let projected = Proj::from_proj_string(&recipe.projection)
        .map_err(|e| format!("The projection {:?} is not usable: {e}.", recipe.projection))?;
    let geographic = Proj::from_proj_string(SOURCE_GEOGRAPHIC)
        .map_err(|e| format!("The source coordinate system is not usable: {e}."))?;

    // proj4rs works in radians for geographic coordinates.
    let mut centre = (recipe.center_lon.to_radians(), recipe.center_lat.to_radians());
    transform(&geographic, &projected, &mut centre)
        .map_err(|e| format!("The region centre cannot be projected: {e}."))?;
    let half = (n / 2) as f64;
    let xy = |i: usize, j: usize| (centre.0 + (i as f64 - half) * cell, centre.1 - (j as f64 - half) * cell);

    let mut grid = Grid { width: n, height: n, cell_size: recipe.cell_size_m, data: vec![f32::NAN; n * n] };
    let mut row: Vec<(f64, f64)> = vec![(0.0, 0.0); n];
    for j in 0..n {
        for (i, point) in row.iter_mut().enumerate() {
            *point = xy(i, j);
        }
        // One point that cannot be inverted fails its whole slice, so retry them singly.
        if transform(&projected, &geographic, row.as_mut_slice()).is_err() {
            for (i, point) in row.iter_mut().enumerate() {
                *point = xy(i, j);
                if transform(&projected, &geographic, point).is_err() {
                    *point = (f64::NAN, f64::NAN);
                }
            }
        }
        for (i, &(lon, lat)) in row.iter().enumerate() {
            let (lon, lat) = (lon.to_degrees(), lat.to_degrees());
            if let Some(h) = rasters.iter().find_map(|r| r.sample(lon, lat)) {
                grid.data[j * n + i] = h;
            }
        }
    }
    Ok(grid)
}

/// Runs `landform pack`.
pub fn run(recipe_path: &Path, tiles_dir: &Path, out: &Path) -> Result<(), String> {
    let recipe = Recipe::load(recipe_path)?;
    let mut rasters = Vec::with_capacity(recipe.tiles.len());
    for tile in &recipe.tiles {
        let path = tiles_dir.join(tile);
        if !path.is_file() {
            return Err(format!("The tile {} named by the recipe does not exist.", path.display()));
        }
        println!("Reading {}", path.display());
        rasters.push(read_geotiff(&path)?);
    }

    let grid = project_region(&rasters, &recipe)?;
    drop(rasters);
    let missing = grid.data.iter().filter(|h| !h.is_finite()).count();
    let percent = 100.0 * missing as f64 / grid.data.len() as f64;
    println!("Projected {} x {} cells at {} m, {percent:.1}% no data.", grid.width, grid.height, grid.cell_size);
    if percent > MAX_NODATA_PERCENT {
        return Err(format!(
            "{percent:.1}% of the region has no data, above the limit of {MAX_NODATA_PERCENT}%. \
             Check that the recipe names the right tiles and centre. Nothing was written."
        ));
    }

    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    write_pack(out, &recipe.id, &recipe.attribution, &grid).map_err(|e| e.to_string())?;
    println!("Wrote {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLYMPIC: &str = include_str!("../../../regions/olympic.toml");

    /// Lon -124..-123, lat 47..48 at 1/1200 degree; height rises 2000 m over the degree.
    fn test_raster() -> GeoRaster {
        let (width, height) = (1200usize, 1200usize);
        let step = 1.0 / 1200.0;
        let mut data = Vec::with_capacity(width * height);
        for r in 0..height {
            let lat = 48.0 - (r as f64 + 0.5) * step;
            let h = (1000.0 + 2000.0 * (lat - 47.0)) as f32;
            data.extend(std::iter::repeat(h).take(width));
        }
        GeoRaster {
            west: -124.0,
            north: 48.0,
            px_lon: step,
            px_lat: step,
            width,
            height,
            nodata: Some(-999_999.0),
            data,
        }
    }

    fn small_recipe() -> Recipe {
        let mut recipe = Recipe::parse(OLYMPIC).unwrap();
        recipe.size_m = 5000.0;
        recipe
    }

    #[test]
    fn projection_puts_north_first_and_keeps_heights() {
        let grid = project_region(&[test_raster()], &small_recipe()).unwrap();
        assert_eq!((grid.width, grid.height), (500, 500));
        assert_eq!(grid.cell_size, 10.0);
        assert!(grid.get(250, 0) > grid.get(250, 499));
        assert!((grid.get(250, 250) - (1000.0 + 2000.0 * 0.75)).abs() < 1.0);
        assert!(grid.data.iter().all(|h| h.is_finite()));
    }

    #[test]
    fn outside_the_tiles_is_missing() {
        let mut recipe = small_recipe();
        recipe.center_lon = -122.0;
        let grid = project_region(&[test_raster()], &recipe).unwrap();
        assert!(grid.data.iter().all(|h| h.is_nan()));
    }

    #[test]
    fn nodata_stays_missing() {
        let mut raster = test_raster();
        // The centre, lon -123.6 and lat 47.75, is column 480, row 300; blank a block around it.
        for r in 295..306 {
            for c in 475..486 {
                raster.data[r * raster.width + c] = -999_999.0;
            }
        }
        let grid = project_region(&[raster], &small_recipe()).unwrap();
        assert!(grid.get(250, 250).is_nan());
        assert!(grid.get(10, 10).is_finite());
    }

    #[test]
    fn the_first_raster_containing_a_point_wins() {
        let mut low = test_raster();
        low.data.iter_mut().for_each(|h| *h = 1.0);
        let mut high = test_raster();
        high.data.iter_mut().for_each(|h| *h = 2.0);
        let grid = project_region(&[low.clone(), high.clone()], &small_recipe()).unwrap();
        assert!(grid.data.iter().all(|&h| h == 1.0));
        // A raster that misses the region does not block the next one.
        let mut away = low;
        away.west = 10.0;
        let grid = project_region(&[away, high], &small_recipe()).unwrap();
        assert!(grid.data.iter().all(|&h| h == 2.0));
    }

    #[test]
    fn a_bad_recipe_is_an_error() {
        let edits: [fn(&mut Recipe); 6] = [
            |r| r.projection = "+proj=nonsense".to_string(),
            |r| r.size_m = 0.0,
            |r| r.size_m = -5.0,
            |r| r.cell_size_m = 0.0,
            |r| r.cell_size_m = -10.0,
            |r| r.cell_size_m = f32::NAN,
        ];
        for edit in edits {
            let mut recipe = small_recipe();
            edit(&mut recipe);
            assert!(project_region(&[test_raster()], &recipe).is_err(), "{recipe:?}");
        }
    }
}
