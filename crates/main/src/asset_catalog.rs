//! Load the complete approved meter-scale catalog before entering the world.
use bevy::{gltf::Gltf, prelude::*};
use shared::{
    actor_animation::{ActorAnimationPlugin, ActorPlayback},
    art::{asset_names, asset_path},
    state::AppState,
};
use std::collections::HashMap;

#[derive(Resource)]
pub struct AssetCatalog {
    sources: HashMap<String, Handle<Gltf>>,
    pub scenes: HashMap<String, Handle<WorldAsset>>,
}

impl AssetCatalog {
    #[cfg(test)]
    pub fn fixture(names: &[&str]) -> Self {
        Self {
            sources: HashMap::new(),
            scenes: names
                .iter()
                .map(|name| ((*name).to_owned(), Handle::default()))
                .collect(),
        }
    }
    pub fn scene(&self, name: &str) -> Handle<WorldAsset> {
        self.scenes
            .get(name)
            .unwrap_or_else(|| panic!("missing asset scene {name}"))
            .clone()
    }

    pub fn actor(&self, name: &str, action: usize) -> ActorPlayback {
        ActorPlayback {
            source: self.sources[name].clone(),
            action,
            paused: false,
        }
    }
}

pub struct AssetCatalogPlugin;
impl Plugin for AssetCatalogPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ActorAnimationPlugin)
            .add_systems(Startup, begin_loading)
            .add_systems(Update, finish_loading.run_if(in_state(AppState::Loading)));
    }
}

fn begin_loading(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(AssetCatalog {
        sources: asset_names()
            .into_iter()
            .map(|name| {
                let handle = server.load(asset_path(&name));
                (name, handle)
            })
            .collect(),
        scenes: HashMap::new(),
    });
}

fn finish_loading(
    mut catalog: ResMut<AssetCatalog>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut next: ResMut<NextState<AppState>>,
) {
    if !catalog
        .sources
        .values()
        .all(|handle| server.is_loaded_with_dependencies(handle))
    {
        return;
    }
    let scenes = catalog
        .sources
        .iter()
        .map(|(name, handle)| {
            let gltf = gltfs.get(handle).expect("loaded catalog source");
            let scene = gltf
                .named_scenes
                .get(name.as_str())
                .unwrap_or_else(|| panic!("missing canonical scene {name}"));
            (name.clone(), scene.clone())
        })
        .collect();
    catalog.scenes = scenes;
    next.set(AppState::Playing);
}

#[cfg(test)]
mod tests {
    #[test]
    fn complete_catalog_has_unique_paths_and_meter_contracts() {
        let names = shared::art::asset_names();
        assert_eq!(names.len(), 68);
        let paths: std::collections::HashSet<_> = names
            .iter()
            .map(|name| shared::art::asset_path(name))
            .collect();
        assert_eq!(paths.len(), 68);
        for name in names {
            assert!(
                shared::asset_contract::candidate_contract(&name).is_some(),
                "{name}"
            );
        }
    }
}
