use gltf::Glb;
use serde_json::{Value, json};
use shared::{
    art::{ACTOR_ANIMATIONS, AssetName, FLORA_KINDS, STRUCTURE_KINDS},
    items::{Material, WeaponKind},
};
use std::{borrow::Cow, env, fs, path::PathBuf};

const CATALOGS: [&str; 4] = [
    "environment.glb",
    "structures.glb",
    "items.glb",
    "actors.glb",
];

fn main() {
    let mut out = PathBuf::from("crates/main/assets/models");
    let mut check = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out-dir" => {
                out = args
                    .next()
                    .map(PathBuf::from)
                    .expect("--out-dir requires a path")
            }
            "--check" => check = true,
            "-h" | "--help" => {
                println!("gen_assets [--out-dir PATH] [--check]");
                return;
            }
            _ => panic!("unknown argument: {arg}"),
        }
    }
    if !check {
        fs::create_dir_all(&out).expect("create output directory");
    }
    for (name, bytes) in catalogs() {
        let path = out.join(name);
        if check {
            let actual = fs::read(&path)
                .unwrap_or_else(|_| panic!("missing generated catalog: {}", path.display()));
            assert_eq!(actual, bytes, "catalog is stale: {}", path.display());
        } else {
            fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
            println!("generated {}", path.display());
        }
    }
}

fn catalogs() -> Vec<(&'static str, Vec<u8>)> {
    let environment: Vec<_> = FLORA_KINDS.into_iter().map(AssetName::asset_name).collect();
    let structures: Vec<_> = STRUCTURE_KINDS
        .into_iter()
        .map(AssetName::asset_name)
        .collect();
    let mut items: Vec<_> = Material::ALL
        .into_iter()
        .map(AssetName::asset_name)
        .collect();
    items.extend(WeaponKind::ALL.into_iter().map(AssetName::asset_name));
    vec![
        (CATALOGS[0], catalog(&environment, false)),
        (CATALOGS[1], catalog(&structures, false)),
        (CATALOGS[2], catalog(&items, false)),
        (
            CATALOGS[3],
            catalog(&["actor.player", "actor.zombie.0", "actor.zombie.1"], true),
        ),
    ]
}

struct MeshData {
    positions: Vec<f32>,
    normals: Vec<f32>,
    colors: Vec<f32>,
    indices: Vec<u16>,
}

impl MeshData {
    fn new() -> Self {
        Self {
            positions: Vec::new(),
            normals: Vec::new(),
            colors: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn add_face(&mut self, pts: [[f32; 3]; 4], color: [f32; 4]) {
        let normal = compute_normal(pts[0], pts[1], pts[2]);
        let base_idx = (self.positions.len() / 3) as u16;
        for p in pts {
            self.positions.extend_from_slice(&p);
            self.normals.extend_from_slice(&normal);
            self.colors.extend_from_slice(&color);
        }
        self.indices.extend_from_slice(&[
            base_idx, base_idx + 1, base_idx + 2,
            base_idx, base_idx + 2, base_idx + 3,
        ]);
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len == 0.0 { [0.0, 0.0, 0.0] } else { [v[0] / len, v[1] / len, v[2] / len] }
}

fn compute_normal(p0: [f32; 3], p1: [f32; 3], p2: [f32; 3]) -> [f32; 3] {
    let v1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
    let v2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
    let normal = [
        v1[1] * v2[2] - v1[2] * v2[1],
        v1[2] * v2[0] - v1[0] * v2[2],
        v1[0] * v2[1] - v1[1] * v2[0],
    ];
    normalize(normal)
}

fn add_triangle(mesh: &mut MeshData, p0: [f32; 3], p1: [f32; 3], p2: [f32; 3], color: [f32; 4]) {
    let normal = compute_normal(p0, p1, p2);
    let base_idx = (mesh.positions.len() / 3) as u16;
    mesh.positions.extend_from_slice(&p0);
    mesh.positions.extend_from_slice(&p1);
    mesh.positions.extend_from_slice(&p2);
    for _ in 0..3 {
        mesh.normals.extend_from_slice(&normal);
        mesh.colors.extend_from_slice(&color);
    }
    mesh.indices.extend_from_slice(&[base_idx, base_idx + 1, base_idx + 2]);
}

fn add_box(mesh: &mut MeshData, min: [f32; 3], max: [f32; 3], color: [f32; 4]) {
    let x0 = min[0]; let y0 = min[1]; let z0 = min[2];
    let x1 = max[0]; let y1 = max[1]; let z1 = max[2];

    // Front face (Z = max)
    mesh.add_face([
        [x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]
    ], color);

    // Back face (Z = min)
    mesh.add_face([
        [x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]
    ], color);

    // Left face (X = min)
    mesh.add_face([
        [x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]
    ], color);

    // Right face (X = max)
    mesh.add_face([
        [x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]
    ], color);

    // Top face (Y = max)
    mesh.add_face([
        [x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]
    ], color);

    // Bottom face (Y = min)
    mesh.add_face([
        [x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]
    ], color);
}

fn add_pyramid(mesh: &mut MeshData, base_min: [f32; 2], base_max: [f32; 2], base_y: f32, top_y: f32, color: [f32; 4]) {
    let x0 = base_min[0];
    let z0 = base_min[1];
    let x1 = base_max[0];
    let z1 = base_max[1];
    let top = [(x0 + x1) / 2.0, top_y, (z0 + z1) / 2.0];

    // Bottom face (quad)
    mesh.add_face([
        [x0, base_y, z0], [x1, base_y, z0], [x1, base_y, z1], [x0, base_y, z1]
    ], color);

    // 4 Sides:
    // Front side (Z = max)
    add_triangle(mesh, [x0, base_y, z1], [x1, base_y, z1], top, color);

    // Right side (X = max)
    add_triangle(mesh, [x1, base_y, z1], [x1, base_y, z0], top, color);

    // Back side (Z = min)
    add_triangle(mesh, [x1, base_y, z0], [x0, base_y, z0], top, color);

    // Left side (X = min)
    add_triangle(mesh, [x0, base_y, z0], [x0, base_y, z1], top, color);
}

fn wedge_mesh() -> MeshData {
    let positions = vec![
        -0.5, 0.0, 0.5,
        0.5, 0.0, 0.5,
        0.42, 0.0, -0.6,
        -0.42, 0.0, -0.6,
        0.0, 1.0, 0.10,
    ];
    let normals = vec![
        0.0, -1.0, 0.0,
        0.0, -1.0, 0.0,
        0.0, -1.0, 0.0,
        0.0, -1.0, 0.0,
        0.0, 1.0, 0.0,
    ];
    let indices = vec![0, 2, 1, 0, 3, 2, 0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4];
    let colors = vec![
        0.82, 0.28, 0.12, 1.0,
        0.95, 0.55, 0.16, 1.0,
        0.55, 0.16, 0.08, 1.0,
        0.70, 0.22, 0.10, 1.0,
        0.95, 0.72, 0.20, 1.0,
    ];
    MeshData { positions, normals, colors, indices }
}

fn generate_mesh(name: &str) -> MeshData {
    let mut mesh = MeshData::new();
    match name {
        "flora.tree" => {
            // Trunk: Brown rectangular prism
            let trunk_color = [0.45, 0.28, 0.13, 1.0];
            add_box(&mut mesh, [-0.15, 0.0, -0.15], [0.15, 1.0, 0.15], trunk_color);

            // Lower Canopy: Green pyramid
            let leaves_color = [0.15, 0.45, 0.15, 1.0];
            add_pyramid(&mut mesh, [-0.65, -0.65], [0.65, 0.65], 0.9, 2.0, leaves_color);

            // Upper Canopy: Green pyramid (slightly smaller, overlapping)
            add_pyramid(&mut mesh, [-0.45, -0.45], [0.45, 0.45], 1.6, 2.8, leaves_color);
        }
        "flora.bush" => {
            let color = [0.18, 0.50, 0.18, 1.0];
            add_box(&mut mesh, [-0.4, 0.0, -0.4], [0.4, 0.6, 0.4], color);
        }
        "flora.flower" => {
            let stem_color = [0.2, 0.6, 0.2, 1.0];
            let petal_color = [0.9, 0.2, 0.5, 1.0];
            add_box(&mut mesh, [-0.03, 0.0, -0.03], [0.03, 0.4, 0.03], stem_color);
            add_pyramid(&mut mesh, [-0.1, -0.1], [0.1, 0.1], 0.4, 0.55, petal_color);
        }
        "flora.rock" => {
            let color = [0.45, 0.45, 0.45, 1.0];
            add_box(&mut mesh, [-0.5, 0.0, -0.4], [0.5, 0.5, 0.4], color);
        }
        "flora.grass" => {
            let color = [0.25, 0.65, 0.25, 1.0];
            // Grass clump made of three staggered small vertical boxes
            add_box(&mut mesh, [-0.15, 0.0, -0.05], [-0.05, 0.35, 0.05], color);
            add_box(&mut mesh, [0.02, 0.0, -0.12], [0.12, 0.4, -0.02], color);
            add_box(&mut mesh, [-0.05, 0.0, 0.08], [0.05, 0.3, 0.18], color);
        }
        "flora.log" => {
            let color = [0.4, 0.25, 0.1, 1.0];
            add_box(&mut mesh, [-0.6, 0.0, -0.2], [0.6, 0.3, 0.2], color);
        }
        "flora.mushroom" => {
            let stem_color = [0.9, 0.9, 0.85, 1.0];
            let cap_color = [0.8, 0.15, 0.15, 1.0];
            add_box(&mut mesh, [-0.06, 0.0, -0.06], [0.06, 0.25, 0.06], stem_color);
            add_box(&mut mesh, [-0.2, 0.22, -0.2], [0.2, 0.35, 0.2], cap_color);
        }
        "flora.cactus" => {
            let color = [0.1, 0.45, 0.15, 1.0];
            // Main trunk
            add_box(&mut mesh, [-0.12, 0.0, -0.12], [0.12, 0.9, 0.12], color);
            // Left branch
            add_box(&mut mesh, [-0.3, 0.4, -0.08], [-0.12, 0.52, 0.08], color);
            add_box(&mut mesh, [-0.3, 0.52, -0.08], [-0.18, 0.75, 0.08], color);
            // Right branch
            add_box(&mut mesh, [0.12, 0.5, -0.08], [0.3, 0.62, 0.08], color);
            add_box(&mut mesh, [0.18, 0.62, -0.08], [0.3, 0.82, 0.08], color);
        }
        "flora.berry" => {
            let bush_color = [0.15, 0.45, 0.2, 1.0];
            let berry_color = [0.85, 0.1, 0.15, 1.0];
            add_box(&mut mesh, [-0.35, 0.0, -0.35], [0.35, 0.55, 0.35], bush_color);
            // Red berry dots
            add_box(&mut mesh, [-0.2, 0.4, 0.36], [-0.1, 0.48, 0.38], berry_color);
            add_box(&mut mesh, [0.15, 0.3, 0.36], [0.25, 0.38, 0.38], berry_color);
            add_box(&mut mesh, [-0.37, 0.35, -0.1], [-0.35, 0.43, 0.0], berry_color);
            add_box(&mut mesh, [0.35, 0.25, 0.1], [0.37, 0.33, 0.2], berry_color);
        }
        "flora.dead_tree" => {
            let color = [0.35, 0.22, 0.12, 1.0];
            // Bare trunk
            add_box(&mut mesh, [-0.12, 0.0, -0.12], [0.12, 1.2, 0.12], color);
            // Angular branch 1
            add_box(&mut mesh, [-0.35, 0.7, -0.08], [-0.12, 0.82, 0.08], color);
            add_box(&mut mesh, [-0.45, 0.82, -0.08], [-0.35, 1.1, 0.08], color);
            // Angular branch 2
            add_box(&mut mesh, [0.12, 0.5, -0.08], [0.35, 0.62, 0.08], color);
            add_box(&mut mesh, [0.25, 0.62, -0.08], [0.45, 0.9, 0.08], color);
        }
        "flora.reed" => {
            let color = [0.55, 0.55, 0.2, 1.0];
            add_box(&mut mesh, [-0.18, 0.0, -0.04], [-0.12, 0.95, 0.02], color);
            add_box(&mut mesh, [0.02, 0.0, -0.15], [0.08, 1.1, -0.09], color);
            add_box(&mut mesh, [-0.04, 0.0, 0.1], [0.02, 0.85, 0.16], color);
        }
        "structure.ruin" => {
            let color = [0.4, 0.4, 0.42, 1.0];
            // Fallen column blocks
            add_box(&mut mesh, [-0.3, 0.0, -0.3], [0.3, 0.4, 0.3], color);
            add_box(&mut mesh, [-0.25, 0.4, -0.25], [0.25, 0.8, 0.25], color);
            add_box(&mut mesh, [-0.1, 0.8, -0.3], [0.4, 1.1, 0.1], color); // tipped piece
        }
        "structure.watchtower" => {
            let wood_color = [0.45, 0.28, 0.12, 1.0];
            let roof_color = [0.5, 0.15, 0.15, 1.0];
            // 4 legs
            add_box(&mut mesh, [-0.4, 0.0, -0.4], [-0.3, 1.5, -0.3], wood_color);
            add_box(&mut mesh, [0.3, 0.0, -0.4], [0.4, 1.5, -0.3], wood_color);
            add_box(&mut mesh, [-0.4, 0.0, 0.3], [-0.3, 1.5, 0.4], wood_color);
            add_box(&mut mesh, [0.3, 0.0, 0.3], [0.4, 1.5, 0.4], wood_color);
            // Platform
            add_box(&mut mesh, [-0.5, 1.4, -0.5], [0.5, 1.55, 0.5], wood_color);
            // Roof
            add_pyramid(&mut mesh, [-0.55, -0.55], [0.55, 0.55], 1.55, 2.1, roof_color);
        }
        "structure.dock" => {
            let color = [0.38, 0.24, 0.10, 1.0];
            // Flat wooden pier
            add_box(&mut mesh, [-0.6, 0.0, -0.6], [0.6, 0.15, 0.6], color);
            add_box(&mut mesh, [-0.5, -0.4, -0.5], [-0.4, 0.0, -0.4], color);
            add_box(&mut mesh, [0.4, -0.4, -0.5], [0.5, 0.0, -0.4], color);
        }
        "structure.farm" => {
            let soil_color = [0.3, 0.18, 0.08, 1.0];
            let crop_color = [0.2, 0.55, 0.15, 1.0];
            // Flat soil base
            add_box(&mut mesh, [-0.6, 0.0, -0.6], [0.6, 0.1, 0.6], soil_color);
            // Row 1 crops
            add_box(&mut mesh, [-0.4, 0.1, -0.3], [-0.2, 0.25, -0.1], crop_color);
            add_box(&mut mesh, [-0.4, 0.1, 0.1], [-0.2, 0.25, 0.3], crop_color);
            // Row 2 crops
            add_box(&mut mesh, [0.2, 0.1, -0.3], [0.4, 0.25, -0.1], crop_color);
            add_box(&mut mesh, [0.2, 0.1, 0.1], [0.4, 0.25, 0.3], crop_color);
        }
        "structure.wall" => {
            let color = [0.48, 0.48, 0.5, 1.0];
            add_box(&mut mesh, [-0.6, 0.0, -0.2], [0.6, 0.9, 0.2], color);
        }
        "structure.well" => {
            let stone_color = [0.4, 0.4, 0.42, 1.0];
            let wood_color = [0.45, 0.28, 0.13, 1.0];
            let roof_color = [0.15, 0.15, 0.15, 1.0];
            // Stone base
            add_box(&mut mesh, [-0.4, 0.0, -0.4], [0.4, 0.4, 0.4], stone_color);
            // 2 wooden posts
            add_box(&mut mesh, [-0.03, 0.4, -0.3], [0.03, 1.0, -0.24], wood_color);
            add_box(&mut mesh, [-0.03, 0.4, 0.24], [0.03, 1.0, 0.3], wood_color);
            // Roof
            add_pyramid(&mut mesh, [-0.45, -0.45], [0.45, 0.45], 1.0, 1.4, roof_color);
        }
        "structure.campfire" => {
            let log_color = [0.35, 0.2, 0.08, 1.0];
            let fire_color = [0.95, 0.35, 0.05, 1.0];
            // Log 1
            add_box(&mut mesh, [-0.4, 0.0, -0.1], [0.4, 0.12, 0.1], log_color);
            // Log 2 (crossed)
            add_box(&mut mesh, [-0.1, 0.0, -0.4], [0.1, 0.12, 0.4], log_color);
            // Fire cone
            add_pyramid(&mut mesh, [-0.18, -0.18], [0.18, 0.18], 0.12, 0.55, fire_color);
        }
        "material.metal" => {
            let color = [0.75, 0.75, 0.8, 1.0];
            add_box(&mut mesh, [-0.25, 0.0, -0.12], [0.25, 0.08, 0.12], color);
        }
        "material.wood" => {
            let color = [0.45, 0.28, 0.12, 1.0];
            add_box(&mut mesh, [-0.3, 0.0, -0.1], [0.3, 0.08, 0.1], color);
        }
        "material.rope" => {
            let color = [0.72, 0.62, 0.48, 1.0];
            add_box(&mut mesh, [-0.2, 0.0, -0.2], [0.2, 0.06, -0.12], color);
            add_box(&mut mesh, [-0.2, 0.0, 0.12], [0.2, 0.06, 0.2], color);
            add_box(&mut mesh, [-0.2, 0.0, -0.12], [-0.12, 0.06, 0.12], color);
            add_box(&mut mesh, [0.12, 0.0, -0.12], [0.2, 0.06, 0.12], color);
        }
        "material.cloth" => {
            let color = [0.85, 0.85, 0.85, 1.0];
            add_box(&mut mesh, [-0.2, 0.0, -0.15], [0.2, 0.05, 0.15], color);
        }
        "weapon.knife" => {
            let handle_color = [0.4, 0.25, 0.1, 1.0];
            let blade_color = [0.7, 0.7, 0.72, 1.0];
            add_box(&mut mesh, [-0.15, 0.0, -0.03], [0.02, 0.06, 0.03], handle_color);
            add_box(&mut mesh, [0.02, 0.0, -0.02], [0.3, 0.05, 0.02], blade_color);
        }
        "weapon.spear" => {
            let shaft_color = [0.45, 0.28, 0.12, 1.0];
            let tip_color = [0.72, 0.72, 0.74, 1.0];
            // Shaft pointing along +X
            add_box(&mut mesh, [-0.5, 0.0, -0.02], [0.4, 0.04, 0.02], shaft_color);
            // Tip pointing along +X
            let tip_x = 0.55;
            let p_top = [tip_x, 0.0, 0.0];
            let b0 = [0.4, -0.04, -0.04];
            let b1 = [0.4, 0.04, -0.04];
            let b2 = [0.4, 0.04, 0.04];
            let b3 = [0.4, -0.04, 0.04];
            add_triangle(&mut mesh, b0, b1, p_top, tip_color);
            add_triangle(&mut mesh, b1, b2, p_top, tip_color);
            add_triangle(&mut mesh, b2, b3, p_top, tip_color);
            add_triangle(&mut mesh, b3, b0, p_top, tip_color);
        }
        "weapon.pistol" => {
            let color = [0.2, 0.2, 0.22, 1.0];
            add_box(&mut mesh, [-0.05, 0.0, -0.03], [0.03, 0.18, 0.03], color);
            add_box(&mut mesh, [-0.05, 0.15, -0.03], [0.2, 0.23, 0.03], color);
        }
        "weapon.sling" => {
            let color = [0.5, 0.32, 0.15, 1.0];
            add_box(&mut mesh, [-0.03, 0.0, -0.03], [0.03, 0.15, 0.03], color);
            add_box(&mut mesh, [-0.12, 0.15, -0.03], [-0.03, 0.25, 0.03], color);
            add_box(&mut mesh, [0.03, 0.15, -0.03], [0.12, 0.25, 0.03], color);
        }
        "weapon.rifle" => {
            let stock_color = [0.4, 0.25, 0.1, 1.0];
            let barrel_color = [0.2, 0.2, 0.22, 1.0];
            add_box(&mut mesh, [-0.25, 0.0, -0.04], [0.2, 0.1, 0.04], stock_color);
            add_box(&mut mesh, [-0.32, -0.08, -0.04], [-0.2, 0.05, 0.04], stock_color);
            add_box(&mut mesh, [0.2, 0.04, -0.02], [0.65, 0.08, 0.02], barrel_color);
        }
        "actor.player" => {
            let clothes_color = [0.15, 0.35, 0.75, 1.0];
            let skin_color = [0.95, 0.8, 0.65, 1.0];
            add_box(&mut mesh, [-0.18, 0.0, -0.1], [0.18, 0.45, 0.1], clothes_color);
            add_box(&mut mesh, [-0.22, 0.45, -0.12], [0.22, 0.95, 0.12], clothes_color);
            add_box(&mut mesh, [-0.12, 0.95, -0.12], [0.12, 1.22, 0.12], skin_color);
        }
        "actor.zombie.0" => {
            let zombie_skin = [0.25, 0.55, 0.3, 1.0];
            let clothes_color = [0.35, 0.35, 0.35, 1.0];
            add_box(&mut mesh, [-0.18, 0.0, -0.1], [0.18, 0.4, 0.1], clothes_color);
            add_box(&mut mesh, [-0.22, 0.4, -0.12], [0.22, 0.9, 0.12], clothes_color);
            add_box(&mut mesh, [-0.12, 0.9, -0.12], [0.12, 1.18, 0.12], zombie_skin);
            add_box(&mut mesh, [-0.06, 0.72, -0.38], [0.06, 0.82, -0.12], zombie_skin);
        }
        "actor.zombie.1" => {
            let zombie_skin = [0.2, 0.5, 0.25, 1.0];
            let clothes_color = [0.4, 0.28, 0.15, 1.0];
            add_box(&mut mesh, [-0.18, 0.0, -0.1], [0.18, 0.4, 0.1], clothes_color);
            add_box(&mut mesh, [-0.22, 0.4, -0.12], [0.22, 0.9, 0.12], clothes_color);
            add_box(&mut mesh, [-0.12, 0.9, -0.12], [0.12, 1.18, 0.12], zombie_skin);
            add_box(&mut mesh, [-0.06, 0.72, -0.38], [0.06, 0.82, -0.12], zombie_skin);
        }
        _ => {
            mesh = wedge_mesh();
        }
    }
    mesh
}

fn push_f32(bin: &mut Vec<u8>, values: &[f32]) -> (usize, usize) {
    let offset = bin.len();
    for value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    (offset, bin.len() - offset)
}

fn catalog(names: &[&str], actors: bool) -> Vec<u8> {
    let mut bin = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut meshes = Vec::new();

    for (i, name) in names.iter().enumerate() {
        let mesh_data = generate_mesh(name);

        let (po, pl) = push_f32(&mut bin, &mesh_data.positions);
        let (no, nl) = push_f32(&mut bin, &mesh_data.normals);
        let (co, cl) = push_f32(&mut bin, &mesh_data.colors);
        let io = bin.len();
        for idx in &mesh_data.indices {
            bin.extend_from_slice(&idx.to_le_bytes());
        }
        let il = bin.len() - io;
        while bin.len() % 4 != 0 {
            bin.push(0);
        }

        // Calculate min/max for positions
        let mut min_pos = [f32::INFINITY; 3];
        let mut max_pos = [f32::NEG_INFINITY; 3];
        for chunk in mesh_data.positions.chunks_exact(3) {
            for c in 0..3 {
                min_pos[c] = min_pos[c].min(chunk[c]);
                max_pos[c] = max_pos[c].max(chunk[c]);
            }
        }

        let base_view = views.len();
        views.push(json!({"buffer":0,"byteOffset":po,"byteLength":pl,"target":34962}));
        views.push(json!({"buffer":0,"byteOffset":no,"byteLength":nl,"target":34962}));
        views.push(json!({"buffer":0,"byteOffset":co,"byteLength":cl,"target":34962}));
        views.push(json!({"buffer":0,"byteOffset":io,"byteLength":il,"target":34963}));

        let base_acc = accessors.len();
        let pos_count = mesh_data.positions.len() / 3;
        let ind_count = mesh_data.indices.len();
        
        accessors.push(json!({"bufferView":base_view,"componentType":5126,"count":pos_count,"type":"VEC3","min":min_pos,"max":max_pos}));
        accessors.push(json!({"bufferView":base_view+1,"componentType":5126,"count":pos_count,"type":"VEC3"}));
        accessors.push(json!({"bufferView":base_view+2,"componentType":5126,"count":pos_count,"type":"VEC4"}));
        accessors.push(json!({"bufferView":base_view+3,"componentType":5123,"count":ind_count,"type":"SCALAR"}));

        meshes.push(json!({
            "name":format!("{name}.mesh"), "primitives":[{"attributes":{"POSITION":base_acc,"NORMAL":base_acc+1,"COLOR_0":base_acc+2},"indices":base_acc+3,"material":i,"mode":4}]
        }));
    }

    let acc_base = accessors.len();
    if actors {
        let (to, tl) = push_f32(&mut bin, &[0.0, 0.5, 1.0]);
        let (ro, rl) = push_f32(
            &mut bin,
            &[
                0.0, 0.0, 0.0, 1.0, 0.0, 0.1305262, 0.0, 0.9914449, 0.0, 0.0, 0.0, 1.0,
            ],
        );
        let base_view = views.len();
        views.push(json!({"buffer":0,"byteOffset":to,"byteLength":tl}));
        views.push(json!({"buffer":0,"byteOffset":ro,"byteLength":rl}));
        accessors.push(json!({"bufferView":base_view,"componentType":5126,"count":3,"type":"SCALAR","min":[0.0],"max":[1.0]}));
        accessors.push(json!({"bufferView":base_view+1,"componentType":5126,"count":3,"type":"VEC4"}));
    }

    let materials: Vec<_> = names.iter().enumerate().map(|(i,n)| {
        let tint = 0.55 + (i % 4) as f64 * 0.08;
        json!({"name":format!("{n}.material"),"pbrMetallicRoughness":{"baseColorFactor":[tint,0.38,0.18,1.0],"metallicFactor":0.0,"roughnessFactor":0.92}})
    }).collect();

    let mut nodes: Vec<Value> = names
        .iter()
        .enumerate()
        .map(|(i, n)| json!({"name":*n,"mesh":i}))
        .collect();
    if actors {
        nodes.push(json!({"name":"socket.hand","translation":[0.55,0.65,-0.15]}));
    }
    let scenes: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let children = if actors && i == 0 {
                vec![i, names.len()]
            } else {
                vec![i]
            };
            json!({"name":*n,"nodes":children})
        })
        .collect();
    let animations: Vec<_> = if actors {
        ACTOR_ANIMATIONS
            .iter()
            .map(|name| {
                let channels: Vec<_> = (0..names.len())
                    .map(|node| json!({"sampler":0,"target":{"node":node,"path":"rotation"}}))
                    .collect();
                json!({
                    "name":*name,"samplers":[{"input":acc_base,"output":acc_base+1,"interpolation":"LINEAR"}],
                    "channels":channels
                })
            })
            .collect()
    } else {
        vec![]
    };
    let root_value = json!({
        "asset":{"version":"2.0","generator":"rs-zombies gen_assets"},
        "scene":0,"scenes":scenes,"nodes":nodes,"meshes":meshes,"materials":materials,
        "buffers":[{"byteLength":bin.len()}],"bufferViews":views,"accessors":accessors,
        "animations":animations
    });
    // Deserialize through gltf-json so schema/type mistakes fail generation.
    let root: gltf_json::Root = serde_json::from_value(root_value).expect("valid glTF document");
    let json = serde_json::to_vec(&root).expect("serialize glTF document");
    let glb = Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            length: 0,
        },
        json: Cow::Owned(json),
        bin: Some(Cow::Owned(bin)),
    };
    let mut bytes = Vec::new();
    glb.to_writer(&mut bytes).expect("write GLB");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_catalogs_parse_and_cover_names() {
        for (name, bytes) in catalogs() {
            let gltf = gltf::Gltf::from_slice(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(gltf.scenes().all(|s| s.name().is_some()));
            assert!(gltf.materials().all(
                |m| m.name().is_some() && m.pbr_metallic_roughness().roughness_factor() >= 0.9
            ));
            if name == "actors.glb" {
                let names: Vec<_> = gltf.animations().filter_map(|a| a.name()).collect();
                assert_eq!(names, ACTOR_ANIMATIONS);
                assert!(
                    gltf.animations()
                        .all(|animation| animation.channels().count() == 3)
                );
                assert!(gltf.nodes().any(|n| n.name() == Some("socket.hand")));
            }
        }
    }
    #[test]
    fn output_is_byte_deterministic() {
        assert_eq!(catalogs(), catalogs());
    }
    #[test]
    fn required_catalog_names_are_complete() {
        let generated = catalogs();
        let expected = [
            FLORA_KINDS.len(),
            STRUCTURE_KINDS.len(),
            Material::ALL.len() + WeaponKind::ALL.len(),
            3,
        ];
        for ((_, bytes), count) in generated.iter().zip(expected) {
            assert_eq!(
                gltf::Gltf::from_slice(bytes).unwrap().scenes().count(),
                count
            );
        }
    }
}
