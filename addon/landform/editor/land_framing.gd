@tool
extends RefCounted
## Frames the editable drawing using the editor's own camera and navigation controller.

const SCREEN_MARGIN := 1.2
const POINT_CONTEXT_M := 6.0
const EDITOR_ZOOM_MIN_M := 0.01

static func bounds(drawing: LandformDrawing, ground: LandformLandLines.Ground, selection: Dictionary) -> AABB:
	if drawing == null or ground == null:
		return AABB()
	var river_index: int = selection.get(LandformLandSession.RIVER, -1)
	if river_index >= 0 and river_index < drawing.rivers.size():
		var river := drawing.rivers[river_index]
		if river == null:
			return AABB()
		var points := LandformLandLines.places(river.points_m)
		if points.is_empty():
			return AABB()
		var point_index: int = selection.get(LandformLandSession.POINT, -1)
		if point_index >= 0 and point_index < points.size():
			return AABB(ground.at(points[point_index]), Vector3.ZERO).grow(POINT_CONTEXT_M)
		return _line_bounds(points, ground).grow(river.width_m / 2.0)
	var lake_index: int = selection.get(LandformLandSession.LAKE, -1)
	if lake_index >= 0 and lake_index < drawing.lakes.size():
		var lake := drawing.lakes[lake_index]
		if lake != null:
			var centre := Vector2(lake.x_m, lake.z_m)
			var radius := lake.radius_m
			var rim := _line_bounds(LandformLandLines.rim(centre, radius), ground).expand(ground.at(centre))
			# The sampled rim does not necessarily include all four cardinal extremes.
			return AABB(Vector3(centre.x - radius, rim.position.y, centre.y - radius),
					Vector3(2.0 * radius, rim.size.y, 2.0 * radius))
	return AABB()


static func _line_bounds(points: Array[Vector2], ground: LandformLandLines.Ground) -> AABB:
	var area := AABB(ground.at(points[0]), Vector3.ZERO)
	for index: int in points.size() - 1:
		var from := points[index]
		var to := points[index + 1]
		var steps := maxi(1, ceili(from.distance_to(to) / LandformLandLines.STEP_M))
		for step: int in range(1, steps + 1):
			area = area.expand(ground.at(from.lerp(to, float(step) / steps)))
	return area


static func frame(viewport: SubViewport, room: AABB, placement: Transform3D) -> void:
	var camera := viewport.get_camera_3d()
	if camera == null or viewport.size.x <= 0 or viewport.size.y <= 0 or not room.has_surface():
		return
	var basis := camera.global_basis
	var limits := (Transform3D(basis.inverse(), Vector3.ZERO) * placement) * room
	var center := limits.get_center()
	var aspect := float(viewport.size.x) / viewport.size.y
	var tangent := tan(deg_to_rad(camera.fov) / 2.0)
	var half := limits.size / 2.0
	var screen_half := SCREEN_MARGIN * maxf(half.y, half.x / aspect)
	var distance := half.z + maxf(camera.near * 4.0, screen_half / tangent)
	if camera.projection == Camera3D.PROJECTION_ORTHOGONAL:
		_set_editor_distance(viewport, camera, maxf(screen_half / tangent, camera.near * 4.0))
		distance = (camera.far - camera.near) / 2.0
	else:
		_set_editor_distance(viewport, camera, distance)
	# The native editor adopts an externally moved editor camera on its next update.
	camera.global_position = basis * (center + Vector3(0.0, 0.0, distance))


static func _set_editor_distance(viewport: SubViewport, camera: Camera3D, distance: float) -> void:
	var container := viewport.get_parent() as SubViewportContainer
	if container == null:
		return
	var editor_view := container.get_parent()
	if editor_view == null or editor_view.get_class() != "Node3DEditorViewport":
		return
	var surface := _navigation_surface(editor_view)
	if surface == null:
		return
	var gesture := InputEventMagnifyGesture.new()
	gesture.position = surface.size / 2.0
	# The editor exposes neither its orbit distance nor an absolute zoom operation. First reach its near
	# zoom limit, then scale from there. Both gestures adjust the cursor before the camera updates.
	var min_distance := maxf(camera.near * 4.0, EDITOR_ZOOM_MIN_M)
	gesture.factor = maxf(1.0, camera.far / min_distance)
	surface.gui_input.emit(gesture)
	gesture.factor = min_distance / distance
	surface.gui_input.emit(gesture)


## Godot routes native 3D navigation through a Control's gui_input, beside the SubViewportContainer.
## Check its connection before sending a gesture, as the editor's controls are an internal detail.
static func _navigation_surface(editor_view: Node) -> Control:
	for child: Node in editor_view.get_children():
		var control := child as Control
		if control == null:
			continue
		for connection: Dictionary in control.gui_input.get_connections():
			var target: Callable = connection["callable"]
			if target.get_object() == editor_view and target.get_method() == &"Node3DEditorViewport::_sinput":
				return control
	return null
