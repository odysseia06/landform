@tool
class_name LandformHoldTool
extends LandformBrushTool
## Hold: a stroke paints held ground, and with the settings' modifier lifts the hold (LandformBrush on masks of the
## land's cells, the session's radius for it). Two brushes paint each dab: one marks the stroke's own mask, the cells it
## passed over, and one the land's held picture as it will be after the stroke (marked for a hold, cleared for a lift),
## which the view draws in the held colour while the stroke goes on (LandformLandView.show_held), so the person sees
## the held ground grow. The release is the land's hold of the stroke's mask, once for the stroke
## (LandformLandSession.hold_stroke, which calls LandformLand.hold), which holds or lifts a lake whose water the stroke
## meets whole; the bar says the cells it changed and any lake held or lifted whole. The stop shortcut before the
## release drops the stroke, the land's held ground drawn as it is.

## The gesture of a stroke going on.
const PAINT_HOLD := "paint hold"

## The brushes over the stroke's mask and over the held picture as it will be, and whether the stroke lifts.
var _stroke: LandformBrush
var _shown: LandformBrush
var _lift := false


func get_gesture() -> String:
	return PAINT_HOLD if is_stroking() else NO_GESTURE


func _begin(at: Vector2, inverted_stroke: bool) -> bool:
	var session := _session()
	var land := session.get_land()
	var size := land.get_size()
	_lift = inverted_stroke
	_stroke = _brush_on(Image.create_empty(size, size, false, Image.FORMAT_L8), true, at)
	_shown = _brush_on(land.get_held_image(), not _lift, at)
	session.show_held(_shown.get_image())
	return true


func _paint(at: Vector2) -> void:
	_stroke.drag_to(at)
	_shown.drag_to(at)
	_session().show_held(_shown.get_image())


func _finish() -> void:
	var stroke := _stroke.get_image()
	var lift := _lift
	_end()
	_session().hold_stroke(stroke, not lift)


func _drop() -> void:
	_end()
	_session().show_held(null)


func _forget() -> void:
	_end()
	_session().show_held(null)


func _end() -> void:
	_stroke = null
	_shown = null
	_lift = false
