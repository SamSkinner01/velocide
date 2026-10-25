//! "Spire of Wicks" — the second co-op campaign level (1-4 players): a climb through a
//! lost-wax bell foundry built inside a hollow spire. Wax, bronze, verdigris and cold stone,
//! lit by thousands of candles; molten bronze only at the bottom of the bell shaft and in the
//! Rood's chasm. The rooms spiral up around the central Bell Shaft: crypt and casting floor
//! at its foot (south), the Hoist (west), the Rood (north), the Wick Stair and finally the
//! Belfry on top of the shaft, where the great bell hangs over a 48 m drop. See
//! `docs/levels.md` for the map, the encounters and the routes.

use glam::Vec3;
use shared::level::{Encounter, Environment, LevelKind};

use super::{Gen, Mat, Room, Side, enemy, op, vol, wave};

mod pal {
    use super::Mat;
    /// Cold pale stone (most walls).
    pub const STONE: Mat = Mat("concrete", [0.78, 0.8, 0.9]);
    pub const STONE_DARK: Mat = Mat("concrete", [0.56, 0.58, 0.68]);
    /// Stone warmed by the casting pits.
    pub const STONE_WARM: Mat = Mat("concrete", [0.9, 0.84, 0.8]);
    /// Blue-grey shaft stone (the Hoist).
    pub const ASHLAR: Mat = Mat("concrete", [0.64, 0.7, 0.84]);
    /// Floor flags.
    pub const TILE: Mat = Mat("tile", [0.6, 0.62, 0.7]);
    /// Fresh ivory wax.
    pub const WAX: Mat = Mat("bone", [1.0, 0.95, 0.82]);
    /// Old, yellowed wax (drips, effigies, candle stubs).
    pub const WAX_OLD: Mat = Mat("bone", [0.86, 0.78, 0.62]);
    /// Bell bronze.
    pub const BRONZE: Mat = Mat("metal", [0.86, 0.64, 0.42]);
    pub const BRASS: Mat = Mat("metal", [1.0, 0.84, 0.5]);
    /// Weathered copper.
    pub const VERDIGRIS: Mat = Mat("metal", [0.5, 0.84, 0.74]);
    pub const IRON: Mat = Mat("metal", [0.5, 0.52, 0.6]);
    /// Bell-frame timber.
    pub const TIMBER: Mat = Mat("wood", [0.66, 0.56, 0.48]);
    pub const ROPE: Mat = Mat("wood", [0.9, 0.8, 0.6]);
    pub const CLAY: Mat = Mat("rust", [0.8, 0.66, 0.56]);
    pub const GRATE: Mat = Mat("grate", [0.7, 0.74, 0.82]);
    pub const GLASS: Mat = Mat("glass", [0.95, 0.9, 0.8]);
    pub const HAZARD: Mat = Mat("hazard", [1.0, 0.86, 0.52]);
    /// Molten bronze / slag.
    pub const SLAG: Mat = Mat("lava", [1.0, 1.0, 1.0]);
    pub const DOOR: Mat = Mat("panel", [0.55, 0.86, 0.78]);
    pub const EXIT_DOOR: Mat = Mat("hazard", [1.0, 0.8, 0.5]);
    pub const PANEL: Mat = Mat("panel", [0.8, 0.8, 0.9]);
}

// Light colours.
/// Pale candle gold.
const L_CANDLE: [f32; 3] = [1.25, 0.98, 0.62];
/// Orange wick flame.
const L_WICK: [f32; 3] = [1.3, 0.7, 0.28];
/// Molten bronze glow.
const L_MOLTEN: [f32; 3] = [1.35, 0.55, 0.16];
/// Cold moonlight through the belfry arches.
const L_MOON: [f32; 3] = [0.55, 0.66, 1.05];
const L_VERDIGRIS: [f32; 3] = [0.38, 1.0, 0.8];
/// The great wick in the lantern.
const L_FLAME: [f32; 3] = [1.4, 1.15, 0.8];

/// Jump-pad velocity that throws a player from `from` (feet) to land on `to` with no input,
/// peaking `over` metres above the landing height.
fn pad_velocity(from: [f32; 3], to: [f32; 3], over: f32) -> [f32; 3] {
    let g = shared::tuning::MovementTuning::default().gravity;
    let rise = to[1] - from[1] + over;
    let vy = (2.0 * g * rise).sqrt();
    let t = vy / g + (2.0 * over / g).sqrt();
    let r = |x: f32| (x * 100.0).round() / 100.0;
    [r((to[0] - from[0]) / t), r(vy), r((to[2] - from[2]) / t)]
}

pub fn spire() -> Gen {
    use Side::*;
    use pal::*;
    let mut g = Gen::new(
        "Spire of Wicks",
        LevelKind::Campaign,
        Environment {
            sky_color: Vec3::new(0.07, 0.08, 0.16),
            fog_color: Vec3::new(0.065, 0.065, 0.12),
            fog_start: 34.0,
            fog_end: 170.0,
            ambient: Vec3::new(0.5, 0.5, 0.58),
            light_dir: Vec3::new(0.3, -1.0, 0.45),
            light_color: Vec3::new(0.4, 0.46, 0.62),
            music: "campaign".into(),
        },
    );

    // Par for the results screen: ~30 s of movement + four fights (see docs/levels.md).
    g.def.par_time = 300.0;

    // ---------------------------------------------------------------- R0 Wax Crypt (start)
    g.section("R0 Wax Crypt (start)");
    let r0 = Room::new("R0", [-11.0, 0.0, 60.0], [11.0, 10.0, 82.0], STONE_DARK)
        .floor(TILE)
        .ceiling(STONE_DARK)
        .opening(op(N, -2.5, 2.5, 0.0, 5.0));
    g.room(&r0);
    // Biers with wax effigies along both walls, candle stubs at their heads.
    for x in [-7.5f32, 7.5] {
        for (z0, z1) in [(63.0, 68.0), (71.0, 76.0)] {
            g.bx("R0 bier", [x - 1.4, 0.0, z0], [x + 1.4, 1.0, z1], STONE);
            g.bx("R0 wax effigy", [x - 0.8, 1.0, z0 + 0.5], [x + 0.8, 1.6, z1 - 0.5], WAX);
            g.deco("R0 candle stub", [x - 0.6, 1.0, z0 + 0.1], [x - 0.2, 1.9, z0 + 0.4], WAX_OLD);
            g.deco("R0 candle stub", [x + 0.2, 1.0, z0 + 0.1], [x + 0.6, 2.3, z0 + 0.4], WAX_OLD);
        }
    }
    // The cracked bell at the back of the crypt, its seam still glowing.
    g.bx("R0 cracked bell lip", [-4.5, 0.0, 77.5], [4.5, 0.6, 82.0], BRONZE);
    g.bx("R0 cracked bell", [-3.5, 0.6, 78.5], [3.5, 5.6, 82.0], BRONZE);
    g.bx("R0 cracked bell crown", [-1.5, 5.6, 79.5], [1.5, 6.6, 82.0], BRASS);
    g.bx("R0 cracked bell seam (behind a grille)", [-0.4, 1.2, 78.4], [0.4, 5.0, 78.5], SLAG);
    g.bx("R0 cracked bell grille", [-0.6, 1.0, 78.2], [0.6, 5.2, 78.4], GRATE);
    for z in [62.0, 70.0, 78.0] {
        g.bx("R0 vault rib", [-11.0, 9.2, z - 0.4], [11.0, 10.0, z + 0.4], TIMBER);
    }
    g.bx("R0 door frame W", [-3.5, 0.0, 60.0], [-2.5, 5.5, 60.4], VERDIGRIS);
    g.bx("R0 door frame E", [2.5, 0.0, 60.0], [3.5, 5.5, 60.4], VERDIGRIS);
    g.bx("R0 door frame top", [-2.5, 5.0, 60.0], [2.5, 5.5, 60.4], VERDIGRIS);
    g.light("R0 crypt", [0.0, 7.5, 70.0], L_CANDLE, 16.0);
    g.light("R0 biers W", [-7.5, 2.8, 69.5], L_WICK, 7.0);
    g.light("R0 biers E", [7.5, 2.8, 69.5], L_WICK, 7.0);
    g.light("R0 cracked bell", [0.0, 3.0, 76.0], L_MOLTEN, 8.0);
    g.light("R0 way out", [0.0, 6.0, 62.0], L_VERDIGRIS, 8.0);
    for x in [-4.5, -1.5, 1.5, 4.5] {
        g.spawn([x, 0.0, 74.0], 0.0);
    }

    // ---------------------------------------------------------------- C1 Fallen Bell
    g.section("C1 Fallen Bell (corridor: slide under the bell W / hop its crown E)");
    let c1 = Room::new("C1", [-4.0, 0.0, 42.0], [4.0, 7.0, 59.0], STONE)
        .floor(TILE)
        .ceiling(STONE_DARK)
        .open(&[N, S]);
    g.room(&c1);
    // A bell that fell from the shaft lies across the west lane, propped on its lip: 1.2 m
    // under it (a slide), its broken crown is a 1 m hurdle in the east lane.
    g.bx("C1 fallen bell lip (propped on the wall)", [-4.0, 0.0, 48.0], [-3.4, 1.2, 53.0], BRASS);
    g.bx("C1 fallen bell (slide under)", [-4.0, 1.2, 48.0], [0.5, 7.0, 53.0], BRONZE);
    g.bx("C1 broken crown (jump over)", [0.5, 0.0, 50.0], [4.0, 1.0, 51.0], BRASS);
    g.light("C1 south", [0.0, 5.5, 56.5], L_CANDLE, 9.0);
    g.light("C1 under the bell", [-1.75, 0.6, 50.5], L_WICK, 5.0);
    g.light("C1 north", [0.0, 5.5, 44.5], L_VERDIGRIS, 9.0);

    // ---------------------------------------------------------------- A1 Casting Floor
    g.section("A1 Casting Floor (arena 1: gnawers, censers on the balconies)");
    let a1_in = op(S, -2.5, 2.5, 0.0, 5.0);
    let a1_out = op(W, 14.5, 19.5, 0.0, 5.0);
    let a1 = Room::new("A1", [-24.0, 0.0, 11.0], [24.0, 18.0, 41.0], STONE_WARM)
        .ceiling(STONE_DARK)
        .bottom(-3.0)
        .opening(a1_in)
        .opening(a1_out)
        // Window into the Bell Shaft (glazed in the shaft's wall).
        .opening(op(N, -6.0, 6.0, 2.0, 12.0));
    g.room(&a1);
    g.door(1, "door 1: A1 entry (locked by encounter 1)", &a1, a1_in, true, DOOR);
    g.door(2, "door 2: A1 exit to the Hoist (opened by encounter 1)", &a1, a1_out, false, EXIT_DOOR);
    // Floor around the casting pit (x -6..6, z 22..30).
    g.bx("A1 floor N", [-24.0, -3.0, 11.0], [24.0, 0.0, 22.0], TILE);
    g.bx("A1 floor S", [-24.0, -3.0, 30.0], [24.0, 0.0, 41.0], TILE);
    g.bx("A1 floor W", [-24.0, -3.0, 22.0], [-6.0, 0.0, 30.0], TILE);
    g.bx("A1 floor E", [6.0, -3.0, 22.0], [24.0, 0.0, 30.0], TILE);
    g.bx("A1 casting pit bottom", [-6.0, -3.0, 22.0], [6.0, -2.5, 30.0], STONE_DARK);
    g.bx("A1 casting pit bronze", [-6.0, -2.5, 22.0], [6.0, -2.2, 30.0], SLAG);
    g.kill("A1 casting pit", vol([-6.0, -2.5, 22.0], [6.0, -1.0, 30.0]));
    g.bx("A1 pit edge N", [-6.0, -0.12, 22.0], [6.0, 0.0, 22.3], HAZARD);
    g.bx("A1 pit edge S", [-6.0, -0.12, 29.7], [6.0, 0.0, 30.0], HAZARD);
    g.bx("A1 pit edge W", [-6.0, -0.12, 22.3], [-5.7, 0.0, 29.7], HAZARD);
    g.bx("A1 pit edge E", [5.7, -0.12, 22.3], [6.0, 0.0, 29.7], HAZARD);
    // Pouring ladle over the pit.
    g.bx("A1 ladle", [-2.0, 10.0, 24.0], [2.0, 12.5, 28.0], CLAY);
    g.bx("A1 ladle glow", [-1.5, 9.9, 24.5], [1.5, 10.0, 27.5], SLAG);
    g.deco("A1 ladle chain", [-1.8, 12.5, 25.8], [-1.4, 17.2, 26.2], IRON);
    g.deco("A1 ladle chain", [1.4, 12.5, 25.8], [1.8, 17.2, 26.2], IRON);
    // Balconies (censer ledges, 8 m deep) with ramps from the south.
    for (sx, (x0, x1), (r0, r1), (p0, p1)) in [
        (-1.0f32, (-24.0, -16.0), (-24.0, -19.0), (-17.0, -16.0)),
        (1.0, (16.0, 24.0), (19.0, 24.0), (16.0, 17.0)),
    ] {
        g.bx("A1 balcony", [x0, 6.4, 11.0], [x1, 7.0, 25.0], IRON);
        let (f0, f1) = if sx < 0.0 { (x1, x1 + 0.3) } else { (x0 - 0.3, x0) };
        g.bx("A1 balcony fascia", [f0, 6.3, 11.0], [f1, 7.1, 25.0], HAZARD);
        g.ramp_z("A1 balcony ramp", (r0, r1), (40.0, 0.0), (25.0, 7.0), 8.0, STONE);
        for z in [12.0, 23.0] {
            g.bx("A1 balcony post", [p0, 0.0, z], [p1, 6.4, z + 1.0], BRONZE);
        }
    }
    // Wall-jump chimney beside the west balcony: 3.5 m between the north wall and a mould rack.
    g.bx("A1 mould rack (wall-jump fin)", [-15.0, 0.0, 14.5], [-8.0, 9.0, 15.5], TIMBER);
    // Bell moulds flanking the pit: cover, and a slam-bounce perch (5.5 m).
    for x in [-11.0f32, 11.0] {
        g.bx("A1 bell mould", [x - 2.0, 0.0, 23.0], [x + 2.0, 5.5, 29.0], CLAY);
        g.deco("A1 mould band", [x - 2.2, 2.4, 22.8], [x + 2.2, 2.9, 29.2], IRON);
        g.deco("A1 mould wax sprue", [x - 0.4, 5.5, 25.6], [x + 0.4, 6.1, 26.4], WAX);
    }
    for z in [15.0, 26.0, 37.0] {
        g.deco("A1 roof truss", [-24.0, 17.2, z - 0.4], [24.0, 18.0, z + 0.4], TIMBER);
    }
    g.light("A1 entry", [0.0, 9.0, 37.0], L_CANDLE, 14.0);
    g.light("A1 pit", [0.0, 1.5, 26.0], L_MOLTEN, 12.0);
    g.light("A1 ladle", [0.0, 8.5, 26.0], L_WICK, 9.0);
    g.light("A1 balcony W", [-20.0, 11.0, 18.0], L_CANDLE, 12.0);
    g.light("A1 balcony E", [20.0, 11.0, 18.0], L_CANDLE, 12.0);
    g.light("A1 shaft window", [0.0, 6.0, 13.0], L_MOLTEN, 8.0);
    g.light("A1 exit", [-21.0, 4.0, 17.0], L_VERDIGRIS, 8.0);
    g.checkpoint(
        "checkpoint 1: A1 vestibule",
        vol([-14.0, -1.0, 35.0], [14.0, 6.0, 41.0]),
        [0.0, 0.0, 38.0],
        0.0,
    );
    g.encounter(
        "encounter 1: Casting Floor — gnawers, then censers on the balconies",
        &a1,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-24.0, -3.0, 11.0], [24.0, 18.0, 34.0]),
            lock_doors: vec![1],
            unlock_doors: vec![2],
            waves: vec![
                wave(0.5, vec![
                    enemy("gnawer", [-15.0, 0.0, 19.0], 180.0),
                    enemy("gnawer", [15.0, 0.0, 19.0], 180.0),
                    enemy("gnawer", [0.0, 0.0, 16.0], 180.0),
                    enemy("gnawer", [-6.0, 0.0, 14.0], 180.0),
                    enemy("gnawer", [6.0, 0.0, 14.0], 180.0),
                ]),
                wave(1.5, vec![
                    enemy("censer", [-20.0, 7.0, 15.0], 180.0),
                    enemy("censer", [20.0, 7.0, 15.0], 180.0),
                    enemy("gnawer", [-18.0, 0.0, 31.0], 180.0),
                    enemy("gnawer", [18.0, 0.0, 31.0], 180.0),
                    enemy("gnawer", [-3.0, 0.0, 15.0], 180.0),
                    enemy("gnawer", [3.0, 0.0, 15.0], 180.0),
                    enemy("gnawer", [11.0, 0.0, 18.0], 180.0),
                ]),
            ],
        },
    );

    // ---------------------------------------------------------------- Bell Shaft
    g.section("Bell Shaft (the spire's core: molten bronze below, the great bell above)");
    let shaft = Room::new("Shaft", [-9.0, -11.0, -9.0], [9.0, 35.0, 9.0], STONE_DARK)
        .bottom(-12.0)
        .opening(op(S, -6.0, 6.0, 2.0, 12.0));
    g.room(&shaft);
    g.bx("Shaft window glass", [-6.0, 2.0, 9.45], [6.0, 12.0, 9.55], GLASS);
    g.deco("Shaft window mullion", [-0.2, 2.0, 9.6], [0.2, 12.0, 10.0], VERDIGRIS);
    g.bx("Shaft pool floor", [-9.0, -12.0, -9.0], [9.0, -11.0, 9.0], STONE_DARK);
    g.bx("Shaft pool bronze", [-9.0, -11.0, -9.0], [9.0, -10.6, 9.0], SLAG);
    // Falling down the shaft from the Belfry ends 8 m below its floor.
    g.kill("Shaft (pool and the drop from the Belfry)", vol([-9.0, -11.0, -9.0], [9.0, 28.0, 9.0]));
    g.deco("Shaft bell rope", [-0.15, -6.0, -0.15], [0.15, 45.4, 0.15], ROPE);
    for (x, z) in [(-8.4, -8.4), (8.4, -8.4), (-8.4, 8.4), (8.4, 8.4)] {
        g.deco("Shaft verdigris pipe", [x - 0.3, -10.6, z - 0.3], [x + 0.3, 35.0, z + 0.3], VERDIGRIS);
    }
    g.light("Shaft pool", [0.0, -6.0, 0.0], L_MOLTEN, 16.0);
    g.light("Shaft at the window", [0.0, 6.0, -3.0], L_MOLTEN, 14.0);
    g.light("Shaft middle", [0.0, 22.0, 0.0], L_WICK, 14.0);

    // ---------------------------------------------------------------- C2 Wick Gate
    g.section("C2 Wick Gate (A1 -> Hoist)");
    let c2 = Room::new("C2", [-29.0, 0.0, 14.0], [-25.0, 6.0, 20.0], STONE_DARK)
        .floor(TILE)
        .ceiling(STONE_DARK)
        .open(&[W, E]);
    g.room(&c2);
    g.deco("C2 candle shelf", [-28.0, 2.0, 19.6], [-26.0, 2.2, 20.0], WAX_OLD);
    g.light("C2", [-27.0, 4.5, 17.0], L_CANDLE, 7.0);

    // ---------------------------------------------------------------- A2 The Hoist
    g.section("A2 The Hoist (arena 2: tall shaft, harriers; spiral ramps, pad line of cages)");
    let a2_in = op(E, 14.5, 19.5, 0.0, 5.0);
    let a2_out = op(N, -59.5, -54.5, 26.0, 31.0);
    let a2 = Room::new("A2", [-60.0, 0.0, -14.0], [-30.0, 42.0, 22.0], ASHLAR)
        .floor(TILE)
        .ceiling(TIMBER)
        .opening(a2_in)
        .opening(a2_out);
    g.room(&a2);
    g.door(3, "door 3: Hoist entry (locked by encounter 2)", &a2, a2_in, true, DOOR);
    g.door(4, "door 4: Hoist exit at the top of the spiral (opened by encounter 2)", &a2, a2_out, false, EXIT_DOOR);
    // One full turn of ramps around the walls: W (south), S (east), E (north), N (west).
    g.ramp_z("A2 ramp 1 (W wall, rising south)", (-60.0, -55.0), (-8.0, 0.0), (16.0, 6.5), 8.0, STONE);
    g.bx("A2 SW landing", [-60.0, 0.0, 16.0], [-54.0, 6.5, 22.0], STONE);
    g.slope("A2 ramp 2 (S wall, rising east)", true, (17.0, 22.0), (-54.0, 6.5), (-36.0, 13.0), 0.8, true, (0.6, 0.0), STONE);
    g.bx("A2 SE landing", [-36.0, 12.2, 16.0], [-30.0, 13.0, 22.0], STONE);
    g.slope("A2 ramp 3 (E wall, rising north)", false, (-35.0, -30.0), (16.0, 13.0), (-8.0, 19.5), 0.8, true, (0.6, 0.0), STONE);
    g.bx("A2 NE landing", [-36.0, 18.7, -14.0], [-30.0, 19.5, -8.0], STONE);
    g.slope("A2 ramp 4 (N wall, rising west)", true, (-14.0, -9.0), (-36.0, 19.5), (-54.0, 26.0), 0.8, true, (0.6, 0.0), STONE);
    g.bx("A2 NW landing", [-60.0, 25.2, -14.0], [-54.0, 26.0, -8.0], STONE);
    // Corbels under the landings and ramp ends.
    g.deco("A2 corbel SE", [-31.0, 11.0, 16.0], [-30.0, 12.2, 22.0], BRONZE);
    g.deco("A2 corbel NE", [-31.0, 17.5, -14.0], [-30.0, 18.7, -8.0], BRONZE);
    g.deco("A2 corbel NW", [-60.0, 24.0, -14.0], [-59.0, 25.2, -8.0], BRONZE);
    g.deco("A2 corbel ramp 2", [-45.5, 8.0, 21.0], [-44.5, 9.0, 22.0], BRONZE);
    g.deco("A2 corbel ramp 3", [-31.0, 14.5, 3.5], [-30.0, 15.5, 4.5], BRONZE);
    g.deco("A2 corbel ramp 4", [-45.5, 21.5, -14.0], [-44.5, 22.5, -13.0], BRONZE);
    // The pad line: a floor pad and two hanging hoist cages, each throwing you to the next,
    // the last onto the NW landing (the top of the spiral).
    let cage_a = ([-51.5f32, -46.5f32], [3.0f32, 8.0f32], 9.0f32);
    let cage_b = ([-53.5f32, -48.5f32], [-5.0f32, 0.0f32], 17.5f32);
    for (name, (xs, zs, y)) in [("A", cage_a), ("B", cage_b)] {
        g.bx(&format!("A2 hoist cage {name} floor"), [xs[0], y - 0.6, zs[0]], [xs[1], y, zs[1]], IRON);
        for (x, z) in [(xs[0], zs[0]), (xs[1] - 0.2, zs[0]), (xs[0], zs[1] - 0.2), (xs[1] - 0.2, zs[1] - 0.2)] {
            g.deco(&format!("A2 hoist cage {name} bar"), [x, y, z], [x + 0.2, y + 3.0, z + 0.2], BRASS);
        }
        g.deco(&format!("A2 hoist cage {name} chain"), [(xs[0] + xs[1]) * 0.5 - 0.15, y + 3.0, (zs[0] + zs[1]) * 0.5 - 0.15], [(xs[0] + xs[1]) * 0.5 + 0.15, 42.0, (zs[0] + zs[1]) * 0.5 + 0.15], IRON);
    }
    let centre = |(xs, zs, y): ([f32; 2], [f32; 2], f32)| [(xs[0] + xs[1]) * 0.5, y, (zs[0] + zs[1]) * 0.5];
    let hoist = [
        ("A2 hoist pad: floor -> cage A", [-46.0, 0.0, 12.0], centre(cage_a)),
        ("A2 hoist pad: cage A -> cage B", centre(cage_a), centre(cage_b)),
        ("A2 hoist pad: cage B -> NW landing", centre(cage_b), [-57.0, 26.0, -11.0]),
    ];
    for (note, from, to) in hoist {
        g.pad(note, vol([from[0] - 1.2, from[1], from[2] - 1.2], [from[0] + 1.2, from[1] + 1.0, from[2] + 1.2]), pad_velocity(from, to, 4.5));
        g.bx("A2 pad ring", [from[0] - 1.5, from[1], from[2] - 1.5], [from[0] + 1.5, from[1] + 0.06, from[2] + 1.5], HAZARD);
    }
    // Verdigris downpipes and wax weeping down the shaft walls in sheets.
    for z in [-4.0, 10.0] {
        g.deco("A2 verdigris pipe W", [-60.0, 0.0, z - 0.4], [-59.4, 42.0, z + 0.4], VERDIGRIS);
    }
    for x in [-48.0, -42.0] {
        g.deco("A2 verdigris pipe N", [x - 0.4, 0.0, -14.0], [x + 0.4, 42.0, -13.4], VERDIGRIS);
    }
    g.deco("A2 wax sheet S", [-50.0, 18.0, 21.6], [-44.0, 36.0, 22.0], WAX_OLD);
    g.deco("A2 wax sheet E", [-30.4, 24.0, -2.0], [-30.0, 38.0, 6.0], WAX_OLD);
    g.deco("A2 hoist wheel", [-48.0, 32.0, 3.0], [-42.0, 41.0, 4.0], TIMBER);
    g.deco("A2 hoist axle", [-45.5, 36.0, -2.0], [-44.5, 37.0, 22.0], IRON);
    g.light("A2 entry", [-33.0, 4.0, 17.0], L_CANDLE, 10.0);
    g.light("A2 floor", [-45.0, 6.0, 4.0], L_WICK, 16.0);
    g.light("A2 SW landing", [-57.0, 10.0, 19.0], L_CANDLE, 10.0);
    g.light("A2 SE landing", [-33.0, 16.5, 19.0], L_CANDLE, 10.0);
    g.light("A2 NE landing", [-33.0, 23.0, -11.0], L_CANDLE, 10.0);
    g.light("A2 NW landing", [-57.0, 29.5, -11.0], L_VERDIGRIS, 10.0);
    g.light("A2 cage A", [-49.0, 11.5, 5.5], L_WICK, 7.0);
    g.light("A2 high", [-45.0, 36.0, 4.0], L_MOON, 18.0);
    g.checkpoint(
        "checkpoint 2: Hoist vestibule (under the SE landing)",
        vol([-36.0, -1.0, 8.0], [-30.0, 6.0, 22.0]),
        [-33.0, 0.0, 17.0],
        90.0,
    );
    g.encounter(
        "encounter 2: The Hoist — harriers in the shaft, gnawers on the floor and landings",
        &a2,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-60.0, -1.0, -14.0], [-37.0, 42.0, 22.0]),
            lock_doors: vec![3],
            unlock_doors: vec![4],
            waves: vec![
                wave(0.5, vec![
                    enemy("harrier", [-44.0, 7.0, 8.0], 90.0),
                    enemy("harrier", [-50.0, 8.0, -4.0], 90.0),
                    enemy("harrier", [-40.0, 8.0, -6.0], 90.0),
                    enemy("gnawer", [-50.0, 0.0, 13.0], 90.0),
                    enemy("gnawer", [-46.0, 0.0, -10.0], 90.0),
                    enemy("gnawer", [-41.0, 0.0, 2.0], 90.0),
                ]),
                wave(1.5, vec![
                    enemy("harrier", [-57.0, 13.0, 19.0], 90.0),
                    enemy("harrier", [-45.0, 17.0, 19.5], 90.0),
                    enemy("harrier", [-42.0, 30.0, -11.5], 90.0),
                    enemy("harrier", [-49.0, 31.0, -11.5], 90.0),
                    enemy("gnawer", [-57.0, 6.5, 19.0], 90.0),
                    enemy("gnawer", [-57.0, 26.0, -11.0], 90.0),
                    enemy("gnawer", [-52.0, 0.0, -6.0], 90.0),
                    enemy("gnawer", [-40.0, 0.0, 12.0], 90.0),
                ]),
            ],
        },
    );

    // ---------------------------------------------------------------- C3 Rope Walk
    g.section("C3 Rope Walk (Hoist -> Rood)");
    let c3 = Room::new("C3", [-60.0, 26.0, -38.0], [-54.0, 33.0, -15.0], STONE_DARK)
        .floor(STONE)
        .ceiling(TIMBER)
        .bottom(25.0)
        .open(&[N, S]);
    g.room(&c3);
    for z in [-21.0, -27.0, -33.0] {
        g.deco("C3 hanging rope", [-57.15, 29.0, z - 0.15], [-56.85, 32.4, z + 0.15], ROPE);
        g.deco("C3 rope beam", [-60.0, 32.4, z - 0.3], [-54.0, 33.0, z + 0.3], TIMBER);
    }
    g.light("C3 south", [-57.0, 31.0, -19.0], L_CANDLE, 9.0);
    g.light("C3 north", [-57.0, 31.0, -34.0], L_WICK, 9.0);

    // ---------------------------------------------------------------- A3 The Rood
    g.section("A3 The Rood (arena 3: herons on the rood loft across a chasm, a gaoler)");
    let a3_in = op(S, -59.5, -54.5, 26.0, 31.0);
    let a3_out = op(S, 10.5, 15.5, 24.0, 29.0);
    let a3 = Room::new("A3", [-64.0, 1.0, -70.0], [16.0, 44.0, -39.0], STONE)
        .ceiling(TIMBER)
        .bottom(0.0)
        .opening(a3_in)
        .opening(a3_out);
    g.room(&a3);
    g.door(5, "door 5: Rood entry (locked by encounter 3)", &a3, a3_in, true, DOOR);
    g.door(6, "door 6: Rood exit from the loft (opened by encounter 3)", &a3, a3_out, false, EXIT_DOOR);
    g.bx("A3 base", [-64.0, 0.0, -70.0], [16.0, 1.0, -39.0], STONE_DARK);
    g.bx("A3 near gallery", [-64.0, 1.0, -70.0], [-52.0, 26.0, -39.0], STONE);
    // The wax run: a 26 m slope down to the chasm (slide it, slide-jump the chasm).
    g.ramp_x("A3 wax run", (-70.0, -39.0), (-26.0, 14.0), (-52.0, 26.0), 15.0, WAX);
    g.bx("A3 chasm lip", [-26.0, 1.0, -70.0], [-23.0, 14.0, -39.0], STONE);
    g.bx("A3 chasm bronze", [-23.0, 1.0, -70.0], [-9.0, 1.4, -39.0], SLAG);
    g.kill("A3 chasm", vol([-23.0, 1.0, -70.0], [-9.0, 3.5, -39.0]));
    g.bx("A3 far floor", [-9.0, 1.0, -70.0], [16.0, 14.0, -39.0], STONE);
    let bridges = [(-67.0f32, -63.0f32), (-47.0, -43.0)];
    for (z0, z1) in bridges {
        g.bx("A3 bridge", [-23.0, 13.4, z0], [-9.0, 14.0, z1], BRONZE);
    }
    // Hazard strips overhanging both chasm edges, between the bridges.
    for (z0, z1) in [(-70.0, -67.0), (-63.0, -47.0), (-43.0, -39.0)] {
        g.bx("A3 chasm edge W", [-23.0, 13.85, z0], [-22.6, 14.0, z1], HAZARD);
        g.bx("A3 chasm edge E", [-9.4, 13.85, z0], [-9.0, 14.0, z1], HAZARD);
    }
    // The rood loft: an L of galleries 10 m above the far floor (herons, censers), with a
    // ramp up the east wall; the exit is at its south-east end.
    g.bx("A3 rood loft W", [-9.0, 23.4, -70.0], [-3.0, 24.0, -47.0], TIMBER);
    g.bx("A3 rood loft S", [-9.0, 23.4, -47.0], [16.0, 24.0, -39.0], TIMBER);
    g.bx("A3 loft fascia W", [-3.0, 23.3, -70.0], [-2.7, 24.1, -47.3], BRASS);
    g.bx("A3 loft fascia S", [-2.7, 23.3, -47.3], [11.0, 24.1, -47.0], BRASS);
    g.ramp_z("A3 loft ramp (E wall)", (11.0, 16.0), (-68.0, 14.0), (-47.0, 24.0), 11.0, STONE);
    for (x, z) in [(-4.0, -62.0), (-4.0, -54.0), (2.0, -46.0), (9.0, -46.0)] {
        g.bx("A3 rood column", [x - 0.8, 14.0, z - 0.8], [x + 0.8, 23.4, z + 0.8], WAX_OLD);
    }
    g.deco("A3 rood beam", [-9.0, 35.0, -55.5], [-3.0, 36.0, -54.5], TIMBER);
    g.deco("A3 rood bell", [-7.5, 31.0, -57.0], [-4.5, 35.0, -53.0], BRONZE);
    // Wax weeping down the chasm lip, candle racks on the near gallery.
    for z in [-60.0, -55.0, -50.0] {
        g.deco("A3 wax drip (down the chasm lip)", [-23.0, 6.0, z - 1.0], [-22.5, 13.8, z + 1.0], WAX_OLD);
    }
    for z in [-66.0, -58.0, -50.0] {
        g.deco("A3 candle rack", [-63.6, 26.0, z - 1.5], [-63.0, 28.5, z + 1.5], WAX);
    }
    for z in [-66.0, -60.0, -54.0] {
        g.deco("A3 organ pipe", [15.2, 24.0, z - 0.5], [16.0, 38.0, z + 0.5], BRASS);
    }
    g.light("A3 near gallery S", [-58.0, 30.0, -45.0], L_CANDLE, 12.0);
    g.light("A3 near gallery N", [-58.0, 30.0, -64.0], L_CANDLE, 12.0);
    g.light("A3 wax run", [-39.0, 25.0, -55.0], L_WICK, 14.0);
    g.light("A3 chasm S", [-16.0, 4.0, -47.0], L_MOLTEN, 13.0);
    g.light("A3 chasm N", [-16.0, 4.0, -63.0], L_MOLTEN, 13.0);
    g.light("A3 rood loft", [-6.0, 28.0, -58.0], L_CANDLE, 12.0);
    g.light("A3 loft S", [4.0, 28.0, -43.0], L_CANDLE, 12.0);
    g.light("A3 far floor", [4.0, 19.0, -60.0], L_WICK, 14.0);
    g.light("A3 exit", [13.0, 28.0, -41.0], L_VERDIGRIS, 8.0);
    g.light("A3 vault", [-24.0, 40.0, -55.0], L_MOON, 24.0);
    g.checkpoint(
        "checkpoint 3: Rood near gallery",
        vol([-64.0, 25.0, -46.0], [-52.0, 32.0, -39.0]),
        [-57.0, 26.0, -42.0],
        -90.0,
    );
    g.encounter(
        "encounter 3: The Rood — herons across the chasm, then a gaoler on the far floor",
        &a3,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-51.0, 0.0, -70.0], [16.0, 44.0, -39.0]),
            lock_doors: vec![5],
            unlock_doors: vec![6],
            waves: vec![
                wave(0.5, vec![
                    enemy("heron", [-6.0, 24.0, -66.0], -90.0),
                    enemy("heron", [-6.0, 24.0, -56.0], -90.0),
                    enemy("gnawer", [0.0, 14.0, -66.0], 90.0),
                    enemy("gnawer", [4.0, 14.0, -58.0], 90.0),
                    enemy("gnawer", [0.0, 14.0, -50.0], 90.0),
                    enemy("gnawer", [6.0, 14.0, -66.0], 90.0),
                ]),
                wave(1.5, vec![
                    enemy("gaoler", [3.0, 14.0, -58.0], 90.0),
                    enemy("heron", [-6.0, 24.0, -62.0], -90.0),
                    enemy("censer", [-3.0, 24.0, -43.0], 90.0),
                    enemy("censer", [6.0, 24.0, -43.0], 90.0),
                    enemy("harrier", [-38.0, 26.0, -62.0], 90.0),
                    enemy("harrier", [4.0, 21.0, -66.0], 90.0),
                    // Flankers at the foot of the wax run (they climb it behind you).
                    enemy("gnawer", [-24.5, 14.0, -66.0], -90.0),
                    enemy("gnawer", [-24.5, 14.0, -52.0], -90.0),
                ]),
            ],
        },
    );

    // ---------------------------------------------------------------- C4 Wick Stair
    g.section("C4 Wick Stair (one long ramp up to the Belfry: slide-climb it)");
    let c4 = Room::new("C4", [-26.0, 24.0, -37.0], [18.0, 46.0, -26.0], STONE)
        .ceiling(TIMBER)
        .bottom(23.0)
        .opening(op(N, 10.5, 15.5, 24.0, 29.0))
        .opening(op(S, -23.5, -18.5, 36.0, 41.0));
    g.room(&c4);
    g.bx("C4 bottom landing", [10.0, 23.0, -37.0], [18.0, 24.0, -26.0], TILE);
    g.ramp_x("C4 wick stair", (-37.0, -26.0), (10.0, 24.0), (-18.0, 36.0), 13.0, TILE);
    g.bx("C4 top landing", [-26.0, 23.0, -37.0], [-18.0, 36.0, -26.0], TILE);
    // Candle shelves climbing with the stair on both walls.
    for i in 0..6 {
        let x = 7.0 - i as f32 * 5.0;
        let y = 24.0 + (10.0 - x) / 28.0 * 12.0 + 1.6;
        for (z0, z1) in [(-37.0, -36.6), (-26.4, -26.0)] {
            g.deco("C4 candle shelf", [x - 1.5, y, z0], [x + 1.5, y + 0.2, z1], WAX_OLD);
            g.deco("C4 candles", [x - 1.2, y + 0.2, z0], [x + 1.2, y + 0.9, z1], WAX);
        }
    }
    g.light("C4 bottom", [14.0, 28.5, -31.5], L_CANDLE, 10.0);
    g.light("C4 middle", [-4.0, 34.5, -31.5], L_WICK, 14.0);
    g.light("C4 top", [-22.0, 40.5, -31.5], L_CANDLE, 10.0);

    // ---------------------------------------------------------------- A4 The Belfry
    g.section("A4 The Belfry (arena 4, finale: the great bell over the shaft)");
    let a4_in = op(N, -23.5, -18.5, 36.0, 41.0);
    let a4_out = op(S, -2.5, 2.5, 44.0, 49.0);
    // Arched openings to the night sky above the galleries (player-clipped).
    let arches = [
        op(W, -12.0, -4.0, 48.0, 58.0),
        op(W, 4.0, 12.0, 48.0, 58.0),
        op(E, -12.0, -4.0, 48.0, 58.0),
        op(E, 4.0, 12.0, 48.0, 58.0),
        op(N, -6.0, 6.0, 50.0, 58.0),
        op(S, -20.0, -12.0, 50.0, 58.0),
        op(S, 12.0, 20.0, 50.0, 58.0),
    ];
    let mut a4 = Room::new("A4", [-24.0, 36.0, -24.0], [24.0, 62.0, 24.0], STONE)
        .ceiling(TIMBER)
        .opening(a4_in)
        .opening(a4_out);
    for o in arches {
        a4 = a4.opening(o);
    }
    g.room(&a4);
    g.door(7, "door 7: Belfry entry (locked by encounter 4)", &a4, a4_in, true, DOOR);
    g.door(8, "door 8: Belfry exit to the Lantern (opened by encounter 4)", &a4, a4_out, false, EXIT_DOOR);
    for o in arches {
        let (min, max) = match o.side {
            N => ([o.lo, o.bottom, -25.0], [o.hi, o.top, -24.0]),
            S => ([o.lo, o.bottom, 24.0], [o.hi, o.top, 25.0]),
            W => ([-25.0, o.bottom, o.lo], [-24.0, o.top, o.hi]),
            E => ([24.0, o.bottom, o.lo], [25.0, o.top, o.hi]),
        };
        g.clip("A4 arch (open to the sky, player clip)", min, max);
        let (c, a) = ((o.lo + o.hi) * 0.5, o.top - 0.5);
        let (mmin, mmax) = match o.side {
            N | S => ([c - 0.2, o.bottom, min[2] + 0.3], [c + 0.2, a, max[2] - 0.3]),
            W | E => ([min[0] + 0.3, o.bottom, c - 0.2], [max[0] - 0.3, a, c + 0.2]),
        };
        g.deco("A4 arch mullion", mmin, mmax, IRON);
    }
    // Floor around the shaft's mouth (x -9..9, z -9..9).
    g.bx("A4 floor N", [-24.0, 35.0, -24.0], [24.0, 36.0, -9.0], TILE);
    g.bx("A4 floor S", [-24.0, 35.0, 9.0], [24.0, 36.0, 24.0], TILE);
    g.bx("A4 floor W", [-24.0, 35.0, -9.0], [-9.0, 36.0, 9.0], TILE);
    g.bx("A4 floor E", [9.0, 35.0, -9.0], [24.0, 36.0, 9.0], TILE);
    g.bx("A4 shaft edge N", [-9.0, 35.88, -9.0], [9.0, 36.0, -8.7], HAZARD);
    g.bx("A4 shaft edge S", [-9.0, 35.88, 8.7], [9.0, 36.0, 9.0], HAZARD);
    g.bx("A4 shaft edge W", [-9.0, 35.88, -8.7], [-8.7, 36.0, 8.7], HAZARD);
    g.bx("A4 shaft edge E", [8.7, 35.88, -8.7], [9.0, 36.0, 8.7], HAZARD);
    // Galleries (8 m up): an L along the east and south walls, ramps along the north and
    // west walls, so the floor, ramps and galleries form one loop around the shaft.
    g.bx("A4 gallery E", [16.0, 43.4, -24.0], [24.0, 44.0, 16.0], TIMBER);
    g.bx("A4 gallery S", [-24.0, 43.4, 16.0], [24.0, 44.0, 24.0], TIMBER);
    g.bx("A4 gallery fascia E", [15.7, 43.3, -24.0], [16.0, 44.1, 16.0], BRASS);
    g.bx("A4 gallery fascia S", [-24.0, 43.3, 15.7], [15.7, 44.1, 16.0], BRASS);
    g.ramp_x("A4 ramp N (to the east gallery)", (-24.0, -19.0), (-2.0, 36.0), (16.0, 44.0), 9.0, STONE);
    g.ramp_z("A4 ramp W (to the south gallery)", (-24.0, -19.0), (-2.0, 36.0), (16.0, 44.0), 9.0, STONE);
    for (x, z) in [(-8.0, 17.0), (8.0, 17.0), (17.0, -8.0), (17.0, 6.0)] {
        g.bx("A4 gallery column", [x - 0.7, 36.0, z - 0.7], [x + 0.7, 43.4, z + 0.7], WAX_OLD);
    }
    g.bx("A4 ingot stack", [-6.0, 36.0, -17.0], [-2.0, 37.2, -13.0], BRONZE);
    g.bx("A4 ingot stack", [-17.0, 36.0, 11.0], [-14.0, 37.2, 13.5], BRONZE);
    // Pads to the galleries: 5 m short of the overhang and a high arc, so a player steering
    // straight at the gallery still clears its edge.
    g.pad(
        "A4 floor -> east gallery",
        vol([9.5, 36.0, -1.25], [12.0, 37.0, 1.25]),
        pad_velocity([10.75, 36.0, 0.0], [20.0, 44.0, 0.0], 4.5),
    );
    g.bx("A4 pad ring E", [9.2, 36.0, -1.55], [12.3, 36.06, 1.55], HAZARD);
    g.pad(
        "A4 floor -> south gallery",
        vol([-12.25, 36.0, 9.5], [-9.75, 37.0, 12.0]),
        pad_velocity([-11.0, 36.0, 10.75], [-11.0, 44.0, 20.0], 4.5),
    );
    g.bx("A4 pad ring S", [-12.55, 36.0, 9.2], [-9.45, 36.06, 12.3], HAZARD);
    // The great bell over the shaft: its lip is a 1.5 m rim (a dash-jump from a gallery).
    // Stepped bell silhouette: lip, sound bow, waist, shoulder, crown.
    g.bx("A4 great bell lip", [-7.5, 45.4, -7.5], [7.5, 46.0, 7.5], BRASS);
    g.bx("A4 great bell sound bow", [-6.5, 46.0, -6.5], [6.5, 47.6, 6.5], BRONZE);
    g.bx("A4 great bell waist", [-5.8, 47.6, -5.8], [5.8, 52.4, 5.8], BRONZE);
    g.deco("A4 great bell waist band", [-6.0, 50.0, -6.0], [6.0, 50.8, 6.0], BRASS);
    g.bx("A4 great bell shoulder", [-4.8, 52.4, -4.8], [4.8, 55.0, 4.8], BRONZE);
    g.bx("A4 great bell crown", [-2.6, 55.0, -2.6], [2.6, 57.0, 2.6], BRASS);
    g.bx("A4 bell hanger", [-0.5, 57.0, -0.5], [0.5, 58.0, 0.5], IRON);
    g.bx("A4 bell yoke", [-1.5, 58.0, -24.0], [1.5, 60.0, 24.0], TIMBER);
    for z in [-12.0, 12.0] {
        g.deco("A4 roof truss", [-24.0, 61.2, z - 0.4], [24.0, 62.0, z + 0.4], TIMBER);
    }
    g.light("A4 entry", [-21.0, 40.0, -21.0], L_VERDIGRIS, 8.0);
    g.light("A4 shaft glow", [0.0, 30.0, 0.0], L_MOLTEN, 18.0);
    g.light("A4 bell", [0.0, 52.0, -10.0], L_WICK, 14.0);
    g.light("A4 bell S", [0.0, 52.0, 10.0], L_WICK, 14.0);
    g.light("A4 NW", [-17.0, 41.0, -17.0], L_CANDLE, 13.0);
    g.light("A4 NE", [17.0, 47.5, -17.0], L_CANDLE, 13.0);
    g.light("A4 SW", [-17.0, 47.5, 20.0], L_CANDLE, 13.0);
    g.light("A4 SE", [20.0, 47.5, 20.0], L_CANDLE, 13.0);
    g.light("A4 arches W", [-21.0, 54.0, 0.0], L_MOON, 14.0);
    g.light("A4 arches E", [21.0, 54.0, 0.0], L_MOON, 14.0);
    g.light("A4 exit", [0.0, 48.0, 21.0], L_VERDIGRIS, 8.0);
    g.checkpoint(
        "checkpoint 4: Belfry vestibule (north-west corner)",
        vol([-24.0, 35.0, -24.0], [-14.0, 42.0, -18.0]),
        [-21.0, 36.0, -21.5],
        -135.0,
    );
    g.encounter(
        "encounter 4: The Belfry — finale, three waves of everything around the shaft",
        &a4,
        Encounter {
            arena: None,
            rally: None,
            trigger: vol([-24.0, 35.0, -17.0], [24.0, 62.0, 24.0]),
            lock_doors: vec![7],
            unlock_doors: vec![8],
            waves: vec![
                wave(1.0, vec![
                    enemy("gaoler", [-14.0, 36.0, -3.0], -90.0),
                    enemy("gnawer", [14.0, 36.0, -4.0], 90.0),
                    enemy("gnawer", [14.0, 36.0, 4.0], 90.0),
                    enemy("gnawer", [-4.0, 36.0, 14.0], 180.0),
                    enemy("gnawer", [4.0, 36.0, 14.0], 180.0),
                ]),
                wave(1.5, vec![
                    enemy("heron", [20.0, 44.0, -12.0], 90.0),
                    enemy("heron", [-12.0, 44.0, 20.0], 180.0),
                    enemy("censer", [20.0, 44.0, 4.0], 90.0),
                    enemy("censer", [6.0, 44.0, 20.0], 180.0),
                    enemy("harrier", [-16.0, 42.0, -6.0], 180.0),
                    enemy("harrier", [10.0, 42.0, -16.0], 180.0),
                    enemy("harrier", [0.0, 42.0, 13.0], 180.0),
                    enemy("gnawer", [-16.0, 36.0, 4.0], -90.0),
                    enemy("gnawer", [12.0, 36.0, 12.0], 180.0),
                    enemy("gnawer", [2.0, 36.0, -14.0], 0.0),
                ]),
                wave(2.0, vec![
                    enemy("gaoler", [14.0, 36.0, -13.0], 90.0),
                    enemy("heron", [20.0, 44.0, 12.0], 90.0),
                    enemy("censer", [-18.0, 44.0, 20.0], 180.0),
                    enemy("censer", [20.0, 44.0, -13.0], 90.0),
                    enemy("harrier", [-15.0, 43.0, 0.0], 180.0),
                    enemy("harrier", [0.0, 43.0, -15.0], 180.0),
                    enemy("harrier", [14.0, 43.0, 2.0], 180.0),
                    enemy("gnawer", [-16.0, 36.0, -6.0], -90.0),
                    enemy("gnawer", [-16.0, 36.0, 6.0], -90.0),
                    enemy("gnawer", [-5.0, 36.0, 13.0], 180.0),
                    enemy("gnawer", [5.0, 36.0, 13.0], 180.0),
                ]),
            ],
        },
    );

    // ---------------------------------------------------------------- C6 + the Lantern (exit)
    g.section("C6 + the Lantern (exit at the top of the spire)");
    let c6 = Room::new("C6", [-3.0, 44.0, 25.0], [3.0, 51.0, 30.0], STONE)
        .floor(STONE)
        .ceiling(TIMBER)
        .bottom(43.0)
        .open(&[N, S]);
    g.room(&c6);
    let lantern_arches = [op(W, 35.0, 45.0, 50.0, 60.0), op(E, 35.0, 45.0, 50.0, 60.0), op(S, -5.0, 5.0, 50.0, 60.0)];
    let mut lantern = Room::new("Lantern", [-10.0, 44.0, 31.0], [10.0, 64.0, 49.0], WAX)
        .floor(STONE)
        .ceiling(TIMBER)
        .opening(op(N, -3.0, 3.0, 44.0, 51.0));
    for o in lantern_arches {
        lantern = lantern.opening(o);
    }
    g.room(&lantern);
    for o in lantern_arches {
        let (min, max) = match o.side {
            S => ([o.lo, o.bottom, 49.0], [o.hi, o.top, 50.0]),
            W => ([-11.0, o.bottom, o.lo], [-10.0, o.top, o.hi]),
            _ => ([10.0, o.bottom, o.lo], [11.0, o.top, o.hi]),
        };
        g.clip("Lantern arch (open to the sky, player clip)", min, max);
    }
    g.bx("Lantern dais", [-6.0, 44.0, 34.0], [6.0, 44.3, 41.0], PANEL);
    g.bx("Lantern great candle", [-2.0, 44.25, 42.0], [2.0, 53.0, 46.0], WAX);
    g.deco("Lantern wax pool", [-3.5, 44.0, 41.0], [3.5, 44.25, 47.5], WAX_OLD);
    g.bx("Lantern flame", [-0.8, 53.5, 43.2], [0.8, 56.5, 44.8], SLAG);
    for (x, z) in [(-8.0, 33.0), (8.0, 33.0), (-8.0, 47.0), (8.0, 47.0)] {
        g.bx("Lantern pillar", [x - 0.75, 44.0, z - 0.75], [x + 0.75, 64.0, z + 0.75], BRASS);
    }
    g.light("C6", [0.0, 49.0, 27.5], L_CANDLE, 8.0);
    g.light("Lantern flame", [0.0, 58.0, 44.0], L_FLAME, 22.0);
    g.light("Lantern dais", [0.0, 47.0, 37.5], L_CANDLE, 10.0);
    g.light("Lantern arches", [0.0, 56.0, 40.0], L_MOON, 16.0);
    g.def.exit = Some(vol([-6.0, 44.0, 34.0], [6.0, 52.0, 41.0]));

    g.kill("below the world", vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]));
    g
}
