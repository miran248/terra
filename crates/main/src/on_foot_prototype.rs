//! Throwaway in-game comparison for “Tune on-foot movement and camera”.

use avian3d::prelude::*;
use bevy::prelude::*;
use shared::{on_foot_prototype::OnFootPrototype, state::AppState};

use crate::{map::Player, ui::UiFont};

pub struct OnFootPrototypePlugin;

#[derive(Component)]
struct PrototypeReadout;

impl Plugin for OnFootPrototypePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OnFootPrototype>()
            .add_systems(OnEnter(AppState::Playing), setup)
            .add_systems(PreUpdate, switch_mode.run_if(in_state(AppState::Playing)))
            .add_systems(
                Update,
                (follow_camera, readout)
                    .chain()
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup(mut commands: Commands, font: Res<UiFont>) {
    commands.spawn((
        PrototypeReadout,
        Text::new("ON-FOOT PROTOTYPE"),
        TextFont {
            font: font.0.clone().into(),
            font_size: 16.0.into(),
            ..default()
        },
        TextColor(shared::theme::INK),
        BackgroundColor(shared::theme::PANEL_BG),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: px(240),
            padding: UiRect::all(px(10)),
            ..default()
        },
    ));
}

fn switch_mode(keys: Res<ButtonInput<KeyCode>>, mut tuning: ResMut<OnFootPrototype>) {
    if keys.just_pressed(KeyCode::F6) {
        tuning.baseline = !tuning.baseline;
    }
}

fn follow_camera(
    tuning: Res<OnFootPrototype>,
    spatial: SpatialQuery,
    player: Query<(Entity, &Player, &Transform), Without<crate::map::MainCamera>>,
    mut camera: Query<&mut Transform, (With<crate::map::MainCamera>, Without<Player>)>,
) {
    if tuning.baseline {
        return;
    }
    let Ok((entity, player, transform)) = player.single() else {
        return;
    };
    let Ok(mut camera) = camera.single_mut() else {
        return;
    };
    let origin = transform.translation;
    let desired = tuning.camera(origin, player.heading, None);
    let offset = desired.translation - origin;
    // A sphere protects the near plane as well as the camera center. Exclude
    // the player's own capsule; terrain, ice, bridges and props can obstruct.
    let hit = spatial.cast_shape(
        &Collider::sphere(0.2),
        origin,
        Quat::IDENTITY,
        Dir3::new(offset).expect("nonzero camera boom"),
        &ShapeCastConfig::from_max_distance(offset.length()),
        &SpatialQueryFilter::from_excluded_entities([entity]),
    );
    *camera = tuning.camera(origin, player.heading, hit.map(|hit| hit.distance));
}

fn readout(
    tuning: Res<OnFootPrototype>,
    player: Query<(&Transform, &LinearVelocity), With<Player>>,
    camera: Query<&Transform, (With<crate::map::MainCamera>, Without<Player>)>,
    mut text: Query<&mut Text, With<PrototypeReadout>>,
) {
    let Ok(mut text) = text.single_mut() else {
        return;
    };
    let Ok((player, velocity)) = player.single() else {
        return;
    };
    let Ok(camera) = camera.single() else {
        return;
    };
    let up = player.translation.normalize();
    let tangent_speed = (velocity.0 - up * velocity.0.dot(up)).length();
    let (mode, framing) = if tuning.baseline {
        (
            "BASELINE",
            "height 4 / back 9 / ahead 45 m; original camera",
        )
    } else {
        (
            "PROPOSED",
            "height 2 / back 5 / ahead 10 m; obstruction sphere 0.2 m",
        )
    };
    **text = format!(
        "ON-FOOT PROTOTYPE — {mode} — F6 compare\nW/S move · A/D turn · Shift sprint · Space jump\nWalk {:.0} / sprint {:.0} m/s; water/ice ×0.4; jump unchanged\n{framing}\nActual ground speed {tangent_speed:.1} m/s · camera distance {:.2} m",
        tuning.speed(false, false),
        tuning.speed(true, false),
        camera.translation.distance(player.translation),
    );
}
