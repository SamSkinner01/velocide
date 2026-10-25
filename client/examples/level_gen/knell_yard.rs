//! Knell Yard: the compact, vertical PvP arena (best for 2–5 players, up to 8).
//!
//! A lantern-lit bell foundry yard at dusk: an open belfry stands in the middle of a moat of
//! molten bell bronze, ringed by pattern sheds, kilns and casting tables. The layout is a
//! **pinwheel** (90° rotational symmetry): everything is described once for the north quarter
//! and turned three times. Layout map, routes and spawns: `docs/levels.md`.

use super::*;

mod mat {
    use super::Mat;
    pub const WALL: Mat = Mat("brick", [0.78, 0.56, 0.48]);
    pub const WALL_DARK: Mat = Mat("brick", [0.6, 0.44, 0.4]);
    pub const KILN: Mat = Mat("brick", [0.68, 0.46, 0.4]);
    pub const COPING: Mat = Mat("concrete", [0.7, 0.62, 0.58]);
    pub const YARD: Mat = Mat("dirt", [0.66, 0.58, 0.52]);
    pub const STONE: Mat = Mat("concrete", [0.84, 0.76, 0.68]);
    pub const STONE_DARK: Mat = Mat("concrete", [0.62, 0.56, 0.52]);
    pub const FLAGS: Mat = Mat("tile", [0.74, 0.64, 0.58]);
    pub const WOOD: Mat = Mat("wood", [0.92, 0.72, 0.52]);
    pub const WOOD_DARK: Mat = Mat("wood", [0.64, 0.48, 0.38]);
    pub const BRONZE: Mat = Mat("metal", [1.0, 0.76, 0.44]);
    pub const VERDIGRIS: Mat = Mat("metal", [0.52, 0.82, 0.7]);
    pub const IRON: Mat = Mat("metal", [0.46, 0.44, 0.48]);
    pub const CLAY: Mat = Mat("rust", [0.82, 0.62, 0.52]);
    pub const GRATE: Mat = Mat("grate", [0.86, 0.7, 0.5]);
    /// Molten bell bronze (always has a kill volume) and lantern flames (behind grates).
    pub const MELT: Mat = Mat("lava", [1.0, 0.92, 0.6]);
}

const L_LANTERN: [f32; 3] = [1.25, 0.82, 0.45];
const L_MELT: [f32; 3] = [1.3, 0.92, 0.38];
const L_KILN: [f32; 3] = [1.3, 0.5, 0.18];
const L_DUSK: [f32; 3] = [0.7, 0.55, 0.85];

/// Interior half size (48 × 48 m).
const H: f32 = 24.0;
/// Visible wall height and the invisible clip ceiling.
const TOP: f32 = 12.0;
const SKY: f32 = 26.0;
/// Belfry tower: outer half size of its walls, inner (arcade) half size.
const TW: f32 = 6.0;
const TI: f32 = 5.0;
/// Moat: molten bronze between the tower (6) and the lining (9.7..10).
const MI: f32 = 9.7;
const MO: f32 = 10.0;
/// Bottom of every floor slab.
const BASE: f32 = -3.0;
/// Deck (belfry floor, y 5) half size, belfry roof (the crown, y 10) half size.
const DECK: f32 = 5.0;
const DH: f32 = 7.0;
const CROWN: f32 = 10.0;
const CH: f32 = 3.0;

/// `(x, z)` turned `k` × 90° counter-clockwise seen from above (north → west → south → east).
fn turn(k: usize, x: f32, z: f32) -> (f32, f32) {
    match k % 4 {
        0 => (x, z),
        1 => (z, -x),
        2 => (-x, -z),
        _ => (-z, x),
    }
}

/// One quarter of the pinwheel: local coordinates are the north quarter's.
struct Quarter<'a> {
    g: &'a mut Gen,
    k: usize,
}

impl Quarter<'_> {
    fn corners(&self, min: [f32; 3], max: [f32; 3]) -> ([f32; 3], [f32; 3]) {
        let (ax, az) = turn(self.k, min[0], min[2]);
        let (bx, bz) = turn(self.k, max[0], max[2]);
        ([ax.min(bx), min[1], az.min(bz)], [ax.max(bx), max[1], az.max(bz)])
    }

    fn bx(&mut self, note: &str, min: [f32; 3], max: [f32; 3], m: Mat) -> &mut Brush {
        let (a, b) = self.corners(min, max);
        self.g.bx(note, a, b, m)
    }

    fn deco(&mut self, note: &str, min: [f32; 3], max: [f32; 3], m: Mat) {
        self.bx(note, min, max, m).no_collide = true;
    }

    fn vol(&self, min: [f32; 3], max: [f32; 3]) -> Volume {
        let (a, b) = self.corners(min, max);
        vol(a, b)
    }

    /// Walkable ramp along local X (`along_x`) or Z, from the low end `a` to the high end `b`
    /// (each `(horizontal, y)`), spanning `w` on the other axis.
    #[allow(clippy::too_many_arguments)]
    fn ramp(&mut self, note: &str, along_x: bool, w: (f32, f32), a: (f32, f32), b: (f32, f32), thick: f32, m: Mat) {
        let (dx, dz) = if along_x { (1.0, 0.0) } else { (0.0, 1.0) };
        let (rx, rz) = turn(self.k, dx, dz);
        let (ex, ez) = turn(self.k, dz, dx);
        let (s, t) = (rx + rz, ex + ez);
        let (c0, c1) = (t * w.0, t * w.1);
        self.g.slope(note, rx.abs() > 0.5, (c0.min(c1), c0.max(c1)), (s * a.0, a.1), (s * b.0, b.1), thick, true, (1.0, 0.0), m);
    }

    fn pad(&mut self, note: &str, min: [f32; 3], max: [f32; 3], [vx, vy, vz]: [f32; 3]) {
        let v = self.vol(min, max);
        let (x, z) = turn(self.k, vx, vz);
        self.g.pad(note, v, [r4(x), vy, r4(z)]);
    }

    fn kill(&mut self, note: &str, min: [f32; 3], max: [f32; 3]) {
        let v = self.vol(min, max);
        self.g.kill(note, v);
    }

    fn light(&mut self, note: &str, [x, y, z]: [f32; 3], color: [f32; 3], radius: f32) {
        let (x, z) = turn(self.k, x, z);
        self.g.light(note, [x, y, z], color, radius);
    }

    /// A player spawn facing the belfry.
    fn spawn(&mut self, [x, y, z]: [f32; 3]) {
        let (x, z) = turn(self.k, x, z);
        self.g.spawn([x, y, z], r4(f32::atan2(x, z).to_degrees()));
    }

    /// A caged lantern: grate box with a glowing flame inside (behind the grate, decoration),
    /// optionally hanging on a rod from `hang_to`.
    fn lantern(&mut self, note: &str, [x, y, z]: [f32; 3], hang_to: Option<f32>) {
        self.bx(&format!("{note} cage"), [x - 0.3, y - 0.4, z - 0.3], [x + 0.3, y + 0.4, z + 0.3], mat::GRATE);
        self.bx(&format!("{note} cap"), [x - 0.38, y + 0.4, z - 0.38], [x + 0.38, y + 0.55, z + 0.38], mat::IRON);
        if let Some(top) = hang_to {
            self.bx(&format!("{note} rod"), [x - 0.05, y + 0.55, z - 0.05], [x + 0.05, top, z + 0.05], mat::IRON);
        }
        self.deco(&format!("{note} flame"), [x - 0.18, y - 0.28, z - 0.18], [x + 0.18, y + 0.24, z + 0.18], mat::MELT);
    }
}

pub(super) fn knell_yard() -> Gen {
    use mat::*;
    let mut g = Gen::new(
        "Knell Yard",
        LevelKind::Arena,
        Environment {
            sky_color: Vec3::new(0.17, 0.09, 0.16),
            fog_color: Vec3::new(0.22, 0.12, 0.17),
            fog_start: 20.0,
            fog_end: 100.0,
            ambient: Vec3::new(0.42, 0.34, 0.36),
            light_dir: Vec3::new(-0.75, -0.55, 0.35),
            light_color: Vec3::new(0.85, 0.55, 0.38),
            music: "arena".into(),
        },
    );

    g.section("Outer shell (open sky, clipped at 26 m)");
    let shell = Room::new("Yard", [-H, 0.0, -H], [H, TOP, H], WALL).bottom(BASE);
    g.room(&shell);
    g.clip("sky clip wall N", [-H - 1.0, TOP, -H - 1.0], [H + 1.0, SKY, -H]);
    g.clip("sky clip wall S", [-H - 1.0, TOP, H], [H + 1.0, SKY, H + 1.0]);
    g.clip("sky clip wall W", [-H - 1.0, TOP, -H], [-H, SKY, H]);
    g.clip("sky clip wall E", [H, TOP, -H], [H + 1.0, SKY, H]);
    g.clip("sky clip ceiling", [-H - 1.0, SKY, -H - 1.0], [H + 1.0, SKY + 1.0, H + 1.0]);

    for k in 0..4 {
        let mut q = Quarter { g: &mut g, k };
        q.g.section(&format!("Quarter {} (pinwheel, turned {}°)", k + 1, k * 90));

        // Floors: the yard band of this quarter, the moat strip (bed, molten bronze, lining,
        // lip). The four quarters tile the ring like a pinwheel.
        q.bx("yard floor", [-MO, BASE, -H], [H, 0.0, -MO], YARD);
        q.bx("moat bed", [-TW, BASE, -MI], [MI, -2.2, -TW], IRON);
        q.bx("molten bronze", [-TW, -2.2, -MI], [MI, -1.8, -TW], MELT);
        q.kill("molten bronze moat", [-TW, BASE, -MI], [MI, -1.0, -TW]);
        q.bx("moat lining", [-MI, BASE, -MO], [MO, 0.0, -MI], WALL_DARK);
        q.bx("moat lip", [-MI, 0.0, -MO - 0.3], [MO + 0.3, 0.1, -MI], BRONZE);
        q.bx("wall coping", [-H - 1.0, TOP, -H - 1.0], [H - 0.2, TOP + 0.35, -H + 0.2], COPING);

        // Belfry tower wall (arcade at y 0): an arch on the east half of the face, a bridge over
        // the moat in front of it.
        q.bx("belfry wall", [-TW, BASE, -TW], [1.5, 4.5, -TI], STONE);
        q.bx("belfry arch sill", [1.5, BASE, -TW], [4.5, 0.0, -TI], STONE);
        q.bx("belfry arch lintel", [1.5, 3.5, -TW], [4.5, 4.5, -TI], STONE);
        q.bx("belfry wall", [4.5, BASE, -TW], [TI, 4.5, -TI], STONE);
        q.bx("arch jamb", [1.2, 0.0, -6.2], [1.7, 3.9, -5.85], BRONZE);
        q.bx("arch jamb", [4.3, 0.0, -6.2], [4.8, 3.9, -5.85], BRONZE);
        q.bx("arch head", [1.2, 3.4, -6.2], [4.8, 3.9, -5.85], BRONZE);
        q.bx("moat bridge planks", [1.5, -0.35, -MI], [4.5, 0.0, -TW], WOOD);
        q.bx("moat bridge girder", [1.5, -0.8, -MI], [1.8, -0.35, -TW], IRON);
        q.bx("moat bridge girder", [4.2, -0.8, -MI], [4.5, -0.35, -TW], IRON);
        q.lantern("arcade lantern", [0.0, 3.0, -1.5], None);
        q.light("arcade", [0.0, 3.0, -2.6], L_LANTERN, 6.0);

        // Deck (y 5, the arcade roof) edge band, belfry pier, corner pad onto the crown.
        q.bx("deck edge band", [-DH - 0.1, 4.4, -DH - 0.1], [DH - 0.1, 4.95, -DH + 0.1], BRONZE);
        q.bx("belfry pier", [1.9, DECK, -CH], [CH, CROWN - 0.4, -1.9], STONE);
        q.bx("crown edge band", [-CH - 0.1, CROWN - 0.5, -CH - 0.1], [CH - 0.1, CROWN - 0.05, -CH + 0.1], VERDIGRIS);
        q.bx("crown finial", [CH - 0.6, CROWN, -CH + 0.1], [CH - 0.1, CROWN + 0.9, -CH + 0.6], VERDIGRIS);
        q.pad("deck corner -> crown", [5.6, DECK, -6.8], [6.8, DECK + 1.0, -5.6], [-3.7, 28.0, 3.7]);

        // Bridge from the pattern-shed roof to the deck (y 5), west half of the face.
        q.bx("deck bridge planks", [-4.25, 4.6, -19.8], [-1.25, DECK, -DH], WOOD);
        q.bx("deck bridge stringer", [-4.25, 4.1, -19.8], [-3.95, 4.6, -DH], WOOD_DARK);
        q.bx("deck bridge stringer", [-1.55, 4.1, -19.8], [-1.25, 4.6, -DH], WOOD_DARK);
        q.lantern("bridge lantern", [-2.75, 3.3, -14.0], Some(4.6));

        // Moat pad: from the yard over the molten bronze onto the deck.
        q.pad("yard -> deck (over the moat)", [-1.0, 0.0, -13.5], [1.2, 1.0, -11.1], [0.0, 26.0, 6.0]);

        // Pattern shed along the north wall: its roof is the gantry (y 5); a ramp down the
        // wall to the east.
        q.bx("pattern shed", [-10.0, 0.0, -H], [6.0, 4.6, -20.0], WOOD_DARK);
        q.bx("pattern shed roof (gantry)", [-10.2, 4.6, -H], [6.0, DECK, -19.8], WOOD);
        q.bx("pattern shed door", [-7.5, 0.0, -20.0], [-3.5, 3.4, -19.85], WOOD);
        q.bx("pattern shed door rail", [-8.2, 3.4, -20.0], [-2.8, 3.7, -19.75], IRON);
        q.bx("pattern shed post", [-10.3, 0.0, -20.0], [-9.8, 4.6, -19.7], STONE_DARK);
        q.bx("pattern shed post", [0.75, 0.0, -20.0], [1.25, 4.6, -19.7], STONE_DARK);
        q.ramp("shed ramp", true, (-H, -20.0), (16.5, 0.0), (6.0, DECK), 8.0, STONE_DARK);
        q.lantern("shed lantern", [3.5, 3.6, -19.7], None);
        q.light("shed", [3.5, 3.0, -18.0], L_LANTERN, 9.0);

        // Kiln in the north-east corner: roof perch (y 7.5), firemouth, chimney; a pad from
        // the yard and a wall-jump shaft between its south face and a flask rack.
        q.bx("kiln", [17.0, 0.0, -H], [H, 7.2, -17.0], KILN);
        q.bx("kiln roof plate", [16.8, 7.2, -H], [H, 7.5, -16.8], IRON);
        q.bx("kiln hoop", [16.85, 3.5, -H + 0.1], [H - 0.1, 3.9, -16.85], IRON);
        q.deco("kiln firemouth flame", [16.85, 0.3, -21.6], [17.2, 1.9, -19.4], MELT);
        q.bx("kiln firemouth grate", [16.7, 0.2, -21.8], [16.9, 2.1, -19.2], GRATE);
        q.bx("kiln chimney", [21.5, 7.5, -H], [H, 14.0, -21.5], KILN);
        q.bx("kiln chimney cap", [21.3, 14.0, -H], [H, 14.4, -21.3], IRON);
        q.light("kiln firemouth", [15.5, 1.5, -20.5], L_KILN, 8.0);
        q.pad("yard -> kiln roof", [11.3, 0.0, -13.7], [13.7, 1.0, -11.3], [6.3, 30.0, -6.3]);
        q.bx("flask rack (wall-jump shaft)", [18.5, 0.0, -13.5], [H, 6.0, -12.0], WOOD_DARK);
        q.bx("flask rack cap", [18.4, 6.0, -13.6], [H, 6.2, -11.9], IRON);

        // Yard cover: a casting table (slide under the 1.2 m gap or hop on), a clay cope mould,
        // a lantern post at the moat.
        q.bx("casting table", [7.0, 1.2, -18.0], [10.0, 2.0, -12.0], IRON);
        for (x, z) in [(7.0, -18.0), (9.5, -18.0), (7.0, -12.5), (9.5, -12.5)] {
            q.bx("casting table leg", [x, 0.0, z], [x + 0.5, 1.2, z + 0.5], WOOD_DARK);
        }
        q.bx("cope mould", [-8.5, 0.0, -16.5], [-5.0, 1.6, -13.0], CLAY);
        q.bx("cope mould top", [-8.0, 1.6, -16.0], [-5.5, 2.6, -13.5], CLAY);
        q.bx("lantern post", [14.35, 0.0, -10.95], [14.65, 3.2, -10.65], IRON);
        q.lantern("post lantern", [14.5, 3.6, -10.8], None);
        q.light("moat post", [14.5, 4.2, -10.8], L_LANTERN, 9.0);
        q.light("molten bronze", [1.5, -0.6, -8.0], L_MELT, 7.0);
        q.light("dusk", [-15.0, 8.0, -15.0], L_DUSK, 12.0);

        // Spawns: yard, shed roof (y 5), kiln roof (y 7.5).
        q.spawn([4.5, 0.0, -15.5]);
        q.spawn([-7.0, DECK, -22.0]);
        q.spawn([20.0, 7.5, -20.0]);
    }

    g.section("Belfry: arcade, deck (y 5), bell, crown (y 10)");
    g.bx("arcade floor", [-TI, BASE, -TI], [TI, 0.0, TI], FLAGS);
    g.bx("arcade pillar", [-1.2, 0.0, -1.2], [1.2, 4.5, 1.2], STONE_DARK);
    g.bx("deck", [-DH, 4.5, -DH], [DH, DECK, DH], STONE);
    g.bx("crown (belfry roof)", [-CH, CROWN - 0.4, -CH], [CH, CROWN, CH], BRONZE);
    // The bell hangs high enough to walk under (lip 2.2 m above the deck, clapper 1.95 m).
    g.bx("bell headstock", [-0.3, 9.2, -1.8], [0.3, CROWN - 0.4, 1.8], WOOD_DARK);
    g.bx("bell crown", [-0.55, 9.0, -0.55], [0.55, 9.2, 0.55], BRONZE);
    g.bx("bell shoulder", [-0.8, 8.5, -0.8], [0.8, 9.0, 0.8], BRONZE);
    g.bx("bell waist", [-1.0, 7.8, -1.0], [1.0, 8.5, 1.0], BRONZE);
    g.bx("bell sound bow", [-1.2, 7.4, -1.2], [1.2, 7.8, 1.2], BRONZE);
    g.bx("bell lip", [-1.3, 7.2, -1.3], [1.3, 7.4, 1.3], VERDIGRIS);
    g.bx("bell clapper", [-0.18, 6.95, -0.18], [0.18, 7.35, 0.18], IRON);
    g.light("belfry", [0.0, 8.0, -3.0], L_LANTERN, 8.0);
    g.light("crown", [0.0, 12.5, 0.0], L_LANTERN, 10.0);

    g.kill("below the world", vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]));
    g
}
