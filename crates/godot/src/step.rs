//! A step of a land (`LandformLandStep`): one thing done to a land, what lies between two of its
//! states (the core's `Change`) with what was done, which `LandformLand.swap` exchanges with the
//! land, so undo and redo are one call; and a land's steps in the order they were made (`Steps`),
//! which `LandformLand.keep_steps_within` lets go, oldest first, to keep them within a budget.
//! Godot's history holds the steps: a land reaches each step's change only weakly, so a step Godot
//! drops frees what it holds, and a step let go holds nothing more and changes nothing.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use godot::prelude::*;
use landform_core::{About, Change, EditTimings, Held};

/// Square metres in a hectare.
pub(crate) const M2_PER_HA: f64 = 10_000.0;

/// What a step does: one thing done to a land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepKind {
    /// `LandformLand.start_regrow`: the land regrown toward the drawing it asks for.
    Regrow,
    /// `start_reseed`: the same from a seed.
    Seed,
    /// `start_regrow_area`: the land's own drawing regrown as drawn on an area.
    Area,
    /// `hold` with `on`: cells held.
    Hold,
    /// `hold` without it: the hold lifted.
    Lift,
    /// `ask`: the drawing the land asks for set.
    Ask,
    /// `set_reach_m`: the reach set.
    Reach,
}

impl StepKind {
    /// The kind as `LandformLandStep.get_kind()` gives it.
    fn name(self) -> &'static str {
        match self {
            Self::Regrow => "regrow",
            Self::Seed => "seed",
            Self::Area => "area",
            Self::Hold => "hold",
            Self::Lift => "lift",
            Self::Ask => "ask",
            Self::Reach => "reach",
        }
    }
}

/// What an edit's step says of the edit, beside its change. Plain Rust: made on the job's thread.
pub(crate) struct EditSaid {
    /// The edit's own notes (`Edit::notes`) and what each is about (`Edit::notes_about`).
    pub(crate) notes: Vec<String>,
    pub(crate) notes_about: Vec<About>,
    pub(crate) timings: EditTimings,
    /// How long `Change::between` took on the job's thread, in milliseconds.
    pub(crate) between_ms: f32,
    /// The world cells inside the edit's outline, and those of them held or shaped by hand, in
    /// square metres (`Outline::inside_m2`, `Outline::fixed_inside_m2`).
    pub(crate) inside_m2: f64,
    pub(crate) fixed_inside_m2: f64,
}

/// What a step says was done, beside its change.
pub(crate) enum Said {
    /// An edit: its notes, timings and outline's area.
    Edit(Box<EditSaid>),
    /// A hold or a lift: how many cells' mark it changed and which lakes it held or lifted whole.
    Hold(Held),
    /// An ask or a reach, which say nothing more.
    Nothing,
}

/// Why a step holds its change no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LetGo {
    /// To keep the land's steps within their budget (`Steps::keep_within`).
    Budget,
    /// The land was read, restored or made anew, which lets go every step of the land it held
    /// (`Steps::let_all_go`).
    NewLand,
}

/// A step's change, where both the step and its land's list of steps reach it: `None` once let go,
/// with why.
pub(crate) struct Slot {
    pub(crate) change: Option<Change>,
    /// Why the change was let go; read only once `change` is `None`.
    pub(crate) let_go: LetGo,
}

type Shared = Arc<Mutex<Slot>>;

/// A step's slot, locked.
fn locked(slot: &Mutex<Slot>) -> MutexGuard<'_, Slot> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a step holds.
pub(crate) struct Contents {
    kind: StepKind,
    /// The two states of its land the step lies between (`LandformLand`'s state ids); one state
    /// twice when its change is empty, so it moves the land nowhere.
    states: [u64; 2],
    slot: Shared,
    said: Said,
}

impl Contents {
    /// The state of the two the step lies between that is not `state`, when `state` is one of them:
    /// where a swap moves a land at `state`.
    pub(crate) fn other_state(&self, state: u64) -> Option<u64> {
        let [one, two] = self.states;
        if state == one {
            Some(two)
        } else if state == two {
            Some(one)
        } else {
            None
        }
    }

    /// The step's slot, locked: its change, `None` once it was let go, and why.
    pub(crate) fn slot(&self) -> MutexGuard<'_, Slot> {
        locked(&self.slot)
    }
}

/// One thing done to a land and what lies between the land before and after it: a regrowth, a
/// regrowth from a seed or of an area (a land's job gives it through `job_finished`, the land
/// unchanged until it is swapped in), a hold, a lift, an ask or a reach set (given by the call
/// that did it, already done). The step lies between two states of its land, and
/// `LandformLand.swap(step)` moves the land from either to the other, exchanging the step's change
/// with the land, so swapping it again undoes what the swap did. Godot frees what the step holds
/// when it lets the step go, and `LandformLand.keep_steps_within` lets it go sooner (`is_kept()`),
/// after which it changes nothing. A step made by `LandformLandStep.new()`, or one an editor reload
/// of the extension rebuilt (a reload lets every step go), holds nothing: its kind is empty.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformLandStep {
    /// `None` in a step that holds nothing.
    contents: Option<Contents>,
}

impl LandformLandStep {
    /// A new step of `kind` between the land's `states`, holding `change` and `said`, recorded in
    /// the land's `steps`.
    pub(crate) fn made(kind: StepKind, states: [u64; 2], change: Change, said: Said, steps: &mut Steps) -> Gd<Self> {
        let slot = Arc::new(Mutex::new(Slot { change: Some(change), let_go: LetGo::Budget }));
        steps.add(&slot);
        Gd::from_object(Self { contents: Some(Contents { kind, states, slot, said }) })
    }

    /// What the step holds, or `None` in a step that holds nothing.
    pub(crate) fn contents(&self) -> Option<&Contents> {
        self.contents.as_ref()
    }

    /// What the edit said, in a step of an edit.
    fn edit(&self) -> Option<&EditSaid> {
        match &self.contents.as_ref()?.said {
            Said::Edit(said) => Some(said),
            Said::Hold(_) | Said::Nothing => None,
        }
    }

    /// What the hold or the lift did, in a step of one.
    fn held(&self) -> Option<&Held> {
        match &self.contents.as_ref()?.said {
            Said::Hold(held) => Some(held),
            Said::Edit(_) | Said::Nothing => None,
        }
    }
}

#[godot_api]
impl LandformLandStep {
    /// What the step does: `"regrow"` (`LandformLand.start_regrow`), `"seed"` (`start_reseed`),
    /// `"area"` (`start_regrow_area`), `"hold"` or `"lift"` (`hold`), `"ask"` (`ask`) or `"reach"`
    /// (`set_reach_m`); empty in a step that holds nothing.
    #[func]
    fn get_kind(&self) -> GString {
        self.contents.as_ref().map_or_else(GString::new, |contents| GString::from(contents.kind.name()))
    }

    /// The edit's own notes, one sentence each, as the core's `Edit::notes` gives them: why a
    /// whole-land edit regrows the whole land, the troughs, the joins, the shares held, then the
    /// edited land's notes about what its outline touches. Empty but in a step of an edit.
    #[func]
    fn get_notes(&self) -> PackedStringArray {
        self.edit().map_or_else(PackedStringArray::new, |said| said.notes.iter().map(|note| GString::from(note.as_str())).collect())
    }

    /// What each of `get_notes()` is about, in its order, in the words of `LandformWorld`'s
    /// `get_note_subjects()`: `land`, `river N` or `lake N`, N counted from 0.
    #[func]
    fn get_note_subjects(&self) -> PackedStringArray {
        self.edit().map_or_else(PackedStringArray::new, |said| said.notes_about.iter().map(|about| GString::from(about.to_string().as_str())).collect())
    }

    /// How long the edit's steps took, in milliseconds, as the core's `EditTimings` gives them
    /// (`footprint_ms`, `grow_ms`, `outline_ms`, `mix_ms`, `fine_steps_ms`, `join_ms`, `total_ms`),
    /// and `between_ms`, how long working out the step took after the edit; empty but in a step of
    /// an edit.
    #[func]
    fn get_timings(&self) -> VarDictionary {
        let Some(said) = self.edit() else {
            return VarDictionary::new();
        };
        // Taken apart, so a timing added to an edit fails to compile until it is given here.
        let EditTimings { footprint_ms, grow_ms, outline_ms, mix_ms, fine_steps_ms, join_ms, total_ms } = said.timings;
        vdict! {
            "footprint_ms" => footprint_ms,
            "grow_ms" => grow_ms,
            "outline_ms" => outline_ms,
            "mix_ms" => mix_ms,
            "fine_steps_ms" => fine_steps_ms,
            "join_ms" => join_ms,
            "total_ms" => total_ms,
            "between_ms" => said.between_ms,
        }
    }

    /// The world cells inside the edit's outline, held and hand-shaped ones among them, in hectares:
    /// the whole land for a whole-land edit, 0 for an edit that changes no ground and in a step that
    /// is not an edit's.
    #[func]
    fn get_inside_ha(&self) -> f64 {
        self.edit().map_or(0.0, |said| said.inside_m2 / M2_PER_HA)
    }

    /// The world cells inside the edit's outline that are held or shaped by hand, in hectares.
    #[func]
    fn get_fixed_ha(&self) -> f64 {
        self.edit().map_or(0.0, |said| said.fixed_inside_m2 / M2_PER_HA)
    }

    /// How many cells' held mark a hold or a lift changed: those it held that were not held, or
    /// lifted that were, a lake's water among them (`Land::hold`), so a hold over ground already
    /// held counts none of that ground, where `landform land hold` says it held every cell of its
    /// disc; 0 in a step of another kind.
    #[func]
    fn get_cells_changed(&self) -> i64 {
        self.held().map_or(0, |held| i64::try_from(held.cells).unwrap_or(i64::MAX))
    }

    /// The lakes a hold or a lift held or lifted whole, by their places in the drawing the land was
    /// grown from (`LandformLand.get_drawing()`, from 0, as `LandformWorld.get_lake_drawn` gives
    /// them), in that order: those whose water it marked, through the area or through water that
    /// meets it, and of whose cells it changed one mark at least (the core's `Land::hold_lakes`).
    /// Empty in a step of another kind.
    #[func]
    fn get_lakes_whole(&self) -> PackedInt64Array {
        self.held().map_or_else(PackedInt64Array::new, |held| held.lakes.iter().map(|&lake| i64::try_from(lake).unwrap_or(i64::MAX)).collect())
    }

    /// What the step holds in memory, in bytes (`Change::bytes`): the cells and parts in which the
    /// land's two states differ, with the other state's values. 0 in a step that holds nothing or
    /// was let go.
    #[func]
    fn get_bytes(&self) -> i64 {
        self.contents.as_ref().and_then(|contents| contents.slot().change.as_ref().map(Change::bytes)).map_or(0, |bytes| i64::try_from(bytes).unwrap_or(i64::MAX))
    }

    /// Whether the step still holds its change, so a swap of it changes the land: false once
    /// `LandformLand.keep_steps_within` let it go, or the land was read, restored or made anew since,
    /// and in a step that holds nothing.
    #[func]
    fn is_kept(&self) -> bool {
        self.contents.as_ref().is_some_and(|contents| contents.slot().change.is_some())
    }

    /// Whether the step changes nothing: its change was empty when it was made (an ask of the
    /// drawing the land asks for already, a hold of ground held already, the land's own reach, a
    /// regrowth that changed no part), so it lies at one state and a swap of it moves the land
    /// nowhere, whether or not it was let go since. False in a step that holds nothing (made by
    /// `LandformLandStep.new()`). The editor tool makes no undo action of such a step (stage 2's
    /// ruling R19, N2).
    #[func]
    fn is_empty(&self) -> bool {
        self.contents.as_ref().is_some_and(|contents| contents.states[0] == contents.states[1])
    }
}

/// A land's steps in the order they were made, each reached weakly, so Godot's history keeps them
/// alive and a step it lets go is gone from here too.
#[derive(Default)]
pub(crate) struct Steps {
    made: Vec<Weak<Mutex<Slot>>>,
}

impl Steps {
    /// Records a new step's slot, the newest.
    fn add(&mut self, slot: &Shared) {
        self.forget_gone();
        self.made.push(Arc::downgrade(slot));
    }

    /// The slots of the steps still kept, oldest first.
    fn kept(&self) -> Vec<Shared> {
        (self.made.iter()).filter_map(Weak::upgrade).filter(|slot| locked(slot).change.is_some()).collect()
    }

    /// The slots of the steps still kept that change something, oldest first: an empty step (its
    /// change empty, so its swap moves the land nowhere) is never counted nor let go by the budget,
    /// so one held outside every undo action cannot make the budget let go older ones (stage 2's
    /// ruling R22, M1).
    fn kept_changing(&self) -> Vec<Shared> {
        self.kept().into_iter().filter(|slot| locked(slot).change.as_ref().is_some_and(|change| !change.is_empty())).collect()
    }

    /// Forgets the steps Godot let go and those let go here.
    fn forget_gone(&mut self) {
        self.made.retain(|slot| slot.upgrade().is_some_and(|slot| locked(&slot).change.is_some()));
    }

    /// Lets go the oldest steps kept beyond `most` bytes: from the newest back, the first step whose
    /// bytes, with those of every step newer than it, come to more than `most`, and every step
    /// older than it, so the land is never put together from steps of different ages (the stage 2
    /// plan's decision 4). Empty steps are skipped: neither counted nor let go. Returns how many it
    /// let go.
    pub(crate) fn keep_within(&mut self, most: usize) -> usize {
        let (mut total, mut over, mut let_go) = (0usize, false, 0);
        for slot in self.kept_changing().iter().rev() {
            let mut slot = locked(slot);
            if !over {
                total = total.saturating_add(slot.change.as_ref().map_or(0, Change::bytes));
                over = total > most;
            }
            if over && slot.change.take().is_some() {
                slot.let_go = LetGo::Budget;
                let_go += 1;
            }
        }
        self.forget_gone();
        let_go
    }

    /// Lets go every step still kept, the land having been read, restored or made anew: each lies
    /// between states of the land it held, which it will never be in again, and says so when swapped
    /// (`LetGo::NewLand`). Returns how many it let go.
    pub(crate) fn let_all_go(&mut self) -> usize {
        let mut let_go = 0;
        for slot in self.kept() {
            let mut slot = locked(&slot);
            if slot.change.take().is_some() {
                slot.let_go = LetGo::NewLand;
                let_go += 1;
            }
        }
        self.forget_gone();
        let_go
    }

    /// What the steps still kept that change something hold, in bytes; an empty step is not
    /// counted.
    pub(crate) fn bytes(&self) -> usize {
        self.kept_changing().iter().map(|slot| locked(slot).change.as_ref().map_or(0, Change::bytes)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool;
    use landform_core::{Drawing, GenConfig, Land};
    use std::sync::atomic::AtomicBool;

    /// The side of the land the test grows, the smallest the example is drawn at.
    const SIZE_M: u32 = 128;
    /// The seed it grows from.
    const SEED: u64 = 11;

    /// A land's steps let go oldest first: within a budget the newest steps that fit are kept and
    /// every step older than the first that does not is let go with it, and a step dropped by
    /// whatever held it is no longer counted; a new land lets every step go, each saying why.
    #[test]
    fn steps_are_let_go_oldest_first_and_every_older_one_with_them() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = pool::run(|| Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false))).unwrap().expect("the example grows");
        // Three steps of one size, each the step back from a reach set.
        let change = || Arc::new(Mutex::new(Slot { change: Some(Change::to_parts(&land, None, None, Some(land.reach_m + 1.0))), let_go: LetGo::Budget }));
        let one = locked(&change()).change.as_ref().map_or(0, Change::bytes);
        let (oldest, middle, newest) = (change(), change(), change());
        let mut steps = Steps::default();
        for made in [&oldest, &middle, &newest] {
            steps.add(made);
        }
        assert_eq!(steps.bytes(), 3 * one);
        assert_eq!(steps.keep_within(3 * one), 0, "all three fit");
        assert_eq!(steps.keep_within(3 * one - 1), 1, "the oldest is let go");
        assert!(locked(&oldest).change.is_none() && locked(&middle).change.is_some() && locked(&newest).change.is_some());
        assert_eq!(locked(&oldest).let_go, LetGo::Budget);
        assert_eq!(steps.bytes(), 2 * one);
        // Within one step's bytes the newest is kept and the middle one let go.
        let (older, newer) = (change(), change());
        steps.add(&older);
        steps.add(&newer);
        assert_eq!(steps.keep_within(one), 3, "the middle one, the newest of the first three and the older of the next two");
        assert!(locked(&newest).change.is_none() && locked(&older).change.is_none() && locked(&newer).change.is_some());
        // A step dropped is no longer counted, and nothing is left to let go.
        drop(newer);
        assert_eq!((steps.bytes(), steps.keep_within(0)), (0, 0));
        // A land read, restored or made anew lets go every step kept, saying why; one the budget let
        // go before keeps its own reason.
        let (first, second) = (change(), change());
        steps.add(&first);
        steps.add(&second);
        assert_eq!(steps.let_all_go(), 2);
        assert!([&first, &second].iter().all(|slot| locked(slot).change.is_none() && locked(slot).let_go == LetGo::NewLand));
        assert_eq!(locked(&oldest).let_go, LetGo::Budget);
        assert_eq!((steps.bytes(), steps.let_all_go()), (0, 0));
    }

    /// Stage 2's ruling R22, M1: an empty step newer than steps kept, as an ask of the drawing the
    /// land asks for already gives one, is neither counted nor let go: kept within the others' sum,
    /// none is let go; a new land still lets it go with the rest.
    #[test]
    fn an_empty_step_is_neither_counted_nor_let_go_by_the_budget() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let land = pool::run(|| Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false))).unwrap().expect("the example grows");
        let changing = || Arc::new(Mutex::new(Slot { change: Some(Change::to_parts(&land, None, None, Some(land.reach_m + 1.0))), let_go: LetGo::Budget }));
        let empty = Arc::new(Mutex::new(Slot { change: Some(Change::to_parts(&land, None, None, Some(land.reach_m))), let_go: LetGo::Budget }));
        assert!(locked(&empty).change.as_ref().is_some_and(Change::is_empty), "the land's own reach gives an empty change");
        let one = locked(&changing()).change.as_ref().map_or(0, Change::bytes);
        let (older, newer) = (changing(), changing());
        let mut steps = Steps::default();
        for made in [&older, &newer, &empty] {
            steps.add(made);
        }
        assert_eq!(steps.bytes(), 2 * one, "the empty step is counted");
        assert_eq!(steps.keep_within(2 * one), 0, "a step was let go within the changing steps' sum");
        assert!([&older, &newer, &empty].iter().all(|slot| locked(slot).change.is_some()));
        assert_eq!(steps.keep_within(one), 1, "the oldest changing step is let go, and the empty one is not");
        assert!(locked(&older).change.is_none() && locked(&newer).change.is_some() && locked(&empty).change.is_some());
        assert_eq!(steps.let_all_go(), 2, "a new land lets go the empty step with the rest");
        assert!(locked(&empty).change.is_none() && locked(&empty).let_go == LetGo::NewLand);
    }
}
