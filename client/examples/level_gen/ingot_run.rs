//! Ingot Run: the long, lane-based PvP arena (best for 4–8 players).
//!
//! A casting line at a cold dawn: two mirrored tapping floors (west and east) face each other
//! across a channel of molten slag. Three lanes run end to end: the high crane gantry along
//! the north wall (long sightlines), the open casting floor in the middle with the hump-backed
//! pour bridge over the slag, and the sunken ingot trough along the south wall (a slide route
//! through two cooling beds and a duct over the slag). Mirror-symmetric west ↔ east. Layout
//! map, routes and spawns: `docs/levels.md`.

use super::*;

mod mat {
    use super::Mat;
    pub const WALL: Mat = Mat("concrete", [0.62, 0.66, 0.72]);
    pub const WALL_PANEL: Mat = Mat("panel", [0.56, 0.62, 0.7]);
    pub const FLOOR: Mat = Mat("concrete", [0.7, 0.72, 0.74]);
    pub const STEEL: Mat = Mat("metal", [0.66, 0.72, 0.8]);
    pub const STEEL_DARK: Mat = Mat("metal", [0.4, 0.44, 0.5]);
    pub const TROUGH: Mat = Mat("metal", [0.5, 0.52, 0.56]);
    pub const GRATE: Mat = Mat("grate", [0.72, 0.78, 0.86]);
    pub const BRICK: Mat = Mat("brick", [0.62, 0.5, 0.48]);
    pub const RUST: Mat = Mat("rust", [0.78, 0.66, 0.6]);
    pub const HAZARD: Mat = Mat("hazard", [1.0, 0.86, 0.45]);
    pub const INGOT: Mat = Mat("metal", [0.86, 0.66, 0.5]);
    /// Molten slag (always has a kill volume, or sits behind a grate).
    pub const SLAG: Mat = Mat("lava", [1.0, 0.72, 0.55]);
}

const L_SLAG: [f32; 3] = [1.35, 0.55, 0.2];
const L_FLOOD: [f32; 3] = [0.8, 0.9, 1.05];
const L_COLD: [f32; 3] = [0.55, 0.68, 0.9];
const L_INGOT: [f32; 3] = [1.2, 0.6, 0.3];

/// Interior half length (x) and half width (z): 96 × 40 m.
const HX: f32 = 48.0;
const HZ: f32 = 20.0;
/// Visible wall height and the invisible clip ceiling.
const TOP: f32 = 14.0;
const SKY: f32 = 30.0;
/// Bottom of every floor slab (the slag channel is 5 m deep).
const BASE: f32 = -5.0;
/// Slag channel half width (x), slag surface.
const CH: f32 = 4.0;
const SLAG_Y: f32 = -3.6;
/// Ingot trough along the south wall: z range and floor height.
const TZ: f32 = 16.0;
const TROUGH_Y: f32 = -1.5;
/// High route: crane gantry (north wall) and the end walkways, y 7.
const HIGH: f32 = 7.0;
const GZ: f32 = -16.0;
/// End walkways: x range (east side; mirrored).
const WX0: f32 = 39.0;
const WX1: f32 = 43.0;

/// One half of the mirror: local coordinates are the east half's (`s` = 1), the west half
/// mirrors x (`s` = -1).
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
        let (a, b) = (self.s * min[0], self.s * max[0]);
        ([a.min(b), min[1], min[2]], [a.max(b), max[1], max[2]])
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
    fn ramp_x(&mut self, what: &str, z: (f32, f32), a: (f32, f32), b: (f32, f32), thick: f32, m: Mat) {
        let note = self.note(what);
        self.g.ramp_x(&note, z, (self.s * a.0, a.1), (self.s * b.0, b.1), thick, m);
    }

    fn pad(&mut self, what: &str, min: [f32; 3], max: [f32; 3], [vx, vy, vz]: [f32; 3]) {
        let (a, b) = self.corners(min, max);
        let note = self.note(what);
        self.g.pad(&note, vol(a, b), [self.s * vx, vy, vz]);
    }

    fn light(&mut self, what: &str, [x, y, z]: [f32; 3], color: [f32; 3], radius: f32) {
        let note = self.note(what);
        self.g.light(&note, [self.s * x, y, z], color, radius);
    }

    /// A player spawn facing the middle of the arena.
    fn spawn(&mut self, [x, y, z]: [f32; 3]) {
        let x = self.s * x;
        self.g.spawn([x, y, z], r4(f32::atan2(x, z).to_degrees()));
    }
}

pub(super) fn ingot_run() -> Gen {
    use mat::*;
    let mut g = Gen::new(
        "Ingot Run",
        LevelKind::Arena,
        Environment {
            sky_color: Vec3::new(0.5, 0.56, 0.64),
            fog_color: Vec3::new(0.44, 0.5, 0.58),
            fog_start: 40.0,
            fog_end: 175.0,
            ambient: Vec3::new(0.5, 0.53, 0.6),
            light_dir: Vec3::new(0.4, -0.75, 0.55),
            light_color: Vec3::new(0.82, 0.8, 0.76),
            music: "arena".into(),
        },
    );

    g.section("Outer shell (open sky, clipped at 30 m)");
    let shell = Room::new("Casting line", [-HX, 0.0, -HZ], [HX, TOP, HZ], WALL).bottom(BASE);
    g.room(&shell);
    g.clip("sky clip wall N", [-HX - 1.0, TOP, -HZ - 1.0], [HX + 1.0, SKY, -HZ]);
    g.clip("sky clip wall S", [-HX - 1.0, TOP, HZ], [HX + 1.0, SKY, HZ + 1.0]);
    g.clip("sky clip wall W", [-HX - 1.0, TOP, -HZ], [-HX, SKY, HZ]);
    g.clip("sky clip wall E", [HX, TOP, -HZ], [HX + 1.0, SKY, HZ]);
    g.clip("sky clip ceiling", [-HX - 1.0, SKY, -HZ - 1.0], [HX + 1.0, SKY + 1.0, HZ + 1.0]);

    g.section("Slag channel, pour bridge, south duct, crane and ladle (centre)");
    g.bx("slag channel bed", [-CH, BASE, -HZ], [CH, SLAG_Y - 0.4, HZ], STEEL_DARK);
    g.bx("molten slag", [-CH, SLAG_Y - 0.4, -HZ], [CH, SLAG_Y, HZ], SLAG);
    g.kill("slag channel", vol([-CH, BASE, -HZ], [CH, -2.4, HZ]));
    g.bx("pour bridge deck", [-CH, 2.5, -3.0], [CH, 3.0, 3.0], STEEL);
    g.bx("pour bridge girder N", [-CH, 1.8, -3.0], [CH, 2.5, -2.6], STEEL_DARK);
    g.bx("pour bridge girder S", [-CH, 1.8, 2.6], [CH, 2.5, 3.0], STEEL_DARK);
    g.bx("south duct floor", [-CH, -2.1, TZ], [CH, TROUGH_Y, HZ], TROUGH);
    g.bx("south duct wall", [-CH, -2.1, TZ - 0.6], [CH, -0.3, TZ], STEEL_DARK);
    g.bx("south duct roof (grate walkway, 1.2 m gap)", [-CH, -0.3, TZ - 0.6], [CH, 0.0, HZ], GRATE);
    g.bx("crane girder", [-1.5, 12.0, -HZ], [1.5, 13.2, HZ], STEEL_DARK);
    g.bx("crane trolley", [-2.0, 11.2, -1.5], [2.0, 12.0, 1.5], RUST);
    for (x, z) in [(-1.1, -1.1), (0.9, 0.9)] {
        g.bx("ladle chain", [x, 10.4, z], [x + 0.2, 11.2, z + 0.2], STEEL_DARK);
    }
    g.bx("ladle rim", [-1.9, 10.0, -1.9], [1.9, 10.4, 1.9], STEEL_DARK);
    g.bx("ladle", [-1.7, 8.2, -1.7], [1.7, 10.0, 1.7], RUST);
    g.bx("ladle base", [-1.2, 7.6, -1.2], [1.2, 8.2, 1.2], RUST);
    g.light("slag channel N", [0.0, -1.5, -12.0], L_SLAG, 10.0);
    g.light("slag under the bridge", [0.0, -1.0, 0.0], L_SLAG, 10.0);
    g.light("slag channel S", [0.0, -1.5, 10.0], L_SLAG, 10.0);
    g.light("south duct", [0.0, -0.9, 18.0], L_INGOT, 5.0);
    g.light("ladle", [0.0, 6.5, 0.0], L_SLAG, 8.0);

    g.section("Crane gantry along the north wall (y 7)");
    g.bx("crane gantry", [-WX0, HIGH - 0.4, -HZ], [WX0, HIGH, GZ], GRATE);
    g.bx("crane gantry girder", [-WX0, HIGH - 1.0, GZ - 0.4], [WX0, HIGH - 0.4, GZ], STEEL_DARK);

    for (s, name) in [(1.0f32, "E"), (-1.0, "W")] {
        let mut h = Half { g: &mut g, s, name };
        h.g.section(&format!("{name} half: casting floor, trough, tapping floor, high route"));

        // Floors: the casting floor, the ingot trough (sunken 1.5 m along the south wall) and
        // its ramp up at the end; a hazard curb along the slag channel.
        h.bx("casting floor", [CH, BASE, -HZ], [HX, 0.0, TZ], FLOOR);
        h.bx("ingot trough floor", [CH, BASE, TZ], [44.0, TROUGH_Y, HZ], TROUGH);
        h.ramp_x("ingot trough ramp", (TZ, HZ), (40.0, TROUGH_Y), (44.0, 0.0), 3.0, TROUGH);
        h.bx("trough end floor", [44.0, BASE, TZ], [HX, 0.0, HZ], FLOOR);
        h.bx("channel curb", [CH, 0.0, -HZ], [CH + 0.5, 0.12, TZ - 0.6], HAZARD);
        h.bx("trough curb", [CH + 0.5, 0.0, TZ - 0.4], [40.0, 0.12, TZ], HAZARD);

        // Cooling bed over the trough (a 1.2 m slide tunnel) with ingots on top.
        h.bx("cooling bed (slide tunnel roof)", [18.0, -0.3, TZ], [28.0, 0.0, HZ], STEEL_DARK);
        for x in [19.0, 22.5, 26.0] {
            h.bx("cooling ingot", [x, 0.0, 17.0], [x + 1.6, 0.3, 19.0], INGOT);
        }
        h.light("cooling bed", [23.0, -0.8, 18.0], L_INGOT, 6.0);

        // Pour bridge approach (hump: 0 -> 3 m).
        h.ramp_x("pour bridge ramp", (-3.0, 3.0), (11.0, 0.0), (CH, 3.0), 4.0, STEEL);

        // End walkway (y 7) from the gantry to the south ramp, the furnace behind it.
        h.bx("end walkway", [WX0, HIGH - 0.4, -HZ], [WX1, HIGH, 14.0], GRATE);
        h.bx("end walkway girder", [WX0, HIGH - 1.0, GZ], [WX0 + 0.4, HIGH - 0.4, 9.0], STEEL_DARK);
        for z in [-10.0, -1.0] {
            h.bx("end walkway post", [WX0, 0.0, z - 0.3], [WX0 + 0.6, HIGH - 1.0, z + 0.3], STEEL_DARK);
        }
        h.bx("tapping house", [WX0, 0.0, 9.0], [WX1, HIGH - 0.4, 14.0], BRICK);
        h.pad("tapping floor -> furnace top", [44.5, 0.0, 8.5], [46.5, 1.0, 10.5], [0.0, 31.0, -4.0]);
        h.ramp_x("south ramp to the walkway", (10.0, 14.0), (24.5, 0.0), (WX0, HIGH), 8.0, FLOOR);
        h.bx("furnace", [WX1, 0.0, -7.0], [HX, 11.0, 7.0], BRICK);
        h.bx("furnace band", [WX1 - 0.2, 5.4, -6.9], [HX - 0.1, 6.0, 6.9], STEEL_DARK);
        h.deco("furnace taphole slag", [WX1 - 0.2, 0.4, -1.6], [WX1 + 0.2, 2.6, 1.6], SLAG);
        h.bx("furnace taphole grate", [WX1 - 0.45, 0.3, -1.8], [WX1 - 0.2, 2.7, 1.8], GRATE);
        h.bx("furnace chimney", [44.5, 11.0, -3.0], [HX, 20.0, 3.0], BRICK);
        h.light("taphole", [WX1 - 2.0, 1.5, 0.0], L_SLAG, 8.0);

        // North-east corner: a wall-jump shaft between the furnace and a buttress up to the
        // walkway, a coal bunker behind it.
        h.bx("buttress (wall-jump shaft)", [WX1, 0.0, -11.5], [HX, 7.6, -10.5], WALL_PANEL);
        h.bx("coal bunker", [WX1, 0.0, -HZ], [HX, 4.0, -11.5], RUST);

        // Wall-jump racks under the gantry (3.5 m shaft up to y 7).
        h.bx("ingot rack (wall-jump shaft)", [19.0, 0.0, GZ], [20.0, 7.6, -11.0], RUST);
        h.bx("ingot rack (wall-jump shaft)", [23.5, 0.0, GZ], [24.5, 7.6, -11.0], RUST);
        for x in [10.0, 32.0] {
            h.bx("gantry post", [x - 0.3, 0.0, GZ - 0.6], [x + 0.3, HIGH - 1.0, GZ], STEEL_DARK);
        }
        h.bx("gantry ingot stack", [14.0, HIGH, -HZ], [16.5, HIGH + 1.2, -18.2], INGOT);
        h.bx("gantry ingot stack", [30.0, HIGH, -17.8], [32.5, HIGH + 1.2, GZ], INGOT);
        h.light("gantry flood", [21.5, 10.0, -18.0], L_FLOOD, 14.0);
        h.light("walkway flood", [41.0, 10.0, -6.0], L_FLOOD, 12.0);

        // Pad from the casting floor onto the gantry.
        h.pad("floor -> gantry", [9.0, 0.0, -10.2], [11.4, 1.0, -7.8], [0.0, 30.0, -7.4]);

        // Ladle stand (y 3, step crate) and cover on the casting floor.
        h.bx("ladle stand", [15.0, 0.0, 3.0], [21.0, 3.0, 9.0], BRICK);
        h.bx("ladle stand step", [13.5, 0.0, 5.0], [15.0, 1.2, 7.0], RUST);
        h.bx("ladle on the stand", [18.5, 3.0, 6.5], [20.5, 4.3, 8.5], RUST);
        h.bx("ingot stack", [27.0, 0.0, -6.0], [30.0, 1.2, -4.0], INGOT);
        h.bx("mould box", [29.0, 0.0, 2.0], [32.0, 2.6, 5.0], STEEL_DARK);
        h.bx("ingot stack", [12.0, 0.0, 12.5], [15.0, 1.2, 14.5], INGOT);
        h.light("casting floor", [24.0, 6.0, 0.0], L_COLD, 14.0);
        h.light("ladle stand", [18.0, 5.0, 6.0], L_INGOT, 7.0);
        h.light("trough", [12.0, 0.5, 18.0], L_COLD, 7.0);

        // Spawns: walkway (y 7), gantry (y 7), ladle stand (y 3), tapping floor, casting floor.
        h.spawn([41.0, HIGH, -8.0]);
        h.spawn([24.0, HIGH, -18.0]);
        h.spawn([17.0, 3.0, 5.0]);
        h.spawn([36.0, 0.0, 3.0]);
        h.spawn([10.0, 0.0, 13.0]);
    }

    g.kill("below the world", vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]));
    g
}
