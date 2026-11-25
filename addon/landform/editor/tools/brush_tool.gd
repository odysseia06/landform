@tool
class_name LandformBrushTool
extends LandformLandTool
## What the editor tool's brushes share (High ground, Hold and Regrow an area as drawn): a stroke from where the left
## button goes down on the land to where it comes up, painted by a LandformBrush at the ground under the pointer
## (LandformPicking.ground_under) where it goes down and at each move, with the session's radius for the tool
## (LandformLandSession.get_brush_radius) and, on the picture of high ground, its strength. The settings' modifier
## (LandformToolSettings.brush_invert_modifier, Ctrl as shipped) held when the button goes down turns the whole stroke
## over: darker, a lift, the area cleared. The settings' keys (brush_smaller_key and brush_larger_key, [ and ] as
## shipped) make the tool's radius smaller and larger by the settings' step, from the next stroke on. A press off the
## land is left to the editor, and a move off it paints nothing; a move without the left button ends a stroke whose
## release the view did not see. A double click's second press begins no stroke, so a double click is one stroke
## (stage 2's ruling R18, N4). Each brush says what its stroke does through `_begin`, `_paint`, `_finish` and `_drop`.

## Whether the tool took the left button's press, so it takes the moves and the release that follow.
var _held := false
## Whether a stroke goes on.
var _stroking := false


func input(event: InputEvent, camera: Camera3D) -> bool:
	if _is_left_press(event):
		var press := event as InputEventMouseButton
		if press.double_click:
			# A double click's second press: its first began the stroke, which its release ended.
			return true
		var hit := _picking.ground_under(press.position, camera)
		if hit == LandformLandView.NO_GROUND:
			return false
		_held = true
		_stroking = _begin(Vector2(hit.x, hit.z), _inverted(press))
		return true
	if _held and event is InputEventMouseMotion:
		if not _is_left_drag(event):
			_release()
			return true
		var hit := _picking.ground_under((event as InputEventMouseMotion).position, camera)
		if _stroking and hit != LandformLandView.NO_GROUND:
			_paint(Vector2(hit.x, hit.z))
		return true
	if _held and _is_left_release(event):
		_release()
		return true
	var session := _session()
	if _is_key(event, [session.settings.brush_smaller_key]):
		session.step_brush_radius(-1)
		return true
	if _is_key(event, [session.settings.brush_larger_key]):
		session.step_brush_radius(1)
		return true
	return false


func cancel() -> void:
	if not _stroking:
		_held = false
		return
	_held = false
	_stroking = false
	_drop()


func forget() -> void:
	if _stroking:
		_held = false
		_stroking = false
		_forget()


## Whether a stroke goes on.
func is_stroking() -> bool:
	return _stroking


## Begins a stroke at this place of the land (east, south), turned over when `inverted`; returns whether it began (a
## brush that cannot paint says why to the session and paints nothing).
func _begin(_at: Vector2, _inverted_stroke: bool) -> bool:
	return false


## Paints the stroke on to this place of the land.
func _paint(_at: Vector2) -> void:
	pass


## The stroke has ended with its release: what it painted is done, as one step.
func _finish() -> void:
	pass


## The stroke is dropped before its release (the stop shortcut, another tool chosen): nothing it painted is done.
func _drop() -> void:
	pass


## The stroke is forgotten, what it painted kept as it is: a change of the drawing was released or dropped by other
## means.
func _forget() -> void:
	pass


func _release() -> void:
	_held = false
	if not _stroking:
		return
	_stroking = false
	_finish()


## Whether the settings' modifier is held in this press.
func _inverted(press: InputEventMouseButton) -> bool:
	var modifier := _session().settings.brush_invert_modifier
	return modifier != 0 and (press.get_modifiers_mask() & modifier) == modifier


## A brush over `image` laid over the session's land, with the session's radius and strength for the tool chosen,
## its stroke begun at `at`.
func _brush_on(image: Image, raise: bool, at: Vector2) -> LandformBrush:
	var session := _session()
	var brush := LandformBrush.new(image, session.get_land().get_size())
	brush.radius_m = session.get_brush_radius()
	brush.strength = session.get_brush_strength()
	brush.begin(raise)
	brush.dab(at)
	return brush
