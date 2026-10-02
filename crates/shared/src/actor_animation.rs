//! Bind each imported actor to its own clips, with short visual-only transitions.
use crate::art::ACTOR_ANIMATIONS;
use bevy::{gltf::Gltf, prelude::*};
use std::{collections::HashMap, time::Duration};

#[derive(Component)]
pub struct ActorPlayback {
    pub source: Handle<Gltf>,
    pub action: usize,
    pub paused: bool,
}

#[derive(Component)]
struct PlayingAction(usize, AssetId<Gltf>);

#[derive(Resource, Default)]
struct Graphs(HashMap<AssetId<Gltf>, (Handle<AnimationGraph>, Vec<AnimationNodeIndex>)>);

pub struct ActorAnimationPlugin;
impl Plugin for ActorAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Graphs>().add_systems(Update, playback);
    }
}

fn playback(
    mut commands: Commands,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut cache: ResMut<Graphs>,
    roots: Query<&ActorPlayback>,
    parents: Query<&ChildOf>,
    mut players: Query<(
        Entity,
        &mut AnimationPlayer,
        Option<&PlayingAction>,
        Option<&mut AnimationTransitions>,
    )>,
) {
    for (entity, mut player, playing, transitions) in &mut players {
        let Some(request) = std::iter::once(entity)
            .chain(parents.iter_ancestors(entity))
            .find_map(|ancestor| roots.get(ancestor).ok())
        else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.0.entry(request.source.id())
        {
            let Some(gltf) = gltfs.get(&request.source) else {
                continue;
            };
            let Some(clips) = ACTOR_ANIMATIONS
                .iter()
                .map(|name| gltf.named_animations.get(*name).cloned())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let (graph, nodes) = AnimationGraph::from_clips(clips);
            entry.insert((graphs.add(graph), nodes));
        }
        let (graph, nodes) = &cache.0[&request.source.id()];
        let action = request.action.min(ACTOR_ANIMATIONS.len() - 1);
        if playing.is_none_or(|playing| playing.0 != action || playing.1 != request.source.id()) {
            if let Some(mut transitions) = transitions {
                transitions
                    .play(&mut player, nodes[action], Duration::from_millis(120))
                    .repeat();
            } else {
                let mut transitions = AnimationTransitions::new();
                transitions
                    .play(&mut player, nodes[action], Duration::ZERO)
                    .repeat();
                commands.entity(entity).insert(transitions);
            }
            commands.entity(entity).insert((
                AnimationGraphHandle(graph.clone()),
                PlayingAction(action, request.source.id()),
            ));
        }
        if request.paused {
            player.pause_all();
        } else {
            player.resume_all();
        }
    }
}
