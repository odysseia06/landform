use crate::error::LandformError;

/// The numbers of a kind of country. Heights in metres, areas in square metres, per round where it says so.
/// The three presets' `slump_m2` was tuned against pictures of their relief.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Country {
    /// Black ground's height above the outlets before carving, in metres.
    pub floor_m: f32,
    /// How far white stands above black before carving, in metres.
    pub relief_m: f32,
    /// Amplitude of the rough noise, in metres.
    pub rough_m: f32,
    /// The rough noise's largest wavelength, in metres.
    pub rough_size_m: f32,
    /// Rise per round on black ground, in metres.
    pub lift_low_m: f32,
    /// Extra rise per round on white ground, in metres.
    pub lift_high_m: f32,
    /// The cut factor `f` at 1 m² drained over a 1 m link.
    pub cut_strength: f32,
    /// The power of the drained area in the cut factor.
    pub cut_area_power: f32,
    /// Smoothing per round: the sum over passes of strength x cell² / 4, in square metres.
    pub slump_m2: f32,
    /// How many rounds of lifting, cutting and smoothing the model runs.
    pub rounds: u32,
    /// 0 makes the rock the same everywhere; otherwise cut strength varies by plus or minus this share.
    pub hardness_variation: f32,
    /// Size of the patches of harder and softer rock, in metres.
    pub hardness_patch_m: f32,
}

/// The names `Country::named` knows, gentlest first.
pub const COUNTRY_NAMES: [&str; 3] = ["gentle", "hills", "mountains"];

/// The fewest and the most rounds a country may run.
pub const MIN_ROUNDS: u32 = 1;
pub const MAX_ROUNDS: u32 = 2000;

impl Country {
    /// Low rolling country with shallow valleys.
    pub fn gentle() -> Self {
        Self {
            floor_m: 30.0,
            relief_m: 90.0,
            rough_m: 14.0,
            rough_size_m: 448.0,
            lift_low_m: 0.05,
            lift_high_m: 0.10,
            cut_strength: 0.0264,
            cut_area_power: 0.6,
            slump_m2: 18.0,
            rounds: 90,
            hardness_variation: 0.0,
            hardness_patch_m: 300.0,
        }
    }

    /// Hill country, the default.
    pub fn hills() -> Self {
        Self { relief_m: 160.0, rough_m: 22.0, lift_high_m: 0.20, slump_m2: 21.0, ..Self::gentle() }
    }

    /// High, steep country with deep valleys.
    pub fn mountains() -> Self {
        Self {
            relief_m: 320.0,
            rough_m: 34.0,
            lift_high_m: 0.45,
            cut_strength: 0.0330,
            slump_m2: 12.0,
            ..Self::gentle()
        }
    }

    /// The preset called `name` (one of `COUNTRY_NAMES`), or `None`.
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "gentle" => Some(Self::gentle()),
            "hills" => Some(Self::hills()),
            "mountains" => Some(Self::mountains()),
            _ => None,
        }
    }

    /// Every field finite and inside its range: `floor_m` 0 to 500, `relief_m` 0 to 2000, `rough_m`
    /// 0 to 200, `rough_size_m` 16 to 4000, `lift_low_m` and `lift_high_m` 0 to 5, `cut_strength` 0
    /// to 1, `cut_area_power` 0.1 to 1, `slump_m2` 0 to 64, `rounds` `MIN_ROUNDS` to `MAX_ROUNDS`
    /// (1 to 2000), `hardness_variation` 0 to 0.9, `hardness_patch_m` 16 to 4000. The error is
    /// `LandformError::BadConfig` and names the field.
    pub fn validate(&self) -> Result<(), LandformError> {
        // Taken apart field by field, so a new field fails to compile until it is checked here.
        let Self {
            floor_m,
            relief_m,
            rough_m,
            rough_size_m,
            lift_low_m,
            lift_high_m,
            cut_strength,
            cut_area_power,
            slump_m2,
            rounds,
            hardness_variation,
            hardness_patch_m,
        } = *self;
        let ranged = [
            ("floor_m", floor_m, 0.0, 500.0),
            ("relief_m", relief_m, 0.0, 2000.0),
            ("rough_m", rough_m, 0.0, 200.0),
            ("rough_size_m", rough_size_m, 16.0, 4000.0),
            ("lift_low_m", lift_low_m, 0.0, 5.0),
            ("lift_high_m", lift_high_m, 0.0, 5.0),
            ("cut_strength", cut_strength, 0.0, 1.0),
            ("cut_area_power", cut_area_power, 0.1, 1.0),
            ("slump_m2", slump_m2, 0.0, 64.0),
            ("hardness_variation", hardness_variation, 0.0, 0.9),
            ("hardness_patch_m", hardness_patch_m, 16.0, 4000.0),
        ];
        for (name, value, low, high) in ranged {
            if !value.is_finite() {
                return Err(LandformError::BadConfig(format!("{name} must be a finite number, not {value}")));
            }
            if !(low..=high).contains(&value) {
                return Err(LandformError::BadConfig(format!("{name} must be between {low} and {high}, not {value}")));
            }
        }
        if !(MIN_ROUNDS..=MAX_ROUNDS).contains(&rounds) {
            return Err(LandformError::BadConfig(format!("rounds must be between {MIN_ROUNDS} and {MAX_ROUNDS}, not {rounds}")));
        }
        Ok(())
    }
}

/// The hill country.
impl Default for Country {
    fn default() -> Self {
        Self::hills()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LandformError;

    fn message(result: Result<(), LandformError>) -> String {
        match result {
            Err(LandformError::BadConfig(reason)) => reason,
            other => panic!("expected BadConfig, got {other:?}"),
        }
    }

    #[test]
    fn the_presets_hold_the_planned_values() {
        let gentle = Country::gentle();
        assert_eq!(
            (gentle.floor_m, gentle.relief_m, gentle.rough_m, gentle.rough_size_m),
            (30.0, 90.0, 14.0, 448.0)
        );
        assert_eq!((gentle.lift_low_m, gentle.lift_high_m, gentle.cut_strength), (0.05, 0.10, 0.0264));
        assert_eq!((gentle.slump_m2, gentle.rounds, gentle.hardness_patch_m), (18.0, 90, 300.0));

        let hills = Country::hills();
        assert_eq!((hills.relief_m, hills.rough_m, hills.lift_high_m), (160.0, 22.0, 0.20));
        assert_eq!((hills.cut_strength, hills.cut_area_power, hills.slump_m2), (0.0264, 0.6, 21.0));
        assert_eq!((hills.floor_m, hills.rounds, hills.hardness_variation), (30.0, 90, 0.0));

        let mountains = Country::mountains();
        assert_eq!((mountains.relief_m, mountains.rough_m, mountains.lift_high_m), (320.0, 34.0, 0.45));
        assert_eq!((mountains.cut_strength, mountains.slump_m2), (0.0330, 12.0));
        assert_eq!((mountains.lift_low_m, mountains.rough_size_m, mountains.rounds), (0.05, 448.0, 90));

        assert_eq!(Country::default(), Country::hills());
        for (name, expected) in COUNTRY_NAMES.iter().zip([gentle, hills, mountains]) {
            assert_eq!(Country::named(name), Some(expected));
            assert_eq!(expected.validate(), Ok(()));
        }
        assert_eq!(Country::named("desert"), None);
    }

    #[test]
    fn every_country_field_is_checked() {
        let base = Country::hills();
        let cases: [(&str, fn(&mut Country)); 11] = [
            ("floor_m", |c| c.floor_m = f32::NAN),
            ("relief_m", |c| c.relief_m = f32::NAN),
            ("rough_m", |c| c.rough_m = f32::NAN),
            ("rough_size_m", |c| c.rough_size_m = f32::NAN),
            ("lift_low_m", |c| c.lift_low_m = f32::NAN),
            ("lift_high_m", |c| c.lift_high_m = f32::NAN),
            ("cut_strength", |c| c.cut_strength = f32::NAN),
            ("cut_area_power", |c| c.cut_area_power = f32::NAN),
            ("slump_m2", |c| c.slump_m2 = f32::NAN),
            ("hardness_variation", |c| c.hardness_variation = f32::NAN),
            ("hardness_patch_m", |c| c.hardness_patch_m = f32::NAN),
        ];
        for (name, spoil) in cases {
            let mut country = base;
            spoil(&mut country);
            let reason = message(country.validate());
            assert!(reason.contains(name), "{reason} should name {name}");
        }
        let mut country = base;
        country.rounds = 0;
        assert!(message(country.validate()).contains("rounds"));
        country.rounds = 2001;
        assert!(message(country.validate()).contains("rounds"));
    }

    #[test]
    fn each_range_holds_its_ends_and_refuses_what_lies_outside_it() {
        /// A ranged field of a country, to read or to set.
        type Field = fn(&mut Country) -> &mut f32;
        let ranges: [(&str, f32, f32, Field); 11] = [
            ("floor_m", 0.0, 500.0, |c| &mut c.floor_m),
            ("relief_m", 0.0, 2000.0, |c| &mut c.relief_m),
            ("rough_m", 0.0, 200.0, |c| &mut c.rough_m),
            ("rough_size_m", 16.0, 4000.0, |c| &mut c.rough_size_m),
            ("lift_low_m", 0.0, 5.0, |c| &mut c.lift_low_m),
            ("lift_high_m", 0.0, 5.0, |c| &mut c.lift_high_m),
            ("cut_strength", 0.0, 1.0, |c| &mut c.cut_strength),
            ("cut_area_power", 0.1, 1.0, |c| &mut c.cut_area_power),
            ("slump_m2", 0.0, 64.0, |c| &mut c.slump_m2),
            ("hardness_variation", 0.0, 0.9, |c| &mut c.hardness_variation),
            ("hardness_patch_m", 16.0, 4000.0, |c| &mut c.hardness_patch_m),
        ];
        let with = |field: Field, value: f32| {
            let mut country = Country::hills();
            *field(&mut country) = value;
            country
        };
        for (name, low, high, field) in ranges {
            for value in [low, high] {
                assert_eq!(with(field, value).validate(), Ok(()), "{name} = {value}");
            }
            // The nearest numbers outside the range.
            for value in [low.next_down(), high.next_up()] {
                let reason = message(with(field, value).validate());
                assert!(reason.contains(name), "{name} = {value}: {reason} should name {name}");
            }
        }
        for rounds in [MIN_ROUNDS, MAX_ROUNDS] {
            assert_eq!(Country { rounds, ..Country::hills() }.validate(), Ok(()), "rounds = {rounds}");
        }
    }
}
