//! Landform for Godot: a thin binding that turns a seed into a world, in the foreground or on a
//! background thread, reads a land kept as a land file, grows and edits it on background threads
//! and works out an edit's outline there while a person drags, hands the world to Terrain3D, and
//! composes the picture the editor tool lays over a land.
//! Godot types are touched on the main thread only.

mod config;
mod drawing;
mod generator;
mod jobs;
mod land;
mod outline;
mod overlay;
mod pool;
mod rules;
mod step;
mod surface;
mod terrain3d;
mod water;
mod world;

use godot::prelude::*;

struct LandformExtension;

#[gdextension]
unsafe impl ExtensionLibrary for LandformExtension {
    fn on_stage_deinit(stage: InitStage) {
        // The last stage this library is told about before Godot unloads it. Workers first, cancelled
        // and then joined: a running job holds the pool. After the pool's threads are joined, no
        // thread runs this library's code, so it can be unloaded and reloaded.
        if stage == InitStage::Scene {
            jobs::join_workers();
            pool::shut_down();
        }
    }
}
