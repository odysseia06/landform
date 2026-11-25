// Draws a world's water surface (the meshes of LandformWaterSurface), seen from above only: the mesh is wound so that
// its front face is its top, so cull_back leaves out its underside. It is drawn after everything solid, blends over
// it and writes no depth, and it reads the picture and the depth the engine drew before it: the picture is what is
// seen through the water, the depth is how far behind the surface the ground lies.
//
// The colour goes by the stored depth (the mesh's CUSTOM0 is the flow's x and z, the depth and the fall; all four are
// read here). How clear the water is goes from 1 at a depth of 0 to 0 at bed_gone_m. The surface's own colour is
// deep_color, lit by the sun and the sky like any surface (so it takes their shadows and darkens at night), weighted
// by what is not clear. What lies behind, multiplied by shallow_color, is seen through by how clear the
// water is; it is handed to the engine as light the surface gives off, as the engine's own refraction does
// (StandardMaterial3D), times EXPOSURE so the engine's exposure leaves it as it was drawn. It is the picture behind,
// so at night it is dark; nothing else of the water gives off light.
//
// The edge goes by the water drawn: the surface's alpha goes from 0 to 1 over edge_fade_m of water, measured as how
// far what was drawn behind the surface lies behind the surface's own plane: the distance along the view from the
// surface to it times the share of the view's direction across the face. The face's normal is taken from how the
// fragment's place changes from one pixel to the next (so it is one for each of the mesh's triangles), since the
// mesh's normals are all up. Measured upright, the measure would shrink on water that rises away from the viewer
// and vanish at and above the eye's height, hiding a brook seen from below; across the face it is the water's
// thickness under the surface on a fall too, and on level water it is how far what lies behind is below the surface,
// so a low camera does not widen the fade. A rim point of the mesh stores a depth of 0 but lies under the ground, so
// the stored depth is still some centimetres where the surface meets the ground; the depth drawn is 0 there.
//
// The ripples are ripple_pattern, a normal map whose red and green are its tilt along world x and z, laid over the
// world by world x and z, so the pieces share it without a seam. It is read at two sizes, ripple_size_m and
// ripple_fine_size_m metres a copy, which drift ripple_drift_m_s metres a second along one line in opposite
// directions, and are mixed in equal shares: on a lake that is all that moves, and it goes nowhere. The stored flow
// carries both sizes by the two-layer method: each layer's picture is taken from farther upstream as it ages, by
// flow_speed times the flow times its age, and starts again every flow_cycle_s seconds, the two layers half a cycle
// apart and the second sampled half a pattern aside, each weighted by how far it is from starting again, so neither
// is seen to jump, and their mix divided by the length of the two weights, so the ripples do not soften where the
// weights are equal and the surface does not throb every half cycle. The mixed tilt, times ripple_strength, leans the
// surface's normal; the meshes have no tangents, so the normal is built in world space and turned into view space.
// What is seen through the water is taken where a shift of the tilt times bend_m metres, at the water, falls in the
// picture: projected, so it shrinks with the distance from the camera. A shifted sample is taken only where what was
// drawn there lies behind the surface's own plane, as the edge measures it, and inside the picture; elsewhere, a bank
// or a blade of grass in front of the water, or standing above its plane beyond it, the straight one is taken, so
// nothing out of the water is smeared into it. No pattern reads flat (hint_normal), a size of 0 leaves that size out,
// and a cycle of 0 or less carries nothing; the shader's clock starts again every 3,600 s, which a cycle that divides
// it does not show, while the drift jumps once.
//
// White water: where the surface falls steeply, foam covers it. The fall is the mesh's CUSTOM0.w, the surface's drop
// per metre over 4 m either side, faded to nothing beyond the waterline and over 8 m at an end where a course meets
// other water. The amount of foam rises in a
// straight line from 0 at foam_from_fall to 1 at foam_full_fall; with the two equal, or the wrong way round, it steps
// from 0 to 1 past foam_from_fall; with both at 0 or less there is none. foam_pattern, foam_size_m metres a copy, is
// carried by the flow with the ripples' two layers, and the water is full foam where the pattern is lighter than
// 1 - amount; below that the foam fades out over foam_soft of grey (one pixel's change at least, never wider than the
// amount nor what is left of it), covering the water by its share. The pattern's greys are spread evenly, so the
// amount is the share of the water in full foam. Two greys that do not match, mixed by weights that sum to 1, lie
// nearer the middle than either, most where the weights are equal: the share of foam would shrink and grow every half
// cycle over the whole brook at once. So the mixed grey is turned back into an even spread, by the share of such mixes
// that lie below it at the cycle's weights, before it is compared, and the share of foam is the amount at every point
// of the cycle. Foam is foam_color, lit by the sun and the sky like the ground and rough to light, so it is dark at
// night, and nothing is seen through it. The ripples' strength grows with the amount, to foam_ripples times under
// full foam. No pattern reads black (hint_default_black), and a size of 0 is read as black, so neither shows foam.
//
// The sky and the sun are mirrored by the engine's own lighting, by reflection (SPECULAR) and roughness. A value of
// 0 where the shader would divide gives the hard result: a bed_gone_m of 0 shows no bed, an edge_fade_m of 0 no fade.
//
// The hills in the water: the view's ray is mirrored at the surface by its rippled normal and followed through the
// picture already drawn, in hills_steps steps of equal length over hills_reach_m. Each step's place is projected into
// the picture and compared with the depth drawn there. The first step at which the ray lies behind what was drawn, by
// no more than one step's length, and what was drawn there lies beyond the water's point along the mirrored ray's way
// over the ground, has found it, and the place between that step and the one before where the two gaps say the ray
// crossed it is what the water mirrors there. A ray that lies behind what was drawn by more than a step is passing
// behind something nearer the camera than itself, and one behind something on the camera's side of the water's point
// is passing behind something in front of the water, a bank or a blade of grass: either goes on, so nothing in front
// of the water is taken. A ray that leaves the picture, comes to the camera's plane or ends without finding anything
// finds nothing, and the sky is mirrored as before; so does one that does not rise from the water. What is found is
// light the surface gives off, like what is seen through the water (times EXPOSURE, which only undoes the engine's
// own scaling of the light given off; it is dark at night because the picture it reads is), as strong as the
// engine's reflection of the sky would be there (its Fresnel from reflection, so stronger the flatter the view meets
// the water) times hills_reflection, faded out toward the screen's edges and over the last hills_fade of the ray's
// reach, and not where foam covers the water. It takes the place of the sky's reflection by that share: the
// engine's reflection is lowered by the same share, not added to. The engine reflects 0.16 * SPECULAR^2 of the light
// at a straight view and fifty times that, at most all, at a flat one, so SPECULAR times the square root of one less
// the share lowers it by the share. It replaces the sky's reflection exactly while reflection is at most about 0.35
// (where fifty times 0.16 * reflection^2 reaches 1); above that the flat view's reflection is held at all and is
// lowered by less, and the two overlap a little. With hills_reflection, hills_steps, hills_reach_m or reflection at 0
// nothing is looked for, the loop is not entered and nothing divides by 0; a hills_fade of 0 gives no fade.
//
// Each point is lifted with its distance from the camera, measured in world space: by lift_per_m metres for each metre
// beyond lift_start_m, and by lift_most_m at most. The lift is for display only; the meshes still stand at their
// ground plus their depth. It is there because Terrain3D draws distant ground from points farther apart than 1 m, so
// beyond some hundred metres a bed flattens and the ground would cover its water. The game's camera, 40 m from its
// focus, sees no water far enough away to be lifted.
//
// Every value of the look is a uniform of the same name as a property of LandformWaterLook, and LandformWater hands
// each over; no uniform here has a default of its own. The two samplers of what the engine drew are not values of the
// look but part of the method, as are the hints of the ripple and foam samplers.

shader_type spatial;
render_mode blend_mix, depth_draw_never, cull_back;

// The line the two sizes drift along, one each way; which line does not matter, since the ripples go nowhere.
const vec2 DRIFT_LINE = vec2(0.6, 0.8);
// The most steps the mirrored ray takes: a loop needs a bound known when the shader is compiled on some drivers, so
// hills_steps is held to it.
const int HILLS_STEPS_MOST = 64;
// What is mirrored fades out over this share of the screen's smaller side at its edges, beyond which nothing is
// known, so a ray that the ripples carry out of the picture and back does not flicker there; part of the method, as
// the engine's own screen-space reflection fixes its screen-edge margin.
const float HILLS_EDGE_FADE = 0.1;

// The picture the engine drew before the water.
uniform sampler2D screen_texture : hint_screen_texture, repeat_disable, filter_linear;
// The depth the engine drew before the water.
uniform sampler2D depth_texture : hint_depth_texture, repeat_disable, filter_nearest;

// What is seen through the water is multiplied by this colour.
uniform vec4 shallow_color : source_color;
// The water's own colour where the bed is no longer seen, lit by the sun and the sky.
uniform vec4 deep_color : source_color;
// The stored depth, in metres, at which the bed is no longer seen.
uniform float bed_gone_m;
// How deep the water is, in metres, where the surface has fully faded in at the bank.
uniform float edge_fade_m;
// How strongly the surface mirrors the sky and the sun (the engine's specular amount).
uniform float reflection : hint_range(0.0, 1.0, 0.01);
// How rough the surface is to light, 0 a mirror.
uniform float roughness : hint_range(0.0, 1.0, 0.01);
// The distance from the camera, in metres, at which the lift begins.
uniform float lift_start_m;
// Metres of lift for each metre of distance from the camera beyond lift_start_m.
uniform float lift_per_m;
// The most the water is lifted, in metres.
uniform float lift_most_m;
// The ripples as a normal map that tiles; red and green are the tilt along world x and z. Without a picture it reads
// as Godot's flat normal.
uniform sampler2D ripple_pattern : hint_normal, repeat_enable, filter_linear_mipmap;
// Metres of water one copy of the pattern covers.
uniform float ripple_size_m;
// Metres of water one copy of the fine copy of the pattern covers.
uniform float ripple_fine_size_m;
// How steep the ripples are to light.
uniform float ripple_strength;
// Metres per second the two sizes drift, in opposite directions.
uniform float ripple_drift_m_s;
// Metres the ripples are carried for each metre the stored flow moves.
uniform float flow_speed;
// Seconds a flow layer is carried before it starts again.
uniform float flow_cycle_s;
// How far a ripple of full tilt shifts what is seen through the water, in metres at the water.
uniform float bend_m;
// The foam as a grey picture that tiles, its greys spread evenly; lighter is where foam comes first. Without a
// picture it reads black, so no foam shows.
uniform sampler2D foam_pattern : hint_default_black, repeat_enable, filter_linear_mipmap;
// The foam's colour, lit by the sun and the sky.
uniform vec4 foam_color : source_color;
// The fall, in metres of drop per metre, at which foam begins.
uniform float foam_from_fall;
// The fall, in metres of drop per metre, at which the foam is full.
uniform float foam_full_fall;
// Metres of water one copy of the foam pattern covers.
uniform float foam_size_m;
// The band of the pattern's grey below 1 - amount over which the foam fades in; 0 gives a hard edge.
uniform float foam_soft;
// How many times stronger the ripples are under full foam.
uniform float foam_ripples;
// How strongly what lies beyond the water is mirrored in it: the share of the sky's reflection it takes the place of
// where it is found. 0 looks for nothing.
uniform float hills_reflection : hint_range(0.0, 1.0, 0.01);
// How many steps the mirrored ray takes, HILLS_STEPS_MOST at most. 0 looks for nothing.
uniform int hills_steps;
// How far the mirrored ray goes, in metres. 0 looks for nothing.
uniform float hills_reach_m;
// The share of hills_reach_m at the ray's end over which what is mirrored fades out, so the end is not seen as a line.
// 0 gives no fade.
uniform float hills_fade : hint_range(0.0, 1.0, 0.01);

// The water's stored depth at this point, in metres.
varying float stored_depth_m;
// The surface's stored fall at this point, its drop per metre.
varying float fall;
// The stored flow at this point, in metres a second along world x and z.
varying vec2 flow;
// Where this point lies, in world x and z.
varying vec2 world_xz;

void vertex() {
	vec3 world = (MODEL_MATRIX * vec4(VERTEX, 1.0)).xyz;
	float away_m = length(world - CAMERA_POSITION_WORLD);
	VERTEX.y += clamp((away_m - lift_start_m) * lift_per_m, 0.0, lift_most_m);
	stored_depth_m = CUSTOM0.z;
	fall = CUSTOM0.w;
	// The mesh's flow, turned as the node is (it has no transform in a scene that shows a world).
	flow = (MODEL_MATRIX * vec4(CUSTOM0.x, 0.0, CUSTOM0.y, 0.0)).xz;
	world_xz = world.xz;
}

// The pattern's tilt at uv: the horizontal part of its normal, along world x and z.
vec2 pattern_tilt(vec2 uv) {
	return texture(ripple_pattern, uv).rg * 2.0 - 1.0;
}

// The tilt of one flow layer whose picture is taken about `at`, `aside` copies of the pattern along both axes: the
// two sizes in equal shares, the first moved by `drift` metres and the fine one by as much the other way. A size of
// 0 is left out, flat.
vec2 layer_tilt(vec2 at, vec2 drift, float aside) {
	vec2 tilt = vec2(0.0);
	if (ripple_size_m > 0.0) {
		tilt += 0.5 * pattern_tilt((at + drift) / ripple_size_m + aside);
	}
	if (ripple_fine_size_m > 0.0) {
		tilt += 0.5 * pattern_tilt((at - drift) / ripple_fine_size_m + aside);
	}
	return tilt;
}

// The foam pattern's grey for one flow layer whose picture is taken about `at`, `aside` copies of the pattern along
// both axes; black with a size of 0. Held to 0..1, so a pattern in a float format that holds more does not reach
// mixed_share_below, which divides by 0 for a grey above 1 at the cycle's ends.
float layer_foam(vec2 at, float aside) {
	if (foam_size_m > 0.0) {
		return clamp(texture(foam_pattern, at / foam_size_m + aside).r, 0.0, 1.0);
	}
	return 0.0;
}

// The share of mixes below `grey`, where a mix is two greys of an even spread that do not match, taken by the weights
// `weight` and 1 - weight: so a mix is turned back into an even spread. Its spread rises in a straight line from 0 to
// the lower weight, stays level up to the higher, and falls to 1; the share is that spread summed up to `grey`. With
// a weight of 0 or 1 it is the grey itself, and for a grey of 0 to 1 (layer_foam holds each layer's to that) it never
// divides by 0: the first and last parts are reached only when the lower weight is above 0, and the higher weight is
// at least 1/2.
float mixed_share_below(float grey, float weight) {
	float lower = min(weight, 1.0 - weight);
	float higher = 1.0 - lower;
	if (grey < lower) {
		return grey * grey / (2.0 * lower * higher);
	}
	if (grey > higher) {
		return 1.0 - (1.0 - grey) * (1.0 - grey) / (2.0 * lower * higher);
	}
	return (grey - 0.5 * lower) / higher;
}

// Follows the mirrored ray from `from` along `along`, a unit vector, both in view space, through the picture drawn
// before the water, in `steps` steps of `step_m` metres, `steps` HILLS_STEPS_MOST at most; `up` is the world's up in
// view space. `gap_from` is how far the start lies behind what was drawn behind it, along the view's z (negative: in
// front). At each step the gap is how far the ray's place lies behind what was drawn where it falls in the picture;
// the first step with a gap above 0 and no more than a step, where what was drawn lies beyond `from` along the ray's
// way over the ground, has found what the ray met, and the place between it and the step before where the straight
// line between their gaps crosses 0 is given: x and y where it falls in the picture, z how far along the ray it lies,
// w 1. A larger gap is a ray passing behind something nearer the camera than itself, and something drawn on the
// camera's side of `from` stands in front of the water; past either the ray goes on. A ray that leaves the picture,
// reaches the camera's plane or ends without that gives w 0.
vec4 follow_mirrored_ray(vec3 from, vec3 along, vec3 up, float gap_from, int steps, float step_m, mat4 projection,
		mat4 inv_projection) {
	// A line in view space is a line in the projection's four coordinates too, so each step's place is found there.
	vec4 clip_from = projection * vec4(from, 1.0);
	vec4 clip_along = projection * vec4(along, 0.0);
	// The ray's way over the ground: what it meets lies beyond `from` along this.
	vec3 onward = along - up * dot(along, up);
	float last_gap = gap_from;
	float last_m = 0.0;
	for (int i = 1; i <= HILLS_STEPS_MOST; i++) {
		if (i > steps) {
			break;
		}
		float at_m = step_m * float(i);
		vec4 clip = clip_from + clip_along * at_m;
		if (clip.w <= 0.0) {
			break;
		}
		vec2 uv = clip.xy / clip.w * 0.5 + 0.5;
		if (any(lessThan(uv, vec2(0.0))) || any(greaterThan(uv, vec2(1.0)))) {
			break;
		}
		vec4 drawn = inv_projection * vec4(uv * 2.0 - 1.0, textureLod(depth_texture, uv, 0.0).r, 1.0);
		float gap = drawn.z / drawn.w - (from.z + along.z * at_m);
		if (gap > 0.0 && gap <= step_m && dot(drawn.xyz / drawn.w - from, onward) > 0.0) {
			// After a step in front, the share of this step at which the gap crossed 0; after one behind something
			// nearer, this step itself.
			float share = last_gap < 0.0 ? -last_gap / (gap - last_gap) : 1.0;
			float found_m = mix(last_m, at_m, share);
			vec4 found = clip_from + clip_along * found_m;
			return vec4(found.xy / found.w * 0.5 + 0.5, found_m, 1.0);
		}
		last_gap = gap;
		last_m = at_m;
	}
	return vec4(0.0);
}

void fragment() {
	// What was drawn behind this point, in view space, found as the engine's own refraction finds it.
	float behind_depth = textureLod(depth_texture, SCREEN_UV, 0.0).r;
	vec4 behind = INV_PROJECTION_MATRIX * vec4(SCREEN_UV * 2.0 - 1.0, behind_depth, 1.0);
	behind.xyz /= behind.w;
	vec3 up = (VIEW_MATRIX * vec4(0.0, 1.0, 0.0, 0.0)).xyz;
	// The surface's own face here: the mesh's normals are all up, for the light, so they cannot say how a falling
	// brook slopes; the face's normal is the cross product of how this point's place changes from one pixel to the
	// next across and down the screen, constant over a triangle. Its sign goes with the screen's handedness (on
	// Forward+ under Direct3D 12, where it was probed, it points away from the camera), so it is turned up: the mesh is
	// a height field, so the top of every face has an upward part, and cull_back draws only tops. Turned toward the
	// camera's own point instead, it would be right only under a perspective camera; under the game's orthographic one
	// a top face whose plane passes beyond the camera's point would be turned down and not drawn.
	// A product that is 0 or not finite (a face seen edge-on, or a change across a pixel that blew up at a
	// silhouette) never reaches the normalisation: it is scaled by its largest part first, and without a usable one
	// the face is taken as level, as the water is everywhere but where it falls.
	vec3 face_across = cross(dFdx(VERTEX), dFdy(VERTEX));
	float face_largest = max(max(abs(face_across.x), abs(face_across.y)), abs(face_across.z));
	vec3 face = up;
	if (!any(isnan(face_across)) && !any(isinf(face_across)) && face_largest > 0.0) {
		face = normalize(face_across / face_largest);
		face = dot(face, up) < 0.0 ? -face : face;
	}
	// How far what was drawn behind this point lies behind the surface's own plane: the view runs from the surface to
	// it along one line, so this is the distance along the view times the share of the view's direction across the
	// face. Measured upright instead, water that rises away from the viewer would measure less than it holds, and at or
	// above the eye's height nothing at all, so it would not be drawn. Across the face it is 0 or more wherever a top
	// is drawn, under a perspective camera and an orthographic one alike: the view reaches a top from above its plane,
	// so what lies behind along the view lies below it. On level water the face is level and this is the height of the
	// surface above what lies behind it, so a low camera does not widen the fade.
	float drawn_depth_m = dot(VERTEX - behind.xyz, face);
	float edge = edge_fade_m > 0.0 ? smoothstep(0.0, 1.0, drawn_depth_m / edge_fade_m) : 1.0;

	// The ripples: two layers carried by the flow, half a cycle apart, each the two sizes drifting apart. With a cycle
	// of 0 the first layer alone is shown, not carried; a cycle below 0 is read as 0, in the phase and in the shift.
	float cycle_s = max(flow_cycle_s, 0.0);
	float phase_a = cycle_s > 0.0 ? fract(TIME / cycle_s) : 0.5;
	float phase_b = fract(phase_a + 0.5);
	// The pattern moves with the flow: a layer's picture is taken from farther upstream as it ages.
	vec2 at_a = world_xz - flow * flow_speed * cycle_s * phase_a;
	vec2 at_b = world_xz - flow * flow_speed * cycle_s * phase_b;
	float weight_a = 1.0 - abs(2.0 * phase_a - 1.0);
	float weight_b = 1.0 - weight_a;
	vec2 drift = DRIFT_LINE * ripple_drift_m_s * TIME;
	// The second layer is sampled half a pattern aside, so the two layers do not show one picture. Two pictures that
	// do not match, mixed by weights that sum to 1, are as strong together as one only at the ends of the cycle; at
	// equal weights their tilts partly cancel, to 1/sqrt(2) of one, and the whole surface would throb every half cycle.
	// Dividing by the length of the two weights keeps the ripples' strength even through the cycle. It is never less
	// than 1/sqrt(2), so it never divides by 0.
	vec2 tilt = (weight_a * layer_tilt(at_a, drift, 0.0) + weight_b * layer_tilt(at_b, drift, 0.5))
		/ sqrt(weight_a * weight_a + weight_b * weight_b);

	// How much foam the fall gives: a straight line from foam_from_fall to foam_full_fall, or a step at foam_from_fall
	// when the two are equal or the wrong way round; either way none at foam_from_fall itself. With both falls at 0 or
	// less there is none at all, so a look that leaves the foam empty keeps its ripples.
	float amount = 0.0;
	if (foam_from_fall > 0.0 || foam_full_fall > 0.0) {
		amount = foam_full_fall > foam_from_fall
			? clamp((fall - foam_from_fall) / (foam_full_fall - foam_from_fall), 0.0, 1.0)
			: float(fall > foam_from_fall);
	}
	// The foam pattern carried by the same two layers, the second half a pattern aside, its mix turned back into an
	// even spread, so the share of foam is the amount through the whole cycle. Where it lies above 1 - amount the foam
	// is full; below, it fades out over foam_soft of the spread, and over one pixel's change at least, so its edge does
	// not shimmer. The fade is never wider than the amount nor than what is left of it, so an amount of 0 shows no
	// foam, a little foam fades in with the amount, and a black pattern (none, or a size of 0) never foams. With no
	// fade at all, foam where it lies above.
	float share = mixed_share_below(weight_a * layer_foam(at_a, 0.0) + weight_b * layer_foam(at_b, 0.5), weight_a);
	float over = share - (1.0 - amount);
	float fade = min(max(foam_soft, fwidth(share)), min(amount, 1.0 - amount));
	float foam = fade > 0.0 ? clamp(over / fade + 1.0, 0.0, 1.0) : float(over > 0.0);

	// The ripples grow stronger with the amount of foam, to foam_ripples times under full foam.
	float strength = ripple_strength * mix(1.0, foam_ripples, amount);
	vec3 ripple_normal = normalize(vec3(tilt.x * strength, 1.0, tilt.y * strength));
	NORMAL = normalize((VIEW_MATRIX * vec4(ripple_normal, 0.0)).xyz);

	// What is seen through the water, shifted by the tilt times bend_m metres at the water, projected into the picture.
	vec3 shift = (VIEW_MATRIX * vec4(tilt.x * bend_m, 0.0, tilt.y * bend_m, 0.0)).xyz;
	vec4 here = PROJECTION_MATRIX * vec4(VERTEX, 1.0);
	vec4 there = PROJECTION_MATRIX * vec4(VERTEX + shift, 1.0);
	vec2 bent_uv = SCREEN_UV + (there.xy / there.w - here.xy / here.w) * 0.5;
	// The bent sample is taken only inside the picture and where what was drawn there lies behind the surface's own
	// plane, as the edge measures it: so a bank or a blade nearer the camera, also one lower than this point on water
	// that rises away, is never smeared into the water.
	vec2 seen_uv = SCREEN_UV;
	if (all(greaterThanEqual(bent_uv, vec2(0.0))) && all(lessThanEqual(bent_uv, vec2(1.0)))) {
		float bent_depth = textureLod(depth_texture, bent_uv, 0.0).r;
		vec4 bent_behind = INV_PROJECTION_MATRIX * vec4(bent_uv * 2.0 - 1.0, bent_depth, 1.0);
		bent_behind.xyz /= bent_behind.w;
		if (dot(VERTEX - bent_behind.xyz, face) > 0.0) {
			seen_uv = bent_uv;
		}
	}

	float clear = bed_gone_m > 0.0 ? clamp(1.0 - stored_depth_m / bed_gone_m, 0.0, 1.0) : 0.0;
	vec3 seen_through = textureLod(screen_texture, seen_uv, 0.0).rgb * shallow_color.rgb;

	// The hills in the water: what the mirrored ray finds in the picture drawn before the water, and the share of the
	// sky's reflection it takes the place of there. Nothing is looked for where it could not show: no strength, no
	// steps, no reach, no reflection, full foam, or a mirrored ray that does not rise from the water.
	vec3 mirrored = vec3(0.0);
	float mirrored_share = 0.0;
	vec3 mirror_dir = reflect(-VIEW, NORMAL);
	int steps = clamp(hills_steps, 0, HILLS_STEPS_MOST);
	if (hills_reflection > 0.0 && steps > 0 && hills_reach_m > 0.0 && reflection > 0.0 && foam < 1.0
			&& dot(mirror_dir, up) > 0.0) {
		vec4 found = follow_mirrored_ray(VERTEX, mirror_dir, up, behind.z - VERTEX.z, steps,
			hills_reach_m / float(steps), PROJECTION_MATRIX, INV_PROJECTION_MATRIX);
		if (found.w > 0.0) {
			mirrored = textureLod(screen_texture, found.xy, 0.0).rgb;
			// Faded out toward the screen's edges, in pixels from the nearest, and over the ray's last hills_fade of its
			// reach; a smooth step between equal ends is not defined, so a hills_fade of 0 is no fade.
			vec2 to_edge = min(found.xy, 1.0 - found.xy) * VIEWPORT_SIZE;
			float edge_fade = smoothstep(0.0, HILLS_EDGE_FADE * min(VIEWPORT_SIZE.x, VIEWPORT_SIZE.y),
				min(to_edge.x, to_edge.y));
			float end_fade = hills_fade > 0.0
				? 1.0 - smoothstep(1.0 - min(hills_fade, 1.0), 1.0, found.z / hills_reach_m)
				: 1.0;
			mirrored_share = min(hills_reflection, 1.0) * edge_fade * end_fade * (1.0 - foam);
		}
	}
	// As strong as the engine's own reflection would be there: its Fresnel for a surface of this reflection, by the
	// rippled normal, 0.16 * reflection^2 at a straight view and fifty times that, at most 1, at a flat one.
	float straight_view = 0.16 * reflection * reflection;
	float flat_view = clamp(50.0 * straight_view, 0.0, 1.0);
	float mirrored_strength = straight_view
		+ (flat_view - straight_view) * pow(1.0 - clamp(dot(NORMAL, VIEW), 0.0, 1.0), 5.0);

	// Foam is foam_color, lit like the ground and rough to light; nothing is seen through it. Faded foam covers the
	// water by its share: colour, what is seen through and roughness all go by it. What is mirrored is light given
	// off, as what is seen through is, and the engine's reflection is lowered by the share it takes the place of: it
	// replaces the sky's reflection exactly while reflection is at most about 0.35, and above that the two overlap a
	// little, since the flat view's reflectance is held at 1 and comes down by less.
	ALBEDO = mix(deep_color.rgb * (1.0 - clear), foam_color.rgb, foam);
	EMISSION = (seen_through * clear * (1.0 - foam) + mirrored * mirrored_strength * mirrored_share) * EXPOSURE;
	SPECULAR = reflection * sqrt(1.0 - mirrored_share);
	ROUGHNESS = mix(roughness, 1.0, foam);
	ALPHA = edge;
}
