//! A region recipe: which tiles to read and which square of ground to cut from them.

use serde::Deserialize;
use std::path::Path;

/// A region recipe, parsed from TOML.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Source id stored in the pack.
    pub id: String,
    /// Credit line stored in the pack.
    pub attribution: String,
    /// Tile file names, looked up inside the tiles directory.
    pub tiles: Vec<String>,
    /// PROJ string of the metric projection the grid is laid out in.
    pub projection: String,
    /// Longitude of the region centre in degrees.
    pub center_lon: f64,
    /// Latitude of the region centre in degrees.
    pub center_lat: f64,
    /// Side length of the square region in metres.
    pub size_m: f64,
    /// Distance between neighbouring grid cells in metres.
    pub cell_size_m: f32,
}

impl Recipe {
    /// Parses a recipe from TOML text.
    pub fn parse(text: &str) -> Result<Recipe, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// Reads and parses the recipe file at `path`.
    pub fn load(path: &Path) -> Result<Recipe, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Recipe::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLYMPIC: &str = include_str!("../../../regions/olympic.toml");

    #[test]
    fn recipe_parses() {
        let recipe = Recipe::parse(OLYMPIC).expect("the olympic recipe parses");
        assert_eq!(recipe.id, "olympic");
        assert_eq!(recipe.tiles, vec!["USGS_13_n48w124.tif".to_string()]);
        assert_eq!(recipe.size_m, 50000.0);
        assert_eq!(recipe.cell_size_m, 10.0);
        assert_eq!(recipe.center_lon, -123.60);
        assert_eq!(recipe.center_lat, 47.75);
        assert!(recipe.projection.contains("+proj=utm"));
        assert!(recipe.attribution.contains("U.S. Geological Survey"));
    }

    #[test]
    fn every_region_recipe_in_the_repository_parses() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../regions");
        let mut ids: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("the regions directory is readable") {
            let path = entry.expect("a directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let recipe = Recipe::load(&path).expect("the recipe parses");
            assert!(!recipe.tiles.is_empty(), "{}: no tiles", recipe.id);
            assert_eq!(recipe.size_m, 50000.0, "{}: size_m", recipe.id);
            ids.push(recipe.id);
        }
        for expected in ["olympic", "driftless"] {
            assert!(ids.iter().any(|id| id == expected), "missing recipe {expected}, found {ids:?}");
        }
    }

    #[test]
    fn unknown_and_missing_fields_are_errors() {
        let misspelt = OLYMPIC.replace("cell_size_m", "cell_size");
        assert!(Recipe::parse(&misspelt).is_err());
        let incomplete = OLYMPIC.replace("size_m = 50000.0\n", "");
        assert!(Recipe::parse(&incomplete).is_err());
    }
}
