use bevy::prelude::*;
use shared::state::AppState;
use crate::constants::*;

#[derive(Component)]
pub struct Ground;

#[derive(Component)]
pub struct Survivor {
    pub hp: f32,
    pub fire_timer: Timer,
    pub damage: f32,
    pub range: f32,
}

#[derive(Resource)]
pub struct SurvivorHp(pub f32);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Playing), setup_map)
            .add_systems(Update, (move_survivor, camera_follow).run_if(in_state(AppState::Playing)));
    }
}

fn setup_map(mut commands: Commands) {
    commands.insert_resource(SurvivorHp(SURVIVOR_HP));

    commands.spawn((
        Sprite::from_color(GROUND_COLOR, Vec2::new(MAP_WIDTH, MAP_HEIGHT)),
        Ground,
    ));

    commands.spawn((
        Sprite::from_color(SURVIVOR_COLOR, Vec2::new(SURVIVOR_SIZE, SURVIVOR_SIZE)),
        Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)),
        Survivor {
            hp: SURVIVOR_HP,
            fire_timer: Timer::from_seconds(ATTACK_INTERVAL, TimerMode::Repeating),
            damage: ATTACK_DAMAGE,
            range: ATTACK_RANGE,
        },
    ));
}

fn move_survivor(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut survivor_q: Query<&mut Transform, With<Survivor>>,
) {
    let Ok(mut tf) = survivor_q.single_mut() else { return };
    const SPEED: f32 = 200.0;
    let dt = time.delta_secs();
    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) { dir.y += 1.0; }
    if keys.pressed(KeyCode::KeyS) { dir.y -= 1.0; }
    if keys.pressed(KeyCode::KeyA) { dir.x -= 1.0; }
    if keys.pressed(KeyCode::KeyD) { dir.x += 1.0; }
    if dir != Vec2::ZERO {
        dir = dir.normalize();
        tf.translation.x += dir.x * SPEED * dt;
        tf.translation.y += dir.y * SPEED * dt;
    }
    let hw = (MAP_WIDTH - SURVIVOR_SIZE) / 2.0;
    let hh = (MAP_HEIGHT - SURVIVOR_SIZE) / 2.0;
    tf.translation.x = tf.translation.x.clamp(-hw, hw);
    tf.translation.y = tf.translation.y.clamp(-hh, hh);
}

fn camera_follow(
    survivor_q: Query<&Transform, With<Survivor>>,
    mut camera_q: Query<&mut Transform, (With<Camera2d>, Without<Survivor>)>,
) {
    let Ok(survivor_tf) = survivor_q.single() else { return };
    let Ok(mut cam_tf) = camera_q.single_mut() else { return };
    cam_tf.translation.x = survivor_tf.translation.x;
    cam_tf.translation.y = survivor_tf.translation.y;
}
