@tool
class_name LandformLandHistory
extends RefCounted
## The seam between the editor tool and an undo history (the stage 2 plan's Task 13): the session adds each action
## through `add`, already done (LandformLandAction), and the bar's Undo undoes through `undo` and names what it undoes
## through `get_undo_name`. This one keeps the actions in Godot's plain UndoRedo, given (the game's tests) or its own
## (`of_its_own`, a session made without the editor); the plugin's keeps them in the editor's history
## (EditorUndoRedoManager, `create_action` with the land's view as its context, `add_do_method`, `add_undo_method`,
## `commit_action(false)`, the action being done already), which Ctrl+Z and Ctrl+Shift+Z undo and redo. The session
## never calls the editor's API itself.

## The history's actions, or its place among them, changed (an action added, undone or redone): what the bar's Undo
## names may have changed.
signal changed()

## The input action of a plain history's undo key (Godot's own, Ctrl+Z).
const UNDO_ACTION := &"ui_undo"

var _undo_redo: UndoRedo
## Whether the UndoRedo is the history's own, freed with it.
var _own := false
## What hears the UndoRedo's version_changed, kept so the history being freed can stop hearing it (a method of the
## history cannot be named while it is freed).
var _heard: Callable


## A history in this UndoRedo, which the caller keeps and frees; null for none (a subclass keeps its actions elsewhere).
func _init(undo_redo: UndoRedo = null) -> void:
	_undo_redo = undo_redo
	_heard = _on_version_changed
	if _undo_redo != null:
		_undo_redo.version_changed.connect(_heard)


## A history in an UndoRedo of its own, freed with the history (a session made without one).
static func of_its_own() -> LandformLandHistory:
	var history := LandformLandHistory.new(UndoRedo.new())
	history._own = true
	return history


func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE and _own and is_instance_valid(_undo_redo):
		# Freed, the UndoRedo clears its history and says so, which this history, being freed, no longer hears.
		_undo_redo.version_changed.disconnect(_heard)
		_undo_redo.free()


## Adds `action`, already done, as one action of the history, named by the action, in the history of `context` (the
## land's view; the plain UndoRedo has one history for all): its undo is the action's `undo`, its redo its `redo`.
func add(action: LandformLandAction, _context: Object) -> void:
	_undo_redo.create_action(action.get_action_name())
	_undo_redo.add_do_method(action.redo)
	_undo_redo.add_undo_method(action.undo)
	# The commit emits the UndoRedo's version_changed, and so `changed`.
	_undo_redo.commit_action(false)


## Whether `event` is this history's undo key pressed: the project's `ui_undo` action (Ctrl+Z) for a plain UndoRedo,
## the editor's own Undo shortcut in the plugin's; a tool drawing a river takes it back a point instead (stage 2's
## ruling R21, S1).
func is_undo(event: InputEvent) -> bool:
	return event.is_action_pressed(UNDO_ACTION, false, true)


## Undoes the history's newest action done, as the editor's Undo does; returns whether there was one.
func undo() -> bool:
	return _undo_redo.undo()


## Redoes the history's oldest action undone; returns whether there was one.
func redo() -> bool:
	return _undo_redo.redo()


## The name of the action an undo would undo in the history of `context`, "" when there is none.
func get_undo_name(_context: Object) -> String:
	return _undo_redo.get_current_action_name() if _undo_redo.has_undo() else ""


func _on_version_changed() -> void:
	changed.emit()
