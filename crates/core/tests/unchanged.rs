//! The worlds the synthetic source gives, recorded before the land engine was added: they must
//! not change. Each constant was computed on Landform `6b21ff5`.

use landform_core::{generate, GenConfig, GroundRules, SyntheticSource, World};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a over the bits of the heights, of every layer in name order, and the three bytes of every
/// ground cell.
fn fingerprint(world: &World) -> u64 {
    let mut hash = FNV_OFFSET;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    };
    let mut feed_floats = |data: &[f32]| {
        for value in data {
            value.to_bits().to_le_bytes().into_iter().for_each(&mut feed);
        }
    };
    feed_floats(&world.heights.data);
    // `layers` is a `BTreeMap`, so this runs in name order.
    for layer in world.layers.values() {
        feed_floats(&layer.data);
    }
    for cell in &world.ground.data {
        [cell.strongest, cell.second, cell.blend].into_iter().for_each(&mut feed);
    }
    hash
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

/// (rules, size in cells, seed, fingerprint), the rules named "default" or "steep".
const RECORDED: [(&str, usize, u64, u64); 12] = [
    ("default", 256, 1, 0x779a5260f85d55d3),
    ("default", 256, 7, 0xc2b38322958d8b93),
    ("default", 256, 42, 0x2b118af8e9fac6d9),
    ("default", 512, 1, 0x9add535129b117b7),
    ("default", 512, 7, 0x770947c60f4b6dfa),
    ("default", 512, 42, 0xf8ed144c17d0db53),
    ("steep", 256, 1, 0x138198ebb888e35f),
    ("steep", 256, 7, 0x3eba9ed825b480bf),
    ("steep", 256, 42, 0x6c5159ed6b1ffb10),
    ("steep", 512, 1, 0x05cda0987570bf72),
    ("steep", 512, 7, 0xb85f44e3a959aa38),
    ("steep", 512, 42, 0x02e3994d757c3623),
];

#[test]
fn synthetic_worlds_are_unchanged() {
    let source = SyntheticSource::standard();
    let mut wrong = Vec::new();
    for (rules_name, size_cells, seed, expected) in RECORDED {
        let rules = if rules_name == "steep" { steep_rules() } else { GroundRules::default() };
        let config = GenConfig { size_cells, rules, ..Default::default() };
        let actual = fingerprint(&generate(seed, &[&source], &config).unwrap());
        if actual != expected {
            wrong.push(format!("(\"{rules_name}\", {size_cells}, {seed}, {actual:#018x}): recorded {expected:#018x}"));
        }
    }
    assert!(wrong.is_empty(), "worlds changed:\n{}", wrong.join("\n"));
}
