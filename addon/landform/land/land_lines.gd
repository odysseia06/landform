@tool
class_name LandformLandLines
extends RefCounted
## The lines the editor tool draws over a land, made as a mesh: each river of a drawing as the line through its points,
## upstream first, with a marker on each point, and each lake as its rim with a marker at its centre, laid on the
## ground of the land's world (its heights, between cells from the four nearest). One ArrayMesh of triangles that
## land_lines.gdshader widens across the screen: a stretch of a line is two triangles whose four corners stand at its
## two ends and hold the stretch's other end in CUSTOM0 (its w 1, for a line) and their side and their end in UV; a
## marker is two triangles whose four corners stand at its point (CUSTOM0's w 0) and hold which corner they are in UV.
## Each vertex holds its colour from the look, made linear, since the shader is unshaded and writes it as it is.

## The keys of a selection (LandformLandView.show_drawing), each an index from 0: the river selected, the point of it
## selected (a point is of the river selected), and the lake selected. A selection without a key selects none of it.
const SELECTED_RIVER := "river"
const SELECTED_POINT := "point"
const SELECTED_LAKE := "lake"
## No item: the index a selection without the key stands for.
const NONE := -1
## The longest stretch of a line between two of its vertices, in metres, so the line follows the ground between two
## points of the drawing.
const STEP_M := 2.0
## The fewest stretches a lake's rim is drawn in, however small the lake.
const RIM_STRETCHES_LEAST := 24
## CUSTOM0's w on a line's corner and on a marker's, which the shader tells them apart by.
const LINE := 1.0
const MARKER := 0.0
## UV's y on a line's corner at the stretch's start and at its end, so both find the same way along it.
const AT_START := 1.0
const AT_END := -1.0
## UV's x on a line's corners on either side of it.
const SIDE_LEFT := -1.0
const SIDE_RIGHT := 1.0
## A marker's four corners in UV, round its square.
const CORNERS: Array[Vector2] = [Vector2(-1.0, -1.0), Vector2(1.0, -1.0), Vector2(1.0, 1.0), Vector2(-1.0, 1.0)]
## The two triangles of four corners, by their places among the four.
const QUAD: Array[int] = [0, 1, 2, 0, 2, 3]


## The mesh of a drawing's lines laid on a world's ground (a Ground of it, which the land's view keeps for its world),
## the item `selection` names in the look's selected colour; null for no drawing, no ground, a world of fewer than two
## cells a side or no look. A river entry that is empty, and a river of fewer than one point, draws nothing; one point
## draws its marker alone.
static func make_mesh(drawing: LandformDrawing, ground: Ground, selection: Dictionary, look: LandformOverlayLook) -> ArrayMesh:
	if drawing == null or ground == null or ground.get_size() < 2 or look == null:
		return null
	var parts := Parts.new()
	var selected_river: int = selection.get(SELECTED_RIVER, NONE)
	var selected_point: int = selection.get(SELECTED_POINT, NONE)
	var selected_lake: int = selection.get(SELECTED_LAKE, NONE)
	for r: int in drawing.rivers.size():
		var river := drawing.rivers[r]
		if river == null:
			continue
		var points := places(river.points_m)
		var line_colour := look.selected_color if r == selected_river else look.river_color
		for i: int in points.size() - 1:
			parts.add_line(ground, points[i], points[i + 1], line_colour)
		for i: int in points.size():
			var chosen := r == selected_river and i == selected_point
			parts.add_marker(ground.at(points[i]), look.selected_color if chosen else look.point_color)
	for l: int in drawing.lakes.size():
		var lake := drawing.lakes[l]
		if lake == null:
			continue
		var centre := Vector2(lake.x_m, lake.z_m)
		var chosen := l == selected_lake
		var edge := rim(centre, lake.radius_m)
		for i: int in edge.size() - 1:
			parts.add_line(ground, edge[i], edge[i + 1], look.selected_color if chosen else look.lake_color)
		parts.add_marker(ground.at(centre), look.selected_color if chosen else look.point_color)
	return parts.mesh()


## A river's points as places, east then south in metres, from its `points_m`; an odd number at the end is left out.
## The editor tool's picking (LandformPicking) reads them too, so it picks what is drawn.
static func places(points_m: PackedFloat64Array) -> Array[Vector2]:
	var found: Array[Vector2] = []
	for i: int in range(0, points_m.size() - 1, 2):
		found.append(Vector2(points_m[i], points_m[i + 1]))
	return found


## The places round a lake's rim, the first again at the end, at most STEP_M apart and never fewer than
## RIM_STRETCHES_LEAST stretches; the picking reads them too.
static func rim(centre: Vector2, radius_m: float) -> Array[Vector2]:
	var stretches := maxi(RIM_STRETCHES_LEAST, ceili(TAU * maxf(radius_m, 0.0) / STEP_M))
	var edge: Array[Vector2] = []
	for i: int in stretches + 1:
		edge.append(centre + Vector2.from_angle(TAU * i / stretches) * radius_m)
	return edge


## A world's ground as heights between its cells: cell (i, j) stands at (i - half, j - half), half the world's side,
## as Terrain3D shows it and as Landform's lands place their cells. It holds a copy of the world's heights
## (LandformWorld.get_height_image, 16 MB at 2,048 m), so it is made once for a world and kept while that world is
## shown (LandformLandView.get_ground).
class Ground extends RefCounted:
	var _heights: Image
	var _size: int
	var _half: float

	func _init(world: LandformWorld) -> void:
		_heights = world.get_height_image()
		_size = world.get_size()
		_half = _size / 2.0

	## The world's side in cells.
	func get_size() -> int:
		return _size

	## The ground at a place, east then south in metres, from the four nearest cells; beyond the world, the height at
	## its nearest edge.
	func at(place: Vector2) -> Vector3:
		var u := clampf(place.x + _half, 0.0, _size - 1.0)
		var v := clampf(place.y + _half, 0.0, _size - 1.0)
		var i := mini(floori(u), _size - 2)
		var j := mini(floori(v), _size - 2)
		var across := u - i
		var down := v - j
		var north := lerpf(_heights.get_pixel(i, j).r, _heights.get_pixel(i + 1, j).r, across)
		var south := lerpf(_heights.get_pixel(i, j + 1).r, _heights.get_pixel(i + 1, j + 1).r, across)
		return Vector3(place.x, lerpf(north, south, down), place.y)


## The mesh's arrays, filled a stretch or a marker at a time.
class Parts extends RefCounted:
	var _vertices := PackedVector3Array()
	var _uvs := PackedVector2Array()
	var _colours := PackedColorArray()
	var _custom := PackedFloat32Array()
	var _indices := PackedInt32Array()

	## The line from `from` to `to`, east then south in metres, on the ground, in stretches of at most STEP_M.
	func add_line(ground: Ground, from: Vector2, to: Vector2, colour: Color) -> void:
		var stretches := maxi(1, ceili(from.distance_to(to) / STEP_M))
		var start := ground.at(from)
		for k: int in stretches:
			var end := ground.at(from.lerp(to, (k + 1.0) / stretches))
			_add_stretch(start, end, colour)
			start = end

	## A marker at a point of the ground.
	func add_marker(at: Vector3, colour: Color) -> void:
		var first := _vertices.size()
		var linear := colour.srgb_to_linear()
		for corner: Vector2 in CORNERS:
			_add_vertex(at, corner, linear, Vector3.ZERO, MARKER)
		_add_quad(first)

	## The mesh of what was added; null when nothing was.
	func mesh() -> ArrayMesh:
		if _vertices.is_empty():
			return null
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = _vertices
		arrays[Mesh.ARRAY_TEX_UV] = _uvs
		arrays[Mesh.ARRAY_COLOR] = _colours
		arrays[Mesh.ARRAY_CUSTOM0] = _custom
		arrays[Mesh.ARRAY_INDEX] = _indices
		var made := ArrayMesh.new()
		var custom_format := Mesh.ARRAY_CUSTOM_RGBA_FLOAT << Mesh.ARRAY_FORMAT_CUSTOM0_SHIFT
		made.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {}, custom_format)
		return made

	func _add_stretch(start: Vector3, end: Vector3, colour: Color) -> void:
		var first := _vertices.size()
		var linear := colour.srgb_to_linear()
		_add_vertex(start, Vector2(SIDE_LEFT, AT_START), linear, end, LINE)
		_add_vertex(start, Vector2(SIDE_RIGHT, AT_START), linear, end, LINE)
		_add_vertex(end, Vector2(SIDE_RIGHT, AT_END), linear, start, LINE)
		_add_vertex(end, Vector2(SIDE_LEFT, AT_END), linear, start, LINE)
		_add_quad(first)

	func _add_vertex(at: Vector3, uv: Vector2, colour: Color, other: Vector3, kind: float) -> void:
		_vertices.append(at)
		_uvs.append(uv)
		_colours.append(colour)
		_custom.append_array(PackedFloat32Array([other.x, other.y, other.z, kind]))

	func _add_quad(first: int) -> void:
		for corner: int in QUAD:
			_indices.append(first + corner)
