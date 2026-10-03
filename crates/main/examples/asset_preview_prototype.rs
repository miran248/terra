//! THROWAWAY: does a 1 m reference make catalog proportions easy to judge?
//! Three native layouts: inspection, baseline comparison, orthographic measurement.
//! Run with `just asset-preview`.
//! No game state or asset files are changed. Runtime scales below are a deliberate
//! snapshot of chunks.rs, map.rs, zombie.rs and loot.rs, not a new shared contract.
#[path = "../src/asset_collision.rs"]
mod asset_collision;

use avian3d::prelude::{Gravity, PhysicsDebugPlugin, PhysicsGizmos, PhysicsPlugins, RigidBody};
use bevy::{
    gltf::Gltf,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    text::FontSize,
};
use shared::actor_animation::{ActorAnimationPlugin, ActorPlayback};
use shared::art::{
    ACTOR_ANIMATIONS, ColliderSpec, SCENERY_KINDS, scenery_collider, scenery_variant_count,
    scenery_variant_name,
};
use std::collections::HashMap;

const PAGE: usize = 12;
const INK: Color = Color::srgb(0.86, 0.90, 0.94);
const PANEL: Color = Color::srgb(0.09, 0.12, 0.17);
const CYAN: Color = Color::srgb(0.35, 0.90, 0.88);

#[derive(Clone, Copy, Debug, PartialEq)]
enum ColliderSnapshot {
    None,
    Box { size: Vec3 },
    Sphere { radius: f32, center_y: f32 },
}
impl ColliderSnapshot {
    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Box { .. } => "box",
            Self::Sphere { .. } => "sphere",
        }
    }
}

struct VisualAsset {
    source: Handle<Gltf>,
    scene: Handle<WorldAsset>,
    min: Vec3,
    max: Vec3,
    runtime: Vec3,
}
impl VisualAsset {
    fn dimensions(&self) -> Vec3 {
        (self.max - self.min) * self.runtime
    }
}
struct Entry {
    name: String,
    baseline: VisualAsset,
    candidate: Option<VisualAsset>,
}
impl Entry {
    fn visual(&self, use_candidates: bool) -> &VisualAsset {
        if use_candidates {
            self.candidate.as_ref().unwrap_or(&self.baseline)
        } else {
            &self.baseline
        }
    }
}
#[derive(Resource)]
struct Workbench {
    entries: Vec<Entry>,
    selected: usize,
    page: usize,
    edits: HashMap<(usize, bool), Vec3>,
    layout: usize,
    view: usize,
    distance: f32,
    yaw: f32,
    pitch: f32,
    target: Vec3,
    overlay: bool,
    animation: usize,
    paused: bool,
    rebuild: bool,
    use_candidates: bool,
}
impl Workbench {
    fn reference_position(&self) -> Vec3 {
        // Keep the reference in front of the selected envelope, not inside houses.
        Vec3::new(
            self.offset() - 0.9,
            0.,
            -(self.dimensions().z * 0.5 + 0.5).max(1.0),
        )
    }
    fn edit_key(&self) -> (usize, bool) {
        (
            self.selected,
            self.use_candidates && self.entries[self.selected].candidate.is_some(),
        )
    }
    fn scale(&self) -> Vec3 {
        self.edits
            .get(&self.edit_key())
            .copied()
            .unwrap_or(Vec3::ONE)
    }
    fn dimensions(&self) -> Vec3 {
        self.entries[self.selected]
            .visual(self.use_candidates)
            .dimensions()
            * self.scale()
    }
    fn offset(&self) -> f32 {
        if self.layout == 1 {
            self.entries[self.selected]
                .baseline
                .dimensions()
                .x
                .max(self.dimensions().x)
                * 0.6
                + 0.8
        } else {
            0.0
        }
    }
}
#[derive(Component)]
struct Stage;
#[derive(Component)]
struct Hud;
#[derive(Component)]
struct PreviewCamera;
#[derive(Component)]
struct ReferenceRoot;
#[derive(Default, Reflect, GizmoConfigGroup)]
struct CollisionGizmos;
#[derive(Component)]
struct RulerLabel(Vec3);
#[derive(Component, Clone, Copy)]
enum Action {
    Select(usize),
    Page(i32),
    Layout(usize),
    View(usize),
    Dimension(usize, f32),
    Uniform(f32),
    Reset,
    Overlay,
    Animation,
    Pause,
    Zoom(f32),
    Source,
}

fn runtime_scale(name: &str) -> Vec3 {
    let xyz = match name {
        "structure.ruin" => [6., 3., 6.],
        "structure.watchtower" => [4.5, 12., 4.5],
        "structure.dock" => [3., 0.8, 10.],
        "structure.farm" => [9., 0.3, 9.],
        "structure.wall" => [6., 3., 1.2],
        "structure.well" => [2., 1.2, 2.],
        "structure.campfire" => [1.6; 3],
        "structure.tent" => [3., 2., 3.],
        "structure.crate" => [1.5; 3],
        "structure.fence" => [6., 1.5, 0.6],
        "structure.barricade" => [4., 1., 0.5],
        "structure.lamp_post" => [1.2, 3., 1.2],
        "structure.signpost" => [1.5, 2.5, 0.3],
        "structure.guardrail" => [3., 1., 0.4],
        "structure.railing" => [3., 1., 0.2],
        "structure.suspension" => [2., 6., 0.6],
        "structure.house" => [8., 5., 6.],
        "actor.player" => [0.55; 3],
        n if n.starts_with("actor.zombie") => [2.; 3],
        n if n.starts_with("weapon.") => [1.08; 3],
        n if n.starts_with("material.") => [0.6; 3],
        _ => [1.; 3],
    };
    Vec3::from_array(xyz)
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Terra | asset scale PROTOTYPE".into(),
                        resolution: (1440, 940).into(),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((
            PhysicsPlugins::default(),
            PhysicsDebugPlugin,
            ActorAnimationPlugin,
        ))
        .insert_resource(Gravity(Vec3::ZERO))
        .insert_gizmo_config(
            PhysicsGizmos {
                axis_lengths: None,
                collider_color: Some(Color::srgb(0.95, 0.72, 0.22)),
                sleeping_color_multiplier: None,
                ..default()
            },
            bevy::gizmos::config::GizmoConfig::default(),
        )
        .add_systems(Update, (candidate_overlay_visibility, attach_pilot_weapon))
        .insert_resource(ClearColor(Color::srgb(0.19, 0.23, 0.29)))
        .init_gizmo_group::<CollisionGizmos>()
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.8, 0.86, 1.),
            brightness: 350.,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (input, rebuild, animate, camera, place_reference, guides).chain(),
        )
        .add_systems(
            PostUpdate,
            labels.after(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            Update,
            capture.run_if(|| std::env::var_os("TERRA_PREVIEW_CAPTURE").is_some()),
        )
        .add_systems(
            Update,
            capture_scenery.run_if(|| std::env::var_os("TERRA_SCENERY_CAPTURE").is_some()),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut gizmo_config: ResMut<GizmoConfigStore>,
) {
    gizmo_config.config_mut::<CollisionGizmos>().0.depth_bias = -1.;
    let manifest =
        std::env::var("TERRA_PREVIEW_MANIFEST").expect("launch using asset_preview_prototype.py");
    let entries: Vec<_> = std::fs::read_to_string(manifest)
        .unwrap()
        .lines()
        .map(|line| {
            let parts: Vec<_> = line.split('\t').collect();
            let numbers: Vec<f32> = parts[2..8].iter().map(|s| s.parse().unwrap()).collect();
            Entry {
                name: parts[1].into(),
                baseline: VisualAsset {
                    source: server.load(parts[0].split('#').next().unwrap().to_owned()),
                    scene: server.load(parts[0].to_owned()),
                    min: Vec3::from_slice(&numbers[..3]),
                    max: Vec3::from_slice(&numbers[3..]),
                    runtime: runtime_scale(parts[1]),
                },
                candidate: if parts.len() == 15 {
                    let extent: Vec<f32> = parts[9..].iter().map(|s| s.parse().unwrap()).collect();
                    Some(VisualAsset {
                        source: server.load(parts[8].split('#').next().unwrap().to_owned()),
                        scene: server.load(parts[8].to_owned()),
                        min: Vec3::from_slice(&extent[..3]),
                        max: Vec3::from_slice(&extent[3..]),
                        runtime: Vec3::ONE,
                    })
                } else {
                    None
                },
            }
        })
        .collect();
    let selected = entries
        .iter()
        .position(|e| e.name == "actor.player")
        .unwrap();
    commands.insert_resource(Workbench {
        entries,
        selected,
        page: selected / PAGE,
        edits: default(),
        layout: 0,
        view: 0,
        distance: 5.5,
        yaw: 0.5,
        pitch: 0.2,
        target: Vec3::Y * 0.8,
        overlay: true,
        animation: 0,
        paused: true,
        rebuild: true,
        use_candidates: true,
    });
    commands.spawn((PreviewCamera, Camera3d::default(), Transform::default()));
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-5., 10., -6.).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(100., 100.))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.36, 0.37),
            perceptual_roughness: 1.,
            ..default()
        })),
        Transform::from_xyz(0., -0.015, 0.),
    ));
    // Exactly 1 m tall human-shaped scale mannequin, independent of current actors.
    let mannequin = materials.add(Color::srgb(0.91, 0.73, 0.59));
    let reference = commands
        .spawn((
            ReferenceRoot,
            Transform::from_xyz(-0.9, 0., -1.),
            Visibility::default(),
        ))
        .id();
    for (size, position) in [
        ([0.16, 0.20, 0.16], [0., 0.90, 0.]),
        ([0.25, 0.32, 0.14], [0., 0.62, 0.]),
        ([0.09, 0.42, 0.12], [-0.08, 0.21, 0.]),
        ([0.09, 0.42, 0.12], [0.08, 0.21, 0.]),
        ([0.075, 0.35, 0.10], [-0.18, 0.61, 0.]),
        ([0.075, 0.35, 0.10], [0.18, 0.61, 0.]),
    ] {
        let part = commands
            .spawn((
                Mesh3d(meshes.add(Cuboid::from_size(Vec3::from_array(size)))),
                MeshMaterial3d(mannequin.clone()),
                Transform::from_translation(Vec3::from_array(position)),
            ))
            .id();
        commands.entity(reference).add_child(part);
    }
}

fn apply(action: Action, work: &mut Workbench) {
    match action {
        Action::Source => {
            work.use_candidates = !work.use_candidates;
        }
        Action::Select(index) => {
            work.selected = index;
            work.page = index / PAGE;
        }
        Action::Page(delta) => {
            work.page = (work.page as i32 + delta)
                .rem_euclid(work.entries.len().div_ceil(PAGE) as i32)
                as usize
        }
        Action::Layout(layout) => {
            work.layout = layout;
            if layout == 2 {
                work.view = 1;
            }
        }
        Action::View(view) => work.view = view,
        Action::Dimension(axis, delta) => {
            let base = work.entries[work.selected]
                .visual(work.use_candidates)
                .dimensions();
            let mut scale = work.scale();
            scale[axis] =
                ((base[axis] * scale[axis] + delta).max(0.01) / base[axis]).clamp(0.001, 100.);
            work.edits.insert(work.edit_key(), scale);
        }
        Action::Uniform(factor) => {
            let scale = work.scale() * factor;
            work.edits.insert(
                work.edit_key(),
                scale.clamp(Vec3::splat(0.001), Vec3::splat(100.)),
            );
        }
        Action::Reset => {
            work.edits.remove(&work.edit_key());
        }
        Action::Overlay => work.overlay = !work.overlay,
        Action::Animation => work.animation = (work.animation + 1) % ACTOR_ANIMATIONS.len(),
        Action::Pause => work.paused = !work.paused,
        Action::Zoom(factor) => work.distance = (work.distance * factor).clamp(0.5, 150.),
    }
    work.rebuild = true;
}

fn input(
    mut work: ResMut<Workbench>,
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    for (interaction, action) in &buttons {
        if *interaction == Interaction::Pressed {
            apply(*action, &mut work);
        }
    }
    for (key, action) in [
        (KeyCode::Digit1, Action::Layout(0)),
        (KeyCode::Digit2, Action::Layout(1)),
        (KeyCode::Digit3, Action::Layout(2)),
        (KeyCode::KeyF, Action::View(1)),
        (KeyCode::KeyS, Action::View(2)),
        (KeyCode::KeyO, Action::View(0)),
        (KeyCode::KeyC, Action::Overlay),
        (KeyCode::Space, Action::Pause),
        (KeyCode::KeyA, Action::Animation),
        (KeyCode::KeyR, Action::Reset),
    ] {
        if keys.just_pressed(key) {
            apply(action, &mut work);
        }
    }
    if keys.just_pressed(KeyCode::ArrowLeft) || keys.just_pressed(KeyCode::ArrowRight) {
        let delta = if keys.just_pressed(KeyCode::ArrowLeft) {
            2
        } else {
            1
        };
        let layout = (work.layout + delta) % 3;
        apply(Action::Layout(layout), &mut work);
    }
    if mouse.pressed(MouseButton::Right) {
        if work.view != 0 {
            work.rebuild = true;
        }
        work.yaw -= motion.delta.x * 0.006;
        work.pitch = (work.pitch + motion.delta.y * 0.006).clamp(-0.15, 1.45);
        work.view = 0;
    }
    if mouse.pressed(MouseButton::Middle) {
        let distance = work.distance;
        let yaw = if work.view == 1 {
            0.
        } else if work.view == 2 {
            std::f32::consts::FRAC_PI_2
        } else {
            work.yaw
        };
        work.target += Vec3::new(yaw.cos(), 0., -yaw.sin()) * motion.delta.x * -distance * 0.001;
        work.target.y += motion.delta.y * distance * 0.001;
    }
    if scroll.delta.y != 0. {
        work.distance = (work.distance * (-scroll.delta.y * 0.08).exp()).clamp(0.5, 150.);
    }
}

fn text(commands: &mut Commands, value: impl Into<String>, size: f32) -> Entity {
    commands
        .spawn((
            Text::new(value),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(INK),
        ))
        .id()
}
fn button(commands: &mut Commands, parent: Entity, label: impl Into<String>, action: Action) {
    let child = text(commands, label, 12.);
    let entity = commands
        .spawn((
            Button,
            action,
            Node {
                padding: UiRect::axes(px(10), px(6)),
                margin: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.18, 0.24, 0.30)),
        ))
        .add_child(child)
        .id();
    commands.entity(parent).add_child(entity);
}
fn panel(commands: &mut Commands, node: Node) -> Entity {
    commands.spawn((Hud, node, BackgroundColor(PANEL))).id()
}
fn row(commands: &mut Commands, parent: Entity) -> Entity {
    let row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            ..default()
        })
        .id();
    commands.entity(parent).add_child(row);
    row
}

fn rebuild(
    mut commands: Commands,
    mut work: ResMut<Workbench>,
    old: Query<Entity, With<Hud>>,
    stage: Query<Entity, With<Stage>>,
    mut previous: Local<Option<(usize, Vec3, bool, bool)>>,
) {
    if !work.rebuild {
        return;
    }
    work.rebuild = false;
    for entity in &old {
        commands.entity(entity).despawn();
    }
    let entry = &work.entries[work.selected];
    let dimensions = work.dimensions();
    let offset = work.offset();
    let visual = entry.visual(work.use_candidates);
    let candidate = work.use_candidates && entry.candidate.is_some();
    let scale = visual.runtime * work.scale();
    // Ground and center each item for measurement.
    let placement = |x: f32, scale: Vec3, asset: &VisualAsset| {
        Vec3::new(x, 0., 0.)
            - Vec3::new(
                (asset.min.x + asset.max.x) * 0.5,
                asset.min.y,
                (asset.min.z + asset.max.z) * 0.5,
            ) * scale
    };
    let signature = (work.selected, scale, work.layout == 1, candidate);
    if *previous != Some(signature) {
        for entity in &stage {
            commands.entity(entity).despawn();
        }
        *previous = Some(signature);
        let mut root = commands.spawn((
            Stage,
            Transform::from_translation(placement(offset, scale, visual)).with_scale(scale),
            Visibility::default(),
        ));
        if candidate
            && let Some(collider) = asset_collision::candidate_collider(&entry.name, Vec3::ONE)
        {
            root.insert((RigidBody::Static, collider));
        }
        root.with_child((
            WorldAssetRoot(visual.scene.clone()),
            Transform::default(),
            ActorPlayback {
                source: visual.source.clone(),
                action: work.animation,
                paused: work.paused,
            },
        ));
        if work.layout == 1 {
            commands.spawn((
                Stage,
                WorldAssetRoot(entry.baseline.scene.clone()),
                ActorPlayback {
                    source: entry.baseline.source.clone(),
                    action: work.animation,
                    paused: work.paused,
                },
                Transform::from_translation(placement(
                    -offset,
                    entry.baseline.runtime,
                    &entry.baseline,
                ))
                .with_scale(entry.baseline.runtime),
            ));
        }
    }
    let header = panel(
        &mut commands,
        Node {
            position_type: PositionType::Absolute,
            top: px(12),
            left: px(14),
            right: px(14),
            padding: UiRect::all(px(12)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
    );
    let label = text(
        &mut commands,
        format!(
            "THROWAWAY  /  {}  /  {} / {}",
            ["INSPECT", "BASELINE vs EDIT", "MEASURE"][work.layout],
            entry.name,
            if candidate {
                "BLENDER CANDIDATE"
            } else {
                "CURRENT BASELINE"
            }
        ),
        20.,
    );
    commands.entity(header).add_child(label);
    let state = text(
        &mut commands,
        format!(
            "W {:.3} m    H {:.3} m    D {:.3} m    |    baseline {:.3} x {:.3} x {:.3} m    |    edit {:.3} x {:.3} x {:.3}\n{} / {}    |    {} {}    |    edits in memory only",
            dimensions.x,
            dimensions.y,
            dimensions.z,
            entry.baseline.dimensions().x,
            entry.baseline.dimensions().y,
            entry.baseline.dimensions().z,
            work.scale().x,
            work.scale().y,
            work.scale().z,
            work.selected + 1,
            work.entries.len(),
            ACTOR_ANIMATIONS[work.animation],
            if work.paused { "(paused)" } else { "(playing)" }
        ),
        14.,
    );
    commands.entity(header).add_child(state);
    let list = panel(
        &mut commands,
        Node {
            position_type: PositionType::Absolute,
            left: px(14),
            top: px(128),
            width: px(if work.layout == 1 { 220 } else { 260 }),
            padding: UiRect::all(px(8)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
    );
    let page_row = row(&mut commands, list);
    button(&mut commands, page_row, "<", Action::Page(-1));
    button(
        &mut commands,
        page_row,
        format!(
            "Page {}/{} >",
            work.page + 1,
            work.entries.len().div_ceil(PAGE)
        ),
        Action::Page(1),
    );
    for (i, asset) in work.entries.iter().enumerate().filter(|(_, asset)| {
        asset.candidate.is_some()
            && matches!(
                asset.name.as_str(),
                "actor.player"
                    | "structure.house"
                    | "scenery.tree.0"
                    | "scenery.rock.0"
                    | "weapon.knife"
            )
    }) {
        button(
            &mut commands,
            list,
            format!("PILOT: {}", asset.name),
            Action::Select(i),
        );
    }
    for (i, asset) in work
        .entries
        .iter()
        .enumerate()
        .skip(work.page * PAGE)
        .take(PAGE)
    {
        button(
            &mut commands,
            list,
            format!(
                "{} {}",
                if i == work.selected { ">" } else { " " },
                asset.name
            ),
            Action::Select(i),
        );
    }
    let controls = panel(
        &mut commands,
        Node {
            position_type: PositionType::Absolute,
            right: px(14),
            top: px(128),
            width: px(244),
            padding: UiRect::all(px(10)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
    );
    for (axis, name) in ["Width X", "Height Y", "Depth Z"].into_iter().enumerate() {
        let label = text(
            &mut commands,
            format!("{name}: {:.3} m", dimensions[axis]),
            15.,
        );
        commands.entity(controls).add_child(label);
        let r = row(&mut commands, controls);
        for (label, delta) in [
            ("-10cm", -0.1),
            ("-1cm", -0.01),
            ("+1cm", 0.01),
            ("+10cm", 0.1),
        ] {
            button(&mut commands, r, label, Action::Dimension(axis, delta));
        }
    }
    let r = row(&mut commands, controls);
    button(&mut commands, r, "All x0.9", Action::Uniform(0.9));
    button(&mut commands, r, "All x1.1", Action::Uniform(1.1));
    button(
        &mut commands,
        controls,
        "Reset selected asset [R]",
        Action::Reset,
    );
    button(
        &mut commands,
        controls,
        "Visual bounds / current collider [C]",
        Action::Overlay,
    );
    button(
        &mut commands,
        controls,
        "Candidate / baseline",
        Action::Source,
    );
    let collision_status = text(
        &mut commands,
        if candidate {
            format!(
                "Visual bounds: measurement only\nGold: actual candidate collider{}",
                if work.layout == 1 {
                    "\nRed: current baseline collider"
                } else {
                    ""
                }
            )
        } else {
            overlay_status(&entry.name, entry.baseline.runtime, work.layout == 1)
        },
        12.,
    );
    commands.entity(controls).add_child(collision_status);
    button(
        &mut commands,
        controls,
        "Idle / walk / attack [A]",
        Action::Animation,
    );
    button(
        &mut commands,
        controls,
        "Play / pause [Space]",
        Action::Pause,
    );
    let bottom = panel(
        &mut commands,
        Node {
            position_type: PositionType::Absolute,
            bottom: px(12),
            left: percent(22),
            right: percent(22),
            padding: UiRect::all(px(8)),
            flex_direction: FlexDirection::Column,
            ..default()
        },
    );
    let r = row(&mut commands, bottom);
    for (i, label) in ["1 Inspect", "2 Compare", "3 Measure"]
        .into_iter()
        .enumerate()
    {
        button(&mut commands, r, label, Action::Layout(i));
    }
    let r = row(&mut commands, bottom);
    for (i, label) in ["Orbit [O]", "Front [F]", "Side [S]"]
        .into_iter()
        .enumerate()
    {
        button(&mut commands, r, label, Action::View(i));
    }
    button(&mut commands, r, "Zoom +", Action::Zoom(0.8));
    button(&mut commands, r, "Zoom -", Action::Zoom(1.25));
    let hint = text(
        &mut commands,
        "Right drag: orbit | Middle drag: pan | Wheel: zoom\nCyan: visual bounds (measurement only) | Red: current collider\nCompare shows the baseline collider; no red outline means none\nCandidate physics shapes. Reference humanoid: 1 m. Grid: 1 m.",
        13.,
    );
    commands.entity(bottom).add_child(hint);
    let ruler_x = offset + dimensions.x * 0.5 + 0.35;
    let step = if dimensions.y > 10. {
        2.
    } else if dimensions.y > 3. {
        1.
    } else {
        0.25
    };
    for index in 0..=((dimensions.y.max(1.) / step).ceil() as usize).min(60) {
        let y = index as f32 * step;
        let label = text(&mut commands, format!("{y:.2} m"), 12.);
        let position = if work.view == 2 {
            Vec3::new(offset, y, size_for_side(dimensions) + 0.45)
        } else {
            Vec3::new(ruler_x + 0.1, y, 0.)
        };
        commands.entity(label).insert((
            Hud,
            RulerLabel(position),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ));
    }
    for (value, position) in [
        (
            "1 m reference".to_string(),
            work.reference_position() + Vec3::Y * 1.15,
        ),
        (
            format!(
                "{} | W {:.2} m / D {:.2} m",
                if candidate { "CANDIDATE" } else { "EDIT" },
                dimensions.x,
                dimensions.z
            ),
            Vec3::new(offset, -0.15, 0.),
        ),
    ] {
        let label = text(&mut commands, value, 13.);
        commands.entity(label).insert((
            Hud,
            RulerLabel(position),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ));
    }
    if work.layout == 1 {
        let label = text(&mut commands, "CURRENT BASELINE", 13.);
        commands.entity(label).insert((
            Hud,
            RulerLabel(Vec3::new(-offset, -0.15, 0.)),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
        ));
    }
}

fn animate(work: Res<Workbench>, mut roots: Query<&mut ActorPlayback>) {
    for mut playback in &mut roots {
        playback.action = work.animation;
        playback.paused = work.paused;
    }
}

fn camera(
    work: Res<Workbench>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<PreviewCamera>>,
) {
    for (mut transform, mut projection) in &mut cameras {
        let (yaw, pitch) = match work.view {
            1 => (0., 0.),
            2 => (std::f32::consts::FRAC_PI_2, 0.),
            _ => (work.yaw, work.pitch),
        };
        let direction = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            -yaw.cos() * pitch.cos(),
        );
        *transform = Transform::from_translation(work.target + direction * work.distance)
            .looking_at(work.target, Vec3::Y);
        *projection = if work.view == 0 {
            Projection::Perspective(PerspectiveProjection::default())
        } else {
            Projection::Orthographic(OrthographicProjection {
                scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                    viewport_height: work.distance * 0.65,
                },
                ..OrthographicProjection::default_3d()
            })
        };
    }
}
fn place_reference(
    work: Res<Workbench>,
    mut reference: Single<&mut Transform, With<ReferenceRoot>>,
) {
    reference.translation = work.reference_position();
}
fn wire_box<T: GizmoConfigGroup>(gizmos: &mut Gizmos<T>, center: Vec3, size: Vec3, color: Color) {
    for axis in 0..3 {
        for a in [-0.5, 0.5] {
            for b in [-0.5, 0.5] {
                let mut from = center;
                let mut to = center;
                from[axis] -= size[axis] * 0.5;
                to[axis] += size[axis] * 0.5;
                for point in [&mut from, &mut to] {
                    point[(axis + 1) % 3] += size[(axis + 1) % 3] * a;
                    point[(axis + 2) % 3] += size[(axis + 2) % 3] * b;
                }
                gizmos.line(from, to, color);
            }
        }
    }
}
fn guides(work: Res<Workbench>, mut gizmos: Gizmos, mut collisions: Gizmos<CollisionGizmos>) {
    let grid = Color::srgb(0.44, 0.51, 0.52);
    for n in -30..=30 {
        gizmos.line(
            Vec3::new(n as f32, 0., -30.),
            Vec3::new(n as f32, 0., 30.),
            grid,
        );
        gizmos.line(
            Vec3::new(-30., 0., n as f32),
            Vec3::new(30., 0., n as f32),
            grid,
        );
    }
    let size = work.dimensions();
    let offset = work.offset();
    for sign in [-1., 1.] {
        let x = offset + sign * (size.x * 0.5 + 0.35);
        let point = |y, tick| {
            if work.view == 2 {
                Vec3::new(offset, y, sign * (size.z * 0.5 + 0.35) + tick)
            } else {
                Vec3::new(x + tick, y, 0.)
            }
        };
        gizmos.line(point(0., 0.), point(size.y.max(1.), 0.), INK);
        let step = if size.y > 10. {
            2.
        } else if size.y > 3. {
            1.
        } else {
            0.25
        };
        for i in 0..=((size.y.max(1.) / step).ceil() as usize).min(60) {
            let y = i as f32 * step;
            gizmos.line(point(y, -0.08), point(y, 0.08), INK);
        }
    }
    if work.overlay {
        wire_box(&mut gizmos, Vec3::new(offset, size.y * 0.5, 0.), size, CYAN);
        if work.layout == 1 {
            let baseline = work.entries[work.selected].baseline.dimensions();
            wire_box(
                &mut gizmos,
                Vec3::new(-offset, baseline.y * 0.5, 0.),
                baseline,
                CYAN,
            );
        }
        if work.layout == 1
            || !work.use_candidates
            || work.entries[work.selected].candidate.is_none()
        {
            current_collision(
                &mut collisions,
                &work.entries[work.selected],
                if work.layout == 1 { -offset } else { offset },
            );
        }
    }
}
fn size_for_side(size: Vec3) -> f32 {
    size.z * 0.5
}

fn current_collision(gizmos: &mut Gizmos<CollisionGizmos>, entry: &Entry, x: f32) {
    let red = Color::srgb(1., 0.35, 0.45);
    let origin = Vec3::new(x, 0., 0.)
        - Vec3::new(
            (entry.baseline.min.x + entry.baseline.max.x) * 0.5,
            entry.baseline.min.y,
            (entry.baseline.min.z + entry.baseline.max.z) * 0.5,
        ) * entry.baseline.runtime;
    match collider_snapshot(&entry.name, entry.baseline.runtime) {
        ColliderSnapshot::None => {}
        ColliderSnapshot::Box { size } => wire_box(gizmos, origin, size, red),
        ColliderSnapshot::Sphere { radius, center_y } => {
            gizmos.sphere(
                Isometry3d::from_translation(origin + Vec3::Y * center_y),
                radius,
                red,
            );
        }
    }
}

fn collider_snapshot(name: &str, runtime: Vec3) -> ColliderSnapshot {
    if name.starts_with("structure.") {
        if matches!(
            name,
            "structure.farm" | "structure.campfire" | "structure.tent"
        ) {
            ColliderSnapshot::None
        } else {
            ColliderSnapshot::Box {
                size: runtime * 0.5,
            }
        }
    } else if name.starts_with("scenery.") {
        SCENERY_KINDS
            .into_iter()
            .find(|kind| {
                (0..scenery_variant_count(*kind)).any(|v| scenery_variant_name(*kind, v) == name)
            })
            .map(scenery_collider)
            .and_then(|collider| match collider {
                ColliderSpec::Box { half_extents } => Some(ColliderSnapshot::Box {
                    // Preserve the constructor arguments used by the runtime,
                    // including values named `half_extents` that are passed as
                    // full side lengths to Avian's cuboid constructor.
                    size: Vec3::from_array(half_extents),
                }),
                ColliderSpec::None | ColliderSpec::Capsule { .. } => None,
            })
            .unwrap_or(ColliderSnapshot::None)
    } else {
        let (radius, center_y) = if name == "actor.player" {
            (0.275, -0.28)
        } else if name.starts_with("actor.") {
            (1., 1.)
        } else if name.starts_with("weapon.") {
            (0.54, 0.54)
        } else {
            (0.3, 0.3)
        };
        ColliderSnapshot::Sphere { radius, center_y }
    }
}

fn overlay_status(name: &str, runtime: Vec3, comparison: bool) -> String {
    let collider = collider_snapshot(name, runtime);
    let location = if comparison && collider != ColliderSnapshot::None {
        " (shown on baseline)"
    } else {
        ""
    };
    format!(
        "Visual bounds: measurement only\nCurrent collider: {}{}",
        collider.label(),
        location
    )
}

// Every scenery candidate instantiated in Bevy, including nonblocking foliage.
fn capture_scenery(
    mut commands: Commands,
    time: Res<Time<Real>>,
    server: Res<AssetServer>,
    mut work: ResMut<Workbench>,
    mut phase: Local<usize>,
    mut next_at: Local<f32>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if now < 4. || now < *next_at {
        return;
    }
    let scenery: Vec<usize> = work
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.name.starts_with("scenery.") && e.candidate.is_some())
        .map(|(i, _)| i)
        .collect();
    assert_eq!(scenery.len(), 37);
    if *phase >= scenery.len() * 2 {
        println!("SCENERY PREVIEW: all 37 candidates imported, instantiated and captured");
        exit.write(AppExit::Success);
        return;
    }
    let selected = scenery[*phase / 2];
    let asset = work.entries[selected].candidate.as_ref().unwrap();
    if !server.is_loaded_with_dependencies(&asset.scene) {
        return;
    }
    if (*phase).is_multiple_of(2) {
        work.use_candidates = true;
        apply(Action::Select(selected), &mut work);
        apply(Action::Reset, &mut work);
        work.layout = 0;
        work.view = 0;
        work.overlay = true;
        let size = work.dimensions();
        work.distance = (size.max_element() * 2.7).max(2.2);
        work.target = Vec3::Y * size.y * 0.5;
        *next_at = now + 0.6;
    } else {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!(
                "/tmp/terra-scenery-review/{}.png",
                work.entries[selected].name
            )));
        *next_at = now + 0.15;
    }
    *phase += 1;
}

// Opt-in visual smoke walkthrough, not part of the normal interactive workflow.
fn capture(
    mut commands: Commands,
    time: Res<Time<Real>>,
    server: Res<AssetServer>,
    mut work: ResMut<Workbench>,
    mut phase: Local<usize>,
    mut exit: MessageWriter<AppExit>,
) {
    let seconds = time.elapsed_secs();
    if seconds > 4. && *phase == 0 {
        let loaded = work
            .entries
            .iter()
            .filter(|e| {
                server.is_loaded_with_dependencies(&e.baseline.scene)
                    && e.candidate
                        .as_ref()
                        .is_none_or(|c| server.is_loaded_with_dependencies(&c.scene))
            })
            .count();
        println!("PREVIEW: {loaded}/{} scenes loaded", work.entries.len());
        assert_eq!(loaded, work.entries.len(), "catalog load incomplete");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-inspect.png"));
        *phase = 1;
    }
    if seconds > 6. && *phase == 1 {
        let selected = work
            .entries
            .iter()
            .position(|e| e.name == "structure.house")
            .unwrap();
        apply(Action::Select(selected), &mut work);
        apply(Action::Layout(1), &mut work);
        if work.entries[selected].candidate.is_none() {
            apply(Action::Uniform(0.65), &mut work);
        }
        work.distance = 35.;
        work.target = Vec3::Y * 2.;
        *phase = 2;
    }
    if seconds > 8. && *phase == 2 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-compare.png"));
        *phase = 3;
    }
    if seconds > 10. && *phase == 3 {
        let selected = work
            .entries
            .iter()
            .position(|e| e.name == "scenery.tree.0")
            .unwrap();
        apply(Action::Select(selected), &mut work);
        apply(Action::Layout(0), &mut work);
        work.distance = 16.;
        work.target = Vec3::Y * 3.;
        *phase = 4;
    }
    if seconds > 12. && *phase == 4 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-tree-collider.png"));
        *phase = 5;
    }
    if seconds > 14. && *phase == 5 {
        let selected = work
            .entries
            .iter()
            .position(|e| e.name == "actor.player")
            .unwrap();
        apply(Action::Select(selected), &mut work);
        apply(Action::Layout(2), &mut work);
        let delta = 1. - work.dimensions().y;
        apply(Action::Dimension(1, delta), &mut work);
        work.distance = 4.5;
        work.target = Vec3::Y * 0.8;
        apply(Action::Pause, &mut work);
        apply(Action::Animation, &mut work);
        *phase = 6;
    }
    if seconds > 16. && *phase == 6 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-measure.png"));
        *phase = 7;
    }
    if seconds > 18. && *phase == 7 {
        let selected = work
            .entries
            .iter()
            .position(|e| e.name == "structure.house")
            .unwrap();
        apply(Action::Select(selected), &mut work);
        apply(Action::Layout(0), &mut work);
        apply(Action::Reset, &mut work);
        apply(Action::View(0), &mut work);
        work.distance = 8.;
        work.target = Vec3::Y * 1.15;
        *phase = 8;
    }
    if seconds > 20. && *phase == 8 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-house-candidate.png"));
        *phase = 9;
    }
    for (start, step, name, path, distance) in [
        (
            22.,
            9,
            "scenery.rock.0",
            "/tmp/terra-preview-rock-candidate.png",
            4.,
        ),
        (
            26.,
            11,
            "weapon.knife",
            "/tmp/terra-preview-knife-candidate.png",
            2.,
        ),
    ] {
        if seconds > start && *phase == step {
            let selected = work.entries.iter().position(|e| e.name == name).unwrap();
            apply(Action::Select(selected), &mut work);
            apply(Action::Reset, &mut work);
            work.distance = distance;
            work.target = Vec3::Y * 0.25;
            *phase += 1;
        }
        if seconds > start + 2. && *phase == step + 1 {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            *phase += 1;
        }
    }
    if seconds > 30. && *phase == 13 {
        let selected = work
            .entries
            .iter()
            .position(|e| e.name == "actor.player")
            .unwrap();
        apply(Action::Select(selected), &mut work);
        apply(Action::View(2), &mut work);
        work.distance = 3.;
        work.target = Vec3::Y * 0.5;
        work.animation = 1;
        work.paused = false;
        *phase = 14;
    }
    if seconds > 31.35 && *phase == 14 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-walk.png"));
        work.animation = 2;
        work.rebuild = true;
        *phase = 15;
    }
    if seconds > 31.95 && *phase == 15 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("/tmp/terra-preview-attack.png"));
        *phase = 16;
    }
    if seconds > 33. && *phase == 16 {
        exit.write(AppExit::Success);
        *phase = 17;
    }
}
fn labels(
    camera: Query<(&Camera, &GlobalTransform), With<PreviewCamera>>,
    mut labels: Query<(&RulerLabel, &mut Node, &mut Visibility)>,
) {
    let Ok((camera, transform)) = camera.single() else {
        return;
    };
    for (label, mut node, mut visibility) in &mut labels {
        if let Ok(point) = camera.world_to_viewport(transform, label.0) {
            node.left = px(point.x);
            node.top = px(point.y);
            *visibility = Visibility::Visible;
        } else {
            *visibility = Visibility::Hidden;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_bounds_are_measurement_only_and_report_no_current_collider() {
        assert_eq!(
            overlay_status("scenery.tree.0", Vec3::ONE, false),
            "Visual bounds: measurement only\nCurrent collider: none"
        );
    }

    #[test]
    fn comparison_identifies_the_baseline_collider_and_rocks_keep_their_box() {
        assert_eq!(
            overlay_status("scenery.rock.0", Vec3::ONE, true),
            "Visual bounds: measurement only\nCurrent collider: box (shown on baseline)"
        );
    }
}

fn candidate_overlay_visibility(work: Res<Workbench>, mut config: ResMut<GizmoConfigStore>) {
    config.config_mut::<PhysicsGizmos>().0.enabled = work.overlay;
}

#[derive(Component)]
struct PilotWeaponAttached;
fn attach_pilot_weapon(
    mut commands: Commands,
    work: Res<Workbench>,
    names: Query<(Entity, &Name), Without<PilotWeaponAttached>>,
    parents: Query<&ChildOf>,
    playback: Query<&ActorPlayback>,
) {
    let Some(actor) = work
        .entries
        .iter()
        .find(|e| e.name == "actor.player")
        .and_then(|e| e.candidate.as_ref())
    else {
        return;
    };
    let Some(weapon) = work
        .entries
        .iter()
        .find(|e| e.name == "weapon.knife")
        .and_then(|e| e.candidate.as_ref())
    else {
        return;
    };
    for (entity, name) in &names {
        if name.as_str() != "socket.hand" {
            continue;
        }
        if parents
            .iter_ancestors(entity)
            .filter_map(|p| playback.get(p).ok())
            .any(|p| p.source == actor.source)
        {
            commands
                .entity(entity)
                .insert(PilotWeaponAttached)
                .with_child((
                    WorldAssetRoot(weapon.scene.clone()),
                    shared::asset_contract::grip_transform("weapon.knife").unwrap(),
                ));
        }
    }
}
