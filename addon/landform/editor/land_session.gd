@tool
class_name LandformLandSession
extends RefCounted
## The editor tool's rules for one land's view, kept here so the game's tests run them headless (the plugin only
## connects them to the editor): the four states and what moves between them, the drawing being edited, the selection,
## "Regrow on release", and what the panel and the bar say in each state, as text the tests read.
##
## The states (the design's **Four states**): NO_LAND, the view shows no land, and the panel says what a land is and
## offers the form that makes one; GROWING, a job of the land runs (a new land, a regrowth, a regrowth from another
## seed or of an area), and the bar says what grows, for how long and how to stop it; FAILED, the job ended with the
## engine's sentence, said with that the land is as it was; LAND, a land shown, and the bar says what regrowing would
## change. A job started moves to GROWING; its step swapped in, its stop, and a new land made and written move to LAND;
## any other end moves to FAILED, left by `dismiss` or by the next change.
##
## The session holds the land in a plain variable, as the view does, never in a stored property or an editor's state,
## and saves it only to the file it was read from or written to (LandformLandView.get_land_path): when the view's
## land_file comes to name another file, the view opens it once the land shown is saved, and until then nothing is
## opened or written there and the bar says why.
## It changes the land only through the land's own calls: `ask` when a change of the drawing is released (a typed
## number is one, as a drag's release is), `hold`, `set_reach_m`, the jobs, and `swap` of a job's step, which the land
## then shows (`LandformLandView.refresh`). What the land delivers is handled after its `poll` returns, never inside its
## signals.
##
## The undo history (the stage 2 plan's Task 13): each thing a person does is one action (LandformLandAction) of
## Godot's history, made through one seam (LandformLandHistory: the editor's EditorUndoRedoManager in the plugin, a
## plain UndoRedo in the tests): a gesture's release, its ask with the regrowth "Regrow on release" starts; a Regrow,
## a regrowth from a seed or of an area; a Hold stroke; the reach, the kind of country, a number typed. An action holds
## the land's steps; its regrowth's step joins it when the regrowth ends while the action is done, and a regrowth whose
## action was undone is stopped first and never swapped in. An undo or a redo stops a running regrowth first and waits
## for it (`begin_history`), then swaps, and the session shows the land as it now is, its asked drawing in the lines
## (`end_history`). After each action and each step joined, the steps are kept within the settings' budget
## (LandformLand.keep_steps_within, the oldest actions let go whole), and the bar says when steps were let go and how
## far undo reaches; an action let go, of another land, or from before a reload of the extension changes nothing and
## says why. A reload (the stage 2 plan's decision 6) is found by the land's world being another object than the one
## the session last saw: the land is as it was, its steps were let go, and the bar says so.
##
## Whole-land edits never run on release (the stage 2 plan's decision 7): a change of the kind of country or of its
## numbers waits for Regrow, said by the outline's reason; a seed changed is no waiting edit, the bar asks at once and
## `keep_seed` takes it back.
##
## The tools (`set_tool`, `input`): Select, River and Lake (LandformSelectTool, LandformRiverTool, LandformLakeTool,
## each picking through one LandformPicking of the view) and the brushes High ground, Hold and Regrow an area as drawn
## (LandformHighGroundTool, LandformHoldTool, LandformAreaTool, each painting with a LandformBrush at the radius the
## session keeps for it, `get_brush`) take the 3D view's events in the LAND state only, so while a regrowth runs the
## tools are off and only the stop shortcut is taken (the stage 2 plan's decision 8). A tool's gesture hands each
## drawing it makes to `set_working`, during which the outline last worked out stays shown until the next arrives, and
## a gesture finished is one step (`release_change`); the stop shortcut with nothing growing ends the gesture and drops
## its change (`drop_change`), and choosing another tool does too. A change released or dropped by other means (Regrow,
## a number typed, a job's step) ends the tool's gesture with the drawing as it then is. A Hold stroke is the land's
## hold at its release (`hold_stroke`), no change of the drawing; an area painted (`set_area`) is what the bar says and
## Regrow regrows while it is painted, and is cleared once its regrowth is swapped in.

## The view's land file could not be shown, a land was made and written, a job ended, the drawing, the selection or
## what the panel and the bar say changed: they show the session again.
signal changed()
## A job runs and its time went on: the growing sentence changed.
signal ticked()
## A new land was written to `path`, which the view's `land_file` now names, so the scene holding the view changed.
signal land_file_set(path: String)

## What the session is in.
enum State { NO_LAND, GROWING, FAILED, LAND }

## The tools, by the names `set_tool` takes; the toolbar shows one button each.
const TOOLS: PackedStringArray = ["select", "high_ground", "river", "lake", "hold", "area"]
## The tools that are brushes, by their names in TOOLS, and the one of them that has a strength.
const BRUSHES: PackedStringArray = ["high_ground", "hold", "area"]
const STRENGTH_BRUSH := "high_ground"
## The kinds of a selected item and of the grown drawing's partners (`LandformLand.get_grown_partner`), and the keys a
## selection holds (as `LandformLandLines.SELECTED_*` name them).
const RIVER := "river"
const LAKE := "lake"
const POINT := "point"
## The fields the panel types, by the names `type_field` takes.
const EAST := "east"
const SOUTH := "south"
const WIDTH := "width"
const RADIUS := "radius"
## The kinds of a job, which say what grows.
const JOB_NEW := "new"
const JOB_REGROW := "regrow"
const JOB_SEED := "seed"
const JOB_AREA := "area"
## The subject of a note about the land as a whole (`LandformWorld.get_note_subjects`).
const ABOUT_LAND := "land"
## Square metres in a hectare, and milliseconds in a second.
const M2_PER_HA := 10_000.0
const MS_PER_S := 1000.0
## An area is said in whole hectares from this many on, and with one decimal below it.
const HA_WHOLE_FROM := 10.0
## The least area said as a number, in hectares; a smaller area above 0 is said as under it.
const HA_LEAST_SAID := 0.1
## The greatest seed the panel takes.
const SEED_MOST := 2_147_483_647
## The extension a land file's name ends in.
const LAND_FILE_EXTENSION := "lfland"
## A share said in percent.
const PERCENT := 100.0
## How many numbers a river's `points_m` holds a point (east, then south), and how many points a river keeps at least.
const NUMBERS_PER_POINT := 2
const RIVER_POINTS_LEAST := 2
## The decimals a typed field of the panel shows a number with (field_text): a centimetre.
const FIELD_DECIMALS := 2
## The key that removes the selected item, under every tool (delete_selected).
const DELETE_KEY := KEY_DELETE
## The actions of the undo history the session makes, by what a person did (the tools name their own gestures' actions),
## as the history and the bar's Undo name them: a change of the drawing released, a regrowth, one from a seed, an area
## regrown as drawn, a Hold stroke, its lift, the reach set, the kind of country chosen, a number typed (by its field),
## a point, a river or a lake removed.
const ACTION_CHANGE := "change"
const ACTION_REGROW := "regrow"
const ACTION_SEED := "regrow from seed"
const ACTION_AREA := "regrow area"
const ACTION_HOLD := "hold"
const ACTION_LIFT := "lift"
const ACTION_REACH := "set reach"
const ACTION_COUNTRY := "set country"
const ACTION_TYPED := "set %s"
const ACTION_REMOVE_POINT := "remove point"
const ACTION_REMOVE_RIVER := "remove river"
const ACTION_REMOVE_LAKE := "remove lake"
## Bytes in a megabyte of the undo budget (LandformToolSettings.undo_budget_mb), 2^20.
const BYTES_PER_MB := 1_048_576

## What a land is, said in the NO_LAND state before the form that makes one.
const WHAT_A_LAND_IS: PackedStringArray = [
	"Creating a land saves the terrain and its empty drawing to the chosen file.",
]
## What regrowing would change, in the LAND state.
const WORKING_OUT := "Calculating the outline."
const CHANGES_NOTHING := "Regrowing changes no ground."
const CHANGES_INSIDE := "Regrowth outline covers %s ha."
## The same of an outline under HA_LEAST_SAID: "under 0.1 ha", without "the" (stage 2's ruling R18, S2).
const CHANGES_UNDER := "Regrowth outline covers under %s ha."
const NONE_HELD := "No held ground inside."
## The held ground inside the outline; one under HA_LEAST_SAID is said "Under 0.1 ha of it is held.", the sentence's
## first letter a capital (stage 2's ruling R18, S2).
const SOME_HELD := "%s ha held inside."
const WHOLE_LAND := "This regrows the whole land except held and hand-shaped ground: %s."
const SEED_CHANGED := "the seed changed"
## What grows, in the GROWING state, with the time it has run, and how to stop it.
const GROWING_INSIDE := "Regrowing the %s ha inside the outline: %s s."
const GROWING_UNDER := "Regrowing under %s ha inside the outline: %s s."
const GROWING_WHOLE := "Regrowing the whole land except held and hand-shaped ground: %s s."
const GROWING_LAND := "Regrowing the land toward its drawing: %s s."
const GROWING_SEED := "Regrowing the whole land from seed %d: %s s."
const GROWING_AREA := "Regrowing the area as drawn: %s s."
const GROWING_NEW := "Growing a new land of %s m from seed %d: %s s."
const STOPS_IT := "%s stops it."
const STOP_BUTTON_STOPS_IT := "The bar's Stop button stops it."
## What follows a job's end.
const AS_IT_WAS := "The land is as it was."
## What a job stopped ends with, in the extension's words for the stop (a step that arrives after `stop` is a stop too).
const REGROWTH_STOPPED := "The regrowth was stopped."
const NEW_LAND_STOPPED := "Growing the new land was stopped."
const NO_LAND_MADE := "No land was made."
## What the extension says of a job a reload of it stopped (LandformLand's job_failed), a stop too.
const RELOAD_STOPPED := "Landform was reloaded, so the regrowth was stopped."
const RELOAD_STOPPED_NEW := "Landform was reloaded, so growing the new land was stopped."
## What the bar says once Landform was reloaded (the stage 2 plan's decision 6).
const RELOADED := "Landform was reloaded: the land is as it was, and its undo steps were let go."
## What the bar says after a regrowth of the ground it changed: the step's own area (stage 2's ruling R21, S3).
const STEP_INSIDE := "Regrowth outline covered %s ha."
## What is said when a river being drawn is dropped by an undo or a reload (stage 2's ruling R21, S1 and N4).
const RIVER_DROPPED := "The river being drawn was dropped."
## What the bar adds while an area is painted and a change of the drawing waits behind it (stage 2's ruling R21, S2).
const CHANGE_WAITS_AFTER_AREA := "A change of the drawing waits to regrow after the area."
## What is said when the view's land file is cleared while it shows a land, which keeps its file (stage 2's ruling
## R21, N5).
const LAND_FILE_KEPT := "The view's land file cannot be cleared while it shows a land, so it names %s again."
## A regrowth's step that arrived after its action was undone, never swapped in.
const REGROWTH_DISCARDED := "The regrowth ended after its action was undone, so it was not swapped in: the land is as the undo left it."
## What the bar's Undo says: the action it undoes (the history's name of it), or no action to undo.
const UNDO_NAMED := "Undo %s"
const UNDO_NOTHING := "Undo"
## What an undo or a redo said: the action undone or redone, or why it changed nothing ("Undo" or "Redo", the action,
## the reason): its steps let go to keep the history within its budget, by a reload, or because the view shows another
## land; it holds no step, its regrowth stopped or refused before it changed the land; or the land refused a step, the
## land's sentence following.
const UNDID := "Undid %s."
const REDID := "Redid %s."
const UNDO_WORD := "Undo"
const REDO_WORD := "Redo"
const CHANGED_NOTHING := "%s %s changed nothing: %s."
const WHY_BUDGET := "its steps were let go to keep the undo history within %s MB"
const WHY_RELOAD := "Landform was reloaded since it was done, which let its steps go"
const WHY_OTHER_LAND := "it was done to another land than the one the view shows"
const WHY_REFUSED := "the land refused its steps"
const WHY_NOTHING_DONE := "its regrowth ended before it changed the land"
## What the bar says when steps were let go to keep the history within its budget, with how far undo then reaches, and
## what the bar's Undo tells of the history: how far undo reaches and what its steps hold of the budget.
const LET_GO_ONE := "The oldest undo step was let go to keep the undo history within %s MB: undo reaches %s back."
const LET_GO_MANY := "The %s oldest undo steps were let go to keep the undo history within %s MB: undo reaches %s back."
const UNDO_REACH := "Undo reaches %s back, holding %s MB of the %s MB it keeps."
const STEPS_NONE := "no step"
const STEPS_ONE := "1 step"
const STEPS_MANY := "%s steps"
const TOOK := "Last regrow took %s s."
const MADE_TOOK := "The new land grew in %s s and was written to %s."
const WRITTEN_NOT_SHOWN := "The new land was written to %s."
## Why a call is refused.
const NOT_IDLE := "The land is growing: stop it, or wait until it ends, first."
const NO_LAND_SHOWN := "The view shows no land: make one, or give the view a land file."
const HAS_LAND := "The view shows a land already: a new land is made in a view that shows none."
const NOTHING_TO_REGROW := "Nothing waits to regrow, so Regrow has nothing to do."
const NOTHING_SELECTED := "Nothing is selected, so there is nothing to type a number into."
const NO_SUCH_FIELD := "The selected %s has no field %s."
const NOT_A_NUMBER := "Enter a number for %s."
const NO_FILE := "Choose a file ending in .lfland."
const FILE_STANDS := "Choose a new path or open the existing file %s in Land File."
const NO_LOOK := "The land's view has no look, so it cannot paint a new land's ground: give it a LandformTerrainLook."
const NO_CONFIG := "The land's view has no config, so it cannot grow a land: give it a LandformConfig."
const SIZE_NOT_OFFERED := "%s m is not a size a new land is made in: give one of %s m."
const NO_SUCH_COUNTRY := "There is no kind of country called %s: give one of %s."
const NO_SUCH_SEED := "The seed %d is not one a land takes: give a whole number from 0 to %d."
const UNSAVED_ONE := "The land %s has changes that are not saved."
const UNSAVED_MANY := "The lands %s have changes that are not saved."
const NO_POINT_SELECTED := "No river's point is selected, so there is no point to remove."
const KEEPS_TWO_POINTS := "River %d keeps two points at least, so its point cannot be removed: remove the whole river with Remove river in the Landform panel."
const NOTHING_TO_REMOVE := "Nothing is selected, so there is nothing to remove."
## What Delete says with a whole river selected (from the panel's list), and while a tool's gesture goes on.
const DELETE_WHOLE_RIVER := "Delete removes a river's point or a lake: remove a whole river with Remove river in the Landform panel."
const DELETE_WAITS := "Delete removes the selected item once the tool's gesture has ended: finish it, or drop it with %s."
const DELETE_WAITS_FINISH := "Delete removes the selected item once the tool's gesture has ended: finish it first."
## Why the outline is not worked out while a river is being drawn and has one point.
const RIVER_NEEDS_POINT := "River %d needs a second point before what regrowing changes can be worked out."
## Why the view does not open the file its land_file names: the land shown, from its own file, has changes not saved;
## a change of the drawing goes on (a gesture); or that file did not load when the view opened it, which the view
## tries again only once the file is named again.
const FILE_WAITS := "The view's land file now names %s, but the land shown has changes not saved to %s: save or undo them first, and the view then opens it."
const FILE_WAITS_CHANGE := "The view's land file now names %s, but a change of the drawing is going on: finish or drop it first, and the view then opens it."
const FILE_NOT_LOADED := "The view's land file names %s, which did not load when the view opened it, so the land shown stays: name the file again to open it once more."
## What the editor says before it closes a scene whose new land's file the scene names only once it is saved.
const UNNAMED_ONE := "The scene names its new land %s only once it is saved."
const UNNAMED_MANY := "The scene names its new lands %s only once it is saved."
## What the panel shows of an item.
const RIVER_NAME := "River %d"
const LAKE_NAME := "Lake %d"
const DRAWN_WIDE := "%s m wide"
const DRAWN_RADIUS := "%s m radius"
const GROWN_WIDTHS := "%s to %s m"
const GROWN_LEVEL := "level %s m"
const WAITING := "waiting to regrow"
const LEFT_OUT := "left out"
const NOT_GROWN := "no water"
const POINT_OF := "Point %d of %d"
const WIDTH_HERE := "Width here %s m"
const WIDTH_VARIES := "Width varies by %s %%"
const LEVEL := "Level %s m"
## What the panel's button that removes the selected item says.
const REMOVE_RIVER := "Remove river"
const REMOVE_LAKE := "Remove lake"
## What regrowing an area painted would change, in the LAND state.
const AREA_CHANGES := "Area regrowth outline covers %s ha."
const AREA_CHANGES_NOTHING := "This area needs no regrowth."
## What a Hold stroke did, said once it is released: the cells whose mark it changed, and the lakes it held or lifted
## whole ("lake 1", "lakes 1 and 2", by their places in the drawing being edited).
const HELD_CELLS := "Held %s%s."
const LIFTED_CELLS := "Lifted the hold on %s%s."
const ONE_CELL := "%s cell"
const CELLS := "%s cells"
const WITH_WHOLE := ", with %s whole"
const ONE_LAKE := "lake %d"
const LAKES := "lakes %s"
const HELD_NOTHING := "This ground is already held."
const LIFTED_NOTHING := "No held ground in this stroke."
## What the panel shows of the chosen brush: its title, and how it is used (the modifier's, the keys' and the stop
## shortcut's names filled in).
const BRUSH_TITLES: Dictionary[String, String] = {
	"high_ground": "High ground brush", "hold": "Hold brush", "area": "Area brush",
}
const BRUSH_USES: Dictionary[String, String] = {
	"high_ground": "Paint high ground or %s-drag to lower it in the drawing.",
	"hold": "Drag to hold ground or %s-drag to release it, including any lake touched in full.",
	"area": "Paint an area or %s-drag to subtract, then press Regrow.",
}
const BRUSH_KEYS := "Resize with %s and %s."
const AREA_CLEARS := "%s clears the area."
## What each tool's button tells of its gestures (LandformLandToolbar's tooltips), the settings' keys named: the
## modifier that turns a brush's stroke over, the keys that change a brush's radius, the stop shortcut.
const TOOL_TIPS: Dictionary[String, String] = {
	"select": "Drag points or lakes to move them. Drag a lake rim to resize. Click a river line to add a point. Delete removes a point or lake.",
	"high_ground": "Paint high ground. {modifier}-drag lowers it in the drawing. Resize with {smaller} and {larger}.",
	"river": "Click points from upstream to downstream. Enter or double-click to finish.",
	"lake": "Drag from the lake center to its edge.",
	"hold": "Protect ground from regrowth, including whole lakes. {modifier}-drag releases it. Resize with {smaller} and {larger}.",
	"area": "Paint where to regrow from the drawing. {modifier}-drag subtracts. Resize with {smaller} and {larger}.",
}
## A lake the hold sentence names that has no place in the drawing being edited (it was moved or removed there).
const A_LAKE := "a lake"
## Why a brush's number is not taken.
const NO_BRUSH := "The tool chosen is no brush, so it has no radius or strength."
const NO_STRENGTH := "The %s has no strength: a cell is in it or not."
## The modifiers the settings may name, by their mask, and the keys that hold them, whose names are said.
const MODIFIER_KEYS: Dictionary[int, Key] = {
	KEY_MASK_SHIFT: KEY_SHIFT, KEY_MASK_ALT: KEY_ALT, KEY_MASK_META: KEY_META, KEY_MASK_CTRL: KEY_CTRL,
}
## The printable keys, whose names are their own characters ([ and ]).
const PRINTABLE_LEAST := 33
const PRINTABLE_MOST := 126

## What the tool turns: the brushes, the stop shortcut, the look.
var settings: LandformToolSettings
## Whether a change of the drawing regrows as soon as it is released; a whole-land change never does.
var regrow_on_release := false

var _view: LandformLandView
## The land the view shows, or the empty land a new one grows into; null for none.
var _land: LandformLand
var _state := State.NO_LAND
## The land's drawings in its present state: what it asks for and what it was grown from.
var _asked: LandformDrawing
var _grown: LandformDrawing
## The drawing being edited: the asked drawing, or a change of it not yet released (`_changed`).
var _working: LandformDrawing
var _changed := false
var _selection: Dictionary = {}
var _tool := TOOLS[0]
## The latest outline of regrowing toward the working drawing, and why one could not be asked for.
var _outline: LandformOutline
var _outline_refused := ""
## A seed typed and not yet regrown or taken back; -1 for none.
var _pending_seed := -1
## The running job: its kind, when it started (ticks in ms), and what it regrows.
var _job_kind := ""
var _job_started_ms := 0
var _job_inside_ha := -1.0
var _job_whole := false
var _job_seed := 0
var _job_size_m := 0
## The file a new land is written to.
var _new_file := ""
## Whether the session stopped the running job, so its end is a stop and not a failure.
var _stopping := false
## FAILED's sentences, what the last job or call said, and why the view's land file could not be shown.
var _failure := PackedStringArray()
var _said := PackedStringArray()
## Notes from the last successful regrowth, between its summary and any subsequent warnings.
var _regrow_notes := PackedStringArray()
var _notes_after := 0
var _read_error := ""
## What the land's `poll` delivered, handled after it returns.
var _delivered: Array = []
## The view must be refreshed once it is back in the tree.
var _refresh_waits := false
## The view's land file the session last opened, or found at the start, so a file that does not load is read once.
var _file_tried := ""
## Whether the latest outline is of the drawing being edited as it is now: it arrived with no newer request waiting
## and nothing was asked since. Such an outline is kept across the drawing's release, which asks for it unchanged.
var _outline_current := false
## Whether a new land's file was set on the view and the view's scene is not yet marked as changed.
var _scene_unmarked := false
## What lies under the pointer, for the tools, and the tools by their names in TOOLS.
var _picking: LandformPicking
var _tools: Dictionary[String, LandformLandTool] = {}
## The radius in metres of each brush by its name in BRUSHES, and the high ground brush's strength: the settings'
## values at the start of the sitting, changed in the panel or by the keys, never written to the settings.
var _brush_radius_m: Dictionary[String, float] = {}
var _brush_strength := 0.0
## The area painted to be regrown as drawn (an Image.FORMAT_L8 of the land's size, a cell in it at 128 or more), or
## null for none.
var _area: Image
## The undo history the session adds its actions to, its actions oldest first, each held weakly (the history holds
## them, and lets an action go when it drops it), and the action whose regrowth runs, its step to come.
var _history: LandformLandHistory
var _actions: Array[WeakRef] = []
var _growing_action: LandformLandAction
## The step the last ask of the land made, null when nothing was asked or the land refused it.
var _last_ask: LandformLandStep
## The land's world as the session last saw it (its instance id, 0 for none): another one, which only a restore of
## the land makes, means Landform was reloaded.
var _world_id := 0
## What an undo or a redo says before what it did (a regrowth it stopped first, a reload it found), and what the
## history said when it last let steps go.
var _history_said := PackedStringArray()
var _let_go_said := ""
## Whether a left press made while the land grows was taken (`_held_while_growing`), so its moves and release are.
var _pressed_while_growing := false


## A session for this view, with the tool's settings, adding its actions to `history` (the editor's, in the plugin), or
## to a history of its own when none is given. A view that names a land file it does not show (it did not load, or the
## view refused it) says why (LandformLandView.get_open_error, or opened once when the view has not tried), in the
## NO_LAND state.
func _init(view: LandformLandView, tool_settings: LandformToolSettings, undo_history: LandformLandHistory = null) -> void:
	_view = view
	settings = tool_settings
	_history = undo_history if undo_history != null else LandformLandHistory.of_its_own()
	_history.changed.connect(_on_history_changed)
	regrow_on_release = settings.regrow_on_release
	_picking = LandformPicking.new(view, settings.pick_pixels)
	_tools = {
		TOOLS[0]: LandformSelectTool.new(self, _picking),
		TOOLS[1]: LandformHighGroundTool.new(self, _picking),
		TOOLS[2]: LandformRiverTool.new(self, _picking),
		TOOLS[3]: LandformLakeTool.new(self, _picking),
		TOOLS[4]: LandformHoldTool.new(self, _picking),
		TOOLS[5]: LandformAreaTool.new(self, _picking),
	}
	_brush_radius_m = {
		BRUSHES[0]: settings.high_ground_radius_m, BRUSHES[1]: settings.hold_radius_m, BRUSHES[2]: settings.area_radius_m,
	}
	_brush_strength = settings.high_ground_strength
	if settings.overlay_look != null:
		_view.set_overlay_colours(settings.overlay_look)
	var land := _view.get_land()
	_file_tried = _view.land_file
	if land == null and not _view.land_file.is_empty():
		_read_error = _view.get_open_error()
		if _read_error.is_empty():
			_read_error = _view.open()
		land = _view.get_land()
	if land != null:
		_take(land)


## Whether the session's view still exists; the plugin lets a session go once it does not.
func is_alive() -> bool:
	return is_instance_valid(_view)


## Lets the land go: no signal of it reaches the session after this, and its steps let go free what they hold.
func close() -> void:
	if _land != null:
		_land.keep_steps_within(0)
	_let_go(_land)
	_land = null
	if _history.changed.is_connected(_on_history_changed):
		_history.changed.disconnect(_on_history_changed)


func get_view() -> LandformLandView:
	return _view


## The land the view shows; null in NO_LAND and while a new land grows.
func get_land() -> LandformLand:
	return _land if _has_land() else null


func get_state() -> State:
	return _state


## The drawing being edited and the selection: {"river": i}, {"river": i, "point": j}, {"lake": k} or {}.
func get_working_drawing() -> LandformDrawing:
	return _working


func get_selection() -> Dictionary:
	return _selection


func get_tool() -> String:
	return _tool


## The gesture the chosen tool is in (LandformLandTool.NO_GESTURE for none): a point dragged, a river being drawn.
func get_gesture() -> String:
	var tool := _tools.get(_tool) as LandformLandTool
	return tool.get_gesture() if tool != null else LandformLandTool.NO_GESTURE


## The latest outline of regrowing toward the working drawing, or null.
func get_outline() -> LandformOutline:
	return _outline


## Calls the land's `poll` and handles what it delivered: a job's step swapped in and shown, a new land written and
## shown, a job's end, an outline. Returns at once while nothing ended. The plugin calls it every frame. A land
## restored since the last call (a reload of the extension) is said first, its actions let go.
func poll() -> void:
	if not is_alive():
		return
	_follow_the_view()
	_check_restored()
	if _land != null:
		_land.poll()
	while not _delivered.is_empty():
		var delivered: Variant = _delivered.pop_front()
		if delivered is LandformLandStep:
			_finished(delivered as LandformLandStep)
		elif delivered is LandformOutline:
			_outline_arrived(delivered as LandformOutline)
		elif delivered is String:
			_failed(delivered as String)
		else:
			_made()
	if _refresh_waits and _view.is_inside_tree():
		_refresh_waits = false
		var refused := _view.refresh()
		if not refused.is_empty():
			_said.append(refused)
			changed.emit()
	if _state == State.GROWING:
		ticked.emit()


## Hands an event of the 3D view, seen through `camera`, to the session; returns whether the session took it, which
## the editor then does nothing more with. Delete with an item selected is the session's under every tool and in every
## state (delete_selected), so it never reaches the editor's own Delete, which would delete the view; with nothing
## selected it is left to the editor. The stop shortcut stops a running job, or with none ends the chosen tool's
## gesture and drops its change; any other event goes to the chosen tool in the LAND state alone, so while the land
## grows the tools are off. What no tool takes (the right and middle buttons, the wheel) is left to the editor's camera.
func input(event: InputEvent, camera: Camera3D) -> bool:
	var tool := _tools.get(_tool) as LandformLandTool
	if _is_delete(event) and not _selection.is_empty():
		delete_selected()
		return true
	if _is_stop(event):
		if _state == State.GROWING:
			stop()
			return true
		if tool != null and tool.get_gesture() != LandformLandTool.NO_GESTURE:
			tool.cancel()
			return true
		return false
	if camera == null or tool == null:
		return false
	if _state == State.GROWING:
		return _held_while_growing(event, camera)
	# A press taken while the land grew ends when it no longer grows, so its moves are the editor's camera's again
	# even if the regrowth landed before the release.
	_pressed_while_growing = false
	if not _state in [State.LAND, State.FAILED]:
		return false
	# The editor's undo while a river is drawn takes back its last point, the river with its first (stage 2's ruling
	# R21, S1).
	var river := tool as LandformRiverTool
	if river != null and river.get_gesture() == LandformRiverTool.DRAW_RIVER and _history.is_undo(event):
		river.take_back()
		return true
	return tool.input(event, camera)


## While the land grows the tools are off, yet a left press whose ray meets the land, its moves and its release are
## taken and do nothing, so the editor's own selection does not take them and deselect the view, which would take
## the panel, the bar and Escape's stop away (stage 2's ruling R21, M1); a press off the land stays the editor's.
func _held_while_growing(event: InputEvent, camera: Camera3D) -> bool:
	var button := event as InputEventMouseButton
	if button != null and button.button_index == MOUSE_BUTTON_LEFT:
		if button.pressed:
			_pressed_while_growing = not button.alt_pressed and _picking.ground_under(button.position, camera) != LandformLandView.NO_GROUND
			return _pressed_while_growing
		var taken := _pressed_while_growing
		_pressed_while_growing = false
		return taken
	return _pressed_while_growing and event is InputEventMouseMotion


# What the panel and the bar say.


## What the bar says now, sentence by sentence: in LAND what regrowing would change, in GROWING what grows and how to
## stop it, in FAILED the engine's sentence and that the land is as it was.
func get_bar_sentences() -> PackedStringArray:
	match _state:
		State.NO_LAND:
			return PackedStringArray()
		State.GROWING:
			return _growing_sentences()
		State.FAILED:
			return _failure
	var sentences := _land_sentences()
	if _file_waits():
		sentences.append(_file_waits_sentence())
	return sentences


## What the last job or call said: how long the last regrowth took and its notes, that a job was stopped, or why a
## call was refused; empty for nothing.
func get_said_sentences(include_regrow_notes := true) -> PackedStringArray:
	if include_regrow_notes and not _regrow_notes.is_empty():
		return _said.slice(0, _notes_after) + _regrow_notes + _said.slice(_notes_after)
	return _said


func get_regrow_notes() -> PackedStringArray:
	return _regrow_notes


func _set_feedback(sentences: PackedStringArray) -> void:
	_said = sentences
	_regrow_notes = PackedStringArray()


## What the panel says in the NO_LAND state: what a land is, and why the view's land file is not shown when it is not.
func get_intro_sentences() -> PackedStringArray:
	var sentences := WHAT_A_LAND_IS.duplicate()
	if not _read_error.is_empty():
		sentences.append(_read_error)
	return sentences


## What the form offers a new land: `sizes` (in metres) and `size`, `countries` and `country`, `seed`, `seed_most`.
func get_new_land_offer() -> Dictionary:
	return {
		"sizes": settings.new_land_sizes_m,
		"size": settings.new_land_size_m,
		"countries": LandformCountry.get_names(),
		"country": settings.new_land_country,
		"seed": settings.new_land_seed,
		"seed_most": SEED_MOST,
	}


## The land as the panel shows it: `size_m` and `size` (its text), `country` (the kind the drawing being edited asks
## for, or "" for numbers of its own) with `countries`, `seed` (a seed typed and not yet regrown, else the land's),
## `land_seed`, `reach_m` with `reach_least_m` and `reach_most_m`, `band` (its text), `file` (the file the land is
## from, which it saves to), `saved`, and `notes`, the land's notes about itself. Empty without a land.
func get_land_facts() -> Dictionary:
	if not _has_land():
		return {}
	return {
		"size_m": _land.get_size(),
		"size": "%s m" % number_text(_land.get_size(), 0),
		"country": _kind_of(_working.country),
		"countries": LandformCountry.get_names(),
		"seed": _pending_seed if _pending_seed >= 0 else _land.get_seed(),
		"land_seed": _land.get_seed(),
		"reach_m": _land.get_reach_m(),
		"reach_least_m": LandformLand.REACH_MIN_M,
		"reach_most_m": LandformLand.REACH_MAX_M,
		"band": "%d m" % LandformLand.BAND_M,
		"file": _view.get_land_path(),
		"saved": _land.is_saved(),
		"notes": _notes_about(ABOUT_LAND),
	}


## The rivers and lakes of the drawing being edited, rivers first, each with what was drawn and what grew: `kind`,
## `index`, `name`, `drawn`, `grown` ("waiting to regrow" for an item the land has not grown, "left out" for a lake
## the ground did not hold), `waiting`, `selected` and `notes`, the notes about the item's grown partner.
func get_items() -> Array[Dictionary]:
	var items: Array[Dictionary] = []
	if not _has_land() or _working == null:
		return items
	var world := _land.get_world()
	for i: int in _working.rivers.size():
		var river := _working.rivers[i] as LandformRiver
		var partner := _partner(RIVER, i)
		var grown := WAITING
		if partner >= 0:
			var widths := _grown_widths(world, partner)
			grown = NOT_GROWN if widths.is_empty() else GROWN_WIDTHS % [_tenths(widths[0]), _tenths(widths[1])]
		items.append(_item(RIVER, i, RIVER_NAME % (i + 1), DRAWN_WIDE % short_text(river.width_m), grown, partner))
	for k: int in _working.lakes.size():
		var lake := _working.lakes[k] as LandformLake
		var partner := _partner(LAKE, k)
		var grown := WAITING
		if partner >= 0:
			var grown_lake := _grown_lake(world, partner)
			grown = LEFT_OUT if grown_lake < 0 else GROWN_LEVEL % _tenths(world.get_lake_level(grown_lake))
		items.append(_item(LAKE, k, LAKE_NAME % (k + 1), DRAWN_RADIUS % short_text(lake.radius_m), grown, partner))
	return items


## The selected item's fields and what is shown beside them: `kind`, `index`, `title`, `remove` (what the button that
## removes the item says, Remove river for a point too); a point's `point` and
## `point_text`, `east` and `south`; a river's `width`; a lake's `east`, `south` and `radius`; `fields`, the names
## `type_field` takes for it; `shown`, the numbers shown and not typed (a point's grown width, the river's wander, a
## lake's level); `notes`. Empty when nothing is selected.
func get_selected() -> Dictionary:
	if not _has_land() or _selection.is_empty():
		return {}
	var world := _land.get_world()
	if _selection.has(LAKE):
		var k: int = _selection[LAKE]
		var lake := _working.lakes[k] as LandformLake
		var partner := _partner(LAKE, k)
		var level := WAITING
		if partner >= 0:
			var grown_lake := _grown_lake(world, partner)
			level = LEFT_OUT if grown_lake < 0 else LEVEL % _tenths(world.get_lake_level(grown_lake))
		return {
			"kind": LAKE, "index": k, "title": LAKE_NAME % (k + 1), "remove": REMOVE_LAKE,
			"east": lake.x_m, "south": lake.z_m, "radius": lake.radius_m,
			"fields": PackedStringArray([EAST, SOUTH, RADIUS]),
			"shown": PackedStringArray([level]),
			"notes": _notes_about("%s %d" % [LAKE, partner]) if partner >= 0 else PackedStringArray(),
		}
	var i: int = _selection[RIVER]
	var river := _working.rivers[i] as LandformRiver
	var partner := _partner(RIVER, i)
	var wander := WIDTH_VARIES % short_text(_width_wander() * PERCENT)
	var selected := {
		"kind": RIVER, "index": i, "title": RIVER_NAME % (i + 1), "remove": REMOVE_RIVER,
		"width": river.width_m,
		"fields": PackedStringArray([WIDTH]),
		"notes": _notes_about("%s %d" % [RIVER, partner]) if partner >= 0 else PackedStringArray(),
	}
	if not _selection.has(POINT):
		var widths := _grown_widths(world, partner) if partner >= 0 else PackedFloat32Array()
		var grown := WAITING if partner < 0 else (NOT_GROWN if widths.is_empty() else GROWN_WIDTHS % [_tenths(widths[0]), _tenths(widths[1])])
		selected["shown"] = PackedStringArray([grown, wander])
		return selected
	var j: int = _selection[POINT]
	var east := river.points_m[2 * j]
	var south := river.points_m[2 * j + 1]
	var here := WAITING if partner < 0 else _width_here(world, partner, east, south)
	selected.merge({
		"point": j, "point_text": POINT_OF % [j + 1, point_count(river.points_m)],
		"east": east, "south": south,
		"fields": PackedStringArray([EAST, SOUTH, WIDTH]),
		"shown": PackedStringArray([here, wander]),
	}, true)
	return selected


## Whether Regrow would start a job: a land shown and idle, and an area painted, a change waiting, a seed typed, or a
## change not yet released, and the outline of the drawing as it is says no refusal (stage 2's ruling R18, N2).
func can_regrow() -> bool:
	if not _state in [State.LAND, State.FAILED] or not _has_land():
		return false
	return (_area != null or _pending_seed >= 0 or _waits()) and _outline_refusal().is_empty()


## What each tool's button tells of its gestures, by the tool's name in TOOLS, the settings' keys named (stage 2's
## ruling R18, N7).
func get_tool_tip(tool_name: String) -> String:
	var keys := {
		"modifier": _modifier_text(),
		"smaller": _key_text(settings.brush_smaller_key), "larger": _key_text(settings.brush_larger_key),
	}
	var tip := String(TOOL_TIPS.get(tool_name, "")).format(keys)
	var stop := _stop_text()
	if not stop.is_empty():
		if tool_name == RIVER:
			tip += " %s cancels." % stop
		elif tool_name == "area":
			tip += " " + AREA_CLEARS % stop
	return tip


## Whether a seed was typed that is not the land's, waiting for Regrow or for `keep_seed`.
func has_pending_seed() -> bool:
	return _pending_seed >= 0


## Whether there are land changes to call out when opening the saved land in a separate preview.
func has_pending_walk_edits() -> bool:
	return _has_land() and (
		not _land.is_saved() or _waits() or has_pending_seed() or _area != null
		or _view.land_file != _view.get_land_path()
	)


## The undo history the session adds its actions to.
func get_history() -> LandformLandHistory:
	return _history


## What the bar's Undo says: "Undo move", naming the action an undo would undo in the view's history (the history's
## name of it, so another of the editor's actions by its own name), or "Undo" when there is none.
func get_undo_text() -> String:
	var action_name := _history.get_undo_name(_view)
	return UNDO_NAMED % action_name if not action_name.is_empty() else UNDO_NOTHING


## Whether the view's history has an action to undo.
func can_undo() -> bool:
	return not _history.get_undo_name(_view).is_empty()


## How far undo reaches: the session's actions done and kept, and what their steps hold of the budget, in one sentence.
func get_undo_reach() -> String:
	var bytes := 0
	for action: LandformLandAction in _alive_actions():
		if action.is_kept():
			bytes += action.get_bytes()
	return UNDO_REACH % [_steps_text(_undo_reach()), number_text(float(bytes) / BYTES_PER_MB, 1), number_text(settings.undo_budget_mb, 0)]


## The chosen brush as the panel shows it: `tool` (its name in BRUSHES), `title`, `radius_m` with `radius_least_m` and
## `radius_most_m` (half the land's side), `strength` for the high ground brush (absent for the others, whose cells are
## in or out), and `uses`, how it is used, a sentence each. Empty when the tool chosen is no brush or no land is shown.
func get_brush() -> Dictionary:
	if not _tool in BRUSHES or not _has_land():
		return {}
	var uses := PackedStringArray([BRUSH_USES[_tool] % _modifier_text()])
	if _tool == BRUSHES[2] and not _stop_text().is_empty():
		uses.append(AREA_CLEARS % _stop_text())
	uses.append(BRUSH_KEYS % [_key_text(settings.brush_smaller_key), _key_text(settings.brush_larger_key)])
	var brush := {
		"tool": _tool, "title": BRUSH_TITLES[_tool], "radius_m": get_brush_radius(),
		"radius_least_m": settings.brush_radius_least_m, "radius_most_m": _brush_radius_most(), "uses": uses,
	}
	if _tool == STRENGTH_BRUSH:
		brush["strength"] = _brush_strength
	return brush


## The radius in metres the chosen brush paints with, held from the settings' least to half the land's side; 0 when
## the tool chosen is no brush.
func get_brush_radius() -> float:
	if not _tool in BRUSHES:
		return 0.0
	return clampf(_brush_radius_m[_tool], settings.brush_radius_least_m, maxf(_brush_radius_most(), settings.brush_radius_least_m))


## The strength the chosen brush paints with: the high ground brush's, 0 for the others.
func get_brush_strength() -> float:
	return _brush_strength if _tool == STRENGTH_BRUSH else 0.0


## The area painted to be regrown as drawn, or null for none: the view draws it, the bar says what regrowing it would
## change, and Regrow regrows it. The session's own: a brush paints it in place.
func get_area() -> Image:
	return _area


# What a person does.


## Chooses the tool by its name in `TOOLS`; an unknown name changes nothing. The tool chosen before ends its gesture
## and drops its change.
func set_tool(tool_name: String) -> void:
	if tool_name in TOOLS and tool_name != _tool:
		var before := _tools.get(_tool) as LandformLandTool
		if before != null:
			before.cancel()
		_tool = tool_name
		changed.emit()


## Sets the chosen brush's radius, held from the settings' least to half the land's side; it paints so from its next
## stroke on. Returns "" or why not (no brush chosen, not a number).
func set_brush_radius(radius_m: float) -> String:
	if not _tool in BRUSHES:
		return NO_BRUSH
	if not is_finite(radius_m):
		return NOT_A_NUMBER % "radius"
	_brush_radius_m[_tool] = radius_m
	_brush_radius_m[_tool] = get_brush_radius()
	changed.emit()
	return ""


## Makes the chosen brush's radius larger (`steps` above 0) or smaller by the settings' step, that many times: the keys
## of the 3D view (LandformToolSettings.brush_larger_key and brush_smaller_key).
func step_brush_radius(steps: int) -> void:
	if _tool in BRUSHES:
		set_brush_radius(get_brush_radius() * pow(settings.brush_radius_step, steps))


## Sets the high ground brush's strength, held to 0..1, from its next stroke on. Returns "" or why not.
func set_brush_strength(strength: float) -> String:
	if not _tool in BRUSHES:
		return NO_BRUSH
	if _tool != STRENGTH_BRUSH:
		return NO_STRENGTH % String(BRUSH_TITLES[_tool]).to_lower()
	if not is_finite(strength):
		return NOT_A_NUMBER % "strength"
	_brush_strength = clampf(strength, 0.0, 1.0)
	changed.emit()
	return ""


## Draws this picture as the held ground in the view, in place of the land's, while a Hold stroke goes on (the land's
## held picture as the stroke leaves it); null draws the land's again.
func show_held(held: Image) -> void:
	if _has_land():
		_view.show_held(held)


## Holds (`on`) or lifts the hold on the cells of a Hold stroke (an Image.FORMAT_L8 of the land's size, a cell in it at
## 128 or more): the land's hold (LandformLand.hold), which holds or lifts a lake whose water it meets whole, done at
## once and shown, one action of the history unless it changed no mark; the bar says the cells whose mark it changed
## and the lakes held or lifted whole. No change of the drawing: nothing regrows. Returns "" or the land's refusal.
func hold_stroke(stroke: Image, on: bool) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	_leave_failed()
	_view.show_held(null)
	var step := _land.hold(stroke, on)
	if step == null:
		var refused := _refused(_land.get_job_error())
		changed.emit()
		return refused
	_set_feedback(PackedStringArray([_held_sentence(step, on)]))
	var not_shown := _show_the_land()
	if not not_shown.is_empty():
		_said.append(not_shown)
	if step.get_cells_changed() > 0:
		var held: Array[LandformLandStep] = [step]
		_act(ACTION_HOLD if on else ACTION_LIFT, held)
	_ask_outline()
	changed.emit()
	return ""


## Makes this the area painted to be regrown as drawn (an Image.FORMAT_L8 of the land's size, a cell in it at 128 or
## more, which a brush paints in place), or a stroke's next move on it: the view draws it and the land works out the
## outline of regrowing it (LandformLand.start_area_outline), which takes a lake it touches whole; while it is painted,
## the outline last worked out stays shown until the next arrives.
func set_area(area: Image) -> void:
	if not _has_land() or area == null or _state == State.GROWING:
		return
	_leave_failed()
	var going_on := _area != null
	_area = area
	_view.show_area(_area)
	_ask_outline(going_on)
	changed.emit()


## A stroke on the area has been released: an area with no cell left painted is cleared.
func release_area() -> void:
	if _area != null and not _area.get_data().has(LandformBrush.MARKED):
		clear_area()
	else:
		changed.emit()


## Clears the area painted: nothing is drawn of it, and the bar says what regrowing the drawing would change again.
func clear_area() -> void:
	if _area == null:
		return
	_area = null
	if _has_land():
		_view.show_area(null)
		_ask_outline()
	changed.emit()


## Turns "Regrow on release" on or off.
func set_regrow_on_release(on: bool) -> void:
	if on != regrow_on_release:
		regrow_on_release = on
		changed.emit()


## Selects an item of the drawing being edited, as `LandformLandLines` names a selection; {} selects nothing, and a
## selection of no item of the drawing too.
func select(selection: Dictionary) -> void:
	_selection = selection if _names_an_item(selection) else {}
	if _has_land():
		_view.show_drawing(_working, _selection)
	changed.emit()


## Makes `drawing` the one being edited, a change not yet released (a drag going on): the lines show it and the land
## works out the outline of regrowing toward it; while the change goes on (a drawing set over one not yet released),
## the outline last worked out stays shown until the next arrives. `release_change` asks the land for it. Nothing
## changes while the land grows (the tools are off then).
func set_working(drawing: LandformDrawing) -> void:
	if not _has_land() or drawing == null or _state == State.GROWING:
		return
	_leave_failed()
	var going_on := _changed
	# The lines are laid again only when the rivers or lakes changed, not for a stroke on the picture of high ground.
	var lines_change := _working == null or drawing.rivers != _working.rivers or drawing.lakes != _working.lakes
	_working = drawing
	_changed = true
	if not _names_an_item(_selection):
		_selection = {}
		lines_change = true
	if lines_change:
		_view.show_drawing(_working, _selection)
	_ask_outline(going_on)
	changed.emit()


## Drops a change of the drawing not yet released: the drawing being edited is the land's asked drawing again, as it
## was before the change, and nothing is asked of the land.
func drop_change() -> void:
	if not _has_land() or not _changed:
		return
	_read_drawings()
	_ask_outline()
	changed.emit()


## Says this sentence below what the bar says, as a call's refusal is said (a tool's, such as a river ended with one
## point).
func say(sentence: String) -> void:
	_set_feedback(PackedStringArray([sentence]))
	changed.emit()


## Removes the selected point from its river, the river then selected: a change of the drawing and its release, as a
## drag's. A river keeps two points, so a point of a river of two is refused (the panel's Remove river removes the
## river). Returns "" or why not, said in the bar.
func remove_point() -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	if not _selection.has(POINT):
		return _refused_now(NO_POINT_SELECTED)
	var i: int = _selection[RIVER]
	var j: int = _selection[POINT]
	var river := (_working.rivers[i] as LandformRiver).duplicate() as LandformRiver
	var points := river.points_m
	if point_count(points) <= RIVER_POINTS_LEAST:
		return _refused_now(KEEPS_TWO_POINTS % (i + 1))
	points.remove_at(2 * j + 1)
	points.remove_at(2 * j)
	river.points_m = points
	var drawing := copy_drawing(_working)
	var rivers := drawing.rivers
	rivers[i] = river
	drawing.rivers = rivers
	_selection = {RIVER: i}
	set_working(drawing)
	return release_change(ACTION_REMOVE_POINT)


## What Delete does with an item selected, under every tool: in LAND or FAILED with no gesture going on, it removes
## the selected point (remove_point) or lake (remove_item), and with a whole river selected says that the panel's
## Remove river removes it; while the land grows it says that it waits, and while a tool's gesture goes on that the
## gesture must end first. Returns "" or what it said.
func delete_selected() -> String:
	if _selection.is_empty():
		return _refused_now(NOTHING_TO_REMOVE)
	if _state == State.GROWING:
		return _refused_now(NOT_IDLE)
	# An area painted is no change of the drawing: Delete removes the item and leaves the area (stage 2's ruling R18,
	# N6).
	if not get_gesture() in [LandformLandTool.NO_GESTURE, LandformAreaTool.AREA_PAINTED]:
		return _refused_now(DELETE_WAITS % _stop_text() if not _stop_text().is_empty() else DELETE_WAITS_FINISH)
	if _selection.has(LAKE):
		return remove_item()
	if _selection.has(POINT):
		return remove_point()
	return _refused_now(DELETE_WHOLE_RIVER)


## Removes the selected river (the whole river, also when a point of it is selected) or lake from the drawing, nothing
## then selected: a change of the drawing and its release, as a drag's. The panel's Remove button. Returns "" or why
## not, said in the bar.
func remove_item() -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	if _selection.is_empty():
		return _refused_now(NOTHING_TO_REMOVE)
	var drawing := copy_drawing(_working)
	var action_name := ACTION_REMOVE_LAKE if _selection.has(LAKE) else ACTION_REMOVE_RIVER
	if _selection.has(LAKE):
		var lakes := drawing.lakes
		lakes.remove_at(int(_selection[LAKE]))
		drawing.lakes = lakes
	else:
		var rivers := drawing.rivers
		rivers.remove_at(int(_selection[RIVER]))
		drawing.rivers = rivers
	_selection = {}
	set_working(drawing)
	return release_change(action_name)


## Releases the change of the drawing being edited: the land asks for it (`LandformLand.ask`), and with "Regrow on
## release" on, a change that is not of the whole land regrows at once; while a seed typed waits nothing regrows on
## release, since the seed is a change of the whole land (the stage 2 plan's decision 7), nor while an area is painted,
## which Regrow regrows first, the change waiting after it (stage 2's ruling R21, S2), nor when the release asks for
## nothing new (N1). Returns "" or why the land
## refused it, the drawing being edited then the land's again. When the outline of the very drawing released, worked
## out during the gesture, already says the regrowth's refusal, the land asks for it and nothing regrows; the bar keeps
## the refusal (stage 2's ruling R19, S2: a number typed has no outline yet, and its regrowth is refused by the land).
## The release is one action of the history named `action_name` (what the person did: "move", "add river"), holding
## the ask's step and, when it regrows, the regrowth's step once it ends; a release that asks for the drawing the land
## asks for already makes none, and changes nothing (stage 2's ruling R19, N2).
func release_change(action_name := ACTION_CHANGE) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	# The current outline is of the drawing being edited, the one asked for now; while an area is painted it is the
	# area's, and the change waits for Regrow (stage 2's ruling R21, S2).
	var refusal := _outline_refusal() if _changed and _area == null else ""
	var refused := _ask_the_land()
	var steps: Array[LandformLandStep] = []
	if _last_ask != null:
		steps.append(_last_ask)
	var regrows := false
	# Only a release that asks for something new regrows (stage 2's ruling R21, N1), never while an area is painted
	# (S2) or a seed waits.
	var asks := not steps.is_empty()
	if asks and refused.is_empty() and refusal.is_empty() and regrow_on_release and _area == null and _pending_seed < 0 and _land.is_waiting() and not _country_changed():
		regrows = _regrow_waiting().is_empty()
	if asks:
		_act(action_name, steps, regrows)
	changed.emit()
	return refused


## Types a number into a field of the selected item (`EAST`, `SOUTH`, `WIDTH`, `RADIUS`, as `get_selected` lists
## them): the same edit as a drag and its release. Returns "" or why not.
func type_field(field: String, value: float) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	if _selection.is_empty():
		return NOTHING_SELECTED
	if not is_finite(value):
		return NOT_A_NUMBER % field
	var kind := LAKE if _selection.has(LAKE) else RIVER
	if not field in (get_selected()["fields"] as PackedStringArray):
		return NO_SUCH_FIELD % [kind, field]
	var drawing := copy_drawing(_working)
	if kind == LAKE:
		var k: int = _selection[LAKE]
		var lakes := drawing.lakes
		var lake := (lakes[k] as LandformLake).duplicate() as LandformLake
		match field:
			EAST: lake.x_m = value
			SOUTH: lake.z_m = value
			RADIUS: lake.radius_m = value
		lakes[k] = lake
		drawing.lakes = lakes
	else:
		var i: int = _selection[RIVER]
		var rivers := drawing.rivers
		var river := (rivers[i] as LandformRiver).duplicate() as LandformRiver
		var points := river.points_m
		match field:
			WIDTH: river.width_m = value
			EAST: points[2 * int(_selection[POINT])] = value
			SOUTH: points[2 * int(_selection[POINT]) + 1] = value
		river.points_m = points
		rivers[i] = river
		drawing.rivers = rivers
	set_working(drawing)
	return release_change(ACTION_TYPED % field)


## Asks for this kind of country (`LandformCountry.get_names()`), a change of the drawing like any other, released at
## once; a change of the whole land, so it never regrows on release. Returns "" or why not.
func set_country(country_name: String) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	if not country_name in LandformCountry.get_names():
		return NO_SUCH_COUNTRY % [country_name, _listed(LandformCountry.get_names())]
	var drawing := copy_drawing(_working)
	drawing.country = LandformCountry.named(country_name)
	set_working(drawing)
	return release_change(ACTION_COUNTRY)


## Types a seed (`typed`). The seed is no waiting edit: another seed than the land's asks at once, the bar saying that
## the whole land regrows, and Regrow regrows it from that seed (`LandformLand.start_reseed`), the changes waiting with
## it; `keep_seed` takes it back, and so does typing the land's own seed. Returns "" or why not.
func set_seed(typed: int) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	if typed < 0 or typed > SEED_MOST:
		return NO_SUCH_SEED % [typed, SEED_MOST]
	_leave_failed()
	_pending_seed = -1 if typed == _land.get_seed() else typed
	_ask_outline()
	changed.emit()
	return ""


## Takes back a seed typed and not regrown: the land keeps its own.
func keep_seed() -> void:
	if _pending_seed < 0:
		return
	_pending_seed = -1
	_ask_outline()
	changed.emit()


## Sets how far an edit's outline reaches beyond the ground it changes, held to the land's bounds
## (`LandformLand.set_reach_m`), done at once, one action of the history when the reach changed. Returns "" or why not.
func set_reach(reach_m: float) -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	_leave_failed()
	var before := _land.get_reach_m()
	var step := _land.set_reach_m(reach_m)
	if step == null:
		return _refused(_land.get_job_error())
	if _land.get_reach_m() != before:
		var reached: Array[LandformLandStep] = [step]
		_act(ACTION_REACH, reached)
	_ask_outline()
	changed.emit()
	return ""


## Regrows the land: the area painted as drawn while one is (`LandformLand.start_regrow_area`, what waits still
## waiting after it); else from the seed typed when one waits (`start_reseed`, the waiting changes with it); else
## toward the drawing being edited, a change not yet released asked for first (`start_regrow`). Returns "" or why not.
## Each is one action of the history, made as the regrowth starts and holding its step once it ends (with the ask of
## a change not yet released before it), so an undo while it runs stops it.
func regrow() -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	if _state == State.GROWING:
		return NOT_IDLE
	_leave_failed()
	var refusal := _outline_refusal()
	if not refusal.is_empty():
		# The outline of what would regrow already says the regrowth's refusal (stage 2's ruling R18, N2).
		return _refused_now(refusal)
	var steps: Array[LandformLandStep] = []
	if _area != null:
		var area_said := _start(JOB_AREA, _land.start_regrow_area(_area))
		if area_said.is_empty():
			_act(ACTION_AREA, steps, true)
		changed.emit()
		return area_said
	if _changed:
		var refused := _ask_the_land()
		if not refused.is_empty():
			changed.emit()
			return refused
		if _last_ask != null:
			steps.append(_last_ask)
	var said := ""
	var action_name := ACTION_REGROW
	if _pending_seed >= 0:
		_job_seed = _pending_seed
		action_name = ACTION_SEED
		said = _start(JOB_SEED, _land.start_reseed(_pending_seed))
		if said.is_empty():
			_pending_seed = -1
	elif _land.is_waiting():
		said = _regrow_waiting()
	else:
		said = NOTHING_TO_REGROW
	if said.is_empty() or not steps.is_empty():
		_act(action_name, steps, said.is_empty())
	changed.emit()
	return said


## Undoes the newest action of the view's history, as the editor's Undo does (the bar's Undo); returns whether there
## was one.
func undo() -> bool:
	return _history.undo()


## Redoes the oldest action undone in the view's history; returns whether there was one.
func redo() -> bool:
	return _history.redo()


## An action of the session is about to be undone or redone (LandformLandAction.undo, redo): a reload not yet found is
## found first, letting the action go; and while the action still changes the land, a regrowth running is stopped and
## waited for (LandformLand.stop_and_wait, at most one step of the edit), and what it gives is discarded, never swapped
## in, its action's regrowth ended.
func begin_history(action: LandformLandAction) -> void:
	_history_said = PackedStringArray()
	if _land == null:
		return
	if _check_restored():
		_history_said.append(RELOADED)
	if not action.is_kept() or not _land.is_busy() or (_state == State.GROWING and _job_kind == JOB_NEW):
		return
	_land.stop_and_wait()
	# What the stopped job gave, and any outline of the land as it was, are let go: the swap makes the land anew.
	_delivered.clear()
	if _growing_action != null:
		_growing_action.set_growing(false)
		_growing_action = null
	_end_job()
	_state = State.LAND
	_history_said.append(REGROWTH_STOPPED)


## An action of the session was undone or redone (`forward`), `refused` the land's sentence when a step of it was
## refused: the land is shown as it now is, its asked drawing being edited and in the lines, the selection kept where
## it still names an item, and what was done said; an action that changes nothing any more says why.
func end_history(action: LandformLandAction, forward: bool, refused: String) -> void:
	var said := _history_said.duplicate()
	_history_said = PackedStringArray()
	var word := REDO_WORD if forward else UNDO_WORD
	var why := action.get_let_go()
	if why != LandformLandAction.KEPT:
		said.append(CHANGED_NOTHING % [word, action.get_action_name(), _why_let_go(why)])
	elif not refused.is_empty():
		said.append(CHANGED_NOTHING % [word, action.get_action_name(), WHY_REFUSED])
		said.append(refused)
		# A step swapped and put back made the land's world anew twice: noted and shown, so the next poll takes it for
		# no reload (stage 2's ruling R19, N5).
		_note_world()
		var not_shown := _show_the_land()
		if not not_shown.is_empty():
			said.append(not_shown)
	elif action.get_steps().is_empty():
		said.append(CHANGED_NOTHING % [word, action.get_action_name(), WHY_NOTHING_DONE])
	else:
		said.append((REDID if forward else UNDID) % action.get_action_name())
		_note_world()
		if _pending_seed == _land.get_seed():
			_pending_seed = -1
		_leave_failed()
		if _river_being_drawn():
			said.insert(said.size() - 1, RIVER_DROPPED)
		_read_drawings()
		var not_shown := _show_the_land()
		if not not_shown.is_empty():
			said.append(not_shown)
		_ask_outline()
	_set_feedback(said)
	changed.emit()


## Stops the running job: the land is as it was once it ends (`LandformLand.cancel`, delivered by the next `poll`).
func stop() -> void:
	if _state != State.GROWING or _land == null:
		return
	_stopping = true
	_land.cancel()


## Leaves the FAILED state for the land, or for no land when none was made.
func dismiss() -> void:
	if _state != State.FAILED:
		return
	_leave_failed()
	changed.emit()


## Saves the land to the file it was read from or written to (LandformLandView.get_land_path), never to another file
## the view's land_file names since. Returns "" or the extension's sentence.
func save() -> String:
	if not _has_land():
		return NO_LAND_SHOWN
	var said := _land.save_file(_view.get_land_path())
	if not said.is_empty():
		_set_feedback(PackedStringArray([said]))
	changed.emit()
	return said


## Makes a new land in a view that shows none: its drawing of `size_m` with no river and no lake and a picture black
## all over of the settings' size, in the kind of country named, from `land_seed`, grown with the view's config
## (`LandformLand.start_new`), then written to `path` (a `res://`, `user://` or absolute path ending in .lfland, where no
## file stands yet), which the view's `land_file` then names, and shown. Making a land is no undo step. Returns "" once
## it grows, else why not.
func make_land(size_m: int, country_name: String, land_seed: int, path: String) -> String:
	if _state == State.GROWING:
		return NOT_IDLE
	if _has_land():
		return HAS_LAND
	var refused := _new_land_refusal(size_m, country_name, land_seed, path)
	if not refused.is_empty():
		_set_feedback(PackedStringArray([refused]))
		changed.emit()
		return refused
	var drawing := LandformDrawing.new()
	drawing.size_m = size_m
	drawing.country = LandformCountry.named(country_name)
	# Held on the CPU, so start_new reads it back from no GPU (stage 2's ruling R19, N6).
	var picture := LandformPicture.new()
	picture.picture = Image.create_empty(settings.new_land_picture_px, settings.new_land_picture_px, false, Image.FORMAT_RF)
	drawing.high_ground = picture
	var land := LandformLand.new()
	_watch(land)
	if not land.start_new(drawing, land_seed, _view.config):
		_let_go(land)
		return _refused(land.get_job_error())
	_land = land
	_world_id = 0
	_new_file = path
	_job_size_m = size_m
	_job_seed = land_seed
	_read_error = ""
	_begin(JOB_NEW)
	changed.emit()
	return ""


## Whether a new land's file was set on this session's view and the scene holding it is not yet marked as changed, so
## the scene names the land only once saved (unsaved_status says so).
func has_scene_mark() -> bool:
	return _scene_unmarked and is_alive()


## Whether the scene the editor edits now holds this session's view while a new land's file was set on it and the
## scene is not yet marked as changed: true once, and the plugin then marks it (EditorInterface.mark_scene_as_unsaved
## marks the scene edited now, which need not be the view's while a land grows).
func take_scene_mark(edited_root: Node) -> bool:
	if not _scene_unmarked or edited_root == null or not is_alive():
		return false
	if edited_root != _view and not edited_root.is_ancestor_of(_view):
		return false
	_scene_unmarked = false
	return true


# Saving, for the plugin, across the scenes open in the editor.


## Every land's view in or under these nodes, the nodes themselves among them.
static func views_under(roots: Array[Node]) -> Array[LandformLandView]:
	var views: Array[LandformLandView] = []
	for root: Node in roots:
		if root == null:
			continue
		if root is LandformLandView:
			views.append(root as LandformLandView)
		for found: Node in root.find_children("*", "LandformLandView", true, false):
			views.append(found as LandformLandView)
	return views


## The open scenes whose lands a save of scenes saves (an `EditorPlugin`'s `_save_external_data`, which Godot calls
## after a scene's save without saying which): the roots whose files were heard saved; for a file heard that no open
## root holds yet (the first save of a new scene, or Save Scene As, whose root takes its new file only after the
## lands are saved), the scene edited now, which is the one Godot saves; and every open scene when none was heard, as
## when the editor saves before it quits or runs.
static func scenes_saved(open_roots: Array[Node], heard: PackedStringArray, edited_root: Node) -> Array[Node]:
	if heard.is_empty():
		return open_roots.duplicate()
	var saved: Array[Node] = []
	var files := PackedStringArray()
	for root: Node in open_roots:
		if root == null:
			continue
		files.append(root.scene_file_path)
		if root.scene_file_path in heard and not root in saved:
			saved.append(root)
	for path: String in heard:
		if not path in files and edited_root != null and not edited_root in saved:
			saved.append(edited_root)
	return saved


## What the editor says before it closes these scenes while a land of theirs is not saved, or while a new land's file
## is named by a view of theirs whose scene was not marked as changed (`unmarked`, the views whose sessions hold that
## mark, has_scene_mark), so the scene names it only once saved: the lands by their files, a sentence each kind, or ""
## when every land is saved and named (an `EditorPlugin`'s `_get_unsaved_status`).
static func unsaved_status(roots: Array[Node], unmarked: Array[LandformLandView] = []) -> String:
	var files := PackedStringArray()
	var unnamed := PackedStringArray()
	for view: LandformLandView in views_under(roots):
		var land := view.get_land()
		if land != null and not land.is_saved():
			files.append(view.get_land_path())
		elif view in unmarked:
			unnamed.append(view.get_land_path())
	var said := PackedStringArray()
	if not files.is_empty():
		said.append((UNSAVED_ONE if files.size() == 1 else UNSAVED_MANY) % _listed(files))
	if not unnamed.is_empty():
		said.append((UNNAMED_ONE if unnamed.size() == 1 else UNNAMED_MANY) % _listed(unnamed))
	return " ".join(said)


## Saves every land of these scenes that is not saved to the file it was read from or written to
## (LandformLandView.get_land_path), never to another file the view's land_file names since; returns the sentences of
## those that could not be (an `EditorPlugin`'s `_save_external_data`).
static func save_lands(roots: Array[Node]) -> PackedStringArray:
	var failed := PackedStringArray()
	for view: LandformLandView in views_under(roots):
		var land := view.get_land()
		if land == null or land.is_saved():
			continue
		var said := land.save_file(view.get_land_path())
		if not said.is_empty():
			failed.append(said)
	return failed


# The land and its jobs.


func _has_land() -> bool:
	return _land != null and _land.get_size() > 0 and not (_state == State.GROWING and _job_kind == JOB_NEW)


## Takes the land the view shows: its signals, its drawings, and the state it is in. The actions of a land taken
## before are of another land from now on, and that land's steps are let go, freeing what they hold.
func _take(land: LandformLand) -> void:
	if _land != null and _land != land:
		for action: LandformLandAction in _alive_actions():
			action.let_go(LandformLandAction.LET_GO_OTHER_LAND)
		_land.keep_steps_within(0)
	_growing_action = null
	_let_go(_land)
	_land = land
	_watch(land)
	_note_world()
	_pending_seed = -1
	_outline = null
	_area = null
	_failure = PackedStringArray()
	if land.is_busy():
		# A job started before this session; what it regrows is not known.
		_job_kind = JOB_REGROW
		_job_inside_ha = -1.0
		_job_whole = false
		_job_started_ms = Time.get_ticks_msec() - int(land.get_busy_ms())
		_state = State.GROWING
	else:
		_state = State.LAND
	_read_drawings()
	_ask_outline()


## The view shows another land than the session's (read again, or given another): the session takes it. When the
## view's land_file names another file than the land shown came from, the view opens it once the land shown has
## nothing unsaved and no job runs, once for each file named (a file that does not load is said, and read again only
## when named again); until then the bar says why (`FILE_WAITS`).
func _follow_the_view() -> void:
	if _state == State.GROWING and _job_kind == JOB_NEW:
		return
	var named := _view.land_file
	if named.is_empty() and _view.get_land() != null and not _view.get_land_path().is_empty():
		# A land shown keeps its file: clearing the name would save a scene that names no land while it shows one
		# (stage 2's ruling R21, N5).
		_view.land_file = _view.get_land_path()
		named = _view.land_file
		_file_tried = named
		_set_feedback(PackedStringArray([LAND_FILE_KEPT % named]))
		changed.emit()
	if _state != State.GROWING and named != _view.get_land_path() and named != _file_tried and not _file_waits():
		_file_tried = named
		var said := _view.open()
		if _view.get_land() == null:
			_read_error = said
			changed.emit()
		elif not said.is_empty():
			_set_feedback(PackedStringArray([said]))
			changed.emit()
	var shown := _view.get_land()
	if shown != null and shown != _land:
		_take(shown)
		changed.emit()


func _watch(land: LandformLand) -> void:
	land.job_finished.connect(_on_job_finished)
	land.job_failed.connect(_on_job_failed)
	land.land_made.connect(_on_land_made)
	land.outline_ready.connect(_on_outline_ready)


func _let_go(land: LandformLand) -> void:
	if land == null:
		return
	for connection: Array in [
		[land.job_finished, _on_job_finished], [land.job_failed, _on_job_failed],
		[land.land_made, _on_land_made], [land.outline_ready, _on_outline_ready],
	]:
		var heard: Signal = connection[0]
		var handler: Callable = connection[1]
		if heard.is_connected(handler):
			heard.disconnect(handler)


func _on_job_finished(step: LandformLandStep) -> void:
	_delivered.append(step)


func _on_job_failed(sentence: String) -> void:
	_delivered.append(sentence)


func _on_land_made() -> void:
	_delivered.append(true)


func _on_outline_ready(outline: LandformOutline) -> void:
	_delivered.append(outline)


## The land's drawings in its present state, the drawing being edited their asked one again, and the lines showing it;
## a tool's gesture ends, the drawing it worked on gone.
func _read_drawings() -> void:
	_asked = _land.get_asked_drawing()
	_grown = _land.get_drawing()
	_working = _asked
	_changed = false
	for tool: LandformLandTool in _tools.values():
		tool.forget()
	if not _names_an_item(_selection):
		_selection = {}
	_view.show_drawing(_working, _selection)


## Asks the land for the drawing being edited; "" or the land's refusal, the drawing being edited then the land's own.
## The outline of the drawing asked, worked out before (a drag's), stays shown and said: the land asks for that very
## drawing, so the outline asked for again is the same. The ask's step is `_last_ask` (null when nothing was asked or
## the land refused it), for the action the caller makes of it.
func _ask_the_land() -> String:
	_last_ask = null
	if not _changed:
		return ""
	var known := _outline_current and _outline != null
	var step := _land.ask(_working)
	# An empty step (the drawing the land asks for already) is kept outside every action by nobody: held here, it
	# would be counted by the land's budget (stage 2's ruling R22, M1).
	_last_ask = step if step != null and not step.is_empty() else null
	var refused := ""
	if step == null:
		refused = _refused(_land.get_job_error())
	_read_drawings()
	_ask_outline(step != null and known)
	return refused


## Starts the regrowth toward what the land asks for, saying what it regrows by the latest outline.
func _regrow_waiting() -> String:
	_job_whole = _outline != null and not _outline.get_whole_reason().is_empty()
	_job_inside_ha = _outline.get_inside_ha() if _outline != null else -1.0
	return _start(JOB_REGROW, _land.start_regrow())


## A job started (or refused, `started` false): GROWING, or the land's sentence.
func _start(kind: String, started: bool) -> String:
	if not started:
		return _refused(_land.get_job_error())
	_begin(kind)
	return ""


func _begin(kind: String) -> void:
	_job_kind = kind
	_job_started_ms = Time.get_ticks_msec()
	_stopping = false
	_set_feedback(PackedStringArray())
	_state = State.GROWING


## A regrowth's step: swapped in and shown, joined to its action (made now for a regrowth that had none), what it took
## and its notes said (and the view's refusal, when it could not show it), and LAND. A step that arrives after `stop`
## (the job had ended before it was stopped), or after its action was undone, is never swapped in: the land stays as
## it was and the bar says so. An action let go by the budget changes nothing when undone, so its undo stops nothing:
## its regrowth's step is swapped in as an action of its own (stage 2's ruling R19, N4).
func _finished(step: LandformLandStep) -> void:
	var stopped := _stopping
	var action := _growing_action
	_growing_action = null
	if action != null:
		action.set_growing(false)
	_end_job()
	if stopped:
		_set_feedback(PackedStringArray([REGROWTH_STOPPED, AS_IT_WAS]))
		_state = State.LAND
		changed.emit()
		return
	if action != null and not action.is_done():
		if action.is_kept():
			_set_feedback(PackedStringArray([REGROWTH_DISCARDED]))
			_state = State.LAND
			changed.emit()
			return
		# An action let go (to keep the history within its budget) changes nothing when undone, so its undo stopped
		# nothing and said so: the regrowth lands, as an action of its own (stage 2's ruling R19, N4).
		action = null
	var swapped := _land.swap(step)
	if not swapped.is_empty():
		_fail(PackedStringArray([swapped, AS_IT_WAS]))
		return
	_note_world()
	if action == null:
		var none: Array[LandformLandStep] = []
		action = _act(ACTION_REGROW, none)
	action.add_step(step)
	if step.get_kind() == JOB_AREA:
		# The area regrown as drawn is cleared once its regrowth is in.
		_area = null
		_view.show_area(null)
	var refused := _show_the_land()
	_set_feedback(PackedStringArray([TOOK % _seconds(Time.get_ticks_msec() - _job_started_ms)]))
	if step.get_inside_ha() > 0.0:
		_said.append(STEP_INSIDE % ha_text(step.get_inside_ha()))
	_notes_after = _said.size()
	_regrow_notes = step.get_notes()
	if not refused.is_empty():
		_said.append(refused)
	_state = State.LAND
	_read_drawings()
	_ask_outline()
	_keep_within_budget()
	changed.emit()


## A job's end without a step: a stop (the session's, or a reload's, in the extension's words) moves back to the land
## as it was (or to no land), anything else to FAILED; a regrowth's action holds what it held before.
func _failed(sentence: String) -> void:
	var was_new := _job_kind == JOB_NEW
	var by_reload := sentence == RELOAD_STOPPED or sentence == RELOAD_STOPPED_NEW
	var stopped := _stopping or by_reload
	if _growing_action != null:
		_growing_action.set_growing(false)
		_growing_action = null
	_end_job()
	if was_new:
		_let_go(_land)
		_land = null
	var after := NO_LAND_MADE if was_new else AS_IT_WAS
	if stopped:
		_state = State.LAND if _has_land() else State.NO_LAND
		_set_feedback(PackedStringArray([sentence, after]))
		if by_reload and _has_land():
			# The reload let the land's steps and its outlines go: said first, and the outline asked for again.
			_check_restored()
			_set_feedback(PackedStringArray([RELOADED, sentence, after]))
			_read_drawings()
			_ask_outline()
		changed.emit()
		return
	_fail(PackedStringArray([sentence, after]))


## A new land grown: written to its file, named by the view's land file, and shown. One that arrives after `stop` (it
## had grown before it was stopped) is let go, no land made.
func _made() -> void:
	var stopped := _stopping
	_end_job()
	if stopped:
		_let_go(_land)
		_land = null
		_set_feedback(PackedStringArray([NEW_LAND_STOPPED, NO_LAND_MADE]))
		_state = State.NO_LAND
		changed.emit()
		return
	var written := _land.save_file(_new_file)
	if not written.is_empty():
		_let_go(_land)
		_land = null
		_fail(PackedStringArray([written, NO_LAND_MADE]))
		return
	_view.land_file = _new_file
	_file_tried = _new_file
	_scene_unmarked = true
	land_file_set.emit(_new_file)
	var shown := _view.show_land(_land, _new_file)
	if not shown.is_empty():
		_let_go(_land)
		_land = null
		_read_error = shown
		_fail(PackedStringArray([shown, WRITTEN_NOT_SHOWN % _new_file]))
		return
	_set_feedback(PackedStringArray([MADE_TOOK % [_seconds(Time.get_ticks_msec() - _job_started_ms), _new_file]]))
	_state = State.LAND
	_note_world()
	_read_drawings()
	_ask_outline()
	changed.emit()


func _end_job() -> void:
	_job_kind = ""
	_stopping = false


# The undo history.


## One action of the history named `action_name`, holding these steps, already done; `growing` when its regrowth has
## started, its step to come. The history's steps are then kept within the budget.
func _act(action_name: String, steps: Array[LandformLandStep], growing := false) -> LandformLandAction:
	var action := LandformLandAction.new(self, action_name, steps)
	_actions.append(weakref(action))
	if growing:
		action.set_growing(true)
		_growing_action = action
	_history.add(action, _view)
	_keep_within_budget()
	return action


## The session's actions the history still holds, oldest first; those it dropped are forgotten.
func _alive_actions() -> Array[LandformLandAction]:
	var alive: Array[LandformLandAction] = []
	var refs: Array[WeakRef] = []
	for ref: WeakRef in _actions:
		var action := ref.get_ref() as LandformLandAction
		if action != null:
			alive.append(action)
			refs.append(ref)
	_actions = refs
	return alive


## Keeps the land's steps within the settings' budget (the stage 2 plan's decision 4), whole actions at a time: from
## the newest action back, those whose steps fit within the budget together are kept, and the first that does not is
## let go with every older one (LandformLand.keep_steps_within of what the kept ones hold, so the land's own rule lets
## go exactly their steps). The bar says how many were let go and how far undo then reaches.
func _keep_within_budget() -> void:
	if not _has_land():
		return
	var budget := maxi(settings.undo_budget_mb, 0) * BYTES_PER_MB
	var actions := _alive_actions()
	var kept := 0
	var over := false
	var let_go := 0
	for i: int in range(actions.size() - 1, -1, -1):
		var action := actions[i]
		if not action.is_kept():
			# Let go before, and so is every action older than it.
			over = true
			continue
		var bytes := action.get_bytes()
		if not over and kept + bytes <= budget:
			kept += bytes
			continue
		over = true
		action.let_go(LandformLandAction.LET_GO_BUDGET)
		let_go += 1
	_land.keep_steps_within(kept)
	if let_go == 0:
		return
	var mb := number_text(settings.undo_budget_mb, 0)
	var reach := _steps_text(_undo_reach())
	# Said after what the call said, in place of what the last letting go said, so the line never grows by it.
	var earlier := _said.find(_let_go_said)
	if earlier >= 0:
		_said.remove_at(earlier)
	_let_go_said = LET_GO_ONE % [mb, reach] if let_go == 1 else LET_GO_MANY % [number_text(let_go, 0), mb, reach]
	_said.append(_let_go_said)


## How many of the session's actions undo reaches back through: those done and kept.
func _undo_reach() -> int:
	var reach := 0
	for action: LandformLandAction in _alive_actions():
		if action.is_done() and action.is_kept():
			reach += 1
	return reach


## A count of steps as said: "no step", "1 step", "3 steps".
static func _steps_text(count: int) -> String:
	if count == 0:
		return STEPS_NONE
	return STEPS_ONE if count == 1 else STEPS_MANY % number_text(count, 0)


## Why an action changes nothing any more (a LandformLandAction.LET_GO_* word), as the bar says it.
func _why_let_go(why: String) -> String:
	match why:
		LandformLandAction.LET_GO_BUDGET:
			return WHY_BUDGET % number_text(settings.undo_budget_mb, 0)
		LandformLandAction.LET_GO_RELOAD:
			return WHY_RELOAD
	return WHY_OTHER_LAND


## Notes the land's world as the session now sees it, after a land taken, made, or swapped by the session itself.
func _note_world() -> void:
	var world := _land.get_world() if _land != null else null
	_world_id = world.get_instance_id() if world != null else 0


## Whether the land was restored since the session last saw it (its world is another object, which only a restore
## makes: a reload of the extension), and if so says so and lets its actions go.
func _check_restored() -> bool:
	if _land == null or _world_id == 0:
		return false
	var world := _land.get_world()
	if world == null or world.get_instance_id() == _world_id:
		return false
	_reloaded()
	return true


## Landform was reloaded (the stage 2 plan's decision 6): the land was restored bit for bit, what waits among it, and
## its steps were let go, so every action of the session changes nothing from now on; the view shows the land's new
## world. A regrowth that ran was stopped by the reload: the land's next poll says so, a stop, and the bar then says
## the reload too; otherwise the bar says it now, and the outline is asked for again.
func _reloaded() -> void:
	for action: LandformLandAction in _alive_actions():
		action.let_go(LandformLandAction.LET_GO_RELOAD)
	if _growing_action != null:
		_growing_action.set_growing(false)
		_growing_action = null
	_note_world()
	_outline = null
	_outline_current = false
	if _state == State.GROWING:
		_stopping = true
	else:
		_set_feedback(PackedStringArray([RELOADED]))
		if _river_being_drawn():
			_said.append(RIVER_DROPPED)
		_read_drawings()
		_ask_outline()
	var not_shown := _show_the_land()
	if not not_shown.is_empty():
		_said.append(not_shown)
	changed.emit()


func _on_history_changed() -> void:
	changed.emit()


## Whether the River tool is drawing a river, which reading the drawings again drops.
func _river_being_drawn() -> bool:
	var river := _tools.get(TOOLS[2]) as LandformRiverTool
	return river != null and river.get_gesture() == LandformRiverTool.DRAW_RIVER


func _fail(sentences: PackedStringArray) -> void:
	_failure = sentences
	_set_feedback(PackedStringArray())
	_state = State.FAILED
	changed.emit()


func _leave_failed() -> void:
	if _state != State.FAILED:
		return
	_failure = PackedStringArray()
	_state = State.LAND if _has_land() else State.NO_LAND


## A call's refusal, said in the bar.
func _refused(sentence: String) -> String:
	_set_feedback(PackedStringArray([sentence]))
	return sentence


## A call's refusal, said in the bar at once: the panel and the bar are shown again.
func _refused_now(sentence: String) -> String:
	_refused(sentence)
	changed.emit()
	return sentence


## Refreshes the view, now or once it is back in the tree (a scene not shown is out of the tree); "" or the view's
## refusal.
func _show_the_land() -> String:
	if _view.is_inside_tree():
		return _view.refresh()
	_refresh_waits = true
	return ""


# The outline.


## Whether the view's land_file names another file than the land shown came from while that land has changes not
## saved or a change of its drawing goes on (a gesture), so the view does not open it yet.
func _file_waits() -> bool:
	return _has_land() and _view.land_file != _view.get_land_path() and (not _land.is_saved() or _changed)


## Why the view does not open the file its land_file names, while it does not (_file_waits): a file tried that did
## not load is opened again only once named again; else the changes not saved, or the change going on.
func _file_waits_sentence() -> String:
	var named := _view.land_file
	if named == _file_tried:
		return FILE_NOT_LOADED % named
	if not _land.is_saved():
		return FILE_WAITS % [named, _view.get_land_path()]
	return FILE_WAITS_CHANGE % named


## Whether a change waits to regrow: one not yet released, or the land's asked drawing not its grown one.
func _waits() -> bool:
	return _changed or _land.is_waiting()


## Asks the land for the outline of regrowing the area painted as drawn while one is; else of regrowing toward the
## drawing being edited, when a change waits and no seed was typed; otherwise the view shows none. A river with one
## point (one being drawn) is said instead of asked for, as the land would refuse it. With `keep_shown`, an outline
## asked for leaves the one last worked out shown until it arrives.
func _ask_outline(keep_shown := false) -> void:
	_outline_refused = ""
	var asked := false
	if _has_land() and _area != null:
		if _land.start_area_outline(_area):
			asked = true
		else:
			_outline_refused = _land.get_outline_error()
	elif _has_land() and _pending_seed < 0 and _waits():
		var short := _river_of_one_point(_working)
		if short >= 0:
			_outline_refused = RIVER_NEEDS_POINT % (short + 1)
		elif _land.start_outline(_working):
			asked = true
		else:
			_outline_refused = _land.get_outline_error()
	if asked:
		_outline_current = false
	if keep_shown and asked:
		return
	_outline = null
	_outline_current = false
	_view.show_outline(null)


## The index of the first river of the drawing with fewer than two points, or -1.
static func _river_of_one_point(drawing: LandformDrawing) -> int:
	for i: int in drawing.rivers.size():
		var river := drawing.rivers[i] as LandformRiver
		if river != null and point_count(river.points_m) < RIVER_POINTS_LEAST:
			return i
	return -1


## How many points a river's `points_m` holds, NUMBERS_PER_POINT numbers each; an odd number at the end is no point.
@warning_ignore("integer_division")
static func point_count(points_m: PackedFloat64Array) -> int:
	return points_m.size() / NUMBERS_PER_POINT


## An outline delivered: shown and said; the latest asked for when no newer request waits. One that arrives while a
## regrowth toward the drawing runs and says nothing of it yet gives the growing sentence its area.
func _outline_arrived(outline: LandformOutline) -> void:
	if not _has_land():
		return
	_outline = outline
	_outline_current = not _land.is_outlining()
	if _state == State.GROWING and _job_kind == JOB_REGROW and _job_inside_ha < 0.0 and not _job_whole:
		_job_whole = not outline.get_whole_reason().is_empty()
		_job_inside_ha = outline.get_inside_ha()
	_view.show_outline(outline)
	changed.emit()


func _land_sentences() -> PackedStringArray:
	if _area != null:
		return _area_sentences()
	if _pending_seed >= 0:
		return PackedStringArray([WHOLE_LAND % SEED_CHANGED])
	if not _waits():
		return PackedStringArray()
	if not _outline_refused.is_empty():
		return PackedStringArray([_outline_refused])
	if _outline == null:
		return PackedStringArray([WORKING_OUT])
	if not _outline.get_refusal().is_empty():
		return PackedStringArray([_outline.get_refusal()])
	var reason := _outline.get_whole_reason()
	if not reason.is_empty():
		return PackedStringArray([WHOLE_LAND % reason])
	if _outline.is_empty():
		return PackedStringArray([CHANGES_NOTHING])
	return PackedStringArray([inside_text(CHANGES_INSIDE, CHANGES_UNDER, _outline.get_inside_ha()), held_text(_outline.get_fixed_ha())])


## What regrowing the area painted would change: the outline's area (the area's, with a lake it touches whole) and the
## held ground inside it.
func _area_sentences() -> PackedStringArray:
	var said := _area_outline_sentences()
	# A change of the drawing behind the area waits for Regrow after it (stage 2's ruling R21, S2).
	if _waits():
		said.append(CHANGE_WAITS_AFTER_AREA)
	return said


func _area_outline_sentences() -> PackedStringArray:
	if not _outline_refused.is_empty():
		return PackedStringArray([_outline_refused])
	if _outline == null:
		return PackedStringArray([WORKING_OUT])
	if not _outline.get_refusal().is_empty():
		return PackedStringArray([_outline.get_refusal()])
	if _outline.is_empty():
		return PackedStringArray([AREA_CHANGES_NOTHING])
	return PackedStringArray([AREA_CHANGES % ha_text(_outline.get_inside_ha()), held_text(_outline.get_fixed_ha())])


## What the outline says of the held ground inside it, its first letter a capital (stage 2's ruling R18, S2).
static func held_text(held_ha: float) -> String:
	return NONE_HELD if held_ha <= 0.0 else capital(SOME_HELD % ha_text(held_ha))


## A sentence of an area inside the outline: `inside` with "the" and the area, or `under`, without it, for an area
## under HA_LEAST_SAID (stage 2's ruling R18, S2); the extra values follow the area.
static func inside_text(inside: String, under: String, ha: float, extra: Array = []) -> String:
	if ha > 0.0 and ha < HA_LEAST_SAID:
		return under % ([number_text(HA_LEAST_SAID, 1)] + extra)
	return inside % ([ha_text(ha)] + extra)


## The sentence with its first letter a capital.
static func capital(sentence: String) -> String:
	return sentence.left(1).to_upper() + sentence.substr(1)


## The refusal the latest outline says, when it is of the drawing or area as it is now: the regrowth's own sentence,
## which Regrow then says without running (stage 2's ruling R18, N2); "" otherwise.
func _outline_refusal() -> String:
	if _outline == null or not _outline_current:
		return ""
	return _outline.get_refusal()


## What a Hold stroke's step did, in one sentence: the cells whose mark it changed and the lakes it held or lifted
## whole, named by their places in the drawing being edited (the grown drawing's place when no item of it grew as
## that lake).
func _held_sentence(step: LandformLandStep, on: bool) -> String:
	var cells := step.get_cells_changed()
	if cells == 0:
		return HELD_NOTHING if on else LIFTED_NOTHING
	var counted := (ONE_CELL if cells == 1 else CELLS) % number_text(cells, 0)
	var whole := ""
	var lakes := step.get_lakes_whole()
	if not lakes.is_empty():
		# Lakes by their places in the drawing being edited, as the panel lists them; one it has no place for (moved or
		# removed there) is "a lake" (stage 2's ruling R18, N10).
		var places := PackedStringArray()
		var unplaced := PackedStringArray()
		for grown: int in lakes:
			var place := _lake_place(grown)
			if place >= 0:
				places.append(str(place + 1))
			else:
				unplaced.append(A_LAKE)
		var named := PackedStringArray()
		if places.size() == 1:
			named.append(ONE_LAKE % int(places[0]))
		elif places.size() > 1:
			named.append(LAKES % _listed(places))
		named.append_array(unplaced)
		whole = WITH_WHOLE % _listed(named)
	return (HELD_CELLS if on else LIFTED_CELLS) % [counted, whole]


## The place in the drawing being edited of the lake that grew as the grown drawing's lake `grown`, or -1 when none
## did (the drawing being edited moved or removed it).
func _lake_place(grown: int) -> int:
	for k: int in _working.lakes.size():
		if _partner(LAKE, k) == grown:
			return k
	return -1


func _growing_sentences() -> PackedStringArray:
	var time := _seconds(_land.get_busy_ms() if _land != null else 0.0)
	var growing := ""
	match _job_kind:
		JOB_NEW:
			growing = GROWING_NEW % [number_text(_job_size_m, 0), _job_seed, time]
		JOB_SEED:
			growing = GROWING_SEED % [_job_seed, time]
		JOB_AREA:
			growing = GROWING_AREA % time
		_:
			if _job_whole:
				growing = GROWING_WHOLE % time
			elif _job_inside_ha >= 0.0:
				growing = inside_text(GROWING_INSIDE, GROWING_UNDER, _job_inside_ha, [time])
			else:
				growing = GROWING_LAND % time
	var key := _stop_text()
	return PackedStringArray([growing, STOP_BUTTON_STOPS_IT if key.is_empty() else STOPS_IT % key])


func _is_delete(event: InputEvent) -> bool:
	var key := event as InputEventKey
	return key != null and key.pressed and not key.echo and key.keycode == DELETE_KEY


func _is_stop(event: InputEvent) -> bool:
	return settings.stop_shortcut != null and event.is_pressed() and not event.is_echo() and settings.stop_shortcut.matches_event(event)


## The greatest radius of a brush: half the land's side.
func _brush_radius_most() -> float:
	return _land.get_size() / 2.0 if _has_land() else INF


## The name of the stop shortcut's key, "Escape" as shipped, or "" without one.
func _stop_text() -> String:
	return settings.stop_shortcut.get_as_text() if settings.stop_shortcut != null else ""


## The name of the settings' modifier that turns a brush's stroke over, "Ctrl" as shipped.
func _modifier_text() -> String:
	var modifier := int(settings.brush_invert_modifier)
	if MODIFIER_KEYS.has(modifier):
		return OS.get_keycode_string(MODIFIER_KEYS[modifier])
	return OS.get_keycode_string(modifier).trim_suffix("+")


## The name of a key as said: a printable one its own character ([ and ]), any other Godot's name of it.
static func _key_text(key: Key) -> String:
	if key >= PRINTABLE_LEAST and key <= PRINTABLE_MOST:
		return String.chr(key)
	return OS.get_keycode_string(key)


# The drawing and what grew of it.


## Whether the drawing being edited asks for another country than the land was grown in: a change of the whole land.
func _country_changed() -> bool:
	return not _same_country(_working.country, _grown.country)


## The grown drawing's item that the working drawing's item `index` of `kind` grew as, or -1: the land's partner
## (`get_grown_partner`) while the item is the land's asked one, none for an item changed and not yet asked for.
func _partner(kind: String, index: int) -> int:
	if _changed:
		var asked: Array = _asked.rivers if kind == RIVER else _asked.lakes
		var working: Array = _working.rivers if kind == RIVER else _working.lakes
		if index >= asked.size() or not _same_item(working[index], asked[index]):
			return -1
	return _land.get_grown_partner(kind, index)


static func _same_item(a: Resource, b: Resource) -> bool:
	if a is LandformRiver and b is LandformRiver:
		var river_a := a as LandformRiver
		var river_b := b as LandformRiver
		return river_a.points_m == river_b.points_m and river_a.width_m == river_b.width_m
	if a is LandformLake and b is LandformLake:
		var lake_a := a as LandformLake
		var lake_b := b as LandformLake
		return lake_a.x_m == lake_b.x_m and lake_a.z_m == lake_b.z_m and lake_a.radius_m == lake_b.radius_m
	return false


## Whether two countries hold the same numbers, none being hill country, as the core reads a drawing without one.
static func _same_country(a: LandformCountry, b: LandformCountry) -> bool:
	var first := a if a != null else LandformCountry.new()
	var second := b if b != null else LandformCountry.new()
	for property: Dictionary in ClassDB.class_get_property_list("LandformCountry", true):
		if int(property["usage"]) & PROPERTY_USAGE_STORAGE and first.get(property["name"]) != second.get(property["name"]):
			return false
	return true


## The kind of country these numbers are, or "" for numbers of their own.
static func _kind_of(country: LandformCountry) -> String:
	for kind: String in LandformCountry.get_names():
		if _same_country(country, LandformCountry.named(kind)):
			return kind
	return ""


## The least and the greatest width the grown drawing's river `drawn` grew to, over every course of it; empty when it
## has none.
static func _grown_widths(world: LandformWorld, drawn: int) -> PackedFloat32Array:
	var least := INF
	var most := -INF
	for course: int in world.get_river_count():
		if world.get_river_drawn(course) != drawn:
			continue
		for width: float in world.get_river_widths(course):
			least = minf(least, width)
			most = maxf(most, width)
	return PackedFloat32Array() if least > most else PackedFloat32Array([least, most])


## The width the grown drawing's river `drawn` grew to at its course point nearest (east, south).
static func _width_here(world: LandformWorld, drawn: int, east: float, south: float) -> String:
	var nearest := INF
	var width := NAN
	for course: int in world.get_river_count():
		if world.get_river_drawn(course) != drawn:
			continue
		var points := world.get_river_points(course)
		var widths := world.get_river_widths(course)
		for p: int in points.size():
			var apart := Vector2(points[p].x - east, points[p].z - south).length_squared()
			if apart < nearest:
				nearest = apart
				width = widths[p]
	return NOT_GROWN if is_nan(width) else WIDTH_HERE % _tenths(width)


## The world's lake grown from the grown drawing's lake `drawn`, or -1 when the ground did not hold it.
static func _grown_lake(world: LandformWorld, drawn: int) -> int:
	for lake: int in world.get_lake_count():
		if world.get_lake_drawn(lake) == drawn:
			return lake
	return -1


## The land's notes about `subject` ("land", "river N", "lake N", N in the grown drawing).
func _notes_about(subject: String) -> PackedStringArray:
	var world := _land.get_world()
	var notes := world.get_notes()
	var subjects := world.get_note_subjects()
	var about := PackedStringArray()
	for n: int in mini(notes.size(), subjects.size()):
		if subjects[n] == subject:
			about.append(notes[n])
	return about


func _item(kind: String, index: int, item_name: String, drawn: String, grown: String, partner: int) -> Dictionary:
	var selected := _selection.has(kind) and int(_selection[kind]) == index
	return {
		"kind": kind, "index": index, "name": item_name, "drawn": drawn, "grown": grown,
		"waiting": partner < 0, "selected": selected,
		"notes": _notes_about("%s %d" % [kind, partner]) if partner >= 0 else PackedStringArray(),
	}


## Whether a selection names an item of the drawing being edited (and a point of its river), or is empty.
func _names_an_item(selection: Dictionary) -> bool:
	if selection.is_empty():
		return true
	if _working == null:
		return false
	if selection.has(LAKE):
		var k: int = selection[LAKE]
		return selection.size() == 1 and k >= 0 and k < _working.lakes.size()
	if not selection.has(RIVER):
		return false
	var i: int = selection[RIVER]
	if i < 0 or i >= _working.rivers.size():
		return false
	if not selection.has(POINT):
		return selection.size() == 1
	var j: int = selection[POINT]
	return selection.size() == 2 and j >= 0 and 2 * j + 1 < (_working.rivers[i] as LandformRiver).points_m.size()


## The water rules' wander of a river's width, of the view's config (the default without one).
func _width_wander() -> float:
	var config := _view.config
	if config != null and config.water_rules != null:
		return config.water_rules.width_wander
	return LandformWaterRules.new().width_wander


## A copy of a drawing whose lists can be changed without changing the drawing's; the items are shared, so an item
## changed is a copy put in its place (the tools' and the panel's changes all are).
static func copy_drawing(drawing: LandformDrawing) -> LandformDrawing:
	var copy := drawing.duplicate(false) as LandformDrawing
	copy.rivers = drawing.rivers.duplicate()
	copy.lakes = drawing.lakes.duplicate()
	return copy


## Why a new land cannot be made so, or "".
func _new_land_refusal(size_m: int, country_name: String, land_seed: int, path: String) -> String:
	if not size_m in settings.new_land_sizes_m:
		var sizes := PackedStringArray()
		for offered: int in settings.new_land_sizes_m:
			sizes.append(number_text(offered, 0))
		return SIZE_NOT_OFFERED % [number_text(size_m, 0), _listed(sizes)]
	if not country_name in LandformCountry.get_names():
		return NO_SUCH_COUNTRY % [country_name, _listed(LandformCountry.get_names())]
	if land_seed < 0 or land_seed > SEED_MOST:
		return NO_SUCH_SEED % [land_seed, SEED_MOST]
	if path.strip_edges().is_empty() or path.get_extension() != LAND_FILE_EXTENSION:
		return NO_FILE
	if FileAccess.file_exists(path):
		return FILE_STANDS % path
	if _view.look == null:
		return NO_LOOK
	if _view.config == null:
		return NO_CONFIG
	return ""


# Numbers as the panel and the bar say them.


## An area in hectares: whole from `HA_WHOLE_FROM` on, else with one decimal, and "under 0.1" above 0 below it.
static func ha_text(ha: float) -> String:
	if ha >= HA_WHOLE_FROM:
		return number_text(roundf(ha), 0)
	if ha > 0.0 and ha < HA_LEAST_SAID:
		return "under %s" % number_text(HA_LEAST_SAID, 1)
	return number_text(ha, 1)


## Milliseconds as seconds with one decimal.
static func _seconds(ms: float) -> String:
	return number_text(ms / MS_PER_S, 1)


## Metres with one decimal.
static func _tenths(value: float) -> String:
	return number_text(value, 1)


## A number as a typed field of the panel shows it: FIELD_DECIMALS decimals without the zeros that end them, a whole
## number without its point (String.num keeps one zero after it: stage 2's ruling R18, N5), no comma between thousands
## and no sign on 0, so the text typed back gives the number shown: 922.99, -153.6, 8.3, 20.
static func field_text(value: float) -> String:
	var text := String.num(value, FIELD_DECIMALS)
	if text.ends_with(".0"):
		text = text.trim_suffix(".0")
	return text.trim_prefix("-") if text.to_float() == 0.0 else text


## A drawn number to two decimals, without the zeros that end it: 16, 4.5, 12.25.
static func short_text(value: float) -> String:
	return number_text(value, 2).rstrip("0").rstrip(".")


## A number with `decimals` decimals and a comma between thousands: 2,048, 1,677.7.
static func number_text(value: float, decimals: int) -> String:
	var text := String.num(absf(value), decimals)
	if decimals > 0 and not text.contains("."):
		text += "." + "0".repeat(decimals)
	var whole := text.get_slice(".", 0)
	var rest := text.substr(whole.length())
	var grouped := ""
	while whole.length() > 3:
		grouped = "," + whole.right(3) + grouped
		whole = whole.left(whole.length() - 3)
	# A number that rounds to 0 is said without a sign.
	return ("-" if value < 0.0 and text.to_float() != 0.0 else "") + whole + grouped + rest


## Words joined as a list: "a", "a and b", "a, b and c".
static func _listed(words: PackedStringArray) -> String:
	if words.size() <= 1:
		return "".join(words)
	return ", ".join(words.slice(0, words.size() - 1)) + " and " + words[words.size() - 1]
