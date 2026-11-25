//! The extension's own rayon pool. Every generation runs inside it, so the core's parallel stages
//! never start rayon's global pool, whose threads would outlive the library. `shut_down` stops the
//! pool and waits for its threads, so nothing runs this library's code once Godot unloads it.
//! Plain Rust: no Godot types.

use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use rayon::{ThreadPool, ThreadPoolBuilder};

/// The running pool and the handles of the threads it started.
struct Pool {
    pool: Arc<ThreadPool>,
    threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

/// Built by the first generation, emptied by `shut_down`.
static POOL: Mutex<Option<Pool>> = Mutex::new(None);

type Handles = Arc<Mutex<Vec<JoinHandle<()>>>>;

/// Builds a pool with the default thread count whose threads this module starts itself and can
/// therefore join: dropping a rayon pool only asks its threads to stop.
fn build() -> Result<Pool, String> {
    let threads: Handles = Arc::new(Mutex::new(Vec::new()));
    let pool = build_into(&threads, None)?;
    Ok(Pool { pool: Arc::new(pool), threads })
}

/// Builds the pool, recording each started thread's handle in `threads`. If the build fails,
/// rayon tells the threads it already started to stop; they are joined before the error returns.
/// `fail_at` makes the thread with that index fail to start, for tests.
fn build_into(threads: &Handles, fail_at: Option<usize>) -> Result<ThreadPool, String> {
    let started = Arc::clone(threads);
    let built = ThreadPoolBuilder::new()
        .thread_name(|index| format!("landform-pool-{index}"))
        .spawn_handler(move |worker| {
            if fail_at == Some(worker.index()) {
                return Err(std::io::Error::other("a test refused to start this thread"));
            }
            let mut builder = thread::Builder::new();
            if let Some(name) = worker.name() {
                builder = builder.name(name.to_string());
            }
            if let Some(size) = worker.stack_size() {
                builder = builder.stack_size(size);
            }
            let handle = builder.spawn(move || worker.run())?;
            started.lock().unwrap_or_else(PoisonError::into_inner).push(handle);
            Ok(())
        })
        .build();
    built.map_err(|e| {
        join_all(threads);
        format!("The generator's threads could not start: {e}.")
    })
}

/// Joins every thread in `threads`; returns how many it waited for.
fn join_all(threads: &Handles) -> usize {
    let handles = std::mem::take(&mut *threads.lock().unwrap_or_else(PoisonError::into_inner));
    let count = handles.len();
    for handle in handles {
        // A pool thread that panicked has nothing left to clean up.
        let _ = handle.join();
    }
    count
}

/// Runs `job` inside the extension's pool, building the pool first if needed. The calling thread
/// waits for the result. Fails, without running `job`, only when the pool cannot start.
pub fn run<R: Send>(job: impl FnOnce() -> R + Send) -> Result<R, String> {
    let pool = {
        let mut slot = POOL.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(build()?);
        }
        let running = slot.as_ref().expect("the pool was just built");
        Arc::clone(&running.pool)
    };
    Ok(pool.install(job))
}

/// Stops the pool and waits for each of its threads to end; returns how many it waited for.
/// Call it after every job has finished. A job still holding the pool delays the wait until it
/// is done. The next `run` builds a new pool.
pub fn shut_down() -> usize {
    let Some(Pool { pool, threads }) = POOL.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return 0;
    };
    // The last reference to the pool tells its threads to stop once their work is done.
    drop(pool);
    join_all(&threads)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use landform_core::{generate, GenConfig, SyntheticSource};

    /// The pool is process-wide, so its tests take turns, and so do the generator's tests that
    /// build it.
    pub(crate) static TURN: Mutex<()> = Mutex::new(());

    #[test]
    fn a_job_runs_on_a_pool_thread_and_shut_down_waits_for_every_thread() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let (index, name) = run(|| (rayon::current_thread_index(), thread::current().name().map(String::from))).unwrap();
        assert!(index.is_some(), "the job did not run on a pool thread");
        assert!(name.is_some_and(|n| n.starts_with("landform-pool-")));
        let expected = run(rayon::current_num_threads).unwrap();
        assert_eq!(shut_down(), expected);
        assert_eq!(shut_down(), 0, "a second shut_down found a pool");
        // A later job builds a new pool.
        assert!(run(rayon::current_thread_index).unwrap().is_some());
        assert_eq!(shut_down(), expected);
    }

    #[test]
    fn a_failed_build_joins_the_threads_it_started() {
        let threads: Handles = Arc::new(Mutex::new(Vec::new()));
        let built = build_into(&threads, Some(2));
        assert!(built.is_err_and(|e| e.contains("could not start")));
        assert!(threads.lock().unwrap().is_empty(), "the started threads were not joined");
    }

    #[test]
    fn generating_through_the_pool_leaves_the_global_pool_unstarted() {
        let _turn = TURN.lock().unwrap_or_else(PoisonError::into_inner);
        let source = SyntheticSource::standard();
        let config = GenConfig { size_cells: 128, ..GenConfig::default() };
        let world = run(|| generate(1, &[&source], &config)).unwrap().unwrap();
        assert_eq!(world.heights.width, 128);
        shut_down();
        // Building the global pool fails if anything has already started it. Nothing else in this
        // test binary touches rayon outside `run`.
        assert!(ThreadPoolBuilder::new().num_threads(1).build_global().is_ok(), "the global pool was started");
    }
}
