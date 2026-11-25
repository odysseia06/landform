extends SceneTree
## Writes the water's patterns as pictures beside this script, so a scene loads them as they are and nothing is
## generated while it loads: ripples.png, the ripples as a normal map that tiles, RIPPLES_SIZE pixels a side, and
## foam.png, the foam as a grey picture that tiles, FOAM_SIZE pixels a side. The same constants give the same files,
## byte for byte. Run it from a project that holds the addon, then import the project:
##   godot --headless --path . -s res://addons/landform/water/make_water_patterns.gd < /dev/null
##
## The ripples are a height field of FastNoiseLite's noise, made to tile by get_seamless_image, turned into a normal
## map whose neighbours are taken round the edges, so the picture tiles without a seam (Image.bump_map_to_normal_map
## takes no neighbour round the edges). The slope at a pixel is read with a Sobel filter, the differences of the three
## columns (or rows) either side weighted 1, 2, 1, which also smooths the steps of the 8-bit heights. A pixel's red and
## green are the normal's tilt along the picture's x and y, which the water's shader lays along world x and z, and its
## blue the normal's upright part, each stored as (n + 1) / 2.
##
## The foam is two kinds of noise, each made to tile by get_seamless_image, added by their weights: patches, from
## simplex noise of several octaves, and bubbles, from cellular noise light along the cells' edges. The sum is then
## spread evenly over the whole range, from black to white: the pixels in order of their sum (ties in reading order),
## the first 1/256 of them black, the next 1/256 one grey lighter, and so on. So every grey holds as many pixels, and
## the share of the picture lighter than a grey g is 1 - g, to within one grey: the water's shader shows foam where the
## pattern is lighter than 1 - amount, so amount is the share of the water the foam covers.

## The picture written, in this script's folder.
const RIPPLES_FILE := "ripples.png"
## Pixels a side.
const RIPPLES_SIZE: int = 512
const RIPPLES_SEED: int = 20261118
const RIPPLES_NOISE := FastNoiseLite.TYPE_SIMPLEX_SMOOTH
## Cycles of the noise's first octave for each pixel: a ripple about every 40 pixels, a dozen across the picture.
const RIPPLES_FREQUENCY := 0.025
const RIPPLES_OCTAVES: int = 3
const RIPPLES_LACUNARITY := 2.0
const RIPPLES_GAIN := 0.5
## The share of the picture's size that get_seamless_image blends its seams over.
const RIPPLES_SKIRT := 0.1
## How far the height field rises from its lowest pixel to its highest, in pixels: how steep the ripples are.
const RIPPLES_RELIEF_PX := 24.0
## The weights of a Sobel filter sum to 8 times the slope across one pixel.
const SOBEL_SUM := 8.0

## The foam's picture, in this script's folder.
const FOAM_FILE := "foam.png"
## Pixels a side.
const FOAM_SIZE: int = 512
const FOAM_SEED: int = 20261119
## The patches: cycles of the simplex noise's first octave for each pixel, about six patches across the picture, so
## a little foam is a few clumps and not a scatter of specks.
const FOAM_PATCH_FREQUENCY := 0.012
const FOAM_PATCH_OCTAVES: int = 4
const FOAM_PATCH_GAIN := 0.5
## The bubbles: cells of the cellular noise's first octave for each pixel, about twenty across the picture; they leave
## round holes in the patches.
const FOAM_BUBBLE_FREQUENCY := 0.04
const FOAM_BUBBLE_OCTAVES: int = 2
const FOAM_BUBBLE_GAIN := 0.5
const FOAM_LACUNARITY := 2.0
## How much the patches and the bubbles weigh in the sum, each grey of theirs counted this many times.
const FOAM_PATCH_WEIGHT: int = 2
const FOAM_BUBBLE_WEIGHT: int = 1
## The share of the picture's size that get_seamless_image blends its seams over.
const FOAM_SKIRT := 0.1
## The greys of an 8-bit picture.
const GREYS: int = 256


func _initialize() -> void:
	var failed := _write_ripples()
	if failed == 0:
		failed = _write_foam()
	quit(failed)


func _write_ripples() -> int:
	var noise := FastNoiseLite.new()
	noise.seed = RIPPLES_SEED
	noise.noise_type = RIPPLES_NOISE
	noise.frequency = RIPPLES_FREQUENCY
	noise.fractal_type = FastNoiseLite.FRACTAL_FBM
	noise.fractal_octaves = RIPPLES_OCTAVES
	noise.fractal_lacunarity = RIPPLES_LACUNARITY
	noise.fractal_gain = RIPPLES_GAIN
	var heights := noise.get_seamless_image(RIPPLES_SIZE, RIPPLES_SIZE, false, false, RIPPLES_SKIRT)
	if heights == null or heights.get_size() != Vector2i(RIPPLES_SIZE, RIPPLES_SIZE):
		printerr("The noise gave no %d x %d picture." % [RIPPLES_SIZE, RIPPLES_SIZE])
		return 1
	heights.convert(Image.FORMAT_L8)
	var normals := _normal_map(heights.get_data(), RIPPLES_SIZE)
	var path := (get_script() as Script).resource_path.get_base_dir().path_join(RIPPLES_FILE)
	var err := normals.save_png(path)
	if err != OK:
		printerr("Cannot write %s: %s" % [path, error_string(err)])
		return 1
	print("wrote %s: %d x %d, %s" % [path, RIPPLES_SIZE, RIPPLES_SIZE, _tilt_summary(normals.get_data())])
	return 0


## The normal map of a square height field of `size` pixels a side, one byte a pixel, every neighbour taken round
## the edges: RGB8, each channel (n + 1) / 2 of the unit normal (-dh/dx, -dh/dy, 1) normalised, h in pixels.
func _normal_map(heights: PackedByteArray, size: int) -> Image:
	var per_step := RIPPLES_RELIEF_PX / 255.0 / SOBEL_SUM
	var data := PackedByteArray()
	data.resize(size * size * 3)
	for y: int in size:
		var above := ((y + size - 1) % size) * size
		var row := y * size
		var below := ((y + 1) % size) * size
		for x: int in size:
			var left := (x + size - 1) % size
			var right := (x + 1) % size
			var across := (heights[above + right] + 2 * heights[row + right] + heights[below + right]) \
				- (heights[above + left] + 2 * heights[row + left] + heights[below + left])
			var down := (heights[below + left] + 2 * heights[below + x] + heights[below + right]) \
				- (heights[above + left] + 2 * heights[above + x] + heights[above + right])
			var normal := Vector3(-across * per_step, -down * per_step, 1.0).normalized()
			var at := (row + x) * 3
			data[at] = roundi((normal.x + 1.0) * 127.5)
			data[at + 1] = roundi((normal.y + 1.0) * 127.5)
			data[at + 2] = roundi((normal.z + 1.0) * 127.5)
	return Image.create_from_data(size, size, false, Image.FORMAT_RGB8, data)


## The mean and the largest tilt of a normal map's pixels (the length of a normal's horizontal part), for the record.
func _tilt_summary(data: PackedByteArray) -> String:
	var total := 0.0
	var largest := 0.0
	var count := data.size() / 3.0
	for at: int in range(0, data.size(), 3):
		var tilt := Vector2(data[at] / 127.5 - 1.0, data[at + 1] / 127.5 - 1.0).length()
		total += tilt
		largest = maxf(largest, tilt)
	return "tilt mean %.3f, largest %.3f" % [total / count, largest]


func _write_foam() -> int:
	var patches := FastNoiseLite.new()
	patches.seed = FOAM_SEED
	patches.noise_type = FastNoiseLite.TYPE_SIMPLEX_SMOOTH
	patches.frequency = FOAM_PATCH_FREQUENCY
	patches.fractal_type = FastNoiseLite.FRACTAL_FBM
	patches.fractal_octaves = FOAM_PATCH_OCTAVES
	patches.fractal_lacunarity = FOAM_LACUNARITY
	patches.fractal_gain = FOAM_PATCH_GAIN
	var bubbles := FastNoiseLite.new()
	bubbles.seed = FOAM_SEED
	bubbles.noise_type = FastNoiseLite.TYPE_CELLULAR
	bubbles.frequency = FOAM_BUBBLE_FREQUENCY
	bubbles.fractal_type = FastNoiseLite.FRACTAL_FBM
	bubbles.fractal_octaves = FOAM_BUBBLE_OCTAVES
	bubbles.fractal_lacunarity = FOAM_LACUNARITY
	bubbles.fractal_gain = FOAM_BUBBLE_GAIN
	bubbles.cellular_distance_function = FastNoiseLite.DISTANCE_EUCLIDEAN
	# The distance to the second-nearest cell less that to the nearest is 0 along the cells' edges; inverted, the
	# edges are light.
	bubbles.cellular_return_type = FastNoiseLite.RETURN_DISTANCE2_SUB
	var patch_greys := _seamless_greys(patches, false)
	var bubble_greys := _seamless_greys(bubbles, true)
	if patch_greys.is_empty() or bubble_greys.is_empty():
		printerr("The noise gave no %d x %d picture." % [FOAM_SIZE, FOAM_SIZE])
		return 1
	var sums := PackedInt32Array()
	sums.resize(FOAM_SIZE * FOAM_SIZE)
	for at: int in sums.size():
		sums[at] = FOAM_PATCH_WEIGHT * patch_greys[at] + FOAM_BUBBLE_WEIGHT * bubble_greys[at]
	var foam := Image.create_from_data(FOAM_SIZE, FOAM_SIZE, false, Image.FORMAT_L8, _spread_evenly(sums))
	var path := (get_script() as Script).resource_path.get_base_dir().path_join(FOAM_FILE)
	var err := foam.save_png(path)
	if err != OK:
		printerr("Cannot write %s: %s" % [path, error_string(err)])
		return 1
	print("wrote %s: %d x %d, %s" % [path, FOAM_SIZE, FOAM_SIZE, _grey_summary(foam.get_data())])
	return 0


## The noise as a grey picture of FOAM_SIZE pixels a side that tiles, one byte a pixel, lightest where the noise is
## highest (or lowest, `inverted`); empty when the noise gives no such picture.
func _seamless_greys(noise: FastNoiseLite, inverted: bool) -> PackedByteArray:
	var image := noise.get_seamless_image(FOAM_SIZE, FOAM_SIZE, inverted, false, FOAM_SKIRT)
	if image == null or image.get_size() != Vector2i(FOAM_SIZE, FOAM_SIZE):
		return PackedByteArray()
	image.convert(Image.FORMAT_L8)
	return image.get_data()


## The values spread evenly over the greys of 8 bits: each pixel's grey is its rank among the values (ties in reading
## order) times GREYS over the count, so every grey holds as many pixels and the darkest is black and the lightest
## white. The values are 0 or more.
func _spread_evenly(values: PackedInt32Array) -> PackedByteArray:
	var top := 0
	for value: int in values:
		top = maxi(top, value)
	# How many pixels hold each value, then how many hold less: the rank of the first pixel of each value.
	var next_rank := PackedInt32Array()
	next_rank.resize(top + 1)
	for value: int in values:
		next_rank[value] += 1
	var ranked := 0
	for value: int in top + 1:
		var holding := next_rank[value]
		next_rank[value] = ranked
		ranked += holding
	var greys := PackedByteArray()
	greys.resize(values.size())
	for at: int in values.size():
		var value := values[at]
		# Whole greys: the rank's share of the count, rounded down.
		@warning_ignore("integer_division")
		greys[at] = next_rank[value] * GREYS / values.size()
		next_rank[value] += 1
	return greys


## The darkest and the lightest grey of a grey picture's pixels and its mean, for the record.
func _grey_summary(data: PackedByteArray) -> String:
	var darkest := 255
	var lightest := 0
	var total := 0
	for grey: int in data:
		darkest = mini(darkest, grey)
		lightest = maxi(lightest, grey)
		total += grey
	return "darkest %d, lightest %d, mean %.2f" % [darkest, lightest, float(total) / data.size()]
