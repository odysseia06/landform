@tool
class_name LandformLakeTool
extends LandformLandTool
## Lake: places a new lake, its middle where the left button goes down on the ground (LandformPicking.ground_under),
## within the land, and its radius as far as the pointer is dragged from it on the ground, from the settings' least
## (LandformToolSettings.new_lake_least_radius_m, however short the drag) to half the land's side; the lake is last in
## the drawing's list and selected. Each move hands the drawing with the lake to the session (the lines and the
## outline follow); the release is one step, and the stop shortcut (Escape) before it drops the lake. A press off the
## land places nothing, and a double click's second press neither, so a double click places one lake.

## The gesture of a lake being placed and sized.
const PLACE_LAKE := "place lake"
## The action of the undo history a lake placed makes, as the bar's Undo names it.
const ACTION_ADD_LAKE := "add lake"

## The drawing before the press, the new lake's index in it once added, and its middle.
var _base: LandformDrawing
var _lake := -1
var _east := 0.0
var _south := 0.0


func input(event: InputEvent, camera: Camera3D) -> bool:
	if _is_left_press(event):
		if (event as InputEventMouseButton).double_click:
			# A double click's second press: its first placed the lake.
			return true
		var hit := _picking.ground_under((event as InputEventMouseButton).position, camera)
		if hit == LandformLandView.NO_GROUND:
			return _lake >= 0
		_place(hit)
		return true
	if _lake < 0:
		return false
	if event is InputEventMouseMotion:
		if not _is_left_drag(event):
			_release()
			return true
		var hit := _picking.ground_under((event as InputEventMouseMotion).position, camera)
		if hit != LandformLandView.NO_GROUND:
			_size(Vector2(hit.x - _east, hit.z - _south).length())
		return true
	if _is_left_release(event):
		_release()
		return true
	return false


func get_gesture() -> String:
	return PLACE_LAKE if _lake >= 0 else NO_GESTURE


func cancel() -> void:
	if _lake < 0:
		return
	_end()
	_session().drop_change()


func forget() -> void:
	_end()


func _place(hit: Vector3) -> void:
	var session := _session()
	_base = session.get_working_drawing()
	_lake = _base.lakes.size()
	var half := session.get_land().get_size() / 2.0
	_east = _within(hit.x, half)
	_south = _within(hit.z, half)
	_size(0.0)
	session.select({LandformLandSession.LAKE: _lake})


## Hands the session the drawing with the lake of this radius, held from the settings' least to half the land's side.
func _size(radius_m: float) -> void:
	var session := _session()
	var lake := LandformLake.new()
	lake.x_m = _east
	lake.z_m = _south
	lake.radius_m = clampf(radius_m, session.settings.new_lake_least_radius_m, session.get_land().get_size() / 2.0)
	var drawing := LandformLandSession.copy_drawing(_base)
	var lakes := drawing.lakes
	lakes.append(lake)
	drawing.lakes = lakes
	session.set_working(drawing)


func _release() -> void:
	_end()
	_session().release_change(ACTION_ADD_LAKE)


func _end() -> void:
	_base = null
	_lake = -1
