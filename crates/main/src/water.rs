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
            },
        },
    }
}

/// Build a flat water-surface mesh for each connected lake (Phase 2). Lakes on
/// high ground aren't covered by the sea sphere (which sits at sea level), so
/// they get their own surface at the lake's waterline. `tris` are the baked,
/// displaced terrain triangles; `face_types` are per-face `Terrain` discriminants.
pub fn build_lake_surfaces(tris: &[[[f32; 3]; 3]], face_types: &[u8]) -> Vec<Mesh> {
    let lake = Terrain::Lake as u8;
    let n = tris.len();
    let vtris: Vec<[Vec3; 3]> = tris
        .iter()
        .map(|t| [Vec3::from_array(t[0]), Vec3::from_array(t[1]), Vec3::from_array(t[2])])
        .collect();
    let adj = build_face_adjacency(&vtris, n);

    let mut visited = vec![false; n];
    let mut meshes = Vec::new();
    for start in 0..n {
        if visited[start] || face_types.get(start).copied() != Some(lake) {
            continue;
        }
        // Flood-fill this connected lake body over face adjacency.
        let mut body = Vec::new();
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(f) = stack.pop() {
            body.push(f);
            for &nb in &adj[f] {
                let nb = nb as usize;
                if nb < n && !visited[nb] && face_types.get(nb).copied() == Some(lake) {
                    visited[nb] = true;
                    stack.push(nb);
                }
            }
        }

        // Waterline radius: the shallowest bed point (lake bed is displaced below
        // the surface, so its max radius is the shore edge). Cap it near the
        // body's own deepest point so a body that happens to span altitudes can't
        // float a sheet up in the sky.
        let mut max_r = 0.0f32;
        let mut min_r = f32::MAX;
        for &f in &body {
            for k in 0..3 {
                let r = vtris[f][k].length();
                max_r = max_r.max(r);
                min_r = min_r.min(r);
            }
        }
        let surf_r = max_r.min(min_r + 50.0) + 1.5;

        let mut positions = Vec::with_capacity(body.len() * 3);
        let mut normals = Vec::with_capacity(body.len() * 3);
        let mut uvs = Vec::with_capacity(body.len() * 3);
        for &f in &body {
            for k in 0..3 {
                let dir = vtris[f][k].normalize();
                positions.push((dir * surf_r).to_array());
                normals.push(dir.to_array());
                uvs.push([0.0, 0.0]);
            }
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        meshes.push(mesh);
    }
    meshes
}

/// Build a single flat ocean-surface mesh over all `Ocean` faces at `sea_r`.
/// Replaces the full sea sphere so the ocean surface only exists over real ocean
/// — a full sphere poked up inside any lake basin whose bed dips below sea level.
pub fn build_ocean_surface(tris: &[[[f32; 3]; 3]], face_types: &[u8], sea_r: f32) -> Option<Mesh> {
    let ocean = Terrain::Ocean as u8;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for (fi, t) in tris.iter().enumerate() {
        if face_types.get(fi).copied() != Some(ocean) {
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

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default());
    }
}
