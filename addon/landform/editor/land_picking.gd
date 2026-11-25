@tool
class_name LandformPicking
extends RefCounted
## What lies under the pointer in the 3D view, for the editor tool's tools: of a drawing laid on the land's ground as
## the view's lines draw it (LandformLandLines), the river's point, the river's line or the lake nearest a position on
## the screen, within `pick_pixels`; on a line, where a point would be added and the index it would take; and the
## ground under a position (LandformLandView.find_ground along the camera's ray through it). Nothing of the land or
## the drawing changes.
##
## Distances are measured on the screen, in pixels, to what the lines draw there: a point's marker, a square of the
## overlay look's `marker_pixels` a side, and a line, `line_pixels` wide (a lake's middle is a marker, its rim a line).
## The nearest wins, and a marker beats a line at equal distance, so a click on a point's marker picks the point
## though its river's line runs through it, and a click on the line beside the marker adds a point there. Off every
## marker and line within the distance, a position whose ground lies inside a lake's disc picks that lake (its
## centre nearest the ground, when discs overlap).

## The key of a pick that says what was picked, and its kinds: a river's point, a river's line, a lake's middle, its
## rim, and the ground inside its disc.
const KIND := "kind"
const POINT := "point"
const LINE := "line"
const MIDDLE := "middle"
const RIM := "rim"
const INSIDE := "inside"
## The keys of a pick that name its item, each an index from 0, as a selection names them (LandformLandLines'
## SELECTED_*): the river, the point of it (a POINT pick), the lake (MIDDLE, RIM and INSIDE picks).
const RIVER := "river"
const LAKE := "lake"
## The keys of a LINE pick: the index in its river a point added there takes (the points from it on move up one), and
## where on the line, its east and south in metres, as 64-bit numbers.
const INDEX := "index"
const EAST := "east"
const SOUTH := "south"
## The key of every pick: how far from the position what was picked is drawn, in pixels (0 for INSIDE).
const PIXELS := "pixels"
## The key of _along_line's answer that says how far along the line its nearest place lies, 0 at its start and 1 at
## its end.
const ALONG := "along"
## What on_screen gives for a place behind the camera.
const OFF_SCREEN := Vector2.INF
## How much nearer the camera than an item's own ground point, in metres along the pointer's ray, the ground under the
## pointer must lie for the item to count as hidden behind it (`hides`, stage 2's ruling R18, S1).
const HIDDEN_LEAST_M := 1.0

## How near the position, in pixels, a marker or a line must be drawn to be picked (LandformToolSettings.pick_pixels).
var pick_pixels: float

var _view: LandformLandView


func _init(view: LandformLandView, pixels: float) -> void:
	_view = view
	pick_pixels = pixels


## What of `drawing` lies nearest `screen` (a position in `camera`'s viewport) within `pick_pixels`, as a pick: {} for
## nothing, {KIND: POINT, RIVER: i, POINT: j, PIXELS: d}, {KIND: LINE, RIVER: i, INDEX: j, EAST: e, SOUTH: s,
## PIXELS: d}, or {KIND: MIDDLE, RIM or INSIDE, LAKE: k, PIXELS: d}. Nothing without a land, a drawing or a camera.
func pick(drawing: LandformDrawing, screen: Vector2, camera: Camera3D) -> Dictionary:
	var best: Dictionary = {}
	var ground := _ground_now()
	if drawing == null or camera == null or ground == null:
		return best
	var look := _look()
	var marker_half := look.marker_pixels / 2.0
	var line_half := look.line_pixels / 2.0
	for r: int in drawing.rivers.size():
		var river := drawing.rivers[r]
		if river == null:
			continue
		var points := LandformLandLines.places(river.points_m)
		for p: int in points.size():
			var pixels := _to_marker(screen, on_ground(ground, points[p], camera), marker_half)
			best = _nearer(best, {KIND: POINT, RIVER: r, POINT: p, PIXELS: pixels})
		for p: int in points.size() - 1:
			var near := _along_line(screen, ground, points[p], points[p + 1], camera, line_half)
			if near.is_empty():
				continue
			var at: float = near[ALONG]
			var from_east := river.points_m[2 * p]
			var from_south := river.points_m[2 * p + 1]
			best = _nearer(best, {
				KIND: LINE, RIVER: r, INDEX: p + 1,
				EAST: from_east + (river.points_m[2 * p + 2] - from_east) * at,
				SOUTH: from_south + (river.points_m[2 * p + 3] - from_south) * at,
				PIXELS: near[PIXELS],
			})
	for k: int in drawing.lakes.size():
		var lake := drawing.lakes[k]
		if lake == null:
			continue
		var centre := Vector2(lake.x_m, lake.z_m)
		best = _nearer(best, {KIND: MIDDLE, LAKE: k, PIXELS: _to_marker(screen, on_ground(ground, centre, camera), marker_half)})
		var edge := LandformLandLines.rim(centre, lake.radius_m)
		for e: int in edge.size() - 1:
			var near := _along_line(screen, ground, edge[e], edge[e + 1], camera, line_half)
			if not near.is_empty():
				best = _nearer(best, {KIND: RIM, LAKE: k, PIXELS: near[PIXELS]})
	if best.is_empty():
		best = _inside_lake(drawing, screen, camera)
	return best


## The ground under `screen`, a position in `camera`'s viewport: where the camera's ray through it first meets the
## land (LandformLandView.find_ground), or LandformLandView.NO_GROUND.
func ground_under(screen: Vector2, camera: Camera3D) -> Vector3:
	if camera == null:
		return LandformLandView.NO_GROUND
	return _view.find_ground(camera.project_ray_origin(screen), camera.project_ray_normal(screen))


## The height of the ground at a place of the land (east, south in metres) as the lines lay it; NAN without a land.
func height_at(place: Vector2) -> float:
	var ground := _ground_now()
	return ground.at(place).y if ground != null else NAN


## Where the camera's ray through `screen`, a position in its viewport, meets the level plane at `height`: a drag
## measures its item's move on the level through the item's own ground (height_at), so an item grabbed where nearer
## ground lies over it in the view does not jump to that ground. LandformLandView.NO_GROUND when the ray runs level or
## away from the plane, or there is no camera.
func on_level(screen: Vector2, camera: Camera3D, height: float) -> Vector3:
	if camera == null or not is_finite(height):
		return LandformLandView.NO_GROUND
	var from := camera.project_ray_origin(screen)
	var along := camera.project_ray_normal(screen)
	if along.y == 0.0:
		return LandformLandView.NO_GROUND
	var t := (height - from.y) / along.y
	return from + along * t if t >= 0.0 else LandformLandView.NO_GROUND


## Whether an item whose own ground point is `item` is hidden at `screen` (a position in `camera`'s viewport: the
## item's own place on the screen, on_screen of it, where its marker is drawn; a press near the marker is not asked,
## since its ray meets ground in front of any item in a slanted view, stage 2's ruling R19, M1): the ground under the
## position lies nearer the camera than the item, along the camera's ray through the position, by more than
## HIDDEN_LEAST_M, as where a marker is drawn over a nearer hill (the lines are drawn over everything). A drag of a hidden item measures the pointer on the level through the item's ground (on_level), so it
## does not jump to that hill; any other drag by the ground under the pointer, so the item stays under it on slopes
## (stage 2's ruling R18, S1, amending R17, N1). False where no ground lies under the position.
func hides(screen: Vector2, camera: Camera3D, item: Vector3) -> bool:
	var under := ground_under(screen, camera)
	if under == LandformLandView.NO_GROUND:
		return false
	var from := camera.project_ray_origin(screen)
	var along := camera.project_ray_normal(screen)
	return (item - from).dot(along) - (under - from).dot(along) > HIDDEN_LEAST_M


## Where a drag measures the pointer at `screen`: on the level plane at `height` (on_level) for an item hidden at the
## press (`level`), else the ground under it (ground_under); LandformLandView.NO_GROUND for neither.
func drag_point(screen: Vector2, camera: Camera3D, height: float, level: bool) -> Vector3:
	return on_level(screen, camera, height) if level else ground_under(screen, camera)


## Where a place of the land (east, south in metres), on its ground as the lines lay it, is drawn in `camera`'s
## viewport; OFF_SCREEN behind the camera or without a land.
func on_screen(place: Vector2, camera: Camera3D) -> Vector2:
	var ground := _ground_now()
	if ground == null or camera == null:
		return OFF_SCREEN
	return on_ground(ground, place, camera)


## Where a place on this ground is drawn in the camera's viewport, OFF_SCREEN behind it.
static func on_ground(ground: LandformLandLines.Ground, place: Vector2, camera: Camera3D) -> Vector2:
	var at := ground.at(place)
	if camera.is_position_behind(at):
		return OFF_SCREEN
	return camera.unproject_position(at)


## The land's ground as the view's lines lay a drawing on it (LandformLandView.get_ground, its heights taken once per
## world); null without a land.
func _ground_now() -> LandformLandLines.Ground:
	return _view.get_ground()


## The look the view draws its lines with.
func _look() -> LandformOverlayLook:
	return _view.get_overlay_look()


## How far `screen` lies from a marker drawn at `at`, a square of `half` pixels each way from it; INF off the screen.
static func _to_marker(screen: Vector2, at: Vector2, half: float) -> float:
	if at == OFF_SCREEN:
		return INF
	var apart := (screen - at).abs()
	return Vector2(maxf(apart.x - half, 0.0), maxf(apart.y - half, 0.0)).length()


## Of the line from `from` to `to` (east, south) as the lines lay it on the ground, in stretches of at most
## LandformLandLines.STEP_M, the stretch drawn nearest `screen`, measured to the line's middle: {PIXELS: how far from
## the line's edge, `half` pixels from its middle, 0 within it, which picks compare between items; ALONG: how far along
## the whole line the nearest place lies, 0 at `from` and 1 at `to`}; {} when every stretch lies behind the camera.
static func _along_line(screen: Vector2, ground: LandformLandLines.Ground, from: Vector2, to: Vector2, camera: Camera3D, half: float) -> Dictionary:
	var stretches := maxi(1, ceili(from.distance_to(to) / LandformLandLines.STEP_M))
	var nearest: Dictionary = {}
	var nearest_distance := INF
	var start := on_ground(ground, from, camera)
	for s: int in stretches:
		var end := on_ground(ground, from.lerp(to, (s + 1.0) / stretches), camera)
		if start != OFF_SCREEN and end != OFF_SCREEN:
			var closest := Geometry2D.get_closest_point_to_segment(screen, start, end)
			# The nearest stretch by its distance to the line's middle: within half the line's width every stretch
			# would lie at 0, and the first of them, not the nearest, would win.
			var distance := screen.distance_to(closest)
			if distance < nearest_distance:
				nearest_distance = distance
				var length := start.distance_to(end)
				var part := start.distance_to(closest) / length if length > 0.0 else 0.0
				nearest = {PIXELS: maxf(distance - half, 0.0), ALONG: (s + part) / stretches}
		start = end
	return nearest


## The nearer of two picks, the one held first when `candidate` lies beyond `pick_pixels`; at equal distance a marker
## (a point or a lake's middle) beats a line (a river's line or a lake's rim), and otherwise the one held first stays.
func _nearer(best: Dictionary, candidate: Dictionary) -> Dictionary:
	var pixels: float = candidate[PIXELS]
	if pixels > pick_pixels:
		return best
	if best.is_empty() or pixels < float(best[PIXELS]):
		return candidate
	if pixels == float(best[PIXELS]) and candidate[KIND] in [POINT, MIDDLE] and best[KIND] in [LINE, RIM]:
		return candidate
	return best


## The lake whose disc holds the ground under `screen`, the one whose centre lies nearest when discs overlap, as an
## INSIDE pick; {} when none does or no ground lies under it.
func _inside_lake(drawing: LandformDrawing, screen: Vector2, camera: Camera3D) -> Dictionary:
	var hit := ground_under(screen, camera)
	if hit == LandformLandView.NO_GROUND:
		return {}
	var found: Dictionary = {}
	var nearest := INF
	for k: int in drawing.lakes.size():
		var lake := drawing.lakes[k]
		if lake == null:
			continue
		var apart := Vector2(hit.x - lake.x_m, hit.z - lake.z_m).length()
		if apart <= lake.radius_m and apart < nearest:
			nearest = apart
			found = {KIND: INSIDE, LAKE: k, PIXELS: 0.0}
	return found
