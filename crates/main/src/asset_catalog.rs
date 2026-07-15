use bevy::{gltf::Gltf, prelude::*};
use shared::{
    art::{
        ACTOR_ANIMATIONS, ACTORS_CATALOG, ENVIRONMENT_CATALOG, ITEMS_CATALOG, STRUCTURES_CATALOG,
    },
    state::AppState,
};
use std::collections::HashMap;

#[derive(Resource)]
pub struct AssetCatalog {
    catalogs: [Handle<Gltf>; 4],
    pub scenes: HashMap<String, Handle<WorldAsset>>,
    pub animations: HashMap<String, Handle<AnimationClip>>,
    animation_graph: Option<Handle<AnimationGraph>>,
    animation_nodes: Vec<AnimationNodeIndex>,
}

impl AssetCatalog {
    pub fn scene(&self, name: &str) -> Handle<WorldAsset> {
        self.scenes
            .get(name)
            .unwrap_or_else(|| panic!("missing asset scene {name}"))
            .clone()
    }
}

pub struct AssetCatalogPlugin;

impl Plugin for AssetCatalogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, begin_loading)
            .add_systems(Update, finish_loading.run_if(in_state(AppState::Loading)))
            .add_systems(
                Update,
                bind_actor_animations.run_if(in_state(AppState::Playing)),
            );
    }
}

fn begin_loading(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(AssetCatalog {
        catalogs: [
            server.load(ENVIRONMENT_CATALOG),
            server.load(STRUCTURES_CATALOG),
            server.load(ITEMS_CATALOG),
            server.load(ACTORS_CATALOG),
        ],
        scenes: HashMap::new(),
        animations: HashMap::new(),
        animation_graph: None,
        animation_nodes: Vec::new(),
    });
}

fn finish_loading(
    mut catalog: ResMut<AssetCatalog>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut next: ResMut<NextState<AppState>>,
) {
    let Some(loaded) = catalog
        .catalogs
        .iter()
        .map(|h| gltfs.get(h))
        .collect::<Option<Vec<_>>>()
    else {
        return;
    };
    for gltf in &loaded {
        for (name, scene) in &gltf.named_scenes {
            catalog.scenes.insert(name.to_string(), scene.clone());
        }
    }
    let actors = loaded[3];
    for name in ACTOR_ANIMATIONS {
        let clip = actors
            .named_animations
            .get(name)
            .unwrap_or_else(|| panic!("actors catalog is missing animation {name}"));
        catalog.animations.insert(name.to_owned(), clip.clone());
    }
    let clips = ACTOR_ANIMATIONS.map(|name| catalog.animations[name].clone());
    let (graph, nodes) = AnimationGraph::from_clips(clips);
    catalog.animation_graph = Some(graphs.add(graph));
    catalog.animation_nodes = nodes;
    next.set(AppState::Playing);
}

fn bind_actor_animations(
    mut commands: Commands,
    catalog: Res<AssetCatalog>,
    mut players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
) {
    let (Some(graph), Some(&idle)) = (
        catalog.animation_graph.as_ref(),
        catalog.animation_nodes.first(),
    ) else {
        return;
    };
    for (entity, mut player) in &mut players {
        player.play(idle).repeat();
        commands
            .entity(entity)
            .insert(AnimationGraphHandle(graph.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_paths_are_unique() {
        let mut paths = [
            ENVIRONMENT_CATALOG,
            STRUCTURES_CATALOG,
            ITEMS_CATALOG,
            ACTORS_CATALOG,
        ];
        paths.sort_unstable();
        assert!(paths.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
