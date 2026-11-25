@tool
class_name LandformAreaTool
extends LandformBrushTool
## Regrow an area as drawn: strokes paint the area to be regrown, and with the settings' modifier clear it
## (LandformBrush on a mask of the land's cells, the session's radius for it), the area the session keeps
## (LandformLandSession.set_area) until Regrow regrows it (LandformLand.start_regrow_area) or it is cleared. While it is
## painted the view draws it and the land works out its outline (LandformLand.start_area_outline), which takes a lake
## the area touches whole with its band, so the person sees the band's inner edge, where the area is fully regrown,
## before it runs (the stage 2 plan's question 1 (a)). The area is cleared once its regrowth is swapped in, by the stop
## shortcut, and when another tool is chosen; a stroke's release that leaves no cell painted clears it too. An area is
## no change of the drawing: it never regrows on release.

## The gestures: a stroke going on, and an area painted waiting for Regrow.
const PAINT_AREA := "paint area"
const AREA_PAINTED := "area painted"

var _brush: LandformBrush


func get_gesture() -> String:
	if is_stroking():
		return PAINT_AREA
	return AREA_PAINTED if _session().get_area() != null else NO_GESTURE


func cancel() -> void:
	super.cancel()
	_brush = null
	_session().clear_area()


func _begin(at: Vector2, inverted_stroke: bool) -> bool:
	var session := _session()
	var size := session.get_land().get_size()
	var area := session.get_area()
	if area == null:
		area = Image.create_empty(size, size, false, Image.FORMAT_L8)
	_brush = _brush_on(area, not inverted_stroke, at)
	session.set_area(area)
	return true


func _paint(at: Vector2) -> void:
	_brush.drag_to(at)
	_session().set_area(_brush.get_image())


func _finish() -> void:
	_brush = null
	_session().release_area()


func _drop() -> void:
	_brush = null


func _forget() -> void:
	_brush = null
