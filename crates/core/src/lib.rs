//! Landform core: terrain generation with no Godot dependency.

pub mod analysis;
pub mod country;
pub mod cover;
pub mod detail;
pub mod drawing;
pub mod error;
pub mod flow;
pub mod grid;
pub mod grow;
pub mod held;
pub mod land;
pub mod noise;
pub mod pack;
pub mod resample;
pub mod rng;
pub mod rules;
pub mod source;
pub mod surface;
pub mod synthetic;
pub mod trial;
mod water;
pub mod water_rules;
pub mod window;
pub mod world;

pub use analysis::{analyse, slope_deg, Analysis, ANALYSIS_CELL_M};
pub use country::{Country, COUNTRY_NAMES, MAX_ROUNDS, MIN_ROUNDS};
pub use cover::{cell_rgb, layer_range, value_grey, water_rgb, GroundCell, Material, LAYER_NAMES};
pub use detail::add_detail;
pub use drawing::{Drawing, Lake, River, PICTURE_PIXELS_MOST};
pub use error::LandformError;
pub use grid::Grid;
pub use grow::{grow, model_water, start_of, About, Grown, GrownLake, GrownRiver, RiverEnd, Start, GROW_CELL_M, GROW_MARGIN_M};
pub use held::{unheld, unheld_cells, DryCell, LakeOverWater, Unheld, UnheldCells, WaterStep, UNHELD_LIMIT_M};
pub use land::{Edit, EditTimings, Held, HoldReach, Land};
pub use land::file::{LandFile, Skipped, LAND_FORMAT};
pub use land::join::{band_ground, beyond_span, ground_threshold, join_ground, BandGround, BAND_GROUND_BEYOND_MOST};
pub use land::model::{mixed_model, Mixed, Trough, PIT_DEPTH_M, TROUGH_FALL, TROUGH_PASSES, TROUGH_REACH_M, TROUGH_SAID_M, TROUGH_SIDE};
pub use land::water::{backwater_cells, join_water, Backwater, new_unheld, CourseJoin, NewUnheld, WaterJoin, BACKWATER_ABOVE_M, BED_MARGIN_M, BROOK_PAIR_M, DROPPED_REACH_M, HEAD_SLACK_M, RISE_LIMIT_M, RIVER_PARTNER_REACH_M};
pub use land::outline::{
    area_outline, early_area_outline, early_outline, footprint, footprint_of_area, lake_zones, outline, Footprint, Marks, Outline, Whole, BAND_M, FOOTPRINT_TOLERANCE_M,
    LAKE_DISC_RADII, LAKE_ZONE_MARGIN_M, REACH_MAX_M, REACH_MIN_M, WIDTH_CHANGE_REACH_M,
};
pub use land::undo::{Change, Touched};
pub use noise::fbm;
pub use pack::{write_pack, PackHeader, PackSource, PACK_MAX_STORED, PACK_NODATA};
pub use resample::resample;
pub use rng::SplitMix64;
pub use rules::GroundRules;
pub use source::{check_source_contract, ElevationSource, HeightBlock, RectM, SourceInfo};
pub use surface::{water_surface, SurfacePiece, WaterSurface, SURFACE_PIECE_CELLS};
pub use synthetic::{SyntheticSource, SYNTHETIC_SEED, SYNTHETIC_SIZE_M};
pub use trial::{example_edit, hill_edit, standard_edit, trial_drawing, EDIT_NAMES, TRIAL_NAMES};
pub use water::{CourseEnd, CoursePoint, LakeWater, RiverCourse, Water, WATER_LAYER};
pub use water_rules::WaterRules;
pub use window::{pick_window, PickRules, Window};
pub use world::{
    finish_drawn, generate, generate_cancellable, generate_drawn, Frame, GenConfig, Timings, World, WorldMeta, DRAWN_SOURCE_ID,
    MAX_SIZE_CELLS, MIN_SIZE_CELLS,
};
