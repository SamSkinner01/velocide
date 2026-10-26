//! Shared pieces for the arenas: **Mould gates** (where grind waves pour in) and a jump-pad
//! solver.
//!
//! A Mould gate is a 5 × 5 m opening in an arena's outer wall leading into a chamber behind
//! it. A baffle wall stands 5 m behind the opening, so from anywhere in the arena you see the
//! baffle, never the chamber's back corners and the strip behind the baffle where the grind
//! spawns are: Moulds appear out of sight and come round the baffle into the arena. The
//! chamber is a loop (round the baffle on either side), 6 m tall and with 5 m wide passages,
//! so every Mould fits, the Gaoler included. See `docs/levels.md` ("Grind spawns").

use super::*;

/// Opening width and height.
pub const GATE_W: f32 = 5.0;
pub const GATE_H: f32 = 5.0;
/// Chamber half width (along the wall), depth (out from the wall's outer face), height.
const HALF: f32 = 12.0;
const DEPTH: f32 = 12.0;
const HEIGHT: f32 = 6.0;
/// Baffle half width, its distance behind the wall's outer face, its thickness.
const BAFFLE_HALF: f32 = 7.0;
const BAFFLE_AT: f32 = 5.0;
const BAFFLE_T: f32 = 2.0;

/// Which grind spawns a gate registers: the chamber's two back corners, the middle of the
/// strip behind the baffle, or all three.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Spawns {
    Corners,
    Middle,
    All,
}

/// Materials and light of a level's gates.
pub struct GateStyle {
    pub wall: Mat,
    pub floor: Mat,
    /// Frame round the opening (arena side).
    pub trim: Mat,
    pub grate: Mat,
    /// The glowing pour spout on the back wall (behind a grate).
    pub glow: Mat,
    pub light: [f32; 3],
}

/// A Mould gate in one wall of an arena's shell.
#[derive(Clone, Copy)]
pub struct Gate {
    pub side: Side,
    /// Inner face of that wall (z for N/S walls, x for W/E walls).
    pub wall: f32,
    /// Centre of the opening along the wall.
    pub at: f32,
    /// Floor height (the opening's sill).
    pub floor: f32,
    /// Frame the opening on the arena side (off when a tunnel leads up to the wall).
    pub frame: bool,
}

impl Gate {
    pub fn new(side: Side, wall: f32, at: f32, floor: f32) -> Self {
        Self {
            side,
            wall,
            at,
            floor,
            frame: true,
        }
    }

    /// Without the frame on the arena side (the gate sits at the end of a tunnel).
    pub fn no_frame(mut self) -> Self {
        self.frame = false;
        self
    }

    /// The hole in the shell wall (add it to the shell `Room` before building it).
    pub fn opening(&self) -> Opening {
        op(
            self.side,
            self.at - GATE_W * 0.5,
            self.at + GATE_W * 0.5,
            self.floor,
            self.floor + GATE_H,
        )
    }

    /// Local `(u, v)` (u along the wall, v outward from the wall's outer face) to world `(x, z)`.
    fn world(&self, u: f32, v: f32) -> (f32, f32) {
        match self.side {
            Side::N => (self.at + u, self.wall - T - v),
            Side::S => (self.at - u, self.wall + T + v),
            Side::W => (self.wall - T - v, self.at - u),
            Side::E => (self.wall + T + v, self.at + u),
        }
    }

    /// World AABB of the local box `u0..u1` × `y0..y1` × `v0..v1`.
    fn corners(
        &self,
        (u0, u1): (f32, f32),
        (y0, y1): (f32, f32),
        (v0, v1): (f32, f32),
    ) -> ([f32; 3], [f32; 3]) {
        let (ax, az) = self.world(u0, v0);
        let (bx, bz) = self.world(u1, v1);
        ([ax.min(bx), y0, az.min(bz)], [ax.max(bx), y1, az.max(bz)])
    }

    fn bx(
        &self,
        g: &mut Gen,
        note: &str,
        u: (f32, f32),
        y: (f32, f32),
        v: (f32, f32),
        m: Mat,
    ) -> usize {
        let (a, b) = self.corners(u, y, v);
        g.bx(note, a, b, m);
        g.def.brushes.len() - 1
    }

    /// A point on the chamber floor.
    pub fn point(&self, u: f32, v: f32) -> [f32; 3] {
        let (x, z) = self.world(u, v);
        [x, self.floor, z]
    }

    /// Builds the chamber behind the opening and registers its grind spawns.
    pub fn build(&self, g: &mut Gen, name: &str, s: &GateStyle, spawns: Spawns) {
        let f = self.floor;
        let (w, d, h) = (HALF, DEPTH, HEIGHT);
        g.section(&format!("{name} (Mould gate: grind spawns)"));
        let n = |what: &str| format!("{name} {what}");
        self.bx(g, &n("floor"), (-w, w), (f - T, f), (0.0, d), s.floor);
        self.bx(
            g,
            &n("ceiling"),
            (-w - T, w + T),
            (f + h, f + h + T),
            (0.0, d + T),
            s.wall,
        );
        self.bx(
            g,
            &n("back wall"),
            (-w - T, w + T),
            (f - T, f + h),
            (d, d + T),
            s.wall,
        );
        self.bx(
            g,
            &n("end wall"),
            (-w - T, -w),
            (f - T, f + h),
            (0.0, d),
            s.wall,
        );
        self.bx(
            g,
            &n("end wall"),
            (w, w + T),
            (f - T, f + h),
            (0.0, d),
            s.wall,
        );
        // The chamber's own front wall either side of the opening, in the plane of the shell
        // wall (same material, so where the shell wall is there too they coincide): a gate
        // near a corner may reach past the end of the shell's wall.
        for (u0, u1) in [(-w - T, -GATE_W * 0.5), (GATE_W * 0.5, w + T)] {
            self.bx(
                g,
                &n("front wall"),
                (u0, u1),
                (f - T, f + h + T),
                (-T, 0.0),
                s.wall,
            );
        }
        self.bx(
            g,
            &n("baffle (hides the chamber from the arena)"),
            (-BAFFLE_HALF, BAFFLE_HALF),
            (f, f + h),
            (BAFFLE_AT, BAFFLE_AT + BAFFLE_T),
            s.wall,
        );
        // A molten grate in the baffle's face, seen from the arena through the opening: the
        // sign of a Mould gate.
        let (b0, b1) = (BAFFLE_AT - 0.1, BAFFLE_AT - 0.02);
        let i = self.bx(
            g,
            &n("baffle glow"),
            (-1.6, 1.6),
            (f + 1.0, f + 3.6),
            (b0, b1),
            s.glow,
        );
        g.def.brushes[i].no_collide = true;
        let (b0, b1) = (BAFFLE_AT - 0.3, BAFFLE_AT - 0.1);
        self.bx(
            g,
            &n("baffle grate"),
            (-1.8, 1.8),
            (f + 0.8, f + 3.8),
            (b0, b1),
            s.grate,
        );
        // The pour spout on the back wall: molten glow behind a grate.
        let i = self.bx(
            g,
            &n("pour spout glow"),
            (-1.2, 1.2),
            (f + 1.0, f + 4.0),
            (d - 0.15, d - 0.05),
            s.glow,
        );
        g.def.brushes[i].no_collide = true;
        self.bx(
            g,
            &n("pour spout grate"),
            (-1.4, 1.4),
            (f + 0.8, f + 4.2),
            (d - 0.4, d - 0.15),
            s.grate,
        );
        // Frame round the opening on the arena side, and a raised portcullis under the lintel.
        let (a, t) = (GATE_W * 0.5, -T);
        if self.frame {
            for (u0, u1) in [(-a - 0.6, -a), (a, a + 0.6)] {
                self.bx(
                    g,
                    &n("jamb"),
                    (u0, u1),
                    (f, f + GATE_H + 0.6),
                    (t - 0.3, t),
                    s.trim,
                );
            }
            self.bx(
                g,
                &n("lintel"),
                (-a, a),
                (f + GATE_H, f + GATE_H + 0.6),
                (t - 0.3, t),
                s.trim,
            );
        }
        let i = self.bx(
            g,
            &n("raised portcullis"),
            (-a, a),
            (f + GATE_H - 0.5, f + GATE_H),
            (-0.7, -0.3),
            s.grate,
        );
        g.def.brushes[i].no_collide = true;
        let glow = self.point(0.0, d - 1.5);
        g.light(&n("pour spout"), [glow[0], f + 3.0, glow[2]], s.light, 12.0);
        let mouth = self.point(0.0, 1.5);
        g.light(
            &n("mouth"),
            [mouth[0], f + 4.0, mouth[2]],
            s.light.map(|c| c * 0.6),
            7.0,
        );

        let corners = [
            self.point(-(w - 2.5), d - 2.5),
            self.point(w - 2.5, d - 2.5),
        ];
        let middle = self.point(0.0, d - 2.5);
        let points = match spawns {
            Spawns::Corners => corners.to_vec(),
            Spawns::Middle => vec![middle],
            Spawns::All => vec![corners[0], middle, corners[1]],
        };
        for p in points {
            g.grind(name, p);
        }
    }
}

/// Jump-pad velocity that throws a player from `from` (feet) to land on `to` with no input,
/// peaking `over` metres above the landing height.
pub fn pad_velocity(from: [f32; 3], to: [f32; 3], over: f32) -> [f32; 3] {
    let g = shared::tuning::MovementTuning::default().gravity;
    let rise = to[1] - from[1] + over;
    let vy = (2.0 * g * rise).sqrt();
    let t = vy / g + (2.0 * over / g).sqrt();
    let r = |x: f32| (x * 100.0).round() / 100.0;
    [r((to[0] - from[0]) / t), r(vy), r((to[2] - from[2]) / t)]
}
