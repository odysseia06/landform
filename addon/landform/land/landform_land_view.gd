@tool
class_name LandformLandView
extends Node3D
## A land shown in a scene: its ground in Terrain3D (%Terrain3D, its data directory empty, so its regions are made from
## the land each time and never saved), its water (%Water, Landform's LandformWater), and, while the land is edited in
## the editor, what the editor tool draws over it: the overlay laid on the ground (%Overlay, a Decal whose texture
## LandformOverlay.compose makes from the outline of a regrowth, the held and hand-shaped ground and the area to be
## regrown, at a pixel a metre) and the lines (%Lines, the drawing's rivers, points and lakes, a mesh LandformLandLines
## makes and land_lines.gdshader widens on the screen). In a running game the node shows its land and nothing else.
##
## The land is held in a plain variable, never in an exported or stored property, in metadata or in an editor's state
## (Godot would write the land's whole file into it). Its world is taken from the land again at each refresh() and never
## kept: refresh() imports the ground and builds the water only when the land's world is another object than the one
## shown, which it tells by the object's id, as after a regrowth swapped in or an editor reload of the extension; a step
## that changes no grid of the world (a hold, a lift, an ask, a reach) keeps the object, and refresh() then remakes the
## overlay alone; it keeps the drawing and the selection show_drawing last gave, the outline and the area show_outline
## and show_area last gave, and the held ground show_held gave in place of the land's, which the tool gives again after
## a change. A land is shown centred on the
## origin, cell (i, j) at (i - half, j - half), in the largest Terrain3D regions it fills whole
## (LandformTerrain3D.region_size_for), so its water and its lines, in the world's frame, lie on it, and the overlay's
## texel (i, j) on cell (i, j).
##
## The look and Terrain3D's material are handed over whenever they are set, also after the node is ready: Terrain3D is
## given copies of the look's assets and of the material, and a new look paints the land again. How the overlay and the
## lines are drawn is the editor tool's (its settings' look, set_overlay_colours), held in a plain variable, so no scene
## stores it.
##
## The view remembers the file the land shown was read from (open, and _ready) or given with (show_land's `path`): the
## editor tool saves the land there and nowhere else (get_land_path), whatever land_file names since.

## The land's world was imported into Terrain3D and its water built (open, show_land, or a refresh after the world
## changed), taking these milliseconds each.
signal shown(import_ms: float, water_ms: float)

## The render layer the overlay projects on, layer 20: the scene's Terrain3D holds it among its render_layers and the
## water's pieces and the lines do not, so the overlay lies on the ground alone (the scene's Overlay has it as its
## cull_mask).
const OVERLAY_LAYER_MASK := 1 << 19
## How far the overlay's box reaches above the highest ground and below the lowest, in metres, so its projection takes
## every cell; find_ground clips a ray to the same box.
const OVERLAY_MARGIN_M := 10.0
## How far west and north of the origin the overlay's box is centred, in metres: half a cell. A decal's texels span its
## box evenly, texel i from i - half to i - half + 1 of a box of the land's side centred on the origin, so its centre
## would lie half a cell east (and south) of cell i, which stands at i - half; moved so, texel i's centre is cell i.
const TEXEL_TO_CELL_M := 0.5
## What find_ground gives when the ray meets no ground.
const NO_GROUND := Vector3.INF
## Terrain3D's get_intersection gives a point whose parts are 3.4e38 or more when it meets no ground.
const TERRAIN3D_NO_HIT := 3.4e38
## A ray whose direction, made a unit, points lower than this is straight down, as Terrain3D tells one: it then gives
## the ground's height under the ray's start exactly.
const STRAIGHT_DOWN := -0.99999
## How often an oblique hit is halved between Terrain3D's step above the ground and its step at or below it: its steps
## are 1 m along the ray, so 20 halvings leave less than a micrometre.
const REFINE_HALVINGS := 20
## How many points 1 m apart Terrain3D's ray march (get_intersection in its CPU mode) checks from its start, the first
## at the start: one march reaches 3,999 m along the ray, so find_ground marches again from the last point checked.
const RAYMARCH_POINTS := 4000
## How far before the land's box find_ground starts a ray, in metres: a ray that meets the land through its side then
## starts off the land, where Terrain3D marches it, and not under the ground at the land's edge, which it refuses.
const CLIP_LEAD_M := 1.0
## The uniforms of land_lines.gdshader the look's sizes go to.
const LINE_PIXELS := "line_pixels"
const MARKER_PIXELS := "marker_pixels"
## Why show_land or refresh shows nothing new: no look to paint with.
const NO_LOOK := "The land's view has no look, so it cannot paint the land's ground: give it a LandformTerrainLook."
## Why show_land shows nothing new: the land given is empty or none.
const EMPTY_LAND := "The land is empty, so the view has nothing to show: read one with LandformLand.load_file."
## Why refresh shows nothing: the view shows no land.
const NO_LAND := "The land's view shows no land: open a land file, or give it a land with show_land."
## Why show_land or refresh shows nothing new: the land fills no whole Terrain3D regions, with its side in metres.
const NO_REGIONS := "A land of %d m fills no whole Terrain3D regions when centred on the origin (LandformTerrain3D.region_size_for), so the view cannot show it."
## Why show_land or refresh shows nothing new: the extension refused to make the land's control or colour map (a world
## without the vigour or wetness layer, or a look whose wet_gloss or rotation_patch_m it refuses), and said why.
const NOT_PAINTED := "The land's ground could not be painted; see the error the extension printed."

## The land file the view opens (open()): when the scene is opened in the editor, when a game runs it, and when asked.
@export_file("*.lfland") var land_file := ""
## How the ground is painted: the project's ground textures, which texture each ground material uses, the tints.
## Terrain3D is given a copy of its assets, so nothing Terrain3D does reaches the look or its file. Set once the node is
## ready, it is handed over at once and the land shown is painted again with it.
@export var look: LandformTerrainLook:
	set(value):
		look = value
		if is_node_ready():
			_hand_over_assets()
			_paint_again()
## Terrain3D's material, the project's; empty, the scene's own. Terrain3D is given a copy, so it never writes the
## project's file. Set once the node is ready, it is handed over at once.
@export var terrain_material: Terrain3DMaterial:
	set(value):
		terrain_material = value
		if is_node_ready():
			_hand_over_material()
## What a new land grows with: its detail, ground rules and water rules (the editor tool reads it; the view does not).
@export var config: LandformConfig

## Milliseconds the last world shown took to import into Terrain3D (its maps made and imported), to build its water,
## and the last overlay and lines took to make.
var last_import_ms := 0.0
var last_water_ms := 0.0
var last_overlay_ms := 0.0
var last_lines_ms := 0.0
## Why the water of the last world shown was not drawn, as LandformWater.show_world answers; empty when it was.
var last_water_reason := ""

## The land shown; never exported or stored.
var _land: LandformLand
## The file the land shown was read from or given with, "" for none; why the land file did not load or show when last
## opened (open, _ready), "" when it did.
var _land_path := ""
var _open_error := ""
## The instance id of the world last imported, 0 for none: the world itself is not kept.
var _world_id := 0
## What the overlay draws besides the land's marks, and the drawing and selection the lines draw.
var _outline: LandformOutline
var _area: Image
var _drawing: LandformDrawing
var _selection: Dictionary = {}
## The land's marks as the overlay last drew them, and the held ground drawn in place of the land's (show_held).
var _held: Image
var _hand: Image
var _held_shown: Image
## Whether the overlay and the lines are shown: in the editor, not in a running game.
var _editing := Engine.is_editor_hint()
## The land's ground as the lines lay a drawing on it (get_ground), and the instance id of the world it was made from.
var _ground: LandformLandLines.Ground
var _ground_of := 0
## The overlay's picture and the texture the decal shows it through, and the lines' mesh. Two textures take turns:
## the one the decal does not show is written in place and then given to it, since Godot's decals draw from an atlas
## that only a change of the decal's texture (or a texture replaced) marks for rebuilding, and writing in place makes
## no new texture of the land's size at each outline, as `ImageTexture.set_image` does.
var _overlay_image: Image
var _overlay_texture: ImageTexture
var _overlay_spare: ImageTexture
var _lines_mesh: ArrayMesh
## How the overlay and the lines are drawn (set_overlay_colours), and the look drawn with when none is given.
var _overlay_look: LandformOverlayLook
var _shipped_look := LandformOverlayLook.new()
## Terrain3D's material as the scene holds it, given back when terrain_material is emptied.
var _scene_material: Terrain3DMaterial
## Whether a save of the scene is under way with the overlay's texture and the lines' mesh taken off (_notification).
var _saving := false


func _ready() -> void:
	_scene_material = (%Terrain3D as Terrain3D).material
	_hand_over_material()
	_hand_over_assets()
	_hand_over_line_sizes()
	_show_editing()
	if land_file.is_empty():
		return
	# A file that does not load has been reported by the extension already; a land the view refuses is reported here.
	var land := LandformLand.load_file(land_file)
	if land == null:
		_open_error = LandformLand.get_last_error()
		return
	_open_error = show_land(land, land_file)
	if not _open_error.is_empty():
		push_error(_open_error)


## Reads land_file and shows its land (show_land), the file then the land's path. Returns "" when it is shown, else the
## sentence, which get_open_error keeps: the extension's for a file that does not load (which the extension has
## printed as an error too), show_land's for a land it refuses; the view then shows what it showed before, which at
## the start is nothing.
func open() -> String:
	var land := LandformLand.load_file(land_file)
	_open_error = LandformLand.get_last_error() if land == null else show_land(land, land_file)
	return _open_error


## Why the land file did not load or show when it was last opened (open, and _ready when land_file is set), "" when it
## did or none was opened; the editor tool says it rather than read the file again.
func get_open_error() -> String:
	return _open_error


## The file the land shown was read from (open) or given with (show_land's `path`), "" for none: the file the editor
## tool saves it to, whatever land_file names since.
func get_land_path() -> String:
	return _land_path


## Shows this land in place of any other: its world imported whole, its water built, and while editing, its marks in
## the overlay and its asked drawing in the lines, with no outline, no area and nothing selected; `path` is the file it
## was read from or written to ("" for none), get_land_path's from then on. Returns "" when it is shown, else the
## sentence and the view as it was: an empty land, one that fills no whole Terrain3D regions when centred, no look, or
## a look that cannot paint the land's ground materials.
func show_land(land: LandformLand, path := "") -> String:
	if land == null or land.get_size() == 0:
		return EMPTY_LAND
	# What painting needs is made before anything of the view changes, so a land refused leaves it as it was.
	var maps: Array[Image] = []
	var refused := _paint(land.get_world(), maps)
	if not refused.is_empty():
		return refused
	_land = land
	_land_path = path
	_world_id = 0
	_outline = null
	_area = null
	_held_shown = null
	_drawing = land.get_asked_drawing()
	_selection = {}
	return _refresh(maps)


## Shows the land as it is now: when its world is another object than the one shown, the world's maps imported whole
## and its water built (emitting shown) and the lines laid on the new ground; and while editing, the overlay made again
## from the land's held and hand pictures, the outline and the area. The drawing and selection the lines draw, and the
## outline and area the overlay draws, are those last given (show_drawing, show_outline, show_area): refresh does not
## read the land's asked drawing again. Returns "" or why nothing new is shown (no land, or show_land's refusals), the
## view then as it was.
func refresh() -> String:
	return _refresh([])


## refresh(), the world's control and colour maps already made when `maps` holds them.
func _refresh(maps: Array[Image]) -> String:
	if _land == null or _land.get_size() == 0:
		return NO_LAND
	if _saving:
		# A save that took the overlay's texture and the lines' mesh off never said it ended.
		_give_back_saved()
	var world := _land.get_world()
	var imported := world.get_instance_id() != _world_id
	if imported:
		if maps.is_empty():
			var refused := _paint(world, maps)
			if not refused.is_empty():
				return refused
		_show_world(world, maps)
		_world_id = world.get_instance_id()
	_hand_over_outline_overlay()
	if _editing:
		_held = _land.get_held_image()
		_hand = _land.get_hand_image()
		_make_overlay()
		if imported:
			_make_lines()
	if imported:
		shown.emit(last_import_ms, last_water_ms)
	return ""


## Draws this outline of a regrowth in the overlay (its rim and its band's inner edge), or none for null.
func show_outline(outline: LandformOutline) -> void:
	_outline = outline
	if _editing and _has_land():
		_make_overlay()


## Draws this area to be regrown in the overlay (an Image.FORMAT_L8 of the land's size, a cell in it where its pixel is
## 128 or more), or none for null.
func show_area(area: Image) -> void:
	_area = area
	if _editing and _has_land():
		_make_overlay()


## Draws this picture as the held ground in the overlay in place of the land's (an Image.FORMAT_L8 of the land's size,
## a cell held where its pixel is 128 or more: the editor tool's Hold stroke going on, the land's held ground as the
## stroke leaves it), until it is given null, which draws the land's again; a land shown anew draws the land's.
func show_held(held: Image) -> void:
	_held_shown = held
	if _editing and _has_land():
		_make_overlay()


## Draws this drawing's rivers, points and lakes as the lines, on the land's ground, the item `selection` names in the
## look's selected colour: {"river": i} a river, {"river": i, "point": j} a point of it, {"lake": k} a lake, each an
## index from 0, and {} nothing (LandformLandLines.SELECTED_RIVER and the rest). null draws none.
func show_drawing(drawing: LandformDrawing, selection: Dictionary) -> void:
	_drawing = drawing
	_selection = selection
	if _editing and _has_land():
		_make_lines()


## The drawing the lines draw: the one show_drawing last gave (a land shown anew gives its asked drawing), which
## refresh() keeps; null for none.
func get_drawing_shown() -> LandformDrawing:
	return _drawing


## Draws the overlay and the lines with this look from now on (null for the shipped values), made again at once once
## the node is ready: the editor tool's settings' look. The view keeps it in a plain variable, so no scene stores it.
func set_overlay_colours(colours: LandformOverlayLook) -> void:
	_overlay_look = colours
	_hand_over_outline_overlay()
	if is_node_ready():
		_hand_over_line_sizes()
		if _editing and _has_land():
			_make_overlay()
			_make_lines()


## The look the overlay and the lines are drawn with: the one set_overlay_colours gave, or the shipped values.
func get_overlay_look() -> LandformOverlayLook:
	return _look_drawn()


## Shows the overlay and the lines (on) or hides them (off): on in the editor and off in a running game, where the node
## shows its land and nothing else. Turned on, they are made from the land at once.
func set_editing(on: bool) -> void:
	_editing = on
	_show_editing()
	_hand_over_outline_overlay()
	if on and _has_land():
		_held = _land.get_held_image()
		_hand = _land.get_hand_image()
		_make_overlay()
		_make_lines()


func is_editing() -> bool:
	return _editing


## The land shown; null before any.
func get_land() -> LandformLand:
	return _land


## The land's ground as the lines lay a drawing on it, made from its world's heights once for each world shown and
## shared with the editor tool's picking (LandformPicking), so no stroke or drag copies the heights at each move; null
## without a land, or for a world of fewer than two cells a side.
func get_ground() -> LandformLandLines.Ground:
	if not _has_land():
		return null
	var world := _land.get_world()
	if world.get_size() < 2:
		return null
	if _ground == null or world.get_instance_id() != _ground_of:
		_ground = LandformLandLines.Ground.new(world)
		_ground_of = world.get_instance_id()
	return _ground


## The overlay's picture as the decal shows it now, an Image.FORMAT_RGBA8 of the land's size, cell (x, z) at pixel
## (x, z); null before any is made. The view's own: change a copy.
func get_overlay_image() -> Image:
	return _overlay_image


## Where a ray from `from` along `direction` first meets the land's ground, however far along it: straight down,
## Terrain3D's height under `from` exactly (get_intersection); any other ray is clipped to the land's box (its square,
## and its heights' range with OVERLAY_MARGIN_M above and below), started CLIP_LEAD_M before it, and marched by
## Terrain3D's get_intersection in its CPU mode, 1 m a step, from where it enters the box and again from the last point
## each march checked until the ray leaves the box, so ground at any distance along the ray is found; the hit is then
## refined by halving between the step above the ground and the step at or below it, REFINE_HALVINGS times. A ridge
## thinner than a step along the ray may be stepped over. NO_GROUND when the ray meets no ground: it starts below the
## ground, misses the land's box, or leaves it above the ground, or there is no land, or its start or direction is no
## finite vector or its direction is zero.
func find_ground(from: Vector3, direction: Vector3) -> Vector3:
	if not from.is_finite() or not direction.is_finite() or direction.is_zero_approx():
		return NO_GROUND
	var terrain := %Terrain3D as Terrain3D
	var step := direction.normalized()
	if step.y < STRAIGHT_DOWN:
		# Terrain3D gives the ground under a ray straight down wherever the ray starts, also below it.
		var under := terrain.get_intersection(from, step, false)
		if not under.is_finite() or under.x >= TERRAIN3D_NO_HIT:
			return NO_GROUND
		return under if under.y <= from.y else NO_GROUND
	if not _has_land():
		return NO_GROUND
	var inside := _box_span(from, step)
	if inside.x > inside.y:
		return NO_GROUND
	var along := maxf(inside.x - CLIP_LEAD_M, 0.0)
	# Terrain3D refuses a march that starts below the ground, which only the first can: each later one starts at the
	# last point the march before checked, which was above it.
	while along <= inside.y:
		var start := from + step * along
		var hit := terrain.get_intersection(start, step, false)
		if hit.is_finite() and hit.x < TERRAIN3D_NO_HIT:
			return _refined(terrain, start, hit, step)
		if terrain.data.get_surface_height(start) > start.y:
			return NO_GROUND
		along += RAYMARCH_POINTS - 1.0
	return NO_GROUND


func _notification(what: int) -> void:
	# A save that would store the view's own nodes (its scene opened itself, a scene inheriting it, or an instance whose
	# children the scene edits) takes off what must never be saved into a scene, the overlay's texture (16 MB at 2 km)
	# and the lines' mesh, and gives them back once it has saved.
	if what == NOTIFICATION_EDITOR_PRE_SAVE and _nodes_saved():
		_saving = true
		(%Overlay as Decal).texture_albedo = null
		(%Lines as MeshInstance3D).mesh = null
	elif what == NOTIFICATION_EDITOR_POST_SAVE and _saving:
		_give_back_saved()


## Gives the overlay's texture and the lines' mesh back after a save took them off: at the save's end, or at the next
## refresh when the editor never said it ended (it does not when the scene could not be packed).
func _give_back_saved() -> void:
	_saving = false
	(%Overlay as Decal).texture_albedo = _overlay_texture
	(%Lines as MeshInstance3D).mesh = _lines_mesh


## Whether a save of the edited scene stores the view's own nodes: the view is the edited scene's root, or an instance
## in it whose children it edits.
func _nodes_saved() -> bool:
	if not is_inside_tree():
		return false
	var root := get_tree().edited_scene_root
	return root != null and (root == self or root.is_editable_instance(self))


func _has_land() -> bool:
	return _land != null and _land.get_size() > 0


## Why this world cannot be shown, or "".
func _refusal(world: LandformWorld) -> String:
	if LandformTerrain3D.region_size_for(world.get_size()) == 0:
		return NO_REGIONS % world.get_size()
	if look == null:
		return NO_LOOK
	return look.problem(world.get_material_names())


## Makes the world's control and colour maps with the look into `maps`; "" or why it cannot be shown, `maps` then empty.
func _paint(world: LandformWorld, maps: Array[Image]) -> String:
	maps.clear()
	var refused := _refusal(world)
	if not refused.is_empty():
		return refused
	var names := world.get_material_names()
	var control := LandformTerrain3D.control_image(world, look.texture_ids(names), look.rotation_patch_m)
	var colour := LandformTerrain3D.color_image(world, look.dry_tint, look.lush_tint, look.wet_gloss)
	if control == null or colour == null:
		return NOT_PAINTED
	maps.append_array([control, colour])
	return ""


## The land shown painted again with the look, as when its world changed (a new look set once the node is ready).
func _paint_again() -> void:
	if not _has_land():
		return
	_world_id = 0
	var refused := refresh()
	if not refused.is_empty():
		push_error(refused)


## Imports the world's maps whole into Terrain3D's regions of its size (its heights and `maps`, the control and colour
## maps _paint made) and builds its water.
func _show_world(world: LandformWorld, maps: Array[Image]) -> void:
	var started := Time.get_ticks_usec()
	var terrain := %Terrain3D as Terrain3D
	for region: Terrain3DRegion in terrain.data.get_regions_active():
		terrain.data.remove_region(region, false)
	# With no region left, the new size has nothing to copy.
	terrain.region_size = LandformTerrain3D.region_size_for(world.get_size()) as Terrain3D.RegionSize
	var half := world.get_size() / 2.0
	var images: Array[Image] = [world.get_height_image(), maps[0], maps[1]]
	terrain.data.import_images(images, Vector3(-half, 0.0, -half), 0.0, 1.0)
	last_import_ms = (Time.get_ticks_usec() - started) / 1000.0
	var water_started := Time.get_ticks_usec()
	last_water_reason = (%Water as LandformWater).show_world(world)
	last_water_ms = (Time.get_ticks_usec() - water_started) / 1000.0
	_place_overlay(world.get_size())


## Sizes the overlay's box to the land: its side square over the land, its height the ground's range and a margin, its
## centre TEXEL_TO_CELL_M west and north of the origin so each texel's centre lies on its cell.
func _place_overlay(size: int) -> void:
	var span := (%Terrain3D as Terrain3D).data.get_height_range()
	var decal := %Overlay as Decal
	decal.size = Vector3(size, span.y - span.x + 2.0 * OVERLAY_MARGIN_M, size)
	decal.position = Vector3(-TEXEL_TO_CELL_M, (span.x + span.y) / 2.0, -TEXEL_TO_CELL_M)


## Where a ray from `from` along the unit `step` lies inside the land's box, as distances along it from `from`: (where
## it enters, where it leaves), the first 0 when it starts inside, and the first beyond the second when it misses the
## box or the box lies behind it. The box is the land's square and its heights' range with OVERLAY_MARGIN_M.
func _box_span(from: Vector3, step: Vector3) -> Vector2:
	var half := _land.get_size() / 2.0
	var span := (%Terrain3D as Terrain3D).data.get_height_range()
	var low := Vector3(-half, span.x - OVERLAY_MARGIN_M, -half)
	var high := Vector3(half, span.y + OVERLAY_MARGIN_M, half)
	var enter := 0.0
	var leave := INF
	for axis: int in 3:
		if step[axis] == 0.0:
			if from[axis] < low[axis] or from[axis] > high[axis]:
				return Vector2(INF, -INF)
			continue
		var near := (low[axis] - from[axis]) / step[axis]
		var far := (high[axis] - from[axis]) / step[axis]
		enter = maxf(enter, minf(near, far))
		leave = minf(leave, maxf(near, far))
	return Vector2(enter, leave)


## The ground a march from `start` along the unit `step` met at `hit`, halved REFINE_HALVINGS times between the step
## above it and the hit.
func _refined(terrain: Terrain3D, start: Vector3, hit: Vector3, step: Vector3) -> Vector3:
	if hit.is_equal_approx(start):
		return hit
	var above := hit - step
	var below := hit
	for _halving: int in REFINE_HALVINGS:
		var middle := (above + below) * 0.5
		# A height that is not a number, off the land, is never at or above the ray.
		if middle.y <= terrain.data.get_surface_height(middle):
			below = middle
		else:
			above = middle
	return below


## Makes the overlay's picture again and hands it to the decal. With no area and no held ground of a stroke drawn, an
## outline of a drawing carries the picture, made on the land's outlines' thread with these colours over the land's
## marks (LandformOutline.get_overlay_image), and only the image is made here (stage 2's ruling R21, N3); else it is
## composed here.
func _make_overlay() -> void:
	var started := Time.get_ticks_usec()
	var colours := _look_drawn()
	var held := _held_shown if _held_shown != null else _held
	_overlay_image = null
	if _outline != null and _area == null and _held_shown == null:
		_overlay_image = _outline.get_overlay_image(colours.rim_color, colours.band_edge_color, colours.held_color,
				colours.hand_color, colours.edge_cells)
	if _overlay_image == null:
		_overlay_image = LandformOverlay.compose(_land.get_size(), _outline, held, _hand, _area, colours.rim_color,
				colours.band_edge_color, colours.held_color, colours.hand_color, colours.area_color, colours.edge_cells)
	var decal := %Overlay as Decal
	if _overlay_image == null:
		# The extension has printed why.
		decal.texture_albedo = null
		_overlay_texture = null
		_overlay_spare = null
	else:
		# Write the texture the decal does not show, then give it to the decal: a texture written in place while the
		# decal shows it is never seen again (Godot keeps the decal's atlas as it was), measured windowed at the
		# stage 2 gate.
		var next := _overlay_spare
		if next != null and Vector2i(next.get_size()) == _overlay_image.get_size():
			next.update(_overlay_image)
		else:
			next = ImageTexture.create_from_image(_overlay_image)
		_overlay_spare = _overlay_texture
		_overlay_texture = next
		if not _saving:
			decal.texture_albedo = _overlay_texture
	last_overlay_ms = (Time.get_ticks_usec() - started) / 1000.0


## Gives the land the overlay's colours, so the outline of each drawing asked from now on carries its overlay's
## picture while the view edits (LandformLand.set_outline_overlay), or none while it does not. Set at each refresh, so
## a land restored by a reload of the extension, which keeps no such setting, has them again.
func _hand_over_outline_overlay() -> void:
	if _land == null:
		return
	var colours := _look_drawn()
	_land.set_outline_overlay(_editing, colours.rim_color, colours.band_edge_color, colours.held_color,
			colours.hand_color, colours.edge_cells)


## Makes the lines again, of the drawing shown, on the land's ground.
func _make_lines() -> void:
	var started := Time.get_ticks_usec()
	_lines_mesh = LandformLandLines.make_mesh(_drawing, get_ground(), _selection, _look_drawn())
	(%Lines as MeshInstance3D).mesh = _lines_mesh
	last_lines_ms = (Time.get_ticks_usec() - started) / 1000.0


## Gives Terrain3D a copy of the project's material (the scene's own when there is none). Terrain3D is handed copies
## that belong to no file: at every save of a scene in the editor it writes its material and its assets back to their
## own files (Terrain3DMaterial.save and Terrain3DAssets.save), and it adds to its assets what it makes of them (a mesh
## asset, its material, pictures of textures it lacks), which Godot then saves into the file that holds them, the
## look's among them. A copy has no path, so neither is written.
func _hand_over_material() -> void:
	var terrain := %Terrain3D as Terrain3D
	if terrain_material != null:
		terrain.material = terrain_material.duplicate(true) as Terrain3DMaterial
	else:
		terrain.material = _scene_material


## Gives Terrain3D a copy of the look's assets (_hand_over_material says why a copy); without a look, or a look without
## assets, Terrain3D keeps what it has.
func _hand_over_assets() -> void:
	if look != null and look.assets != null:
		(%Terrain3D as Terrain3D).assets = look.assets.duplicate(true) as Terrain3DAssets


func _hand_over_line_sizes() -> void:
	var material := (%Lines as MeshInstance3D).material_override as ShaderMaterial
	var colours := _look_drawn()
	material.set_shader_parameter(LINE_PIXELS, colours.line_pixels)
	material.set_shader_parameter(MARKER_PIXELS, colours.marker_pixels)


func _show_editing() -> void:
	(%Overlay as Decal).visible = _editing
	(%Lines as MeshInstance3D).visible = _editing


func _look_drawn() -> LandformOverlayLook:
	return _overlay_look if _overlay_look != null else _shipped_look
