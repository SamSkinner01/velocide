//! "The Drowned Chandlery" — the third co-op campaign level (1-4 players): a candle factory of
//! the Foundry, flooded when its cistern burst. Water snuffs a Shrike's wick, so the deep water
//! kills; the shallows only slow the eye. Wax still melts in the dipping vats, candles still burn
//! on every dry shelf. You come in by the loading dock, fight through the Dipping Hall, the
//! Cistern (where the flood came from), the Wax Press (half drowned, wax floes for stepping
//! stones) and finish round the Great Wick: an unfinished candle as tall as a bell tower,
//! standing in a flooded well and burning under a hole in the roof. See `docs/levels.md` for
//! the map, the encounters and the routes.

use glam::Vec3;
use shared::level::{Encounter, Environment, LevelKind};

use super::furnace_ring::{half, seg_around};
use super::gates::pad_velocity;
use super::{Gen, Mat, Room, Side, enemy, op, vol, wave};

mod pal {
    use super::Mat;
    /// Drowned brick and mossy stone.
    pub const BRICK: Mat = Mat("brick", [0.5, 0.64, 0.54]);
    pub const STONE: Mat = Mat("concrete", [0.6, 0.68, 0.64]);
    pub const STONE_DARK: Mat = Mat("concrete", [0.42, 0.5, 0.48]);
    pub const TILE: Mat = Mat("tile", [0.62, 0.7, 0.66]);
    /// Waterlogged timber.
    pub const TIMBER: Mat = Mat("wood", [0.56, 0.52, 0.44]);
    pub const TIMBER_DARK: Mat = Mat("wood", [0.38, 0.36, 0.32]);
    pub const VERDIGRIS: Mat = Mat("metal", [0.5, 0.84, 0.72]);
    pub const BRASS: Mat = Mat("metal", [1.0, 0.84, 0.5]);
    pub const IRON: Mat = Mat("metal", [0.45, 0.5, 0.52]);
    pub const RUST: Mat = Mat("rust", [0.7, 0.62, 0.52]);
    /// Fresh and old wax: candles, wax blocks, the Great Wick.
    pub const WAX: Mat = Mat("wax", [1.0, 0.95, 0.8]);
    pub const WAX_OLD: Mat = Mat("wax", [0.86, 0.8, 0.62]);
    pub const WAXSTONE: Mat = Mat("waxstone", [0.9, 0.86, 0.72]);
    /// Water (translucent, no collision) over a dark bed.
    pub const WATER: Mat = Mat("glass", [0.3, 0.8, 0.72]);
    pub const DEEP: Mat = Mat("panel", [0.1, 0.28, 0.28]);
    /// Molten wax in the dipping vats (always has a kill volume) and candle flames.
    pub const MOLTEN: Mat = Mat("lava", [1.0, 0.86, 0.58]);
    pub const GRATE: Mat = Mat("grate", [0.62, 0.74, 0.7]);
    pub const HAZARD: Mat = Mat("hazard", [1.0, 0.86, 0.5]);
    pub const DOOR: Mat = Mat("panel", [0.5, 0.82, 0.74]);
    pub const EXIT_DOOR: Mat = Mat("hazard", [1.0, 0.84, 0.52]);
}

/// Warm candle gold, teal water glow, molten wax, the cold sky, the Great Wick's flame.
const L_CANDLE: [f32; 3] = [1.25, 0.98, 0.62];
const L_WATER: [f32; 3] = [0.32, 0.92, 0.82];
const L_MOLTEN: [f32; 3] = [1.3, 0.82, 0.36];
const L_SKY: [f32; 3] = [0.55, 0.72, 0.85];
const L_FLAME: [f32; 3] = [1.45, 1.15, 0.7];

/// Deep water over `x` × `z`: a dark bed, the translucent surface (no collision) and a kill
/// volume from the bed to just over the surface — water snuffs a Shrike's wick.
fn water(g: &mut Gen, note: &str, (x0, x1): (f32, f32), (z0, z1): (f32, f32), bed: f32, top: f32) {
    use pal::*;
    g.bx(
        &format!("{note} bed"),
        [x0, bed - 0.5, z0],
        [x1, bed, z1],
        DEEP,
    );
    g.deco(
        &format!("{note} (deep water)"),
        [x0, top - 0.1, z0],
        [x1, top, z1],
        WATER,
    );
    g.kill(
        &format!("{note}: deep water snuffs the wick"),
        vol([x0, bed - 0.5, z0], [x1, top + 0.1, z1]),
    );
}

/// Ankle-deep water on a floor at `y` (decoration).
fn shallows(g: &mut Gen, note: &str, (x0, x1): (f32, f32), (z0, z1): (f32, f32), y: f32) {
    g.deco(note, [x0, y, z0], [x1, y + 0.25, z1], pal::WATER);
}

/// A cluster of candles on a shelf or ledge at `y` (decoration) and its light.
fn candles(g: &mut Gen, note: &str, [x, y, z]: [f32; 3], light: bool) {
    use pal::*;
    for (dx, dz, h) in [(-0.5, -0.2, 0.9), (0.1, 0.3, 0.6), (0.5, -0.3, 1.2)] {
        g.deco(
            note,
            [x + dx - 0.12, y, z + dz - 0.12],
            [x + dx + 0.12, y + h, z + dz + 0.12],
            WAX,
        );
        g.deco(
            &format!("{note} flame"),
            [x + dx - 0.06, y + h, z + dz - 0.06],
            [x + dx + 0.06, y + h + 0.2, z + dz + 0.06],
            MOLTEN,
        );
    }
    if light {
        g.light(note, [x, y + 1.6, z], L_CANDLE, 7.0);
    }
}

pub fn drowned_chandlery() -> Gen {
    use Side::*;
    use pal::*;
    let mut g = Gen::new(
        "The Drowned Chandlery",
        LevelKind::Campaign,
        Environment {
            sky_color: Vec3::new(0.06, 0.1, 0.11),
            fog_color: Vec3::new(0.08, 0.14, 0.13),
            fog_start: 20.0,
            fog_end: 125.0,
            ambient: Vec3::new(0.5, 0.57, 0.55),
            light_dir: Vec3::new(0.3, -1.0, 0.4),
            light_color: Vec3::new(0.45, 0.6, 0.62),
            music: "campaign".into(),
        },
    );

    // Par for the results screen: ~40 s of movement + four fights (see docs/levels.md).
    g.def.par_time = 300.0;

    // ---------------------------------------------------------------- R0 Loading Dock (start)
    g.section("R0 Loading Dock (start): the barge slip, the moored barge");
    let r0 = Room::new("R0", [-12.0, 0.0, 64.0], [12.0, 9.0, 86.0], BRICK)
        .ceiling(TIMBER_DARK)
        .bottom(-4.0)
        .opening(op(N, -2.5, 2.5, 0.0, 5.0));
    g.room(&r0);
    g.bx("R0 dock", [-5.0, -4.0, 64.0], [12.0, 0.0, 86.0], TIMBER);
    g.bx("R0 quay N", [-12.0, -4.0, 64.0], [-5.0, 0.0, 66.0], STONE);
    g.bx("R0 quay S", [-12.0, -4.0, 84.0], [-5.0, 0.0, 86.0], STONE);
    g.bx(
        "R0 dock edge",
        [-5.0, 0.0, 66.0],
        [-4.7, 0.15, 84.0],
        HAZARD,
    );
    water(
        &mut g,
        "R0 barge slip",
        (-12.0, -5.0),
        (66.0, 84.0),
        -3.5,
        -0.5,
    );
    // The barge (moored, a 1.5 m hop from the dock) with its cabin: a candle shrine inside
    // (secret 1). The deck in front of the cabin door is 1.7 m wide.
    g.bx(
        "R0 barge hull",
        [-11.0, -1.6, 69.0],
        [-6.5, 0.0, 81.0],
        TIMBER_DARK,
    );
    #[rustfmt::skip]
    let cabin = [
        ("R0 barge cabin W", [-10.6, 0.0, 71.5], [-10.3, 2.6, 77.5]),
        ("R0 barge cabin N", [-10.3, 0.0, 71.5], [-8.2, 2.6, 71.8]),
        ("R0 barge cabin S", [-10.3, 0.0, 77.2], [-8.2, 2.6, 77.5]),
        ("R0 barge cabin E", [-8.5, 0.0, 71.8], [-8.2, 2.6, 73.5]),
        ("R0 barge cabin E", [-8.5, 0.0, 75.5], [-8.2, 2.6, 77.2]),
        ("R0 barge cabin lintel", [-8.5, 2.1, 73.5], [-8.2, 2.6, 75.5]),
    ];
    for (note, a, b) in cabin {
        g.bx(note, a, b, TIMBER);
    }
    g.bx(
        "R0 barge cabin roof",
        [-10.8, 2.6, 71.3],
        [-8.0, 2.9, 77.7],
        TIMBER_DARK,
    );
    candles(&mut g, "R0 barge shrine", [-9.4, 0.0, 76.4], true);
    g.secret(
        "secret 1: the barge cabin's candle shrine",
        vol([-10.3, 0.0, 71.8], [-8.5, 2.6, 77.2]),
    );
    for (x, z) in [(6.0, 68.0), (8.5, 82.0)] {
        g.bx(
            "R0 crate",
            [x - 1.2, 0.0, z - 1.2],
            [x + 1.2, 2.0, z + 1.2],
            TIMBER,
        );
        g.bx(
            "R0 crate band",
            [x - 1.25, 0.8, z - 1.25],
            [x + 1.25, 1.1, z + 1.25],
            IRON,
        );
    }
    g.bx(
        "R0 candle crates",
        [7.5, 0.0, 70.5],
        [10.5, 1.2, 72.5],
        TIMBER_DARK,
    );
    candles(&mut g, "R0 candle crates", [9.0, 1.2, 71.5], false);
    for x in [-3.0, 3.0] {
        g.bx(
            "R0 door post",
            [x - 0.5, 0.0, 64.0],
            [x + 0.5, 5.5, 64.5],
            VERDIGRIS,
        );
    }
    g.bx(
        "R0 door head",
        [-2.5, 5.0, 64.0],
        [2.5, 5.5, 64.5],
        VERDIGRIS,
    );
    g.light("R0 dock", [3.0, 7.0, 76.0], L_CANDLE, 14.0);
    g.light("R0 slip", [-8.5, -1.2, 75.0], L_WATER, 9.0);
    g.light("R0 way out", [0.0, 6.0, 66.0], L_SKY, 8.0);
    for x in [-3.5, -1.0, 1.5, 4.0] {
        g.spawn([x, 0.0, 79.0], 0.0);
    }

    // ---------------------------------------------------------------- C1 Culvert
    g.section("C1 Culvert (a 4 m flooded sluice to jump)");
    let c1 = Room::new("C1", [-4.0, 0.0, 45.0], [4.0, 6.0, 63.0], STONE)
        .ceiling(STONE_DARK)
        .bottom(-3.5)
        .open(&[N, S]);
    g.room(&c1);
    g.bx("C1 floor N", [-4.0, -3.5, 45.0], [4.0, 0.0, 50.5], TILE);
    g.bx("C1 floor S", [-4.0, -3.5, 54.5], [4.0, 0.0, 63.0], TILE);
    water(&mut g, "C1 sluice", (-4.0, 4.0), (50.5, 54.5), -3.0, -0.6);
    shallows(&mut g, "C1 shallows", (-4.0, 4.0), (54.5, 63.0), 0.0);
    for x in [-3.9, 3.4] {
        g.bx(
            "C1 sluice post",
            [x, -3.0, 52.0],
            [x + 0.5, 6.0, 53.0],
            VERDIGRIS,
        );
    }
    g.bx(
        "C1 sluice beam",
        [-3.5, 5.0, 52.0],
        [3.5, 5.6, 53.0],
        VERDIGRIS,
    );
    g.deco(
        "C1 sluice gate (raised)",
        [-3.5, 4.0, 52.2],
        [3.5, 5.0, 52.8],
        GRATE,
    );
    g.light("C1 south", [0.0, 5.0, 59.0], L_CANDLE, 8.0);
    g.light("C1 sluice", [0.0, -1.5, 52.5], L_WATER, 7.0);
    g.light("C1 north", [0.0, 5.0, 47.0], L_CANDLE, 8.0);

    // ---------------------------------------------------------------- A1 Dipping Hall
    g.section("A1 Dipping Hall (arena 1: gnawers, then censers on the gantry)");
    let a1_in = op(S, -2.5, 2.5, 0.0, 5.0);
    let a1_out = op(N, 14.0, 19.0, 4.0, 9.0);
    let a1 = Room::new("A1", [-22.0, 0.0, 10.0], [22.0, 14.0, 44.0], BRICK)
        .ceiling(TIMBER_DARK)
        .bottom(-3.0)
        .opening(a1_in)
        .opening(a1_out);
    g.room(&a1);
    g.door(
        1,
        "door 1: A1 entry (locked by encounter 1)",
        &a1,
        a1_in,
        true,
        DOOR,
    );
    g.door(
        2,
        "door 2: A1 exit, gantry level (opened by encounter 1)",
        &a1,
        a1_out,
        false,
        EXIT_DOOR,
    );
    // Floor round two dipping vats of molten wax (x -16..-9 and 9..16, z 18..34).
    g.bx("A1 floor N", [-22.0, -3.0, 10.0], [22.0, 0.0, 18.0], TILE);
    g.bx("A1 floor S", [-22.0, -3.0, 34.0], [22.0, 0.0, 44.0], TILE);
    g.bx("A1 floor W", [-22.0, -3.0, 18.0], [-16.0, 0.0, 34.0], TILE);
    g.bx(
        "A1 floor middle",
        [-9.0, -3.0, 18.0],
        [9.0, 0.0, 34.0],
        TILE,
    );
    g.bx("A1 floor E", [16.0, -3.0, 18.0], [22.0, 0.0, 34.0], TILE);
    for (x0, x1) in [(-16.0f32, -9.0f32), (9.0, 16.0)] {
        let xc = (x0 + x1) * 0.5;
        g.bx("A1 vat bed", [x0, -3.0, 18.0], [x1, -2.5, 34.0], IRON);
        g.bx(
            "A1 vat (molten wax)",
            [x0, -2.5, 18.0],
            [x1, -1.0, 34.0],
            MOLTEN,
        );
        g.kill(
            "A1 dipping vat (molten wax)",
            vol([x0, -3.0, 18.0], [x1, -0.6, 34.0]),
        );
        g.bx(
            "A1 vat rim",
            [x0 - 0.4, 0.0, 17.6],
            [x1 + 0.4, 0.7, 18.0],
            WAXSTONE,
        );
        g.bx(
            "A1 vat rim",
            [x0 - 0.4, 0.0, 34.0],
            [x1 + 0.4, 0.7, 34.4],
            WAXSTONE,
        );
        g.bx(
            "A1 vat rim",
            [x0 - 0.4, 0.0, 18.0],
            [x0, 0.7, 34.0],
            WAXSTONE,
        );
        g.bx(
            "A1 vat rim",
            [x1, 0.0, 18.0],
            [x1 + 0.4, 0.7, 34.0],
            WAXSTONE,
        );
        // The dipping rack over the vat: a beam on two posts, strings of half-dipped
        // candles hanging into the wax.
        g.bx(
            "A1 dipping rack post",
            [xc - 0.3, 0.0, 34.6],
            [xc + 0.3, 7.5, 35.2],
            TIMBER_DARK,
        );
        g.bx(
            "A1 dipping rack post",
            [xc - 0.3, 0.0, 16.8],
            [xc + 0.3, 7.5, 17.4],
            TIMBER_DARK,
        );
        g.bx(
            "A1 dipping rack beam",
            [xc - 0.4, 7.5, 16.8],
            [xc + 0.4, 8.1, 35.2],
            TIMBER_DARK,
        );
        for z in [20.0, 23.5, 27.0, 30.5] {
            for dx in [-1.6, 1.6] {
                g.deco(
                    "A1 candle string",
                    [xc + dx - 0.08, 1.5, z - 0.08],
                    [xc + dx + 0.08, 7.3, z + 0.08],
                    WAX,
                );
            }
            g.deco(
                "A1 rack crossbar",
                [xc - 1.8, 7.3, z - 0.1],
                [xc + 1.8, 7.5, z + 0.1],
                TIMBER,
            );
        }
        g.light("A1 vat", [xc, 1.0, 26.0], L_MOLTEN, 11.0);
    }
    // The gantry along the north wall (y 4), ramps up both side walls, posts.
    g.bx("A1 gantry", [-22.0, 3.6, 10.0], [22.0, 4.0, 17.0], TIMBER);
    g.bx(
        "A1 gantry edge",
        [-18.0, 3.3, 16.75],
        [18.0, 3.6, 17.05],
        HAZARD,
    );
    for x in [-11.0, -4.0, 4.0, 11.0] {
        g.bx(
            "A1 gantry post",
            [x - 0.3, 0.0, 16.0],
            [x + 0.3, 3.6, 16.6],
            IRON,
        );
    }
    g.ramp_z(
        "A1 ramp W",
        (-22.0, -18.0),
        (30.0, 0.0),
        (17.0, 4.0),
        6.0,
        STONE,
    );
    g.ramp_z(
        "A1 ramp E",
        (18.0, 22.0),
        (30.0, 0.0),
        (17.0, 4.0),
        6.0,
        STONE,
    );
    g.pad(
        "A1 floor -> gantry",
        vol([-1.2, 0.0, 21.8], [1.2, 1.0, 24.2]),
        pad_velocity([0.0, 0.0, 23.0], [0.0, 4.0, 12.5], 4.0),
    );
    // The great wick spool in the middle (cover), crates under the gantry.
    g.bx("A1 wick spool", [-3.0, 0.0, 26.0], [3.0, 2.6, 30.0], TIMBER);
    g.bx(
        "A1 spool flange",
        [-3.3, 0.0, 25.6],
        [3.3, 3.0, 26.0],
        BRASS,
    );
    g.bx(
        "A1 spool flange",
        [-3.3, 0.0, 30.0],
        [3.3, 3.0, 30.4],
        BRASS,
    );
    for x in [-8.0, 8.0] {
        g.bx(
            "A1 crate",
            [x - 1.0, 0.0, 10.5],
            [x + 1.0, 1.8, 12.5],
            TIMBER_DARK,
        );
    }
    shallows(
        &mut g,
        "A1 shallows (seeping in from the culvert)",
        (-6.0, 6.0),
        (36.0, 44.0),
        0.0,
    );
    for (x, z) in [(-15.0, 12.0), (15.0, 12.0)] {
        candles(&mut g, "A1 gantry candles", [x, 4.0, z], true);
    }
    for (x, z) in [(-19.0, 40.0), (19.0, 40.0)] {
        candles(&mut g, "A1 floor candles", [x, 0.0, z], true);
    }
    g.light("A1 hall", [0.0, 11.0, 27.0], L_CANDLE, 20.0);
    g.light("A1 entry", [0.0, 6.0, 41.0], L_SKY, 9.0);
    g.checkpoint(
        "checkpoint 1: A1 vestibule",
        vol([-8.0, -1.0, 38.0], [8.0, 6.0, 44.0]),
        [0.0, 0.0, 41.0],
        0.0,
    );
    g.encounter(
        "encounter 1: Dipping Hall — gnawers, then snuffers and censers on the gantry",
        &a1,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-22.0, -3.0, 10.0], [22.0, 14.0, 38.0]),
            lock_doors: vec![1],
            unlock_doors: vec![2],
            waves: vec![
                wave(
                    0.5,
                    vec![
                        enemy("gnawer", [-4.5, 0.0, 21.0], 180.0),
                        enemy("gnawer", [4.5, 0.0, 21.0], 180.0),
                        enemy("gnawer", [0.0, 0.0, 13.5], 180.0),
                        enemy("gnawer", [-19.5, 0.0, 13.5], 180.0),
                        enemy("gnawer", [19.5, 0.0, 13.5], 180.0),
                    ],
                ),
                wave(
                    1.5,
                    vec![
                        enemy("snuffer", [-19.5, 0.0, 32.5], 180.0),
                        enemy("snuffer", [19.5, 0.0, 32.5], 180.0),
                        enemy("snuffer", [0.0, 0.0, 22.0], 180.0),
                        enemy("gnawer", [-6.0, 0.0, 13.5], 180.0),
                        enemy("gnawer", [6.0, 0.0, 13.5], 180.0),
                        enemy("censer", [-8.0, 4.0, 13.5], 180.0),
                        enemy("censer", [8.0, 4.0, 13.5], 180.0),
                    ],
                ),
            ],
        },
    );

    // ---------------------------------------------------------------- C2 Spool Walk
    g.section("C2 Spool Walk (gantry level; a 6 m flooded break with a wax floe)");
    let c2 = Room::new("C2", [13.0, 4.0, -12.0], [20.0, 10.0, 9.0], STONE)
        .ceiling(STONE_DARK)
        .bottom(-1.0)
        .open(&[N, S]);
    g.room(&c2);
    g.bx("C2 floor S", [13.0, -1.0, 2.0], [20.0, 4.0, 9.0], TILE);
    g.bx("C2 floor N", [13.0, -1.0, -12.0], [20.0, 4.0, -4.0], TILE);
    water(
        &mut g,
        "C2 flooded break",
        (13.0, 20.0),
        (-4.0, 2.0),
        -0.5,
        3.4,
    );
    g.bx("C2 wax floe", [15.0, -0.5, -2.0], [17.5, 4.1, 0.0], WAX_OLD);
    for z in [-11.0, 7.0] {
        g.deco(
            "C2 pipe",
            [13.0, 8.4, z - 0.3],
            [20.0, 9.0, z + 0.3],
            VERDIGRIS,
        );
    }
    g.light("C2 south", [16.5, 8.0, 6.0], L_CANDLE, 9.0);
    g.light("C2 break", [16.5, 2.0, -1.0], L_WATER, 8.0);
    g.light("C2 north", [16.5, 8.0, -9.0], L_CANDLE, 9.0);

    // ---------------------------------------------------------------- A2 The Cistern
    g.section("A2 The Cistern (arena 2: harriers, herons; 30 m tall, the flood's source)");
    let a2_in = op(S, 14.0, 19.0, 4.0, 9.0);
    let a2_out = op(W, -40.0, -35.0, 14.0, 19.0);
    let a2 = Room::new("A2", [-4.0, 4.0, -54.0], [34.0, 34.0, -13.0], BRICK)
        .ceiling(STONE_DARK)
        .bottom(-1.0)
        .opening(a2_in)
        .opening(a2_out);
    g.room(&a2);
    g.door(
        3,
        "door 3: A2 entry (locked by encounter 2)",
        &a2,
        a2_in,
        true,
        DOOR,
    );
    g.door(
        4,
        "door 4: A2 exit, west gallery (opened by encounter 2)",
        &a2,
        a2_out,
        false,
        EXIT_DOOR,
    );
    // Walkways round the cistern pool (x 6..24, z -44..-22).
    g.bx("A2 floor S", [-4.0, -1.0, -22.0], [34.0, 4.0, -13.0], TILE);
    g.bx("A2 floor N", [-4.0, -1.0, -54.0], [34.0, 4.0, -44.0], TILE);
    g.bx("A2 floor W", [-4.0, -1.0, -44.0], [6.0, 4.0, -22.0], TILE);
    g.bx("A2 floor E", [24.0, -1.0, -44.0], [34.0, 4.0, -22.0], TILE);
    water(
        &mut g,
        "A2 cistern pool",
        (6.0, 24.0),
        (-44.0, -22.0),
        -0.5,
        3.4,
    );
    for (x0, x1, z0, z1) in [
        (6.0, 24.0, -22.0, -21.7),
        (6.0, 24.0, -44.3, -44.0),
        (5.7, 6.0, -44.3, -21.7),
        (24.0, 24.3, -44.3, -21.7),
    ] {
        g.bx("A2 pool kerb", [x0, 4.0, z0], [x1, 4.15, z1], HAZARD);
    }
    // Galleries (y 14): north and west, an east ramp and landing up to them.
    g.ramp_z(
        "A2 east ramp",
        (29.0, 34.0),
        (-17.0, 4.0),
        (-42.0, 14.0),
        10.0,
        STONE,
    );
    g.bx(
        "A2 east landing",
        [29.0, 4.0, -46.0],
        [34.0, 14.0, -42.0],
        STONE,
    );
    g.bx(
        "A2 north gallery",
        [-4.0, 13.4, -54.0],
        [34.0, 14.0, -46.0],
        TIMBER,
    );
    g.bx(
        "A2 west gallery",
        [-4.0, 13.4, -46.0],
        [4.0, 14.0, -30.0],
        TIMBER,
    );
    g.bx(
        "A2 north gallery edge",
        [4.0, 13.1, -46.3],
        [29.0, 13.4, -46.0],
        VERDIGRIS,
    );
    g.bx(
        "A2 west gallery edge",
        [3.7, 13.1, -46.0],
        [4.0, 13.4, -30.0],
        VERDIGRIS,
    );
    for (x, z) in [(3.0, -30.5), (3.0, -45.0), (14.0, -47.0), (24.0, -47.0)] {
        g.bx(
            "A2 gallery post",
            [x - 0.3, 4.0, z - 0.3],
            [x + 0.3, 13.4, z + 0.3],
            IRON,
        );
    }
    g.pad(
        "A2 west walk -> west gallery",
        vol([-1.0, 4.0, -26.0], [1.4, 5.0, -23.6]),
        pad_velocity([0.2, 4.0, -24.8], [0.0, 14.0, -36.5], 4.5),
    );
    // The burst cistern tank overhead and the flood still pouring out of it.
    g.bx(
        "A2 cistern tank",
        [8.0, 24.0, -40.0],
        [22.0, 30.0, -26.0],
        RUST,
    );
    g.bx(
        "A2 tank hoop",
        [7.7, 26.5, -40.3],
        [22.3, 27.2, -25.7],
        IRON,
    );
    for (x, z) in [(8.5, -39.5), (21.5, -39.5), (8.5, -26.5), (21.5, -26.5)] {
        g.bx(
            "A2 tank hanger",
            [x - 0.15, 30.0, z - 0.15],
            [x + 0.15, 34.0, z + 0.15],
            IRON,
        );
    }
    g.deco(
        "A2 the flood (pouring)",
        [13.5, 3.4, -34.0],
        [16.5, 24.0, -31.0],
        WATER,
    );
    g.deco(
        "A2 the flood (splash)",
        [12.5, 3.4, -35.0],
        [17.5, 4.4, -30.0],
        WATER,
    );
    for (x, z) in [(-1.0, -50.0), (20.0, -51.0), (31.0, -51.0)] {
        candles(&mut g, "A2 gallery candles", [x, 14.0, z], true);
    }
    candles(&mut g, "A2 west gallery candles", [-1.5, 14.0, -32.0], true);
    g.light("A2 pool", [10.0, 1.5, -26.0], L_WATER, 12.0);
    g.light("A2 pool", [20.0, 1.5, -40.0], L_WATER, 12.0);
    g.light("A2 the flood", [15.0, 12.0, -28.0], L_WATER, 14.0);
    g.light("A2 tank", [15.0, 31.0, -33.0], L_SKY, 14.0);
    g.light("A2 entry", [16.5, 8.0, -16.0], L_CANDLE, 10.0);
    g.light("A2 east ramp", [31.5, 12.0, -30.0], L_CANDLE, 10.0);
    g.checkpoint(
        "checkpoint 2: A2 vestibule",
        vol([10.0, 3.0, -19.0], [23.0, 10.0, -13.0]),
        [16.5, 4.0, -16.0],
        0.0,
    );
    g.encounter(
        "encounter 2: The Cistern — harriers out of the spray, then herons on the galleries",
        &a2,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-4.0, -1.0, -54.0], [34.0, 34.0, -19.0]),
            lock_doors: vec![3],
            unlock_doors: vec![4],
            waves: vec![
                wave(
                    0.5,
                    vec![
                        enemy("harrier", [26.5, 11.0, -28.0], 0.0),
                        enemy("harrier", [0.5, 10.0, -26.0], 0.0),
                        enemy("harrier", [19.0, 10.0, -27.0], 0.0),
                        enemy("gnawer", [-1.0, 4.0, -36.0], 0.0),
                        enemy("gnawer", [26.5, 4.0, -32.0], 0.0),
                        enemy("gnawer", [9.0, 4.0, -49.0], 0.0),
                    ],
                ),
                wave(
                    1.5,
                    vec![
                        enemy("heron", [6.0, 14.0, -50.0], 0.0),
                        enemy("heron", [26.0, 14.0, -50.0], 0.0),
                        enemy("censer", [0.0, 14.0, -42.0], 0.0),
                        enemy("censer", [16.0, 14.0, -50.0], 0.0),
                        enemy("harrier", [8.0, 10.0, -25.0], 0.0),
                        enemy("harrier", [26.0, 11.0, -37.0], 0.0),
                        enemy("bellwether", [16.0, 4.0, -50.0], 0.0),
                        enemy("gnawer", [-1.0, 4.0, -50.0], 0.0),
                        enemy("gnawer", [27.0, 4.0, -50.0], 0.0),
                    ],
                ),
            ],
        },
    );

    // ---------------------------------------------------------------- C3 Pipe Gallery
    g.section("C3 Pipe Gallery (gallery level: slide under the pipes N / hop the valve box S)");
    let c3 = Room::new("C3", [-15.0, 14.0, -41.0], [-5.0, 20.0, -34.0], STONE)
        .ceiling(STONE_DARK)
        .open(&[W, E]);
    g.room(&c3);
    g.bx("C3 floor", [-15.0, 13.0, -41.0], [-5.0, 14.0, -34.0], TILE);
    g.bx(
        "C3 pipes (slide under)",
        [-11.0, 15.2, -41.0],
        [-9.0, 16.4, -37.5],
        VERDIGRIS,
    );
    g.bx(
        "C3 valve box (hop over)",
        [-11.0, 14.0, -37.5],
        [-9.5, 15.0, -34.0],
        BRASS,
    );
    g.light("C3", [-10.0, 18.5, -37.5], L_CANDLE, 9.0);

    // ---------------------------------------------------------------- A3 The Wax Press
    g.section("A3 The Wax Press (arena 3: the first gaoler; half drowned, wax floes)");
    let a3_in = op(E, -40.0, -35.0, 14.0, 19.0);
    let a3_out = op(W, -30.0, -25.0, 11.0, 16.0);
    let a3 = Room::new("A3", [-54.0, 11.0, -60.0], [-16.0, 28.0, -22.0], BRICK)
        .ceiling(TIMBER_DARK)
        .bottom(6.5)
        .opening(a3_in)
        .opening(a3_out);
    g.room(&a3);
    g.door(
        5,
        "door 5: A3 entry, balcony (locked by encounter 3)",
        &a3,
        a3_in,
        true,
        DOOR,
    );
    g.door(
        6,
        "door 6: A3 exit, past the floes (opened by encounter 3)",
        &a3,
        a3_out,
        false,
        EXIT_DOOR,
    );
    // Dry floor (east), the drowned west third, the landing at the exit.
    g.bx("A3 floor", [-42.0, 6.5, -60.0], [-16.0, 11.0, -22.0], TILE);
    g.bx(
        "A3 exit landing",
        [-54.0, 6.5, -33.0],
        [-49.0, 11.0, -22.0],
        TILE,
    );
    water(
        &mut g,
        "A3 drowned floor",
        (-54.0, -42.0),
        (-60.0, -33.0),
        7.0,
        10.4,
    );
    water(
        &mut g,
        "A3 drowned floor",
        (-49.0, -42.0),
        (-33.0, -22.0),
        7.0,
        10.4,
    );
    g.bx(
        "A3 water's edge",
        [-42.0, 11.0, -60.0],
        [-41.7, 11.12, -22.0],
        HAZARD,
    );
    for (x, z, s) in [
        (-45.5, -27.5, 1.25),
        (-50.0, -38.5, 1.4),
        (-46.0, -47.0, 1.4),
        (-50.5, -55.0, 1.4),
    ] {
        g.bx(
            "A3 wax floe",
            [x - s, 7.0, z - s],
            [x + s, 11.2, z + s],
            WAX_OLD,
        );
    }
    // The balcony at the entry (y 14) and its ramp down west to the floor.
    g.bx(
        "A3 balcony",
        [-22.0, 11.0, -46.0],
        [-16.0, 14.0, -30.0],
        STONE,
    );
    g.bx(
        "A3 balcony edge",
        [-22.3, 13.7, -46.0],
        [-22.0, 14.1, -36.0],
        VERDIGRIS,
    );
    g.ramp_x(
        "A3 balcony ramp",
        (-36.0, -30.0),
        (-29.0, 11.0),
        (-22.0, 14.0),
        6.0,
        STONE,
    );
    // The wax press: its bed (a platform, y 15), the screw and the raised platen.
    g.bx(
        "A3 press bed",
        [-40.0, 11.0, -56.0],
        [-28.0, 15.0, -44.0],
        IRON,
    );
    g.bx(
        "A3 press bed band",
        [-40.3, 13.0, -56.3],
        [-27.7, 13.6, -43.7],
        BRASS,
    );
    g.bx(
        "A3 press screw",
        [-35.5, 15.0, -56.0],
        [-32.5, 26.0, -53.0],
        BRASS,
    );
    g.bx(
        "A3 press platen",
        [-38.0, 22.0, -55.5],
        [-30.0, 23.0, -49.0],
        IRON,
    );
    g.pad(
        "A3 floor -> press bed",
        vol([-35.2, 11.0, -39.2], [-32.8, 12.0, -36.8]),
        pad_velocity([-34.0, 11.0, -38.0], [-34.0, 15.0, -47.5], 3.5),
    );
    // Mould racks (cover).
    g.bx(
        "A3 mould rack",
        [-38.0, 11.0, -34.0],
        [-36.0, 14.0, -28.0],
        TIMBER_DARK,
    );
    g.bx(
        "A3 mould rack",
        [-30.0, 11.0, -25.0],
        [-28.0, 14.0, -23.0],
        TIMBER_DARK,
    );
    for (x, z) in [(-37.0, -31.0), (-29.0, -24.0)] {
        candles(&mut g, "A3 rack candles", [x, 14.0, z], true);
    }
    candles(&mut g, "A3 press candles", [-30.5, 15.0, -45.0], true);
    g.light("A3 hall", [-32.0, 24.0, -40.0], L_CANDLE, 20.0);
    g.light("A3 drowned floor", [-48.0, 9.0, -46.0], L_WATER, 13.0);
    g.light("A3 exit landing", [-51.5, 14.0, -27.5], L_SKY, 8.0);
    g.checkpoint(
        "checkpoint 3: A3 balcony",
        vol([-22.0, 13.0, -42.0], [-16.0, 20.0, -33.0]),
        [-19.0, 14.0, -37.5],
        90.0,
    );
    g.encounter(
        "encounter 3: The Wax Press — a gaoler, then a smelter, censers on the press, a heron",
        &a3,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-54.0, 6.5, -60.0], [-22.0, 28.0, -22.0]),
            lock_doors: vec![5],
            unlock_doors: vec![6],
            waves: vec![
                wave(
                    0.5,
                    vec![
                        enemy("gaoler", [-33.0, 11.0, -26.0], 90.0),
                        enemy("gnawer", [-38.0, 11.0, -24.0], 90.0),
                        enemy("gnawer", [-26.0, 11.0, -48.0], 90.0),
                        enemy("gnawer", [-40.0, 11.0, -40.0], 90.0),
                        enemy("snuffer", [-25.0, 11.0, -26.0], 90.0),
                        enemy("snuffer", [-24.0, 11.0, -57.0], 90.0),
                    ],
                ),
                wave(
                    1.5,
                    vec![
                        enemy("smelter", [-22.0, 11.0, -53.0], 90.0),
                        enemy("censer", [-34.0, 15.0, -48.0], 90.0),
                        enemy("heron", [-24.0, 11.0, -58.0], 90.0),
                        enemy("harrier", [-30.0, 18.0, -28.0], 90.0),
                        enemy("harrier", [-36.0, 18.0, -40.0], 90.0),
                        enemy("gnawer", [-27.0, 11.0, -26.5], 90.0),
                        enemy("gnawer", [-40.0, 11.0, -25.0], 90.0),
                    ],
                ),
            ],
        },
    );

    // ---------------------------------------------------------------- C4 Wick Passage
    g.section("C4 Wick Passage");
    let c4 = Room::new("C4", [-65.0, 11.0, -31.0], [-55.0, 17.0, -24.0], STONE)
        .ceiling(STONE_DARK)
        .open(&[W, E]);
    g.room(&c4);
    g.bx("C4 floor", [-65.0, 10.0, -31.0], [-55.0, 11.0, -24.0], TILE);
    for x in [-62.0, -58.0] {
        g.bx(
            "C4 candle shelf",
            [x - 1.0, 13.0, -31.0],
            [x + 1.0, 13.2, -30.4],
            TIMBER_DARK,
        );
        candles(&mut g, "C4 shelf candles", [x, 13.2, -30.7], false);
    }
    g.light("C4", [-60.0, 15.5, -27.5], L_CANDLE, 9.0);

    // ---------------------------------------------------------------- A4 The Great Wick
    g.section("A4 The Great Wick (arena 4, the finale): a flooded well, the colossal candle");
    // Room centre, well apothem (octagonal), gallery height and depth.
    const CX: f32 = -88.0;
    const CZ: f32 = -28.0;
    const WELL: f32 = 13.0;
    const GY: f32 = 21.0;
    let a4_in = op(E, -30.0, -25.0, 11.0, 16.0);
    let a4_out = op(N, -91.0, -86.0, GY, GY + 5.0);
    let a4 = Room::new("A4", [-110.0, 11.0, -50.0], [-66.0, 50.0, -6.0], BRICK)
        .bottom(3.5)
        .opening(a4_in)
        .opening(a4_out);
    g.room(&a4);
    g.door(
        7,
        "door 7: A4 entry (locked by encounter 4)",
        &a4,
        a4_in,
        true,
        DOOR,
    );
    g.door(
        8,
        "door 8: A4 exit, north gallery (opened by encounter 4)",
        &a4,
        a4_out,
        false,
        EXIT_DOOR,
    );
    // The roof, with a smoke hole over the Great Wick (clipped).
    g.bx(
        "A4 roof",
        [-111.0, 50.0, -51.0],
        [-65.0, 51.0, -32.0],
        TIMBER_DARK,
    );
    g.bx(
        "A4 roof",
        [-111.0, 50.0, -24.0],
        [-65.0, 51.0, -5.0],
        TIMBER_DARK,
    );
    g.bx(
        "A4 roof",
        [-111.0, 50.0, -32.0],
        [-92.0, 51.0, -24.0],
        TIMBER_DARK,
    );
    g.bx(
        "A4 roof",
        [-84.0, 50.0, -32.0],
        [-65.0, 51.0, -24.0],
        TIMBER_DARK,
    );
    g.clip(
        "A4 smoke hole clip",
        [-92.0, 50.0, -32.0],
        [-84.0, 51.0, -24.0],
    );
    // Floor round the octagonal well: a square ring plus four turned corner fillers.
    let (x0, x1, z0, z1) = (-110.0, -66.0, -50.0, -6.0);
    let (wx0, wx1, wz0, wz1) = (CX - WELL, CX + WELL, CZ - WELL, CZ + WELL);
    g.bx("A4 floor N", [x0, 3.5, z0], [x1, 11.0, wz0], TILE);
    g.bx("A4 floor S", [x0, 3.5, wz1], [x1, 11.0, z1], TILE);
    g.bx("A4 floor W", [x0, 3.5, wz0], [wx0, 11.0, wz1], TILE);
    g.bx("A4 floor E", [wx1, 3.5, wz0], [x1, 11.0, wz1], TILE);
    let corner = WELL * std::f32::consts::SQRT_2;
    for k in [1, 3, 5, 7] {
        seg_around(
            &mut g,
            "A4 floor (well corner)",
            (CX, CZ),
            k,
            (-half(WELL), half(WELL)),
            (WELL, corner),
            (3.5, 11.0),
            TILE,
        );
    }
    water(&mut g, "A4 the well", (wx0, wx1), (wz0, wz1), 4.0, 10.4);
    for k in 0..8 {
        seg_around(
            &mut g,
            "A4 well kerb",
            (CX, CZ),
            k,
            (-half(WELL), half(WELL)),
            (WELL, WELL + 0.4),
            (11.0, 11.15),
            WAXSTONE,
        );
    }
    // The Great Wick: an octagonal candle 34 m tall standing in the well, wax runs down its
    // sides, the wick and its flame under the smoke hole. Its top is secret 2.
    const WICK: f32 = 5.0;
    const WICK_TOP: f32 = 38.0;
    let sw = half(WICK);
    g.bx(
        "A4 the Great Wick",
        [CX - sw, 4.0, CZ - WICK],
        [CX + sw, WICK_TOP, CZ + WICK],
        WAX,
    );
    g.bx(
        "A4 the Great Wick",
        [CX - WICK, 4.0, CZ - sw],
        [CX + WICK, WICK_TOP, CZ + sw],
        WAX,
    );
    for k in [1, 3, 5, 7] {
        seg_around(
            &mut g,
            "A4 the Great Wick",
            (CX, CZ),
            k,
            (-sw, sw),
            (0.0, WICK),
            (4.0, WICK_TOP),
            WAX,
        );
    }
    for (k, t, top) in [
        (0, -1.5, 33.0),
        (2, 2.0, 30.0),
        (4, 0.5, 35.0),
        (6, -2.5, 27.0),
        (1, 1.0, 31.5),
        (5, -1.0, 29.0),
    ] {
        seg_around(
            &mut g,
            "A4 wax run",
            (CX, CZ),
            k,
            (t - 0.8, t + 0.8),
            (WICK - 0.1, WICK + 0.35),
            (12.0, top),
            WAX_OLD,
        );
    }
    g.bx(
        "A4 wick",
        [CX - 0.6, WICK_TOP, CZ - 0.6],
        [CX + 0.6, WICK_TOP + 3.5, CZ + 0.6],
        IRON,
    );
    g.deco(
        "A4 flame",
        [CX - 0.9, WICK_TOP + 3.0, CZ - 0.9],
        [CX + 0.9, WICK_TOP + 7.5, CZ + 0.9],
        MOLTEN,
    );
    g.deco(
        "A4 flame tip",
        [CX - 0.4, WICK_TOP + 7.5, CZ - 0.4],
        [CX + 0.4, WICK_TOP + 9.5, CZ + 0.4],
        MOLTEN,
    );
    g.secret(
        "secret 2: the top of the Great Wick",
        vol(
            [CX - WICK, WICK_TOP, CZ - WICK],
            [CX + WICK, WICK_TOP + 3.0, CZ + WICK],
        ),
    );
    // Wax floes in the well; the south-west one carries a pad up onto the Great Wick.
    for (k, d) in [(0, 9.0), (2, 9.0), (4, 9.0), (6, 9.0), (5, 9.0)] {
        let (fx, fz) = super::furnace_ring::at(k, 0.0, d);
        g.bx(
            "A4 wax floe",
            [CX + fx - 1.4, 4.0, CZ + fz - 1.4],
            [CX + fx + 1.4, 11.2, CZ + fz + 1.4],
            WAX_OLD,
        );
    }
    let (fx, fz) = super::furnace_ring::at(5, 0.0, 9.0);
    let (fx, fz) = (CX + fx, CZ + fz);
    g.pad(
        "A4 south-west floe -> the top of the Great Wick",
        vol([fx - 1.0, 11.2, fz - 1.0], [fx + 1.0, 12.2, fz + 1.0]),
        pad_velocity([fx, 11.2, fz], [CX - 2.5, WICK_TOP, CZ + 2.5], 2.5),
    );
    // Galleries (y 21) along the west, north and east walls; the south wall ramp up to the
    // west gallery.
    g.bx(
        "A4 west gallery",
        [x0, GY - 0.6, z0],
        [x0 + 7.0, GY, z1],
        TIMBER,
    );
    g.bx(
        "A4 north gallery",
        [x0 + 7.0, GY - 0.6, z0],
        [x1, GY, z0 + 7.0],
        TIMBER,
    );
    g.bx(
        "A4 east gallery",
        [x1 - 7.0, GY - 0.6, z0 + 7.0],
        [x1, GY, -13.0],
        TIMBER,
    );
    g.bx(
        "A4 west gallery edge",
        [x0 + 7.0, GY - 0.9, z0 + 7.0],
        [x0 + 7.3, GY - 0.6, -11.5],
        VERDIGRIS,
    );
    g.bx(
        "A4 north gallery edge",
        [x0 + 7.0, GY - 0.9, z0 + 7.0],
        [x1 - 7.0, GY - 0.6, z0 + 7.3],
        VERDIGRIS,
    );
    g.bx(
        "A4 east gallery edge",
        [x1 - 7.3, GY - 0.9, z0 + 7.3],
        [x1 - 7.0, GY - 0.6, -13.0],
        VERDIGRIS,
    );
    for (x, z) in [
        (-104.0, -36.0),
        (-104.0, -20.0),
        (-96.0, -44.0),
        (-80.0, -44.0),
        (-72.0, -36.0),
        (-72.0, -20.0),
    ] {
        g.bx(
            "A4 gallery post",
            [x - 0.3, 11.0, z - 0.3],
            [x + 0.3, GY - 0.6, z + 0.3],
            IRON,
        );
    }
    g.ramp_x(
        "A4 south ramp",
        (-11.5, -6.0),
        (-75.0, 11.0),
        (-103.0, GY),
        9.0,
        STONE,
    );
    g.pad(
        "A4 floor -> north gallery",
        vol([-79.2, 11.0, -39.2], [-76.8, 12.0, -36.8]),
        pad_velocity([-78.0, 11.0, -38.0], [-82.0, GY, -47.0], 4.0),
    );
    for (x, z) in [
        (-106.5, -47.0),
        (-70.0, -47.0),
        (-106.5, -9.0),
        (-70.0, -16.0),
    ] {
        candles(&mut g, "A4 gallery candles", [x, GY, z], true);
    }
    g.light("A4 the flame", [CX, WICK_TOP + 5.0, CZ], L_FLAME, 36.0);
    g.light("A4 the Great Wick", [CX, 24.0, CZ], L_FLAME, 18.0);
    for k in [1, 3, 5, 7] {
        let (lx, lz) = super::furnace_ring::at(k, 0.0, 10.0);
        g.light("A4 the well", [CX + lx, 8.5, CZ + lz], L_WATER, 11.0);
    }
    g.light("A4 smoke hole", [CX, 49.0, CZ], L_SKY, 12.0);
    g.light("A4 entry", [-69.0, 15.0, -27.5], L_CANDLE, 9.0);
    g.checkpoint(
        "checkpoint 4: A4 vestibule (under the east gallery)",
        vol([-72.0, 10.0, -33.0], [-66.0, 17.0, -22.0]),
        [-69.0, 11.0, -27.5],
        90.0,
    );
    g.encounter(
        "encounter 4: The Great Wick — everything, three waves, round the flooded well",
        &a4,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-110.0, 3.5, -50.0], [-72.0, 50.0, -6.0]),
            lock_doors: vec![7],
            unlock_doors: vec![8],
            waves: vec![
                wave(
                    0.5,
                    vec![
                        enemy("gnawer", [-88.0, 11.0, -45.5], 90.0),
                        enemy("gnawer", [-104.0, 11.0, -40.0], 90.0),
                        enemy("gnawer", [-104.0, 11.0, -16.0], 90.0),
                        enemy("gnawer", [-88.0, 11.0, -12.5], 90.0),
                        enemy("snuffer", [-96.0, 11.0, -45.5], 90.0),
                        enemy("snuffer", [-80.0, 11.0, -45.5], 90.0),
                        enemy("censer", [-95.0, GY, -46.5], 90.0),
                        enemy("censer", [-106.5, GY, -30.0], 90.0),
                        enemy("bellwether", [-105.5, 11.0, -28.0], 90.0),
                    ],
                ),
                wave(
                    1.5,
                    vec![
                        enemy("smelter", [-78.0, 11.0, -45.5], 90.0),
                        enemy("heron", [-82.0, GY, -46.5], 90.0),
                        enemy("heron", [-106.5, GY, -40.0], 90.0),
                        enemy("harrier", [-81.6, 14.0, -34.4], 90.0),
                        enemy("harrier", [-94.4, 14.0, -34.4], 90.0),
                        enemy("harrier", [-81.6, 14.0, -21.6], 90.0),
                        enemy("gnawer", [-104.0, 11.0, -46.0], 90.0),
                        enemy("gnawer", [-79.0, 11.0, -12.5], 90.0),
                    ],
                ),
                wave(
                    1.5,
                    vec![
                        enemy("gaoler", [-100.0, 11.0, -45.5], 90.0),
                        enemy("heron", [-104.5, GY, -46.5], 90.0),
                        enemy("censer", [-99.0, GY, -46.5], 90.0),
                        enemy("censer", [-77.0, GY, -46.5], 90.0),
                        enemy("harrier", [-94.4, 14.0, -34.4], 90.0),
                        enemy("harrier", [-81.6, 14.0, -21.6], 90.0),
                        enemy("harrier", [-81.6, 14.0, -34.4], 90.0),
                        enemy("snuffer", [-104.0, 11.0, -34.0], 90.0),
                        enemy("snuffer", [-104.0, 11.0, -22.0], 90.0),
                        enemy("bellwether", [-106.5, 11.0, -40.0], 90.0),
                    ],
                ),
            ],
        },
    );

    // ---------------------------------------------------------------- C5 + the Wick Terrace (exit)
    g.section("C5 and the Wick Terrace (exit, open to the sky)");
    let c5 = Room::new("C5", [-91.5, GY, -66.0], [-85.5, GY + 6.0, -51.0], STONE)
        .ceiling(STONE_DARK)
        .open(&[N, S]);
    g.room(&c5);
    g.bx(
        "C5 floor",
        [-91.5, GY - 1.0, -66.0],
        [-85.5, GY, -51.0],
        TILE,
    );
    g.light("C5", [-88.5, GY + 4.5, -58.5], L_CANDLE, 9.0);
    let x = Room::new("Exit", [-97.0, GY, -82.0], [-80.0, GY + 8.0, -67.0], BRICK)
        .floor(TILE)
        .opening(op(S, -91.0, -86.0, GY, GY + 5.0));
    g.room(&x);
    g.clip(
        "Exit sky clip",
        [-98.0, GY + 8.0, -83.0],
        [-79.0, GY + 9.0, -66.0],
    );
    g.bx(
        "Exit dais",
        [-92.0, GY, -79.0],
        [-85.0, GY + 0.3, -72.0],
        WAXSTONE,
    );
    for (dx, dz) in [(-2.5, -2.5), (2.5, -2.5), (-2.5, 2.5), (2.5, 2.5)] {
        candles(
            &mut g,
            "Exit candles",
            [-88.5 + dx, GY + 0.3, -75.5 + dz],
            false,
        );
    }
    g.light("Exit", [-88.5, GY + 5.0, -75.5], L_CANDLE, 14.0);
    g.light("Exit sky", [-88.5, GY + 9.0, -75.5], L_SKY, 16.0);
    g.def.exit = Some(vol([-92.0, GY, -79.0], [-85.0, GY + 5.0, -72.0]));

    g.kill(
        "below the world",
        vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]),
    );
    g
}
