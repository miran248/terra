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

fn push_f32(bin: &mut Vec<u8>, values: &[f32]) -> (usize, usize) {
    let offset = bin.len();
    for value in values {
        bin.extend_from_slice(&value.to_le_bytes());
    }
    (offset, bin.len() - offset)
}

fn catalog(names: &[&str], actors: bool) -> Vec<u8> {
    // A deliberately faceted, asymmetric low-poly wedge. Its bottom is Y=0 and
    // its nose points toward -Z, making orientation and pivot mistakes visible.
    let positions = [
        -0.5, 0.0, 0.5, 0.5, 0.0, 0.5, 0.42, 0.0, -0.6, -0.42, 0.0, -0.6, 0.0, 1.0, 0.10,
    ];
    let normals = [
        0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, -1.0, 0.0, 0.0, 1.0, 0.0,
    ];
    let indices: [u16; 18] = [0, 2, 1, 0, 3, 2, 0, 1, 4, 1, 2, 4, 2, 3, 4, 3, 0, 4];
    let colors = [
        0.82, 0.28, 0.12, 1.0, 0.95, 0.55, 0.16, 1.0, 0.55, 0.16, 0.08, 1.0, 0.70, 0.22, 0.10, 1.0,
        0.95, 0.72, 0.20, 1.0,
    ];
    let mut bin = Vec::new();
    let (po, pl) = push_f32(&mut bin, &positions);
    let (no, nl) = push_f32(&mut bin, &normals);
    let (co, cl) = push_f32(&mut bin, &colors);
    let io = bin.len();
    for i in indices {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let il = bin.len() - io;
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let (to, tl) = push_f32(&mut bin, &[0.0, 0.5, 1.0]);
    let (ro, rl) = push_f32(
        &mut bin,
        &[
            0.0, 0.0, 0.0, 1.0, 0.0, 0.1305262, 0.0, 0.9914449, 0.0, 0.0, 0.0, 1.0,
        ],
    );

    let mut views = vec![
        json!({"buffer":0,"byteOffset":po,"byteLength":pl,"target":34962}),
        json!({"buffer":0,"byteOffset":no,"byteLength":nl,"target":34962}),
        json!({"buffer":0,"byteOffset":co,"byteLength":cl,"target":34962}),
        json!({"buffer":0,"byteOffset":io,"byteLength":il,"target":34963}),
    ];
    let mut accessors = vec![
        json!({"bufferView":0,"componentType":5126,"count":5,"type":"VEC3","min":[-0.5,0.0,-0.6],"max":[0.5,1.0,0.5]}),
        json!({"bufferView":1,"componentType":5126,"count":5,"type":"VEC3"}),
        json!({"bufferView":2,"componentType":5126,"count":5,"type":"VEC4"}),
        json!({"bufferView":3,"componentType":5123,"count":18,"type":"SCALAR"}),
    ];
    if actors {
        views.push(json!({"buffer":0,"byteOffset":to,"byteLength":tl}));
        views.push(json!({"buffer":0,"byteOffset":ro,"byteLength":rl}));
        accessors.push(json!({"bufferView":4,"componentType":5126,"count":3,"type":"SCALAR","min":[0.0],"max":[1.0]}));
        accessors.push(json!({"bufferView":5,"componentType":5126,"count":3,"type":"VEC4"}));
    }
    let materials: Vec<_> = names.iter().enumerate().map(|(i,n)| {
        let tint = 0.55 + (i % 4) as f64 * 0.08;
        json!({"name":format!("{n}.material"),"pbrMetallicRoughness":{"baseColorFactor":[tint,0.38,0.18,1.0],"metallicFactor":0.0,"roughnessFactor":0.92}})
    }).collect();
    let meshes: Vec<_> = names.iter().enumerate().map(|(i,n)| json!({
        "name":format!("{n}.mesh"), "primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"COLOR_0":2},"indices":3,"material":i,"mode":4}]
    })).collect();
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
                    "name":*name,"samplers":[{"input":4,"output":5,"interpolation":"LINEAR"}],
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
