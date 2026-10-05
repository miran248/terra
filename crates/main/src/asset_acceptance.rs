//! Opt-in production acceptance walkthrough; compiled only with `asset-review`.
use crate::map::{Player, SunLock, TimeOfDay};
use avian3d::prelude::*;
use bevy::{
    camera::{ImageRenderTarget, RenderTarget},
    prelude::*,
    render::render_resource::TextureFormat,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use shared::{level::LevelData, state::AppState, terrain::Terrain};
use terra_geometry::sphere::SpherePos;
use std::path::PathBuf;

mod diagnostic_drag;
mod planet_acceptance;
mod sidebar_diagnostic;
mod transition_diagnostic;

const SIDEBAR_DIAGNOSTIC_ENV: &str = "TERRA_PLANET_SIDEBAR_DIAGNOSTIC";

#[cfg(feature = "asset-review-schedule-trace")]
mod phase_trace;
#[cfg(not(feature = "asset-review-schedule-trace"))]
mod phase_trace {
    pub(super) fn enabled() -> bool {
        false
    }
}

#[cfg(feature = "asset-review-schedule-trace")]
pub(crate) fn schedule_trace_layer(app: &mut App) -> Option<bevy::log::BoxedLayer> {
    phase_trace::install_layer(app)
}

pub struct AssetAcceptancePlugin;
impl Plugin for AssetAcceptancePlugin {
    fn build(&self, app: &mut App) {
        if background_capture_enabled() {
            assert!(
                (std::env::var_os("TERRA_PLANET_TRANSITION_DIAGNOSTIC").is_some()
                    || std::env::var_os(SIDEBAR_DIAGNOSTIC_ENV).is_some())
                    && std::env::var_os("TERRA_PLANET_ACCEPTANCE_CAPTURE").is_none(),
                "background capture is restricted to the transition or sidebar diagnostic"
            );
            app.add_systems(PreUpdate, configure_background_capture_target);
        }
        if let Some(directory) = std::env::var_os(SIDEBAR_DIAGNOSTIC_ENV) {
            assert!(
                [
                    "TERRA_PLANET_TRANSITION_DIAGNOSTIC",
                    "TERRA_PLANET_ACCEPTANCE_CAPTURE",
                    "TERRA_PRODUCTION_CAPTURE",
                    "TERRA_LIGHTING_CAPTURE",
                    "TERRA_FLIGHT_CAPTURE",
                ]
                .into_iter()
                .all(|name| std::env::var_os(name).is_none()),
                "TERRA_PLANET_SIDEBAR_DIAGNOSTIC cannot run beside another capture mode"
            );
            sidebar_diagnostic::register_sidebar_diagnostic(app, PathBuf::from(directory));
            return;
        }
        if let Some(directory) = std::env::var_os("TERRA_PLANET_TRANSITION_DIAGNOSTIC") {
            assert!(
                [
                    "TERRA_PLANET_ACCEPTANCE_CAPTURE",
                    "TERRA_PRODUCTION_CAPTURE",
                    "TERRA_LIGHTING_CAPTURE",
                    "TERRA_FLIGHT_CAPTURE",
                ]
                .into_iter()
                .all(|name| std::env::var_os(name).is_none()),
                "TERRA_PLANET_TRANSITION_DIAGNOSTIC cannot run beside another capture mode"
            );
            transition_diagnostic::register_transition_diagnostic(app, PathBuf::from(directory));
            return;
        }
        if let Some(directory) = std::env::var_os("TERRA_PLANET_ACCEPTANCE_CAPTURE") {
            assert!(
                [
                    "TERRA_PRODUCTION_CAPTURE",
                    "TERRA_LIGHTING_CAPTURE",
                    "TERRA_FLIGHT_CAPTURE",
                ]
                .into_iter()
                .all(|name| std::env::var_os(name).is_none()),
                "TERRA_PLANET_ACCEPTANCE_CAPTURE cannot run beside another capture mode"
            );
            planet_acceptance::register(app, PathBuf::from(directory));
            return;
        }
        if std::env::var_os("TERRA_PRODUCTION_CAPTURE").is_some() {
            app.add_systems(Update, capture.run_if(in_state(AppState::Playing)));
        }
    }
}

pub(crate) fn background_capture_enabled() -> bool {
    std::env::var("TERRA_PLANET_CAPTURE_BACKGROUND").is_ok_and(|value| value == "1")
}

pub(crate) fn window_plugin(background_capture: bool) -> bevy::window::WindowPlugin {
    let mut plugin = bevy::window::WindowPlugin::default();
    if background_capture && let Some(window) = &mut plugin.primary_window {
        window.focused = false;
        window.visible = false;
    }
    plugin
}

#[derive(Resource, Clone)]
pub(super) struct BackgroundCaptureTarget {
    pub image: Handle<Image>,
    pub physical_size: UVec2,
    pub logical_size: Vec2,
    pub scale_factor: f32,
}

fn configure_background_capture_target(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<Entity, With<crate::map::MainCamera>>,
    target: Option<Res<BackgroundCaptureTarget>>,
    mut configured: Local<bool>,
) {
    if *configured || target.is_some() {
        return;
    }
    let (Ok(window), Ok(camera)) = (windows.single(), cameras.single()) else {
        return;
    };
    let (physical_size, logical_size, scale_factor) = background_target_layout(window);
    if physical_size.min_element() == 0 {
        return;
    }
    let handle = images.add(Image::new_target_texture(
        physical_size.x,
        physical_size.y,
        TextureFormat::Rgba8Unorm,
        Some(TextureFormat::Rgba8UnormSrgb),
    ));
    commands.entity(camera).insert((
        RenderTarget::Image(ImageRenderTarget {
            handle: handle.clone(),
            scale_factor,
        }),
        bevy::ui::IsDefaultUiCamera,
    ));
    commands.insert_resource(BackgroundCaptureTarget {
        image: handle,
        physical_size,
        logical_size,
        scale_factor,
    });
    *configured = true;
}

fn background_target_layout(window: &Window) -> (UVec2, Vec2, f32) {
    (
        UVec2::new(window.physical_width(), window.physical_height()),
        Vec2::new(window.width(), window.height()),
        window.scale_factor().max(f32::EPSILON),
    )
}

pub(super) fn diagnostic_screenshot(world: &World) -> Option<Screenshot> {
    if background_capture_enabled() {
        world
            .get_resource::<BackgroundCaptureTarget>()
            .map(background_screenshot)
    } else {
        Some(Screenshot::primary_window())
    }
}

fn background_screenshot(target: &BackgroundCaptureTarget) -> Screenshot {
    Screenshot(RenderTarget::Image(ImageRenderTarget {
        handle: target.image.clone(),
        scale_factor: target.scale_factor,
    }))
}

pub(super) fn background_capture_description(world: &World) -> String {
    match world.get_resource::<BackgroundCaptureTarget>() {
        Some(target) => format!(
            "image={}x{} logical={:.1}x{:.1} scale_factor={} visible_window=false update=continuous",
            target.physical_size.x,
            target.physical_size.y,
            target.logical_size.x,
            target.logical_size.y,
            target.scale_factor,
        ),
        None if background_capture_enabled() => "image=not-ready visible_window=false".into(),
        None => "primary-window".into(),
    }
}

#[cfg(test)]
mod background_capture_tests {
    use bevy::{
        camera::{ImageRenderTarget, RenderTarget},
        math::{UVec2, Vec2},
        prelude::{Handle, Image},
        window::Window,
    };

    #[test]
    fn background_runner_creates_an_unfocused_hidden_primary_window() {
        let background = super::window_plugin(true)
            .primary_window
            .expect("asset review keeps a primary window for synthetic input");
        assert!(!background.visible);
        assert!(!background.focused);

        let normal = super::window_plugin(false)
            .primary_window
            .expect("normal runner keeps its primary window");
        assert!(normal.visible);
        assert!(normal.focused);
    }

    #[test]
    fn offscreen_target_keeps_primary_window_physical_size_and_scale() {
        let mut window = Window::default();
        window.resolution.set_scale_factor_override(Some(2.0));
        window.resolution.set(320.0, 180.0);

        let (physical, logical, scale_factor) = super::background_target_layout(&window);
        assert_eq!(logical, Vec2::new(320.0, 180.0));
        assert_eq!(physical, UVec2::new(640, 360));
        assert_eq!(scale_factor, 2.0);
    }

    #[test]
    fn screenshot_uses_the_same_normalized_image_target_as_the_main_camera() {
        let target = super::BackgroundCaptureTarget {
            image: Handle::<Image>::default(),
            physical_size: UVec2::new(640, 360),
            logical_size: Vec2::new(320.0, 180.0),
            scale_factor: 2.0,
        };
        let camera_target = RenderTarget::Image(ImageRenderTarget {
            handle: target.image.clone(),
            scale_factor: target.scale_factor,
        });
        let screenshot = super::background_screenshot(&target);

        assert_eq!(screenshot.0.normalize(None), camera_target.normalize(None));
    }
}
struct Stop {
    name: String,
    position: Vec3,
}
#[derive(Default)]
struct Review {
    stops: Vec<Stop>,
    index: usize,
    elapsed: f32,
    started: bool,
    frames: Vec<f32>,
    captured: bool,
    min_clearance: f32,
}

#[allow(
    clippy::too_many_arguments,
    reason = "isolated opt-in acceptance fixture"
)]
fn capture(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut state: Local<Review>,
    mut player: Query<(&mut Position, &mut LinearVelocity, &mut Player)>,
    meshes: Query<&ViewVisibility, With<Mesh3d>>,
    ground: Res<crate::map::CollisionTerrain>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut sun: ResMut<TimeOfDay>,
    mut sun_lock: ResMut<SunLock>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((mut position, mut velocity, mut player)) = player.single_mut() else {
        return;
    };
    if state.stops.is_empty() {
        let level =
            LevelData::from_artifact_bytes(include_bytes!("../assets/level_1337.bin")).unwrap();
        state.stops.push(Stop {
            name: "settlement".into(),
            position: position.0,
        });
        for biome in [
            Terrain::Forest,
            Terrain::Jungle,
            Terrain::Desert,
            Terrain::Snow,
            Terrain::Swamp,
        ] {
            let mut density = std::collections::BTreeMap::<usize, usize>::new();
            for item in &level.scenery {
                if level.face_types[item.face as usize] == biome {
                    *density.entry(item.face as usize).or_default() += 1;
                }
            }
            if let Some((&face, _)) = density
                .iter()
                .max_by_key(|(face, count)| (**count, std::cmp::Reverse(**face)))
            {
                let tri = level.terrain_tris[face].map(Vec3::from_array);
                let surface = (tri[0] + tri[1] + tri[2]) / 3.;
                state.stops.push(Stop {
                    name: format!("{biome:?}").to_lowercase(),
                    position: surface + surface.normalize() * 0.7,
                });
            }
        }
        if std::env::var_os("TERRA_PRODUCTION_FIRST_ONLY").is_some() {
            state.stops.truncate(1);
        }
        std::fs::create_dir_all("/tmp/terra-production-review").unwrap();
    }
    if !state.started {
        position.0 = state.stops[state.index].position;
        velocity.0 = Vec3::ZERO;
        let up = position.0.normalize();
        player.heading = SpherePos::new(up).tangent_basis().1;
        sun.angle = up.z.atan2(up.x);
        sun_lock.0 = true;
        state.min_clearance = f32::INFINITY;
        state.started = true;
        info!(
            stop = state.stops[state.index].name,
            "production acceptance stop"
        );
    }
    let surface_radius = ground
        .0
        .facet_radius(position.0.normalize(), terra_geometry::sphere::PLANET_RADIUS);
    state.min_clearance = state
        .min_clearance
        .min(position.0.length() - surface_radius);
    state.elapsed += time.delta_secs();
    if state.elapsed > 2. && state.elapsed < 4. && std::env::var_os("TERRA_CAPTURE_STILL").is_none()
    {
        keys.press(KeyCode::KeyW);
    } else {
        keys.release(KeyCode::KeyW);
    }
    if state.elapsed > 8. {
        state.frames.push(time.delta_secs() * 1000.);
    }
    if state.elapsed > 12. && !state.captured {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!(
                "/tmp/terra-production-review/{}.png",
                state.stops[state.index].name
            )));
        let mut frames = state.frames.clone();
        frames.sort_by(f32::total_cmp);
        let p50 = frames[frames.len() / 2];
        let p95 = frames[frames.len() * 95 / 100];
        info!(
            stop = state.stops[state.index].name,
            p50_ms = p50,
            p95_ms = p95,
            minimum_body_clearance = state.min_clearance,
            visible_meshes = meshes.iter().filter(|v| v.get()).count(),
            total_meshes = meshes.iter().count(),
            "PRODUCTION_ACCEPTANCE"
        );
        state.captured = true;
    }
    if state.elapsed > 13. {
        state.index += 1;
        if state.index == state.stops.len() {
            exit.write(AppExit::Success);
            return;
        }
        state.elapsed = 0.;
        state.frames.clear();
        state.started = false;
        state.captured = false;
    }
}
