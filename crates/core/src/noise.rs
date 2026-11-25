use crate::rng::mix64;

const R: f64 = std::f64::consts::FRAC_1_SQRT_2;
const C: f64 = 0.923_879_532_511_286_7; // cos 22.5 degrees
const S: f64 = 0.382_683_432_365_089_8; // sin 22.5 degrees

/// Sixteen unit gradients at 22.5 degree steps (literal table: no libm dependence).
const GRADIENTS: [(f64, f64); 16] = [
    (1.0, 0.0), (C, S), (R, R), (S, C),
    (0.0, 1.0), (-S, C), (-R, R), (-C, S),
    (-1.0, 0.0), (-C, -S), (-R, -R), (-S, -C),
    (0.0, -1.0), (S, -C), (R, -R), (C, -S),
];

/// Hash of a lattice point and seed; the seed and both coordinates pass through the finaliser.
fn lattice_hash(seed: u64, ix: i64, iy: i64) -> u64 {
    let h = mix64(seed ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    mix64(h ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
}

fn quintic(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn corner(seed: u64, ix: i64, iy: i64, dx: f64, dy: f64) -> f64 {
    let (gx, gy) = GRADIENTS[(lattice_hash(seed, ix, iy) >> 60) as usize];
    gx * dx + gy * dy
}

/// Gradient noise with quintic interpolation, in `[-1, 1]`; one lattice cell per unit.
fn gradient_noise(seed: u64, x: f64, y: f64) -> f64 {
    let fx = x.floor();
    let fy = y.floor();
    let (ix, iy) = (fx as i64, fy as i64);
    let (dx, dy) = (x - fx, y - fy);
    let (u, v) = (quintic(dx), quintic(dy));
    let n00 = corner(seed, ix, iy, dx, dy);
    let n10 = corner(seed, ix + 1, iy, dx - 1.0, dy);
    let n01 = corner(seed, ix, iy + 1, dx, dy - 1.0);
    let n11 = corner(seed, ix + 1, iy + 1, dx - 1.0, dy - 1.0);
    let top = n00 + u * (n10 - n00);
    let bottom = n01 + u * (n11 - n01);
    // Unit gradients bound the raw value by sqrt(2)/2; rescale to [-1, 1].
    ((top + v * (bottom - top)) / R).clamp(-1.0, 1.0)
}

/// Offset of an octave's lattice in lattice cells, in `[0, 1024)` on each axis, derived only
/// from the octave seed.
fn octave_offset(octave_seed: u64) -> (f64, f64) {
    let to_cells = |h: u64| (h >> 11) as f64 / (1u64 << 53) as f64 * 1024.0;
    (
        to_cells(mix64(octave_seed ^ 0xA5A5_A5A5_A5A5_A5A5)),
        to_cells(mix64(octave_seed ^ 0x5A5A_5A5A_5A5A_5A5A)),
    )
}

/// Fractal Brownian motion: `octaves` layers of gradient noise, the first with the given
/// wavelength in metres, each next with half the wavelength (lacunarity 2) and half the
/// amplitude (gain 0.5). Result in `[-1, 1]`; deterministic in all arguments.
pub fn fbm(seed: u64, x: f64, y: f64, octaves: u32, wavelength_m: f64) -> f32 {
    let mut frequency = 1.0 / wavelength_m;
    let mut amplitude = 1.0;
    let mut sum = 0.0;
    let mut total = 0.0;
    for octave in 0..octaves {
        let octave_seed = mix64(seed.wrapping_add(u64::from(octave).wrapping_mul(0x9E37_79B9_7F4A_7C15)));
        // Each octave gets its own lattice offset (in lattice cells) so the zeros of
        // gradient noise at lattice points never line up across octaves.
        let (offset_x, offset_y) = octave_offset(octave_seed);
        sum += amplitude * gradient_noise(octave_seed, x * frequency + offset_x, y * frequency + offset_y);
        total += amplitude;
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    if total == 0.0 {
        return 0.0;
    }
    ((sum / total) as f32).clamp(-1.0, 1.0)
}

/// `fbm` is bounded by -1 and 1 but spreads far less: its standard deviation is about 0.23 with
/// two octaves and 0.20 with three, and fewer than 3 % of its values lie beyond 0.5. A field meant
/// to span -1 to 1 is `fbm` times this, clamped to -1..1 (`widened`).
const NOISE_WIDENING: f32 = 3.0;

/// A value of `fbm` spread over -1..1: times `NOISE_WIDENING`, clamped.
pub(crate) fn widened(noise: f32) -> f32 {
    (NOISE_WIDENING * noise).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbm_is_bounded_continuous_and_seeded() {
        for i in 0..10_000 {
            let v = fbm(3, i as f64 * 1.7, i as f64 * 0.9, 6, 400.0);
            assert!((-1.0..=1.0).contains(&v));
        }
        assert!((fbm(3, 100.0, 50.0, 6, 400.0) - fbm(3, 100.01, 50.0, 6, 400.0)).abs() < 0.05);
        assert_eq!(fbm(3, 12.5, 8.0, 6, 400.0), fbm(3, 12.5, 8.0, 6, 400.0));
        assert_ne!(fbm(3, 12.5, 8.0, 6, 400.0), fbm(4, 12.5, 8.0, 6, 400.0));
    }

    #[test]
    fn fbm_has_no_zero_lattice() {
        let zeros = (0..100)
            .filter(|k| fbm(3, (*k as f64) * 400.0, (*k as f64 * 7.0) * 400.0, 6, 400.0) == 0.0)
            .count();
        assert!(zeros < 5);
    }

    #[test]
    fn fbm_spans_a_useful_range() {
        let v: Vec<f32> = (0..10_000).map(|i| fbm(3, i as f64 * 37.3, i as f64 * 11.9, 6, 400.0)).collect();
        let (lo, hi) = v.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| (a.min(*x), b.max(*x)));
        assert!(hi - lo > 1.0);
    }
}
