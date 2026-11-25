use godot::prelude::*;
use landform_core::{GenConfig, GroundRules, WaterRules};

use crate::drawing::LandformDrawing;
use crate::rules::LandformGroundRules;
use crate::water::LandformWaterRules;

/// What a `LandformGenerator` builds a world from: the terrain source, the world size, the
/// strength of the added detail, the ground rules, the water rules of a drawn world and the layers
/// the world keeps.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformConfig {
    /// `SOURCE_SYNTHETIC`, `SOURCE_PACKS` or `SOURCE_DRAWING`.
    #[export(enum = (
        Synthetic = LandformConfig::SOURCE_SYNTHETIC,
        Packs = LandformConfig::SOURCE_PACKS,
        Drawing = LandformConfig::SOURCE_DRAWING,
    ))]
    #[init(val = LandformConfig::SOURCE_SYNTHETIC)]
    pub source: i64,
    /// Folder of terrain packs (`res://`, `user://` or absolute); every `*.lfpk` in it is used,
    /// in file-name order.
    #[export]
    pub pack_dir: GString,
    /// The drawing the land is grown from when the source is `SOURCE_DRAWING`.
    #[export]
    pub drawing: Option<Gd<LandformDrawing>>,
    /// World side length in cells of one metre. Not used with a drawing, whose `size_m` sets the
    /// world's size.
    #[export]
    #[init(val = GenConfig::default().size_cells as i64)]
    pub map_size: i64,
    /// Amplitude in metres of the low hummocks added on top of the source terrain. 0 switches off all
    /// the ground shapes: the hummocks, the roughness, the ledges and the gullies.
    #[export]
    #[init(val = f64::from(GenConfig::default().detail_amplitude_m))]
    pub detail_strength: f64,
    /// What lies where; `null` uses the core's default rules.
    #[export]
    pub ground_rules: Option<Gd<LandformGroundRules>>,
    /// How the rivers, brooks and lakes of a world grown from the drawing lie; `null` uses the
    /// core's default rules. Read only when the source is `SOURCE_DRAWING`: no other world has water.
    #[export]
    pub water_rules: Option<Gd<LandformWaterRules>>,
    /// The layers the world keeps, by name: any of `flow`, `slope`, `vigour` and `wetness`, which
    /// every world has, and `water`, the water's depth in metres, which only a world grown from the
    /// drawing has (any other world simply lacks it). The others are dropped as soon as they are
    /// made; by default the world keeps none.
    #[export]
    pub export_layers: PackedStringArray,
}

#[godot_api]
impl LandformConfig {
    /// Procedural terrain, available without any pack.
    #[constant]
    pub const SOURCE_SYNTHETIC: i64 = 0;
    /// The packs in `pack_dir`.
    #[constant]
    pub const SOURCE_PACKS: i64 = 1;
    /// Land grown from `drawing`.
    #[constant]
    pub const SOURCE_DRAWING: i64 = 2;
}

impl LandformConfig {
    /// The core's settings of a world `size_cells` across: the detail's strength, the ground rules
    /// and, when `with_water`, the water rules (each `null` the core's default; without, the water
    /// rules are the default and not read), checked in that order, or why one is not valid. Main
    /// thread only: it reads the rules' resources.
    pub(crate) fn settings(&self, size_cells: usize, with_water: bool) -> Result<GenConfig, String> {
        let strength = self.detail_strength;
        // The core works in 32-bit floats: a strength beyond their range becomes infinite.
        let amplitude = strength as f32;
        if !(amplitude.is_finite() && amplitude >= 0.0) {
            return Err(format!(
                "detail_strength must be a number of metres from 0 to {}, not {strength}.",
                f32::MAX
            ));
        }
        let rules = match &self.ground_rules {
            Some(rules) => rules.bind().to_core()?,
            None => GroundRules::default(),
        };
        let water = match &self.water_rules {
            Some(water) if with_water => water.bind().to_core()?,
            _ => WaterRules::default(),
        };
        Ok(GenConfig { size_cells, detail_amplitude_m: amplitude, rules, water, ..GenConfig::default() })
    }
}
