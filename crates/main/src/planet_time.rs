//! One place owns the base simulation speed and Planet view's zoom contribution.

use crate::{
    exploration::{Exploration, ExplorationUpdate},
    map::MainCamera,
};
use bevy::prelude::*;
use bevy::time::TimeSystems;

/// The base simulation rate combined with the attained Planet view zoom.
#[derive(Resource, Clone, Debug)]
pub struct PlanetSimulationClock {
    base_rate: f64,
    attained_radius: f32,
    planet_view_active: bool,
}

impl Default for PlanetSimulationClock {
    fn default() -> Self {
        Self {
            base_rate: 1.0,
            attained_radius: shared::sphere::PLANET_RADIUS,
            planet_view_active: false,
        }
    }
}

impl PlanetSimulationClock {
    /// Change the rate requested by gameplay time controls.
    pub fn set_base_rate(&mut self, rate: f64) {
        assert!(rate.is_finite() && rate >= 0.0);
        self.base_rate = rate;
    }

    /// Publish the collision-limited radial distance reached by the main camera.
    pub fn publish_planet_view(&mut self, active: bool, attained_radius: f32) {
        self.planet_view_active = active;
        self.attained_radius = if attained_radius.is_finite() {
            attained_radius.max(0.0)
        } else {
            shared::planet_view::PLANET_VIEW_NEAR_RADIUS
        };
    }

    /// Return the single effective rate to apply to Bevy's virtual clock.
    pub fn effective_rate(&self) -> f64 {
        let zoom = if self.planet_view_active {
            let near = f64::from(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
            let far = f64::from(shared::planet_view::PLANET_VIEW_FAR_RADIUS);
            ((f64::from(self.attained_radius) - near) / (far - near)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let smoothstep = zoom * zoom * (3.0 - 2.0 * zoom);
        self.base_rate * (1.0 - 0.5 * smoothstep)
    }
}

pub struct PlanetTimePlugin;

impl Plugin for PlanetTimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetSimulationClock>()
            .add_systems(First, apply_planet_simulation_rate.before(TimeSystems))
            .add_systems(Update, publish_planet_view_radius.after(ExplorationUpdate));
    }
}

fn publish_planet_view_radius(
    exploration: Option<Res<Exploration>>,
    camera: Query<&Transform, With<MainCamera>>,
    mut clock: ResMut<PlanetSimulationClock>,
) {
    let Some(exploration) = exploration else {
        return;
    };
    let active = exploration.is_planet_view_active();
    let near_radius = shared::planet_view::PLANET_VIEW_NEAR_RADIUS;
    let attained_radius = camera
        .single()
        .map_or(near_radius, |camera| camera.translation.length());
    clock.publish_planet_view(active, attained_radius);
}

fn apply_planet_simulation_rate(
    clock: Res<PlanetSimulationClock>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    virtual_time.set_relative_speed_f64(clock.effective_rate());
}

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    #[test]
    fn planet_view_rate_uses_attained_zoom_and_multiplies_the_base_rate() {
        let mut clock = PlanetSimulationClock::default();
        clock.set_base_rate(2.0);

        for (radius, expected) in [(2024.0, 2.0), (4012.0, 1.5), (6000.0, 1.0)] {
            clock.publish_planet_view(true, radius);
            assert_eq!(clock.effective_rate(), expected);
        }
    }

    #[test]
    fn planet_view_rate_clamps_zoom_and_decreases_monotonically() {
        let mut clock = PlanetSimulationClock::default();
        clock.set_base_rate(2.0);
        let expected = [2.0, 1.84375, 1.5, 1.15625, 1.0];

        let rates = [1000.0, 3018.0, 4012.0, 5006.0, 9000.0]
            .into_iter()
            .map(|radius| {
                clock.publish_planet_view(true, radius);
                clock.effective_rate()
            })
            .collect::<Vec<_>>();

        assert_eq!(rates, expected);
        assert!(rates.windows(2).all(|pair| pair[0] > pair[1]));
    }

    #[test]
    fn a_blocked_zoom_request_changes_speed_only_after_attainment() {
        let mut camera = shared::planet_view::PlanetViewCamera::default();
        camera.record_attained_radius(shared::planet_view::PLANET_VIEW_FAR_RADIUS);
        camera.request_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
        assert_eq!(
            camera.requested_radius(),
            shared::planet_view::PLANET_VIEW_NEAR_RADIUS
        );

        let mut clock = PlanetSimulationClock::default();
        clock.publish_planet_view(true, camera.attained_radius());
        assert_eq!(clock.effective_rate(), 0.5);
        clock.publish_planet_view(true, camera.attained_radius());
        assert_eq!(clock.effective_rate(), 0.5);

        camera.record_attained_radius(4012.0);
        clock.publish_planet_view(true, camera.attained_radius());
        assert_eq!(clock.effective_rate(), 0.75);
        camera.record_attained_radius(shared::planet_view::PLANET_VIEW_NEAR_RADIUS);
        clock.publish_planet_view(true, camera.attained_radius());
        assert_eq!(clock.effective_rate(), 1.0);

        clock.publish_planet_view(false, shared::planet_view::PLANET_VIEW_FAR_RADIUS);
        assert_eq!(clock.effective_rate(), 1.0);
    }

    #[test]
    fn base_rate_changes_and_view_exit_do_not_compound_or_override_other_time() {
        let mut clock = PlanetSimulationClock::default();
        clock.publish_planet_view(true, 6000.0);
        assert_eq!(clock.effective_rate(), 0.5);

        clock.set_base_rate(2.0);
        assert_eq!(clock.effective_rate(), 1.0);
        clock.publish_planet_view(true, 6000.0);
        assert_eq!(clock.effective_rate(), 1.0);

        clock.publish_planet_view(false, 6000.0);
        assert_eq!(clock.effective_rate(), 2.0);
    }

    #[test]
    fn virtual_clock_uses_the_published_rate_on_the_next_advance() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, PlanetTimePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));

        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(100)
        );

        app.world_mut()
            .resource_mut::<PlanetSimulationClock>()
            .publish_planet_view(true, 4012.0);
        app.update();

        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            0.75
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(75)
        );
    }

    #[test]
    fn planet_view_speed_uses_the_camera_transform_as_attained_radius() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, PlanetTimePlugin))
            .init_resource::<Exploration>()
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));
        app.world_mut()
            .spawn((MainCamera, Transform::from_translation(Vec3::Y * 4012.0)));
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);

        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            0.75
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(75)
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            1.0
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(100)
        );
    }

    #[test]
    fn changing_rate_preserves_pause_and_reapplies_the_base_without_compounding() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, PlanetTimePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )));
        app.update();
        app.update();

        let mut clock = app.world_mut().resource_mut::<PlanetSimulationClock>();
        clock.set_base_rate(2.0);
        clock.publish_planet_view(true, 4012.0);
        drop(clock);
        app.update();
        app.update();
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            1.5
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(150)
        );

        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        app.world_mut()
            .resource_mut::<PlanetSimulationClock>()
            .set_base_rate(4.0);
        app.update();
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(
            app.world().resource::<Time<Virtual>>().relative_speed(),
            3.0
        );
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::ZERO
        );

        app.world_mut().resource_mut::<Time<Virtual>>().unpause();
        app.update();
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(
            app.world().resource::<Time<Virtual>>().delta(),
            Duration::from_millis(300)
        );
    }

    fn physics_distance_after(zoom: f32, updates: usize) -> f32 {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TransformPlugin,
            PhysicsPlugins::default(),
            PlanetTimePlugin,
        ))
        .insert_resource(Gravity::ZERO)
        .insert_resource(SubstepCount(12))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )));
        app.world_mut()
            .resource_mut::<PlanetSimulationClock>()
            .publish_planet_view(true, if zoom == 0.0 { 2024.0 } else { 6000.0 });
        let body = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.25),
                Position(Vec3::ZERO),
                Rotation::default(),
                Transform::default(),
                LinearVelocity(Vec3::X),
            ))
            .id();
        app.finish();
        app.cleanup();

        for _ in 0..updates {
            app.update();
        }
        app.world().get::<Position>(body).unwrap().0.x
    }

    #[test]
    fn physics_motion_matches_for_equal_virtual_durations_at_near_and_far_zoom() {
        let near = physics_distance_after(0.0, 101);
        let far = physics_distance_after(1.0, 201);

        assert!((near - 1.0).abs() < 0.02, "near: {near}");
        assert!((far - 1.0).abs() < 0.02, "far: {far}");
        assert!((near - far).abs() < 0.02, "near: {near}, far: {far}");
    }
}
