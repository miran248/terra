//! Water rendering (Phase 1): an `ExtendedMaterial<StandardMaterial, WaterExt>`
//! that gives the ocean depth-based colour + opacity (limited underwater
//! visibility) from the depth prepass and animated surface normals for swell.
//! Base PBR lighting/specular comes from `StandardMaterial`, so the sun glints
//! off the surface. Per-body meshes and river flow are later phases.

use bevy::asset::{Asset, RenderAssetUsages};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::mesh::PrimitiveTopology;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use shared::level::WaterPhase;

pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExt>;

/// Extension uniforms (group 2, binding 100). Colours are linear.
#[derive(Clone, Copy, ShaderType, Reflect, Debug)]
pub struct WaterParams {
    pub shallow: LinearRgba,
    pub deep: LinearRgba,
    /// View distance (world units) at which water becomes fully deep/opaque.
    pub max_visibility: f32,
    pub wave_amp: f32,
    pub wave_scale: f32,
    pub wave_speed: f32,
    /// 0 = still water (ripples drift by world position). >0 = river: the ripple
    /// pattern scrolls downstream along the per-vertex flow direction (vertex
    /// colour), at this speed.
    pub flow: f32,
}

#[derive(Asset, AsBindGroup, Clone, Reflect, Debug)]
pub struct WaterExt {
    #[uniform(100)]
    pub params: WaterParams,
}

impl MaterialExtension for WaterExt {
    fn fragment_shader() -> ShaderRef {
        "shaders/water.wgsl".into()
    }
}

/// Build the configured ocean material. Kept as a helper so the spawn site
/// (map::setup_map, which owns the `Ground` marker for level cleanup) stays simple.
pub fn water_material() -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            // Alpha is driven by the shader (depth fade); this base colour mostly
            // feeds specular/roughness. Smooth + reflective so the sun glints.
            base_color: Color::srgba(0.2, 0.42, 0.6, 0.7),
            perceptual_roughness: 0.08,
            reflectance: 0.5,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        },
        extension: WaterExt {
            params: WaterParams {
                shallow: LinearRgba::rgb(0.10, 0.42, 0.48),
                deep: LinearRgba::rgb(0.01, 0.06, 0.18),
                max_visibility: 14.0,
                wave_amp: 0.05,
                wave_scale: 0.35,
                wave_speed: 0.7,
                flow: 0.0,
            },
        },
    }
}

/// River water: clearer/shallower depth fade and downstream-scrolling ripples.
pub fn river_material() -> WaterMaterial {
    let mut m = water_material();
    m.extension.params.shallow = LinearRgba::rgb(0.12, 0.4, 0.5);
    m.extension.params.deep = LinearRgba::rgb(0.05, 0.2, 0.32);
    m.extension.params.max_visibility = 5.0;
    m.extension.params.wave_amp = 0.08;
    m.extension.params.wave_scale = 0.8;
    m.extension.params.flow = 6.0;
    m
}

/// Build the flat water-surface mesh — sea and lakes together. The clustering
/// into connected bodies + per-body waterline is done at gen time
/// (`worldgen::water_surface_radii`); `water_r[fi]` is that per-face radius, 0.0
/// for non-surface faces. Each face is simply drawn flat at its radius, so the
/// sea can't flood an inland basin and a lake can't spill onto land.
pub fn build_water_surface(
    tris: &[[[f32; 3]; 3]],
    water_r: &[f32],
    water_phase: &[Option<WaterPhase>],
) -> Option<Mesh> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (fi, t) in tris.iter().enumerate() {
        let r = water_r.get(fi).copied().unwrap_or(0.0);
        if r <= 0.0 || water_phase.get(fi) == Some(&Some(WaterPhase::Frozen)) {
            continue;
        }
        for &corner in t {
            let dir = Vec3::from_array(corner).normalize();
            positions.push((dir * r).to_array());
            normals.push(dir.to_array());
            uvs.push([0.0, 0.0]);
        }
    }
    if positions.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    Some(mesh)
}

/// Build the generator-baked, smooth terrain-following river mesh. Each face
/// carries its downhill flow direction (from its highest to lowest corner,
/// projected onto the surface) encoded in vertex colour, which the water shader
/// reads to scroll ripples downstream.
pub fn build_river_surfaces(
    tris: &[[[f32; 3]; 3]],
    river_r: &[[f32; 3]],
    water_phase: &[Option<WaterPhase>],
) -> Option<Mesh> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    for (fi, t) in tris.iter().enumerate() {
        if water_phase.get(fi) == Some(&Some(WaterPhase::Frozen)) {
            continue;
        }
        let radii = river_r.get(fi).copied().unwrap_or([0.0; 3]);
        if radii.iter().all(|&radius| radius <= 0.0) {
            continue;
        }
        let c = [
            Vec3::from_array(t[0]),
            Vec3::from_array(t[1]),
            Vec3::from_array(t[2]),
        ];
        let r = [c[0].length(), c[1].length(), c[2].length()];
        // Downhill across the face: highest corner → lowest corner, made tangent.
        let hi = (0..3).max_by(|&a, &b| r[a].total_cmp(&r[b])).unwrap();
        let lo = (0..3).min_by(|&a, &b| r[a].total_cmp(&r[b])).unwrap();
        let face_dir = (c[0] + c[1] + c[2]).normalize();
        let mut flow = c[lo] - c[hi];
        flow -= face_dir * flow.dot(face_dir);
        let flow = flow.normalize_or_zero() * 0.5 + Vec3::splat(0.5); // encode to 0..1
        for k in 0..3 {
            let dir = c[k].normalize();
            positions.push((dir * radii[k]).to_array());
            normals.push(dir.to_array());
            uvs.push([0.0, 0.0]);
            colors.push([flow.x, flow.y, flow.z, 1.0]);
        }
    }
    if positions.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    Some(mesh)
}

/// Collider-backed ice for locally frozen lake and river faces. The terrain
/// below remains unchanged, so mixed shore faces close with their neighboring
/// ground instead of being retyped into an ice terrain patch.
pub fn build_ice_surface(
    tris: &[[[f32; 3]; 3]],
    water_r: &[f32],
    river_r: &[[f32; 3]],
    water_phase: &[Option<WaterPhase>],
) -> Option<(Mesh, Vec<[[f32; 3]; 3]>)> {
    let mut ice_tris = Vec::new();
    for (fi, terrain_tri) in tris.iter().enumerate() {
        if water_phase.get(fi) != Some(&Some(WaterPhase::Frozen)) {
            continue;
        }
        let river = river_r.get(fi).copied().unwrap_or([0.0; 3]);
        let body_radius = water_r.get(fi).copied().unwrap_or(0.0);
        let radii = river.map(|radius| if radius > 0.0 { radius } else { body_radius });
        if radii.iter().all(|&radius| radius <= 0.0) {
            continue;
        }
        ice_tris.push(std::array::from_fn(|corner| {
            (Vec3::from_array(terrain_tri[corner]).normalize() * radii[corner]).to_array()
        }));
    }
    if ice_tris.is_empty() {
        return None;
    }
    let mut positions = Vec::with_capacity(ice_tris.len() * 3);
    let mut normals = Vec::with_capacity(ice_tris.len() * 3);
    let mut uvs = Vec::with_capacity(ice_tris.len() * 3);
    for tri in &ice_tris {
        for &corner in tri {
            positions.push(corner);
            normals.push(Vec3::from_array(corner).normalize().to_array());
            uvs.push([0.0, 0.0]);
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    Some((mesh, ice_tris))
}

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::mesh::VertexAttributeValues;
    use shared::terrain::Terrain;
    use std::collections::HashMap;

    /// A hexagonal lake — 6 fan faces around a centre, ringed by 6 LakeShore
    /// faces. The centre bed is deeper (smaller radius) than the shore, so the
    /// test proves the surface is flat at the shore level regardless of the bed.
    fn hex_lake() -> (Vec<[[f32; 3]; 3]>, Vec<u8>) {
        let r = 100.0f32;
        let on_sphere = |spread: f32, ang: f32, radius: f32| {
            Vec3::new(ang.cos() * spread, ang.sin() * spread, 1.0).normalize() * radius
        };
        // 0 = centre (deeper), 1..=6 inner ring (shore level), 7..=12 outer ring.
        let mut v = vec![Vec3::new(0.0, 0.0, 1.0) * (r - 10.0)];
        for i in 0..6 {
            let ang = i as f32 * std::f32::consts::FRAC_PI_3;
            v.push(on_sphere(0.15, ang, r));
        }
        for i in 0..6 {
            let ang = i as f32 * std::f32::consts::FRAC_PI_3;
            v.push(on_sphere(0.30, ang, r));
        }
        let mut tris = Vec::new();
        let mut ft = Vec::new();
        for i in 0..6 {
            let (a, b) = (1 + i, 1 + (i + 1) % 6);
            tris.push([v[0].to_array(), v[a].to_array(), v[b].to_array()]);
            ft.push(Terrain::Lake as u8);
        }
        for i in 0..6 {
            let (a, b, o) = (1 + i, 1 + (i + 1) % 6, 7 + i);
            tris.push([v[a].to_array(), v[o].to_array(), v[b].to_array()]);
            ft.push(Terrain::LakeShore as u8);
        }
        (tris, ft)
    }

    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }

    #[test]
    fn lake_surface_is_connected_and_level() {
        let (tris, _ft) = hex_lake();
        // Gen bakes one rim-locked radius per body; here the whole hex is one
        // body, so every face shares a single waterline radius.
        let water_r = vec![100.0f32; tris.len()];
        let phase = vec![Some(WaterPhase::Liquid); tris.len()];
        let mesh = build_water_surface(&tris, &water_r, &phase).expect("a lake mesh");
        let Some(VertexAttributeValues::Float32x3(pos)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        assert!(!pos.is_empty(), "empty lake mesh");

        // 1) Every vertex at the same radius (one altitude), regardless of bed.
        let r0 = Vec3::from_array(pos[0]).length();
        for p in pos {
            let r = Vec3::from_array(*p).length();
            assert!((r - r0).abs() < 1e-2, "vertex radius {r} != {r0}");
        }

        // 2) All faces form ONE connected component (via shared vertices).
        let key = |p: &[f32; 3]| {
            [
                (p[0] * 100.0).round() as i64,
                (p[1] * 100.0).round() as i64,
                (p[2] * 100.0).round() as i64,
            ]
        };
        let mut ids: HashMap<[i64; 3], usize> = HashMap::new();
        let mut parent: Vec<usize> = Vec::new();
        for tri in pos.chunks(3) {
            let mut idx = [0usize; 3];
            for (j, corner) in tri.iter().enumerate() {
                let next = ids.len();
                let id = *ids.entry(key(corner)).or_insert(next);
                if id == parent.len() {
                    parent.push(id);
                }
                idx[j] = id;
            }
            let r1 = find(&mut parent, idx[1]);
            let r0 = find(&mut parent, idx[0]);
            parent[r0] = r1;
            let r2 = find(&mut parent, idx[2]);
            let r0 = find(&mut parent, idx[0]);
            parent[r0] = r2;
        }
        let root = find(&mut parent, 0);
        for i in 0..parent.len() {
            assert_eq!(
                find(&mut parent, i),
                root,
                "lake mesh is not one connected component"
            );
        }
    }

    #[test]
    fn river_surface_uses_baked_smooth_corner_radii() {
        let a = Vec3::new(0.0, 0.0, 100.0);
        let b = Vec3::new(10.0, 0.0, 101.0);
        let c = Vec3::new(0.0, 10.0, 98.0);
        let d = Vec3::new(10.0, 10.0, 102.0);
        let e = Vec3::new(30.0, 0.0, 100.0);
        let f = Vec3::new(40.0, 0.0, 100.0);
        let g = Vec3::new(30.0, 10.0, 100.0);
        let tris = vec![
            [a.to_array(), b.to_array(), c.to_array()],
            [b.to_array(), d.to_array(), c.to_array()],
            [e.to_array(), f.to_array(), g.to_array()],
        ];
        let river_r = vec![[100.5, 100.0, 99.5], [100.0, 101.0, 99.5], [0.0; 3]];
        let phase = vec![Some(WaterPhase::Liquid); tris.len()];
        let mesh = build_river_surfaces(&tris, &river_r, &phase).expect("river mesh");
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no river positions");
        };
        assert_eq!(positions.len(), 6, "channel and bank faces only");

        let key = |p: &[f32; 3]| {
            [
                (p[0] * 100.0).round() as i64,
                (p[1] * 100.0).round() as i64,
                (p[2] * 100.0).round() as i64,
            ]
        };
        let mut heights = HashMap::new();
        for position in positions {
            let height = Vec3::from_array(*position).length();
            let existing = heights.insert(key(position), height);
            if let Some(previous) = existing {
                assert!(
                    (previous - height).abs() < 1e-3,
                    "shared river vertex stepped"
                );
            }
        }
        let min_height = heights.values().copied().reduce(f32::min).unwrap();
        let max_height = heights.values().copied().reduce(f32::max).unwrap();
        assert!(
            max_height - min_height > 0.1,
            "river surface should follow terrain relief"
        );
    }

    #[test]
    fn frozen_faces_move_from_liquid_mesh_to_ice_collider() {
        let tris = vec![
            [[0.0, 0.0, 99.0], [1.0, 0.0, 99.0], [0.0, 1.0, 99.0]],
            [[0.0, 0.0, 99.0], [-1.0, 0.0, 99.0], [0.0, -1.0, 99.0]],
        ];
        let water_r = vec![100.0; 2];
        let river_r = vec![[101.0, 0.0, 102.0], [0.0; 3]];
        let phase = vec![Some(WaterPhase::Frozen), Some(WaterPhase::Liquid)];
        let liquid = build_water_surface(&tris, &water_r, &phase).unwrap();
        let Some(VertexAttributeValues::Float32x3(liquid_positions)) =
            liquid.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no liquid positions");
        };
        assert_eq!(liquid_positions.len(), 3);
        let (_, ice_tris) = build_ice_surface(&tris, &water_r, &river_r, &phase).unwrap();
        assert_eq!(ice_tris.len(), 1);
        let radii = ice_tris[0].map(|corner| Vec3::from_array(corner).length());
        for (actual, expected) in radii.into_iter().zip([101.0, 100.0, 102.0]) {
            assert!((actual - expected).abs() < 1e-3);
        }
    }
}
