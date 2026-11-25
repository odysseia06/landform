@tool
extends EditorPlugin
## Landform's editor tool: makes and edits a land in Godot's editor, on a LandformLandView of the edited scene. The
## plugin only connects the tool's rules to the editor, and holds none of them: each view edited in a sitting has a
## LandformLandSession (the rules, tested headless by the game), which the panel (docked on the right), the bar (under
## the 3D view) and the toolbar (in the 3D view's menu) show; the 3D view's events go to the session; every frame the
## sessions poll their lands; saving a scene saves its lands, and closing a scene or the editor with a land not saved
## asks first (the stage 2 plan's decision 11). A land is held by its view and its session, never in the plugin's
## editor state (`_get_state` is not used), which Godot writes into `.godot/editor/`. The sessions' actions go into the
## editor's own undo history (EditorHistory, over the plugin's EditorUndoRedoManager), each in the history of the scene
## its view is in, so Ctrl+Z and Ctrl+Shift+Z undo and redo them.

const SETTINGS := preload("res://addons/landform/editor/tool_settings.tres")
const DOCK_SCENE := preload("res://addons/landform/editor/land_dock.tscn")
const LAND_FRAMING := preload("res://addons/landform/editor/land_framing.gd")
const VIEWPORT_COUNT := 4
const BAR_SCENE := preload("res://addons/landform/editor/land_bar.tscn")
const TOOLBAR_SCENE := preload("res://addons/landform/editor/land_toolbar.tscn")

const OPEN_DIALOG_SCENE := preload("res://addons/landform/editor/open_landform_dialog.tscn")
const LAND_VIEW_SCENE := preload("res://addons/landform/land/landform_land_view.tscn")
const OPEN_LANDFORM := "Open Landform"
const AUTHORING_SCENE_SETTING := "landform/editor/authoring_scene"
const LAND_VIEW_SCRIPT := "res://addons/landform/land/landform_land_view.gd"

var _open_dialog: ConfirmationDialog
var _open_dialog_root: Node
var _open_dialog_authoring_path: String
## The dock on the right (an EditorDock) and the panel it holds.
var _dock: EditorDock
var _panel: LandformLandPanel
var _bar: LandformLandBar
var _toolbar: LandformLandToolbar
var _last_viewport: SubViewport
var _walk_picking := false
var _walk_pressed := false
## A session for each land's view edited in this sitting, by the view's instance id, and the one edited now.
var _sessions: Dictionary[int, LandformLandSession] = {}
var _session: LandformLandSession
## The scenes saved since the lands were last saved, by their files: Godot calls `_save_external_data` after a scene
## is saved without saying which.
var _saved_scenes := PackedStringArray()
## The editor's undo history, as the sessions' seam.
var _history: EditorHistory


## The seam between the sessions and the editor's undo history: an action goes into the history of the scene its view
## is in (`create_action` with the view as its context, `add_do_method` and `add_undo_method` of the action's `redo`
## and `undo`, `commit_action(false)`, since it is done already), which the editor's Undo and Redo then undo and redo
## (Ctrl+Z and Ctrl+Shift+Z, the editor's own shortcuts). The bar's Undo presses the editor's own Undo shortcut, as a
## person's key would, since the editor gives a plugin no call that undoes (an UndoRedo of its histories undone
## directly would leave the editor's own record of them behind); so it undoes what Ctrl+Z undoes, the newer of the
## scene's history and the editor's global one, and names the scene's.
class EditorHistory extends LandformLandHistory:
	## The editor's shortcuts the bar's Undo presses.
	const UNDO_SHORTCUT := "ui_undo"
	const REDO_SHORTCUT := "ui_redo"

	var _manager: EditorUndoRedoManager
	## The histories this seam added an action to, by their ids: only those are asked for their actions' names, since
	## the editor reports an error for a history it has not made.
	var _histories: Dictionary[int, bool] = {}

	func _init(manager: EditorUndoRedoManager) -> void:
		super(null)
		_manager = manager
		_manager.history_changed.connect(_on_version_changed)
		_manager.version_changed.connect(_on_version_changed)

	func add(action: LandformLandAction, context: Object) -> void:
		_manager.create_action(action.get_action_name(), UndoRedo.MERGE_DISABLE, context)
		_manager.add_do_method(action, &"redo")
		_manager.add_undo_method(action, &"undo")
		_manager.commit_action(false)
		_histories[_manager.get_object_history_id(context)] = true

	func is_undo(event: InputEvent) -> bool:
		var shortcut := EditorInterface.get_editor_settings().get_shortcut(UNDO_SHORTCUT)
		return shortcut != null and event.is_pressed() and not event.is_echo() and shortcut.matches_event(event)

	func undo() -> bool:
		return _press(UNDO_SHORTCUT)

	func redo() -> bool:
		return _press(REDO_SHORTCUT)

	func get_undo_name(context: Object) -> String:
		var id := _manager.get_object_history_id(context)
		if not _histories.has(id):
			return ""
		var history := _manager.get_history_undo_redo(id)
		return history.get_current_action_name() if history != null and history.has_undo() else ""

	## Presses the editor's shortcut of this name, as its key would be pressed; whether it has one.
	func _press(shortcut_name: String) -> bool:
		var shortcut := EditorInterface.get_editor_settings().get_shortcut(shortcut_name)
		if shortcut == null:
			return false
		for event: InputEvent in shortcut.events:
			var key := event as InputEventKey
			if key == null:
				continue
			var down := key.duplicate() as InputEventKey
			down.pressed = true
			Input.parse_input_event(down)
			var up := key.duplicate() as InputEventKey
			up.pressed = false
			Input.parse_input_event(up)
			return true
		return false


func _enter_tree() -> void:
	_history = EditorHistory.new(get_undo_redo())
	_dock = DOCK_SCENE.instantiate() as EditorDock
	_panel = _dock.get_node("%Panel") as LandformLandPanel
	_panel.frame_requested.connect(_on_frame_requested, CONNECT_DEFERRED)
	add_dock(_dock)
	_dock.close()
	_bar = BAR_SCENE.instantiate() as LandformLandBar
	add_control_to_container(CONTAINER_SPATIAL_EDITOR_BOTTOM, _bar)
	_bar.hide()
	_toolbar = TOOLBAR_SCENE.instantiate() as LandformLandToolbar
	add_control_to_container(CONTAINER_SPATIAL_EDITOR_MENU, _toolbar)
	_toolbar.hide()
	_toolbar.walk_toggled.connect(_set_walk_picking)
	scene_saved.connect(_on_scene_saved)
	_open_dialog = OPEN_DIALOG_SCENE.instantiate() as ConfirmationDialog
	_open_dialog.set_unparent_when_invisible(true)
	_open_dialog.confirmed.connect(_add_land_view)
	_open_dialog.add_button("Open Authoring Scene").pressed.connect(_open_authoring_from_dialog)
	add_tool_menu_item(OPEN_LANDFORM, _open_landform)


func _exit_tree() -> void:
	remove_tool_menu_item(OPEN_LANDFORM)
	_open_dialog.queue_free()
	scene_saved.disconnect(_on_scene_saved)
	for session: LandformLandSession in _sessions.values():
		session.close()
	_sessions.clear()
	_session = null
	_history = null
	remove_dock(_dock)
	_dock.queue_free()
	remove_control_from_container(CONTAINER_SPATIAL_EDITOR_BOTTOM, _bar)
	_bar.queue_free()
	remove_control_from_container(CONTAINER_SPATIAL_EDITOR_MENU, _toolbar)
	_toolbar.queue_free()


## Opens the current scene's land editor or offers a setup from the project's authoring scene.
func _open_landform() -> void:
	var root := EditorInterface.get_edited_scene_root()
	if _open_scene_land_view(root):
		return
	var scene_path: String = str(ProjectSettings.get_setting(AUTHORING_SCENE_SETTING, ""))
	if scene_path.is_empty():
		_landform_notice("No Landform view in this scene. Set %s to a scene with a configured Landform view." % AUTHORING_SCENE_SETTING)
		return
	if not ResourceLoader.exists(scene_path, "PackedScene"):
		_landform_notice("Landform authoring scene could not be found: %s" % scene_path)
		return
	scene_path = ResourceUID.ensure_path(scene_path)
	if root is Node3D and root.scene_file_path != scene_path:
		_open_dialog_root = root
		_open_dialog_authoring_path = scene_path
		EditorInterface.popup_dialog_centered(_open_dialog)
	else:
		_open_authoring_scene(scene_path)


func _open_authoring_from_dialog() -> void:
	_open_dialog.hide()
	_open_authoring_scene(_open_dialog_authoring_path)


func _open_authoring_scene(scene_path: String) -> void:
	var root := EditorInterface.get_edited_scene_root()
	if root == null or root.scene_file_path != scene_path:
		EditorInterface.open_scene_from_path(scene_path)
		root = EditorInterface.get_edited_scene_root()
	if root == null or root.scene_file_path != scene_path:
		_landform_notice("Could not open the Landform authoring scene: %s" % scene_path)
	elif not _open_scene_land_view(root):
		_landform_notice("The Landform authoring scene has no editable Landform view: %s" % scene_path)


## Copy the authoring scene's terrain setup without its land file or any live editor state.
func _add_land_view() -> void:
	var root := EditorInterface.get_edited_scene_root()
	if not root is Node3D or root != _open_dialog_root or _open_scene_land_view(root):
		return
	var packed := ResourceLoader.load(_open_dialog_authoring_path, "PackedScene") as PackedScene
	if packed == null:
		_landform_notice("Could not read the Landform authoring scene: %s" % _open_dialog_authoring_path)
		return
	var template := packed.instantiate()
	if template == null:
		_landform_notice("Could not read the Landform authoring scene: %s" % _open_dialog_authoring_path)
		return
	var view := _configured_land_view(template)
	template.free()
	if view == null:
		_landform_notice("The authoring scene needs a Landform view with a terrain look and generation config.")
		return

	var undo := get_undo_redo()
	undo.create_action("Add Landform View", UndoRedo.MERGE_DISABLE, root)
	undo.add_do_method(root, &"add_child", view, true)
	undo.add_do_method(view, &"set_owner", root)
	undo.add_undo_method(root, &"remove_child", view)
	undo.add_do_reference(view)
	undo.commit_action()
	_reveal_land_view(view)


static func _configured_land_view(template: Node) -> LandformLandView:
	for node: Node in _land_views(template):
		var source := node as LandformLandView
		if source.look == null or source.config == null:
			continue
		var view := LAND_VIEW_SCENE.instantiate() as LandformLandView
		view.look = source.look
		view.config = source.config
		if source.terrain_material != null:
			view.terrain_material = source.terrain_material.duplicate() as Terrain3DMaterial
		return view
	return null


## Matches the native scene selector: prune branches hidden inside a non-editable instance.
static func _land_views(root: Node) -> Array[Node]:
	var views: Array[Node] = []
	_collect_land_views(root, root, views)
	return views


static func _collect_land_views(root: Node, node: Node, views: Array[Node]) -> void:
	var scene_owner := node.owner
	if node != root and scene_owner != root and (scene_owner == null or not root.is_editable_instance(scene_owner)):
		return
	if node is LandformLandView:
		views.append(node)
	for child: Node in node.get_children():
		_collect_land_views(root, child, views)


## Whether the scene supplied a view or a choice of views.
func _open_scene_land_view(root: Node) -> bool:
	if root == null:
		return false
	var views := _land_views(root)
	if views.is_empty():
		return false
	var selected := EditorInterface.get_selection().get_selected_nodes()
	if selected.size() == 1 and selected[0] in views:
		_reveal_land_view(selected[0] as LandformLandView)
	elif views.size() == 1:
		_reveal_land_view(views[0] as LandformLandView)
	else:
		EditorInterface.popup_node_selector(_on_land_view_chosen.bind(root), [LAND_VIEW_SCRIPT])
	return true


func _on_land_view_chosen(path: NodePath, root: Node) -> void:
	if path.is_empty() or not is_instance_valid(root) or root != EditorInterface.get_edited_scene_root():
		return
	var view := root.get_node_or_null(path) as LandformLandView
	if view != null:
		_reveal_land_view(view)


func _reveal_land_view(view: LandformLandView) -> void:
	var spatial_dock := EditorInterface.get_dock_by_name("3D")
	if spatial_dock != null:
		spatial_dock.make_visible()
	var selection := EditorInterface.get_selection()
	selection.clear()
	selection.add_node(view)
	EditorInterface.edit_node(view)
	_make_visible(true)


func _landform_notice(message: String) -> void:
	EditorInterface.get_editor_toaster().push_toast(message, EditorToaster.SEVERITY_WARNING)


func _handles(object: Object) -> bool:
	return object is LandformLandView


func _edit(object: Object) -> void:
	var view := object as LandformLandView
	_session = _session_of(view) if view != null else null
	_panel.show_session(_session)
	_bar.show_session(_session)
	_toolbar.show_session(_session)
	_set_walk_picking(false)
	_walk_pressed = false


func _make_visible(visible: bool) -> void:
	if visible:
		_dock.open()
		_dock.make_visible()
	else:
		_dock.close()
		_set_walk_picking(false)
		_walk_pressed = false
	_bar.visible = visible
	_toolbar.visible = visible


## The 3D view's events go to the edited view's session, its tools among them; one the session takes is the tool's
## alone. The editor stops handling it in the 3D view on AFTER_GUI_INPUT_STOP, but does not mark it handled, so a key
## would go on to the editor's shortcuts (the scene tree's Delete removes the selected node, the view): the window
## the 3D view lies in is told it is handled. One the session does not take (the right and middle buttons, the wheel)
## moves the editor's camera as ever.
func _forward_3d_gui_input(viewport_camera: Camera3D, event: InputEvent) -> AfterGUIInput:
	var viewport := viewport_camera.get_viewport() as SubViewport
	if viewport != null and viewport.get_camera_3d() == viewport_camera:
		_last_viewport = viewport
	var handled := false
	if _walk_picking or _walk_pressed:
		handled = _walk_input(viewport_camera, event)
	elif _session != null:
		handled = _session.input(event, viewport_camera)
	if not handled:
		return AFTER_GUI_INPUT_PASS
	var container := viewport_camera.get_viewport().get_parent()
	if container != null:
		container.get_viewport().set_input_as_handled()
	return AFTER_GUI_INPUT_STOP


func _set_walk_picking(picking: bool) -> void:
	_walk_picking = picking
	_toolbar.set_walk_picking(picking)
	var hint := ""
	if picking:
		hint = "Click the ground to start walking."
		if SETTINGS.stop_shortcut != null:
			hint += " Press %s to cancel." % SETTINGS.stop_shortcut.get_as_text()
	_bar.show_walk_hint(hint)


func _walk_input(camera: Camera3D, event: InputEvent) -> bool:
	var stop := SETTINGS.stop_shortcut
	if _walk_picking and stop != null and stop.matches_event(event) and event.is_pressed() and not event.is_echo():
		_set_walk_picking(false)
		return true
	var key := event as InputEventKey
	if _walk_picking and key != null and key.keycode == KEY_DELETE:
		return true
	var mouse := event as InputEventMouseButton
	if mouse == null:
		return _walk_pressed and event is InputEventMouseMotion
	if mouse.button_index != MOUSE_BUTTON_LEFT:
		return false
	if mouse.pressed:
		_walk_pressed = _walk_picking and not (
			mouse.alt_pressed or mouse.ctrl_pressed or mouse.shift_pressed or mouse.meta_pressed
		)
		return _walk_pressed
	if not _walk_pressed:
		return false
	_walk_pressed = false
	if _walk_picking:
		_walk_from(camera, mouse.position)
	return true


func _walk_from(camera: Camera3D, screen: Vector2) -> void:
	var reason := LandformLandToolbar.walk_unavailable(_session)
	if not reason.is_empty():
		_set_walk_picking(false)
		EditorInterface.get_editor_toaster().push_toast(reason, EditorToaster.SEVERITY_WARNING)
		return
	var view := _session.get_view()
	var hit := view.find_ground(camera.project_ray_origin(screen), camera.project_ray_normal(screen))
	if hit == LandformLandView.NO_GROUND:
		_bar.show_walk_hint("Click a spot on the land to start walking.")
		return
	_set_walk_picking(false)
	var path := view.get_land_path()
	var pending := _session.has_pending_walk_edits()
	var args := LandformLandToolbar.walk_arguments(LandformLandToolbar.walk_scene(), path, hit, pending)
	if OS.create_instance(args) <= 0:
		EditorInterface.get_editor_toaster().push_toast("Could not launch the walking preview.", EditorToaster.SEVERITY_WARNING)
	elif pending:
		EditorInterface.get_editor_toaster().push_toast("Walking the last saved terrain in %s." % path.get_file())


func _on_frame_requested(selection: Dictionary) -> void:
	if _session == null or not _session.is_alive():
		return
	var view := _session.get_view()
	var room := LAND_FRAMING.bounds(_session.get_working_drawing(), view.get_ground(), selection)
	if not room.has_surface():
		return
	var target: SubViewport
	for index: int in VIEWPORT_COUNT:
		var viewport := EditorInterface.get_editor_viewport_3d(index)
		var host := viewport.get_parent() as Control
		if host == null or not host.is_visible_in_tree():
			continue
		if viewport == _last_viewport:
			target = viewport
			break
		if target == null:
			target = viewport
	if target != null:
		LAND_FRAMING.frame(target, room, view.global_transform)


func _process(_delta: float) -> void:
	for id: int in _sessions.keys():
		var session := _sessions[id]
		if session.is_alive():
			session.poll()
			# A new land's file set on a view marks the view's own scene changed, once it is the scene edited.
			if session.take_scene_mark(EditorInterface.get_edited_scene_root()):
				EditorInterface.mark_scene_as_unsaved()
			continue
		session.close()
		_sessions.erase(id)
		if session == _session:
			_edit(null)


func _save_external_data() -> void:
	# The extension reports a land that cannot be written as an error.
	var open_roots: Array[Node] = []
	open_roots.assign(EditorInterface.get_open_scene_roots())
	LandformLandSession.save_lands(LandformLandSession.scenes_saved(open_roots, _saved_scenes, EditorInterface.get_edited_scene_root()))
	_saved_scenes.clear()


func _get_unsaved_status(for_scene: String) -> String:
	var unmarked: Array[LandformLandView] = []
	for session: LandformLandSession in _sessions.values():
		if session.has_scene_mark():
			unmarked.append(session.get_view())
	return LandformLandSession.unsaved_status(_scene_roots(PackedStringArray() if for_scene.is_empty() else PackedStringArray([for_scene])), unmarked)


func _session_of(view: LandformLandView) -> LandformLandSession:
	var id := view.get_instance_id()
	if not _sessions.has(id):
		_sessions[id] = LandformLandSession.new(view, SETTINGS, _history)
	return _sessions[id]


func _on_scene_saved(path: String) -> void:
	_saved_scenes.append(path)


## The open scenes whose files are named, or every open scene when none is (what the editor asks before it closes).
func _scene_roots(files: PackedStringArray) -> Array[Node]:
	var roots: Array[Node] = []
	for root: Node in EditorInterface.get_open_scene_roots():
		if files.is_empty() or root.scene_file_path in files:
			roots.append(root)
	return roots
