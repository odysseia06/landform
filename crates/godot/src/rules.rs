use godot::prelude::*;
use landform_core::GroundRules;

/// One field of the core's default rules, as the float Godot shows.
fn default_of(field: fn(&GroundRules) -> f32) -> f64 {
    f64::from(field(&GroundRules::default()))
}

/// What lies where: one number per field of the core's `GroundRules`, under the same name, each
/// starting at the core's default. Slopes are in degrees, heights and distances in metres, areas
/// in square metres. A `LandformConfig` without one uses the defaults.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformGroundRules {
    /// Slope in degrees where rock begins to show.
    #[export]
    #[init(val = default_of(|r| r.rock_slope_start_deg))]
    rock_slope_start_deg: f64,
    /// Slope in degrees from which the ground is all rock.
    #[export]
    #[init(val = default_of(|r| r.rock_slope_full_deg))]
    rock_slope_full_deg: f64,
    /// Slope in degrees where bare soil begins to show and meadow begins to fade.
    #[export]
    #[init(val = default_of(|r| r.soil_slope_start_deg))]
    soil_slope_start_deg: f64,
    /// Slope in degrees from which the ground is all bare soil and meadow is gone.
    #[export]
    #[init(val = default_of(|r| r.soil_slope_full_deg))]
    soil_slope_full_deg: f64,
    /// Distance in metres that loose material travels below steep ground.
    #[export]
    #[init(val = default_of(|r| r.scree_reach_m))]
    scree_reach_m: f64,
    /// Slope in degrees below which loose material comes to rest as scree.
    #[export]
    #[init(val = default_of(|r| r.scree_rest_slope_deg))]
    scree_rest_slope_deg: f64,
    /// Drained area in square metres where wet ground begins. The area is the one after
    /// smoothing, where a stream one analysis cell wide keeps roughly three eighths of its peak,
    /// so a thin stream needs more water than the number suggests before it reads wet.
    #[export]
    #[init(val = default_of(|r| r.wet_flow_start_m2))]
    wet_flow_start_m2: f64,
    /// Drained area in square metres from which the ground is all wet. The area is the one
    /// after smoothing, where a stream one analysis cell wide keeps roughly three eighths of its
    /// peak, so a thin stream needs more water than the number suggests before it reads wet.
    #[export]
    #[init(val = default_of(|r| r.wet_flow_full_m2))]
    wet_flow_full_m2: f64,
    /// Wetness, from 0 to 1, where wet ground begins.
    #[export]
    #[init(val = default_of(|r| r.wet_start))]
    wet_start: f64,
    /// Wetness, from 0 to 1, from which the ground is all wet.
    #[export]
    #[init(val = default_of(|r| r.wet_full))]
    wet_full: f64,
    /// Slope in degrees above which no ground is wet.
    #[export]
    #[init(val = default_of(|r| r.wet_slope_limit_deg))]
    wet_slope_limit_deg: f64,
    /// Height in metres above the 48 m neighbourhood mean where dry grass begins.
    #[export]
    #[init(val = default_of(|r| r.dry_ridge_start_m))]
    dry_ridge_start_m: f64,
    /// Height in metres above the 48 m neighbourhood mean from which the grass is all dry.
    #[export]
    #[init(val = default_of(|r| r.dry_ridge_full_m))]
    dry_ridge_full_m: f64,
    /// Slope in degrees, on ground facing the sun, where dry grass begins.
    #[export]
    #[init(val = default_of(|r| r.dry_sun_slope_start_deg))]
    dry_sun_slope_start_deg: f64,
    /// Slope in degrees, on ground facing the sun, from which the grass is all dry.
    #[export]
    #[init(val = default_of(|r| r.dry_sun_slope_full_deg))]
    dry_sun_slope_full_deg: f64,
    /// Weight of meadow grass, which every other material must beat to show.
    #[export]
    #[init(val = default_of(|r| r.meadow_weight))]
    meadow_weight: f64,
    /// How far patch noise moves the borders, where 0 makes them follow the rules exactly.
    #[export]
    #[init(val = default_of(|r| r.patch_strength))]
    patch_strength: f64,
    /// Size in metres of the largest patches.
    #[export]
    #[init(val = default_of(|r| r.patch_size_m))]
    patch_size_m: f64,
    /// How close the second material's weight must come to the strongest's to blend in, from 0
    /// (never) to 1 (whenever it weighs anything).
    #[export]
    #[init(val = default_of(|r| r.transition_width))]
    transition_width: f64,
    /// Gullies cut along water lines: the deepest cut in metres, where the flow is full. 0 switches
    /// gullies off.
    #[export]
    #[init(val = default_of(|r| r.gully_depth_m))]
    gully_depth_m: f64,
    /// Drained area in square metres where gullies begin. The area is the smoothed one, like
    /// `wet_flow_start_m2`.
    #[export]
    #[init(val = default_of(|r| r.gully_flow_start_m2))]
    gully_flow_start_m2: f64,
    /// Drained area in square metres where gullies reach their full depth; only the largest streams
    /// reach it, so most gullies are shallower and their banks slope.
    #[export]
    #[init(val = default_of(|r| r.gully_flow_full_m2))]
    gully_flow_full_m2: f64,
    /// Gullies fade in as the mean slope of the ground around a cell rises from 0 to this many
    /// degrees, so they die out gradually toward flat ground. Not negative (it is refused
    /// otherwise). 0, the default, means no fade: gullies at their full depth everywhere the flow
    /// allows.
    #[export]
    #[init(val = default_of(|r| r.gully_min_slope_deg))]
    gully_min_slope_deg: f64,
    /// Ledges on steep ground: the typical height between them in metres.
    #[export]
    #[init(val = default_of(|r| r.ledge_spacing_m))]
    ledge_spacing_m: f64,
    /// How far a ledge moves the ground at most, in metres. 0 switches ledges off.
    #[export]
    #[init(val = default_of(|r| r.ledge_height_m))]
    ledge_height_m: f64,
    /// Slope in degrees where ledges begin on steep ground.
    #[export]
    #[init(val = default_of(|r| r.ledge_slope_start_deg))]
    ledge_slope_start_deg: f64,
    /// Slope in degrees from which the ledges are complete.
    #[export]
    #[init(val = default_of(|r| r.ledge_slope_full_deg))]
    ledge_slope_full_deg: f64,
    /// Roughness on rock and on resting loose material: its amplitude in metres. 0 switches
    /// roughness off.
    #[export]
    #[init(val = default_of(|r| r.rough_amplitude_m))]
    rough_amplitude_m: f64,
    /// Size of the roughness in metres: the wavelength of its coarser octave, with a second at
    /// half of it; keep it at 4 or more on a 1 m grid.
    #[export]
    #[init(val = default_of(|r| r.rough_size_m))]
    rough_size_m: f64,
    /// No cell moves further than this many metres from the height the detail stage received.
    #[export]
    #[init(val = default_of(|r| r.detail_limit_m))]
    detail_limit_m: f64,
}

impl LandformGroundRules {
    /// The rules as the core's `GroundRules`, or the message of its `validate` when they cannot
    /// work. Every field is named, with no default to fill the rest, so a field added to the core
    /// fails to compile until it is added here.
    pub fn to_core(&self) -> Result<GroundRules, String> {
        // Godot's floats have 64 bits and the core's 32: a value beyond their range becomes
        // infinite, which `validate` refuses.
        let rules = GroundRules {
            rock_slope_start_deg: self.rock_slope_start_deg as f32,
            rock_slope_full_deg: self.rock_slope_full_deg as f32,
            soil_slope_start_deg: self.soil_slope_start_deg as f32,
            soil_slope_full_deg: self.soil_slope_full_deg as f32,
            scree_reach_m: self.scree_reach_m as f32,
            scree_rest_slope_deg: self.scree_rest_slope_deg as f32,
            wet_flow_start_m2: self.wet_flow_start_m2 as f32,
            wet_flow_full_m2: self.wet_flow_full_m2 as f32,
            wet_start: self.wet_start as f32,
            wet_full: self.wet_full as f32,
            wet_slope_limit_deg: self.wet_slope_limit_deg as f32,
            dry_ridge_start_m: self.dry_ridge_start_m as f32,
            dry_ridge_full_m: self.dry_ridge_full_m as f32,
            dry_sun_slope_start_deg: self.dry_sun_slope_start_deg as f32,
            dry_sun_slope_full_deg: self.dry_sun_slope_full_deg as f32,
            meadow_weight: self.meadow_weight as f32,
            patch_strength: self.patch_strength as f32,
            patch_size_m: self.patch_size_m as f32,
            transition_width: self.transition_width as f32,
            gully_depth_m: self.gully_depth_m as f32,
            gully_flow_start_m2: self.gully_flow_start_m2 as f32,
            gully_flow_full_m2: self.gully_flow_full_m2 as f32,
            gully_min_slope_deg: self.gully_min_slope_deg as f32,
            ledge_spacing_m: self.ledge_spacing_m as f32,
            ledge_height_m: self.ledge_height_m as f32,
            ledge_slope_start_deg: self.ledge_slope_start_deg as f32,
            ledge_slope_full_deg: self.ledge_slope_full_deg as f32,
            rough_amplitude_m: self.rough_amplitude_m as f32,
            rough_size_m: self.rough_size_m as f32,
            detail_limit_m: self.detail_limit_m as f32,
        };
        rules.validate().map_err(|e| e.to_string())?;
        Ok(rules)
    }
}
