@tool
class_name LandformLandAction
extends RefCounted
## One action of the editor tool's undo history: one thing a person did to a land (a gesture's release, its ask and the
## regrowth "Regrow on release" starts with it; a Regrow, a regrowth from a seed or of an area; a Hold stroke; a setting,
## the reach, the kind of country or a number typed). It holds the land's steps (LandformLandStep) in the order they
## were done: `redo` swaps them into the land in that order and `undo` in reverse (LandformLand.swap), so the land comes
## back bit for bit. An action whose regrowth still runs holds its ask's step at once and its regrowth's step when the
## regrowth ends while the action is done (`add_step`); its `undo` first stops a regrowth still running and waits for
## it (LandformLandSession.begin_history, LandformLand.stop_and_wait), and a regrowth that ends after its action was
## undone is discarded, never swapped in, unless the action was let go: its undo then changed nothing and stopped
## nothing, and the regrowth lands as an action of its own (stage 2's ruling R19, N4).
##
## The history (Godot's UndoRedo, or the editor's EditorUndoRedoManager, through LandformLandHistory) holds the action,
## and through it its steps, so the steps live as long as the history keeps the action and are freed when it lets the
## action go. The action holds its session weakly, and each undo and redo goes through the session, which stops what
## grows first and afterwards shows the land as it now is and says what was done. An action whose steps were let go
## (to keep the history within its budget, by a reload of the extension, or because the view shows another land)
## changes nothing when undone or redone, and the session says why (`get_let_go`).

## Why an action changes nothing any more (`get_let_go`): kept; its steps were let go to keep the history within its
## budget (LandformLand.keep_steps_within); Landform was reloaded since it was done, which lets every step go; it was
## done to another land than the one the session's view shows now.
const KEPT := ""
const LET_GO_BUDGET := "budget"
const LET_GO_RELOAD := "reload"
const LET_GO_OTHER_LAND := "other land"

var _session_ref: WeakRef
var _name: String
var _steps: Array[LandformLandStep] = []
## Whether the action is done (committed, or redone) rather than undone, as the history's place among its actions says.
var _done := true
## Whether its regrowth still runs, its step not yet arrived.
var _growing := false
var _let_go := KEPT


## An action of `session` named `action_name` ("move", "regrow", "hold"), already done, holding these steps in the
## order they were done.
func _init(session: LandformLandSession, action_name: String, steps: Array[LandformLandStep]) -> void:
	_session_ref = weakref(session)
	_name = action_name
	_steps = steps.duplicate()


## What the action is called in the history and the bar ("Undo move").
func get_action_name() -> String:
	return _name


## The land's steps the action holds, in the order they were done.
func get_steps() -> Array[LandformLandStep]:
	return _steps.duplicate()


## Whether the action is done (committed or redone), not undone.
func is_done() -> bool:
	return _done


## Whether the action's regrowth still runs.
func is_growing() -> bool:
	return _growing


## Marks the action's regrowth as running, its step to come (`add_step`), or as ended.
func set_growing(on: bool) -> void:
	_growing = on


## The regrowth's step, once it is swapped in while the action is done: the action then holds it after its others.
func add_step(step: LandformLandStep) -> void:
	_steps.append(step)


## What the action's steps hold in memory, in bytes (LandformLandStep.get_bytes).
func get_bytes() -> int:
	var bytes := 0
	for step: LandformLandStep in _steps:
		bytes += step.get_bytes()
	return bytes


## Why the action changes nothing any more, one of the LET_GO_* words, or KEPT while it changes the land.
func get_let_go() -> String:
	return _let_go


## Whether undoing or redoing the action changes the land: it was not let go and every step still holds its change.
func is_kept() -> bool:
	if _let_go != KEPT:
		return false
	for step: LandformLandStep in _steps:
		if not step.is_kept():
			return false
	return true


## Lets the action go for this reason (a LET_GO_* word): its undo and redo change nothing from now on. A reason given
## first is kept.
func let_go(reason: String) -> void:
	if _let_go == KEPT:
		_let_go = reason


## Swaps the steps back, last first, through the session: the land as it was before the action.
func undo() -> void:
	_swap(false)


## Swaps the steps again, first first, through the session: the land as it was after the action.
func redo() -> void:
	_swap(true)


func _swap(forward: bool) -> void:
	_done = forward
	var session := _session_ref.get_ref() as LandformLandSession
	if session == null:
		return
	session.begin_history(self)
	var refused := ""
	if is_kept():
		refused = _swap_steps(session.get_land(), forward)
	elif _let_go == KEPT:
		# A step let go without the session's knowing: by a reload, a step rebuilt holding nothing has no kind.
		_let_go = LET_GO_RELOAD if _rebuilt() else LET_GO_BUDGET
	session.end_history(self, forward, refused)


## Swaps every step into `land` in the order given, or none: a refusal (the land empty, a step of other states) puts
## back the steps swapped before it and gives the land's sentence, the land as it was.
func _swap_steps(land: LandformLand, forward: bool) -> String:
	if land == null:
		return LandformLandSession.NO_LAND_SHOWN
	var order := _steps.duplicate()
	if not forward:
		order.reverse()
	var swapped: Array[LandformLandStep] = []
	for step: LandformLandStep in order:
		var refused := land.swap(step)
		if not refused.is_empty():
			swapped.reverse()
			for back: LandformLandStep in swapped:
				land.swap(back)
			return refused
		swapped.append(step)
	return ""


## Whether a step of the action holds nothing because a reload rebuilt it (its kind is empty).
func _rebuilt() -> bool:
	for step: LandformLandStep in _steps:
		if step.get_kind().is_empty():
			return true
	return false
