//! The water rules: every tunable number of the rivers, brooks and lakes a drawn world holds.

use crate::error::LandformError;

/// Every number the water of a drawn world follows; the defaults are what ships. Widths, depths,
/// heights and distances are in metres, areas in square metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterRules {
    /// A brook starts where this much ground drains.
    pub brook_start_m2: f32,
    /// A brook's width where it starts.
    pub brook_width_m: f32,
    /// A brook widens with the ground it drains: its width is `brook_width_m` times the ground
    /// drained over `brook_start_m2`, to this power.
    pub width_area_power: f32,
    /// The water a river entering at the world's edge brings into the land model, as ground
    /// drained, when it is `river_inflow_width_m` wide; it scales with the width to the power 0.5.
    pub river_inflow_m2: f32,
    /// The width of river that brings `river_inflow_m2`.
    pub river_inflow_width_m: f32,
    /// A bed's depth as a share of the water's width, kept from `bed_depth_min_m` to
    /// `bed_depth_max_m`.
    pub bed_depth_share: f32,
    /// The shallowest a bed is; not above `bed_depth_max_m`.
    pub bed_depth_min_m: f32,
    /// The deepest a bed is.
    pub bed_depth_max_m: f32,
    /// The banks' slope, as metres across per metre up.
    pub bank_run: f32,
    /// The water's surface lies this share of the bed's depth above the bed.
    pub water_fill: f32,
    /// Ground less than this above the nearest water surface is fully wet; below
    /// `bank_wet_none_m`.
    pub bank_wet_full_m: f32,
    /// The wetting fades from `bank_wet_full_m` above the nearest water surface to no effect at
    /// this height above it.
    pub bank_wet_none_m: f32,
    /// The water wets ground only within this distance of it.
    pub bank_wet_reach_m: f32,
    /// A lake's shore is graded to no steeper than this, as metres across per metre up, within
    /// `shore_reach_m` of the water.
    pub shore_run: f32,
    /// How far from a lake's water its shore is graded.
    pub shore_reach_m: f32,
    /// How far a channel's width drifts from its own, as a share of it; from 0 to 0.5.
    pub width_wander: f32,
    /// The length of one drift of the width, in widths of the channel; from 3 to 20.
    pub wander_length_widths: f32,
    /// How far the deep line moves toward the outside of the tightest bend, as a share of the half
    /// width; from 0 to 0.5.
    pub bend_shift: f32,
}

/// The shipped values: the design's, but `brook_start_m2`, tuned to 50,000 m² from the design's
/// 150,000 m².
impl Default for WaterRules {
    fn default() -> Self {
        Self {
            brook_start_m2: 50_000.0,
            brook_width_m: 1.2,
            width_area_power: 0.4,
            river_inflow_m2: 1.6e6,
            river_inflow_width_m: 16.0,
            bed_depth_share: 0.12,
            bed_depth_min_m: 0.3,
            bed_depth_max_m: 2.5,
            bank_run: 2.0,
            water_fill: 0.6,
            bank_wet_full_m: 0.3,
            bank_wet_none_m: 1.5,
            bank_wet_reach_m: 40.0,
            shore_run: 3.0,
            shore_reach_m: 12.0,
            width_wander: 0.25,
            wander_length_widths: 6.0,
            bend_shift: 0.3,
        }
    }
}

impl WaterRules {
    /// Every field finite and inside its range: `brook_start_m2` 1,000 to 1e8, `brook_width_m` 0.2
    /// to 20, `width_area_power` 0.1 to 1, `river_inflow_m2` 0 to 1e9, `river_inflow_width_m` 1 to
    /// 200, `bed_depth_share` 0.01 to 0.5, `bed_depth_min_m` 0 to 5, `bed_depth_max_m` 0 to 10,
    /// `bank_run` 0.5 to 10, `water_fill` 0.1 to 1, `bank_wet_full_m` and `bank_wet_none_m` 0 to
    /// 10, `bank_wet_reach_m` 0 to 200, `shore_run` 0.5 to 10, `shore_reach_m` 0 to 64,
    /// `width_wander` 0 to 0.5, `wander_length_widths` 3 to 20, `bend_shift` 0 to 0.5; then
    /// `bed_depth_min_m` not above `bed_depth_max_m`, and `bank_wet_full_m` below
    /// `bank_wet_none_m`. The error is `LandformError::BadConfig` and names the field first.
    pub fn validate(&self) -> Result<(), LandformError> {
        // Taken apart field by field, so a new field fails to compile until it is checked here.
        let Self {
            brook_start_m2,
            brook_width_m,
            width_area_power,
            river_inflow_m2,
            river_inflow_width_m,
            bed_depth_share,
            bed_depth_min_m,
            bed_depth_max_m,
            bank_run,
            water_fill,
            bank_wet_full_m,
            bank_wet_none_m,
            bank_wet_reach_m,
            shore_run,
            shore_reach_m,
            width_wander,
            wander_length_widths,
            bend_shift,
        } = *self;
        let ranged = [
            ("brook_start_m2", brook_start_m2, 1_000.0, 1e8),
            ("brook_width_m", brook_width_m, 0.2, 20.0),
            ("width_area_power", width_area_power, 0.1, 1.0),
            ("river_inflow_m2", river_inflow_m2, 0.0, 1e9),
            ("river_inflow_width_m", river_inflow_width_m, 1.0, 200.0),
            ("bed_depth_share", bed_depth_share, 0.01, 0.5),
            ("bed_depth_min_m", bed_depth_min_m, 0.0, 5.0),
            ("bed_depth_max_m", bed_depth_max_m, 0.0, 10.0),
            ("bank_run", bank_run, 0.5, 10.0),
            ("water_fill", water_fill, 0.1, 1.0),
            ("bank_wet_full_m", bank_wet_full_m, 0.0, 10.0),
            ("bank_wet_none_m", bank_wet_none_m, 0.0, 10.0),
            ("bank_wet_reach_m", bank_wet_reach_m, 0.0, 200.0),
            ("shore_run", shore_run, 0.5, 10.0),
            ("shore_reach_m", shore_reach_m, 0.0, 64.0),
            ("width_wander", width_wander, 0.0, 0.5),
            ("wander_length_widths", wander_length_widths, 3.0, 20.0),
            ("bend_shift", bend_shift, 0.0, 0.5),
        ];
        let refuse = |reason: String| Err(LandformError::BadConfig(reason));
        for (name, value, low, high) in ranged {
            if !value.is_finite() {
                return refuse(format!("{name} must be a finite number, not {value}"));
            }
            if !(low..=high).contains(&value) {
                return refuse(format!("{name} must be between {low} and {high}, not {value}"));
            }
        }
        if bed_depth_min_m > bed_depth_max_m {
            return refuse(format!(
                "bed_depth_min_m must not be above bed_depth_max_m, but {bed_depth_min_m} is above {bed_depth_max_m}"
            ));
        }
        if bank_wet_full_m >= bank_wet_none_m {
            return refuse(format!(
                "bank_wet_full_m must be below bank_wet_none_m, but {bank_wet_full_m} is not below {bank_wet_none_m}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(result: Result<(), LandformError>) -> String {
        match result {
            Err(LandformError::BadConfig(reason)) => reason,
            other => panic!("expected BadConfig, got {other:?}"),
        }
    }

    /// A field of the water rules, to read or to set.
    type Field = fn(&mut WaterRules) -> &mut f32;

    /// Every field: its name, its range and how to reach it.
    fn ranges() -> [(&'static str, f32, f32, Field); 18] {
        [
            ("brook_start_m2", 1_000.0, 1e8, |r| &mut r.brook_start_m2),
            ("brook_width_m", 0.2, 20.0, |r| &mut r.brook_width_m),
            ("width_area_power", 0.1, 1.0, |r| &mut r.width_area_power),
            ("river_inflow_m2", 0.0, 1e9, |r| &mut r.river_inflow_m2),
            ("river_inflow_width_m", 1.0, 200.0, |r| &mut r.river_inflow_width_m),
            ("bed_depth_share", 0.01, 0.5, |r| &mut r.bed_depth_share),
            ("bed_depth_min_m", 0.0, 5.0, |r| &mut r.bed_depth_min_m),
            ("bed_depth_max_m", 0.0, 10.0, |r| &mut r.bed_depth_max_m),
            ("bank_run", 0.5, 10.0, |r| &mut r.bank_run),
            ("water_fill", 0.1, 1.0, |r| &mut r.water_fill),
            ("bank_wet_full_m", 0.0, 10.0, |r| &mut r.bank_wet_full_m),
            ("bank_wet_none_m", 0.0, 10.0, |r| &mut r.bank_wet_none_m),
            ("bank_wet_reach_m", 0.0, 200.0, |r| &mut r.bank_wet_reach_m),
            ("shore_run", 0.5, 10.0, |r| &mut r.shore_run),
            ("shore_reach_m", 0.0, 64.0, |r| &mut r.shore_reach_m),
            ("width_wander", 0.0, 0.5, |r| &mut r.width_wander),
            ("wander_length_widths", 3.0, 20.0, |r| &mut r.wander_length_widths),
            ("bend_shift", 0.0, 0.5, |r| &mut r.bend_shift),
        ]
    }

    #[test]
    fn the_default_water_rules_are_the_designs() {
        let rules = WaterRules::default();
        let designed = WaterRules {
            brook_start_m2: 50_000.0,
            brook_width_m: 1.2,
            width_area_power: 0.4,
            river_inflow_m2: 1.6e6,
            river_inflow_width_m: 16.0,
            bed_depth_share: 0.12,
            bed_depth_min_m: 0.3,
            bed_depth_max_m: 2.5,
            bank_run: 2.0,
            water_fill: 0.6,
            bank_wet_full_m: 0.3,
            bank_wet_none_m: 1.5,
            bank_wet_reach_m: 40.0,
            shore_run: 3.0,
            shore_reach_m: 12.0,
            width_wander: 0.25,
            wander_length_widths: 6.0,
            bend_shift: 0.3,
        };
        assert_eq!(rules, designed);
        assert_eq!(rules.validate(), Ok(()));
        // The incoming water is computed in `f64`: these two convert to exactly the numbers the
        // model used before they were rules.
        assert_eq!((f64::from(rules.river_inflow_m2), f64::from(rules.river_inflow_width_m)), (1.6e6, 16.0));
    }

    #[test]
    fn every_water_rule_is_checked() {
        let ranges = ranges();
        // The table holds every field: as many numbers as the rules, and no name twice.
        assert_eq!(std::mem::size_of::<WaterRules>(), ranges.len() * std::mem::size_of::<f32>());
        let mut names: Vec<&str> = ranges.iter().map(|&(name, ..)| name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), ranges.len(), "a name comes twice");

        // The rules each range is tried on: the pairs' other fields out of the way, so that each
        // end of a range is tried on its own.
        let open = WaterRules {
            bed_depth_min_m: 0.0,
            bed_depth_max_m: 10.0,
            bank_wet_full_m: 0.0,
            bank_wet_none_m: 10.0,
            ..WaterRules::default()
        };
        assert_eq!(open.validate(), Ok(()));
        let with = |field: Field, value: f32| {
            let mut rules = open;
            *field(&mut rules) = value;
            rules
        };
        let refused_naming = |rules: WaterRules, name: &str, what: &str| {
            let reason = message(rules.validate());
            assert!(reason.starts_with(name), "{what}: {reason} should name {name} first");
        };
        for (name, low, high, field) in ranges {
            refused_naming(with(field, f32::NAN), name, &format!("{name} = NaN"));
            for value in [low, high] {
                // `bank_wet_full_m` must lie below `bank_wet_none_m`, whose range ends where its
                // own does: the top of the one and the bottom of the other are never valid.
                if (name, value) == ("bank_wet_full_m", 10.0) || (name, value) == ("bank_wet_none_m", 0.0) {
                    refused_naming(with(field, value), "bank_wet_full_m", &format!("{name} = {value}"));
                } else {
                    assert_eq!(with(field, value).validate(), Ok(()), "{name} = {value}");
                }
            }
            // The nearest numbers outside the range.
            for value in [low.next_down(), high.next_up()] {
                refused_naming(with(field, value), name, &format!("{name} = {value}"));
            }
        }

        let rules = WaterRules::default();
        let deeper_least = WaterRules { bed_depth_min_m: 2.6, ..rules };
        refused_naming(deeper_least, "bed_depth_min_m", "bed_depth_min_m above bed_depth_max_m");
        assert_eq!(WaterRules { bed_depth_min_m: 2.5, ..rules }.validate(), Ok(()), "bed_depth_min_m at bed_depth_max_m");
        for full in [1.5, 2.0] {
            let wet_beyond = WaterRules { bank_wet_full_m: full, ..rules };
            refused_naming(wet_beyond, "bank_wet_full_m", &format!("bank_wet_full_m = {full}, bank_wet_none_m = 1.5"));
        }
        let just_below = WaterRules { bank_wet_full_m: 1.5_f32.next_down(), ..rules };
        assert_eq!(just_below.validate(), Ok(()), "bank_wet_full_m just below bank_wet_none_m");
    }
}
