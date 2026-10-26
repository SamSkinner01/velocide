//! Tallow Works: a mid-size, vertical PvP arena (2–8 players) and grind arena, indoors.
//!
//! The Foundry's rendering house: a 28 m brick hall thick with yellow steam, where tallow is
//! rendered in two vats and run up conveyor belts to the press lofts. Four layers: the floor
//! (y 0) round the vats and the central boiler; the mezzanines along the long walls (y 6); the
//! skimmer platforms hanging over the vats on hoist chains (y 8) and the boiler's top (y 9);
//! the press lofts at both ends (y 12), reached by the belts. Point-symmetric (180°): the north
//! half is described once and turned. Layout map, routes and spawns: `docs/levels.md`.

use super::gates::{Gate, GateStyle, Spawns, pad_velocity};
use super::*;

mod mat {
    use super::Mat;
    /// Sooty brick and limewash stained with grease.
    pub const BRICK: Mat = Mat("brick", [0.56, 0.48, 0.4]);
    pub const LIMEWASH: Mat = Mat("concrete", [0.84, 0.78, 0.6]);
    pub const FLOOR: Mat = Mat("tile", [0.66, 0.6, 0.46]);
    pub const IRON: Mat = Mat("metal", [0.46, 0.44, 0.4]);
    pub const IRON_DARK: Mat = Mat("metal", [0.3, 0.29, 0.27]);
    pub const TIMBER: Mat = Mat("wood", [0.62, 0.52, 0.4]);
    /// Rubber belts.
    pub const BELT: Mat = Mat("panel", [0.3, 0.28, 0.26]);
    /// Blocks of rendered tallow.
    pub const TALLOW: Mat = Mat("wax", [1.0, 0.9, 0.6]);
    pub const TALLOW_OLD: Mat = Mat("wax", [0.84, 0.72, 0.46]);
    pub const GRATE: Mat = Mat("grate", [0.72, 0.68, 0.56]);
    pub const HAZARD: Mat = Mat("hazard", [1.0, 0.86, 0.45]);
    /// Molten tallow in the vats (always has a kill volume) and the boiler's fire (behind
    /// grates).
    pub const MOLTEN: Mat = Mat("lava", [1.0, 0.92, 0.58]);
    pub const FIRE: Mat = Mat("lava", [1.0, 0.7, 0.45]);
}

/// Sickly gas lamps, molten tallow, the boiler's fire, grey daylight through the steam.
const L_GAS: [f32; 3] = [1.05, 1.02, 0.55];
const L_VAT: [f32; 3] = [1.3, 0.9, 0.38];
const L_FIRE: [f32; 3] = [1.3, 0.55, 0.2];
const L_DAY: [f32; 3] = [0.62, 0.64, 0.66];

/// Interior half size (64 × 52 m), the roof, the clip lid over it.
const HX: f32 = 32.0;
const HZ: f32 = 26.0;
const ROOF: f32 = 28.0;
const LID: f32 = 29.0;
/// Heights: mezzanines, skimmers, the boiler's top, the lofts.
const MEZZ: f32 = 6.0;
const SKIM: f32 = 8.0;
const BOIL: f32 = 9.0;
const LOFT: f32 = 12.0;
/// The west vat (x, z) — the east one is turned 180° — and the west press house.
const VAT: (f32, f32, f32, f32) = (-19.0, -9.0, -12.0, -4.0);
const PRESS: (f32, f32, f32, f32) = (-HX, -24.0, -15.5, 15.5);

/// One half of the point symmetry: `s` = 1 is the north half as written, -1 turns it 180°.
struct Half<'a> {
    g: &'a mut Gen,
    s: f32,
    name: &'static str,
}

impl Half<'_> {
    fn note(&self, what: &str) -> String {
        format!("{} {what}", self.name)
    }

    fn corners(&self, min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let s = self.s;
        let (ax, bx) = (s * min[0], s * max[0]);
        let (az, bz) = (s * min[2], s * max[2]);
        (
            [ax.min(bx), min[1], az.min(bz)],
            [ax.max(bx), max[1], az.max(bz)],
        )
    }

    fn bx(&mut self, what: &str, min: [f32; 3], max: [f32; 3], m: Mat) -> &mut Brush {
        let (a, b) = self.corners(min, max);
        let note = self.note(what);
        self.g.bx(&note, a, b, m)
    }

    fn deco(&mut self, what: &str, min: [f32; 3], max: [f32; 3], m: Mat) {
        self.bx(what, min, max, m).no_collide = true;
    }

    /// Ramp along x from the low end `a` to the high end `b` (`(x, y)`), spanning `z`.
    fn ramp_x(
        &mut self,
        what: &str,
        (z0, z1): (f32, f32),
        a: (f32, f32),
        b: (f32, f32),
        thick: f32,
        m: Mat,
    ) {
        let s = self.s;
        let note = self.note(what);
        let z = ((s * z0).min(s * z1), (s * z0).max(s * z1));
        self.g
            .ramp_x(&note, z, (s * a.0, a.1), (s * b.0, b.1), thick, m);
    }

    /// A jump pad on the floor at `from` throwing onto `to`, arcing `over` above it.
    fn pad(&mut self, what: &str, from: [f32; 3], to: [f32; 3], over: f32) {
        let s = self.s;
        let (fx, fz) = (s * from[0], s * from[2]);
        let v = pad_velocity([fx, from[1], fz], [s * to[0], to[1], s * to[2]], over);
        let note = self.note(what);
        self.g.pad(
            &note,
            vol(
                [fx - 1.2, from[1], fz - 1.2],
                [fx + 1.2, from[1] + 1.0, fz + 1.2],
            ),
            v,
        );
    }

    fn light(&mut self, what: &str, [x, y, z]: [f32; 3], color: [f32; 3], radius: f32) {
        let note = self.note(what);
        self.g
            .light(&note, [self.s * x, y, self.s * z], color, radius);
    }

    /// A player spawn facing the middle of the hall.
    fn spawn(&mut self, [x, y, z]: [f32; 3]) {
        let (x, z) = (self.s * x, self.s * z);
        self.g.spawn([x, y, z], r4(f32::atan2(x, z).to_degrees()));
    }

    /// A stack of tallow blocks (cover) standing at `y`.
    fn stack(&mut self, what: &str, [x, y, z]: [f32; 3], (w, d, h): (f32, f32, f32)) {
        self.bx(what, [x - w, y, z - d], [x + w, y + h, z + d], mat::TALLOW);
        self.bx(
            &format!("{what} (top block)"),
            [x - w * 0.6, y + h, z - d * 0.6],
            [x + w * 0.6, y + h + 0.6, z + d * 0.6],
            mat::TALLOW_OLD,
        );
    }
}

pub(super) fn tallow_works() -> Gen {
    use mat::*;
    let mut g = Gen::new(
        "Tallow Works",
        LevelKind::Arena,
        Environment {
            sky_color: Vec3::new(0.2, 0.17, 0.12),
            fog_color: Vec3::new(0.3, 0.26, 0.17),
            fog_start: 18.0,
            fog_end: 95.0,
            ambient: Vec3::new(0.56, 0.53, 0.43),
            light_dir: Vec3::new(0.2, -1.0, 0.3),
            light_color: Vec3::new(0.5, 0.48, 0.4),
            music: "arena".into(),
        },
    );

    // Mould gates (grind spawns): one in each wall, under the mezzanines on the long walls,
    // beside the press houses on the short ones.
    let mould_gates = [
        ("N gate", Gate::new(Side::N, -HZ, -6.0, 0.0)),
        ("S gate", Gate::new(Side::S, HZ, 6.0, 0.0)),
        ("W gate", Gate::new(Side::W, -HX, 20.5, 0.0)),
        ("E gate", Gate::new(Side::E, HX, -20.5, 0.0)),
    ];
    g.section("The hall (roofed, 28 m; a clip lid on the roof)");
    let mut hall = Room::new("Hall", [-HX, 0.0, -HZ], [HX, ROOF, HZ], BRICK)
        .ceiling(IRON_DARK)
        .bottom(-3.0);
    for (_, gate) in &mould_gates {
        hall = hall.opening(gate.opening());
    }
    g.room(&hall);
    g.clip(
        "clip lid (over the roof and the gate chambers)",
        [-HX - 16.0, LID, -HZ - 16.0],
        [HX + 16.0, LID + 1.0, HZ + 16.0],
    );
    let gate_style = GateStyle {
        wall: BRICK,
        floor: FLOOR,
        trim: IRON,
        grate: GRATE,
        glow: MOLTEN,
        light: L_VAT,
    };
    for (name, gate) in &mould_gates {
        gate.build(&mut g, name, &gate_style, Spawns::All);
    }
    // Roof trusses and the hoist rails the skimmers hang from.
    for x in [-20.0, -8.0, 8.0, 20.0] {
        g.bx(
            "roof truss",
            [x - 0.4, ROOF - 1.6, -HZ],
            [x + 0.4, ROOF, HZ],
            IRON_DARK,
        );
    }

    g.section("Centre: the boiler and its flue");
    g.bx(
        "floor (middle band)",
        [-HX, -3.0, -4.0],
        [HX, 0.0, 4.0],
        FLOOR,
    );
    g.bx("boiler", [-5.0, 0.0, -5.0], [5.0, BOIL, 5.0], BRICK);
    g.bx("boiler band", [-5.2, 3.0, -5.2], [5.2, 3.6, 5.2], IRON);
    g.bx(
        "boiler rim",
        [-5.2, BOIL - 0.4, -5.2],
        [5.2, BOIL + 0.05, 5.2],
        IRON,
    );
    g.bx(
        "boiler flue",
        [-1.5, BOIL, -1.5],
        [1.5, ROOF, 1.5],
        IRON_DARK,
    );
    g.bx(
        "boiler flue collar",
        [-1.8, BOIL + 0.05, -1.8],
        [1.8, BOIL + 1.2, 1.8],
        IRON,
    );
    // Fireboxes on all four faces: the fire behind grates.
    for (x0, x1, z0, z1) in [
        (-1.5, 1.5, -5.12, -5.02),
        (-1.5, 1.5, 5.02, 5.12),
        (-5.12, -5.02, -1.5, 1.5),
        (5.02, 5.12, -1.5, 1.5),
    ] {
        g.deco("boiler firebox", [x0, 0.4, z0], [x1, 2.4, z1], FIRE);
    }
    for (x0, x1, z0, z1) in [
        (-1.7, 1.7, -5.35, -5.12),
        (-1.7, 1.7, 5.12, 5.35),
        (-5.35, -5.12, -1.7, 1.7),
        (5.12, 5.35, -1.7, 1.7),
    ] {
        g.bx("boiler firebox grate", [x0, 0.2, z0], [x1, 2.6, z1], GRATE);
    }
    for (x, z) in [(0.0, -6.5), (0.0, 6.5), (-6.5, 0.0), (6.5, 0.0)] {
        g.light("boiler firebox", [x, 1.5, z], L_FIRE, 8.0);
    }
    g.light("boiler top", [0.0, BOIL + 4.0, 0.0], L_FIRE, 12.0);

    for (s, name) in [(1.0f32, "N"), (-1.0, "S")] {
        let mut h = Half { g: &mut g, s, name };
        h.g.section(&format!(
            "{name} half: vat, skimmer, mezzanine, belt, press house and loft"
        ));
        let (vx0, vx1, vz0, vz1) = VAT;
        let (px0, px1, pz0, pz1) = PRESS;

        // Floor round the vat (the middle band is shared).
        h.bx("floor", [-HX, -3.0, -HZ], [HX, 0.0, vz0], FLOOR);
        h.bx(
            "floor beside the vat",
            [-HX, -3.0, vz0],
            [vx0, 0.0, vz1],
            FLOOR,
        );
        h.bx(
            "floor beside the vat",
            [vx1, -3.0, vz0],
            [HX, 0.0, vz1],
            FLOOR,
        );

        // The rendering vat: molten tallow behind a 0.6 m iron rim; the skimmer platform
        // hanging over it from the hoist rail, a pad up onto it from the south.
        h.bx("vat bed", [vx0, -3.0, vz0], [vx1, -2.5, vz1], IRON_DARK);
        h.bx(
            "vat (molten tallow)",
            [vx0, -2.5, vz0],
            [vx1, -1.2, vz1],
            MOLTEN,
        );
        let (k0, k1) = h.corners([vx0, -3.0, vz0], [vx1, -0.7, vz1]);
        let note = h.note("rendering vat (molten tallow)");
        h.g.kill(&note, vol(k0, k1));
        h.bx(
            "vat rim",
            [vx0 - 0.4, 0.0, vz0 - 0.4],
            [vx1 + 0.4, 0.6, vz0],
            IRON,
        );
        h.bx(
            "vat rim",
            [vx0 - 0.4, 0.0, vz1],
            [vx1 + 0.4, 0.6, vz1 + 0.4],
            IRON,
        );
        h.bx("vat rim", [vx0 - 0.4, 0.0, vz0], [vx0, 0.6, vz1], IRON);
        h.bx("vat rim", [vx1, 0.0, vz0], [vx1 + 0.4, 0.6, vz1], IRON);
        let (cx, cz) = ((vx0 + vx1) * 0.5, (vz0 + vz1) * 0.5);
        h.bx(
            "skimmer platform",
            [cx - 2.0, SKIM - 0.4, cz - 2.0],
            [cx + 2.0, SKIM, cz + 2.0],
            GRATE,
        );
        h.bx(
            "skimmer kerb",
            [cx - 2.1, SKIM - 0.6, cz - 2.1],
            [cx + 2.1, SKIM - 0.4, cz + 2.1],
            IRON,
        );
        for (dx, dz) in [(-1.9, -1.9), (1.7, -1.9), (-1.9, 1.7), (1.7, 1.7)] {
            h.deco(
                "hoist chain",
                [cx + dx, SKIM, cz + dz],
                [cx + dx + 0.2, ROOF - 1.6, cz + dz + 0.2],
                IRON_DARK,
            );
        }
        h.bx(
            "hoist trolley",
            [cx - 2.5, ROOF - 2.4, cz - 0.6],
            [cx + 2.5, ROOF - 1.6, cz + 0.6],
            IRON,
        );
        h.pad(
            "floor -> skimmer",
            [cx, 0.0, vz1 + 2.6],
            [cx, SKIM, cz],
            3.0,
        );
        h.light("vat", [cx, 1.5, cz], L_VAT, 13.0);
        h.light("skimmer", [cx, SKIM + 3.0, cz], L_GAS, 8.0);

        // Pad onto the boiler's top from its west side (south of the vat).
        h.pad(
            "floor -> boiler top",
            [-12.0, 0.0, 3.5],
            [-3.5, BOIL, 3.5],
            2.5,
        );

        // The press house at the west end: a brick block with its loft on top (y 12); tallow
        // stacks on the loft; a wall-jump shaft between it and the solid end of the
        // mezzanine.
        h.bx(
            "press house",
            [px0, 0.0, pz0],
            [px1, LOFT - 0.2, pz1],
            BRICK,
        );
        h.bx(
            "press loft floor",
            [px0, LOFT - 0.2, pz0],
            [px1, LOFT, pz1],
            TIMBER,
        );
        h.bx(
            "press loft edge",
            [px1, LOFT - 0.6, pz0],
            [px1 + 0.3, LOFT + 0.05, pz1],
            HAZARD,
        );
        h.stack("loft tallow stack", [-28.0, LOFT, -5.0], (1.4, 1.4, 1.6));
        h.stack("loft tallow stack", [-27.0, LOFT, 8.5], (1.2, 1.6, 1.4));
        for z in [-10.0, 0.0, 10.0] {
            h.bx(
                "press screw",
                [px1, 3.0, z - 1.2],
                [px1 + 0.5, 9.0, z + 1.2],
                IRON,
            );
            h.bx(
                "press wheel",
                [px1 + 0.5, 5.0, z - 2.0],
                [px1 + 0.8, 7.0, z + 2.0],
                IRON_DARK,
            );
        }
        h.light("loft", [-28.0, LOFT + 4.0, 0.0], L_GAS, 13.0);
        h.light("press house", [px1 + 3.0, 4.0, -6.0], L_GAS, 10.0);

        // The mezzanine along the north wall (y 6): its west end solid (storage), the rest a
        // timber floor on posts; a ramp down to the floor at its east end; a pad up to the
        // loft.
        h.bx(
            "mezzanine (solid end)",
            [-HX, 0.0, -HZ],
            [-24.0, MEZZ, -19.0],
            LIMEWASH,
        );
        h.bx(
            "mezzanine",
            [-24.0, MEZZ - 0.4, -HZ],
            [14.0, MEZZ, -19.0],
            TIMBER,
        );
        h.bx(
            "mezzanine edge",
            [-24.0, MEZZ - 0.7, -19.3],
            [14.0, MEZZ - 0.4, -19.0],
            HAZARD,
        );
        for x in [-14.0, 0.0, 10.0] {
            h.bx(
                "mezzanine post",
                [x - 0.3, 0.0, -20.0],
                [x + 0.3, MEZZ - 0.7, -19.4],
                IRON,
            );
        }
        h.ramp_x(
            "mezzanine ramp",
            (-HZ, -21.0),
            (28.0, 0.0),
            (14.0, MEZZ),
            8.0,
            LIMEWASH,
        );
        h.pad(
            "mezzanine -> loft",
            [-20.0, MEZZ, -22.5],
            [-28.0, LOFT, -9.0],
            3.0,
        );
        h.stack(
            "mezzanine tallow stack",
            [-4.0, MEZZ, -24.0],
            (1.6, 1.4, 1.5),
        );
        h.light("mezzanine", [-6.0, MEZZ + 4.0, -22.5], L_GAS, 12.0);
        h.light("under the mezzanine", [2.0, 3.5, -22.5], L_GAS, 9.0);

        // The belt: a conveyor from the floor up to the loft in a solid housing (it walls off
        // the strip under the mezzanine from the middle of the hall up to its foot).
        h.ramp_x(
            "belt housing",
            (pz0, pz0 + 3.0),
            (8.0, -0.1),
            (px1, LOFT - 0.1),
            10.0,
            LIMEWASH,
        );
        let z = (pz0 + 0.15, pz0 + 2.85);
        h.ramp_x("conveyor belt", z, (8.0, 0.0), (px1, LOFT), 0.3, BELT);
        h.light("belt", [-10.0, 9.0, pz0 + 1.5], L_GAS, 9.0);
        // A parapet along the loft's edge (beside the belt's top): cover for the loft.
        h.bx(
            "loft parapet",
            [px1 - 0.5, LOFT, pz0 + 3.0],
            [px1, LOFT + 1.2, pz1],
            BRICK,
        );

        // Floor clutter: tallow stacks (cover), a skimming tub.
        h.stack("floor tallow stack", [14.0, 0.0, -9.0], (1.5, 1.5, 1.4));
        h.stack("floor tallow stack", [-12.0, 0.0, -22.5], (1.2, 1.2, 1.2));
        h.bx("skimming tub", [20.0, 0.0, -2.0], [23.0, 1.1, 1.0], IRON);
        h.deco(
            "skimming tub (tallow)",
            [20.3, 0.9, -1.7],
            [22.7, 1.05, 0.7],
            TALLOW,
        );
        h.light("floor", [12.0, 6.0, -12.0], L_GAS, 12.0);
        h.light(
            "daylight through the steam",
            [-10.0, ROOF - 3.0, -14.0],
            L_DAY,
            16.0,
        );

        // Spawns: the loft (y 12), the mezzanine (y 6), the floor (y 0) under the mezzanine and
        // in the corner by the gate (out of sight of each other).
        h.spawn([-29.0, LOFT, 2.0]);
        h.spawn([-10.0, MEZZ, -24.0]);
        h.spawn([8.0, 0.0, -23.5]);
        h.spawn([26.0, 0.0, -18.5]);
    }

    g.kill(
        "below the world",
        vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]),
    );
    g
}
