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
use shared::planet::build_face_adjacency;
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
    let lake = Terrain::Lake as u8;
    let shore = Terrain::LakeShore as u8;
    let n = tris.len();
    let vtris: Vec<[Vec3; 3]> = tris
        .iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let adj = build_face_adjacency(&vtris, n);

    let mut visited = vec![false; n];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for start in 0..n {
        if visited[start] || face_types.get(start).copied() != Some(lake) {
            continue;
        }
        // Flood-fill the lake body; fill level = its lowest surrounding shore
        // (the spill point), like the ocean's sea level. Only faces whose bed
        // dips below that level get water — shallow shelves above it stay dry, so
        // the surface can't float.
        let mut body = Vec::new();
        let mut fill_r = f32::MAX;
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(f) = stack.pop() {
            body.push(f);
            for &nb in &adj[f] {
                let nb = nb as usize;
                if nb >= n {
                    continue;
                }
                let kind = face_types.get(nb).copied();
                if !visited[nb] && kind == Some(lake) {
                    visited[nb] = true;
                    stack.push(nb);
                } else if kind == Some(shore) {
                    for k in 0..3 {
                        fill_r = fill_r.min(vtris[nb][k].length());
                    }
                }
            }
        }
        // No shore found (rare) → fall back to the shallowest bed corner.
        if fill_r == f32::MAX {
            for &f in &body {
                for k in 0..3 {
                    fill_r = fill_r.max(vtris[f][k].length());
                }
            }
            if fill_r == f32::MAX {
                continue;
            }
        }
        for &f in &body {
            if !(0..3).any(|k| vtris[f][k].length() < fill_r) {
                continue; // shelf above the waterline: dry
            }
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
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
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
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
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
        let c = [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])];
        let r = [c[0].length(), c[1].length(), c[2].length()];
        // Downhill across the face: highest corner → lowest corner, made tangent
        // to the surface. (The baked tris are already displaced, so radius is the
        // local surface height.)
        let hi = (0..3).max_by(|&a, &b| r[a].total_cmp(&r[b])).unwrap();
        let lo = (0..3).min_by(|&a, &b| r[a].total_cmp(&r[b])).unwrap();
        let face_dir = ((c[0] + c[1] + c[2]) / 3.0).normalize();
        let mut flow = c[lo] - c[hi];
        flow -= face_dir * flow.dot(face_dir);
        let flow = flow.normalize_or_zero() * 0.5 + Vec3::splat(0.5); // encode to 0..1
        for k in 0..3 {
            let dir = c[k].normalize();
            positions.push((dir * (r[k] + 0.3)).to_array());
            normals.push(dir.to_array());
            uvs.push([0.0, 0.0]);
            colors.push([flow.x, flow.y, flow.z, 1.0]);
        }
    }
    if positions.is_empty() {
        return None;
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
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
