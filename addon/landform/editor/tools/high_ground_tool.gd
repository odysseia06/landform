@tool
class_name LandformHighGroundTool
extends LandformBrushTool
## High ground: a stroke paints the drawing's picture of high ground lighter, and with the settings' modifier darker
## (LandformBrush, the session's radius and strength for it), on a copy of the picture of the drawing being edited
## (LandformBrush.picture_of: a drawing without one is given a picture of the settings' side at 0, which grows as none
## does). Each move hands the session the drawing with the picture as painted so far (set_working: the outline
## follows); the release is one step (release_change: the land asks for it and, with "Regrow on release" on,
## regrows), and the stop shortcut before it drops the stroke, the picture as it was.

## The gesture of a stroke going on, and the action of the undo history its release makes, as the bar's Undo names it.
const PAINT_HIGH_GROUND := "paint high ground"
const ACTION_PAINT := "paint high ground"
## Why a stroke paints nothing: the drawing's texture holds no picture the brush can read.
const PICTURE_UNREAD := "The drawing's picture of high ground cannot be read, so it cannot be painted: give the drawing a picture Godot can read."

## The drawing when the stroke began, the brush over its picture's copy, and the texture the drawings made during the
## stroke hold the copy by.
var _base: LandformDrawing
var _brush: LandformBrush
var _texture: LandformBrush.Picture


func get_gesture() -> String:
	return PAINT_HIGH_GROUND if is_stroking() else NO_GESTURE


func _begin(at: Vector2, inverted_stroke: bool) -> bool:
	var session := _session()
	_base = session.get_working_drawing()
	var picture := LandformBrush.picture_of(_base, session.settings.new_land_picture_px)
	if picture == null:
		session.say(PICTURE_UNREAD)
		return false
	_brush = _brush_on(picture, not inverted_stroke, at)
	_texture = LandformBrush.Picture.new(picture)
	_hand_over()
	return true


func _paint(at: Vector2) -> void:
	_brush.drag_to(at)
	_hand_over()


func _finish() -> void:
	_end()
	_session().release_change(ACTION_PAINT)


func _drop() -> void:
	_end()
	_session().drop_change()


func _forget() -> void:
	_end()


## Hands the session the drawing being edited with the picture as painted so far.
func _hand_over() -> void:
	var drawing := LandformLandSession.copy_drawing(_base)
	drawing.high_ground = _texture
	_session().set_working(drawing)


func _end() -> void:
	if _brush != null:
		_brush.end()
	_brush = null
	_base = null
	_texture = null
