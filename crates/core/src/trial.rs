//! The trial drawings: seventeen small drawings that put a world's water where it is hard to hold,
//! each grown in every kind of country to count what the ground does not hold (`held::unheld`).
//! And the edits the editor tool is measured with: nine on the example valley (`example_edit`) and
//! one on each trial drawing (`standard_edit`).

use crate::country::Country;
use crate::drawing::{Drawing, Lake, River};
use crate::grid::Grid;

/// The names of the trial drawings, in the order the counting program grows them.
pub const TRIAL_NAMES: [&str; 17] = [
    "shoulder", "ridge", "lake3", "parallel", "cross", "zigzag", "edge", "lonelake", "twoout", "wide", "thin", "chain", "headjoin", "overpeak", "repeated",
    "none", "flat",
];

/// The names of the editor tool's test edits on the example valley (`example_edit`), in the order
/// of the first table of its prototype's second report (`report2.md`, table 1), not the order the
/// prototype ran them in (it ran `c4`, which is not one of these, after `c`, and `h` after `d`).
pub const EDIT_NAMES: [&str; 9] = ["a", "b", "c", "d", "e1", "e2", "f", "g", "h"];

/// Cells along a side of the picture of high ground `ridge` draws.
const RIDGE_PICTURE_PX: usize = 64;

/// A hill a test edit paints adds this much to the picture of high ground at its top (a quarter of
/// the picture's range, which runs from 0 to 1), never taking a pixel above 1...
const HILL_RISE: f64 = 0.25;
/// ...and falls off from its top as a Gaussian of this many metres.
const HILL_SPREAD_M: f64 = 25.0;
/// The standard edit moves its river's second point this many metres across the river's line...
const STANDARD_MOVE_M: f64 = 40.0;
/// ...and on a drawing without a river paints a hill this many halves of the size east and north of
/// the centre.
const STANDARD_HILL_AT: f64 = 0.68;

/// A picture of high ground `size_m` across: a ridge running north to south, its crest `at` from
/// the middle (-1 the west edge, 1 the east), falling to nothing `wide` away on either side.
fn ridge_picture(size_m: u32, at: f64, wide: f64) -> Grid<f32> {
    let side = RIDGE_PICTURE_PX;
    let mut picture = Grid::<f32>::new(side, side, (f64::from(size_m) / side as f64) as f32);
    for row in 0..side {
        for column in 0..side {
            let x = ((column as f64 + 0.5) / side as f64 - 0.5) * 2.0;
            picture.set(column, row, (1.0 - (x - at).abs() / wide).max(0.0) as f32);
        }
    }
    picture
}

/// The trial drawing called `name`, `size_m` across, in `country`; `None` for a name that is not
/// in `TRIAL_NAMES`. Points are in halves of the size from the centre, as the design's appendix
/// writes them. All but `ridge` and `flat` use the example's picture of high ground, and all but
/// `ridge`, `lake3` and `flat` the example's rivers and lakes where they name none of their own.
pub fn trial_drawing(name: &str, size_m: u32, country: Country) -> Option<Drawing> {
    let h = f64::from(size_m) / 2.0;
    let river = |points: &[(f64, f64)], width_m: f32| River { points: points.iter().map(|&(x, z)| (x * h, z * h)).collect(), width_m };
    let lake = |x: f64, z: f64, radius_m: f32| Lake { x_m: x * h, z_m: z * h, radius_m };
    // The example's picture of high ground, the country asked for, and these rivers and lakes.
    let on_the_example = |rivers: Vec<River>, lakes: Vec<Lake>, country: Country| Drawing { rivers, lakes, country, ..Drawing::example_valley(size_m) };
    let drawing = match name {
        "shoulder" => on_the_example(vec![river(&[(0.8, 0.2), (0.4, 0.1), (0.0, 0.0)], 4.0)], vec![], country),
        "ridge" => Drawing {
            size_m,
            high_ground: Some(ridge_picture(size_m, 0.0, 0.25)),
            rivers: vec![river(&[(0.0, 0.4), (0.45, 0.2), (0.95, -0.1)], 4.0)],
            lakes: vec![],
            country,
        },
        "lake3" => Drawing {
            size_m,
            high_ground: None,
            rivers: vec![
                river(&[(-1.0, -0.5), (-0.4, -0.2), (0.0, 0.0), (0.5, 0.2), (1.0, 0.4)], 12.0),
                river(&[(-0.3, -1.0), (-0.1, -0.5), (0.0, -0.05)], 6.0),
                river(&[(-0.5, 1.0), (-0.2, 0.5), (-0.02, 0.05)], 5.0),
            ],
            lakes: vec![lake(0.0, 0.0, (0.12 * h) as f32)],
            country,
        },
        "parallel" => on_the_example(vec![river(&[(-1.0, -0.02), (0.0, -0.03), (1.0, -0.02)], 8.0), river(&[(-1.0, 0.03), (0.0, 0.02), (1.0, 0.03)], 6.0)], vec![], country),
        "cross" => on_the_example(vec![river(&[(-0.9, -0.9), (0.0, 0.0), (0.9, 0.9)], 10.0), river(&[(-0.9, 0.9), (0.0, 0.0), (0.9, -0.9)], 6.0)], vec![], country),
        "zigzag" => on_the_example(
            vec![
                river(&[(-0.9, 0.0), (-0.6, 0.15), (-0.45, -0.15), (-0.3, 0.15), (-0.15, -0.15), (0.0, 0.15), (0.15, -0.15), (0.5, 0.0), (1.0, 0.0)], 30.0),
                river(&[(-0.3, 0.8), (-0.3, 0.5), (-0.3, 0.17)], 2.0),
            ],
            vec![],
            country,
        ),
        "edge" => on_the_example(
            vec![river(&[(-0.9, -0.985), (0.0, -0.99), (0.9, -0.985)], 8.0), river(&[(-0.5, 0.5), (0.3, 0.2), (0.9, 0.0)], 10.0)],
            vec![lake(0.93, 0.0, (0.12 * h) as f32)],
            country,
        ),
        "lonelake" => on_the_example(vec![river(&[(-0.9, 0.7), (0.0, 0.8), (0.9, 0.7)], 8.0)], vec![lake(0.2, -0.4, (0.1 * h) as f32)], country),
        "twoout" => on_the_example(
            vec![
                river(&[(-0.9, -0.6), (-0.4, -0.3), (0.0, 0.0)], 10.0),
                river(&[(0.0, 0.0), (0.4, 0.4), (0.95, 0.6)], 8.0),
                river(&[(0.0, 0.0), (0.3, -0.5), (0.6, -0.97)], 6.0),
            ],
            vec![lake(0.0, 0.0, (0.1 * h) as f32)],
            country,
        ),
        "wide" => on_the_example(
            vec![river(&[(-0.95, 0.1), (-0.3, 0.0), (0.3, 0.05), (0.95, -0.1)], 60.0), river(&[(0.1, -0.9), (0.15, -0.4), (0.2, 0.02)], 1.0)],
            vec![lake(-0.3, 0.0, 40.0)],
            country,
        ),
        "thin" => on_the_example(vec![river(&[(-0.95, 0.1), (-0.3, 0.0), (0.3, 0.05), (0.95, -0.1)], 0.5), river(&[(0.1, -0.9), (0.15, -0.4), (0.2, 0.02)], 1.0)], vec![], country),
        "chain" => on_the_example(
            vec![river(&[(-0.95, -0.4), (-0.5, -0.2), (0.0, 0.0), (0.5, 0.2), (0.95, 0.4)], 10.0)],
            vec![lake(-0.5, -0.2, (0.08 * h) as f32), lake(0.5, 0.2, (0.1 * h) as f32)],
            country,
        ),
        "headjoin" => on_the_example(
            vec![
                river(&[(-0.2, 0.0), (0.4, 0.1), (0.95, 0.3)], 8.0),
                river(&[(-0.2, -0.8), (-0.25, -0.4), (-0.2, 0.0)], 4.0),
                river(&[(-0.8, 0.5), (-0.5, 0.2), (-0.2, 0.01)], 3.0),
            ],
            vec![],
            country,
        ),
        // The example's highest blob lies north-east; a river is drawn straight over the middle of the
        // map's high ground.
        "overpeak" => on_the_example(vec![river(&[(-0.9, -0.9), (-0.3, -0.6), (0.3, -0.6), (0.9, -0.9)], 6.0), river(&[(0.6, 0.9), (0.6, 0.0), (0.6, -0.72)], 3.0)], vec![], country),
        // Points drawn twice and three times on one place, and a river 0.5 m long.
        "repeated" => on_the_example(
            vec![
                River { points: vec![(-0.9 * h, 0.0), (-0.9 * h, 0.0), (0.0, 10.0), (0.0, 10.0), (0.0, 10.0), (0.9 * h, 0.0), (0.9 * h, 0.0)], width_m: 6.0 },
                River { points: vec![(100.0, -200.0), (100.5, -200.0)], width_m: 3.0 },
            ],
            vec![],
            country,
        ),
        "none" => on_the_example(vec![], vec![], country),
        "flat" => Drawing { size_m, high_ground: None, rivers: vec![river(&[(-0.95, 0.0), (0.95, 0.0)], 10.0)], lakes: vec![lake(0.0, 0.0, (0.2 * h) as f32)], country },
        _ => return None,
    };
    Some(drawing)
}

/// One of the editor tool's test edits (`EDIT_NAMES`) made to `drawing`, or `None` for a name not
/// in the list or a drawing without what the edit changes (a river, a river's point, or a picture
/// of high ground). They are the nine edits its prototype measured (`edit_probe.rs` and
/// `report2.md` in the plan's workspace), made exactly as it made them, for the example valley at
/// 2,048 m (`Drawing::example_valley(2048)`) in any kind of country, and written in that world's
/// metres; made to another drawing they change the same rivers' points by the same metres and put
/// the same things at the same places, which may give a drawing `Drawing::validate` refuses. A
/// moved point is held within the world, half the size from the centre on each axis. Made to a
/// drawing already edited, an edit changes that drawing (the prototype's sequence of edits).
///
/// - `a`: the brook's (river 2's) second point moved 30 m east; `b`: the same point moved 150 m
///   east.
/// - `c`: the main river's (river 1's) fourth point, its middle one, moved 60 m across its line,
///   square to the way from its third point to its fifth, to the side that keeps the brook joined
///   to it.
/// - `d`: a hill painted on the picture of high ground at (700, -700): `HILL_RISE` at its top,
///   a Gaussian of `HILL_SPREAD_M`.
/// - `e1`: a lake of 60 m added at (-500, 300), away from the rivers; `e2`: a lake of 60 m added
///   at (104, 296), on the brook.
/// - `f`: a tributary 4 m wide added through (620, -320), (600, -100) and (628, 150).
/// - `g`: the brook removed.
/// - `h`: the main river's second point moved 250 m across its line, square to the way from its
///   first point to its third, which lengthens the river by about 140 m.
pub fn example_edit(drawing: &Drawing, name: &str) -> Option<Drawing> {
    let with_lake = |lake: Lake| {
        let mut edited = drawing.clone();
        edited.lakes.push(lake);
        Some(edited)
    };
    match name {
        "a" => moved(drawing, 1, 1, (30.0, 0.0)),
        "b" => moved(drawing, 1, 1, (150.0, 0.0)),
        "c" => moved(drawing, 0, 3, across(drawing, 0, (2, 4), 60.0)?),
        "d" => hill(drawing, (700.0, -700.0)),
        "e1" => with_lake(Lake { x_m: -500.0, z_m: 300.0, radius_m: 60.0 }),
        "e2" => with_lake(Lake { x_m: 104.0, z_m: 296.0, radius_m: 60.0 }),
        "f" => {
            let mut edited = drawing.clone();
            edited.rivers.push(River { points: vec![(620.0, -320.0), (600.0, -100.0), (628.0, 150.0)], width_m: 4.0 });
            Some(edited)
        }
        "g" => {
            let mut edited = drawing.clone();
            (edited.rivers.len() > 1).then(|| edited.rivers.remove(1))?;
            Some(edited)
        }
        "h" => moved(drawing, 0, 1, across(drawing, 0, (0, 2), 250.0)?),
        _ => None,
    }
}

/// The standard edit of a trial drawing, as the editor tool's prototype made it on each of
/// `TRIAL_NAMES`: the first river's second point moved `STANDARD_MOVE_M` across its line, square to
/// the way from its first point to its third (to its second when it has only two), held within the
/// world; on a drawing without a river, a hill as `example_edit`'s `d` paints it, at
/// (`STANDARD_HILL_AT`, -`STANDARD_HILL_AT`) halves of the size from the centre. `None` for a
/// drawing with neither a river nor a picture of high ground, or whose first river has fewer than
/// two points.
pub fn standard_edit(drawing: &Drawing) -> Option<Drawing> {
    let Some(river) = drawing.rivers.first() else {
        let half = f64::from(drawing.size_m) / 2.0;
        return hill(drawing, (STANDARD_HILL_AT * half, -STANDARD_HILL_AT * half));
    };
    let to = if river.points.len() > 2 { 2 } else { 1 };
    moved(drawing, 0, 1, across(drawing, 0, (0, to), STANDARD_MOVE_M)?)
}

/// `drawing` with point `point` of river `river` moved by `(dx, dz)` metres and then held within the
/// world; `None` when there is no such point.
fn moved(drawing: &Drawing, river: usize, point: usize, (dx, dz): (f64, f64)) -> Option<Drawing> {
    let mut edited = drawing.clone();
    let half = f64::from(drawing.size_m) / 2.0;
    let place = edited.rivers.get_mut(river)?.points.get_mut(point)?;
    place.0 += dx;
    place.1 += dz;
    place.0 = place.0.clamp(-half, half);
    place.1 = place.1.clamp(-half, half);
    Some(edited)
}

/// A move of `by_m` metres square to the way from point `from` to point `to` of river `river`, a
/// quarter turn from it (`(-dz, dx)` for a way `(dx, dz)`); no move when the two points lie on one
/// place, and `None` when either point is missing.
fn across(drawing: &Drawing, river: usize, (from, to): (usize, usize), by_m: f64) -> Option<(f64, f64)> {
    let points = &drawing.rivers.get(river)?.points;
    let (a, b) = (*points.get(from)?, *points.get(to)?);
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let length = dx.hypot(dz);
    if length == 0.0 {
        return Some((0.0, 0.0));
    }
    Some((-dz / length * by_m, dx / length * by_m))
}

/// The hill of `example_edit`'s `d` painted at world `at`, in metres, instead of at (700, -700): what
/// a test paints beside ground it holds. `None` for a drawing without a picture of high ground.
pub fn hill_edit(drawing: &Drawing, at: (f64, f64)) -> Option<Drawing> {
    hill(drawing, at)
}

/// `drawing` with a hill painted on its picture of high ground at world `(x, z)`: each pixel raised
/// by `HILL_RISE` times a Gaussian of `HILL_SPREAD_M` of its centre's distance from `(x, z)`, and
/// held at 1; `None` for a drawing without a picture.
fn hill(drawing: &Drawing, (x, z): (f64, f64)) -> Option<Drawing> {
    let mut edited = drawing.clone();
    let size_m = f64::from(drawing.size_m);
    let picture = edited.high_ground.as_mut()?;
    let per_m = picture.width as f64 / size_m;
    let (column_at, row_at, spread) = ((x + size_m / 2.0) * per_m, (z + size_m / 2.0) * per_m, HILL_SPREAD_M * per_m);
    for row in 0..picture.height {
        for column in 0..picture.width {
            let squared = (column as f64 + 0.5 - column_at).powi(2) + (row as f64 + 0.5 - row_at).powi(2);
            let rise = HILL_RISE * (-squared / (2.0 * spread * spread)).exp();
            let before = picture.get(column, row);
            picture.set(column, row, (f64::from(before) + rise).min(1.0) as f32);
        }
    }
    Some(edited)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::country::COUNTRY_NAMES;

    /// The size the trial drawings are grown at.
    const TRIAL_SIZE_M: u32 = 1024;

    fn named(name: &str, size_m: u32) -> Drawing {
        trial_drawing(name, size_m, Country::hills()).unwrap_or_else(|| panic!("{name} is a trial drawing"))
    }

    #[test]
    fn every_trial_drawing_is_valid_in_every_kind_of_country() {
        assert_eq!(TRIAL_NAMES.len(), 17);
        for name in TRIAL_NAMES {
            for country in COUNTRY_NAMES {
                let drawing = trial_drawing(name, TRIAL_SIZE_M, Country::named(country).expect("a known country")).unwrap_or_else(|| panic!("{name} is a trial drawing"));
                assert_eq!(drawing.size_m, TRIAL_SIZE_M, "{name}");
                assert_eq!(drawing.country, Country::named(country).expect("a known country"), "{name}");
                assert_eq!(drawing.validate(), Ok(()), "{name} in {country} country");
            }
        }
    }

    #[test]
    fn the_names_are_distinct() {
        let mut names = TRIAL_NAMES.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TRIAL_NAMES.len());
    }

    #[test]
    fn an_unknown_name_gives_no_drawing() {
        assert_eq!(trial_drawing("example", TRIAL_SIZE_M, Country::hills()), None);
        assert_eq!(trial_drawing("", TRIAL_SIZE_M, Country::hills()), None);
        assert_eq!(trial_drawing("Headjoin", TRIAL_SIZE_M, Country::hills()), None);
    }

    /// The strokes the spec's **The measure** gives, `(x, z)` in halves of the size from the centre:
    /// each river's points and width, and each lake's centre and its radius in metres.
    type Strokes = (Vec<(Vec<(f64, f64)>, f32)>, Vec<((f64, f64), f32)>);

    fn strokes_of(drawing: &Drawing) -> Strokes {
        let half = f64::from(drawing.size_m) / 2.0;
        let rivers = drawing.rivers.iter().map(|river| (river.points.iter().map(|&(x, z)| (x / half, z / half)).collect(), river.width_m)).collect();
        let lakes = drawing.lakes.iter().map(|lake| ((lake.x_m / half, lake.z_m / half), lake.radius_m)).collect();
        (rivers, lakes)
    }

    #[test]
    fn headjoin_twoout_and_wide_have_the_strokes_of_the_spec() {
        for size_m in [1024, 2048] {
            let half = f64::from(size_m) / 2.0;
            let example = Drawing::example_valley(size_m);
            let head_join = named("headjoin", size_m);
            let rivers = vec![
                (vec![(-0.2, 0.0), (0.4, 0.1), (0.95, 0.3)], 8.0),
                (vec![(-0.2, -0.8), (-0.25, -0.4), (-0.2, 0.0)], 4.0),
                (vec![(-0.8, 0.5), (-0.5, 0.2), (-0.2, 0.01)], 3.0),
            ];
            assert_eq!(strokes_of(&head_join), (rivers, vec![]), "headjoin at {size_m} m");
            assert_eq!(head_join.high_ground, example.high_ground, "headjoin uses the example's picture");

            let two_out = named("twoout", size_m);
            let rivers = vec![
                (vec![(-0.9, -0.6), (-0.4, -0.3), (0.0, 0.0)], 10.0),
                (vec![(0.0, 0.0), (0.4, 0.4), (0.95, 0.6)], 8.0),
                (vec![(0.0, 0.0), (0.3, -0.5), (0.6, -0.97)], 6.0),
            ];
            assert_eq!(strokes_of(&two_out), (rivers, vec![((0.0, 0.0), (0.1 * half) as f32)]), "twoout at {size_m} m");
            assert_eq!(two_out.high_ground, example.high_ground, "twoout uses the example's picture");

            let wide = named("wide", size_m);
            let rivers = vec![(vec![(-0.95, 0.1), (-0.3, 0.0), (0.3, 0.05), (0.95, -0.1)], 60.0), (vec![(0.1, -0.9), (0.15, -0.4), (0.2, 0.02)], 1.0)];
            assert_eq!(strokes_of(&wide), (rivers, vec![((-0.3, 0.0), 40.0)]), "wide at {size_m} m");
            assert_eq!(wide.high_ground, example.high_ground, "wide uses the example's picture");
        }
    }

    #[test]
    fn ridge_and_flat_keep_their_own_picture_and_the_others_use_the_examples() {
        let example = Drawing::example_valley(TRIAL_SIZE_M).high_ground;
        assert!(example.is_some());
        for name in TRIAL_NAMES {
            let picture = named(name, TRIAL_SIZE_M).high_ground;
            match name {
                "flat" | "lake3" => assert_eq!(picture, None, "{name} has no picture"),
                "ridge" => {
                    let picture = picture.expect("ridge draws a picture");
                    assert_ne!(Some(&picture), example.as_ref());
                    // Highest along the middle column, nothing beyond a quarter of the way out.
                    let crest = picture.get(picture.width / 2, 0);
                    assert!(crest > 0.9, "the crest is {crest}");
                    assert_eq!(picture.get(0, 0), 0.0);
                    assert_eq!(picture.get(picture.width - 1, 0), 0.0);
                }
                _ => assert_eq!(picture, example, "{name} uses the example's picture"),
            }
        }
    }

    #[test]
    fn the_same_name_gives_the_same_drawing() {
        for name in TRIAL_NAMES {
            assert_eq!(named(name, TRIAL_SIZE_M), named(name, TRIAL_SIZE_M), "{name}");
        }
    }

    #[test]
    fn each_edit_of_the_example_changes_what_it_says_and_nothing_else() {
        for country in COUNTRY_NAMES {
            let example = Drawing { country: Country::named(country).expect("a known country"), ..Drawing::example_valley(2048) };
            for name in EDIT_NAMES {
                let edited = example_edit(&example, name).unwrap_or_else(|| panic!("{name} edits the example"));
                assert_eq!(edited.validate(), Ok(()), "{name} in {country} country");
                assert_ne!(edited, example, "{name} changes the drawing");
                assert_eq!((edited.size_m, edited.country), (example.size_m, example.country), "{name}");
                let changes_picture = name == "d";
                assert_eq!(edited.high_ground != example.high_ground, changes_picture, "{name} and the picture");
            }
        }
        let example = Drawing::example_valley(2048);
        let edit = |name: &str| example_edit(&example, name).expect("an edit of the example");
        let moved = |edited: &Drawing, river: usize, point: usize| {
            let (before, after) = (example.rivers[river].points[point], edited.rivers[river].points[point]);
            let mut others = edited.clone();
            others.rivers[river].points[point] = before;
            assert_eq!(others, example, "only river {} point {} moves", river + 1, point + 1);
            (after.0 - before.0, after.1 - before.1)
        };
        assert_eq!(moved(&edit("a"), 1, 1), (30.0, 0.0));
        assert_eq!(moved(&edit("b"), 1, 1), (150.0, 0.0));
        for (name, point, by_m) in [("c", 3, 60.0), ("h", 1, 250.0)] {
            let (dx, dz) = moved(&edit(name), 0, point);
            assert!((dx.hypot(dz) - by_m).abs() < 1e-9, "{name} moves its point {by_m} m: ({dx}, {dz})");
        }
        assert_eq!(edit("e1").lakes[1..], [Lake { x_m: -500.0, z_m: 300.0, radius_m: 60.0 }]);
        assert_eq!(edit("e2").lakes[1..], [Lake { x_m: 104.0, z_m: 296.0, radius_m: 60.0 }]);
        assert_eq!(edit("f").rivers[2], River { points: vec![(620.0, -320.0), (600.0, -100.0), (628.0, 150.0)], width_m: 4.0 });
        assert_eq!(edit("g").rivers, example.rivers[..1]);
        // The hill tops out at (700, -700) by a quarter of the picture's range, at most.
        let (before, after) = (example.high_ground.as_ref().expect("a picture"), edit("d").high_ground.expect("a picture"));
        let rises: Vec<f32> = after.data.iter().zip(&before.data).map(|(a, b)| a - b).collect();
        let (top, _) = rises.iter().enumerate().fold((0, f32::MIN), |best, (pixel, &rise)| if rise > best.1 { (pixel, rise) } else { best });
        let per_m = before.width as f64 / 2048.0;
        let at = |index: usize| (index as f64 + 0.5) / per_m - 1024.0;
        assert!((at(top % before.width) - 700.0).abs() <= 1.0 / per_m && (at(top / before.width) + 700.0).abs() <= 1.0 / per_m);
        assert!(rises.iter().all(|&rise| (0.0..=0.25).contains(&rise)));
        // An unknown name, and a drawing without what an edit changes.
        assert_eq!(example_edit(&example, "i"), None);
        let bare = Drawing { high_ground: None, rivers: Vec::new(), ..example.clone() };
        for name in ["a", "b", "c", "d", "g", "h"] {
            assert_eq!(example_edit(&bare, name), None, "{name} on a drawing without rivers or a picture");
        }
    }

    /// Every trial drawing has a standard edit, and it is the move `standard_edit` says: the first
    /// river's second point moved `STANDARD_MOVE_M` square to the way from its first point to its
    /// third (its second when it has two), a quarter turn from that way (`(-dz, dx)`), or the hill.
    #[test]
    fn the_standard_edit_changes_every_trial_drawing() {
        /// The move is asked to be this exact, in metres.
        const MOVE_WITHIN_M: f64 = 1e-9;
        let mut moves = 0;
        for name in TRIAL_NAMES {
            let drawing = named(name, TRIAL_SIZE_M);
            let edited = standard_edit(&drawing).unwrap_or_else(|| panic!("{name} has a standard edit"));
            assert_eq!(edited.validate(), Ok(()), "{name}");
            assert_ne!(edited, drawing, "{name}");
            if drawing.rivers.is_empty() {
                assert_eq!(Drawing { high_ground: drawing.high_ground.clone(), ..edited.clone() }, drawing, "{name}: only the picture changes");
            } else {
                let mut others = edited.clone();
                others.rivers[0].points[1] = drawing.rivers[0].points[1];
                assert_eq!(others, drawing, "{name}: only the first river's second point moves");
                let points = &drawing.rivers[0].points;
                let to = if points.len() > 2 { 2 } else { 1 };
                let (dx, dz) = (points[to].0 - points[0].0, points[to].1 - points[0].1);
                let length = dx.hypot(dz);
                let (before, after) = (points[1], edited.rivers[0].points[1]);
                let (moved_x, moved_z) = (after.0 - before.0, after.1 - before.1);
                // No trial's point lies within the move of the world's edge, so none is held back.
                assert!(
                    (moved_x.hypot(moved_z) - STANDARD_MOVE_M).abs() < MOVE_WITHIN_M,
                    "{name}: the point moves {STANDARD_MOVE_M} m, not {}",
                    moved_x.hypot(moved_z)
                );
                let (side_x, side_z) = (-dz / length * STANDARD_MOVE_M, dx / length * STANDARD_MOVE_M);
                assert!(
                    (moved_x - side_x).abs() < MOVE_WITHIN_M && (moved_z - side_z).abs() < MOVE_WITHIN_M,
                    "{name}: the point moves ({moved_x}, {moved_z}), a quarter turn from the way to point {} is ({side_x}, {side_z})",
                    to + 1
                );
                moves += 1;
            }
        }
        assert!(moves > 0 && moves < TRIAL_NAMES.len(), "{moves} trials move a point, the others paint a hill");
        let bare = Drawing { high_ground: None, rivers: Vec::new(), ..named("none", TRIAL_SIZE_M) };
        assert_eq!(standard_edit(&bare), None);
    }
}
