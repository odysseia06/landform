//! A world's water as meshes: the core's surface, one `ArrayMesh` for each of its pieces.

use godot::classes::mesh::{ArrayCustomFormat, ArrayFormat, ArrayType, PrimitiveType};
use godot::classes::ArrayMesh;
use godot::prelude::*;
use landform_core::{water_surface, GenConfig, Grid, SurfacePiece, WATER_LAYER};

use crate::pool;
use crate::world::{or_report, LandformWorld};

/// How many floats a point's custom channel holds: the flow's `x` and `z`, the depth, the fall.
const CUSTOM_FLOATS: usize = 4;

/// A world's water as meshes. Static functions only: `LandformWaterSurface.build(world)`. The class
/// holds no state; the meshes it makes are Godot's resources, so an editor reload leaves them be.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformWaterSurface {}

#[godot_api]
impl LandformWaterSurface {
    /// One `ArrayMesh` per piece of `world`'s surface, in the world's frame (centred on the origin,
    /// like its course points), each one surface of triangles: `ARRAY_VERTEX`, `ARRAY_NORMAL` (up),
    /// `ARRAY_INDEX`, and `ARRAY_CUSTOM0` as four floats per point: the flow's x and z in metres per
    /// second, the depth in metres, the fall. Empty for a world whose `water` layer marks no water.
    /// Reports an error and returns an empty array for no world, an empty world and a world without
    /// the `water` layer (the message `LandformWorld.get_layer_image("water")` gives).
    #[func]
    fn build(world: Option<Gd<LandformWorld>>) -> Array<Gd<ArrayMesh>> {
        or_report(meshes(world)).unwrap_or_default()
    }
}

/// The four floats of each of `piece`'s points, point by point: the flow's `x` and `z`, the depth,
/// the fall.
fn custom0(piece: &SurfacePiece) -> Vec<f32> {
    let points = piece.velocity.iter().zip(&piece.depth_m).zip(&piece.fall);
    points.flat_map(|((velocity, &depth), &fall)| [velocity[0], velocity[1], depth, fall]).collect()
}

/// A float grid of `size` x `size` cells of `cell_m` metres over the bytes of an `Image.FORMAT_RF`.
fn grid_from(bytes: &[u8], size: usize, cell_m: f32) -> Grid<f32> {
    let data = bytes.as_chunks::<4>().0.iter().map(|&value| f32::from_ne_bytes(value)).collect();
    Grid { width: size, height: size, cell_size: cell_m, data }
}

/// The meshes of `world`'s surface, or why it has none.
fn meshes(world: Option<Gd<LandformWorld>>) -> Result<Array<Gd<ArrayMesh>>, String> {
    let world = world.ok_or("No world was given.")?;
    let world = world.bind();
    let depth = world.layer(WATER_LAYER)?.get_data();
    let heights = world.heights()?.get_data();
    let size = world.side();
    // The pool reads Godot's copies of the bytes and the world's water; this thread waits in
    // `pool::run` meanwhile. The binding generates every world with the core's default cell size.
    let (heights, depth, water) = (heights.as_slice(), depth.as_slice(), world.water());
    let cell_m = GenConfig::default().cell_size_m;
    let surface = pool::run(|| water_surface(&grid_from(heights, size, cell_m), &grid_from(depth, size, cell_m), water))?;
    let surface = surface.map_err(|e| e.to_string())?;
    Ok(surface.pieces.iter().map(mesh_of).collect())
}

/// One surface of triangles holding `piece`: points, upward normals, indices and the custom channel.
fn mesh_of(piece: &SurfacePiece) -> Gd<ArrayMesh> {
    let points: PackedVector3Array = piece.positions.iter().map(|&[x, y, z]| Vector3::new(x, y, z)).collect();
    let normals: PackedVector3Array = std::iter::repeat_n(Vector3::UP, points.len()).collect();
    let indices: PackedInt32Array = piece.indices.iter().map(|&index| index as i32).collect();
    let custom = PackedFloat32Array::from(custom0(piece));
    debug_assert_eq!(custom.len(), CUSTOM_FLOATS * points.len());

    let mut arrays = VarArray::new();
    arrays.resize(ArrayType::MAX.ord() as usize, &Variant::nil());
    arrays.set(ArrayType::VERTEX.ord() as usize, &points.to_variant());
    arrays.set(ArrayType::NORMAL.ord() as usize, &normals.to_variant());
    arrays.set(ArrayType::CUSTOM0.ord() as usize, &custom.to_variant());
    arrays.set(ArrayType::INDEX.ord() as usize, &indices.to_variant());
    // Without the format flag Godot reads the channel as four bytes a point.
    let custom_format = ArrayFormat::from_ord((ArrayCustomFormat::RGBA_FLOAT.ord() as u64) << ArrayFormat::CUSTOM0_SHIFT.ord());
    let mut mesh = ArrayMesh::new_gd();
    mesh.add_surface_from_arrays_ex(PrimitiveType::TRIANGLES, &arrays).flags(custom_format).done();
    mesh
}

/// Nothing here calls rayon: the test in `pool.rs` checks that nothing in this test binary starts
/// rayon's global pool.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_custom_channel_holds_the_flow_the_depth_and_the_fall_of_each_point_in_order() {
        let piece = SurfacePiece {
            column: 0,
            row: 0,
            positions: vec![[0.0, 1.0, 0.0], [1.0, 1.5, 0.0], [0.0, 2.0, 1.0]],
            depth_m: vec![0.5, 0.0, 1.25],
            velocity: vec![[0.25, -0.5], [0.0, 0.0], [-1.5, 2.0]],
            fall: vec![0.125, 0.0, 0.0625],
            indices: vec![0, 1, 2],
        };
        assert_eq!(custom0(&piece), [0.25, -0.5, 0.5, 0.125, 0.0, 0.0, 0.0, 0.0, -1.5, 2.0, 1.25, 0.0625]);
    }
}
