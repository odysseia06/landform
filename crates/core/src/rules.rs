use crate::error::LandformError;

/// Every threshold and strength the analysis and cover stages use; the defaults are what ships.
/// Slopes are in degrees, heights and distances in metres, areas in square metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundRules {
    /// Rock begins on ground this steep and is complete at `rock_slope_full_deg`.
    pub rock_slope_start_deg: f32,
    pub rock_slope_full_deg: f32,
    /// Bare soil begins on ground this steep and is complete at `soil_slope_full_deg`.
    pub soil_slope_start_deg: f32,
    pub soil_slope_full_deg: f32,
    /// How far loose material travels below steep ground.
    pub scree_reach_m: f32,
    /// Loose material rests only on ground gentler than this.
    pub scree_rest_slope_deg: f32,
    /// Drained area where wet ground begins, and where it is complete. The area is the one after
    /// smoothing, where a stream one analysis cell wide keeps roughly three eighths of its peak,
    /// so a thin stream needs more water than the number suggests before it reads wet.
    pub wet_flow_start_m2: f32,
    pub wet_flow_full_m2: f32,
    /// Where wet ground begins and is complete on the wetness layer, which runs from 0 to 1.
    pub wet_start: f32,
    pub wet_full: f32,
    /// No wet ground steeper than this.
    pub wet_slope_limit_deg: f32,
    /// Dry grass begins this far above the 48 m neighbourhood mean, and is complete at
    /// `dry_ridge_full_m`.
    pub dry_ridge_start_m: f32,
    pub dry_ridge_full_m: f32,
    /// Dry grass begins on slopes facing the sun this steep, and is complete at
    /// `dry_sun_slope_full_deg`.
    pub dry_sun_slope_start_deg: f32,
    pub dry_sun_slope_full_deg: f32,
    /// The meadow grass weight, which every other material must beat; it fades out as bare soil
    /// comes in, from `soil_slope_start_deg` to `soil_slope_full_deg`.
    pub meadow_weight: f32,
    /// How far patch noise moves the borders; 0 makes them follow the rules exactly.
    pub patch_strength: f32,
    /// Size of the largest patches.
    pub patch_size_m: f32,
    /// How close the second material's weight must come to the strongest's to blend in, from 0
    /// (never) to 1 (whenever it weighs anything).
    pub transition_width: f32,
    /// Gullies cut along water lines: the deepest cut in metres, where the flow is full.
    pub gully_depth_m: f32,
    /// Drained area where gullies begin and where they reach their full depth; only the largest
    /// streams reach it, so most gullies are shallower and their banks slope. The area is the
    /// smoothed one, like `wet_flow_start_m2`.
    pub gully_flow_start_m2: f32,
    pub gully_flow_full_m2: f32,
    /// Gullies fade in as the mean slope of the ground around a cell rises from 0 to this many
    /// degrees, so they die out gradually toward flat ground. 0, the default, switches the fade
    /// off: gullies reach their full depth everywhere the flow allows, even on dead-flat ground
    /// (a gully whose cut fades along a valley floor ends in a hollow with no way out). Not negative.
    pub gully_min_slope_deg: f32,
    /// Ledges on steep ground: the typical height between them in metres.
    pub ledge_spacing_m: f32,
    /// How far a ledge moves the ground at most, in metres.
    pub ledge_height_m: f32,
    /// Ledges begin on ground this steep and are complete at `ledge_slope_full_deg`.
    pub ledge_slope_start_deg: f32,
    pub ledge_slope_full_deg: f32,
    /// Roughness on rock and on resting loose material: its amplitude in metres.
    pub rough_amplitude_m: f32,
    /// Size of the roughness in metres: the wavelength of its coarser octave, with a second at
    /// half of it; keep it at 4 or more on a 1 m grid.
    pub rough_size_m: f32,
    /// No cell moves further than this many metres from the height the detail stage received.
    pub detail_limit_m: f32,
}

/// Values tuned on three windows of the Driftless Area.
impl Default for GroundRules {
    fn default() -> Self {
        Self {
            rock_slope_start_deg: 28.0,
            rock_slope_full_deg: 36.0,
            soil_slope_start_deg: 20.0,
            soil_slope_full_deg: 28.0,
            scree_reach_m: 40.0,
            scree_rest_slope_deg: 24.0,
            wet_flow_start_m2: 3000.0,
            wet_flow_full_m2: 30000.0,
            wet_start: 0.6,
            wet_full: 0.85,
            wet_slope_limit_deg: 12.0,
            dry_ridge_start_m: 1.0,
            dry_ridge_full_m: 3.0,
            dry_sun_slope_start_deg: 12.0,
            dry_sun_slope_full_deg: 20.0,
            meadow_weight: 0.5,
            patch_strength: 0.35,
            patch_size_m: 60.0,
            transition_width: 0.25,
            gully_depth_m: 1.2,
            gully_flow_start_m2: 1500.0,
            gully_flow_full_m2: 400000.0,
            gully_min_slope_deg: 0.0,
            ledge_spacing_m: 6.0,
            ledge_height_m: 0.6,
            ledge_slope_start_deg: 24.0,
            ledge_slope_full_deg: 34.0,
            rough_amplitude_m: 0.35,
            rough_size_m: 4.0,
            detail_limit_m: 2.0,
        }
    }
}

impl GroundRules {
    /// Every field finite; each `_start` at most its `_full`; strengths, weights, widths, depths,
    /// heights, amplitudes, the detail limit and `gully_min_slope_deg` not negative; `transition_width` at most 1;
    /// `patch_size_m`, `scree_reach_m`, `ledge_spacing_m` and `rough_size_m` above zero. The error
    /// is `LandformError::BadConfig` and names the field.
    pub fn validate(&self) -> Result<(), LandformError> {
        // Taken apart field by field, so a new field fails to compile until it is checked here.
        let Self {
            rock_slope_start_deg,
            rock_slope_full_deg,
            soil_slope_start_deg,
            soil_slope_full_deg,
            scree_reach_m,
            scree_rest_slope_deg,
            wet_flow_start_m2,
            wet_flow_full_m2,
            wet_start,
            wet_full,
            wet_slope_limit_deg,
            dry_ridge_start_m,
            dry_ridge_full_m,
            dry_sun_slope_start_deg,
            dry_sun_slope_full_deg,
            meadow_weight,
            patch_strength,
            patch_size_m,
            transition_width,
            gully_depth_m,
            gully_flow_start_m2,
            gully_flow_full_m2,
            gully_min_slope_deg,
            ledge_spacing_m,
            ledge_height_m,
            ledge_slope_start_deg,
            ledge_slope_full_deg,
            rough_amplitude_m,
            rough_size_m,
            detail_limit_m,
        } = *self;
        let every_field = [
            ("rock_slope_start_deg", rock_slope_start_deg),
            ("rock_slope_full_deg", rock_slope_full_deg),
            ("soil_slope_start_deg", soil_slope_start_deg),
            ("soil_slope_full_deg", soil_slope_full_deg),
            ("scree_reach_m", scree_reach_m),
            ("scree_rest_slope_deg", scree_rest_slope_deg),
            ("wet_flow_start_m2", wet_flow_start_m2),
            ("wet_flow_full_m2", wet_flow_full_m2),
            ("wet_start", wet_start),
            ("wet_full", wet_full),
            ("wet_slope_limit_deg", wet_slope_limit_deg),
            ("dry_ridge_start_m", dry_ridge_start_m),
            ("dry_ridge_full_m", dry_ridge_full_m),
            ("dry_sun_slope_start_deg", dry_sun_slope_start_deg),
            ("dry_sun_slope_full_deg", dry_sun_slope_full_deg),
            ("meadow_weight", meadow_weight),
            ("patch_strength", patch_strength),
            ("patch_size_m", patch_size_m),
            ("transition_width", transition_width),
            ("gully_depth_m", gully_depth_m),
            ("gully_flow_start_m2", gully_flow_start_m2),
            ("gully_flow_full_m2", gully_flow_full_m2),
            ("gully_min_slope_deg", gully_min_slope_deg),
            ("ledge_spacing_m", ledge_spacing_m),
            ("ledge_height_m", ledge_height_m),
            ("ledge_slope_start_deg", ledge_slope_start_deg),
            ("ledge_slope_full_deg", ledge_slope_full_deg),
            ("rough_amplitude_m", rough_amplitude_m),
            ("rough_size_m", rough_size_m),
            ("detail_limit_m", detail_limit_m),
        ];
        let start_and_full = [
            ("rock_slope_start_deg", rock_slope_start_deg, "rock_slope_full_deg", rock_slope_full_deg),
            ("soil_slope_start_deg", soil_slope_start_deg, "soil_slope_full_deg", soil_slope_full_deg),
            ("wet_flow_start_m2", wet_flow_start_m2, "wet_flow_full_m2", wet_flow_full_m2),
            ("wet_start", wet_start, "wet_full", wet_full),
            ("dry_ridge_start_m", dry_ridge_start_m, "dry_ridge_full_m", dry_ridge_full_m),
            ("dry_sun_slope_start_deg", dry_sun_slope_start_deg, "dry_sun_slope_full_deg", dry_sun_slope_full_deg),
            ("gully_flow_start_m2", gully_flow_start_m2, "gully_flow_full_m2", gully_flow_full_m2),
            ("ledge_slope_start_deg", ledge_slope_start_deg, "ledge_slope_full_deg", ledge_slope_full_deg),
        ];
        let not_negative = [
            ("meadow_weight", meadow_weight),
            ("patch_strength", patch_strength),
            ("transition_width", transition_width),
            ("gully_depth_m", gully_depth_m),
            ("gully_min_slope_deg", gully_min_slope_deg),
            ("ledge_height_m", ledge_height_m),
            ("rough_amplitude_m", rough_amplitude_m),
            ("detail_limit_m", detail_limit_m),
        ];
        let above_zero = [
            ("patch_size_m", patch_size_m),
            ("scree_reach_m", scree_reach_m),
            ("ledge_spacing_m", ledge_spacing_m),
            ("rough_size_m", rough_size_m),
        ];

        let refuse = |reason: String| Err(LandformError::BadConfig(reason));
        for (name, value) in every_field {
            if !value.is_finite() {
                return refuse(format!("{name} must be a finite number, not {value}"));
            }
        }
        for (start_name, start, full_name, full) in start_and_full {
            if start > full {
                return refuse(format!("{start_name} must not be above {full_name}, but {start} is above {full}"));
            }
        }
        for (name, value) in not_negative {
            if value < 0.0 {
                return refuse(format!("{name} must be zero or more, not {value}"));
            }
        }
        if transition_width > 1.0 {
            return refuse(format!("transition_width must be at most 1, not {transition_width}"));
        }
        for (name, value) in above_zero {
            if value <= 0.0 {
                return refuse(format!("{name} must be above zero, not {value}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::ramp;

    #[test]
    fn the_default_rules_are_valid() {
        let rules = GroundRules::default();
        assert_eq!(rules.validate(), Ok(()));
        let shape = [
            rules.gully_depth_m,
            rules.gully_flow_start_m2,
            rules.gully_flow_full_m2,
            rules.gully_min_slope_deg,
            rules.ledge_spacing_m,
            rules.ledge_height_m,
            rules.ledge_slope_start_deg,
            rules.ledge_slope_full_deg,
            rules.rough_amplitude_m,
            rules.rough_size_m,
            rules.detail_limit_m,
        ];
        assert_eq!(shape, [1.2, 1500.0, 400000.0, 0.0, 6.0, 0.6, 24.0, 34.0, 0.35, 4.0, 2.0]);
    }

    #[test]
    fn shape_rules_that_make_no_sense_are_refused_by_name() {
        let no_spacing = GroundRules { ledge_spacing_m: 0.0, ..Default::default() };
        let negative_depth = GroundRules { gully_depth_m: -1.0, ..Default::default() };
        let not_finite = GroundRules { rough_size_m: f32::NAN, ..Default::default() };
        let inverted = GroundRules { gully_flow_start_m2: 500000.0, ..Default::default() };
        let negative_slope = GroundRules { gully_min_slope_deg: -1.0, ..Default::default() };
        let negative_limit = GroundRules { detail_limit_m: -1.0, ..Default::default() };
        let negative_ledge = GroundRules { ledge_height_m: -1.0, ..Default::default() };
        let negative_rough = GroundRules { rough_amplitude_m: -1.0, ..Default::default() };
        let no_rough_size = GroundRules { rough_size_m: 0.0, ..Default::default() };
        let ledge_inverted = GroundRules { ledge_slope_start_deg: 40.0, ..Default::default() };
        for (rules, field) in [
            (no_spacing, "ledge_spacing_m"),
            (negative_depth, "gully_depth_m"),
            (not_finite, "rough_size_m"),
            (inverted, "gully_flow_start_m2"),
            (negative_slope, "gully_min_slope_deg"),
            (negative_limit, "detail_limit_m"),
            (negative_ledge, "ledge_height_m"),
            (negative_rough, "rough_amplitude_m"),
            (no_rough_size, "rough_size_m"),
            (ledge_inverted, "ledge_slope_start_deg"),
        ] {
            match rules.validate() {
                Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains(field), "{e}"),
                other => panic!("{field}: expected BadConfig, got {other:?}"),
            }
        }
    }

    #[test]
    fn rules_that_are_not_finite_or_inverted_are_refused_by_name() {
        let not_finite = GroundRules { rock_slope_start_deg: f32::NAN, ..Default::default() };
        let inverted = GroundRules { wet_start: 0.9, wet_full: 0.8, ..Default::default() };
        let too_wide = GroundRules { transition_width: 1.5, ..Default::default() };
        for (rules, field) in [(not_finite, "rock_slope_start_deg"), (inverted, "wet_start"), (too_wide, "transition_width")] {
            match rules.validate() {
                Err(e @ LandformError::BadConfig(_)) => assert!(e.to_string().contains(field), "{e}"),
                other => panic!("{field}: expected BadConfig, got {other:?}"),
            }
        }
    }

    #[test]
    fn equal_start_and_full_make_a_step() {
        let (below, above) = (ramp(5.0, 5.0, 4.9), ramp(5.0, 5.0, 5.1));
        assert!(!below.is_nan() && !above.is_nan(), "{below} {above}");
        assert_eq!((below, above), (0.0, 1.0));
    }
}
