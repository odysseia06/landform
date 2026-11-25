//! What a land file costs: the example at 2,048 m written and read, timed in a test binary of its
//! own so that `cargo test` runs nothing beside it (the plan's Task 9, Step 5; the design's budget
//! of 3 s to save a land and 3 s to open it at 2 km). The file's size goes to the gate with the
//! question of packing it (the plan's decision 6).

use landform_core::{Drawing, GenConfig, Land};
use std::fs;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

/// The most writing or reading the example's land file may take, in milliseconds: the design's
/// budget.
const FILE_MS_MOST: f64 = 3000.0;
/// How many times the land is written and read; the least of each is what is asserted.
const RUNS: usize = 3;
/// The example's seed and size.
const SEED: u64 = 11;
const SIZE_M: u32 = 2048;
/// Bytes in a megabyte, as the plan counts the heights' 16.8 MB.
const MB: f64 = 1e6;

/// The example at 2,048 m in hill country from seed 11 is written and read `RUNS` times; the least
/// time of each is under `FILE_MS_MOST`, and the land read is the land written.
#[test]
fn the_examples_land_is_written_and_read_in_less_than_three_seconds_each() {
    let land = Land::grow(&Drawing::example_valley(SIZE_M), SEED, &GenConfig::default(), &AtomicBool::new(false)).expect("the example grows");
    let path = std::env::temp_dir().join(format!("landform_land_file_time_{}.lfland", std::process::id()));
    let (mut writes, mut reads) = (Vec::new(), Vec::new());
    let mut read = None;
    for _ in 0..RUNS {
        let clock = Instant::now();
        land.write(&path).expect("the land is written");
        writes.push(clock.elapsed().as_secs_f64() * 1000.0);
        let clock = Instant::now();
        read = Some(Land::read(&path).expect("the land is read"));
        reads.push(clock.elapsed().as_secs_f64() * 1000.0);
    }
    let bytes = fs::metadata(&path).expect("the file is there").len();
    let _ = fs::remove_file(&path);
    let least = |times: &[f64]| times.iter().copied().fold(f64::INFINITY, f64::min);
    let (write_ms, read_ms) = (least(&writes), least(&reads));
    println!(
        "the example's land at {SIZE_M} m: {bytes} bytes ({:.1} MB); written in {write_ms:.1} ms at the least {writes:.1?}, read in {read_ms:.1} ms at the least {reads:.1?}",
        bytes as f64 / MB
    );
    assert!(read.as_ref() == Some(&land), "the land read is the land written");
    assert!(write_ms < FILE_MS_MOST, "writing took {write_ms:.0} ms at the least, beyond {FILE_MS_MOST} ms");
    assert!(read_ms < FILE_MS_MOST, "reading took {read_ms:.0} ms at the least, beyond {FILE_MS_MOST} ms");
}
