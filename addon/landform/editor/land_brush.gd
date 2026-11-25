@tool
class_name LandformBrush
extends RefCounted
## The dabs of a stroke on a picture laid over the land, for the editor tool's brushes (High ground, Hold and Regrow an
## area as drawn). The picture is the drawing's picture of high ground, an Image.FORMAT_RF of whatever size it has,
## stretched over the whole land as the core reads it (pixel (p, q)'s middle lies ((p + 0.5) / width - 0.5) times the
## land's side east of the centre and ((q + 0.5) / height - 0.5) times it south), or a mask of the land's size, an
## Image.FORMAT_L8 with cell (i, j) at pixel (i, j), its middle at (i - half, j - half), a metre a cell, as the land's
## area and held pictures are. The brush paints the picture given it, in place.
##
## A dab is the brush at a place of the land: a radius in metres and, on a picture, a strength; its falloff is 1 at its
## middle and falls smoothly to 0 at its radius (1 - smoothstep), so a pixel whose middle lies at or beyond the radius
## is not touched. A stroke is the dabs at every place along the pointer's path: `begin`, then `dab` where it starts
## and `drag_to` at each move, each painting the dabs along the straight line from the last place; how fast the pointer
## moves and how many events it sends change nothing.
##
## On a picture a stroke moves each pixel toward white (lighter) or black (darker) by the strength times the falloff of
## its nearest place on the path, from the picture as it was when the stroke began, each value read as the core reads
## it, held to 0..1, so a pixel above 1 or below 0 moves as the land sees it (stage 2's ruling R18, N8); the value
## painted is held to 0..1 too: a stroke that passes a place again moves it no further, and a pixel the stroke does
## not reach keeps its value bit for bit. On a mask a stroke sets every cell whose middle lies within the radius of the
## path (where the falloff is above 0) to the stroke's mark, 255 to mark it and 0 to clear it; a mask has no strength,
## since a cell is in it or not.
##
## Only the pixels in the dabs' rows are visited, each row's span worked out from the path, so the cost of a move is
## the area it paints, not the picture's: a mask's spans are filled by Image.fill_rect, a picture's pixels set one by
## one.

## The picture a brush paints: one of high ground, or a mask of the land's cells.
enum Kind { PICTURE, MASK }

## What a mask's cell holds where a stroke marks it and where it clears it.
const MARKED := 255
const CLEARED := 0
## A drag shorter than this, in metres, paints the dab where it ends alone.
const SAME_PLACE_M := 1e-9

## The radius in metres of the stroke's dabs, and on a picture how far a pixel at its path's middle moves (0 to 1).
var radius_m := 1.0
var strength := 0.0

var _image: Image
var _kind := Kind.MASK
var _size_m := 0
var _width := 0
var _height := 0
## On a picture: the values when the stroke began, and the falloff each pixel was given by the stroke (the most of its
## dabs'); +1 to lighten, -1 to darken.
var _base := PackedFloat32Array()
var _cover := PackedFloat32Array()
var _sign := 1.0
## On a mask: the colour a stroke fills its cells with.
var _mark := Color.WHITE
## The last place the stroke painted, east and south in metres; whether a stroke goes on.
var _last := Vector2.ZERO
var _stroking := false


## A brush over `image` laid over a land of `size_m` metres: an Image.FORMAT_RF picture of any size, or an
## Image.FORMAT_L8 mask of `size_m` pixels a side. Any other image is refused: `is_valid()` is then false and the brush
## paints nothing.
func _init(image: Image, size_m: int) -> void:
	_size_m = size_m
	if image == null or size_m <= 0 or image.is_empty() or image.has_mipmaps():
		return
	if image.get_format() == Image.FORMAT_RF:
		_kind = Kind.PICTURE
	elif image.get_format() == Image.FORMAT_L8 and image.get_width() == size_m and image.get_height() == size_m:
		_kind = Kind.MASK
	else:
		return
	_image = image
	_width = image.get_width()
	_height = image.get_height()


## Whether the brush has a picture or a mask to paint.
func is_valid() -> bool:
	return _image != null


func get_kind() -> Kind:
	return _kind


## The picture or mask the brush paints.
func get_image() -> Image:
	return _image


## Begins a stroke: on a picture lighter (`raise`) or darker, from the picture as it is now; on a mask marking
## (`raise`) or clearing its cells.
func begin(raise: bool) -> void:
	_stroking = is_valid()
	if not _stroking:
		return
	if _kind == Kind.PICTURE:
		_base = _image.get_data().to_float32_array()
		_cover = PackedFloat32Array()
		_cover.resize(_base.size())
		_sign = 1.0 if raise else -1.0
	else:
		_mark = Color.WHITE if raise else Color.BLACK


## Whether a stroke goes on (begun, and not ended).
func is_stroking() -> bool:
	return _stroking


## Ends the stroke: what it painted stays painted.
func end() -> void:
	_stroking = false
	_base = PackedFloat32Array()
	_cover = PackedFloat32Array()


## Paints the dab at this place (east and south in metres, as the land's cells stand), where a stroke starts.
func dab(at: Vector2) -> void:
	_last = at
	_paint(at, at)


## Paints the dabs along the straight line from the last place painted to this one, where the pointer moved.
func drag_to(at: Vector2) -> void:
	var from := _last
	_last = at
	_paint(from, at)


## The falloff of a dab of `radius` at `distance` from its middle: 1 at the middle, falling smoothly to 0 at the radius
## and 0 beyond it.
static func falloff(distance: float, radius: float) -> float:
	if radius <= 0.0 or distance >= radius:
		return 0.0
	return 1.0 - smoothstep(0.0, radius, distance)


## The distance from a place to the straight line from `from` to `to` (to `from` when the two are one place).
static func distance_to_line(place: Vector2, from: Vector2, to: Vector2) -> float:
	var along := to - from
	var length_squared := along.length_squared()
	if length_squared <= SAME_PLACE_M * SAME_PLACE_M:
		return place.distance_to(from)
	var t := clampf((place - from).dot(along) / length_squared, 0.0, 1.0)
	return place.distance_to(from + along * t)


## The span, east in metres, of the places in the row `south` that lie nearer than `radius` to the line from `from` to
## `to`: (west end, east end), open at both ends, or the first beyond the second for none. The places nearer than the
## radius are the discs at both ends and the band between them, each of whose rows is a span, and their union, the
## line's dabs, is a span too.
static func row_span(south: float, from: Vector2, to: Vector2, radius: float) -> Vector2:
	var span := Vector2(INF, -INF)
	for end_place: Vector2 in [from, to]:
		var across := south - end_place.y
		if absf(across) < radius:
			var half := sqrt(radius * radius - across * across)
			span = Vector2(minf(span.x, end_place.x - half), maxf(span.y, end_place.x + half))
	var along := to - from
	var length := along.length()
	if length <= SAME_PLACE_M:
		return span
	var unit := along / length
	var normal := Vector2(-unit.y, unit.x)
	# Along the row, a place's distance along the line, t = unit.x * east + t0, lies in 0..length, and its distance off
	# it, s = normal.x * east + s0, within the radius either side (not at it): each a span of the row's east.
	var t0 := unit.y * (south - from.y) - unit.x * from.x
	var s0 := normal.y * (south - from.y) - normal.x * from.x
	var band := Vector2(-INF, INF)
	if unit.x == 0.0:
		if t0 < 0.0 or t0 > length:
			return span
	else:
		band = _narrowed(band, -t0 / unit.x, (length - t0) / unit.x)
	if normal.x == 0.0:
		if absf(s0) >= radius:
			return span
	else:
		band = _narrowed(band, (-radius - s0) / normal.x, (radius - s0) / normal.x)
	if band.x < band.y:
		span = Vector2(minf(span.x, band.x), maxf(span.y, band.y))
	return span


## The span `band` narrowed to lie between `a` and `b`, in either order.
static func _narrowed(band: Vector2, a: float, b: float) -> Vector2:
	return Vector2(maxf(band.x, minf(a, b)), minf(band.y, maxf(a, b)))


func _paint(from: Vector2, to: Vector2) -> void:
	if not _stroking or radius_m <= 0.0:
		return
	if _kind == Kind.MASK:
		_paint_mask(from, to)
	else:
		_paint_picture(from, to)


## Fills the mask's cells whose middles lie nearer than the radius to the line with the stroke's mark, a row's span at
## a time.
func _paint_mask(from: Vector2, to: Vector2) -> void:
	var half := _size_m / 2.0
	# Cell j's middle lies at j - half, so the rows whose middles lie within the radius of the line's south.
	var first_row := maxi(int(floor(minf(from.y, to.y) - radius_m + half)) + 1, 0)
	var last_row := mini(int(ceil(maxf(from.y, to.y) + radius_m + half)) - 1, _size_m - 1)
	for j: int in range(first_row, last_row + 1):
		var span := row_span(j - half, from, to, radius_m)
		if span.x >= span.y:
			continue
		var first := maxi(int(floor(span.x + half)) + 1, 0)
		var last := mini(int(ceil(span.y + half)) - 1, _size_m - 1)
		if first <= last:
			_image.fill_rect(Rect2i(first, j, last - first + 1, 1), _mark)


## Moves the picture's pixels whose middles lie nearer than the radius to the line by the strength times their falloff
## (the most the stroke gave each), from the picture as the stroke began, held to 0..1.
func _paint_picture(from: Vector2, to: Vector2) -> void:
	var across := float(_size_m) / _width
	var down := float(_size_m) / _height
	var half := _size_m / 2.0
	# Pixel q's middle lies (q + 0.5) * down - half south of the centre.
	var first_row := maxi(int(floor((minf(from.y, to.y) - radius_m + half) / down - 0.5)) + 1, 0)
	var last_row := mini(int(ceil((maxf(from.y, to.y) + radius_m + half) / down - 0.5)) - 1, _height - 1)
	for q: int in range(first_row, last_row + 1):
		var south := (q + 0.5) * down - half
		var span := row_span(south, from, to, radius_m)
		if span.x >= span.y:
			continue
		var first := maxi(int(floor((span.x + half) / across - 0.5)) + 1, 0)
		var last := mini(int(ceil((span.y + half) / across - 0.5)) - 1, _width - 1)
		for p: int in range(first, last + 1):
			var place := Vector2((p + 0.5) * across - half, south)
			var weight := falloff(distance_to_line(place, from, to), radius_m)
			var index := q * _width + p
			if weight <= _cover[index]:
				continue
			_cover[index] = weight
			var value := clampf(clampf(_base[index], 0.0, 1.0) + _sign * strength * _cover[index], 0.0, 1.0)
			_image.set_pixel(p, q, Color(value, 0.0, 0.0))


## A copy of the drawing's picture of high ground as a brush paints it, an Image.FORMAT_RF without mipmaps read as the
## extension reads a drawing's picture (decompressed, its first channel's values, or its grey's), or, for a drawing
## without one, a picture of `side` pixels a side at 0, which grows as no picture does (low ground everywhere); null
## when the drawing's texture holds no picture or one that cannot be read.
static func picture_of(drawing: LandformDrawing, side: int) -> Image:
	if drawing == null or drawing.high_ground == null:
		return Image.create_empty(side, side, false, Image.FORMAT_RF)
	var held := drawing.high_ground.get_image()
	if held == null or held.is_empty():
		return null
	# The texture may hand out its own image, which must stay as it is.
	var copy := Image.new()
	copy.copy_from(held)
	if copy.is_compressed() and copy.decompress() != OK:
		return null
	copy.clear_mipmaps()
	copy.convert(Image.FORMAT_RF)
	return copy if copy.get_format() == Image.FORMAT_RF else null


## A texture over a picture a brush paints, for a drawing's high_ground: whoever reads it (`get_image`, as the
## extension's LandformDrawing does on the main thread) is handed the picture itself as it is now, so a drawing made
## at each move of a stroke reads no texture back from the GPU. It is never drawn, so it holds nothing on the GPU.
class Picture extends Texture2D:
	var image: Image

	func _init(picture: Image) -> void:
		image = picture

	func _get_width() -> int:
		return image.get_width()

	func _get_height() -> int:
		return image.get_height()

	func _get_image() -> Image:
		return image
