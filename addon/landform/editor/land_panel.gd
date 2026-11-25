@tool
class_name LandformLandPanel
extends MarginContainer
## The Landform panel, docked on the right of the editor inside `land_dock.tscn` (an EditorDock, which Godot makes only
## in the editor, so the game's tests make the panel alone): what the session says in each state, the form that makes a
## new land, the land's numbers (its size shown; its country, seed and reach typed; its band shown; its notes), its
## rivers and lakes with what was drawn and what grew, the selected item's fields with the button that removes it, and
## while a brush is chosen its radius and strength typed and how it is used.
## It holds no rule: it shows what `LandformLandSession` says and hands what is typed, chosen or pressed to it (typing
## a number is the same edit as a drag, and Remove river or Remove lake is one step, as a drag's release is).
##
## The fields that type a number of an item or a brush are LineEdits whose text the panel writes, in the session's
## field text (LandformLandSession.field_text, two decimals at most): a field hands its number to the session only
## when its text is no longer the text the panel wrote, at Enter or when it loses focus, so entering and leaving a
## field changes nothing (stage 2's ruling R17, M1), and a number typed is the number its text parses to, kept
## exactly. A field the person is typing in is not written over. Typing exactly the text shown changes nothing either,
## even where the number is not the one the text gives (a point at 9.87654 shows "9.88": "9.88" typed keeps 9.87654);
## another text, such as "9.880", hands over its number (stage 2's ruling R18, N5).

signal frame_requested(selection: Dictionary)

## The sentences of a list shown one after another.
const SENTENCE_GAP := " "
## The words that name a brush's numbers when what is typed is no number.
const RADIUS_WORD := "radius"
const STRENGTH_WORD := "strength"

var _session: LandformLandSession
## Set while the panel fills its controls, so their signals are not taken for a person's typing.
var _filling := false
## The session whose offer the form shows: the form is filled from the offer once a session, and keeps what a person
## chose after that.
var _offered: LandformLandSession
## The text the panel last wrote into each number field, which a submission of the same text leaves as it is, and
## the word that names each field's number.
var _written: Dictionary[LineEdit, String] = {}
var _words: Dictionary[LineEdit, String] = {}
## The rows the list shows, each item's kind, index, name, drawn, grown and notes: the list is built again only when
## they change, never because a row was selected, so it is never cleared inside the Tree's own signal (a row selected
## with the keyboard frees the rows the Tree is still going through, and a click refuses the clearing).
var _rows: Array = []


func _ready() -> void:
	%MakeLand.pressed.connect(_on_make_land)
	%ChooseFile.pressed.connect(func() -> void: (%FileDialog as FileDialog).popup_centered_ratio())
	(%FileDialog as FileDialog).file_selected.connect(func(path: String) -> void: (%NewFile as LineEdit).text = path)
	%StopGrowing.pressed.connect(_on_stop)
	%Dismiss.pressed.connect(_on_dismiss)
	%SaveLand.pressed.connect(_on_save)
	%RemoveItem.pressed.connect(_on_remove)
	(%CountryChoice as OptionButton).item_selected.connect(_on_country_chosen)
	(%SeedValue as SpinBox).value_changed.connect(_on_seed_typed)
	(%ReachValue as SpinBox).value_changed.connect(_on_reach_typed)
	_take_numbers(%BrushRadius as LineEdit, _on_brush_radius_typed, RADIUS_WORD)
	_take_numbers(%BrushStrength as LineEdit, _on_brush_strength_typed, STRENGTH_WORD)
	(%Items as Tree).item_selected.connect(_on_item_selected)
	(%Items as Tree).item_activated.connect(_on_item_activated)
	%FramePoint.pressed.connect(_on_frame_point)
	for field: String in [LandformLandSession.EAST, LandformLandSession.SOUTH, LandformLandSession.WIDTH, LandformLandSession.RADIUS]:
		_take_numbers(_field(field) as LineEdit, _on_field_typed.bind(field), field)
	var items := %Items as Tree
	items.set_column_title(0, "Name")
	items.set_column_title(1, "Drawn")
	items.set_column_title(2, "Grown")
	show_session(_session)


## Shows this session from now on (null shows that no land's view is edited).
func show_session(session: LandformLandSession) -> void:
	if _session != null and _session.changed.is_connected(_show):
		_session.changed.disconnect(_show)
		_session.ticked.disconnect(_show_growing)
	_session = session
	if _session != null:
		_session.changed.connect(_show)
		_session.ticked.connect(_show_growing)
	if is_node_ready():
		_show()


func get_session() -> LandformLandSession:
	return _session


func _show() -> void:
	_filling = true
	var state := _session.get_state() if _session != null else LandformLandSession.State.NO_LAND
	var has_land := _session != null and _session.get_land() != null
	(%Unedited as Control).visible = _session == null
	(%NoLand as Control).visible = _session != null and state == LandformLandSession.State.NO_LAND
	(%Growing as Control).visible = _session != null and state == LandformLandSession.State.GROWING
	(%Failed as Control).visible = _session != null and state == LandformLandSession.State.FAILED
	(%Land as Control).visible = has_land
	if _session != null:
		(%StopGrowing as Button).shortcut = _session.settings.stop_shortcut
		(%StopGrowing as Button).shortcut_context = self
		match state:
			LandformLandSession.State.NO_LAND:
				_show_form()
			LandformLandSession.State.GROWING:
				_show_growing()
			LandformLandSession.State.FAILED:
				(%FailedText as Label).text = SENTENCE_GAP.join(_session.get_bar_sentences())
		if has_land:
			# A failure leaves the land as it was: its fields stay on, and any change leaves FAILED (stage 2's ruling
			# R21, N2).
			_show_land(state in [LandformLandSession.State.LAND, LandformLandSession.State.FAILED])
	_filling = false


func _show_form() -> void:
	(%Intro as Label).text = SENTENCE_GAP.join(_session.get_intro_sentences())
	(%FormSaid as Label).text = SENTENCE_GAP.join(_session.get_said_sentences())
	if _offered == _session:
		return
	_offered = _session
	var offer := _session.get_new_land_offer()
	var sizes := %NewSize as OptionButton
	sizes.clear()
	for size_m: int in offer["sizes"] as PackedInt32Array:
		sizes.add_item("%s m" % LandformLandSession.number_text(size_m, 0), size_m)
		if size_m == int(offer["size"]):
			sizes.select(sizes.item_count - 1)
	var countries := %NewCountry as OptionButton
	countries.clear()
	for country: String in offer["countries"] as PackedStringArray:
		countries.add_item(country.capitalize())
		countries.set_item_metadata(countries.item_count - 1, country)
		if country == String(offer["country"]):
			countries.select(countries.item_count - 1)
	var seed_box := %NewSeed as SpinBox
	seed_box.max_value = float(offer["seed_most"])
	seed_box.set_value_no_signal(float(offer["seed"]))


func _show_growing() -> void:
	if _session == null or _session.get_state() != LandformLandSession.State.GROWING:
		return
	(%GrowingText as Label).text = SENTENCE_GAP.join(_session.get_bar_sentences())


## The land's numbers, its rivers and lakes and the selected item; typed fields only while the land is idle.
func _show_land(idle: bool) -> void:
	var facts := _session.get_land_facts()
	(%FileValue as Label).text = String(facts["file"])
	(%FileValue as Label).tooltip_text = String(facts["file"])
	(%SizeValue as Label).text = String(facts["size"])
	(%BandValue as Label).text = String(facts["band"])
	var notes := facts["notes"] as PackedStringArray
	var section := %TerrainNotes as FoldableContainer
	section.visible = not notes.is_empty()
	section.title = "Terrain notes (%d)" % notes.size()
	(%LandNotes as Label).text = "\n".join(notes)
	var hint := %ToolHint as Label
	hint.visible = idle and _session.get_tool() in [LandformLandSession.RIVER, LandformLandSession.LAKE]
	if hint.visible:
		hint.text = _session.get_tool_tip(_session.get_tool())
	(%SaveLand as Button).disabled = bool(facts["saved"])
	var countries := %CountryChoice as OptionButton
	countries.clear()
	for country: String in facts["countries"] as PackedStringArray:
		countries.add_item(country.capitalize())
		countries.set_item_metadata(countries.item_count - 1, country)
		if country == String(facts["country"]):
			countries.select(countries.item_count - 1)
	if String(facts["country"]).is_empty():
		countries.add_item("Its own numbers")
		countries.set_item_disabled(countries.item_count - 1, true)
		countries.select(countries.item_count - 1)
	countries.disabled = not idle
	var seed_box := %SeedValue as SpinBox
	seed_box.max_value = LandformLandSession.SEED_MOST
	seed_box.set_value_no_signal(float(facts["seed"]))
	seed_box.editable = idle
	var reach := %ReachValue as SpinBox
	reach.min_value = float(facts["reach_least_m"])
	reach.max_value = float(facts["reach_most_m"])
	reach.set_value_no_signal(float(facts["reach_m"]))
	reach.editable = idle
	_show_brush(idle)
	_show_items()
	_show_selected(idle)


## The chosen brush, while the tool chosen is one: its radius typed, its strength for high ground, and how it is used.
func _show_brush(idle: bool) -> void:
	var brush := _session.get_brush()
	(%Brush as Control).visible = not brush.is_empty()
	if brush.is_empty():
		return
	(%BrushTitle as Label).text = String(brush["title"])
	var radius := %BrushRadius as LineEdit
	_write(radius, float(brush["radius_m"]))
	radius.editable = idle
	var strength := %BrushStrength as LineEdit
	strength.visible = brush.has("strength")
	(%BrushStrengthLabel as Control).visible = strength.visible
	if strength.visible:
		_write(strength, float(brush["strength"]))
		strength.editable = idle
	(%BrushUses as Label).text = SENTENCE_GAP.join(brush["uses"] as PackedStringArray)


## The rivers and lakes: built again when they changed, and the row of the selected river or lake selected (none for a
## point, which the list does not show).
func _show_items() -> void:
	var tree := %Items as Tree
	var items := _session.get_items()
	tree.visible = not items.is_empty()
	(%ItemsTitle as Label).visible = tree.visible
	var rows: Array = []
	for item: Dictionary in items:
		rows.append([item["kind"], item["index"], item["name"], item["drawn"], item["grown"], item["notes"]])
	if rows != _rows or tree.get_root() == null:
		# Keep open rows open while the drawing or its grown measurements change.
		var expanded := PackedStringArray()
		if tree.get_root() != null:
			for row: TreeItem in tree.get_root().get_children():
				if row.get_child_count() > 0 and not row.collapsed:
					expanded.append(row.get_text(0))
		_rows = rows
		tree.clear()
		var root := tree.create_item()
		for item: Dictionary in items:
			var row := tree.create_item(root)
			row.set_text(0, String(item["name"]))
			row.set_text(1, String(item["drawn"]))
			row.set_text(2, String(item["grown"]))
			row.set_metadata(0, {item["kind"]: item["index"]})
			row.set_tooltip_text(0, "Double-click to frame in the 3D view.")
			for note: String in item["notes"] as PackedStringArray:
				var line := tree.create_item(row)
				line.set_text(0, note)
				line.set_autowrap_mode(0, TextServer.AUTOWRAP_WORD_SMART)
				line.set_expand_right(0, true)
				line.set_metadata(0, {item["kind"]: item["index"]})
			row.collapsed = row.get_child_count() > 0 and not row.get_text(0) in expanded
	var selection := _session.get_selection()
	var wanted: Variant = null if selection.is_empty() or selection.has(LandformLandSession.POINT) else selection
	var selected := tree.get_selected()
	# A row the person selected (or a note under it) already names the selection: nothing is selected again.
	if selected != null and selected.get_metadata(0) == wanted:
		return
	if wanted == null:
		tree.deselect_all()
		return
	for row: TreeItem in tree.get_root().get_children():
		if row.get_metadata(0) == wanted:
			row.select(0)


func _show_selected(idle: bool) -> void:
	var selected := _session.get_selected()
	(%Selected as Control).visible = not selected.is_empty()
	if selected.is_empty():
		return
	(%SelectedTitle as Label).text = String(selected["title"])
	(%PointText as Label).text = String(selected.get("point_text", ""))
	(%PointText as Label).visible = selected.has("point_text")
	(%FramePoint as Button).visible = selected.has("point_text")
	var fields := selected["fields"] as PackedStringArray
	for field: String in [LandformLandSession.EAST, LandformLandSession.SOUTH, LandformLandSession.WIDTH, LandformLandSession.RADIUS]:
		var edit := _field(field) as LineEdit
		var shown := field in fields
		edit.visible = shown
		(get_node("%" + field.capitalize() + "Label") as Control).visible = shown
		if not shown:
			continue
		edit.editable = idle
		_write(edit, float(selected[field]))
	(%Shown as Label).text = "\n".join(selected["shown"] as PackedStringArray)
	(%SelectedNotes as Label).text = SENTENCE_GAP.join(selected["notes"] as PackedStringArray)
	(%RemoveItem as Button).text = String(selected["remove"])
	(%RemoveItem as Button).disabled = not idle


func _field(field: String) -> Node:
	return get_node("%" + field.capitalize())


## Hands a number field's number to `typed` when the field is submitted (Enter) or left, if its text was changed;
## `word` names the number when its text is none.
func _take_numbers(edit: LineEdit, typed: Callable, word: String) -> void:
	_words[edit] = word
	edit.text_submitted.connect(func(_text: String) -> void: _submit(edit, typed))
	edit.focus_exited.connect(func() -> void: _submit(edit, typed))


## Writes a number into its field in the session's field text, unless the person is typing in it (it has focus and
## its text is no longer what the panel wrote).
func _write(edit: LineEdit, value: float) -> void:
	if edit.has_focus() and edit.text != _written.get(edit, edit.text):
		return
	var text := LandformLandSession.field_text(value)
	_written[edit] = text
	edit.text = text


## A number field submitted or left: its text, when it is no longer what the panel wrote, as the number it parses to;
## text that is no number is said and the field written back.
func _submit(edit: LineEdit, typed: Callable) -> void:
	if _filling or _session == null:
		return
	var text := edit.text.strip_edges()
	var written: String = _written.get(edit, "")
	if text == written:
		edit.text = written
		return
	if not text.is_valid_float():
		_session.say(LandformLandSession.NOT_A_NUMBER % _words[edit])
		edit.text = written
		return
	# The text typed counts as written, so the session's answer (its changed signal) writes the field again.
	_written[edit] = text
	typed.call(text.to_float())


func _on_make_land() -> void:
	var sizes := %NewSize as OptionButton
	var countries := %NewCountry as OptionButton
	if _session == null or sizes.selected < 0 or countries.selected < 0:
		return
	_session.make_land(sizes.get_item_id(sizes.selected), String(countries.get_item_metadata(countries.selected)),
			int((%NewSeed as SpinBox).value), (%NewFile as LineEdit).text.strip_edges())


func _on_stop() -> void:
	if _session != null:
		_session.stop()


func _on_dismiss() -> void:
	if _session != null:
		_session.dismiss()


func _on_save() -> void:
	if _session != null:
		_session.save()


func _on_remove() -> void:
	if _session != null:
		_session.remove_item()


func _on_country_chosen(index: int) -> void:
	var countries := %CountryChoice as OptionButton
	var chosen: Variant = countries.get_item_metadata(index)
	if _filling or _session == null or chosen == null:
		return
	_session.set_country(String(chosen))


func _on_seed_typed(value: float) -> void:
	if not _filling and _session != null:
		_session.set_seed(int(value))


func _on_reach_typed(value: float) -> void:
	if not _filling and _session != null:
		_session.set_reach(value)


func _on_brush_radius_typed(value: float) -> void:
	if not _filling and _session != null:
		_session.set_brush_radius(value)


func _on_brush_strength_typed(value: float) -> void:
	if not _filling and _session != null:
		_session.set_brush_strength(value)


func _on_item_selected() -> void:
	var row := (%Items as Tree).get_selected()
	if _filling or _session == null or row == null:
		return
	_session.select(row.get_metadata(0) as Dictionary)


func _on_item_activated() -> void:
	var tree := %Items as Tree
	var row := tree.get_selected()
	if not _filling and _session != null and row != null and row.get_parent() == tree.get_root():
		frame_requested.emit(row.get_metadata(0) as Dictionary)


func _on_frame_point() -> void:
	if not _filling and _session != null:
		var selection := _session.get_selection()
		if selection.has(LandformLandSession.POINT):
			frame_requested.emit(selection)


func _on_field_typed(value: float, field: String) -> void:
	if not _filling and _session != null:
		_session.type_field(field, value)
