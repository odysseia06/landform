@tool
class_name LandformTerrainLook
extends Resource
## How a land's ground is painted in Terrain3D, which the project supplies: the Terrain3D textures, which texture each
## ground material of the land's ground map uses, the two tints that follow vigour, the shine on wet ground and the
## size of the patches that share one texture rotation. LandformLandView reads it; a game may extend it with what else
## its own scenes paint, such as grass.

## The ground textures.
@export var assets: Terrain3DAssets
## Material name to the id of its texture in `assets`.
@export var material_textures: Dictionary[String, int] = {}
## The tint of ground of no vigour and of full vigour, mixed by the vigour layer (LandformTerrain3D.color_image).
@export var dry_tint := Color(1.0, 0.95, 0.78)
@export var lush_tint := Color(0.82, 1.0, 0.8)
## How much glossier the wettest ground is than dry ground.
@export_range(0.0, 0.5) var wet_gloss: float = 0.2
## Size in metres of the patches that share one texture rotation; 0 turns rotation off.
@export var rotation_patch_m: float = 24.0


## The texture id for each name, in order; empty, with an error naming the problem, when the look cannot paint them.
func texture_ids(material_names: PackedStringArray) -> PackedInt32Array:
	var found := problem(material_names)
	if not found.is_empty():
		push_error(found)
		return PackedInt32Array()
	var ids := PackedInt32Array()
	for material_name: String in material_names:
		ids.append(material_textures[material_name])
	return ids


## A sentence saying what keeps this look from painting these ground materials, or an empty string when it is sound:
## no texture assets, a material without an entry in the table, an id no asset has, or two materials on one id.
func problem(material_names: PackedStringArray) -> String:
	if assets == null:
		return "The look has no texture assets."
	var asset_ids := _asset_ids()
	var owners: Dictionary[int, String] = {}
	for material_name: String in material_names:
		if not material_textures.has(material_name):
			return "The look has no texture for the ground material \"%s\"." % material_name
		var id := material_textures[material_name]
		if not asset_ids.has(id):
			return "The look gives the ground material \"%s\" texture id %d, but no texture asset has that id." % [material_name, id]
		if owners.has(id):
			return "The look gives the ground materials \"%s\" and \"%s\" the same texture id %d." % [owners[id], material_name, id]
		owners[id] = material_name
	return ""


## The id of every texture asset the look holds.
func _asset_ids() -> Array[int]:
	var asset_ids: Array[int] = []
	if assets == null:
		return asset_ids
	for asset: Terrain3DTextureAsset in assets.texture_list:
		if asset != null:
			asset_ids.append(asset.id)
	return asset_ids
