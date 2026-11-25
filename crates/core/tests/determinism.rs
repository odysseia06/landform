use landform_core::{
    example_edit, generate, generate_drawn, standard_edit, trial_drawing, water_surface, About, Country, CourseEnd, Drawing, GenConfig,
    GroundCell, GroundRules, Land, SyntheticSource, World, WATER_LAYER,
};
use std::collections::BTreeMap;
use std::sync::atomic::AtomicBool;

/// A course of the water as bits: its drawn river and its lake, each point's position, surface,
/// width and depth, and its end.
type CourseBits = (Option<usize>, Option<usize>, Vec<[u32; 5]>, CourseEnd);
/// A lake of the water as bits: its drawn lake, then its level, position and area.
type LakeBits = (usize, [u32; 4]);

/// Everything of a world that must not depend on the thread count, floats as their bits.
#[derive(PartialEq)]
struct Bits {
    heights: Vec<u32>,
    layers: BTreeMap<String, Vec<u32>>,
    ground: Vec<GroundCell>,
    rivers: Vec<CourseBits>,
    lakes: Vec<LakeBits>,
}

impl Bits {
    fn of(world: World) -> Bits {
        let bits = |data: &[f32]| data.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
        let rivers = (world.water.rivers.iter())
            .map(|course| {
                let points = (course.points.iter())
                    .map(|p| [p.x_m, p.z_m, p.surface_m, p.width_m, p.depth_m].map(f32::to_bits))
                    .collect();
                (course.drawn, course.from_lake, points, course.end)
            })
            .collect();
        let lakes = (world.water.lakes.iter())
            .map(|lake| (lake.drawn, [lake.level_m, lake.x_m, lake.z_m, lake.area_m2].map(f32::to_bits)))
            .collect();
        Bits {
            heights: bits(&world.heights.data),
            layers: world.layers.iter().map(|(name, layer)| (name.clone(), bits(&layer.data))).collect(),
            ground: world.ground.data,
            rivers,
            lakes,
        }
    }

    /// Which parts differ from `other`, for a failure message: the vectors are too long to print.
    fn differences(&self, other: &Bits) -> Vec<String> {
        fn differing<T: PartialEq>(a: &[T], b: &[T]) -> usize {
            a.iter().zip(b).filter(|(x, y)| x != y).count() + a.len().abs_diff(b.len())
        }
        let mut parts = vec![format!("heights {}", differing(&self.heights, &other.heights))];
        for (name, layer) in &self.layers {
            let count = other.layers.get(name).map_or(layer.len(), |theirs| differing(layer, theirs));
            parts.push(format!("{name} {count}"));
        }
        parts.push(format!("ground {}", differing(&self.ground, &other.ground)));
        parts.push(format!("rivers {}", differing(&self.rivers, &other.rivers)));
        parts.push(format!("lakes {}", differing(&self.lakes, &other.lakes)));
        parts
    }
}

fn world_bits(seed: u64, source: &SyntheticSource, cfg: &GenConfig) -> Bits {
    Bits::of(generate(seed, &[source], cfg).unwrap())
}

/// `work` on a pool of `threads` threads.
fn on_pool<T: Send>(threads: usize, work: impl FnOnce() -> T + Send) -> T {
    rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(work)
}

/// `world_bits` on a pool of `threads` threads.
fn on_threads(threads: usize, seed: u64, source: &SyntheticSource, cfg: &GenConfig) -> Bits {
    on_pool(threads, || world_bits(seed, source, cfg))
}

#[test]
fn same_seed_is_bit_identical_on_any_thread_count() {
    let s = SyntheticSource::new(1, 50_000.0);
    let cfg = GenConfig { size_cells: 512, ..Default::default() };
    let one = on_threads(1, 42, &s, &cfg);
    assert_eq!(one.layers.len(), 4);
    for threads in [8, 1] {
        let again = on_threads(threads, 42, &s, &cfg);
        assert!(one == again, "on {threads} threads, differing values: {:?}", one.differences(&again));
    }
    let other = world_bits(43, &s, &cfg);
    assert!(one.heights != other.heights, "seed 43 gives the heights of seed 42");
    assert!(one.ground != other.ground, "seed 43 gives the ground map of seed 42");
}

/// The synthetic source has no ground above 24 degrees, so with the default rules only the
/// gullies and hummocks act. These thresholds bring ledges and roughness onto it.
fn steep_rules() -> GroundRules {
    GroundRules {
        ledge_slope_start_deg: 2.0,
        ledge_slope_full_deg: 6.0,
        rock_slope_start_deg: 4.0,
        rock_slope_full_deg: 8.0,
        ..Default::default()
    }
}

#[test]
fn every_ground_shape_is_bit_identical_on_any_thread_count() {
    let s = SyntheticSource::new(1, 50_000.0);
    let cfg = GenConfig { size_cells: 512, rules: steep_rules(), ..Default::default() };
    let full = on_threads(1, 42, &s, &cfg);

    // Each shape moves cells: the world without it differs from the full one by more than a
    // millimetre somewhere. Heights are shifted so the lowest cell is 0, and a shape that lowers
    // the lowest cell would change every shifted height, so the shift is added back.
    let surface = |rules: GroundRules| {
        let world = generate(42, &[&s], &GenConfig { rules, ..cfg }).unwrap();
        world.heights.data.iter().map(|h| h + world.meta.base_elevation_m).collect::<Vec<f32>>()
    };
    let rules = cfg.rules;
    let all = surface(rules);
    for (shape, off) in [
        ("gullies", GroundRules { gully_depth_m: 0.0, ..rules }),
        ("ledges", GroundRules { ledge_height_m: 0.0, ..rules }),
        ("roughness", GroundRules { rough_amplitude_m: 0.0, ..rules }),
    ] {
        let moved = surface(off).iter().zip(&all).filter(|(a, b)| (*a - *b).abs() > 1e-3).count();
        assert!(moved > 0, "switching {shape} off moves no cell");
    }

    for threads in [8, 1] {
        let again = on_threads(threads, 42, &s, &cfg);
        assert!(full == again, "on {threads} threads, differing values: {:?}", full.differences(&again));
    }
}

#[test]
fn a_drawn_world_is_the_same_on_any_thread_count() {
    let drawing = Drawing::example_valley(512);
    let cfg = GenConfig::default();
    let drawn = |threads: usize| {
        on_pool(threads, || Bits::of(generate_drawn(1, &drawing, &cfg, &AtomicBool::new(false)).unwrap()))
    };
    let one = drawn(1);
    // The four layers every world has, and the water's.
    assert_eq!(one.layers.len(), 5);
    assert!(!one.rivers.is_empty(), "the drawn world has no river to compare");
    assert!(!one.lakes.is_empty(), "the drawn world has no lake to compare");
    for threads in [4, 16] {
        let again = drawn(threads);
        assert!(one == again, "on {threads} threads, differing values: {:?}", one.differences(&again));
    }
}

/// The drawn worlds whose water the held-water rules shape most: the trial drawing `headjoin` at
/// 1,024 m in hill country from seed 3, where the river is brought down to a tributary in a second
/// pass of the junctions' rule, and the example at 2,048 m in hill country from seed 11, one of the
/// counting program's example worlds, its lake at the level its ground holds and its outflow held at
/// that level beside it.
/// Each gives the same heights, layers, ground map, courses and lakes on 1 thread, on 2 and on all
/// the machine has.
#[test]
fn the_held_waters_worlds_are_the_same_on_any_thread_count() {
    let all = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let headjoin = trial_drawing("headjoin", 1024, Country::hills()).unwrap();
    let example = Drawing { country: Country::hills(), ..Drawing::example_valley(2048) };
    for (name, drawing, seed) in [("headjoin, hills, seed 3", &headjoin, 3), ("the example, hills, seed 11", &example, 11)] {
        let drawn = |threads: usize| on_pool(threads, || generate_drawn(seed, drawing, &GenConfig::default(), &AtomicBool::new(false)).unwrap());
        let world = drawn(1);
        assert!(!world.water.rivers.is_empty(), "{name}: no river to compare");
        if drawing.lakes.is_empty() {
            assert!(world.notes.iter().any(|note| note.starts_with("River 1 was cut")), "{name}: no river is brought down to a tributary: {:?}", world.notes);
        } else {
            assert!(!world.water.lakes.is_empty(), "{name}: no lake to compare");
        }
        let one = Bits::of(world);
        for threads in [2, all] {
            let again = Bits::of(drawn(threads));
            assert!(one == again, "{name}: on {threads} threads, differing values: {:?}", one.differences(&again));
        }
    }
}

/// A piece of a water surface as bits: its column and row, then its positions, depths, velocities,
/// falls and indices.
type PieceBits = (usize, usize, Vec<[u32; 3]>, Vec<u32>, Vec<[u32; 2]>, Vec<u32>, Vec<u32>);

/// The pieces of the surface of a drawn world made on a pool of `threads` threads, floats as their
/// bits; the world is made on one thread, so only the surface's own split differs.
fn surface_bits(world: &World, threads: usize) -> Vec<PieceBits> {
    let surface = on_pool(threads, || water_surface(&world.heights, &world.layers[WATER_LAYER], &world.water).unwrap());
    let bits = |data: &[f32]| data.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
    (surface.pieces.iter())
        .map(|piece| {
            (
                piece.column,
                piece.row,
                piece.positions.iter().map(|p| p.map(f32::to_bits)).collect(),
                bits(&piece.depth_m),
                piece.velocity.iter().map(|v| v.map(f32::to_bits)).collect(),
                bits(&piece.fall),
                piece.indices.clone(),
            )
        })
        .collect()
}

#[test]
fn a_drawn_worlds_water_surface_is_the_same_on_any_thread_count() {
    let world = on_pool(1, || {
        generate_drawn(1, &Drawing::example_valley(512), &GenConfig::default(), &AtomicBool::new(false)).unwrap()
    });
    let one = surface_bits(&world, 1);
    assert!(!one.is_empty(), "the drawn world has no water surface to compare");
    for threads in [4, 16] {
        let again = surface_bits(&world, threads);
        assert!(one == again, "on {threads} threads, the surface differs in {} pieces", one.iter().zip(&again).filter(|(a, b)| a != b).count());
    }
}

/// An edited land as bits, beyond its world's (`Bits`): the model's heights, the first lakes'
/// levels, the outline's weights, the troughs, the marks and the notes with their subjects, the
/// edit's and the land's.
type EditBits = (Vec<u32>, Vec<(usize, u32)>, Vec<u32>, Vec<u32>, Vec<[u64; 3]>, Vec<bool>, Vec<String>, Vec<About>, Vec<String>, Vec<About>);

/// The same edit of a land gives a bit-identical land on 1, 4 and 16 threads: the example at 2,048 m
/// in hill country from seed 11 with a tributary added (`f`, whose mending lowers points at weight 1
/// and the water beside them), a disc held east of it, and the trial `shoulder` at 1,024 m in hill
/// country from seed 3 with its standard edit (a trough 14 m deep).
#[test]
fn an_edit_is_the_same_on_any_thread_count() {
    const HELD_AT: (f64, f64) = (900.0, -300.0);
    const HELD_M: f64 = 40.0;
    let example = Drawing::example_valley(2048);
    let shoulder = trial_drawing("shoulder", 1024, Country::hills()).unwrap();
    let cases = [
        ("the example's tributary added", example.clone(), 11, example_edit(&example, "f").unwrap(), Some((HELD_AT, HELD_M))),
        ("shoulder's standard edit", shoulder.clone(), 3, standard_edit(&shoulder).unwrap(), None),
    ];
    for (name, drawing, seed, edited, held) in cases {
        let mut land = Land::grow(&drawing, seed, &GenConfig::default(), &AtomicBool::new(false)).unwrap();
        let size = drawing.size_m as usize;
        if let Some(((x, z), radius_m)) = held {
            let half = size as f64 / 2.0;
            let area: Vec<bool> = (0..size * size).map(|cell| ((cell % size) as f64 - half - x).hypot((cell / size) as f64 - half - z) <= radius_m).collect();
            land.hold(&area, true);
        }
        let edited_on = |threads: usize| {
            on_pool(threads, || {
                let edit = land.edit(&edited, &AtomicBool::new(false)).unwrap();
                let bits = |data: &[f32]| data.iter().map(|v| v.to_bits()).collect::<Vec<u32>>();
                let more: EditBits = (
                    bits(&edit.land.model_heights.data),
                    edit.land.first_lake_levels.iter().map(|lake| (lake.drawn, lake.level_m.to_bits())).collect(),
                    bits(&edit.outline.weight.data),
                    bits(&edit.outline.model_weight),
                    edit.troughs.iter().map(|t| [t.at.0.to_bits(), t.at.1.to_bits(), u64::from(t.depth_m.to_bits()) << 1 | u64::from(t.filled)]).collect(),
                    edit.land.marks.held.clone(),
                    edit.notes.clone(),
                    edit.notes_about.clone(),
                    edit.land.world.notes.clone(),
                    edit.land.world.notes_about.clone(),
                );
                (Bits::of(edit.land.world), more, edit.troughs.len())
            })
        };
        let (one, one_more, troughs) = edited_on(1);
        assert!(!one.rivers.is_empty() && one_more.2.iter().any(|&w| w != 0), "{name}: an edit that changes something");
        println!("{name}: {troughs} troughs, {} notes", one_more.6.len());
        for threads in [4, 16] {
            let (again, again_more, _) = edited_on(threads);
            assert!(one == again, "{name}: on {threads} threads, differing values: {:?}", one.differences(&again));
            assert!(one_more == again_more, "{name}: on {threads} threads, the model, the outline, the troughs, the marks or the notes differ");
        }
    }
}
