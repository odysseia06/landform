#!/usr/bin/env bash
# Build the Godot extension in debug and release and copy both libraries into addon/landform/bin/,
# where landform.gdextension expects them. Usage: tools/build.sh
set -euo pipefail
cd "$(dirname "$0")/.."
: "${GDRUST_GODOT_BIN:?Set GDRUST_GODOT_BIN to the Windows path of your Godot 4.8 executable}"
target_dir=${CARGO_TARGET_DIR:-target}
bin=addon/landform/bin
mkdir -p "$bin"
cargo build --locked -p landform_godot
cargo build --locked -p landform_godot --release

# The Godot editor loads a copy of the library, but a running game or test run locks the library itself on
# Windows, so the copy can fail.
copy_library() {
	if ! cp "$1" "$2"; then
		echo "Could not copy $1 to $2: stop any running game or test run that has the extension loaded, then build again." >&2
		exit 1
	fi
}
copy_library "$target_dir/debug/landform_godot.dll" "$bin/landform_godot.windows.debug.x86_64.dll"
copy_library "$target_dir/release/landform_godot.dll" "$bin/landform_godot.windows.release.x86_64.dll"
echo "Built $bin/landform_godot.windows.debug.x86_64.dll and $bin/landform_godot.windows.release.x86_64.dll"
