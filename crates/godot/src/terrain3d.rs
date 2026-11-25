//! Terrain3D's map formats, and only here: a world's ground map as Terrain3D's control map and its
//! vigour and wetness layers as Terrain3D's colour map.

use godot::classes::image::Format;
use godot::classes::Image;
use godot::prelude::*;
use landform_core::rng::mix64;
use landform_core::{fbm, GenConfig, Material};
use rayon::prelude::*;

use crate::pool;
use crate::world::{image_from, or_report, LandformWorld};

/// How many materials a ground map names, so how many texture ids a control map needs.
const MATERIALS: usize = Material::ALL.len();
/// Terrain3D's texture ids have five bits.
const MAX_TEXTURE_ID: u8 = 31;
/// The bytes of "rotate" and a version: the rotation field's seed is `mix64(seed ^ ROTATION_TAG)`.
const ROTATION_TAG: u64 = 0x726F_7461_7465_0001;
const ROTATION_OCTAVES: u32 = 2;
/// Terrain3D turns a texture in sixteen steps of a sixteenth of a turn.
const ROTATION_STEPS: f32 = 16.0;
/// The colour map's alpha that leaves Terrain3D's roughness as it is; lower is glossier.
const NEUTRAL_ROUGHNESS: f32 = 0.5;
/// The largest `wet_gloss`: the wettest ground's alpha then reaches 0 and goes no lower.
const MAX_WET_GLOSS: f64 = NEUTRAL_ROUGHNESS as f64;
/// The layers the colour map is made from.
const VIGOUR: &str = "vigour";
const WETNESS: &str = "wetness";
/// The largest and the smallest side of a Terrain3D region, in cells (`Terrain3D.SIZE_1024` and
/// `Terrain3D.SIZE_64`): a world is shown in regions of one of the sizes between, each half the one
/// above it.
const REGION_SIZE_MOST: i64 = 1024;
const REGION_SIZE_LEAST: i64 = 64;
/// How many regions a side Terrain3D's region map holds (`Terrain3DData.REGION_MAP_SIZE`), so the
/// widest world regions of a size can show.
const REGION_MAP_SIDE: i64 = 32;

/// Terrain3D's control map layout (src/terrain_3d_util.h): base id, overlay id, blend, UV rotation.
fn control_bits(base: u8, overlay: u8, blend: u8, rotation: u8) -> u32 {
    (u32::from(base & 0x1F) << 27)
        | (u32::from(overlay & 0x1F) << 22)
        | (u32::from(blend) << 14)
        | (u32::from(rotation & 0xF) << 10)
}

/// Hands a `LandformWorld` to Terrain3D as its control and colour maps. Static functions only:
/// `LandformTerrain3D.control_image(world, ...)`.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformTerrain3D {}

#[godot_api]
impl LandformTerrain3D {
    /// Terrain3D's control map of `world` as an `Image.FORMAT_RF`, `size` x `size`, cell `(x, z)`
    /// at pixel `(x, z)`: each pixel's 32 bits hold the cell's strongest material as the base
    /// texture, its second as the overlay, its blend and a texture rotation, as `Terrain3DUtil`
    /// reads them. `texture_ids[m]` is the Terrain3D texture of material index `m`, in the order of
    /// `LandformWorld.get_material_names()`. `rotation_patch_m` is the size in metres of the patches
    /// that turn the textures, neighbouring patches by small steps; 0 turns nothing. Reports an
    /// error and returns `null` for an empty world, for ids that are not six numbers from 0 to 31,
    /// and for a patch size that is negative or not a finite number.
    #[func]
    fn control_image(world: Option<Gd<LandformWorld>>, texture_ids: PackedInt32Array, rotation_patch_m: f64) -> Option<Gd<Image>> {
        or_report(control(world, &texture_ids, rotation_patch_m))
    }

    /// Terrain3D's colour map of `world` as an `Image.FORMAT_RGBA8`, `size` x `size`, cell `(x, z)`
    /// at pixel `(x, z)`: the colour mixes `dry_tint` and `lush_tint` by the `vigour` layer (their
    /// alpha is not used), and the alpha, Terrain3D's roughness modifier, is
    /// `0.5 - wet_gloss * wetness`, so 0.5 leaves the roughness alone and wet ground is glossier.
    /// Reports an error and returns `null` for an empty world, for a world whose config did not
    /// export `vigour` and `wetness`, and for a `wet_gloss` outside 0 to 0.5.
    #[func]
    fn color_image(world: Option<Gd<LandformWorld>>, dry_tint: Color, lush_tint: Color, wet_gloss: f64) -> Option<Gd<Image>> {
        or_report(color(world, dry_tint, lush_tint, wet_gloss))
    }

    /// The side, in cells, of the Terrain3D regions a world of `world_size` cells is shown in: the
    /// largest of Terrain3D's sizes, from 1,024 down to 64, whose regions the world fills whole when
    /// centred on the origin (half its side a whole number of regions) and that Terrain3D's region
    /// map, 32 regions a side, can hold; 0 when there is none, and for a world of no cells. So a
    /// world of 2,048 cells takes four regions of 1,024, one of 1,024 cells four of 512 (not four of
    /// 1,024, three quarters of which would be flat ground around it), one of 128 four of 64, and
    /// one of 1,000 none. The terrain preview's `TerrainPreview.region_size_for` is the same rule.
    #[func]
    fn region_size_for(world_size: i64) -> i64 {
        region_size(world_size)
    }
}

/// `LandformTerrain3D.region_size_for`'s rule.
fn region_size(world_size: i64) -> i64 {
    if world_size <= 0 {
        return 0;
    }
    let mut size = REGION_SIZE_MOST;
    while size >= REGION_SIZE_LEAST {
        if world_size % (2 * size) == 0 && world_size <= size * REGION_MAP_SIDE {
            return size;
        }
        size /= 2;
    }
    0
}

/// `texture_ids` as one Terrain3D texture id per material, or why they cannot be.
fn texture_table(texture_ids: &PackedInt32Array) -> Result<[u8; MATERIALS], String> {
    let ids = texture_ids.as_slice();
    if ids.len() != MATERIALS {
        let names: Vec<&str> = Material::ALL.iter().map(|material| material.name()).collect();
        return Err(format!(
            "texture_ids must hold {MATERIALS} Terrain3D texture ids, one for each of {}, not {}.",
            names.join(", "),
            ids.len()
        ));
    }
    let mut table = [0; MATERIALS];
    for ((slot, &id), material) in table.iter_mut().zip(ids).zip(Material::ALL) {
        *slot = u8::try_from(id).ok().filter(|&id| id <= MAX_TEXTURE_ID).ok_or_else(|| {
            format!(
                "texture_ids holds {id} for {}: Terrain3D's texture ids run from 0 to {MAX_TEXTURE_ID}.",
                material.name()
            )
        })?;
    }
    Ok(table)
}

/// The noise field that turns the textures, in metres.
#[derive(Clone, Copy)]
struct RotationField {
    seed: u64,
    patch_m: f64,
    cell_m: f64,
}

impl RotationField {
    /// Terrain3D's rotation step at the middle of cell `(x, z)`.
    fn step_at(&self, x: usize, z: usize) -> u8 {
        let (x_m, z_m) = ((x as f64 + 0.5) * self.cell_m, (z as f64 + 0.5) * self.cell_m);
        rotation_step(fbm(self.seed, x_m, z_m, ROTATION_OCTAVES, self.patch_m))
    }
}

/// A field value in -1..1 as one of Terrain3D's sixteen rotation steps, 0 to 15, in sixteen equal
/// parts: values close together give the same step or neighbouring ones.
fn rotation_step(value: f32) -> u8 {
    ((value + 1.0) * 0.5 * ROTATION_STEPS).clamp(0.0, ROTATION_STEPS - 1.0) as u8
}

fn control(world: Option<Gd<LandformWorld>>, texture_ids: &PackedInt32Array, rotation_patch_m: f64) -> Result<Gd<Image>, String> {
    let ids = texture_table(texture_ids)?;
    if !(rotation_patch_m.is_finite() && rotation_patch_m >= 0.0) {
        return Err(format!(
            "rotation_patch_m must be 0, for no rotation, or a positive number of metres, not {rotation_patch_m}."
        ));
    }
    let world = world.ok_or("No world was given.")?;
    let world = world.bind();
    let ground = world.ground()?.get_data();
    let size = world.side();
    let rotation = (rotation_patch_m > 0.0).then(|| RotationField {
        seed: mix64(world.seed_bits() ^ ROTATION_TAG),
        patch_m: rotation_patch_m,
        // The binding generates every world with the core's default cell size.
        cell_m: f64::from(GenConfig::default().cell_size_m),
    });
    // The pool reads Godot's copy of the bytes; this thread waits in `pool::run` meanwhile.
    let cells = ground.as_slice();
    let bytes = pool::run(|| control_bytes(cells, size, &ids, rotation))?;
    Ok(image_from(size, size, Format::RF, bytes))
}

/// The control map's bytes from the ground image's. Rows run in parallel and each cell reads only
/// its own, so the result does not depend on threads. Inside `pool::run` only.
fn control_bytes(ground: &[u8], size: usize, ids: &[u8; MATERIALS], rotation: Option<RotationField>) -> Vec<u8> {
    let mut bytes = vec![0; size * size * 4];
    let rows = bytes.par_chunks_mut(size * 4).zip(ground.par_chunks(size * 3)).enumerate();
    rows.for_each(|(z, (pixels, cells))| control_row(pixels, cells, z, ids, rotation));
    bytes
}

/// Row `z` of the control map: four bytes per cell, as any `FORMAT_RF` pixel holds them, from the
/// ground image's three.
fn control_row(pixels: &mut [u8], cells: &[u8], z: usize, ids: &[u8; MATERIALS], rotation: Option<RotationField>) {
    let (pixels, cells) = (pixels.as_chunks_mut::<4>().0, cells.as_chunks::<3>().0);
    for (x, (pixel, &[strongest, second, blend])) in pixels.iter_mut().zip(cells).enumerate() {
        let turn = rotation.map_or(0, |field| field.step_at(x, z));
        *pixel = control_bits(ids[usize::from(strongest)], ids[usize::from(second)], blend, turn).to_ne_bytes();
    }
}

fn color(world: Option<Gd<LandformWorld>>, dry_tint: Color, lush_tint: Color, wet_gloss: f64) -> Result<Gd<Image>, String> {
    if !(0.0..=MAX_WET_GLOSS).contains(&wet_gloss) {
        return Err(format!("wet_gloss must be from 0 to {MAX_WET_GLOSS}, not {wet_gloss}."));
    }
    let world = world.ok_or("No world was given.")?;
    let world = world.bind();
    let vigour = world.layer(VIGOUR)?.get_data();
    let wetness = world.layer(WETNESS)?.get_data();
    let size = world.side();
    let tints = [dry_tint, lush_tint].map(|tint| [tint.r, tint.g, tint.b]);
    // The pool reads Godot's copies of the bytes; this thread waits in `pool::run` meanwhile.
    let (vigour, wetness) = (vigour.as_slice(), wetness.as_slice());
    let bytes = pool::run(|| color_bytes(vigour, wetness, size, tints, wet_gloss as f32))?;
    Ok(image_from(size, size, Format::RGBA8, bytes))
}

/// The colour map's bytes from the two layers' float bytes. Rows run in parallel. Inside
/// `pool::run` only.
fn color_bytes(vigour: &[u8], wetness: &[u8], size: usize, tints: [[f32; 3]; 2], wet_gloss: f32) -> Vec<u8> {
    let mut bytes = vec![0; size * size * 4];
    let rows = bytes.par_chunks_mut(size * 4).zip(vigour.par_chunks(size * 4)).zip(wetness.par_chunks(size * 4));
    rows.for_each(|((pixels, vigour), wetness)| color_row(pixels, vigour, wetness, tints, wet_gloss));
    bytes
}

/// One row of the colour map: four bytes per cell from the vigour and wetness pixels' floats.
fn color_row(pixels: &mut [u8], vigour: &[u8], wetness: &[u8], [dry, lush]: [[f32; 3]; 2], wet_gloss: f32) {
    let values = vigour.as_chunks::<4>().0.iter().zip(wetness.as_chunks::<4>().0);
    for (pixel, (&vigour, &wetness)) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(values) {
        let (vigour, wetness) = (f32::from_ne_bytes(vigour), f32::from_ne_bytes(wetness));
        let [r, g, b] = [0, 1, 2].map(|channel| unit_byte(dry[channel] + (lush[channel] - dry[channel]) * vigour));
        *pixel = [r, g, b, unit_byte(NEUTRAL_ROUGHNESS - wet_gloss * wetness)];
    }
}

/// A value from 0 to 1 as a byte from 0 to 255, rounded; beyond the ends it clamps, and a value
/// that is not a number gives 0.
fn unit_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Nothing here calls rayon, `control_bytes` or `color_bytes`: the test in `pool.rs` checks that
/// nothing in this test binary starts rayon's global pool.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_bits_follow_terrain3ds_layout_and_mask_ids_above_31() {
        assert_eq!(control_bits(3, 5, 128, 9), 0x1960_2400);
        assert_eq!(control_bits(32 + 3, 64 + 5, 128, 16 + 9), 0x1960_2400);
        assert_eq!(control_bits(0xFF, 0, 0, 0), 0xF800_0000, "the base spilled out of bits 27..31");
        assert_eq!(control_bits(0, 0xFF, 0, 0), 0x07C0_0000, "the overlay spilled out of bits 22..26");
        assert_eq!(control_bits(0, 0, 0xFF, 0), 0x003F_C000, "the blend spilled out of bits 14..21");
        assert_eq!(control_bits(0, 0, 0, 0xFF), 0x0000_3C00, "the rotation spilled out of bits 10..13");
    }

    #[test]
    fn a_world_takes_the_largest_regions_it_fills_whole_when_centred() {
        let sizes = [128, 512, 1000, 1024, 1536, 2048, 4096, 64, 192, 0, -2048];
        let regions = sizes.map(region_size);
        assert_eq!(regions, [64, 256, 0, 512, 256, 1024, 1024, 0, 0, 0, 0], "for {sizes:?}");
        // The region map holds 32 regions a side: a world wider than 32 of the largest fills none.
        assert_eq!(region_size(REGION_SIZE_MOST * REGION_MAP_SIDE), REGION_SIZE_MOST);
        assert_eq!(region_size(REGION_SIZE_MOST * (REGION_MAP_SIDE + 2)), 0);
    }

    #[test]
    fn the_rotation_field_maps_onto_sixteen_equal_steps() {
        assert_eq!([-1.0, -0.876, 0.0, 0.124, 0.99, 1.0, f32::NAN].map(rotation_step), [0, 0, 8, 8, 15, 15, 0]);
        assert_eq!(rotation_step(-0.874), 1);
    }

    #[test]
    fn a_control_row_maps_materials_to_texture_ids_and_turns_by_the_field() {
        let ids = [10, 11, 12, 13, 14, 15];
        // Rock blended a quarter with scree, then pure meadow.
        let cells = [Material::Rock as u8, Material::Scree as u8, 64, 0, 1, 0];
        let mut pixels = [0; 8];
        control_row(&mut pixels, &cells, 0, &ids, None);
        assert_eq!(pixels[..4], control_bits(13, 14, 64, 0).to_ne_bytes());
        assert_eq!(pixels[4..], control_bits(10, 11, 0, 0).to_ne_bytes());
        let field = RotationField { seed: 1, patch_m: 30.0, cell_m: 1.0 };
        control_row(&mut pixels, &cells, 5, &ids, Some(field));
        assert_eq!(pixels[4..], control_bits(10, 11, 0, field.step_at(1, 5)).to_ne_bytes());
    }

    #[test]
    fn rotation_steps_vary_over_a_world_and_change_by_one_between_neighbours() {
        let field = RotationField { seed: mix64(7 ^ ROTATION_TAG), patch_m: 30.0, cell_m: 1.0 };
        let steps: Vec<u8> = (0..256).map(|x| field.step_at(x, 0)).collect();
        let mut seen = steps.clone();
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() >= 4, "a 256 m row turns only to steps {seen:?}");
        let jumps = steps.windows(2).filter(|pair| pair[0].abs_diff(pair[1]) > 1).count();
        assert_eq!(jumps, 0, "neighbouring cells jump by more than one step");
    }

    #[test]
    fn a_colour_row_mixes_the_tints_by_vigour_and_lowers_the_alpha_on_wet_ground() {
        let floats = |values: [f32; 3]| -> Vec<u8> { values.iter().flat_map(|v| v.to_ne_bytes()).collect() };
        // Dry and half wet, lush and fully wet, half lush and dry.
        let (vigour, wetness) = (floats([0.0, 1.0, 0.5]), floats([0.5, 1.0, 0.0]));
        let tints = [[0.2, 0.4, 0.0], [1.0, 0.0, 0.5]];
        let mut pixels = [0; 12];
        color_row(&mut pixels, &vigour, &wetness, tints, 0.5);
        assert_eq!(pixels, [51, 102, 0, 64, 255, 0, 128, 0, 153, 51, 64, 128]);
        color_row(&mut pixels, &vigour, &wetness, tints, 0.0);
        assert_eq!([pixels[3], pixels[7], pixels[11]], [128; 3], "no gloss leaves the roughness alone");
    }
}
