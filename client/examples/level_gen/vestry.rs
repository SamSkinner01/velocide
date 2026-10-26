//! The Vestry: the lobby's warm-up hub (`LevelKind::Hub`, never a match level).
//!
//! A candlelit stone hall where Shrikes gather between sorties, 64 × 48 m under a beamed roof.
//! Everything is a short run from the spawn **nave** in the middle:
//!
//! ```text
//!            N (-Z)
//!   +---------------------+----------+---------------------+
//!   | FIRING RANGE (fixture: Effigy dummies, 3 depths)  | JUMP-PAD TOWER (5 / 10 / 15 m) |
//!   |  back platform y 3  | READY    |   three tiers, a pad on each           |
//!   +---- partition ------+  BOARD   +-----------------------------------------+
//!   | SLIDE TUNNEL (1.2 m)|  NAVE    |  CONSOLE  (USE: practice pack)          |
//!   | WALL-JUMP SHAFT     | spawns + |  PIT (pen, floor y -4, escape pad)      |
//!   | SLAM-BOUNCE plate + | brazier  |                                         |
//!   |   5 m ledge         |          |                                         |
//!   | DASH GAPS along the south wall at y 6 (12 m and 11 m)                    |
//!   +--------------------------------------------------------------------------+
//!            S (+Z)
//! ```
//!
//! Hub markup (read by `shared::sim::warmup` and the client's `ingame::hub`):
//! - encounter 0, a **fixture** (trigger without volume): the firing range's Effigy dummies,
//!   always standing (skipped by servers that don't know `effigy`);
//! - encounter 1, a **console** (trigger = where you stand to press USE, arena = the pit):
//!   each press summons the next of five practice packs into the pit;
//! - props: the ready board on the nave's screen wall and the console at the pit's rim.

use shared::level::Prop;

use super::*;

mod mat {
    use super::Mat;
    pub const WALL: Mat = Mat("brick", [0.72, 0.58, 0.52]);
    pub const WALL_DARK: Mat = Mat("brick", [0.54, 0.44, 0.44]);
    pub const PILASTER: Mat = Mat("waxstone", [0.92, 0.86, 0.72]);
    pub const STONE: Mat = Mat("concrete", [0.86, 0.8, 0.74]);
    pub const STONE_DARK: Mat = Mat("concrete", [0.62, 0.57, 0.56]);
    pub const FLOOR: Mat = Mat("tile", [0.8, 0.7, 0.64]);
    pub const DAIS: Mat = Mat("tile", [0.94, 0.82, 0.68]);
    pub const PIT_FLOOR: Mat = Mat("dirt", [0.72, 0.6, 0.52]);
    pub const WOOD: Mat = Mat("wood", [0.76, 0.6, 0.48]);
    pub const WOOD_DARK: Mat = Mat("wood", [0.48, 0.38, 0.34]);
    pub const BRASS: Mat = Mat("metal", [1.0, 0.8, 0.46]);
    pub const IRON: Mat = Mat("metal", [0.46, 0.44, 0.48]);
    pub const WAX: Mat = Mat("wax", [1.0, 0.86, 0.62]);
    pub const HAZARD: Mat = Mat("hazard", [1.0, 0.82, 0.4]);
    pub const GRATE: Mat = Mat("grate", [0.86, 0.72, 0.56]);
    /// Coals and glowing seams (the `lava` material; nowhere to stand in it).
    pub const EMBER: Mat = Mat("lava", [1.0, 0.9, 0.62]);
    /// Crimson banners (wax-stiffened cloth).
    pub const BANNER: Mat = Mat("wax", [0.82, 0.26, 0.22]);
}

const L_CANDLE: [f32; 3] = [1.25, 0.86, 0.5];
const L_BRAZIER: [f32; 3] = [1.3, 0.6, 0.22];
const L_RANGE: [f32; 3] = [1.0, 0.95, 0.85];
const L_PIT: [f32; 3] = [1.3, 0.42, 0.16];

/// Hall interior: x -32..32, z -24..24, walls up to the roof at 18.
const X0: f32 = -32.0;
const X1: f32 = 32.0;
const Z0: f32 = -24.0;
const Z1: f32 = 24.0;
const ROOF: f32 = 18.0;

/// The practice pit (pen): interior and floor height.
const PIT_X: (f32, f32) = (12.0, 30.0);
const PIT_Z: (f32, f32) = (6.0, 22.0);
const PIT_Y: f32 = -4.0;

/// Effigies face the firing line (east, +X).
const FACE_EAST: f32 = -90.0;

pub fn vestry() -> Gen {
    let mut g = Gen::new(
        "The Vestry",
        LevelKind::Hub,
        Environment {
            sky_color: Vec3::new(0.08, 0.05, 0.06),
            fog_color: Vec3::new(0.16, 0.1, 0.09),
            fog_start: 30.0,
            fog_end: 110.0,
            ambient: Vec3::new(0.5, 0.44, 0.42),
            light_dir: Vec3::new(-0.4, -0.85, 0.3),
            light_color: Vec3::new(0.55, 0.42, 0.34),
            music: "campaign".into(),
        },
    );
    shell(&mut g);
    nave(&mut g);
    range(&mut g);
    course(&mut g);
    tower(&mut g);
    pit(&mut g);
    g.kill(
        "below the world",
        vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]),
    );
    g
}

fn shell(g: &mut Gen) {
    use mat::*;
    g.section("Hall: floor (with the pit's hole), walls, roof");
    let (px0, px1) = PIT_X;
    let (pz0, pz1) = PIT_Z;
    g.bx(
        "floor north half",
        [X0, -1.0, Z0],
        [X1, 0.0, pz0 - 1.0],
        FLOOR,
    );
    g.bx(
        "floor south-west",
        [X0, -1.0, pz0 - 1.0],
        [px0 - 1.0, 0.0, Z1],
        FLOOR,
    );
    g.bx(
        "floor east strip",
        [px1 + 1.0, -1.0, pz0 - 1.0],
        [X1, 0.0, Z1],
        FLOOR,
    );
    g.bx(
        "floor south strip",
        [px0 - 1.0, -1.0, pz1 + 1.0],
        [px1 + 1.0, 0.0, Z1],
        FLOOR,
    );
    let hall = Room::new("hall", [X0, 0.0, Z0], [X1, ROOF, Z1], WALL).ceiling(WOOD_DARK);
    g.room(&hall);
    // Wax-stone pilasters on the long walls and roof beams across the hall.
    for x in [-24.0, -8.0, 8.0, 24.0] {
        g.bx(
            "north pilaster",
            [x - 0.7, 0.0, Z0],
            [x + 0.7, ROOF, Z0 + 0.6],
            PILASTER,
        );
    }
    // (South: clear of the dash course's blocks.)
    for x in [-18.0, -4.0, 16.0, 26.0] {
        g.bx(
            "south pilaster",
            [x - 0.7, 0.0, Z1 - 0.6],
            [x + 0.7, ROOF, Z1],
            PILASTER,
        );
    }
    for z in [-16.0, -8.0, 0.0, 8.0, 16.0] {
        g.deco(
            "roof beam",
            [X0, ROOF - 1.1, z - 0.35],
            [X1, ROOF - 0.05, z + 0.35],
            WOOD,
        );
    }
    // Wax weeping down the end walls high up.
    for (note, min, max) in [
        ("wax band west", [X0, 12.0, Z0], [X0 + 0.15, 13.0, Z1]),
        ("wax band east", [X1 - 0.15, 12.0, Z0], [X1, 13.0, Z1]),
    ] {
        g.deco(note, min, max, WAX);
    }
    for (note, min, max) in [
        // On the range's back platform.
        (
            "hall corner brazier NW",
            [X0 + 1.0, 3.0, Z0 + 1.0],
            [X0 + 2.4, 4.0, Z0 + 2.4],
        ),
        (
            "hall corner brazier SE",
            [X1 - 2.4, 0.0, Z1 - 2.4],
            [X1 - 1.0, 1.0, Z1 - 1.0],
        ),
    ] {
        g.bx(note, min, max, IRON);
        let [x0, y0, z0] = min;
        let [x1, y1, z1] = max;
        g.deco(
            "  coals",
            [x0 + 0.2, y1, z0 + 0.2],
            [x1 - 0.2, y1 + 0.25, z1 - 0.2],
            EMBER,
        );
        g.light(
            note,
            [(x0 + x1) * 0.5, y0 + 2.5, (z0 + z1) * 0.5],
            L_BRAZIER,
            16.0,
        );
    }
    for (x, z) in [(-16.0, 0.0), (0.0, -16.0), (16.0, 0.0), (0.0, 16.0)] {
        g.light("roof lantern", [x, ROOF - 2.0, z], L_CANDLE, 26.0);
    }
    // Crimson banners on the long walls, each on a brass rod with a brass boss.
    for (x, z, dir) in [
        (-16.0, Z0, 1.0),
        (0.0, Z0, 1.0),
        (16.0, Z0, 1.0),
        (-11.0, Z1, -1.0),
        (6.0, Z1, -1.0),
        (21.0, Z1, -1.0),
    ] {
        let at = |d0: f32, d1: f32| {
            let (a, b) = (z + dir * d0, z + dir * d1);
            (a.min(b), a.max(b))
        };
        let (z0, z1) = at(0.05, 0.15);
        g.deco("banner", [x - 1.2, 7.0, z0], [x + 1.2, 14.0, z1], BANNER);
        let (z0, z1) = at(0.02, 0.32);
        g.deco("  rod", [x - 1.5, 14.0, z0], [x + 1.5, 14.2, z1], BRASS);
        let (z0, z1) = at(0.15, 0.21);
        g.deco("  boss", [x - 0.45, 10.0, z0], [x + 0.45, 10.9, z1], BRASS);
    }
}

/// The spawn nave: a low dais facing the ready board on its screen wall, two braziers by it.
fn nave(g: &mut Gen) {
    use mat::*;
    g.section("Nave: spawn dais, ready-board screen, braziers");
    g.bx("dais", [-7.0, 0.0, -5.0], [7.0, 0.2, 9.0], DAIS);
    // Two rows of four spawns, all facing the board (north).
    for z in [0.5, 4.5] {
        for x in [-5.25, -1.75, 1.75, 5.25] {
            g.spawn([x, 0.2, z], 0.0);
        }
    }
    // The screen wall the ready board hangs on (the board itself is a prop).
    g.bx(
        "board screen",
        [-6.0, 0.5, -8.0],
        [6.0, 5.4, -7.0],
        WALL_DARK,
    );
    g.bx(
        "board screen cap",
        [-6.3, 5.4, -8.2],
        [6.3, 5.8, -6.8],
        STONE,
    );
    g.bx(
        "board screen plinth",
        [-6.3, 0.0, -8.2],
        [6.3, 0.5, -6.8],
        STONE_DARK,
    );
    g.light("ready board lamp", [0.0, 5.6, -4.8], L_CANDLE, 10.0);
    g.def.props.push(Prop {
        model: "models/prop_ready_board.gltf".into(),
        position: Vec3::new(0.0, 0.6, -6.97),
        rotation: Vec3::new(0.0, 180.0, 0.0),
        scale: 1.25,
    });
    for (note, x0) in [("brazier west", -7.9), ("brazier east", 6.7)] {
        let x1 = x0 + 1.2;
        g.bx(note, [x0, 0.0, -8.0], [x1, 1.0, -6.8], IRON);
        g.deco(
            "  coals",
            [x0 + 0.15, 1.0, -7.85],
            [x1 - 0.15, 1.3, -6.95],
            EMBER,
        );
        for (x, z) in [(x0, -8.0), (x1, -8.0), (x0, -6.8), (x1, -6.8)] {
            g.deco(
                "  post",
                [x - 0.07, 1.0, z - 0.07],
                [x + 0.07, 1.7, z + 0.07],
                BRASS,
            );
        }
        g.light(note, [(x0 + x1) * 0.5, 2.6, -7.4], L_BRAZIER, 12.0);
    }
    // Candle clusters on the dais corners.
    for (x, z) in [(-6.4, 8.4), (6.4, 8.4), (-6.4, -4.4), (6.4, -4.4)] {
        g.bx(
            "candle cluster",
            [x - 0.3, 0.2, z - 0.3],
            [x + 0.3, 0.9, z + 0.3],
            WAX,
        );
    }
}

/// The firing range (north-west): a counter, lane walls, Effigy dummies at three depths.
fn range(g: &mut Gen) {
    use mat::*;
    g.section("Firing range (north-west)");
    g.bx(
        "range partition",
        [X0, 0.0, -9.0],
        [-14.0, 4.0, -8.0],
        WALL_DARK,
    );
    g.bx(
        "range partition cap",
        [X0, 4.0, -9.2],
        [-14.0, 4.3, -7.8],
        STONE,
    );
    g.bx(
        "firing counter",
        [-14.0, 0.0, -22.0],
        [-13.2, 1.1, -10.0],
        WOOD,
    );
    g.deco(
        "counter top",
        [-14.1, 1.1, -22.1],
        [-13.1, 1.2, -9.9],
        WOOD_DARK,
    );
    for z in [-17.8, -13.4] {
        g.bx(
            "lane wall",
            [-26.0, 0.0, z - 0.2],
            [-15.0, 1.3, z + 0.2],
            STONE_DARK,
        );
    }
    g.bx("back platform", [X0, 0.0, Z0], [-27.0, 3.0, -9.0], STONE);
    g.deco(
        "back platform lip",
        [-27.0, 2.8, Z0],
        [-26.85, 3.0, -9.0],
        HAZARD,
    );
    for (x, z) in [
        (-19.0, -20.0),
        (-19.0, -11.2),
        (-29.5, -20.0),
        (-29.5, -11.2),
    ] {
        let y = if x < -27.0 { 3.0 } else { 0.0 };
        g.light("target lamp", [x + 1.5, y + 4.0, z], L_RANGE, 8.0);
    }
    g.light("firing line lamp", [-11.0, 4.0, -16.0], L_CANDLE, 10.0);
    let dummies = vec![
        enemy("effigy", [-19.0, 0.0, -20.0], FACE_EAST),
        enemy("effigy", [-19.0, 0.0, -11.2], FACE_EAST),
        enemy("effigy", [-23.5, 0.0, -15.6], FACE_EAST),
        enemy("effigy", [-29.5, 3.0, -20.0], FACE_EAST),
        enemy("effigy", [-29.5, 3.0, -15.6], FACE_EAST),
        enemy("effigy", [-29.5, 3.0, -11.2], FACE_EAST),
    ];
    // A fixture: the trigger has no volume (always standing, see the module docs).
    let at = [-22.0, 0.0, -16.0];
    g.def.encounters.push(Encounter {
        trigger: vol(at, at),
        arena: None,
        rally: None,
        lock_doors: vec![],
        unlock_doors: vec![],
        waves: vec![wave(0.0, dummies)],
    });
    g.encounter_notes
        .push("firing range (fixture): Effigy dummies at 6, 10 and 16 m".into());
}

/// The movement course (south-west): slide tunnel, wall-jump shaft, slam-bounce ledge, and the
/// dash gaps along the south wall.
fn course(g: &mut Gen) {
    use mat::*;
    g.section("Course: slide tunnel (1.2 m high, slide under it)");
    g.bx(
        "tunnel wall north",
        [-26.0, 0.0, -7.5],
        [-14.0, 1.2, -6.5],
        STONE_DARK,
    );
    g.bx(
        "tunnel wall south",
        [-26.0, 0.0, -3.5],
        [-14.0, 1.2, -2.5],
        STONE_DARK,
    );
    g.bx("tunnel roof", [-26.0, 1.2, -7.5], [-14.0, 2.6, -2.5], WALL);
    for (note, x0, x1) in [
        ("tunnel mouth stripes", -14.0, -13.9),
        ("tunnel exit stripes", -26.1, -26.0),
    ] {
        g.deco(note, [x0, 1.2, -7.5], [x1, 2.6, -2.5], HAZARD);
    }
    g.light("tunnel lamp", [-20.0, 3.4, -5.0], L_CANDLE, 8.0);

    g.section("Course: wall-jump shaft (3.5 m wide, 9 m to the top)");
    g.bx(
        "shaft tower",
        [-28.5, 0.0, 4.0],
        [-23.0, 9.0, 12.0],
        WALL_DARK,
    );
    g.bx(
        "shaft tower cap",
        [-28.7, 9.0, 3.8],
        [-22.8, 9.3, 12.2],
        STONE,
    );
    g.deco(
        "shaft floor stripes",
        [X0, 0.0, 5.0],
        [-28.5, 0.05, 11.0],
        HAZARD,
    );
    g.light("shaft top lamp", [-25.75, 11.0, 8.0], L_CANDLE, 9.0);

    g.section("Course: slam-bounce plate and the 5 m ledge");
    g.deco("slam plate", [-20.0, 0.0, 8.0], [-16.0, 0.06, 12.0], HAZARD);
    g.bx("bounce ledge", [-16.0, 0.0, 6.0], [-12.0, 5.0, 14.0], STONE);
    g.bx(
        "bounce ledge cap",
        [-16.2, 5.0, 5.8],
        [-11.8, 5.3, 14.2],
        BRASS,
    );
    g.light("bounce ledge lamp", [-14.0, 7.0, 10.0], L_CANDLE, 9.0);

    g.section("Course: dash gaps along the south wall (y 6: 12 m and 11 m)");
    g.bx("dash start", [X0, 0.0, 18.0], [-24.0, 6.0, Z1], WALL);
    g.bx("dash start cap", [X0, 6.0, 17.8], [-23.8, 6.3, Z1], STONE);
    for (note, x0, x1) in [("dash pillar 1", -12.0, -10.0), ("dash pillar 2", 1.0, 3.0)] {
        g.bx(note, [x0, 0.0, 19.0], [x1, 6.0, 23.0], WALL_DARK);
        g.bx("  cap", [x0 - 0.2, 6.0, 18.8], [x1 + 0.2, 6.3, 23.2], STONE);
        g.light(note, [(x0 + x1) * 0.5, 8.5, 21.0], L_CANDLE, 8.0);
    }
    g.bx("dash end", [8.0, 0.0, 17.0], [11.0, 6.0, Z1], WALL);
    g.bx("dash end cap", [7.8, 6.0, 16.8], [11.0, 6.3, Z1], STONE);
    // A ramp up to the start (from the shaft tower's top you can also drop onto it).
    g.ramp_x(
        "dash start ramp",
        (18.6, 23.4),
        (-14.0, 0.0),
        (-24.0, 6.0),
        3.0,
        STONE_DARK,
    );
}

/// The jump-pad tower (north-east): three tiers at 5, 10 and 15 m, a pad on each.
fn tower(g: &mut Gen) {
    use mat::*;
    g.section("Jump-pad tower (north-east)");
    g.bx("tier 1", [17.0, 0.0, -23.0], [31.0, 5.0, -9.0], WALL);
    g.bx("tier 2", [19.5, 5.0, -21.0], [28.5, 10.0, -12.0], WALL_DARK);
    g.bx("tier 3", [22.0, 10.0, -19.0], [26.0, 15.0, -15.0], STONE);
    g.deco(
        "tier 1 coping",
        [17.0, 5.0, -9.3],
        [31.0, 5.25, -9.0],
        BRASS,
    );
    g.deco(
        "tier 2 coping",
        [19.5, 10.0, -12.3],
        [28.5, 10.25, -12.0],
        BRASS,
    );
    g.deco(
        "crown brazier",
        [23.2, 15.0, -17.8],
        [24.8, 15.5, -16.2],
        EMBER,
    );
    g.light("tower crown brazier", [24.0, 17.0, -17.0], L_BRAZIER, 18.0);
    g.light("tower foot lamp", [20.0, 3.0, -6.0], L_CANDLE, 10.0);
    // Each pad throws you up onto the next tier (6 m rise, a few metres north).
    // They are staggered so no pad lands you on the next one.
    g.pad(
        "floor -> tier 1",
        vol([18.8, 0.0, -7.7], [21.2, 1.0, -5.3]),
        [0.0, 25.0, -4.0],
    );
    g.pad(
        "tier 1 -> tier 2",
        vol([25.8, 5.0, -11.6], [28.2, 6.0, -9.4]),
        [0.0, 25.0, -4.0],
    );
    g.pad(
        "tier 2 -> crown",
        vol([23.0, 10.0, -14.4], [25.0, 11.0, -12.6]),
        [0.0, 25.0, -4.0],
    );
}

/// The practice pit (south-east): a sunken pen; the console on its north rim summons a pack.
fn pit(g: &mut Gen) {
    use mat::*;
    g.section("Practice pit (pen) and its console");
    let (x0, x1) = PIT_X;
    let (z0, z1) = PIT_Z;
    g.bx(
        "pit floor",
        [x0, PIT_Y - 1.0, z0],
        [x1, PIT_Y, z1],
        PIT_FLOOR,
    );
    g.bx(
        "pit rim north",
        [x0 - 1.0, PIT_Y - 1.0, z0 - 1.0],
        [x1 + 1.0, 0.0, z0],
        STONE,
    );
    g.bx(
        "pit rim south",
        [x0 - 1.0, PIT_Y - 1.0, z1],
        [x1 + 1.0, 0.0, z1 + 1.0],
        STONE,
    );
    g.bx(
        "pit rim west",
        [x0 - 1.0, PIT_Y - 1.0, z0],
        [x0, 0.0, z1],
        STONE,
    );
    g.bx(
        "pit rim east",
        [x1, PIT_Y - 1.0, z0],
        [x1 + 1.0, 0.0, z1],
        STONE,
    );
    // On the console side only a kerb you can step onto: the pack comes for whoever summoned
    // it and gathers under this wall, so you have to be able to look straight down at it.
    g.bx(
        "kerb north",
        [x0 - 1.0, 0.0, z0 - 1.0],
        [x1 + 1.0, 0.3, z0],
        STONE_DARK,
    );
    g.bx(
        "parapet west",
        [x0 - 1.0, 0.0, z0],
        [x0, 1.0, z1],
        STONE_DARK,
    );
    g.deco(
        "kerb edge",
        [x0 - 1.0, 0.3, z0 - 0.25],
        [x1 + 1.0, 0.36, z0 - 0.05],
        BRASS,
    );
    // Glowing seams low in the pit walls, a grate over a fire pit in the corner.
    for (note, min, max) in [
        ("pit seam south", [x0, -3.2, z1 - 0.1], [x1, -3.0, z1]),
        ("pit seam east", [x1 - 0.1, -3.2, z0], [x1, -3.0, z1]),
    ] {
        g.deco(note, min, max, EMBER);
    }
    g.deco(
        "pit grate",
        [27.0, PIT_Y, 19.0],
        [29.5, PIT_Y + 0.05, 21.5],
        GRATE,
    );
    g.light("pit glow", [21.0, -1.5, 14.0], L_PIT, 14.0);
    g.light("pit corner glow", [28.0, -2.0, 20.0], L_PIT, 9.0);
    g.light("console lamp", [24.0, 3.5, 3.5], L_CANDLE, 9.0);
    // Out of the pit: a pad by the north-west corner throws you back onto the rim.
    g.pad(
        "pit -> rim",
        vol([13.0, PIT_Y, 6.6], [15.4, PIT_Y + 1.0, 9.0]),
        [0.0, 24.0, -5.0],
    );
    // The console: a lectern by the rim facing the nave (west), so its sign never blocks the
    // view into the pit (an invisible clip for its body; the model is a prop).
    g.clip("console body", [24.6, 0.0, 3.0], [25.4, 1.3, 4.0]);
    g.def.props.push(Prop {
        model: "models/prop_console.gltf".into(),
        position: Vec3::new(25.0, 0.0, 3.5),
        rotation: Vec3::new(0.0, 90.0, 0.0),
        scale: 1.0,
    });
    let pen = vol([x0, PIT_Y - 1.0, z0], [x1, 6.0, z1]);
    let y = PIT_Y;
    let packs = vec![
        wave(
            0.0,
            vec![
                enemy("gnawer", [17.0, y, 15.0], 0.0),
                enemy("gnawer", [21.0, y, 17.0], 0.0),
                enemy("gnawer", [25.0, y, 15.0], 0.0),
                enemy("gnawer", [21.0, y, 12.5], 0.0),
            ],
        ),
        wave(
            0.0,
            vec![
                enemy("censer", [16.0, y, 19.5], 0.0),
                enemy("censer", [26.0, y, 19.5], 0.0),
                enemy("gnawer", [19.0, y, 14.0], 0.0),
                enemy("gnawer", [23.0, y, 14.0], 0.0),
            ],
        ),
        wave(0.0, vec![enemy("gaoler", [21.0, y, 16.0], 0.0)]),
        wave(
            0.0,
            vec![
                enemy("heron", [21.0, y, 19.5], 0.0),
                enemy("gnawer", [17.0, y, 13.0], 0.0),
                enemy("gnawer", [25.0, y, 13.0], 0.0),
            ],
        ),
        wave(
            0.0,
            vec![
                enemy("gaoler", [21.0, y, 17.0], 0.0),
                enemy("censer", [15.5, y, 20.0], 0.0),
                enemy("gnawer", [18.0, y, 12.0], 0.0),
                enemy("gnawer", [24.0, y, 12.0], 0.0),
            ],
        ),
    ];
    g.def.encounters.push(Encounter {
        trigger: vol([22.4, -0.5, 1.8], [24.6, 3.0, 5.0]),
        arena: Some(pen),
        rally: None,
        lock_doors: vec![],
        unlock_doors: vec![],
        waves: packs,
    });
    g.encounter_notes.push(
        "practice console: USE summons the next pack into the pit (gnawers, censers, gaoler, heron, mixed)"
            .into(),
    );
}
