//! The extension's background jobs: the generations of `generator.rs` and the growing and editing of
//! a land in `land.rs`. Each runs on a thread of its own, which runs its parallel work in the
//! extension's pool (`pool::run`), and reads a cancel flag of its own (`Cancel`) between its steps.
//! `join_workers` sets every job's flag and waits for every job's thread, before the library unloads.
//! Plain Rust: no Godot types.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// What a job whose thread ended without giving its result is said to have done: its work
/// panicked, which Godot's output shows above this.
const STOPPED_UNEXPECTEDLY: &str = "The job stopped unexpectedly.";

/// Background jobs that may still be running, generations and land jobs alike, including those
/// whose node or land was freed. A thread still running this library's code when Godot unloads it
/// crashes the process, so `join_workers` waits for them first.
static WORKERS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// The cancel flag of every job that may still be running, in the foreground or the background,
/// each its own (`Cancel`), held weakly: a flag is gone once its job and whatever started it have
/// let it go. `join_workers` sets every one, so the jobs it waits for stop at their next check; a
/// job started after it has a flag of its own, not set, and runs to its end.
static FLAGS: Mutex<Vec<Weak<AtomicBool>>> = Mutex::new(Vec::new());

/// One job's cancel flag, which the job reads between its steps: whatever started a job sets its
/// flag, and no other, and `join_workers` sets every job's.
#[derive(Clone)]
pub(crate) struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag of a new job, not set, recorded in `FLAGS` so `join_workers` can set it.
    pub(crate) fn new() -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let mut flags = FLAGS.lock().unwrap_or_else(PoisonError::into_inner);
        flags.retain(|kept| kept.strong_count() > 0);
        flags.push(Arc::downgrade(&flag));
        Self(flag)
    }

    /// Asks the job to stop at its next check.
    pub(crate) fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// The flag, as the core reads it.
    pub(crate) fn flag(&self) -> &AtomicBool {
        &self.0
    }
}

/// Runs `work` on a new thread named `name`, recorded in `WORKERS` so `join_workers` waits for it.
/// When the thread cannot start, `work` is dropped without running and the error is returned.
pub(crate) fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> io::Result<()> {
    let handle = thread::Builder::new().name(name.to_string()).spawn(work)?;
    let mut workers = WORKERS.lock().unwrap_or_else(PoisonError::into_inner);
    workers.retain(|worker| !worker.is_finished());
    workers.push(handle);
    Ok(())
}

/// Cancels every job, setting each one's own flag, and waits for each background job to end.
/// Called before the library unloads, and before the pool is shut down, since a running job holds
/// the pool. A job stops at its next check of its flag, so the wait is at most one step of the job:
/// one stage of a generation, one round of growing land, or one step of a land's edit. A job started
/// afterwards has a new flag, not set.
pub fn join_workers() {
    let flags: Vec<Arc<AtomicBool>> = FLAGS.lock().unwrap_or_else(PoisonError::into_inner).iter().filter_map(Weak::upgrade).collect();
    for flag in flags {
        Cancel(flag).set();
    }
    let workers = std::mem::take(&mut *WORKERS.lock().unwrap_or_else(PoisonError::into_inner));
    for worker in workers {
        // A job that panicked has nothing left to clean up.
        let _ = worker.join();
    }
}

/// Where a job's result arrives: put once by the job's thread, taken once by whatever started it.
struct Slot<T> {
    state: Mutex<Arrived<T>>,
    arrived: Condvar,
}

/// What a `Slot` holds: whether the job has ended, and its result until it is taken.
struct Arrived<T> {
    ended: bool,
    result: Option<Result<T, String>>,
}

impl<T> Slot<T> {
    fn put(&self, result: Result<T, String>) {
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = Arrived { ended: true, result: Some(result) };
        self.arrived.notify_all();
    }
}

/// The job's end of its `Slot`: `send` puts the result there, and a job whose thread ends without
/// sending (its work panicked) puts `STOPPED_UNEXPECTEDLY` there when this is dropped, so a result
/// always arrives. (A job whose thread never started drops it too, where nothing reads it: its
/// start is refused at once, `Running::start`.)
struct Sender<T>(Option<Arc<Slot<T>>>);

impl<T> Sender<T> {
    fn send(mut self, result: Result<T, String>) {
        if let Some(slot) = self.0.take() {
            slot.put(result);
        }
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        if let Some(slot) = self.0.take() {
            slot.put(Err(STOPPED_UNEXPECTEDLY.to_string()));
        }
    }
}

/// A background job that may still be running: its own cancel flag and where its result arrives.
/// Letting it go sets the flag, so a job whose result nobody will take stops at its next check.
pub(crate) struct Running<T> {
    cancel: Cancel,
    slot: Arc<Slot<T>>,
}

impl<T: Send + 'static> Running<T> {
    /// Starts `work` on a thread named `name`, with a new cancel flag that `work` is given to read,
    /// once the thread runs; when the thread cannot start, `work` is let go without running and the
    /// sentence is `cannot_start` with the reason, so whatever started it says so at once.
    pub(crate) fn start(name: &str, cannot_start: &str, work: impl FnOnce(&AtomicBool) -> Result<T, String> + Send + 'static) -> Result<Self, String> {
        let cancel = Cancel::new();
        let slot = Arc::new(Slot { state: Mutex::new(Arrived { ended: false, result: None }), arrived: Condvar::new() });
        let (flag, sender) = (cancel.clone(), Sender(Some(Arc::clone(&slot))));
        spawn(name, move || sender.send(work(flag.flag()))).map_err(|error| format!("{cannot_start}: {error}."))?;
        Ok(Self { cancel, slot })
    }
}

impl<T> Running<T> {
    /// Asks the job to stop at its next check of its flag.
    pub(crate) fn cancel(&self) {
        self.cancel.set();
    }

    /// The job's result, once it has ended: `None` while it runs, and after the result was taken.
    pub(crate) fn take(&self) -> Option<Result<T, String>> {
        self.slot.state.lock().unwrap_or_else(PoisonError::into_inner).result.take()
    }

    /// Waits until the job has ended, or until `most` has passed when it is given, without taking
    /// the result; whether the job has ended.
    pub(crate) fn wait(&self, most: Option<Duration>) -> bool {
        let state = self.slot.state.lock().unwrap_or_else(PoisonError::into_inner);
        let running = |state: &mut Arrived<T>| !state.ended;
        match most {
            None => self.slot.arrived.wait_while(state, running).unwrap_or_else(PoisonError::into_inner).ended,
            Some(most) => self.slot.arrived.wait_timeout_while(state, most, running).unwrap_or_else(PoisonError::into_inner).0.ended,
        }
    }
}

impl<T> Drop for Running<T> {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

/// A land job here runs its parallel work through the extension's pool, as a generation does, so no
/// test here starts rayon's global pool, which the test in `pool.rs` checks.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::tests::{cancelled, drawn_job};
    use crate::generator::start;
    use crate::land::tests::start_regrow;
    use crate::pool;
    use landform_core::{example_edit, Drawing, GenConfig, Land};
    use std::sync::mpsc::Receiver;
    use std::time::Instant;

    /// What a land job's regrowth ends with when its flag stops it.
    const STOPPED: &str = "The regrowth was stopped.";
    /// How long a job's result is waited for before a test gives up on it: far longer than any job
    /// here takes.
    const WAIT_MOST: Duration = Duration::from_secs(120);

    /// The example `size_m` across in hills from seed 11, asking for its edit `a` (the brook's
    /// second point moved 30 m east), so a regrowth of it grows and joins; grown and asked in the
    /// pool.
    fn waiting_land(size_m: u32) -> Arc<Land> {
        let land = pool::run(|| {
            let mut land = Land::grow(&Drawing::example_valley(size_m), 11, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
            let edited = example_edit(&land.drawing, "a").expect("the example's edit a");
            land.ask(edited).expect("the edit is one the land may ask for");
            land
        });
        Arc::new(land.unwrap())
    }

    /// A job's result, waited for at most `WAIT_MOST`.
    fn ended<T: Send + 'static>(job: &Running<T>) -> Result<T, String> {
        assert!(job.wait(Some(WAIT_MOST)), "the job did not end");
        job.take().expect("the result of a job that ended")
    }

    /// A job's result arrives once, after it has ended; one whose work panics arrives as
    /// `STOPPED_UNEXPECTEDLY`; and letting a job go sets its flag, and no other. It takes turns
    /// with the tests that call `join_workers`, which sets every flag, the other job's among them.
    #[test]
    fn a_jobs_result_arrives_once_and_letting_it_go_stops_it() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let job = Running::start("landform-test", "cannot start", move |_| {
            wait.recv().expect("the test lets the job go on");
            Ok(7)
        })
        .expect("the test's thread starts");
        assert!(job.take().is_none(), "a result before the job ended");
        assert!(!job.wait(Some(Duration::from_millis(20))), "a job waiting for the test ended");
        go.send(()).unwrap();
        assert_eq!(ended(&job), Ok(7));
        assert!(job.take().is_none(), "a result taken twice");
        assert!(job.wait(None), "a job whose result was taken has still ended");

        let panicked = Running::<i32>::start("landform-test", "cannot start", |_| panic!("a test's job panics")).expect("the test's thread starts");
        assert_eq!(ended(&panicked), Err(STOPPED_UNEXPECTEDLY.to_string()));

        let (seen, flag) = std::sync::mpsc::channel::<bool>();
        let (go, wait) = std::sync::mpsc::channel::<()>();
        let other = Cancel::new();
        let let_go = Running::start("landform-test", "cannot start", move |cancel| {
            wait.recv().expect("the test lets the job go on");
            seen.send(cancel.load(Ordering::Relaxed)).expect("the test reads the flag");
            Ok(())
        })
        .expect("the test's thread starts");
        drop(let_go);
        go.send(()).unwrap();
        assert!(flag.recv_timeout(WAIT_MOST).expect("the job reads its flag"), "letting the job go left its flag unset");
        assert!(!other.flag().load(Ordering::Relaxed), "letting one job go set another's flag");
    }

    /// A land job and a generation started together, the land job cancelled: it ends with
    /// `STOPPED` and the generation with its world. Then `join_workers`, called as a land job and a
    /// generation of 1,024 m start, sets both their flags and waits for both, which end stopped
    /// whether or not they had begun to run, and a land job started after it runs to its end. The
    /// pool's tests take turns with this one, since it builds the pool and `join_workers` reaches
    /// every job of the process.
    #[test]
    fn a_land_job_stops_on_its_own_flag_and_join_workers_stops_it_with_a_generation() {
        let _turn = pool::tests::TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let small = waiting_land(512);
        let land_job = start_regrow(Arc::clone(&small));
        let (generation, _generation_cancel) = start(drawn_job(256, 2));
        land_job.cancel();
        assert_eq!(ended(&land_job).err(), Some(STOPPED.to_string()));
        let world = generation.recv_timeout(WAIT_MOST).expect("the generation sends its result").expect("the generation ends with a world");
        assert_eq!((world.heights.width, world.meta.seed), (256, 2));

        let large = waiting_land(1024);
        let land_job = start_regrow(Arc::clone(&large));
        let (generation, generation_cancel): (Receiver<_>, _) = start(drawn_job(1024, 3));
        let clock = Instant::now();
        join_workers();
        println!("join_workers waited {:.0} ms for a land job and a generation of 1,024 m", clock.elapsed().as_secs_f64() * 1000.0);
        assert!(land_job.wait(Some(Duration::ZERO)), "join_workers did not wait for the land job");
        assert_eq!(land_job.take().expect("the land job's result").err(), Some(STOPPED.to_string()));
        assert!(generation_cancel.flag().load(Ordering::Relaxed), "join_workers left the generation's flag unset");
        assert_eq!(generation.try_recv().expect("join_workers waited for the generation").err(), cancelled());

        let after = start_regrow(small);
        assert!(ended(&after).is_ok(), "a land job after join_workers was stopped");
        join_workers();
        pool::shut_down();
    }
}
