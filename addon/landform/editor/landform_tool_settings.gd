@tool
class_name LandformToolSettings
extends Resource
## What a person turns in Landform's editor tool, never written in the scripts that use it: the brushes, how near a
## point or line is picked, what a new river and a new lake start as, how much memory the undo history keeps, the
## sizes a new land is offered in, the key that stops a regrowth and how the overlay is drawn. The defaults are the
## shipped values; the tool reads `editor/tool_settings.tres`, which holds them.

## The radius in metres and the strength (how far a stroke moves a pixel at its path's middle toward white or black, 0
## to 1, a pixel nearer the radius by less; LandformBrush) the brush that paints high ground starts with in a sitting.
@export var high_ground_radius_m: float = 64.0
@export_range(0.0, 1.0) var high_ground_strength: float = 0.25
## The radius in metres the brushes that paint held ground and the area to be regrown as drawn start with; a cell is
## held, or in the area, or not, so they have no strength.
@export var hold_radius_m: float = 30.0
@export var area_radius_m: float = 50.0
## The least radius in metres of every brush; the greatest is half the land's side.
@export var brush_radius_least_m: float = 1.0
## How much one press of the keys below changes the chosen brush's radius: it is multiplied by this, or divided.
@export var brush_radius_step: float = 1.25
## The keys in the 3D view that make the chosen brush's radius smaller and larger (the panel types it too).
@export var brush_smaller_key: Key = KEY_BRACKETLEFT
@export var brush_larger_key: Key = KEY_BRACKETRIGHT
## The modifier which, held when a brush's stroke begins, turns the stroke over: High ground paints darker, Hold lifts
## the hold, and Regrow an area as drawn clears the area.
@export var brush_invert_modifier: KeyModifierMask = KEY_MASK_CTRL
## How near the pointer, in pixels of the screen, a river's point, its line or a lake must lie to be picked.
@export_range(1.0, 64.0) var pick_pixels: float = 12.0
## The width in metres a new river is drawn with.
@export var new_river_width_m: float = 8.0
## The least radius in metres a new lake is drawn with, however short the drag that sizes it.
@export var new_lake_least_radius_m: float = 10.0
## How many megabytes of undo steps the history keeps of a land; past it the oldest are let go (the stage 2 plan's
## decision 4).
@export var undo_budget_mb: int = 1024
## The sizes in metres a new land is offered in, and the one offered first.
@export var new_land_sizes_m: PackedInt32Array = PackedInt32Array([512, 1024, 2048, 4096])
@export var new_land_size_m: int = 2048
## The kind of country and the seed a new land is offered with.
@export var new_land_country: String = "hills"
@export var new_land_seed: int = 11
## The side in pixels of a new land's picture of high ground, black all over, which grows as no picture does.
@export var new_land_picture_px: int = 256
## Whether "Regrow on release" is on when a land is first edited in a sitting.
@export var regrow_on_release: bool = false
## The key that stops a running regrowth, in the 3D view, the panel and the bar.
@export var stop_shortcut: Shortcut
## How the overlay and the lines are drawn (the outline's rim red, the band's inner edge amber, held ground cyan, the
## area magenta, as stage 1's pictures drew them): the shipped values are `editor/overlay_look.tres`, which
## `tool_settings.tres` holds. The session hands it to each land's view it edits
## (`LandformLandView.set_overlay_colours`), which keeps it in a plain variable, so no scene stores it; empty, the view
## draws with LandformOverlayLook's defaults.
@export var overlay_look: LandformOverlayLook
