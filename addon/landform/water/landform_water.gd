@tool
class_name LandformWater
extends Node3D
## A world's water, shown: a piece for each mesh LandformWaterSurface builds, every piece drawn with the one material
## that takes every value of the look, each under its own name. A game places this scene beside its terrain, at the
## world's origin (the meshes are in the world's frame), and its parent calls show_world() after a new world is made,
## clear() to remove the water and set_shown() to hide it. It looks for nothing itself.
##
## The shader lifts the water with its distance from the camera, by the look's lift_most_m at most, for display only:
## the meshes still stand at their ground plus their depth. It is there because Terrain3D draws distant ground from
## points farther apart than 1 m, so beyond some hundred metres a bed flattens and the ground would cover its water;
## the game's camera, 40 m from its focus, sees no water far enough away to be lifted. Every piece's box is grown by
## lift_most_m (extra_cull_margin), so a flat piece is not culled by its unlifted box.

## The layer the extension builds the surface from.
const WATER_LAYER := "water"
## What show_world() answers for a world that has rivers or lakes but did not keep the water layer.
const NO_LAYER_REASON := "The world's config did not keep the water layer, so its water is not drawn: add \"water\" to export_layers."
## What show_world() answers when it has water to show and no look to show it with.
const NO_LOOK_REASON := "The water has no look, so it is not drawn: give it a LandformWaterLook."

## The look the material is given; the scene ships with water_look.tres. It is read when a world is shown: after
## changing the look, or a value of it, show the world again.
@export var look: LandformWaterLook
## One piece of the water; the scene holds a MeshInstance3D that casts no shadow.
@export var piece_scene: PackedScene
## The material every piece shares. Local to the scene, so each water node has its own.
@export var material: ShaderMaterial

var _pieces: Array[MeshInstance3D] = []
var _triangles := 0


## Shows this world's water in place of whatever was shown. Returns "" when the water is shown or the world has none
## (no world, an empty one, one without rivers and lakes). Returns a reason, and shows nothing, when the world has
## rivers or lakes but its config did not keep the water layer (NO_LAYER_REASON), or when it has the layer but this
## node has no look to show it with (NO_LOOK_REASON).
func show_world(world: LandformWorld) -> String:
	clear()
	if world == null or world.get_size() == 0:
		return ""
	if world.get_river_count() + world.get_lake_count() == 0:
		return ""
	if not world.get_layer_names().has(WATER_LAYER):
		return NO_LAYER_REASON
	if look == null:
		return NO_LOOK_REASON
	_hand_over_look()
	for mesh: ArrayMesh in LandformWaterSurface.build(world):
		var piece := piece_scene.instantiate() as MeshInstance3D
		piece.mesh = mesh
		piece.material_override = material
		piece.extra_cull_margin = look.lift_most_m
		add_child(piece)
		_pieces.append(piece)
		# Three corners a triangle, so the division is exact.
		@warning_ignore("integer_division")
		_triangles += mesh.surface_get_array_index_len(0) / 3
	return ""


## Removes the water shown. The pieces leave the tree at once and are freed at the end of the frame.
func clear() -> void:
	for piece: MeshInstance3D in _pieces:
		remove_child(piece)
		piece.queue_free()
	_pieces.clear()
	_triangles = 0


## Shows or hides the water without rebuilding it.
func set_shown(on: bool) -> void:
	visible = on


## How many pieces are shown.
func piece_count() -> int:
	return _pieces.size()


## How many triangles all the pieces hold.
func triangle_count() -> int:
	return _triangles


## Gives the material every script variable of the look, each under its own name: the shader has a uniform of that
## name for each, so a value added to the look and the shader needs nothing here.
func _hand_over_look() -> void:
	for property: Dictionary in look.get_property_list():
		var usage: int = property["usage"]
		if usage & PROPERTY_USAGE_SCRIPT_VARIABLE:
			var property_name: String = property["name"]
			material.set_shader_parameter(property_name, look.get(property_name))
