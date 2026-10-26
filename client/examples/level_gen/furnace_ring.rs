//! Furnace Ring: a big round tiered pit built for grind (fine for PvP too, 4–8 players).
//!
//! An open-air smelting pit at night: an octagonal bowl of charcoal stone stepping up in three
//! rings — the pit floor (y 0) round the Great Crucible standing in a pool of white-hot slag,
//! tier 1 (y 7) and the rim (y 14) under a 10 m outer wall. Twelve Mould tunnels feed it: on
//! each of the four axis sides a bore at the pit floor and one at tier 1 run back through the
//! tiers to gate chambers behind the outer wall, and a gate opens straight onto the rim. Ramps
//! on the diagonal sides (a pinwheel), jump pads, wall-jump slots and the crucible's top tie the
//! rings together. Four-fold rotational symmetry: one quarter (an axis side and the diagonal
//! after it, clockwise) is described and turned four times. Layout map, routes and spawns:
//! `docs/levels.md`.

use super::gates::{Gate, GateStyle, Spawns, pad_velocity};
use super::*;

mod mat {
    use super::Mat;
    /// Charcoal stone: the tiers and the outer wall.
    pub const STONE: Mat = Mat("concrete", [0.52, 0.52, 0.58]);
    pub const STONE_DARK: Mat = Mat("concrete", [0.4, 0.4, 0.46]);
    /// Ash-pale floor flags.
    pub const ASH: Mat = Mat("tile", [0.62, 0.6, 0.6]);
    pub const IRON: Mat = Mat("metal", [0.46, 0.48, 0.55]);
    pub const IRON_DARK: Mat = Mat("metal", [0.3, 0.31, 0.36]);
    /// Firebrick bands and the crucible's lining.
    pub const FIREBRICK: Mat = Mat("brick", [0.72, 0.46, 0.38]);
    pub const GRATE: Mat = Mat("grate", [0.62, 0.62, 0.7]);
    pub const HAZARD: Mat = Mat("hazard", [0.95, 0.8, 0.5]);
    /// White-hot slag (always has a kill volume, or sits behind a grate).
    pub const SLAG: Mat = Mat("lava", [1.0, 0.95, 0.82]);
}

/// White-hot slag, furnace orange, cold sky fill.
const L_WHITE: [f32; 3] = [1.4, 1.18, 0.88];
const L_FURNACE: [f32; 3] = [1.3, 0.6, 0.25];
const L_SKY: [f32; 3] = [0.5, 0.6, 0.95];

/// tan 22.5°: an octagon of apothem `a` has sides `2 a TAN8` long.
const TAN8: f32 = 0.414_213_56;
const S2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Apothems: slag pool (outer), crucible, flue, pit edge (tier 1 front), tier 2 front, outer
/// wall (inner face).
const POOL: f32 = 8.5;
const CRU: f32 = 5.0;
const FLUE: f32 = 2.0;
const A0: f32 = 20.0;
const A1: f32 = 30.0;
const A2: f32 = 40.0;
/// Heights: tier 1, the rim, the outer wall, the clip ceiling, the bottom of the floor slabs,
/// the crucible's top and its flue.
const Y1: f32 = 7.0;
const Y2: f32 = 14.0;
const TOP: f32 = 24.0;
const SKY: f32 = 40.0;
const BASE: f32 = -2.0;
const CRU_TOP: f32 = 10.0;
const FLUE_TOP: f32 = 22.0;
/// Tunnel bores (5 × 5 m, the gate opening) and the spawn alcoves (along the side, depth,
/// height).
const BORE: f32 = 2.5;
const ALCOVE: (f32, f32) = (-6.5, -3.5);
const ALCOVE_D: f32 = 3.0;
const ALCOVE_H: f32 = 3.5;
/// Wall-jump slot in the tier 2 front (along the side, depth).
const SLOT: (f32, f32) = (4.0, 7.5);
const SLOT_D: f32 = 2.0;

pub(super) fn half(apothem: f32) -> f32 {
    apothem * TAN8
}

/// Outward normal `(x, z)` of octagon side `k` (0 north, 1 north-east, ... clockwise seen
/// from above).
fn normal(k: usize) -> (f32, f32) {
    match k % 8 {
        0 => (0.0, -1.0),
        1 => (S2, -S2),
        2 => (1.0, 0.0),
        3 => (S2, S2),
        4 => (0.0, 1.0),
        5 => (-S2, S2),
        6 => (-1.0, 0.0),
        _ => (-S2, -S2),
    }
}

/// World `(x, z)` of side `k`'s local `(t, d)`: `t` along the side (clockwise), `d` out from
/// the centre.
pub(super) fn at(k: usize, t: f32, d: f32) -> (f32, f32) {
    let (nx, nz) = normal(k);
    (-nz * t + nx * d, nx * t + nz * d)
}

/// A box in side `k`'s frame. Axis sides (even `k`) are axis-aligned; diagonal sides are turned
/// 45° and their tops sit 2 cm lower, so where they overlap an axis-aligned neighbour no two
/// differently turned faces are coplanar (the 2 cm step is invisible and below step height).
fn seg(
    g: &mut Gen,
    note: &str,
    k: usize,
    t: (f32, f32),
    d: (f32, f32),
    y: (f32, f32),
    m: Mat,
) -> usize {
    seg_around(g, note, (0.0, 0.0), k, t, d, y, m)
}

/// [`seg`] for an octagon centred on `(cx, cz)`.
#[allow(clippy::too_many_arguments)]
pub(super) fn seg_around(
    g: &mut Gen,
    note: &str,
    (ox, oz): (f32, f32),
    k: usize,
    (t0, t1): (f32, f32),
    (d0, d1): (f32, f32),
    (y0, y1): (f32, f32),
    m: Mat,
) -> usize {
    if k.is_multiple_of(2) {
        let (ax, az) = at(k, t0, d0);
        let (bx, bz) = at(k, t1, d1);
        g.bx(
            note,
            [ox + ax.min(bx), y0, oz + az.min(bz)],
            [ox + ax.max(bx), y1, oz + az.max(bz)],
            m,
        );
    } else {
        let y1 = y1 - 0.02;
        let (cx, cz) = at(k, (t0 + t1) * 0.5, (d0 + d1) * 0.5);
        let c = Vec3::new(ox + cx, (y0 + y1) * 0.5, oz + cz);
        let h = Vec3::new((t1 - t0).abs(), y1 - y0, (d1 - d0).abs()) * 0.5;
        let (nx, nz) = normal(k);
        let b = g.push(note, brush(c - h, c + h, m));
        // Local +Z turns onto the side's outward normal.
        b.rotation = rv(Vec3::new(0.0, nx.atan2(nz).to_degrees(), 0.0));
    }
    g.def.brushes.len() - 1
}

/// A walkable ramp along side `k`, its surface rising from `(t_lo, y_lo)` to `(t_hi, y_hi)`
/// (along the side) across `d0..d1`; the low end is buried 1 m, `thick` fills the wedge.
#[allow(clippy::too_many_arguments)]
fn side_ramp(
    g: &mut Gen,
    note: &str,
    k: usize,
    (t_lo, y_lo): (f32, f32),
    (t_hi, y_hi): (f32, f32),
    (d0, d1): (f32, f32),
    thick: f32,
    m: Mat,
) {
    let d = (d0 + d1) * 0.5;
    let (ax, az) = at(k, t_lo, d);
    let (bx, bz) = at(k, t_hi, d);
    let run = Vec3::new(bx - ax, 0.0, bz - az);
    let len = run.length();
    let hdir = run / len;
    let rise = y_hi - y_lo;
    let slope = (rise * rise + len * len).sqrt();
    let (sin, cos) = (rise / slope, len / slope);
    // Unit vector up the slope, and the surface normal.
    let up = hdir * cos + Vec3::Y * sin;
    let n = Vec3::new(-sin * hdir.x, cos, -sin * hdir.z);
    let a = Vec3::new(ax, y_lo, az) - up * 1.0;
    let b = Vec3::new(bx, y_hi, bz);
    let c = (a + b) * 0.5 - n * (thick * 0.5);
    let h = Vec3::new((d1 - d0).abs() * 0.5, thick * 0.5, (slope + 1.0) * 0.5);
    let r = g.push(note, brush(c - h, c + h, m));
    r.rotation = rv(Vec3::new(
        -sin.atan2(cos).to_degrees(),
        hdir.x.atan2(hdir.z).to_degrees(),
        0.0,
    ));
}

/// One quarter: axis side `k` (0 N, 2 E, 4 S, 6 W) and the diagonal side after it.
struct Quarter<'a> {
    g: &'a mut Gen,
    k: usize,
    name: &'static str,
}

impl Quarter<'_> {
    fn note(&self, what: &str) -> String {
        format!("{} {what}", self.name)
    }

    /// A box on the axis side (`diag` false) or the diagonal side after it.
    fn seg(
        &mut self,
        diag: bool,
        what: &str,
        t: (f32, f32),
        d: (f32, f32),
        y: (f32, f32),
        m: Mat,
    ) -> usize {
        let note = self.note(what);
        seg(self.g, &note, self.k + diag as usize, t, d, y, m)
    }

    fn deco(
        &mut self,
        diag: bool,
        what: &str,
        t: (f32, f32),
        d: (f32, f32),
        y: (f32, f32),
        m: Mat,
    ) {
        let i = self.seg(diag, what, t, d, y, m);
        self.g.def.brushes[i].no_collide = true;
    }

    fn world(&self, diag: bool, t: f32, d: f32) -> (f32, f32) {
        at(self.k + diag as usize, t, d)
    }

    fn light(
        &mut self,
        diag: bool,
        what: &str,
        (t, d, y): (f32, f32, f32),
        color: [f32; 3],
        radius: f32,
    ) {
        let (x, z) = self.world(diag, t, d);
        let note = self.note(what);
        self.g.light(&note, [x, y, z], color, radius);
    }

    /// A jump pad (a 2.4 m square on the floor at side-local `(t, d)`, height `y`) that throws
    /// onto `to` (side-local `(t, d)`, height), arcing `over` metres above it.
    fn pad(
        &mut self,
        diag: bool,
        what: &str,
        (t, d, y): (f32, f32, f32),
        to: (f32, f32, f32),
        over: f32,
    ) {
        let (x, z) = self.world(diag, t, d);
        let (tx, tz) = self.world(diag, to.0, to.1);
        let v = pad_velocity([x, y, z], [tx, to.2, tz], over);
        let note = self.note(what);
        self.g.pad(
            &note,
            vol([x - 1.2, y, z - 1.2], [x + 1.2, y + 1.0, z + 1.2]),
            v,
        );
    }

    /// A player spawn facing the crucible.
    fn spawn(&mut self, diag: bool, (t, d, y): (f32, f32, f32)) {
        let (x, z) = self.world(diag, t, d);
        self.g.spawn([x, y, z], r4(f32::atan2(x, z).to_degrees()));
    }

    /// Iron jambs and a lintel round a bore mouth in a tier's front face at apothem `a`.
    fn bore_frame(&mut self, a: f32, f: f32) {
        use mat::*;
        for t in [(-BORE - 0.6, -BORE), (BORE, BORE + 0.6)] {
            self.seg(false, "bore jamb", t, (a - 0.3, a), (f, f + 5.6), IRON_DARK);
        }
        self.seg(
            false,
            "bore lintel (hazard)",
            (-BORE, BORE),
            (a - 0.3, a),
            (f + 5.0, f + 5.6),
            HAZARD,
        );
        self.light(false, "bore", (0.0, a + 4.0, f + 4.0), L_FURNACE, 8.0);
    }
}

pub(super) fn furnace_ring() -> Gen {
    use mat::*;
    let mut g = Gen::new(
        "Furnace Ring",
        LevelKind::Arena,
        Environment {
            sky_color: Vec3::new(0.04, 0.045, 0.085),
            fog_color: Vec3::new(0.075, 0.07, 0.1),
            fog_start: 45.0,
            fog_end: 190.0,
            ambient: Vec3::new(0.56, 0.55, 0.64),
            light_dir: Vec3::new(0.25, -1.0, 0.35),
            light_color: Vec3::new(0.6, 0.66, 0.86),
            music: "arena".into(),
        },
    );
    let (s1, s2, s3) = (half(A1), half(A2), half(A2 + T));
    // (Stacked chambers share their slabs, so floors and walls are the same stone.)
    let gate_style = GateStyle {
        wall: STONE,
        floor: STONE,
        trim: IRON_DARK,
        grate: GRATE,
        glow: SLAG,
        light: L_FURNACE,
    };

    g.section("The Great Crucible in its slag pool (centre)");
    g.bx(
        "slag pool bed",
        [-POOL - 0.5, -4.0, -POOL - 0.5],
        [POOL + 0.5, -3.0, POOL + 0.5],
        IRON_DARK,
    );
    g.bx(
        "white-hot slag",
        [-POOL, -3.0, -POOL],
        [POOL, -1.5, POOL],
        SLAG,
    );
    g.kill("slag pool", vol([-POOL, -4.0, -POOL], [POOL, -0.6, POOL]));
    // The crucible: an octagon (a cross of two boxes and four turned corner boxes), its
    // flue rising out of the top, which is a 3 m ring to stand on.
    let sc = half(CRU);
    g.bx("crucible", [-sc, -3.5, -CRU], [sc, CRU_TOP, CRU], FIREBRICK);
    g.bx("crucible", [-CRU, -3.5, -sc], [CRU, CRU_TOP, sc], FIREBRICK);
    for k in [1, 3, 5, 7] {
        seg(
            &mut g,
            "crucible",
            k,
            (-sc, sc),
            (0.0, CRU),
            (-3.5, CRU_TOP),
            FIREBRICK,
        );
    }
    let sf = half(FLUE);
    g.bx(
        "crucible flue",
        [-sf, CRU_TOP, -FLUE],
        [sf, FLUE_TOP, FLUE],
        IRON_DARK,
    );
    g.bx(
        "crucible flue",
        [-FLUE, CRU_TOP, -sf],
        [FLUE, FLUE_TOP, sf],
        IRON_DARK,
    );
    for k in [1, 3, 5, 7] {
        seg(
            &mut g,
            "crucible flue",
            k,
            (-sf, sf),
            (0.0, FLUE),
            (CRU_TOP, FLUE_TOP),
            IRON_DARK,
        );
    }
    for k in 0..8 {
        // Iron hoops round the crucible (proud of its faces), slag glowing through the
        // pour lips below the rim.
        let t = (-sc - 0.15, sc + 0.15);
        seg(
            &mut g,
            "crucible hoop",
            k,
            t,
            (CRU - 0.1, CRU + 0.25),
            (2.0, 2.6),
            IRON,
        );
        seg(
            &mut g,
            "crucible hoop",
            k,
            t,
            (CRU - 0.1, CRU + 0.25),
            (CRU_TOP - 0.6, CRU_TOP + 0.05),
            IRON,
        );
        let i = seg(
            &mut g,
            "crucible pour lip glow",
            k,
            (-1.0, 1.0),
            (CRU, CRU + 0.12),
            (6.0, 8.6),
            SLAG,
        );
        g.def.brushes[i].no_collide = true;
        seg(
            &mut g,
            "crucible pour lip grate",
            k,
            (-1.2, 1.2),
            (CRU + 0.12, CRU + 0.3),
            (5.8, 8.8),
            GRATE,
        );
        seg(
            &mut g,
            "flue band",
            k,
            (-sf - 0.1, sf + 0.1),
            (FLUE - 0.05, FLUE + 0.15),
            (FLUE_TOP - 1.0, FLUE_TOP - 0.4),
            FIREBRICK,
        );
    }
    g.light("crucible top", [0.0, CRU_TOP + 3.0, 0.0], L_WHITE, 14.0);
    for k in [0, 2, 4, 6] {
        let (x, z) = at(k, 0.0, 7.0);
        g.light("slag pool", [x, 0.5, z], L_WHITE, 9.0);
    }
    g.light("flue mouth", [0.0, FLUE_TOP + 1.0, 0.0], L_FURNACE, 10.0);

    for (q, name) in [(0, "N"), (2, "E"), (4, "S"), (6, "W")] {
        let mut h = Quarter {
            g: &mut g,
            k: q,
            name,
        };
        h.g.section(&format!(
            "{name} quarter (axis side {name} and the diagonal after it)"
        ));

        // Masonry, as (diagonal side?, what, t, d, y, material): the pit floor (out to the
        // outer wall, under everything); tier 1 (y 7) with the floor bore and a spawn alcove on
        // the axis side; tier 2 (the rim, y 14) with both bores, a spawn alcove and a wall-jump
        // slot; the outer wall with three gate openings on the axis side and a spawn alcove in
        // the diagonal side. Axis-side pieces stand on the floor; diagonal ones dip into it.
        let (ta, tb) = ALCOVE;
        let (sa, sb) = SLOT;
        let sf = half(A2 + 0.5);
        let (a3, a4) = (A2 + T, A2 + T + ALCOVE_D);
        let (ry0, ry1) = (Y2 - 1.0, Y2 + ALCOVE_H + 1.0);
        let (c0, c1) = (Y2 - 0.5, Y2 + 0.05);
        #[rustfmt::skip]
        let pieces = [
            (false, "floor", (-sf, sf), (POOL, A2 + 0.5), (BASE, 0.0), ASH),
            (true, "floor", (-sf, sf), (POOL, A2 + 0.5), (BASE, 0.0), ASH),
            (false, "tier 1", (-s1, ta), (A0, A1), (0.0, Y1), STONE),
            (false, "tier 1 behind the alcove", (ta, tb), (A0 + ALCOVE_D, A1), (0.0, Y1), STONE),
            (false, "tier 1 over the alcove", (ta, tb), (A0, A0 + ALCOVE_D), (ALCOVE_H, Y1), STONE),
            (false, "tier 1", (tb, -BORE), (A0, A1), (0.0, Y1), STONE),
            (false, "tier 1 over the floor bore", (-BORE, BORE), (A0, A1), (5.0, Y1), STONE),
            (false, "tier 1", (BORE, s1), (A0, A1), (0.0, Y1), STONE),
            (true, "tier 1", (-s1, s1), (A0, A1), (-1.0, Y1), STONE),
            (false, "tier 2", (-s2, ta), (A1, A2), (0.0, Y2), STONE),
            (false, "tier 2 behind the alcove", (ta, tb), (A1 + ALCOVE_D, A2), (0.0, Y2), STONE),
            (false, "tier 2 under the alcove", (ta, tb), (A1, A1 + ALCOVE_D), (0.0, Y1), STONE),
            (false, "tier 2 over the alcove", (ta, tb), (A1, A1 + ALCOVE_D), (Y1 + ALCOVE_H, Y2), STONE),
            (false, "tier 2", (tb, -BORE), (A1, A2), (0.0, Y2), STONE),
            (false, "tier 2 between the bores", (-BORE, BORE), (A1, A2), (5.0, Y1), STONE),
            (false, "tier 2 over the tier-1 bore", (-BORE, BORE), (A1, A2), (Y1 + 5.0, Y2), STONE),
            (false, "tier 2", (BORE, sa), (A1, A2), (0.0, Y2), STONE),
            (false, "tier 2 behind the wall-jump slot", (sa, sb), (A1 + SLOT_D, A2), (0.0, Y2), STONE),
            (false, "tier 2 under the wall-jump slot", (sa, sb), (A1, A1 + SLOT_D), (0.0, Y1), STONE),
            (false, "tier 2", (sb, s2), (A1, A2), (0.0, Y2), STONE),
            (true, "tier 2", (-s2, s2), (A1, A2), (-1.0, Y2), STONE),
            // Iron coping along the axis side's tier edges (the slot breaks the tier 2 one; the
            // diagonal faces have ramps along them).
            (false, "tier 1 coping", (-half(A0), half(A0)), (A0 - 0.3, A0 + 0.6), (Y1 - 0.5, Y1 + 0.05), IRON),
            (false, "tier 2 coping", (-half(A1), sa - 0.1), (A1 - 0.3, A1 + 0.6), (c0, c1), IRON),
            (false, "tier 2 coping", (sb + 0.1, half(A1)), (A1 - 0.3, A1 + 0.6), (c0, c1), IRON),
            (false, "outer wall", (-s3, -BORE), (A2, a3), (BASE - 1.0, TOP), STONE),
            (false, "outer wall", (BORE, s3), (A2, a3), (BASE - 1.0, TOP), STONE),
            (false, "floor (the bore through the outer wall)", (-BORE, BORE), (A2, a3), (BASE - 1.0, 0.0), ASH),
            (false, "outer wall between the gates", (-BORE, BORE), (A2, a3), (5.0, Y1), STONE),
            (false, "outer wall between the gates", (-BORE, BORE), (A2, a3), (Y1 + 5.0, Y2), STONE),
            (false, "outer wall over the rim gate", (-BORE, BORE), (A2, a3), (Y2 + 5.0, TOP), STONE),
            (true, "outer wall", (-s3, -1.5), (A2, a3), (BASE - 1.0, TOP), STONE),
            (true, "outer wall", (1.5, s3), (A2, a3), (BASE - 1.0, TOP), STONE),
            (true, "outer wall under the rim alcove", (-1.5, 1.5), (A2, a3), (BASE - 1.0, Y2), STONE),
            (true, "outer wall over the rim alcove", (-1.5, 1.5), (A2, a3), (Y2 + ALCOVE_H, TOP), STONE),
            (true, "rim alcove floor", (-1.5, 1.5), (a3, a4), (Y2 - 1.0, Y2), STONE),
            (true, "rim alcove back", (-2.5, 2.5), (a4, a4 + T), (ry0, ry1), STONE),
            (true, "rim alcove side", (-2.5, -1.5), (a3, a4), (ry0, ry1), STONE),
            (true, "rim alcove side", (1.5, 2.5), (a3, a4), (ry0, ry1), STONE),
            (true, "rim alcove roof", (-2.5, 2.5), (a3, a4 + T), (ry1 - 1.0, ry1), STONE),
            // A firebrick band along the foot of the outer wall over the rim (broken by the rim
            // gate and the rim alcove).
            (false, "outer wall band", (-s3, -BORE - 0.6), (A2 - 0.25, A2), (Y2, Y2 + 0.6), FIREBRICK),
            (false, "outer wall band", (BORE + 0.6, s3), (A2 - 0.25, A2), (Y2, Y2 + 0.6), FIREBRICK),
            (true, "outer wall band", (-s3, -1.5), (A2 - 0.25, A2), (Y2, Y2 + 0.6), FIREBRICK),
            (true, "outer wall band", (1.5, s3), (A2 - 0.25, A2), (Y2, Y2 + 0.6), FIREBRICK),
        ];
        for (diag, what, t, d, y, m) in pieces {
            h.seg(diag, what, t, d, y, m);
        }
        h.bore_frame(A0, 0.0);
        h.bore_frame(A1, Y1);

        // Furnace vents high in the outer wall (slag glowing behind grates); the sky clip.
        for diag in [false, true] {
            for t in [-9.0, 9.0] {
                let (v0, v1) = (Y2 + 6.5, Y2 + 8.5);
                h.deco(
                    diag,
                    "furnace vent glow",
                    (t - 1.6, t + 1.6),
                    (A2 - 0.1, A2 - 0.02),
                    (v0, v1),
                    SLAG,
                );
                let (v0, v1) = (v0 - 0.2, v1 + 0.2);
                h.seg(
                    diag,
                    "furnace vent grate",
                    (t - 1.8, t + 1.8),
                    (A2 - 0.3, A2 - 0.1),
                    (v0, v1),
                    GRATE,
                );
            }
            let i = h.seg(
                diag,
                "sky clip wall",
                (-s3, s3),
                (A2, a3),
                (TOP, SKY),
                STONE,
            );
            h.g.def.brushes[i].clip_only = true;
        }
        h.light(
            true,
            "furnace vents",
            (0.0, A2 - 3.0, Y2 + 7.5),
            L_FURNACE,
            14.0,
        );
        h.light(
            false,
            "furnace vents",
            (9.0, A2 - 3.0, Y2 + 7.5),
            L_FURNACE,
            11.0,
        );

        // Ramps on the diagonal side (the pinwheel): floor -> tier 1 along the tier 1 face,
        // tier 1 -> rim along the tier 2 face; both rise towards this quarter's axis side.
        let k1 = h.k + 1;
        let note = h.note("ramp floor -> tier 1");
        side_ramp(
            h.g,
            &note,
            k1,
            (half(A0), 0.0),
            (-half(A0), Y1),
            (A0 - 5.0, A0),
            9.0,
            STONE_DARK,
        );
        let note = h.note("ramp tier 1 -> rim");
        side_ramp(
            h.g,
            &note,
            k1,
            (10.5, Y1),
            (-10.5, Y2),
            (A1 - 5.0, A1),
            9.0,
            STONE_DARK,
        );
        h.light(true, "floor ramp", (0.0, A0 - 6.0, 5.0), L_SKY, 10.0);

        // Pads: floor -> tier 1 in front of the floor ramp (over it), floor -> the
        // crucible's top in front of the floor bore.
        h.pad(
            true,
            "floor -> tier 1 (over the ramp)",
            (0.0, 12.2, 0.0),
            (0.0, 25.0, Y1),
            3.0,
        );
        h.pad(
            false,
            "floor -> crucible top",
            (0.0, 11.2, 0.0),
            (0.0, 3.5, CRU_TOP),
            2.5,
        );

        // Pig-iron stacks in front of the tier-1 and rim alcoves: cover for whoever spawns
        // there, and they cut the long diagonals between alcoves across the pit.
        let (p0, p1) = (Y1, Y1 + 2.5);
        h.seg(
            false,
            "pig-iron stack",
            (ta - 2.0, tb),
            (A1 - 5.5, A1 - 3.5),
            (p0, p1),
            IRON_DARK,
        );
        let (p0, p1) = (Y2, Y2 + 2.5);
        h.seg(
            true,
            "pig-iron stack",
            (-3.0, 3.0),
            (A2 - 4.5, A2 - 2.5),
            (p0, p1),
            IRON_DARK,
        );

        // Spawn alcoves (lanterns) and spawns: pit floor, tier 1, rim.
        let tm = (ta + tb) * 0.5;
        h.light(false, "floor alcove", (tm, A0 + 1.5, 2.8), L_FURNACE, 5.0);
        h.light(
            false,
            "tier 1 alcove",
            (tm, A1 + 1.5, Y1 + 2.8),
            L_FURNACE,
            5.0,
        );
        h.light(
            true,
            "rim alcove",
            (0.0, a3 + 1.5, Y2 + 2.8),
            L_FURNACE,
            5.0,
        );
        h.light(false, "tier 1", (8.0, A0 + 5.0, Y1 + 3.0), L_SKY, 12.0);
        h.light(false, "rim", (-10.0, A1 + 5.0, Y2 + 3.0), L_SKY, 12.0);
        h.spawn(false, (tm, A0 + 1.6, 0.0));
        h.spawn(false, (tm, A1 + 1.6, Y1));
        h.spawn(true, (0.0, a3 + 1.6, Y2));

        // The three Mould gates behind the outer wall (stacked; their slabs coincide).
        let side = [Side::N, Side::E, Side::S, Side::W][q / 2];
        let wall = if q == 0 || q == 6 { -A2 } else { A2 };
        let floor_gate = Gate::new(side, wall, 0.0, 0.0).no_frame();
        let tier_gate = Gate::new(side, wall, 0.0, Y1).no_frame();
        let rim_gate = Gate::new(side, wall, 0.0, Y2);
        floor_gate.build(
            h.g,
            &format!("{name} floor gate"),
            &gate_style,
            Spawns::Corners,
        );
        tier_gate.build(
            h.g,
            &format!("{name} tier-1 gate"),
            &gate_style,
            Spawns::Middle,
        );
        rim_gate.build(
            h.g,
            &format!("{name} rim gate"),
            &gate_style,
            Spawns::Middle,
        );
    }

    g.section("Sky clip");
    g.clip(
        "sky clip ceiling",
        [-A2 - 16.0, SKY, -A2 - 16.0],
        [A2 + 16.0, SKY + 1.0, A2 + 16.0],
    );
    g.kill(
        "below the world",
        vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]),
    );
    g
}
