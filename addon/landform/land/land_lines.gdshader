// The lines the editor tool draws over a land, whose mesh land_lines.gd makes: each line a strip of two triangles a
// stretch and each point a square marker of four corners at one place, widened here across the screen, so they keep
// their width in pixels at every distance and on every rendering driver (Direct3D 12 draws a point one pixel wide
// whatever size a material asks). Drawn over everything, so the ground and the water never hide them, in the colour
// each vertex holds. Every value of the look is a uniform of the same name.
shader_type spatial;
render_mode unshaded, depth_test_disabled, cull_disabled, shadows_disabled, fog_disabled;

// How wide a line is drawn and how large a point's marker, in pixels: LandformOverlayLook.line_pixels and
// marker_pixels, which LandformLandView hands over; neither has a default of its own here.
uniform float line_pixels;
uniform float marker_pixels;

void vertex() {
	vec4 own = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(VERTEX, 1.0));
	vec2 offset_pixels;
	if (CUSTOM0.w > 0.5) {
		// A line's corner: UV.x its side (-1 or 1), UV.y 1 at the stretch's start and -1 at its end, and CUSTOM0.xyz the
		// stretch's other end, so both ends of a stretch find the same way along it.
		vec4 other = PROJECTION_MATRIX * (MODELVIEW_MATRIX * vec4(CUSTOM0.xyz, 1.0));
		vec2 along = (other.xy / other.w - own.xy / own.w) * VIEWPORT_SIZE * UV.y;
		float along_length = length(along);
		vec2 across = along_length > 0.0 ? vec2(-along.y, along.x) / along_length : vec2(0.0);
		offset_pixels = across * UV.x * line_pixels * 0.5;
	} else {
		// A marker's corner: UV the corner, each part -1 or 1.
		offset_pixels = UV * marker_pixels * 0.5;
	}
	// Pixels to the clip space's units: the screen is 2 units across, and the clip space is w times that.
	own.xy += offset_pixels * 2.0 / VIEWPORT_SIZE * own.w;
	POSITION = own;
}

void fragment() {
	ALBEDO = COLOR.rgb;
	ALPHA = COLOR.a;
}
