@tool
class_name LandformSelectTool
extends LandformLandTool
## Select: a click on a river's point selects it and a drag moves it; a click on a river's line adds a point there,
## between the two it lies between, selected, and a drag on moves it; a click on a lake (its middle, its rim or the
## ground inside it) selects it, a drag of its middle or inside moves it and a drag of its rim sizes it; a click on
## bare ground selects nothing. Delete, which removes the selected point or lake, is the session's under every tool
## (LandformLandSession.delete_selected). What lies under the pointer is LandformPicking's.
##
## A drag moves the item by as much as the pointer moved from where the button went down, so it does not jump to the
## pointer: measured on the ground under the pointer, so the item stays under it on slopes, or, for an item hidden at
## the press behind nearer ground drawn over it in a slanted view (LandformPicking.hides, at the item's own place on the
## screen: stage 2's ruling R19, M1), on the level plane through the item's own ground (a point's, a lake's centre's:
## LandformPicking.on_level), so it does not jump to that ground (stage 2's ruling R18, S1, amending R17, N1); points
## and centres stay within the land,
## and a lake's radius from its least, the settings' least or its own when smaller, to half the land's side. Each move
## hands the session the drawing changed (the lines and the outline follow); the release is one step. A click that
## moves less than DRAG_PIXELS_LEAST only selects.

## The gestures: a point (or one just added on a line) pressed and perhaps dragged, a lake pressed by its middle or
## inside and perhaps dragged, and a lake pressed by its rim and perhaps sized.
const MOVE_POINT := "move point"
const MOVE_LAKE := "move lake"
const SIZE_LAKE := "size lake"
## The actions of the undo history a release makes, as the bar's Undo names them: a point or a lake moved, a point
## added on a line (and perhaps dragged on), a lake sized.
const ACTION_MOVE := "move"
const ACTION_ADD_POINT := "add point"
const ACTION_SIZE_LAKE := "size lake"

var _gesture := NO_GESTURE
## Whether the tool took the left button's press, so it takes its release too.
var _held := false
## Whether the press moved beyond DRAG_PIXELS_LEAST, and whether it added a point on a line: either makes its
## release a step.
var _dragging := false
var _added := false
## Where the button went down on the screen, the height of the item's own ground, whether the drag is measured on the
## level plane at that height (the item hidden at the press) or on the ground under the pointer, and where the pointer
## met it then.
var _pressed_at := Vector2.ZERO
var _level := 0.0
var _on_level := false
var _grab := Vector3.ZERO
## The drawing when the gesture began (with the point added), the item it moves, and that item's place or radius
## then.
var _start: LandformDrawing
var _item: Dictionary = {}
var _start_east := 0.0
var _start_south := 0.0
var _start_radius := 0.0


func input(event: InputEvent, camera: Camera3D) -> bool:
	if _is_left_press(event):
		_held = _press((event as InputEventMouseButton).position, camera)
		return _held
	if _gesture != NO_GESTURE and event is InputEventMouseMotion:
		if not _is_left_drag(event):
			_held = false
			_release()
			return true
		_drag((event as InputEventMouseMotion).position, camera)
		return true
	if _held and _is_left_release(event):
		_held = false
		_release()
		return true
	return false


func get_gesture() -> String:
	return _gesture


func cancel() -> void:
	if _gesture == NO_GESTURE:
		return
	_end()
	_session().drop_change()


func forget() -> void:
	_end()


func _press(screen: Vector2, camera: Camera3D) -> bool:
	var session := _session()
	var drawing := session.get_working_drawing()
	var picked := _picking.pick(drawing, screen, camera)
	if picked.is_empty():
		if _picking.ground_under(screen, camera) == LandformLandView.NO_GROUND:
			return false
		session.select({})
		return true
	_end()
	var kind := String(picked[LandformPicking.KIND])
	if kind == LandformPicking.POINT:
		_item = {LandformLandSession.RIVER: int(picked[LandformPicking.RIVER]), LandformLandSession.POINT: int(picked[LandformPicking.POINT])}
		_gesture = MOVE_POINT
	elif kind == LandformPicking.LINE:
		var river := int(picked[LandformPicking.RIVER])
		var index := int(picked[LandformPicking.INDEX])
		session.set_working(_point_added(drawing, river, index, picked[LandformPicking.EAST], picked[LandformPicking.SOUTH]))
		_item = {LandformLandSession.RIVER: river, LandformLandSession.POINT: index}
		_gesture = MOVE_POINT
		_added = true
	else:
		_item = {LandformLandSession.LAKE: int(picked[LandformPicking.LAKE])}
		_gesture = SIZE_LAKE if kind == LandformPicking.RIM else MOVE_LAKE
	session.select(_item)
	_start = session.get_working_drawing()
	_pressed_at = screen
	if _gesture == MOVE_POINT:
		var points := (_start.rivers[_item[LandformLandSession.RIVER]] as LandformRiver).points_m
		var j := int(_item[LandformLandSession.POINT])
		_start_east = points[2 * j]
		_start_south = points[2 * j + 1]
	else:
		var lake := _start.lakes[_item[LandformLandSession.LAKE]] as LandformLake
		_start_east = lake.x_m
		_start_south = lake.z_m
		_start_radius = lake.radius_m
	_level = _picking.height_at(Vector2(_start_east, _start_south))
	# Hidden is decided at the item's own place on the screen, never along the press's ray: a press a few pixels below
	# a marker in a slanted view, or beside a point in its bed, meets nearer ground though nothing hides the item
	# (stage 2's ruling R19, M1). The grab is measured from the press.
	var item := Vector3(_start_east, _level, _start_south)
	_on_level = _picking.hides(_picking.on_screen(Vector2(_start_east, _start_south), camera), camera, item)
	_grab = _picking.drag_point(screen, camera, _level, _on_level)
	if _grab == LandformLandView.NO_GROUND:
		# Measured from the item itself when the pointer's ray meets neither.
		_grab = Vector3(_start_east, _level, _start_south)
	return true


func _drag(screen: Vector2, camera: Camera3D) -> void:
	if not _dragging:
		if screen.distance_to(_pressed_at) < DRAG_PIXELS_LEAST:
			return
		_dragging = true
	var hit := _picking.drag_point(screen, camera, _level, _on_level)
	if hit == LandformLandView.NO_GROUND:
		return
	var session := _session()
	var half := session.get_land().get_size() / 2.0
	var east := _within(_start_east + (hit.x - _grab.x), half)
	var south := _within(_start_south + (hit.z - _grab.z), half)
	var drawing := LandformLandSession.copy_drawing(_start)
	match _gesture:
		MOVE_POINT:
			var i := int(_item[LandformLandSession.RIVER])
			var j := int(_item[LandformLandSession.POINT])
			var rivers := drawing.rivers
			var river := rivers[i].duplicate() as LandformRiver
			var points := river.points_m
			points[2 * j] = east
			points[2 * j + 1] = south
			river.points_m = points
			rivers[i] = river
			drawing.rivers = rivers
		MOVE_LAKE, SIZE_LAKE:
			var k := int(_item[LandformLandSession.LAKE])
			var lakes := drawing.lakes
			var lake := lakes[k].duplicate() as LandformLake
			if _gesture == MOVE_LAKE:
				lake.x_m = east
				lake.z_m = south
			else:
				var reach := Vector2(hit.x - _start_east, hit.z - _start_south).length()
				var grabbed := Vector2(_grab.x - _start_east, _grab.z - _start_south).length()
				var least := minf(session.settings.new_lake_least_radius_m, _start_radius)
				lake.radius_m = clampf(_start_radius + reach - grabbed, least, half)
			lakes[k] = lake
			drawing.lakes = lakes
	session.set_working(drawing)


## The button went up: a drag or a point added is one step, one action of the history named for what was done, and
## a click that only selected (or a gesture already forgotten) is none.
func _release() -> void:
	var changed := _gesture != NO_GESTURE and (_dragging or _added)
	var action_name := ACTION_ADD_POINT if _added else (ACTION_SIZE_LAKE if _gesture == SIZE_LAKE else ACTION_MOVE)
	_end()
	if changed:
		_session().release_change(action_name)


func _end() -> void:
	_gesture = NO_GESTURE
	_dragging = false
	_on_level = false
	_added = false
	_start = null
	_item = {}


## A copy of the drawing with a point at (east, south) put into river `river` at `index`, the points from it on moved
## up one.
static func _point_added(drawing: LandformDrawing, river: int, index: int, east: float, south: float) -> LandformDrawing:
	var added := LandformLandSession.copy_drawing(drawing)
	var rivers := added.rivers
	var changed := rivers[river].duplicate() as LandformRiver
	var points := changed.points_m
	points.insert(2 * index, south)
	points.insert(2 * index, east)
	changed.points_m = points
	rivers[river] = changed
	added.rivers = rivers
	return added
