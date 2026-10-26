//! Wickmarket: an open PvP arena with long sightlines (4–8 players) and a grind arena.
//!
//! The Foundry's candle market under a smoky red sky: the guildhall's arcade along the north
//! side with the guild walk on its roof (y 7), chandlers' shophouses along the west and east
//! sides (shop alcoves at the market, flat roofs at y 4), rows of stalls whose awnings make a
//! broken high road (y 3.6), the bell gallows in the middle (a pad up onto its beam, y 14), and
//! on the south a balustrade with gaps over a gorge of molten slag, two jetties sticking out over
//! it. Long north-south sightlines from the guild walk to the jetties for the bow; the stalls,
//! the gallows and its bell break them at ground level. Mirror-symmetric west ↔ east. Layout
//! map, routes and spawns: `docs/levels.md`.

use super::gates::{Gate, GateStyle, Spawns, pad_velocity};
use super::*;

mod mat {
    use super::Mat;
    /// Guild sandstone, its paler trim, cobbles, the gorge's rock.
    pub const SANDSTONE: Mat = Mat("concrete", [0.86, 0.74, 0.6]);
    pub const SANDSTONE_PALE: Mat = Mat("concrete", [0.95, 0.88, 0.76]);
    pub const COBBLES: Mat = Mat("tile", [0.68, 0.62, 0.58]);
    pub const ROCK: Mat = Mat("concrete", [0.42, 0.36, 0.34]);
    pub const BRICK: Mat = Mat("brick", [0.8, 0.62, 0.52]);
    pub const TIMBER: Mat = Mat("wood", [0.7, 0.56, 0.42]);
    pub const TIMBER_DARK: Mat = Mat("wood", [0.46, 0.38, 0.32]);
    pub const BRONZE: Mat = Mat("metal", [0.95, 0.7, 0.42]);
    pub const IRON: Mat = Mat("metal", [0.44, 0.42, 0.44]);
    pub const WAX: Mat = Mat("wax", [1.0, 0.95, 0.82]);
    pub const WAXSTONE: Mat = Mat("waxstone", [0.92, 0.86, 0.74]);
    pub const GRATE: Mat = Mat("grate", [0.7, 0.62, 0.56]);
    /// Molten slag in the gorge (always has a kill volume) and candle flames.
    pub const SLAG: Mat = Mat("lava", [1.0, 0.75, 0.55]);
}

/// Stall awnings: madder, indigo, saffron, verdigris canvas.
const CANVAS: [[f32; 3]; 4] = [
    [0.86, 0.36, 0.3],
    [0.36, 0.42, 0.78],
    [1.0, 0.78, 0.36],
    [0.4, 0.76, 0.6],
];

/// Candle gold, the slag's glow, the low red sun through the smoke.
const L_CANDLE: [f32; 3] = [1.25, 0.95, 0.6];
const L_SLAG: [f32; 3] = [1.35, 0.5, 0.2];
const L_SUN: [f32; 3] = [1.0, 0.62, 0.48];

/// Interior half size (88 × 68 m), wall height, clip ceiling, the gorge's bed.
const HX: f32 = 44.0;
const HZ: f32 = 34.0;
const TOP: f32 = 12.0;
const SKY: f32 = 30.0;
const GORGE: f32 = -15.0;
/// The market's south edge (the balustrade), the arcade's front, the guild walk's height.
const EDGE: f32 = 22.0;
const ARCADE: f32 = -26.0;
const WALK: f32 = 7.0;
/// Shophouses (east side; mirrored): inner face, roof height, south end.
const SHOP_X: f32 = 36.0;
const ROOF: f32 = 4.0;
const SHOP_S: f32 = 14.0;
/// The gallows' beam.
const BEAM: f32 = 14.2;

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
    fn ramp_x(
        &mut self,
        what: &str,
        z: (f32, f32),
        a: (f32, f32),
        b: (f32, f32),
        thick: f32,
        m: Mat,
    ) {
        let note = self.note(what);
        let s = self.s;
        self.g
            .ramp_x(&note, z, (s * a.0, a.1), (s * b.0, b.1), thick, m);
    }

    /// A jump pad on the floor at `from` throwing onto `to`, arcing `over` above it.
    fn pad(&mut self, what: &str, from: [f32; 3], to: [f32; 3], over: f32) {
        let s = self.s;
        let fx = s * from[0];
        let v = pad_velocity([fx, from[1], from[2]], [s * to[0], to[1], to[2]], over);
        let note = self.note(what);
        let (z0, z1) = (from[2] - 1.2, from[2] + 1.2);
        self.g.pad(
            &note,
            vol([fx - 1.2, from[1], z0], [fx + 1.2, from[1] + 1.0, z1]),
            v,
        );
    }

    fn light(&mut self, what: &str, [x, y, z]: [f32; 3], color: [f32; 3], radius: f32) {
        let note = self.note(what);
        self.g.light(&note, [self.s * x, y, z], color, radius);
    }

    /// A player spawn facing the middle of the market.
    fn spawn(&mut self, [x, y, z]: [f32; 3]) {
        let x = self.s * x;
        self.g.spawn([x, y, z], r4(f32::atan2(x, z).to_degrees()));
    }

    /// A market stall facing the gallows: its counter's front at `front` (x), centred on `z`,
    /// with a back board, two posts and a canvas awning (y 3.6) a little wider than the stall.
    fn stall(&mut self, front: f32, z: f32, canvas: [f32; 3], goods: bool) {
        let (z0, z1) = (z - 2.5, z + 2.5);
        self.bx(
            "stall counter",
            [front, 0.0, z0],
            [front + 1.2, 1.1, z1],
            TIMBER_COUNTER,
        );
        self.bx(
            "stall back board",
            [front + 2.7, 0.0, z0],
            [front + 3.0, 2.8, z1],
            mat::TIMBER_DARK,
        );
        for (a, b) in [(z0, z0 + 0.25), (z1 - 0.25, z1)] {
            self.bx(
                "stall post",
                [front, 1.1, a],
                [front + 0.25, 3.4, b],
                mat::TIMBER_DARK,
            );
        }
        self.bx(
            "stall awning",
            [front - 1.0, 3.4, z0 - 0.2],
            [front + 3.2, 3.6, z1 + 0.2],
            Mat("panel", canvas),
        );
        if goods {
            for dz in [-1.6, 0.0, 1.6] {
                self.deco(
                    "stall candles",
                    [front + 0.3, 1.1, z + dz - 0.15],
                    [front + 0.6, 1.9, z + dz + 0.15],
                    mat::WAX,
                );
            }
        }
    }
}

/// Stall counters (a shade lighter than the timber elsewhere).
const TIMBER_COUNTER: Mat = Mat("wood", [0.78, 0.62, 0.46]);

pub(super) fn wickmarket() -> Gen {
    use mat::*;
    let mut g = Gen::new(
        "Wickmarket",
        LevelKind::Arena,
        Environment {
            sky_color: Vec3::new(0.42, 0.26, 0.24),
            fog_color: Vec3::new(0.38, 0.29, 0.28),
            fog_start: 35.0,
            fog_end: 175.0,
            ambient: Vec3::new(0.56, 0.5, 0.5),
            light_dir: Vec3::new(0.7, -0.45, -0.55),
            light_color: Vec3::new(0.95, 0.6, 0.46),
            music: "arena".into(),
        },
    );

    // Mould gates (grind spawns): in the north wall under the guild walk, and in the west and
    // east walls between the shophouses and the balustrade.
    let mould_gates = [
        ("N gate", Gate::new(Side::N, -HZ, 0.0, 0.0)),
        ("W gate", Gate::new(Side::W, -HX, 18.0, 0.0)),
        ("E gate", Gate::new(Side::E, HX, 18.0, 0.0)),
    ];
    g.section("Outer shell (open sky, clipped at 30 m)");
    let mut shell = Room::new("Market", [-HX, 0.0, -HZ], [HX, TOP, HZ], SANDSTONE).bottom(GORGE);
    for (_, gate) in &mould_gates {
        shell = shell.opening(gate.opening());
    }
    g.room(&shell);
    g.clip(
        "sky clip wall N",
        [-HX - 1.0, TOP, -HZ - 1.0],
        [HX + 1.0, SKY, -HZ],
    );
    g.clip(
        "sky clip wall S",
        [-HX - 1.0, TOP, HZ],
        [HX + 1.0, SKY, HZ + 1.0],
    );
    g.clip("sky clip wall W", [-HX - 1.0, TOP, -HZ], [-HX, SKY, HZ]);
    g.clip("sky clip wall E", [HX, TOP, -HZ], [HX + 1.0, SKY, HZ]);
    // (Wide enough to cover the gate chambers behind the walls too.)
    g.clip(
        "sky clip ceiling",
        [-HX - 16.0, SKY, -HZ - 16.0],
        [HX + 16.0, SKY + 1.0, HZ + 16.0],
    );
    let gate_style = GateStyle {
        wall: SANDSTONE,
        floor: COBBLES,
        trim: BRONZE,
        grate: GRATE,
        glow: SLAG,
        light: L_SLAG,
    };
    for (name, gate) in &mould_gates {
        gate.build(&mut g, name, &gate_style, Spawns::All);
    }
    g.bx(
        "wall coping",
        [-HX - 1.0, TOP, -HZ - 1.0],
        [HX + 1.0, TOP + 0.4, -HZ + 0.3],
        SANDSTONE_PALE,
    );

    g.section("The market floor, the gorge of slag on the south, the balustrade");
    g.bx(
        "market floor (bedrock)",
        [-HX, GORGE, -HZ],
        [HX, -0.2, EDGE],
        ROCK,
    );
    g.bx("cobbles", [-HX, -0.2, -HZ], [HX, 0.0, EDGE], COBBLES);
    g.bx("gorge bed", [-HX, GORGE, EDGE], [HX, GORGE + 1.0, HZ], ROCK);
    g.bx(
        "molten slag",
        [-HX, GORGE + 1.0, EDGE],
        [HX, GORGE + 2.0, HZ],
        SLAG,
    );
    g.kill(
        "the gorge (molten slag)",
        vol([-HX, GORGE, EDGE], [HX, GORGE + 4.0, HZ]),
    );
    g.bx(
        "balustrade",
        [-14.0, 0.0, EDGE - 0.5],
        [14.0, 1.0, EDGE],
        WAXSTONE,
    );
    for x in [-36.0, -26.0, -14.0, 14.0, 26.0, 36.0] {
        candles_post(&mut g, x);
    }
    g.light("gorge", [0.0, -6.0, 28.0], L_SLAG, 20.0);

    g.section("The bell gallows (centre)");
    g.bx(
        "gallows dais",
        [-7.0, 0.0, -5.0],
        [7.0, 0.9, 5.0],
        SANDSTONE_PALE,
    );
    g.bx(
        "gallows step N",
        [-3.0, 0.0, -6.5],
        [3.0, 0.45, -5.0],
        SANDSTONE,
    );
    g.bx(
        "gallows step S",
        [-3.0, 0.0, 5.0],
        [3.0, 0.45, 6.5],
        SANDSTONE,
    );
    for x in [-5.2f32, 4.0] {
        g.bx(
            "gallows upright",
            [x, 1.5, -0.6],
            [x + 1.2, 13.0, 0.6],
            TIMBER_DARK,
        );
        g.bx(
            "gallows foot",
            [x - 0.4, 0.9, -1.0],
            [x + 1.6, 1.5, 1.0],
            IRON,
        );
    }
    g.bx(
        "gallows beam",
        [-6.0, 13.0, -0.7],
        [6.0, BEAM, 0.7],
        TIMBER_DARK,
    );
    g.deco("bell rope", [-0.1, 12.4, -0.1], [0.1, 13.0, 0.1], TIMBER);
    for (r, y0, y1) in [
        (0.4, 11.8, 12.4),
        (1.0, 11.0, 11.8),
        (1.3, 9.0, 11.0),
        (1.6, 8.2, 9.0),
        (1.8, 7.8, 8.2),
    ] {
        g.bx("the market bell", [-r, y0, -r], [r, y1, r], BRONZE);
    }
    g.light("gallows", [0.0, 6.5, 0.0], L_CANDLE, 14.0);
    g.light("gallows beam", [0.0, BEAM + 3.0, 0.0], L_SUN, 10.0);

    for (s, name) in [(1.0f32, "E"), (-1.0, "W")] {
        let mut h = Half { g: &mut g, s, name };
        h.g.section(&format!(
            "{name} half: guild walk, shophouses, stalls, jetty"
        ));

        // The guildhall arcade: the guild walk on its roof (y 7) with a parapet, arcade
        // pillars, the guild stair down to the market, a kiosk at the end of the walk.
        h.bx(
            "guild walk",
            [0.0, WALK - 0.6, -HZ],
            [HX, WALK, ARCADE],
            SANDSTONE_PALE,
        );
        h.bx(
            "guild walk parapet",
            [0.0, WALK, ARCADE - 0.6],
            [19.5, WALK + 1.1, ARCADE],
            SANDSTONE,
        );
        for x in [4.0, 12.0, 20.0, 28.0] {
            h.bx(
                "arcade pillar",
                [x - 0.6, 0.0, ARCADE - 1.2],
                [x + 0.6, WALK - 0.6, ARCADE],
                SANDSTONE,
            );
        }
        h.ramp_x(
            "guild stair",
            (ARCADE, ARCADE + 5.0),
            (20.0, 0.0),
            (36.0, WALK),
            9.0,
            SANDSTONE,
        );
        // The kiosk on the walk (a spawn): walls, a roof, a door to the south.
        let (kx0, kx1, kz0, kz1) = (27.0, 33.0, -HZ, -29.0);
        h.bx(
            "walk kiosk",
            [kx0, WALK, kz0],
            [kx0 + 0.4, WALK + 3.6, kz1],
            BRICK,
        );
        h.bx(
            "walk kiosk",
            [kx1 - 0.4, WALK, kz0],
            [kx1, WALK + 3.6, kz1],
            BRICK,
        );
        h.bx(
            "walk kiosk",
            [kx0 + 0.4, WALK, kz1 - 0.4],
            [29.0, WALK + 3.6, kz1],
            BRICK,
        );
        h.bx(
            "walk kiosk",
            [31.0, WALK, kz1 - 0.4],
            [kx1 - 0.4, WALK + 3.6, kz1],
            BRICK,
        );
        h.bx(
            "walk kiosk lintel",
            [29.0, WALK + 2.6, kz1 - 0.4],
            [31.0, WALK + 3.6, kz1],
            BRICK,
        );
        h.bx(
            "walk kiosk roof",
            [kx0 - 0.3, WALK + 3.6, kz0],
            [kx1 + 0.3, WALK + 4.0, kz1 + 0.3],
            TIMBER_DARK,
        );
        h.light("walk kiosk", [30.0, WALK + 2.5, -31.5], L_CANDLE, 5.0);
        h.light("arcade", [12.0, 4.5, -30.0], L_CANDLE, 12.0);
        h.light("guild walk", [10.0, WALK + 4.0, -29.0], L_SUN, 14.0);
        h.pad(
            "market -> guild walk",
            [8.0, 0.0, -19.0],
            [8.0, WALK, -31.0],
            2.5,
        );

        // The shophouse: a row of candle shops (alcoves at the market, spawns) under a flat
        // roof (y 4) with chimneys; a ramp up to the roof from the south.
        let (x0, x1) = (SHOP_X, HX);
        let alcoves = [-20.0f32, -8.0, 4.0];
        let mut z = ARCADE;
        for c in alcoves {
            h.bx(
                "shophouse front",
                [x0, 0.0, z],
                [x0 + 3.0, ROOF - 0.2, c - 2.0],
                BRICK,
            );
            h.bx(
                "shop (over the alcove)",
                [x0, 3.2, c - 2.0],
                [x0 + 3.0, ROOF - 0.2, c + 2.0],
                BRICK,
            );
            h.bx(
                "shop awning",
                [x0 - 1.4, 3.0, c - 2.2],
                [x0, 3.2, c + 2.2],
                TIMBER,
            );
            h.deco(
                "shop sign (painted)",
                [x0 - 0.1, 3.3, c - 1.2],
                [x0, 3.9, c + 1.2],
                Mat("panel", CANVAS[((c + 20.0) / 12.0) as usize % 4]),
            );
            h.light("candle shop", [x0 + 1.5, 2.4, c], L_CANDLE, 5.0);
            z = c + 2.0;
        }
        h.bx(
            "shophouse front",
            [x0, 0.0, z],
            [x0 + 3.0, ROOF - 0.2, SHOP_S],
            BRICK,
        );
        h.bx(
            "shophouse",
            [x0 + 3.0, 0.0, ARCADE],
            [x1, ROOF - 0.2, SHOP_S],
            BRICK,
        );
        h.bx(
            "shophouse roof",
            [x0, ROOF - 0.2, ARCADE],
            [x1, ROOF, SHOP_S],
            TIMBER,
        );
        h.bx(
            "chimney",
            [37.0, ROOF, -9.5],
            [38.6, ROOF + 4.5, -6.5],
            BRICK,
        );
        h.bx("chimney", [37.0, ROOF, 0.5], [38.6, ROOF + 4.5, 3.5], BRICK);
        h.ramp_x(
            "shophouse ramp",
            (10.0, SHOP_S),
            (26.0, 0.0),
            (x0, ROOF),
            8.0,
            SANDSTONE,
        );
        h.light("shophouse roof", [40.0, ROOF + 3.0, -10.0], L_SUN, 12.0);

        // Stalls: an inner row facing the gallows (4) and an outer row (3).
        for (i, z) in [-18.0f32, -7.0, 4.0, 15.0].into_iter().enumerate() {
            h.stall(11.0, z, CANVAS[i], i % 2 == 0);
        }
        for (i, z) in [-14.0f32, -3.0, 6.0].into_iter().enumerate() {
            h.stall(22.0, z, CANVAS[(i + 2) % 4], i == 1);
        }
        h.light("stalls", [12.5, 3.0, -1.5], L_CANDLE, 11.0);
        h.light("stalls", [23.5, 3.0, -8.5], L_CANDLE, 11.0);
        h.light("stalls", [12.5, 3.0, 9.5], L_CANDLE, 11.0);

        // The gallows pad (from the dais onto the beam) and a candle pyramid (cover).
        h.pad(
            "gallows dais -> beam",
            [3.0, 0.9, 3.0],
            [3.0, BEAM, 0.0],
            1.5,
        );
        h.bx(
            "candle pyramid",
            [17.0, 0.0, 9.5],
            [19.5, 1.2, 12.0],
            WAXSTONE,
        );
        h.bx("candle pyramid", [17.5, 1.2, 10.0], [19.0, 2.2, 11.5], WAX);

        // The balustrade's sections (gaps at the jetty and one more), the jetty over the
        // gorge.
        h.bx(
            "balustrade",
            [18.0, 0.0, EDGE - 0.5],
            [26.0, 1.0, EDGE],
            WAXSTONE,
        );
        h.bx(
            "balustrade",
            [30.0, 0.0, EDGE - 0.5],
            [HX, 1.0, EDGE],
            WAXSTONE,
        );
        h.bx("jetty", [14.0, -0.4, EDGE], [18.0, 0.0, 28.0], TIMBER);
        for x in [14.2, 17.4] {
            h.bx(
                "jetty post",
                [x, GORGE + 2.0, 27.0],
                [x + 0.4, -0.4, 27.4],
                TIMBER_DARK,
            );
        }
        h.light("jetty", [16.0, 3.0, 27.0], L_CANDLE, 8.0);

        // Spawns: the candle shops (y 0), the shophouse roof behind a chimney (y 4), the
        // kiosk on the guild walk (y 7).
        for c in [alcoves[0], alcoves[2]] {
            h.spawn([38.2, 0.0, c]);
        }
        h.spawn([41.5, ROOF, -8.0]);
        h.spawn([30.0, WALK, -32.0]);
    }

    g.kill(
        "below the world",
        vol([-1000.0, -200.0, -1000.0], [1000.0, -30.0, 1000.0]),
    );
    g
}

/// A lamp post with a candle lantern on the balustrade line.
fn candles_post(g: &mut Gen, x: f32) {
    use mat::*;
    let z = EDGE - 0.25;
    g.bx(
        "lantern post",
        [x - 0.15, 1.0, z - 0.15],
        [x + 0.15, 3.4, z + 0.15],
        IRON,
    );
    g.bx(
        "lantern",
        [x - 0.3, 3.4, z - 0.3],
        [x + 0.3, 4.1, z + 0.3],
        GRATE,
    );
    g.deco(
        "lantern flame",
        [x - 0.15, 3.5, z - 0.15],
        [x + 0.15, 3.95, z + 0.15],
        SLAG,
    );
    g.light("lantern", [x, 3.8, z], L_CANDLE, 7.0);
}
