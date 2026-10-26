//! Procedural models shared by the `render_*` examples.

use engine::assets::gltf::ModelPart;
use engine::assets::meshgen::{self, BoxDesc, MeshBuilder};
use engine::assets::procedural;
use engine::gfx::MeshData;
use engine::glam::{Mat4, Quat, Vec3, Vec4};

/// The "Warden": a faceted wax-stone orb ringed with rusted spikes and a glowing eye (original
/// design, used as the demo's spinning glTF model).
pub fn warden_parts() -> Vec<ModelPart> {
    let mut body = meshgen::icosphere(0.7, 1, Vec4::ONE);
    body.ao_spheres(&[(Vec3::new(0.0, -0.9, 0.0), 0.9)], 0.45);

    let mut spikes = MeshBuilder::new();
    let rust = Vec4::new(0.62, 0.32, 0.16, 1.0);
    for i in 0..8 {
        let angle = i as f32 / 8.0 * std::f32::consts::TAU;
        let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
        let rotation = Quat::from_rotation_arc(Vec3::Y, (dir + Vec3::Y * 0.25).normalize());
        let mut cone = meshgen::cone(0.14, 0.75, 5, rust);
        cone.vertical_gradient(Vec4::splat(0.55).with_w(1.0), Vec4::ONE);
        spikes.append_builder(
            &cone,
            Mat4::from_rotation_translation(rotation, dir * 0.85 + Vec3::Y * 0.2),
        );
    }
    // A collar ring and a lower stinger.
    let collar = meshgen::cylinder(0.78, 0.12, 12, Vec4::new(0.25, 0.26, 0.3, 1.0));
    spikes.append_builder(&collar, Mat4::from_translation(Vec3::new(0.0, -0.1, 0.0)));
    let mut stinger = meshgen::cone(0.2, 0.9, 6, Vec4::new(0.3, 0.3, 0.34, 1.0));
    stinger.flip_upside_down();
    spikes.append_builder(&stinger, Mat4::from_translation(Vec3::new(0.0, -1.05, 0.0)));
    // Eye: a short cylinder pointing forward (+Z), bright red.
    spikes.add_cylinder_between(
        Vec3::new(0.0, 0.1, 0.55),
        Vec3::new(0.0, 0.1, 0.78),
        0.18,
        8,
        Vec4::new(1.0, 0.15, 0.1, 1.0),
    );
    spikes.shade_by_normal(1.0, 0.85, 0.6);

    vec![
        ModelPart {
            name: "warden_body".into(),
            mesh: body.build(),
            texture: Some(procedural::material_texture("waxstone")),
            base_color: Vec4::new(1.0, 0.95, 0.9, 1.0),
        },
        ModelPart {
            name: "warden_spikes".into(),
            mesh: spikes.build(),
            texture: None,
            base_color: Vec4::ONE,
        },
    ]
}

/// Extension used above: mirror a primitive vertically (keeping faces outward).
trait FlipUpsideDown {
    fn flip_upside_down(&mut self);
}

impl FlipUpsideDown for MeshBuilder {
    fn flip_upside_down(&mut self) {
        self.transform(Mat4::from_rotation_x(std::f32::consts::PI));
    }
}

/// Viewmodel demo gun (original design): returns (body mesh, glowing strip mesh, muzzle
/// position in the mesh's local space). Local space: barrel points down -Z.
pub fn demo_gun() -> (MeshData, MeshData, Vec3) {
    let steel = Vec4::new(0.72, 0.75, 0.8, 1.0);
    let dark = Vec4::new(0.32, 0.33, 0.37, 1.0);
    let grip_color = Vec4::new(0.36, 0.22, 0.16, 1.0);
    let mut b = MeshBuilder::new();
    // Receiver.
    b.add_box(&BoxDesc {
        center: Vec3::new(0.0, 0.0, -0.08),
        half_extents: Vec3::new(0.04, 0.05, 0.17),
        color: steel,
        ..Default::default()
    });
    // Top rail and sight.
    b.add_box(&BoxDesc {
        center: Vec3::new(0.0, 0.058, -0.1),
        half_extents: Vec3::new(0.015, 0.01, 0.13),
        color: dark,
        ..Default::default()
    });
    b.add_box(&BoxDesc {
        center: Vec3::new(0.0, 0.08, -0.2),
        half_extents: Vec3::new(0.006, 0.014, 0.01),
        color: dark,
        ..Default::default()
    });
    // Barrel + shroud.
    b.add_cylinder_between(
        Vec3::new(0.0, 0.015, -0.24),
        Vec3::new(0.0, 0.015, -0.46),
        0.02,
        8,
        dark,
    );
    b.add_cylinder_between(
        Vec3::new(0.0, 0.015, -0.25),
        Vec3::new(0.0, 0.015, -0.33),
        0.034,
        8,
        steel,
    );
    // Grip, angled back.
    b.add_box(&BoxDesc {
        center: Vec3::new(0.0, -0.1, 0.02),
        half_extents: Vec3::new(0.03, 0.075, 0.035),
        rotation: Quat::from_rotation_x(0.3),
        color: grip_color,
        ..Default::default()
    });
    // Trigger guard.
    b.add_box(&BoxDesc {
        center: Vec3::new(0.0, -0.065, -0.06),
        half_extents: Vec3::new(0.008, 0.008, 0.04),
        color: dark,
        ..Default::default()
    });
    b.shade_by_normal(1.0, 0.8, 0.55);

    // Energy cell on the side (drawn unlit so it glows).
    let mut glow = MeshBuilder::new();
    glow.add_box(&BoxDesc {
        center: Vec3::new(0.043, 0.0, -0.1),
        half_extents: Vec3::new(0.006, 0.02, 0.09),
        color: Vec4::new(0.3, 1.0, 0.95, 1.0),
        ..Default::default()
    });
    (b.build(), glow.build(), Vec3::new(0.0, 0.015, -0.47))
}
