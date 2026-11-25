@tool
class_name LandformWaterLook
extends Resource
## How the water looks: every colour, number and pattern of the water's look, never written in the shader or in the
## scripts that show the water. LandformWater hands each script variable of this resource to the material under its
## own name, and water.gdshader has a uniform of that name for each, so a value is added here and in the shader and
## nowhere else. The script gives none of them a value of its own, so a new look is neutral (black, no bed seen, a
## hard edge, no reflection, no lift, flat and still, no foam, nothing mirrored) and still shows the water;
## water_look.tres holds the shipped values, all of them written out. Godot leaves out of a saved resource every value
## equal to its script's default, so a value tuned to 0, to black or to no pattern drops out of water_look.tres when
## the editor saves it, and the test that the shipped look writes every value out fails: write that line back into the
## file by hand.
##
## The colour goes by the water's stored depth: at a depth of 0 the water is clear and what lies behind it is seen,
## tinted by shallow_color; by bed_gone_m the bed is no longer seen and the surface shows deep_color, lit by the sun
## and the sky. The edge goes by how deep the water drawn is, measured across the surface, so a falling brook fades in
## at its banks as level water does: the surface fades in over edge_fade_m of water.
##
## The lift raises the drawn water with its distance from the camera, for display only: the meshes still stand at
## their ground plus their depth. It is there because Terrain3D draws distant ground from points farther apart than
## 1 m, so beyond some hundred metres a bed flattens and the ground covers its water. The game's camera, 40 m from its
## focus, sees no water far enough away to be lifted.
##
## The ripples are ripple_pattern laid over the world by world x and z at two sizes, ripple_size_m and
## ripple_fine_size_m, which drift at ripple_drift_m_s in opposite directions, so on a lake they move and go nowhere.
## On a river the water's stored flow carries both sizes downstream, flow_speed metres for each metre the flow moves,
## by two layers that start again every flow_cycle_s seconds, half a cycle apart, each faded out as it starts again.
## The ripples lean the surface to the light by ripple_strength and bend what is seen through the water by bend_m.
## No pattern leaves the water flat; a size of 0 leaves that size out, so both sizes at 0 leave it flat; a cycle of 0,
## or less, carries nothing.
##
## White water: where the surface falls steeply (the fall each point of the mesh stores, its drop per metre, faded to
## nothing beyond the waterline and over 8 m at an end where a course meets other water), foam covers it. Its amount rises in a straight line from 0 at a fall of
## foam_from_fall to 1 at foam_full_fall, and with the two equal, or the wrong way round, it steps from 0 to 1 past
## foam_from_fall; with both at 0 or less there is no foam. foam_pattern, foam_size_m metres a copy, is carried by the
## flow with the ripples' two layers, and the water is full foam where the pattern is lighter than 1 - amount, fading
## out below that over foam_soft of grey, a band the shader keeps to at least one pixel's change and to no more than
## the amount nor than one less the amount: foam_color, not see-through, rough to light and lit by the sun and the sky
## like the ground, so it is dark at night; faded foam covers the water by its share. Under foam the ripples are
## stronger, up to foam_ripples times under full foam. No pattern, or a size of 0, shows no foam.
##
## The hills in the water: the view's ray is mirrored at the surface by the rippled normal and followed through the
## picture already drawn, in hills_steps steps over hills_reach_m, each step's place compared with the depth drawn
## there. Where it first passes behind what was drawn, by no more than one step and beyond the water's point, that
## place's colour is mirrored in place of the sky, hills_reflection of the sky's reflection there, as strong as the
## sky's would be (stronger the flatter the view meets the water), fading out toward the screen's edges and over the
## last hills_fade of the reach; it is light the surface gives off, like what is seen through, and dark at night
## because the picture it reads is dark then. Something drawn in front of the water, nearer the camera than the ray
## by more than a step or on the camera's side of the water's point, is passed over. Where nothing is found the sky is
## mirrored as before; with hills_reflection, hills_steps or hills_reach_m at 0 nothing is looked for, and with
## hills_fade at 0 nothing fades at the ray's end.

## What is seen through the water is multiplied by this colour; white leaves it as it is. It has no alpha: the
## shader reads its red, green and blue.
@export_color_no_alpha var shallow_color: Color
## The water's own colour where the bed is no longer seen, lit by the sun and the sky. It has no alpha.
@export_color_no_alpha var deep_color: Color
## The stored depth, in metres, at which the bed is no longer seen. 0 shows no bed at all.
@export_range(0.0, 10.0, 0.01, "or_greater") var bed_gone_m: float
## How deep the water is, in metres across its surface, where the surface has fully faded in at the bank. 0 gives a
## hard edge.
@export_range(0.0, 1.0, 0.001, "or_greater") var edge_fade_m: float
## How strongly the surface mirrors the sky and the sun (the engine's specular amount; 0.5 is the engine's usual).
@export_range(0.0, 1.0) var reflection: float
## How rough the surface is to light, 0 a mirror.
@export_range(0.0, 1.0) var roughness: float
## The distance from the camera, in metres, at which the lift begins.
@export_range(0.0, 1000.0, 1.0, "or_greater") var lift_start_m: float
## Metres of lift for each metre of distance from the camera beyond lift_start_m.
@export_range(0.0, 0.1, 0.0001, "or_greater") var lift_per_m: float
## The most the water is lifted, in metres; LandformWater grows every piece's box by as much, so no piece is culled
## by its unlifted box.
@export_range(0.0, 20.0, 0.1, "or_greater") var lift_most_m: float
## The ripples as a normal map that tiles: red and green are the tilt along world x and z, blue the upright part, each
## stored as (n + 1) / 2. make_water_patterns.gd writes the shipped one, ripples.png. None leaves the water flat.
@export var ripple_pattern: Texture2D
## Metres of water one copy of the pattern covers. 0 leaves this size out.
@export_range(0.0, 50.0, 0.01, "or_greater") var ripple_size_m: float
## Metres of water one copy of the fine copy of the pattern covers. 0 leaves this size out.
@export_range(0.0, 10.0, 0.01, "or_greater") var ripple_fine_size_m: float
## How steep the ripples are to light: the surface leans by the ripples' tilt times this. 0 lights the water flat.
@export_range(0.0, 2.0, 0.01, "or_greater") var ripple_strength: float
## Metres per second the two sizes drift, in opposite directions, on any water; on a lake that is all that moves.
@export_range(0.0, 1.0, 0.001, "or_greater") var ripple_drift_m_s: float
## Metres the ripples are carried for each metre the stored flow moves. 0 carries nothing.
@export_range(0.0, 4.0, 0.01, "or_greater") var flow_speed: float
## Seconds a flow layer is carried before it starts again. 0, or less, carries nothing. The shader's clock starts
## again every 3,600 s, so a cycle that divides 3,600 is not seen to jump then.
@export_range(0.0, 20.0, 0.01, "or_greater") var flow_cycle_s: float
## How far a ripple of full tilt shifts what is seen through the water, in metres at the water. 0 bends nothing.
@export_range(0.0, 1.0, 0.001, "or_greater") var bend_m: float
## The foam as a grey picture that tiles, lighter where foam comes first. make_water_patterns.gd writes the shipped
## one, foam.png, whose greys are spread evenly, so the amount of foam is the share of the water it covers. None
## shows no foam.
@export var foam_pattern: Texture2D
## The foam's colour. It has no alpha.
@export_color_no_alpha var foam_color: Color
## The fall, in metres of drop per metre, at which foam begins. With this and foam_full_fall both at 0 there is no
## foam.
@export_range(0.0, 1.0, 0.001, "or_greater") var foam_from_fall: float
## The fall, in metres of drop per metre, at which the foam is full. At or below foam_from_fall the foam steps from
## none to full past foam_from_fall.
@export_range(0.0, 1.0, 0.001, "or_greater") var foam_full_fall: float
## Metres of water one copy of the foam pattern covers. 0 shows no foam.
@export_range(0.0, 10.0, 0.01, "or_greater") var foam_size_m: float
## The band of the pattern's grey, below where the foam is full, over which it fades in: the soft edge of each clump of
## foam. The shader holds the band to two bounds. It is never under one pixel's change of the grey, so 0 gives the
## hardest edge that does not shimmer, not a cut. It is never wider than the amount of foam nor than one less the
## amount, so where the amount is under this value or over one less it (above about 0.8 at the shipped 0.2) turning
## this up does little.
@export_range(0.0, 1.0, 0.01) var foam_soft: float
## How many times stronger the ripples are under full foam; under less foam, in proportion. 1 leaves them as they
## are, 0 makes the water under full foam flat.
@export_range(0.0, 10.0, 0.01, "or_greater") var foam_ripples: float
## How strongly what lies beyond the water is mirrored in it: the share of the sky's reflection it takes the place of
## where the mirrored ray finds it, 1 wholly. 0 looks for nothing and costs nothing.
@export_range(0.0, 1.0, 0.01) var hills_reflection: float
## How many steps the mirrored ray takes, each hills_reach_m / hills_steps long; the shader takes 64 at most
## (HILLS_STEPS_MOST, since a loop needs a bound known when it is compiled; a test holds this range's upper end to it).
## More steps miss less and cost more. 0 looks for nothing.
@export_range(0, 64) var hills_steps: int
## How far the mirrored ray goes, in metres; what it finds fades out over the last hills_fade of this. 0 looks for
## nothing.
@export_range(0.0, 1000.0, 1.0, "or_greater") var hills_reach_m: float
## The share of the reach over which the mirror fades out toward the ray's end, so the end is not seen as a line. 0 is
## no fade: what is found shows at full strength to the end of the reach.
@export_range(0.0, 1.0, 0.01) var hills_fade: float
