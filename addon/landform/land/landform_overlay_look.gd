@tool
class_name LandformOverlayLook
extends Resource
## How the editor tool draws over a land, every colour and size of it, never written in the scripts that draw: the
## overlay laid over the ground (LandformOverlay.compose: the outline's rim, the band's inner edge, held ground, ground
## shaped by hand and the area to be regrown, at a pixel a metre) and the lines over it (LandformLandLines: each
## river's line and its points, each lake's rim and its centre). The defaults are the shipped values: the outline red,
## the band's inner edge amber, held ground cyan and the area magenta, as stage 1's pictures drew them. A colour's alpha
## is how much it covers what lies under it. The tool's settings hold the look a person turns; LandformLandView draws
## with the one it is given (set_overlay_colours).

## The outline's rim: its cells within edge_cells of one the regrowth leaves as it is.
@export var rim_color := Color(0.95, 0.1, 0.1)
## The band's inner edge: its fully regrown cells within edge_cells of one only partly regrown.
@export var band_edge_color := Color(1.0, 0.7, 0.0)
## Held ground, which no edit changes.
@export var held_color := Color(0.0, 0.85, 0.95, 0.4)
## Ground shaped by hand.
@export var hand_color := Color(0.3, 0.9, 0.2, 0.4)
## The area to be regrown as drawn.
@export var area_color := Color(0.95, 0.0, 0.95, 0.4)
## How wide the rim and the band's inner edge are drawn, in cells of 1 m.
@export_range(1, 64) var edge_cells: int = 3
## A river's line, and a lake's rim.
@export var river_color := Color(0.25, 0.6, 1.0)
@export var lake_color := Color(0.25, 0.6, 1.0)
## A river's points and a lake's centre.
@export var point_color := Color(1.0, 1.0, 1.0)
## The selected river, point or lake.
@export var selected_color := Color(1.0, 0.85, 0.1)
## How wide a line is drawn and how large a point's marker, in pixels of the screen, whatever the distance.
@export_range(1.0, 16.0) var line_pixels: float = 2.0
@export_range(2.0, 32.0) var marker_pixels: float = 9.0
