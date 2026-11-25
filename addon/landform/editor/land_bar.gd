@tool
class_name LandformLandBar
extends HBoxContainer
## The bar under the 3D view: what regrowing would change, what grows and how to stop it, or what failed, and what the
## last regrowth said (how long it took, its notes); "Regrow on release", Undo (naming the action it undoes, "Undo
## move", and telling in its tooltip how far undo reaches), Keep seed (while a seed typed waits), Stop and Regrow. It
## holds no rule: it shows what `LandformLandSession` says and hands each press to it.

## The sentences of a list shown one after another.
const SENTENCE_GAP := " "
## The words of Keep seed, with the land's seed.
const KEEP_SEED := "Keep seed %d"

var _session: LandformLandSession
var _walk_hint := ""


func show_walk_hint(hint: String) -> void:
	_walk_hint = hint
	_show()


func _ready() -> void:
	(%RegrowOnRelease as CheckBox).toggled.connect(_on_regrow_on_release)
	(%Undo as Button).pressed.connect(_on_undo)
	(%KeepSeed as Button).pressed.connect(_on_keep_seed)
	(%Stop as Button).pressed.connect(_on_stop)
	(%Regrow as Button).pressed.connect(_on_regrow)
	show_session(_session)


## Shows this session from now on (null shows nothing).
func show_session(session: LandformLandSession) -> void:
	if _session != null and _session.changed.is_connected(_show):
		_session.changed.disconnect(_show)
		_session.ticked.disconnect(_show)
	_session = session
	if _session != null:
		_session.changed.connect(_show)
		_session.ticked.connect(_show)
	if is_node_ready():
		_show()


func get_session() -> LandformLandSession:
	return _session


func _show() -> void:
	var state := _session.get_state() if _session != null else LandformLandSession.State.NO_LAND
	var change := %Change as Label
	change.text = SENTENCE_GAP.join(_session.get_bar_sentences()) if _session != null else ""
	change.visible = not change.text.is_empty()
	var said := %Said as Label
	# The new-land form shows its own feedback next to the file field.
	said.text = SENTENCE_GAP.join(_session.get_said_sentences(false)) if state != LandformLandSession.State.NO_LAND else ""
	if not _walk_hint.is_empty():
		said.text = _walk_hint
	said.visible = not said.text.is_empty()
	var notes := _session.get_regrow_notes() if _session != null else PackedStringArray()
	var section := %RegrowNotes as FoldableContainer
	section.visible = not notes.is_empty()
	if section.visible:
		section.title = "Regrowth notes (%d)" % notes.size()
		(%NoteText as Label).text = "\n".join(notes)
	var growing := state == LandformLandSession.State.GROWING
	var has_land := _session != null and _session.get_land() != null
	var regrow_on_release := %RegrowOnRelease as CheckBox
	regrow_on_release.set_pressed_no_signal(_session != null and _session.regrow_on_release)
	regrow_on_release.disabled = _session == null
	var undo := %Undo as Button
	undo.text = _session.get_undo_text() if _session != null else LandformLandSession.UNDO_NOTHING
	undo.disabled = _session == null or not _session.can_undo()
	undo.tooltip_text = _session.get_undo_reach() if _session != null else ""
	var keep_seed := %KeepSeed as Button
	keep_seed.visible = has_land and _session.has_pending_seed()
	if keep_seed.visible:
		keep_seed.text = KEEP_SEED % _session.get_land().get_seed()
	var stop := %Stop as Button
	stop.disabled = not growing
	stop.visible = growing
	if _session != null:
		stop.shortcut = _session.settings.stop_shortcut
		stop.shortcut_context = self
	(%Regrow as Button).disabled = _session == null or not _session.can_regrow()


func _on_regrow_on_release(on: bool) -> void:
	if _session != null:
		_session.set_regrow_on_release(on)


func _on_undo() -> void:
	if _session != null:
		_session.undo()


func _on_keep_seed() -> void:
	if _session != null:
		_session.keep_seed()


func _on_stop() -> void:
	if _session != null:
		_session.stop()


func _on_regrow() -> void:
	if _session != null:
		_session.regrow()
