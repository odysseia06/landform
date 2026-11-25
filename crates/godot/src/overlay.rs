//! The picture the editor tool lays over a land (`LandformOverlay`): one `Image.FORMAT_RGBA8` of the
//! land's size, a pixel a cell of 1 m, made in the extension's pool from the outline of a regrowth,
//! the held cells, the cells shaped by hand and an area to be regrown, each drawn in a colour the
//! caller gives. A view lays it over the ground as a `Decal`'s texture: Godot's decals take
//! textures and no shader, so the picture is composed here, where 4 million cells take a few
//! milliseconds.

use godot::classes::image::Format;
use godot::classes::Image;
use godot::prelude::*;
use landform_core::world::MAX_SIZE_CELLS;
use rayon::prelude::*;

use landform_core::Marks;

use crate::land::AREA_LEAST;
use crate::outline::{LandformOutline, WEIGHT_FULL, WEIGHT_NONE};
use crate::pool;
use crate::world::{fits, image_from, or_report};


/// The widest the outline's rim and the band's inner edge are drawn, in cells: an edge of 64 m is
/// wider than any view of a land needs, and it bounds the work of finding the edge.
pub(crate) const EDGE_CELLS_MOST: i64 = 64;

/// The bytes of one pixel of the picture: red, green, blue and alpha.
const PIXEL_BYTES: usize = 4;

/// The largest a channel's byte is.
const BYTE_MOST: f32 = 255.0;

/// A colour as four channels from 0 to 1, straight (not premultiplied) alpha.
type Paint = [f32; 4];

/// A colour Godot gave as a `Paint`, each channel held to 0 to 1 (a channel that is not a number
/// reads 0).
pub(crate) fn paint(colour: Color) -> Paint {
    [colour.r, colour.g, colour.b, colour.a].map(|channel| if channel.is_nan() { 0.0 } else { channel.clamp(0.0, 1.0) })
}

/// `top` laid over `under` by its alpha, as a painter lays one colour over another: over a pixel
/// still clear it is `top` exactly.
fn over(under: Paint, top: Paint) -> Paint {
    let [.., under_alpha] = under;
    let [.., top_alpha] = top;
    if under_alpha <= 0.0 {
        return top;
    }
    let alpha = top_alpha + under_alpha * (1.0 - top_alpha);
    if alpha <= 0.0 {
        return [0.0; 4];
    }
    let mix = |channel: usize| (top[channel] * top_alpha + under[channel] * under_alpha * (1.0 - top_alpha)) / alpha;
    [mix(0), mix(1), mix(2), alpha]
}

/// A channel from 0 to 1 as a byte, rounded as Godot rounds `Color.r8`.
fn byte(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * BYTE_MOST).round() as u8
}

/// The colours of what the picture draws, each laid over the ones before it in this order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Colours {
    pub(crate) hand: Paint,
    pub(crate) held: Paint,
    pub(crate) area: Paint,
    pub(crate) band_edge: Paint,
    pub(crate) rim: Paint,
}

/// What the picture is made of, as plain bytes: each part one byte a cell of a land `size` cells a
/// side, in the world's cell order, or `None` for none.
struct Parts<'a> {
    size: usize,
    weights: Option<&'a [u8]>,
    held: Option<&'a [u8]>,
    hand: Option<&'a [u8]>,
    area: Option<&'a [u8]>,
}

/// Composes the picture the editor tool lays over a land. Static functions only:
/// `LandformOverlay.compose(...)`.
#[derive(GodotClass)]
#[class(tool, init, base = RefCounted)]
pub struct LandformOverlay {}

#[godot_api]
impl LandformOverlay {
    /// The overlay of a land `size` cells a side as an `Image.FORMAT_RGBA8` of that size, cell
    /// `(x, z)` at pixel `(x, z)`, as the land's held picture is, made in the extension's pool:
    /// clear (every byte 0) where nothing is drawn, and where something is, its colour laid over
    /// what lies under it by its alpha, in this order:
    ///
    /// - `hand_colour` on the cells shaped by hand, `held_colour` on the held cells and
    ///   `area_colour` on the cells of the area, each where its picture (an `Image.FORMAT_L8` of the
    ///   land's size, as `LandformLand.get_held_image()` and `get_hand_image()` give theirs and
    ///   `start_regrow_area` takes an area) is 128 or more;
    /// - `band_edge_colour` on the band's inner edge: the cells where the outline's weight is 1
    ///   (its picture 255) that lie within `edge_cells` cells of one where it is below 1;
    /// - `rim_colour` on the outline's rim: the cells where the weight is above 0 that lie within
    ///   `edge_cells` cells of one where it is 0, outside the outline or on fixed ground.
    ///
    /// "Within" counts the cells of the square of `2 edge_cells + 1` cells a side round a cell, and
    /// none beyond the land, so an outline that reaches the land's border has no rim along it. Over
    /// a clear pixel a colour is drawn exactly, so a held cell under nothing else reads
    /// `held_colour`'s bytes. `outline`, `held`, `hand` and `area` may each be `null`, for nothing
    /// of it. Reports an error and returns `null` for a size outside 1 to 8,192, an `edge_cells`
    /// outside 1 to 64, an outline of another size, and a picture of another size or format or with
    /// mipmaps.
    #[func]
    #[allow(clippy::too_many_arguments)]
    fn compose(
        size: i64,
        outline: Option<Gd<LandformOutline>>,
        held: Option<Gd<Image>>,
        hand: Option<Gd<Image>>,
        area: Option<Gd<Image>>,
        rim_colour: Color,
        band_edge_colour: Color,
        held_colour: Color,
        hand_colour: Color,
        area_colour: Color,
        edge_cells: i64,
    ) -> Option<Gd<Image>> {
        let colours = Colours {
            hand: paint(hand_colour),
            held: paint(held_colour),
            area: paint(area_colour),
            band_edge: paint(band_edge_colour),
            rim: paint(rim_colour),
        };
        or_report(compose(size, outline, [held, hand, area], colours, edge_cells))
    }
}

fn compose(size: i64, outline: Option<Gd<LandformOutline>>, pictures: [Option<Gd<Image>>; 3], colours: Colours, edge_cells: i64) -> Result<Gd<Image>, String> {
    if !(1..=MAX_SIZE_CELLS as i64).contains(&size) {
        return Err(format!("compose: the land's size is {size} cells; it is 1 to {MAX_SIZE_CELLS}."));
    }
    if !(1..=EDGE_CELLS_MOST).contains(&edge_cells) {
        return Err(format!("compose: edge_cells is {edge_cells}; the rim and the band's edge are 1 to {EDGE_CELLS_MOST} cells wide."));
    }
    let side = size as usize;
    let outline = outline.as_ref().map(|outline| outline.bind());
    let weights = match outline.as_ref().and_then(|outline| outline.weights()) {
        Some((_, outline_side)) if outline_side != side => {
            return Err(format!("compose: the outline is of a land {outline_side} cells a side and the overlay {size}."));
        }
        found => found.map(|(weights, _)| weights),
    };
    let [held, hand, area] = pictures;
    // Godot's bytes, shared and not copied, held here while the pool reads them.
    let mut bytes = Vec::new();
    for (name, picture) in [("held", &held), ("hand", &hand), ("area", &area)] {
        bytes.push(match picture {
            Some(picture) if !fits(picture, Format::L8, size) => {
                return Err(format!(
                    "compose: the {name} picture is {} x {} pixels in FORMAT_{:?}{} and the land {size} x {size} cells: give an Image.FORMAT_L8 of the land's size, without mipmaps.",
                    picture.get_width(),
                    picture.get_height(),
                    picture.get_format(),
                    if picture.has_mipmaps() { " with mipmaps" } else { "" },
                ));
            }
            Some(picture) => Some(picture.get_data()),
            None => None,
        });
    }
    let slice = |at: usize| bytes[at].as_ref().map(|data| data.as_slice());
    let parts = Parts { size: side, weights, held: slice(0), hand: slice(1), area: slice(2) };
    let pixels = pool::run(|| overlay_bytes(&parts, colours, edge_cells as usize))?;
    Ok(image_from(side, side, Format::RGBA8, pixels))
}

/// The overlay's bytes of an outline's `weights` with a land's `marks` (its held and hand-shaped
/// cells, a mark beyond the marks' length off) and no area: `LandformOverlay.compose`'s picture of
/// that outline over that land, as an outline's thread makes it beside the outline, so the main
/// thread only makes the image (stage 2's ruling R21, N3). Inside `pool::run` only.
pub(crate) fn marks_overlay(size: usize, weights: &[u8], marks: &Marks, colours: Colours, edge_cells: usize) -> Vec<u8> {
    let picture = |marks: &[bool]| {
        marks.contains(&true).then(|| (0..size * size).into_par_iter().map(|cell| if marks.get(cell) == Some(&true) { u8::MAX } else { 0 }).collect::<Vec<u8>>())
    };
    let (held, hand) = rayon::join(|| picture(&marks.held), || picture(&marks.by_hand));
    let parts = Parts { size, weights: Some(weights), held: held.as_deref(), hand: hand.as_deref(), area: None };
    overlay_bytes(&parts, colours, edge_cells)
}

/// The picture's bytes (`LandformOverlay.compose`'s rule). Rows run in parallel and each pixel reads
/// only its own cells and those within `reach`, so the result does not depend on threads. Inside
/// `pool::run` only.
fn overlay_bytes(parts: &Parts, colours: Colours, reach: usize) -> Vec<u8> {
    let size = parts.size;
    let edges = parts.weights.map(|weights| {
        rayon::join(
            || near(weights, size, reach, |weight| weight == WEIGHT_NONE),
            || near(weights, size, reach, |weight| weight != WEIGHT_FULL),
        )
    });
    let mut pixels = vec![0; size * size * PIXEL_BYTES];
    pixels.par_chunks_mut(size * PIXEL_BYTES).enumerate().for_each(|(z, row)| {
        let cells = z * size..(z + 1) * size;
        let marked = |picture: Option<&[u8]>, cell: usize| picture.is_some_and(|picture| picture[cell] >= AREA_LEAST);
        for (pixel, cell) in row.as_chunks_mut::<PIXEL_BYTES>().0.iter_mut().zip(cells) {
            let mut colour: Paint = [0.0; 4];
            let mut drawn = false;
            let mut lay = |on: bool, top: Paint| {
                if on {
                    colour = over(colour, top);
                    drawn = true;
                }
            };
            lay(marked(parts.hand, cell), colours.hand);
            lay(marked(parts.held, cell), colours.held);
            lay(marked(parts.area, cell), colours.area);
            if let (Some(weights), Some((near_none, near_short))) = (parts.weights, &edges) {
                let weight = weights[cell];
                lay(weight == WEIGHT_FULL && near_short[cell], colours.band_edge);
                lay(weight != WEIGHT_NONE && near_none[cell], colours.rim);
            }
            if drawn {
                *pixel = colour.map(byte);
            }
        }
    });
    pixels
}

/// For each cell of a grid `size` cells a side, in its cell order, whether a cell whose value `of`
/// takes lies in the square of `2 reach + 1` cells a side round it, itself among them; no cell
/// beyond the grid is counted. Along each row by running counts, then down the columns by each row
/// taking in the rows within `reach`; rows run in parallel.
fn near(cells: &[u8], size: usize, reach: usize, of: impl Fn(u8) -> bool + Sync) -> Vec<bool> {
    let mut across = vec![false; size * size];
    across.par_chunks_mut(size).zip(cells.par_chunks(size)).for_each(|(out, row)| {
        let mut counts = Vec::with_capacity(size + 1);
        counts.push(0_u32);
        let mut count = 0;
        for &value in row {
            count += u32::from(of(value));
            counts.push(count);
        }
        for (x, out) in out.iter_mut().enumerate() {
            *out = counts[(x + reach + 1).min(size)] > counts[x.saturating_sub(reach)];
        }
    });
    let mut near = vec![false; size * size];
    near.par_chunks_mut(size).enumerate().for_each(|(z, out)| {
        for row in z.saturating_sub(reach)..(z + reach + 1).min(size) {
            for (out, &found) in out.iter_mut().zip(&across[row * size..(row + 1) * size]) {
                *out |= found;
            }
        }
    });
    near
}

/// Nothing here starts rayon's global pool: `near` and `overlay_bytes` run under a pool of the
/// test's own, as `pool.rs`'s test of the global pool requires.
#[cfg(test)]
mod tests {
    use super::*;

    const RED: Paint = [1.0, 0.0, 0.0, 1.0];
    const CYAN: Paint = [0.0, 0.8, 0.9, 0.4];
    const MAGENTA: Paint = [0.9, 0.0, 0.9, 0.5];
    const AMBER: Paint = [1.0, 0.7, 0.0, 1.0];
    const GREEN: Paint = [0.2, 0.9, 0.2, 0.3];
    const COLOURS: Colours = Colours { hand: GREEN, held: CYAN, area: MAGENTA, band_edge: AMBER, rim: RED };

    /// `work` on a pool of two threads of the test's own.
    fn in_pool<R: Send>(work: impl FnOnce() -> R + Send) -> R {
        rayon::ThreadPoolBuilder::new().num_threads(2).build().expect("a pool of two threads").install(work)
    }

    #[test]
    fn a_colour_over_a_clear_pixel_is_itself_and_over_another_is_laid_by_its_alpha() {
        assert_eq!(over([0.0; 4], CYAN), CYAN);
        assert_eq!(over(MAGENTA, RED), RED, "an opaque colour hides what lies under it");
        let laid = over([0.0, 0.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(laid.map(byte), [128, 0, 128, 255]);
        let thin = over([0.0, 0.0, 1.0, 0.5], [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(thin[3], 0.75, "two halves cover three quarters");
        assert_eq!(paint(Color::from_rgba(2.0, -1.0, f32::NAN, 0.5)), [1.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn near_finds_a_cell_within_its_square_and_none_beyond_it() {
        // A 7 x 7 grid with one cell marked at (5, 1).
        let size = 7;
        let mut cells = vec![0_u8; size * size];
        cells[size + 5] = 1;
        let found = in_pool(|| near(&cells, size, 2, |value| value == 1));
        for z in 0..size {
            for x in 0..size {
                let within = x.abs_diff(5) <= 2 && z.abs_diff(1) <= 2;
                assert_eq!(found[z * size + x], within, "at ({x}, {z})");
            }
        }
        assert!(in_pool(|| near(&cells, size, 1, |value| value == 2)).iter().all(|&found| !found));
    }

    #[test]
    fn the_overlay_draws_the_rim_the_bands_edge_and_the_marks_in_their_order() {
        // An outline on a 16 x 16 land: weight 0 on the two western columns, 1 to 254 on the next
        // four, 255 on the rest; the cells (10, 10) held, (11, 10) and (10, 10) in the area, (12, 10)
        // shaped by hand.
        let size = 16;
        let weights: Vec<u8> = (0..size * size).map(|cell| match cell % size { 0 | 1 => 0, 2..=5 => 128, _ => 255 }).collect();
        let mark = |at: &[usize]| -> Vec<u8> { (0..size * size).map(|cell| if at.contains(&cell) { 255 } else { 0 }).collect() };
        let (held, area, hand) = (mark(&[10 * size + 10]), mark(&[10 * size + 11, 10 * size + 10]), mark(&[10 * size + 12]));
        let parts = Parts { size, weights: Some(&weights), held: Some(&held), hand: Some(&hand), area: Some(&area) };
        let pixels = in_pool(|| overlay_bytes(&parts, COLOURS, 1));
        let at = |x: usize, z: usize| -> [u8; 4] { pixels[(z * size + x) * PIXEL_BYTES..][..PIXEL_BYTES].try_into().unwrap() };
        for z in 0..size {
            assert_eq!(at(0, z), [0; 4], "outside the outline nothing is drawn");
            assert_eq!(at(1, z), [0; 4], "the rim lies inside the outline");
            assert_eq!(at(2, z), RED.map(byte), "the rim, beside weight 0");
            assert_eq!(at(3, z), [0; 4], "the band, away from both edges");
            assert_eq!(at(5, z), [0; 4], "the band's last column is below 1");
            assert_eq!(at(6, z), AMBER.map(byte), "the band's inner edge, beside weight below 1");
            assert_eq!(at(7, z), [0; 4]);
            assert_eq!(at(15, z), [0; 4], "the land's border is no edge");
        }
        assert_eq!(at(10, 10), over(CYAN, MAGENTA).map(byte), "the area over the held cell");
        assert_eq!(at(11, 10), MAGENTA.map(byte));
        assert_eq!(at(12, 10), GREEN.map(byte));
        let wide = in_pool(|| overlay_bytes(&parts, COLOURS, 2));
        assert_eq!(wide[(3 * PIXEL_BYTES)..][..PIXEL_BYTES], RED.map(byte), "two cells wide, the rim takes the next column");
        let bare = Parts { size, weights: None, held: None, hand: None, area: None };
        assert!(in_pool(|| overlay_bytes(&bare, COLOURS, 1)).iter().all(|&value| value == 0));
    }

    /// Stage 2's ruling R21, N3: the overlay an outline's thread makes from a land's marks is the
    /// picture `compose` makes from the marks' pictures, no area; marks shorter than the land read as
    /// off beyond their end.
    #[test]
    fn the_overlay_of_the_marks_is_composes_picture_without_an_area() {
        let size = 16;
        let weights: Vec<u8> = (0..size * size).map(|cell| match cell % size { 0 | 1 => 0, 2..=5 => 128, _ => 255 }).collect();
        let held: Vec<bool> = (0..size * size).map(|cell| cell % size == 10).collect();
        let by_hand: Vec<bool> = (0..size * size - size).map(|cell| cell / size == 4).collect();
        let marks = Marks { held: held.clone(), by_hand: by_hand.clone() };
        let picture = |on: &[bool]| -> Vec<u8> { (0..size * size).map(|cell| if on.get(cell) == Some(&true) { 255 } else { 0 }).collect() };
        let (held_picture, hand_picture) = (picture(&held), picture(&by_hand));
        let parts = Parts { size, weights: Some(&weights), held: Some(&held_picture), hand: Some(&hand_picture), area: None };
        assert_eq!(in_pool(|| marks_overlay(size, &weights, &marks, COLOURS, 2)), in_pool(|| overlay_bytes(&parts, COLOURS, 2)));
        let unmarked = Parts { size, weights: Some(&weights), held: None, hand: None, area: None };
        assert_eq!(in_pool(|| marks_overlay(size, &weights, &Marks::default(), COLOURS, 1)), in_pool(|| overlay_bytes(&unmarked, COLOURS, 1)));
    }
}
