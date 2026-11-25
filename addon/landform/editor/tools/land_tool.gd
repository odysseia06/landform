@tool
class_name LandformLandTool
extends RefCounted
## What the editor tool's tools share (Select, River and Lake, and the brushes High ground, Hold and Regrow an area as
## drawn, whose strokes LandformBrushTool shares). A tool takes the 3D view's events from its session (`input`, which
## says whether the tool took the event) and is in at most one gesture at a time (`get_gesture`). It works on the
## session's drawing being edited: while a gesture goes on it hands each new drawing to the session
## (LandformLandSession.set_working, the lines and the outline following), and a gesture finished is one step
## (LandformLandSession.release_change, which asks the land and, with "Regrow on release" on, regrows), one action of
## the undo history named for what the gesture did ("move", "add river", the tool's ACTION_* names). A tool takes
## the left mouse button and the keys it names, never the right or middle button, the wheel or the left button with
## Alt held, so the editor's camera moves as ever; the session hands it nothing while the land grows, nor the stop
## shortcut, which ends its gesture.
##
## A tool holds its session weakly, while the session holds the tool, so neither keeps the other alive.

## The gesture of a tool in none.
const NO_GESTURE := ""
## How far, in pixels, the pointer moves from where the left button went down before the press is a drag: a click
## that stays within it changes nothing it picked.
const DRAG_PIXELS_LEAST := 4.0

var _session_ref: WeakRef
var _picking: LandformPicking


func _init(session: LandformLandSession, picking: LandformPicking) -> void:
	_session_ref = weakref(session)
	_picking = picking


## Takes an event of the 3D view, seen through `camera`; returns whether the tool took it (the editor then does
## nothing more with it).
func input(_event: InputEvent, _camera: Camera3D) -> bool:
	return false


## The gesture the tool is in, NO_GESTURE for none.
func get_gesture() -> String:
	return NO_GESTURE


## Ends the gesture and drops its change: the drawing being edited is the land's asked drawing again.
func cancel() -> void:
	pass


## Forgets the gesture and keeps the drawing as it is: its change was released or dropped by other means (Regrow, a
## number typed, a job's step).
func forget() -> void:
	pass


func _session() -> LandformLandSession:
	return _session_ref.get_ref() as LandformLandSession


## Whether the event is the left mouse button going down, without Alt (which moves the camera in some of the
## editor's schemes).
static func _is_left_press(event: InputEvent) -> bool:
	var button := event as InputEventMouseButton
	return button != null and button.button_index == MOUSE_BUTTON_LEFT and button.pressed and not button.alt_pressed


## Whether the event is the left mouse button going up.
static func _is_left_release(event: InputEvent) -> bool:
	var button := event as InputEventMouseButton
	return button != null and button.button_index == MOUSE_BUTTON_LEFT and not button.pressed


## Whether the event is a move of the pointer with the left button held; a move without it ends a drag whose release
## the view did not see.
static func _is_left_drag(event: InputEvent) -> bool:
	var motion := event as InputEventMouseMotion
	return motion != null and motion.button_mask & MOUSE_BUTTON_MASK_LEFT != 0


## Whether the event is one of these keys pressed, not held down and repeating.
static func _is_key(event: InputEvent, keys: Array[Key]) -> bool:
	var key := event as InputEventKey
	return key != null and key.pressed and not key.echo and key.keycode in keys


## A number held within half the land's side either way, as a drawing holds its points and lakes' centres.
static func _within(value: float, half: float) -> float:
	return clampf(value, -half, half)
