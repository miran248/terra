#[cfg(feature = "asset-review")]
mod asset_acceptance;
mod asset_catalog;
mod asset_collision;
#[cfg(feature = "asset-review")]
mod asset_showcase_prototype;
#[cfg(all(feature = "car-prototype", any(not(feature = "plane-prototype"), test)))]
mod car_prototype;
mod chunks;
#[cfg(feature = "plane-prototype")]
mod plane_prototype;
// mod combat;
mod constants;
// mod loot;
mod map;
mod minimap;
#[cfg(all(
    feature = "on-foot-prototype",
    not(any(feature = "car-prototype", feature = "plane-prototype"))
))]
mod on_foot_prototype;
mod physics;
// mod prestige;
// mod turret;
mod foliage;
mod shader_motion;
mod ui;
mod water;
// mod wave;
mod weather;
// mod zombie;

use avian3d::prelude::*;
use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::light::Atmosphere;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::pbr::AtmosphereSettings;
use bevy::post_process::bloom::{Bloom, BloomPrefilter};
use bevy::prelude::*;
use shared::sphere::PLANET_RADIUS;
use shared::state::AppState;

fn main() {
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins,
        PhysicsPlugins::default()
            .build()
            .disable::<PhysicsInterpolationPlugin>(),
        PhysicsDiagnosticsPlugin,
        PhysicsDiagnosticsUiPlugin,
        FrameTimeDiagnosticsPlugin::default(),
        // Draws collider wireframes (a sphere on the player capsule + terrain
        // trimesh triangles), which read as grainy speckle around the player.
        // Re-enable when debugging physics.
        // PhysicsDebugPlugin,
    ))
    .insert_resource(SubstepCount(12))
    .insert_resource(PhysicsDiagnosticsUiSettings {
        enabled: false,
        ..default()
    })
    // Soft sky-blue fill so the shadowed sides of terrain and flora read as
    // lit rather than pure black. The sun (map.rs) still does the key light.
    // Cool constant fill that reads as moonlight on the night hemisphere
    // (the day side is dominated by the ~13000-lux sun, so this mostly shows
    // at night). Global, so it can't track the hemispheres itself.
    .insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.5, 0.62, 0.9),
        brightness: 150.0,
        ..default()
    })
    // Sits behind the atmosphere (space). Must be dark, otherwise it shows
    // through the dark night-side sky and makes night look bright. During the
    // day the atmosphere covers it.
    .insert_resource(ClearColor(Color::srgb(0.02, 0.03, 0.07)))
    .init_state::<AppState>()
    .add_plugins((
        asset_catalog::AssetCatalogPlugin,
        map::MapPlugin,
        // wave::WavePlugin,
        // zombie::ZombiePlugin,
        // turret::TurretPlugin,
        // combat::CombatPlugin,
        // loot::LootPlugin,
        ui::UiPlugin,
        minimap::MinimapPlugin,
        // prestige::PrestigePlugin,
        physics::PhysicsPlugin,
        weather::WeatherPlugin,
        shader_motion::ShaderMotionPlugin,
        water::WaterPlugin,
        foliage::FoliagePlugin,
    ))
    .add_systems(Startup, setup_camera);
    #[cfg(feature = "asset-review")]
    app.add_plugins((
        asset_showcase_prototype::AssetShowcasePlugin,
        asset_acceptance::AssetAcceptancePlugin,
    ));
    #[cfg(all(
        feature = "on-foot-prototype",
        not(any(feature = "car-prototype", feature = "plane-prototype"))
    ))]
    app.add_plugins(on_foot_prototype::OnFootPrototypePlugin);
    #[cfg(all(feature = "car-prototype", not(feature = "plane-prototype")))]
    app.add_plugins(car_prototype::CarPrototypePlugin);
    #[cfg(feature = "plane-prototype")]
    app.add_plugins(plane_prototype::PlanePrototypePlugin);
    app.run();
}

fn setup_camera(mut commands: Commands, mut media: ResMut<Assets<ScatteringMedium>>) {
    // Physically-based atmosphere. The earth scattering coefficients are
    // per-metre and assume a ~60 km air column; our planet is only
    // PLANET_RADIUS (2000) units with a thin SHELL-unit atmosphere, so the
    // optical path is ~60_000 / SHELL times shorter. Scale the densities up by
    // that factor so the horizon still glows. Tune SHELL / the multiplier by eye.
    // The planet is small and curves away fast, so a thin shell makes the blue
    // dome barely taller than the terrain horizon (sky "cuts out" before the
    // skyline). A tall shell relative to the radius keeps blue down to the
    // horizon and up to the zenith.
    const SHELL: f32 = 900.0;
    // A full 60_000/SHELL match oversaturates into gray/brown haze that swallows
    // the whole planet at the horizon; a low, flat multiplier keeps a blue sky
    // and lets distant land stay visible rather than washing out to sky.
    let medium = media.add(ScatteringMedium::earth(256, 256).with_density_multiplier(50.0));
    // Planet is centred at the world origin (unlike the default surface-relative
    // placement), so pin the Atmosphere entity there with an identity Transform.
    commands.spawn((
        Atmosphere {
            // Drop the atmosphere's "ground" below the water surface so the sky
            // dome engulfs the planet down past the visible horizon instead of
            // ending in a band above it. This is deep (-96) to hide a band that a
            // proper water material would otherwise hide itself (depth-limited
            // visibility); it can rise toward ~-32 once water shades correctly.
            inner_radius: PLANET_RADIUS - 24.0,
            outer_radius: PLANET_RADIUS + SHELL,
            // Low albedo → less white multiscattered light bouncing back up, so
            // the sky reads as a deeper blue rather than a pale/gray blue.
            ground_albedo: Vec3::splat(0.1),
            medium,
        },
        Transform::default(),
    ));

    commands.spawn((
        Camera3d::default(),
        map::MainCamera,
        Hdr,
        Tonemapping::TonyMcMapface,
        // The ordered deband dither shows up as static grain on the smoothest
        // surfaces (the capsule + nearby ground); disable it. HDR + atmosphere
        // already have enough tonal range to avoid banding.
        DebandDither::Disabled,
        // Thresholded bloom: only genuinely bright pixels (emissive campfire,
        // projectiles) glow, not the whole daylit scene. The old player-centred
        // grain was the sun hotspot riding the player, which is gone now that the
        // sun is a fixed world body — so bloom is safe to run again.
        Bloom {
            intensity: 0.18,
            prefilter: BloomPrefilter {
                threshold: 0.7,
                threshold_softness: 0.4,
            },
            ..Bloom::NATURAL
        },
        // The dense flora packed around the player is a field of sub-pixel
        // triangles that shimmer (grain) as the camera moves — a temporal
        // problem MSAA can't fix. TAA resolves it over frames. TAA requires MSAA
        // off (they don't combine).
        Msaa::Off,
        TemporalAntiAliasing::default(),
        // (No AtmosphereEnvironmentMapLight: its sky IBL is global — it lit every
        // surface, causing the pre-dawn "glow" on stones/flora, and it can't be
        // scoped to water only. Removed. Water keeps its sun-glint specular; a
        // water-only sky reflection would be computed in the water shader.)
        AtmosphereSettings {
            // The whole world spans ~2000 units; shrink the aerial-perspective
            // range from Earth's 32 km so distance haze reads at this scale, but
            // keep it moderate: far enough to blend the near horizon, but not so
            // far that haze swallows the whole planet into sky.
            aerial_view_lut_max_distance: 2500.0,
            // The aerial-perspective LUT is a coarse froxel volume recomputed each
            // frame, densest right in front of the camera (where the player sits),
            // so its in-scattering shimmers/flickers on near geometry. Raise both
            // its resolution (esp. depth slices) and the per-froxel sample count
            // to smooth it out.
            aerial_view_lut_size: UVec3::new(24, 24, 32),
            aerial_view_lut_samples: 24,
            sky_view_lut_samples: 16,
            ..default()
        },
        // Distance fog coloured to the horizon sky, kept moderate so the far
        // ocean fades into the sky at the waterline (softening the hard sky/ocean
        // divide) and props dissolve into haze at the scenery cull band (grass ~90
        // … trees ~550, see map::scenery_cull) instead of popping.
        DistanceFog {
            color: Color::srgb(0.7, 0.8, 0.92),
            falloff: FogFalloff::from_visibility_colors(
                1700.0,
                Color::srgb(0.7, 0.8, 0.92),
                Color::srgb(0.7, 0.8, 0.92),
            ),
            ..default()
        },
        Transform::from_xyz(0.0, 900.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
    ));
}
