@tool
class_name LandformRiverTool
extends LandformLandTool
## River: draws a new river, its points clicked on the ground upstream first, as a drawing holds them, the river the
## settings' width (LandformToolSettings.new_river_width_m) and last in the drawing's list. Each click adds a point
## where the ground under the pointer lies (LandformPicking.ground_under), within the land, and hands the drawing with
## the river so far to the session, the new point selected; Enter or a double click ends the river, once it has two
## points, as one step (a double click's second press adds no point), and the stop shortcut (Escape) drops it, the
## drawing as it was before its first click; the editor's undo while it is drawn takes back its last point (`take_back`,
## through the session: stage 2's ruling R21, S1). A click off the land adds nothing.

## The gesture of a river being drawn.
const DRAW_RIVER := "draw river"
## What ending a river of one point says.
const NEEDS_TWO_POINTS := "A river needs two points at least: click where it runs next."
## The action of the undo history a river ended makes, as the bar's Undo names it.
const ACTION_ADD_RIVER := "add river"

## The drawing before the river's first click, the river's index in it once added, and its points so far, two numbers
## a point, east then south.
var _base: LandformDrawing
var _river := -1
var _points := PackedFloat64Array()
## Whether the tool took the left button's press, so it takes its release too.
var _held := false


func input(event: InputEvent, camera: Camera3D) -> bool:
	if _is_left_press(event):
		var press := event as InputEventMouseButton
		_held = true
		if press.double_click and _river >= 0:
			_finish()
			return true
		var hit := _picking.ground_under(press.position, camera)
		if hit == LandformLandView.NO_GROUND:
			_held = _river >= 0
			return _held
		_add(hit)
		return true
	if _held and _is_left_release(event):
		_held = false
		return true
	if _river >= 0 and _is_key(event, [KEY_ENTER, KEY_KP_ENTER]):
		_finish()
		return true
	return false


func get_gesture() -> String:
	return DRAW_RIVER if _river >= 0 else NO_GESTURE


func cancel() -> void:
	if _river < 0:
		return
	_end()
	_session().drop_change()


func forget() -> void:
	_end()


## A point at the ground `hit`, the river begun with it when none is being drawn.
func _add(hit: Vector3) -> void:
	var session := _session()
	if _river < 0:
		_base = session.get_working_drawing()
		_river = _base.rivers.size()
		_points = PackedFloat64Array()
	var half := session.get_land().get_size() / 2.0
	_points.append(_within(hit.x, half))
	_points.append(_within(hit.z, half))
	var river := LandformRiver.new()
	river.points_m = _points
	river.width_m = session.settings.new_river_width_m
	var drawing := LandformLandSession.copy_drawing(_base)
	var rivers := drawing.rivers
	rivers.append(river)
	drawing.rivers = rivers
	session.set_working(drawing)
	session.select({LandformLandSession.RIVER: _river, LandformLandSession.POINT: LandformLandSession.point_count(_points) - 1})


## Takes back the river's last point, as the editor's undo while it is drawn does (stage 2's ruling R21, S1): the
## drawing with the river so far, its new last point selected; with one point the river is dropped, as the stop
## shortcut drops it.
func take_back() -> void:
	if _river < 0:
		return
	if LandformLandSession.point_count(_points) <= 1:
		cancel()
		return
	var session := _session()
	_points.resize(_points.size() - LandformLandSession.NUMBERS_PER_POINT)
	var river := LandformRiver.new()
	river.points_m = _points
	river.width_m = session.settings.new_river_width_m
	var drawing := LandformLandSession.copy_drawing(_base)
	var rivers := drawing.rivers
	rivers.append(river)
	drawing.rivers = rivers
	session.set_working(drawing)
	session.select({LandformLandSession.RIVER: _river, LandformLandSession.POINT: LandformLandSession.point_count(_points) - 1})


## Ends the river as one step once it has two points; with one it says so and goes on.
func _finish() -> void:
	if LandformLandSession.point_count(_points) < LandformLandSession.RIVER_POINTS_LEAST:
		_session().say(NEEDS_TWO_POINTS)
		return
	_end()
	_session().release_change(ACTION_ADD_RIVER)


func _end() -> void:
	_base = null
	_river = -1
	_points = PackedFloat64Array()
