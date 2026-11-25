//! The water rules as a Godot resource, mirroring the core's `WaterRules`; and a world's water as
//! plain Godot values, for the world's saved state.

use godot::prelude::*;
use landform_core::{CourseEnd, CoursePoint, LakeWater, RiverCourse, Water, WaterRules};

use crate::world::{read, LandformWorld};

/// One field of the core's default water rules, as the float Godot shows.
fn default_of(field: fn(&WaterRules) -> f32) -> f64 {
    f64::from(field(&WaterRules::default()))
}

/// How the water of a world grown from a drawing lies: one number per field of the core's
/// `WaterRules`, under the same name, each starting at the core's default. Widths, depths, heights
/// and distances are in metres, areas in square metres. A `LandformConfig` without one uses the
/// defaults; a config whose source is not the drawing does not read it.
#[derive(GodotClass)]
#[class(tool, init, base = Resource)]
pub struct LandformWaterRules {
    /// A brook starts where this many square metres of ground drain; from 1,000 to 100,000,000.
    #[export]
    #[init(val = default_of(|r| r.brook_start_m2))]
    brook_start_m2: f64,
    /// A brook's width in metres where it starts; from 0.2 to 20.
    #[export]
    #[init(val = default_of(|r| r.brook_width_m))]
    brook_width_m: f64,
    /// How a brook widens with the ground it drains: its width is `brook_width_m` times the ground
    /// drained over `brook_start_m2`, to this power; from 0.1 to 1.
    #[export]
    #[init(val = default_of(|r| r.width_area_power))]
    width_area_power: f64,
    /// The water a river entering at the world's edge brings into the land, as square metres of
    /// ground drained, when it is `river_inflow_width_m` wide; it scales with the width to the power
    /// 0.5. From 0 to 1,000,000,000.
    #[export]
    #[init(val = default_of(|r| r.river_inflow_m2))]
    river_inflow_m2: f64,
    /// The width in metres of a river that brings `river_inflow_m2`; from 1 to 200.
    #[export]
    #[init(val = default_of(|r| r.river_inflow_width_m))]
    river_inflow_width_m: f64,
    /// A bed's depth as a share of the water's width before the wander, divided by how much the
    /// wander widened it there, kept from `bed_depth_min_m` to `bed_depth_max_m`; from 0.01 to
    /// 0.5.
    #[export]
    #[init(val = default_of(|r| r.bed_depth_share))]
    bed_depth_share: f64,
    /// The shallowest a bed is, in metres; from 0 to 5, and not above `bed_depth_max_m`.
    #[export]
    #[init(val = default_of(|r| r.bed_depth_min_m))]
    bed_depth_min_m: f64,
    /// The deepest a bed is, in metres; from 0 to 10.
    #[export]
    #[init(val = default_of(|r| r.bed_depth_max_m))]
    bed_depth_max_m: f64,
    /// The banks' slope, as metres across per metre up; from 0.5 to 10.
    #[export]
    #[init(val = default_of(|r| r.bank_run))]
    bank_run: f64,
    /// The water's surface lies this share of the bed's depth above the bed; from 0.1 to 1.
    #[export]
    #[init(val = default_of(|r| r.water_fill))]
    water_fill: f64,
    /// Ground less than this many metres above the nearest water surface is fully wet; from 0 to 10,
    /// and below `bank_wet_none_m`.
    #[export]
    #[init(val = default_of(|r| r.bank_wet_full_m))]
    bank_wet_full_m: f64,
    /// The wetting fades from `bank_wet_full_m` above the nearest water surface to no effect this many
    /// metres above it; from 0 to 10.
    #[export]
    #[init(val = default_of(|r| r.bank_wet_none_m))]
    bank_wet_none_m: f64,
    /// The water wets ground only within this many metres of it; from 0 to 200.
    #[export]
    #[init(val = default_of(|r| r.bank_wet_reach_m))]
    bank_wet_reach_m: f64,
    /// A lake's shore is graded to no steeper than this, as metres across per metre up, within
    /// `shore_reach_m` of the water; from 0.5 to 10.
    #[export]
    #[init(val = default_of(|r| r.shore_run))]
    shore_run: f64,
    /// How far from a lake's water its shore is graded, in metres; from 0 to 64.
    #[export]
    #[init(val = default_of(|r| r.shore_reach_m))]
    shore_reach_m: f64,
    /// How far a channel's width drifts from its own, as a share of it; from 0 to 0.5.
    #[export]
    #[init(val = default_of(|r| r.width_wander))]
    width_wander: f64,
    /// The length of one drift of the width, in widths of the channel; from 3 to 20.
    #[export]
    #[init(val = default_of(|r| r.wander_length_widths))]
    wander_length_widths: f64,
    /// How far the deep line moves toward the outside of the tightest bend, as a share of the half
    /// width; from 0 to 0.5.
    #[export]
    #[init(val = default_of(|r| r.bend_shift))]
    bend_shift: f64,
}

impl LandformWaterRules {
    /// The rules as the core's `WaterRules`, or the message of its `validate`, which names the
    /// field, when they cannot work. Every field is named, with no default to fill the rest, so a
    /// field added to the core fails to compile until it is added here.
    pub fn to_core(&self) -> Result<WaterRules, String> {
        // Godot's floats have 64 bits and the core's 32: a value beyond their range becomes
        // infinite, which `validate` refuses.
        let rules = WaterRules {
            brook_start_m2: self.brook_start_m2 as f32,
            brook_width_m: self.brook_width_m as f32,
            width_area_power: self.width_area_power as f32,
            river_inflow_m2: self.river_inflow_m2 as f32,
            river_inflow_width_m: self.river_inflow_width_m as f32,
            bed_depth_share: self.bed_depth_share as f32,
            bed_depth_min_m: self.bed_depth_min_m as f32,
            bed_depth_max_m: self.bed_depth_max_m as f32,
            bank_run: self.bank_run as f32,
            water_fill: self.water_fill as f32,
            bank_wet_full_m: self.bank_wet_full_m as f32,
            bank_wet_none_m: self.bank_wet_none_m as f32,
            bank_wet_reach_m: self.bank_wet_reach_m as f32,
            shore_run: self.shore_run as f32,
            shore_reach_m: self.shore_reach_m as f32,
            width_wander: self.width_wander as f32,
            wander_length_widths: self.wander_length_widths as f32,
            bend_shift: self.bend_shift as f32,
        };
        rules.validate().map_err(|e| e.to_string())?;
        Ok(rules)
    }
}

/// An index that may be absent as Godot's int: -1 for none.
pub(crate) fn index_or_none(index: Option<usize>) -> i64 {
    index.map_or(-1, |index| index as i64)
}

/// `index_or_none`'s int back as an index, when it is -1 or names one of `count` items.
fn index_from(value: i64, count: usize) -> Option<Option<usize>> {
    if value == -1 {
        return Some(None);
    }
    usize::try_from(value).ok().filter(|&index| index < count).map(Some)
}

/// How a course ends as Godot's two ints: one of the `RIVER_ENDS_*` constants, and the index of
/// the river or lake it ends in, -1 at the edge.
pub(crate) fn end_code(end: CourseEnd) -> (i64, i64) {
    match end {
        CourseEnd::Edge => (LandformWorld::RIVER_ENDS_AT_EDGE, -1),
        CourseEnd::River(river) => (LandformWorld::RIVER_ENDS_IN_RIVER, river as i64),
        CourseEnd::Lake(lake) => (LandformWorld::RIVER_ENDS_IN_LAKE, lake as i64),
    }
}

/// `end_code`'s two ints back as a course end, when the index names one of `rivers` or `lakes`.
fn end_from(code: i64, index: i64, rivers: usize, lakes: usize) -> Option<CourseEnd> {
    match code {
        LandformWorld::RIVER_ENDS_AT_EDGE if index == -1 => Some(CourseEnd::Edge),
        LandformWorld::RIVER_ENDS_IN_RIVER => index_from(index, rivers)?.map(CourseEnd::River),
        LandformWorld::RIVER_ENDS_IN_LAKE => index_from(index, lakes)?.map(CourseEnd::Lake),
        _ => None,
    }
}

/// How many numbers a river's point takes in the state: `x_m`, `z_m`, `surface_m`, `width_m` and
/// `depth_m`.
const POINT_NUMBERS: usize = 5;

/// The water as plain dictionaries, arrays and numbers, for `_get_state`: `rivers`, one dictionary
/// per course holding `drawn`, `from_lake`, `end` and `end_index` as the world's `get_river_*`
/// give them and `points`, a `PackedFloat32Array` of `POINT_NUMBERS` numbers per point; and
/// `lakes`, one dictionary per lake holding `drawn`, `level_m`, `x_m`, `z_m` and `area_m2`.
pub(crate) fn water_state(water: &Water) -> VarDictionary {
    let rivers: VarArray = (water.rivers.iter())
        .map(|river| {
            let (end, end_index) = end_code(river.end);
            let points: PackedFloat32Array = (river.points.iter())
                .flat_map(|point| [point.x_m, point.z_m, point.surface_m, point.width_m, point.depth_m])
                .collect();
            let river = vdict! {
                "drawn" => index_or_none(river.drawn),
                "from_lake" => index_or_none(river.from_lake),
                "end" => end,
                "end_index" => end_index,
                "points" => &points,
            };
            river.to_variant()
        })
        .collect();
    let lakes: VarArray = (water.lakes.iter())
        .map(|lake| {
            let lake = vdict! {
                "drawn" => lake.drawn as i64,
                "level_m" => lake.level_m,
                "x_m" => lake.x_m,
                "z_m" => lake.z_m,
                "area_m2" => lake.area_m2,
            };
            lake.to_variant()
        })
        .collect();
    vdict! { "rivers" => &rivers, "lakes" => &lakes }
}

/// The water `water_state` wrote, or `None` when any part of it is missing or of the wrong type, a
/// river's points do not come in whole `POINT_NUMBERS`, or an index names no river or lake.
pub(crate) fn water_from(state: &VarDictionary) -> Option<Water> {
    let list = |key: &str| -> Option<Vec<VarDictionary>> {
        let items: VarArray = read(state.get(key))?;
        items.iter_shared().map(|item| item.try_to::<VarDictionary>().ok()).collect()
    };
    let (rivers, lakes) = (list("rivers")?, list("lakes")?);
    let lakes: Vec<LakeWater> = (lakes.iter())
        .map(|lake| {
            let number = |key: &str| read::<f32>(lake.get(key));
            Some(LakeWater {
                drawn: usize::try_from(read::<i64>(lake.get("drawn"))?).ok()?,
                level_m: number("level_m")?,
                x_m: number("x_m")?,
                z_m: number("z_m")?,
                area_m2: number("area_m2")?,
            })
        })
        .collect::<Option<_>>()?;
    let rivers = (rivers.iter())
        .map(|river| {
            let int = |key: &str| read::<i64>(river.get(key));
            let numbers: PackedFloat32Array = read(river.get("points"))?;
            let (points, rest) = numbers.as_slice().as_chunks::<POINT_NUMBERS>();
            if !rest.is_empty() {
                return None;
            }
            Some(RiverCourse {
                // A drawn river's index names one of the drawing's, which the world does not keep.
                drawn: index_from(int("drawn")?, usize::MAX)?,
                from_lake: index_from(int("from_lake")?, lakes.len())?,
                points: (points.iter())
                    .map(|&[x_m, z_m, surface_m, width_m, depth_m]| CoursePoint { x_m, z_m, surface_m, width_m, depth_m })
                    .collect(),
                end: end_from(int("end")?, int("end_index")?, rivers.len(), lakes.len())?,
            })
        })
        .collect::<Option<_>>()?;
    Some(Water { rivers, lakes })
}
