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
use shared::terrain::Terrain;

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

/// Build a flat water-surface mesh for each connected lake (Phase 2). Lakes on
/// high ground aren't covered by the sea sphere (which sits at sea level), so
/// they get their own surface at the lake's waterline. `tris` are the baked,
/// displaced terrain triangles; `face_types` are per-face `Terrain` discriminants.
pub fn build_lake_surfaces(tris: &[[[f32; 3]; 3]], face_types: &[u8]) -> Option<Mesh> {
    use std::collections::{HashMap, HashSet};
    let lake = Terrain::Lake as u8;
    let n = tris.len();
    let vtris: Vec<[Vec3; 3]> = tris
        .iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    // Quantise a vertex position to a key so shared corners map together. Coarse
    // (~1 unit) since distinct grid vertices are ~35 units apart, so this reliably
    // merges shared corners despite any float wobble.
    let vkey = |v: Vec3| [v.x.round() as i64, v.y.round() as i64, v.z.round() as i64];

    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }

    // Every face touching each vertex (used for both grouping and expansion).
    let mut vk_faces: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for fi in 0..n {
        for k in 0..3 {
            vk_faces.entry(vkey(vtris[fi][k])).or_default().push(fi);
        }
    }

    // Group Lake faces into bodies by SHARED VERTICES (union-find). Vertex
    // connectivity means a lake that's only pinched at a corner still counts as
    // one body, so it can't fragment into detached pieces at different heights.
    let mut parent: Vec<usize> = (0..n).collect();
    for faces in vk_faces.values() {
        let lakes: Vec<usize> = faces.iter().copied().filter(|&f| face_types.get(f).copied() == Some(lake)).collect();
        for w in lakes.windows(2) {
            let (a, b) = (find(&mut parent, w[0]), find(&mut parent, w[1]));
            parent[a] = b;
        }
    }

    // Per body: the Lake faces, and the radii of ALL their vertices — the altitude
    // is measured from Lake tiles ONLY.
    let mut body_faces: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut body_radii: HashMap<usize, Vec<f32>> = HashMap::new();
    for fi in 0..n {
        if face_types.get(fi).copied() == Some(lake) {
            let rep = find(&mut parent, fi);
            body_faces.entry(rep).or_default().push(fi);
            let e = body_radii.entry(rep).or_default();
            for k in 0..3 {
                e.push(vtris[fi][k].length());
            }
        }
    }

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (rep, faces) in &body_faces {
        // Single altitude for the whole body: a high percentile of the Lake tiles'
        // vertex radii (the shallow rim ≈ waterline; the worldgen keeps lakes below
        // their shore). Absolute radius from the planet centre → curvature-correct
        // and identical for every vertex.
        let mut radii = body_radii[rep].clone();
        radii.sort_by(f32::total_cmp);
        let fill_r = radii[radii.len() * 9 / 10];

        // Render the Lake faces, then EXPAND one ring into neighbouring faces to
        // fill the gaps up to the true waterline. All rendered at the same fill_r,
        // ignoring the neighbours' own altitude; where their terrain rises above
        // fill_r they're simply hidden behind it (depth test).
        let mut render: HashSet<usize> = faces.iter().copied().collect();
        for &f in faces {
            for k in 0..3 {
                if let Some(neigh) = vk_faces.get(&vkey(vtris[f][k])) {
                    for &nf in neigh {
                        render.insert(nf);
                    }
                }
            }
        }
        for f in render {
            for k in 0..3 {
                let dir = vtris[f][k].normalize();
                positions.push((dir * fill_r).to_array());
                normals.push(dir.to_array());
                uvs.push([0.0, 0.0]);
            }
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

/// Build a single flat ocean-surface mesh at `sea_r` over every face that dips
/// below sea level — i.e. real ocean *and* the submerged part of the coast (beach
/// tiles below 0 m). Selecting by height rather than only `Ocean` faces closes
/// the gap that appeared between the ocean edge and the beach. Lake/River faces
/// are excluded (they have their own surfaces).
pub fn build_ocean_surface(tris: &[[[f32; 3]; 3]], face_types: &[u8], sea_r: f32) -> Option<Mesh> {
    let lake = Terrain::Lake as u8;
    let river = Terrain::River as u8;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (fi, t) in tris.iter().enumerate() {
        let kind = face_types.get(fi).copied();
        if kind == Some(lake) || kind == Some(river) {
            continue;
        }
        // Underwater if any corner is below sea level (covers the coastline).
        let underwater = t.iter().any(|c| Vec3::from_array(*c).length() < sea_r);
        if !underwater {
            continue;
        }
        for k in 0..3 {
            let dir = Vec3::from_array(t[k]).normalize();
            positions.push((dir * sea_r).to_array());
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

/// Build a river-surface mesh over all `River` faces at the carved channel
/// height. Each face carries its downhill flow direction (from its highest to
/// lowest corner, projected onto the surface) encoded in vertex colour, which the
/// water shader reads to scroll ripples downstream.
pub fn build_river_surfaces(tris: &[[[f32; 3]; 3]], face_types: &[u8]) -> Option<Mesh> {
    let river = Terrain::River as u8;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    for (fi, t) in tris.iter().enumerate() {
        if face_types.get(fi).copied() != Some(river) {
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
        let face_dir = ((c[0] + c[1] + c[2]) / 3.0).normalize();
        let mut flow = c[lo] - c[hi];
        flow -= face_dir * flow.dot(face_dir);
        let flow = flow.normalize_or_zero() * 0.5 + Vec3::splat(0.5); // encode to 0..1
        for k in 0..3 {
            let dir = c[k].normalize();
            positions.push((dir * (r[k] + 0.5)).to_array());
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
        let (tris, ft) = hex_lake();
        let mesh = build_lake_surfaces(&tris, &ft).expect("a lake mesh");
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
}
