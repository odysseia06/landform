@tool
class_name LandformLandToolbar
extends HBoxContainer
## The tools in the 3D view's toolbar, one button each, named as `LandformLandSession.TOOLS` names the tools: Select,
## High ground, River, Lake, Hold and Regrow an area as drawn (Sculpt comes in stage 3). It holds no rule: a press
## chooses the session's tool, the buttons are off while the land grows (the stage 2 plan's decision 8) and on in
## FAILED, where any change leaves it (stage 2's ruling R21, N2), and each
## button's tooltip is the session's, which names the settings' keys (stage 2's ruling R18, N7).

signal walk_toggled(picking: bool)

const WALK_SCENE_SETTING := "landform/walk_preview_scene"

var _session: LandformLandSession


func _ready() -> void:
	for tool_name: String in LandformLandSession.TOOLS:
		(get_node("%" + tool_name) as Button).pressed.connect(_on_tool.bind(tool_name))
	(%Walk as Button).toggled.connect(_on_walk_toggled)
	show_session(_session)


## Shows this session from now on (null turns the buttons off).
func show_session(session: LandformLandSession) -> void:
	if _session != null and _session.changed.is_connected(_show):
		_session.changed.disconnect(_show)
	_session = session
	if _session != null:
		_session.changed.connect(_show)
	if is_node_ready():
		_show()


func _show() -> void:
	var walk := %Walk as Button
	walk.visible = not walk_scene().is_empty()
	var reason := walk_unavailable(_session)
	walk.disabled = not reason.is_empty()
	walk.tooltip_text = reason if walk.disabled else "Pick a spot to walk the saved land."
	if walk.disabled and walk.button_pressed:
		walk.button_pressed = false
	_show_tools()


func _show_tools() -> void:
	var picking := (%Walk as Button).button_pressed
	# On in FAILED too: the land is as it was, and any change leaves FAILED (stage 2's ruling R21, N2).
	var idle := _session != null and _session.get_state() in [LandformLandSession.State.LAND, LandformLandSession.State.FAILED]
	for tool_name: String in LandformLandSession.TOOLS:
		var button := get_node("%" + tool_name) as Button
		button.set_pressed_no_signal(not picking and _session != null and _session.get_tool() == tool_name)
		button.disabled = not idle
		button.tooltip_text = _session.get_tool_tip(tool_name) if _session != null else ""


func _on_tool(tool_name: String) -> void:
	if _session != null:
		(%Walk as Button).button_pressed = false
		_session.set_tool(tool_name)


func set_walk_picking(picking: bool) -> void:
	(%Walk as Button).set_pressed_no_signal(picking)
	_show_tools()


func _on_walk_toggled(picking: bool) -> void:
	walk_toggled.emit(picking)


static func walk_scene() -> String:
	return String(ProjectSettings.get_setting(WALK_SCENE_SETTING, ""))


static func walk_unavailable(session: LandformLandSession) -> String:
	var scene := walk_scene()
	if scene.is_empty() or not ResourceLoader.exists(scene, "PackedScene"):
		return "The walking preview scene is unavailable."
	if session == null or not session.is_alive() or not session.get_state() in [
		LandformLandSession.State.LAND, LandformLandSession.State.FAILED,
	]:
		return "Wait until the land is ready."
	if not session.get_gesture() in [LandformLandTool.NO_GESTURE, LandformAreaTool.AREA_PAINTED]:
		return "Finish or cancel the current gesture first."
	var path := session.get_view().get_land_path()
	if path.is_empty() or not FileAccess.file_exists(path):
		return "Save the land to a file before opening the walking preview."
	return ""


## Each argument is passed separately, so a land path with spaces stays a single argument.
static func walk_arguments(scene: String, path: String, hit: Vector3, pending: bool) -> PackedStringArray:
	var args := PackedStringArray([
		"--path", ProjectSettings.globalize_path("res://"), scene, "--",
		"--land=" + ProjectSettings.globalize_path(path), "--camera=walk",
		"--at=%s,%s" % [str(hit.x), str(hit.z)],
	])
	if pending:
		args.append("--last-saved-land")
	return args
