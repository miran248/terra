use bevy::prelude::*;
use shared::planet::PlanetMesh;
use shared::roads::{
    SurfaceRoadSample, build_surface_road_ribbon, build_terrain_following_road_ribbon,
    projected_surface_ribbon_width,
};
use std::time::Instant;

use crate::{
    exploration::{Exploration, ExplorationUpdate},
    map::MainCamera,
};

const MIN_SCREEN_WIDTH_PX: f32 = 2.5;
const MIN_ROAD_WIDTH_METERS: f32 = 4.0;
pub(crate) const ROAD_SAMPLE_SPACING_METERS: f32 = 4.0;
pub(crate) const ROAD_SURFACE_LIFT_METERS: f32 = 0.22;
const WIDTH_REFRESH_SECONDS: f32 = 0.2;
const WIDTH_REFRESH_POSITION_METERS: f32 = 24.0;
const ROAD_GEOMETRY_PATHS_PER_FRAME: usize = 8;

#[derive(Resource, Default)]
pub(crate) struct RoadHighlightPaths {
    pub(crate) terrain: Vec<Vec<SurfaceRoadSample>>,
    pub(crate) bridges: Vec<Vec<SurfaceRoadSample>>,
}

#[derive(Clone, Debug, PartialEq)]
struct RoadHighlightWidths {
    terrain: Vec<Vec<f32>>,
    bridges: Vec<Vec<f32>>,
}

#[derive(Component)]
struct RoadHighlightMesh {
    ready: bool,
}

#[derive(Component)]
struct RoadHighlightOpacity(u32);

#[derive(Clone)]
struct RoadGeometryRequest {
    epoch: crate::map::WorldEpoch,
    widths: RoadHighlightWidths,
}

struct RoadGeometryJob {
    epoch: crate::map::WorldEpoch,
    widths: RoadHighlightWidths,
    next_path: usize,
    prepared_mesh: PreparedRoadMesh,
}

impl RoadGeometryJob {
    fn new(epoch: crate::map::WorldEpoch, widths: RoadHighlightWidths) -> Self {
        Self {
            epoch,
            widths,
            next_path: 0,
            prepared_mesh: PreparedRoadMesh::default(),
        }
    }

    fn advance(
        &mut self,
        paths: &RoadHighlightPaths,
        ground: &PlanetMesh,
        path_budget: usize,
    ) -> usize {
        let total_paths = paths.terrain.len() + paths.bridges.len();
        let end = self.next_path.saturating_add(path_budget).min(total_paths);
        let processed = end - self.next_path;

        for path_index in self.next_path..end {
            let triangles = if path_index < paths.terrain.len() {
                build_terrain_following_road_ribbon(
                    &paths.terrain[path_index],
                    &self.widths.terrain[path_index],
                    ground,
                    ROAD_SURFACE_LIFT_METERS,
                )
            } else {
                let bridge_index = path_index - paths.terrain.len();
                build_surface_road_ribbon(
                    &paths.bridges[bridge_index],
                    &self.widths.bridges[bridge_index],
                )
            };
            self.prepared_mesh.append_white_triangles(&triangles);
        }

        self.next_path = end;
        processed
    }

    fn is_complete(&self, paths: &RoadHighlightPaths) -> bool {
        self.next_path >= paths.terrain.len() + paths.bridges.len()
    }

    fn finish(self) -> CompletedRoadGeometry {
        CompletedRoadGeometry {
            epoch: self.epoch,
            widths: self.widths,
            prepared_mesh: self.prepared_mesh,
        }
    }
}

struct CompletedRoadGeometry {
    epoch: crate::map::WorldEpoch,
    widths: RoadHighlightWidths,
    prepared_mesh: PreparedRoadMesh,
}

#[derive(Default)]
struct PreparedRoadMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl PreparedRoadMesh {
    fn append_white_triangles(&mut self, triangles: &[[[f32; 3]; 3]]) {
        let additional_vertices = triangles.len() * 3;
        self.positions.reserve(additional_vertices);
        self.normals.reserve(additional_vertices);
        self.colors.reserve(additional_vertices);

        for triangle in triangles {
            let a = Vec3::from_array(triangle[0]);
            let b = Vec3::from_array(triangle[1]);
            let c = Vec3::from_array(triangle[2]);
            let normal = (b - a).cross(c - a).normalize().to_array();

            self.positions.extend_from_slice(triangle);
            self.normals.extend([normal; 3]);
            self.colors.extend([[1.0, 1.0, 1.0, 1.0]; 3]);
        }
    }

    fn into_mesh(self) -> Mesh {
        crate::map::build_visual_mesh_from_attributes(self.positions, self.normals, self.colors)
    }

    fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

#[derive(Resource, Default)]
struct RoadMeshRefresh {
    elapsed: f32,
    last_camera_position: Option<Vec3>,
    last_viewport_height: Option<f32>,
    last_vertical_fov: Option<f32>,
    last_widths: Option<RoadHighlightWidths>,
    last_world_epoch: Option<crate::map::WorldEpoch>,
    observed_world_epoch: Option<crate::map::WorldEpoch>,
    request_world_epoch: Option<crate::map::WorldEpoch>,
    job: Option<RoadGeometryJob>,
    pending_request: Option<RoadGeometryRequest>,
    completed_geometry: Option<CompletedRoadGeometry>,
}

impl RoadMeshRefresh {
    fn geometry_is_current(
        &self,
        world_epoch: crate::map::WorldEpoch,
        widths: &RoadHighlightWidths,
    ) -> bool {
        self.last_world_epoch == Some(world_epoch) && self.last_widths.as_ref() == Some(widths)
    }

    fn observe_world_epoch(&mut self, epoch: crate::map::WorldEpoch) -> bool {
        if self.observed_world_epoch == Some(epoch) {
            return false;
        }

        self.observed_world_epoch = Some(epoch);
        self.cancel_uncommitted_work();
        self.last_world_epoch = None;
        self.last_widths = None;
        true
    }

    fn request_geometry(&mut self, epoch: crate::map::WorldEpoch, widths: RoadHighlightWidths) {
        let request = RoadGeometryRequest { epoch, widths };
        let current_in_flight = self
            .job
            .as_ref()
            .map(|job| (job.epoch, &job.widths))
            .or_else(|| {
                self.completed_geometry
                    .as_ref()
                    .map(|completed| (completed.epoch, &completed.widths))
            });

        if let Some((in_flight_epoch, in_flight_widths)) = current_in_flight {
            if in_flight_epoch == epoch && in_flight_widths == &request.widths {
                self.pending_request = None;
            } else {
                self.pending_request = Some(request);
            }
        } else if self.geometry_is_current(epoch, &request.widths) {
            self.pending_request = None;
        } else {
            self.pending_request = Some(request);
        }
    }

    fn start_pending_geometry(&mut self) {
        if self.job.is_some() || self.completed_geometry.is_some() {
            return;
        }

        let Some(request) = self.pending_request.take() else {
            return;
        };
        if !self.geometry_is_current(request.epoch, &request.widths) {
            self.job = Some(RoadGeometryJob::new(request.epoch, request.widths));
        }
    }

    fn advance_geometry(
        &mut self,
        paths: &RoadHighlightPaths,
        ground: &PlanetMesh,
        path_budget: usize,
    ) -> usize {
        if self.completed_geometry.is_some() {
            return 0;
        }
        self.start_pending_geometry();

        let Some(job) = self.job.as_mut() else {
            return 0;
        };
        let processed = job.advance(paths, ground, path_budget);
        if job.is_complete(paths) {
            let completed = self.job.take().expect("completed job is present");
            self.completed_geometry = Some(completed.finish());
        }
        processed
    }

    fn take_ready_geometry(
        &mut self,
        epoch: crate::map::WorldEpoch,
    ) -> Option<CompletedRoadGeometry> {
        let completed = self.completed_geometry.take()?;
        (completed.epoch == epoch).then_some(completed)
    }

    fn record_geometry_commit(
        &mut self,
        epoch: crate::map::WorldEpoch,
        widths: RoadHighlightWidths,
    ) {
        self.last_world_epoch = Some(epoch);
        self.last_widths = Some(widths);
    }

    fn cancel_uncommitted_work(&mut self) {
        self.job = None;
        self.pending_request = None;
        self.completed_geometry = None;
        self.elapsed = 0.0;
        self.last_camera_position = None;
        self.last_viewport_height = None;
        self.last_vertical_fov = None;
        self.request_world_epoch = None;
    }
}

pub(crate) struct PlanetRoadsPlugin;

impl Plugin for PlanetRoadsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RoadMeshRefresh>()
            .add_systems(
                OnEnter(shared::state::AppState::Playing),
                setup_road_highlight.after(crate::map::setup_map),
            )
            .add_systems(
                OnExit(shared::state::AppState::Playing),
                cleanup_road_highlight,
            )
            .add_systems(
                Update,
                (
                    update_road_highlight_widths,
                    update_road_highlight_presentation,
                )
                    .chain()
                    .after(ExplorationUpdate)
                    .run_if(in_state(shared::state::AppState::Playing)),
            );
    }
}

fn setup_road_highlight(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut refresh: ResMut<RoadMeshRefresh>,
) {
    *refresh = RoadMeshRefresh::default();
    commands.spawn((
        MeshMaterial3d(materials.add(road_highlight_material(1.0))),
        Transform::default(),
        Visibility::Hidden,
        RoadHighlightMesh { ready: false },
        RoadHighlightOpacity(u32::MAX),
    ));
}

fn cleanup_road_highlight(
    mut commands: Commands,
    entities: Query<Entity, With<RoadHighlightMesh>>,
    mut refresh: ResMut<RoadMeshRefresh>,
) {
    *refresh = RoadMeshRefresh::default();
    for entity in &entities {
        commands.entity(entity).despawn();
    }
}

fn update_road_highlight_presentation(
    state: Res<Exploration>,
    mut highlights: Query<
        (
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
            &mut RoadHighlightOpacity,
            &RoadHighlightMesh,
        ),
        With<RoadHighlightMesh>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let opacity = state.planet_view_interface_opacity().clamp(0.0, 1.0);
    let view_active = state.is_planet_view_active();
    let show_highlight = planet_roads_highlight_visible(view_active, opacity);
    for (mut visibility, material_handle, mut applied_opacity, mesh_state) in &mut highlights {
        let desired_visibility = if show_highlight && mesh_state.ready {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != desired_visibility {
            *visibility = desired_visibility;
        }
        let opacity_bits = opacity.to_bits();
        if applied_opacity.0 != opacity_bits {
            if let Some(mut material) = materials.get_mut(&material_handle.0) {
                *material = road_highlight_material(opacity);
            }
            applied_opacity.0 = opacity_bits;
        }
    }
}

fn road_highlight_material(opacity: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(0.9, 0.64, 0.18, opacity.clamp(0.0, 1.0)),
        emissive: LinearRgba::rgb(0.26, 0.13, 0.025),
        perceptual_roughness: 0.9,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

fn planet_roads_highlight_visible(view_active: bool, opacity: f32) -> bool {
    view_active && opacity > 0.0
}

#[cfg(test)]
fn build_highlight_triangles(
    paths: &RoadHighlightPaths,
    ground: &PlanetMesh,
    widths: &RoadHighlightWidths,
) -> Vec<[[f32; 3]; 3]> {
    let mut triangles = Vec::new();
    debug_assert_eq!(paths.terrain.len(), widths.terrain.len());
    debug_assert_eq!(paths.bridges.len(), widths.bridges.len());
    for (samples, path_widths) in paths.terrain.iter().zip(&widths.terrain) {
        triangles.extend(build_terrain_following_road_ribbon(
            samples,
            path_widths,
            ground,
            ROAD_SURFACE_LIFT_METERS,
        ));
    }
    for (samples, path_widths) in paths.bridges.iter().zip(&widths.bridges) {
        triangles.extend(build_surface_road_ribbon(samples, path_widths));
    }
    triangles
}

fn road_highlight_widths(
    paths: &RoadHighlightPaths,
    mut width_at: impl FnMut(&SurfaceRoadSample) -> f32,
) -> RoadHighlightWidths {
    RoadHighlightWidths {
        terrain: widths_for_path_collection(&paths.terrain, &mut width_at),
        bridges: widths_for_path_collection(&paths.bridges, &mut width_at),
    }
}

fn widths_for_path_collection(
    paths: &[Vec<SurfaceRoadSample>],
    width_at: &mut impl FnMut(&SurfaceRoadSample) -> f32,
) -> Vec<Vec<f32>> {
    paths
        .iter()
        .map(|samples| samples.iter().map(&mut *width_at).collect())
        .collect()
}

fn update_road_highlight_widths(
    mut commands: Commands,
    time: Res<Time<Real>>,
    state: Res<Exploration>,
    paths: Res<RoadHighlightPaths>,
    ground: Res<crate::map::CollisionTerrain>,
    world_epoch: Res<crate::map::WorldEpoch>,
    cameras: Query<(&Camera, &Projection, &Transform), With<MainCamera>>,
    mut highlight_mesh: Query<
        (Entity, Option<&Mesh3d>, &mut RoadHighlightMesh),
        With<RoadHighlightMesh>,
    >,
    mut meshes: ResMut<Assets<Mesh>>,
    mut refresh: ResMut<RoadMeshRefresh>,
    mut probe: Option<ResMut<crate::chunks::PlanetWorkTrace>>,
) {
    let started = probe
        .as_ref()
        .is_some_and(|probe| probe.active())
        .then(Instant::now);
    let camera_radius_m = cameras
        .single()
        .map(|(_, _, transform)| transform.translation.length())
        .unwrap_or(f32::NAN);
    let epoch_changed = refresh.observe_world_epoch(*world_epoch);
    if epoch_changed && let Ok((_, _, mut mesh_state)) = highlight_mesh.single_mut() {
        mesh_state.ready = false;
    }

    if !state.is_planet_view_active() {
        refresh.cancel_uncommitted_work();
        return;
    }

    refresh.elapsed += time.delta_secs();
    let mut width_request = false;
    let mut mesh_committed = false;
    if let Ok((camera, Projection::Perspective(perspective), camera_transform)) = cameras.single()
        && let Some(viewport_size) = camera.logical_viewport_size()
        && viewport_size.y > 0.0
    {
        let first_request = refresh.request_world_epoch != Some(*world_epoch);
        let moved = refresh.last_camera_position.is_none_or(|last| {
            last.distance(camera_transform.translation) >= WIDTH_REFRESH_POSITION_METERS
        });
        let resized = refresh
            .last_viewport_height
            .is_none_or(|height| (height - viewport_size.y).abs() > 1.0);
        let changed_fov = refresh
            .last_vertical_fov
            .is_none_or(|fov| (fov - perspective.fov).abs() > 0.001);
        let request_due = first_request || moved || resized || changed_fov;
        if request_due && (first_request || refresh.elapsed >= WIDTH_REFRESH_SECONDS) {
            let widths = road_highlight_widths(&paths, |sample| {
                let depth = perspective_camera_depth(*camera_transform, sample.center);
                projected_surface_ribbon_width(
                    depth.max(f32::EPSILON),
                    perspective.fov,
                    viewport_size.y,
                    MIN_SCREEN_WIDTH_PX,
                    MIN_ROAD_WIDTH_METERS,
                )
                .unwrap_or(MIN_ROAD_WIDTH_METERS)
            });
            refresh.request_geometry(*world_epoch, widths);
            width_request = true;
            refresh.request_world_epoch = Some(*world_epoch);
            refresh.elapsed = 0.0;
            refresh.last_camera_position = Some(camera_transform.translation);
            refresh.last_viewport_height = Some(viewport_size.y);
            refresh.last_vertical_fov = Some(perspective.fov);
        }
    }

    if refresh.completed_geometry.is_some() {
        if let Ok((entity, current_mesh, mut mesh_state)) = highlight_mesh.single_mut()
            && let Some(completed) = refresh.take_ready_geometry(*world_epoch)
        {
            let CompletedRoadGeometry {
                epoch,
                widths,
                prepared_mesh,
            } = completed;
            if !prepared_mesh.is_empty() {
                let mesh = prepared_mesh.into_mesh();
                if let Some(current_mesh) = current_mesh {
                    if let Some(mut mesh_asset) = meshes.get_mut(&current_mesh.0) {
                        *mesh_asset = mesh;
                        mesh_state.ready = true;
                        refresh.record_geometry_commit(epoch, widths);
                        mesh_committed = true;
                    }
                } else {
                    commands.entity(entity).insert(Mesh3d(meshes.add(mesh)));
                    mesh_state.ready = true;
                    refresh.record_geometry_commit(epoch, widths);
                    mesh_committed = true;
                }
            }
        }
        if let (Some(probe), Some(started)) = (probe.as_mut(), started) {
            probe.record(
                time.elapsed_secs_f64(),
                "road_geometry",
                camera_radius_m,
                started.elapsed().as_secs_f64() * 1_000.0,
                0,
                usize::from(width_request),
                1,
                usize::from(mesh_committed),
            );
        }
        return;
    }

    let paths_processed =
        refresh.advance_geometry(&paths, &ground.0, ROAD_GEOMETRY_PATHS_PER_FRAME);
    let geometry_complete = refresh.completed_geometry.is_some();
    if (paths_processed > 0 || width_request || geometry_complete || epoch_changed)
        && let (Some(probe), Some(started)) = (probe.as_mut(), started)
    {
        probe.record(
            time.elapsed_secs_f64(),
            "road_geometry",
            camera_radius_m,
            started.elapsed().as_secs_f64() * 1_000.0,
            paths_processed,
            usize::from(width_request),
            usize::from(geometry_complete),
            usize::from(mesh_committed),
        );
    }
}

fn perspective_camera_depth(camera_transform: Transform, world_position: Vec3) -> f32 {
    -(camera_transform.rotation.inverse() * (world_position - camera_transform.translation)).z
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::build_visual_mesh;
    use bevy::camera::{CameraProjection, RenderTargetInfo, Viewport};

    #[test]
    fn road_highlight_setup_defers_mesh_asset_until_geometry_is_ready() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(RoadMeshRefresh::default())
            .add_systems(Update, setup_road_highlight);

        app.update();

        let mut highlights = app
            .world_mut()
            .query_filtered::<Entity, With<RoadHighlightMesh>>();
        let highlight = highlights
            .iter(app.world())
            .next()
            .expect("road highlight entity should be prepared");
        assert!(app.world().get::<Mesh3d>(highlight).is_none());
        assert_eq!(
            *app.world().get::<Visibility>(highlight).unwrap(),
            Visibility::Hidden
        );
        assert!(
            !app.world()
                .get::<RoadHighlightMesh>(highlight)
                .unwrap()
                .ready
        );
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), 0);
    }

    #[test]
    fn road_highlights_are_automatic_in_planet_view_and_follow_interface_fade() {
        assert!(!planet_roads_highlight_visible(false, 0.5));
        assert!(planet_roads_highlight_visible(true, 0.5));
        assert!(!planet_roads_highlight_visible(true, 0.0));
    }

    #[test]
    fn road_highlight_fades_with_alpha_and_uses_standard_depth_testing() {
        let material = road_highlight_material(0.4);
        let color = material.base_color.to_srgba();

        assert_eq!(material.alpha_mode, AlphaMode::Blend);
        assert!(!material.unlit);
        assert!((color.red - 0.9).abs() < 0.001);
        assert!((color.green - 0.64).abs() < 0.001);
        assert!((color.blue - 0.18).abs() < 0.001);
        assert!((color.alpha - 0.4).abs() < 0.001);
    }

    #[test]
    fn road_width_uses_depth_along_the_perspective_camera_axis() {
        let camera = Transform::from_xyz(0.0, 0.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
        let point = Vec3::new(100.0, 0.0, 0.0);

        assert!((perspective_camera_depth(camera, point) - 10.0).abs() < 0.001);
    }

    #[test]
    fn captured_road_projection_direction_resolves_its_shared_facet_edge() {
        let level = shared::level::LevelData::from_artifact_bytes(include_bytes!(
            "../assets/level_1337.bin"
        ))
        .expect("decode canonical level artifact");
        let ground = PlanetMesh::new(
            level
                .terrain_tris
                .iter()
                .map(|triangle| triangle.map(Vec3::from_array))
                .collect(),
        );
        let direction = Vec3::new(
            f32::from_bits(0xbeba_40e7),
            f32::from_bits(0x3f4c_cd4c),
            f32::from_bits(0x3ef4_4a98),
        );
        let triangle = ground
            .triangle(2091)
            .expect("captured edge triangle remains in canonical terrain");

        let edge_normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
        let expected = edge_normal.dot(triangle[0]) / edge_normal.dot(direction);
        let actual = ground.facet_radius(direction, shared::sphere::PLANET_RADIUS);
        let road_projection =
            ground.facet_radius_for_road_projection(direction, shared::sphere::PLANET_RADIUS);

        assert_eq!(actual, shared::sphere::PLANET_RADIUS);
        assert!((road_projection - expected).abs() < 0.01);
    }

    #[test]
    fn bridge_highlight_stays_at_its_sampled_deck_height() {
        let ground = shared::planet::PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let terrain_radius = ground.facet_radius(Vec3::Y, shared::sphere::PLANET_RADIUS);
        let deck_radius = terrain_radius + ROAD_SURFACE_LIFT_METERS + 5.0;
        let samples = [-10.0, 10.0].map(|z| SurfaceRoadSample {
            center: Vec3::Y * deck_radius + Vec3::Z * z,
            normal: Vec3::Y,
        });
        let paths = RoadHighlightPaths {
            terrain: Vec::new(),
            bridges: vec![samples.to_vec()],
        };
        let widths = road_highlight_widths(&paths, |_| 4.0);
        let triangles = build_highlight_triangles(&paths, &ground, &widths);

        assert!(!triangles.is_empty());
        let minimum_z = triangles
            .iter()
            .flatten()
            .map(|vertex| vertex[2])
            .fold(f32::INFINITY, f32::min);
        let maximum_z = triangles
            .iter()
            .flatten()
            .map(|vertex| vertex[2])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((minimum_z + 10.0).abs() < 0.01);
        assert!((maximum_z - 10.0).abs() < 0.01);
        let minimum_deck_clearance = triangles
            .iter()
            .flatten()
            .map(|vertex| {
                let point = Vec3::from_array(*vertex);
                point.length()
                    - ground.facet_radius(point.normalize(), shared::sphere::PLANET_RADIUS)
            })
            .fold(f32::INFINITY, f32::min);
        assert!(minimum_deck_clearance > 4.0);
    }

    #[test]
    fn highlight_geometry_cache_requires_same_world_and_projected_widths() {
        let epoch = crate::map::WorldEpoch::new(3);
        let widths = RoadHighlightWidths {
            terrain: vec![vec![4.0, 7.0]],
            bridges: vec![vec![5.0, 6.0]],
        };
        let mut refresh = RoadMeshRefresh {
            last_widths: Some(widths.clone()),
            last_world_epoch: Some(epoch),
            ..default()
        };

        assert!(refresh.geometry_is_current(epoch, &widths));

        let changed_widths = RoadHighlightWidths {
            terrain: vec![vec![4.0, 7.1]],
            bridges: vec![vec![5.0, 6.0]],
        };
        assert!(!refresh.geometry_is_current(epoch, &changed_widths));
        refresh.last_widths = Some(changed_widths.clone());
        assert!(!refresh.geometry_is_current(crate::map::WorldEpoch::new(4), &changed_widths));
    }

    fn road_job_fixture_paths() -> RoadHighlightPaths {
        let make_path = |index: usize| {
            let center = Vec3::Y * shared::sphere::PLANET_RADIUS + Vec3::X * (index as f32 * 4.0);
            vec![
                SurfaceRoadSample {
                    center: center - Vec3::Z * 8.0,
                    normal: Vec3::Y,
                },
                SurfaceRoadSample {
                    center: center + Vec3::Z * 8.0,
                    normal: Vec3::Y,
                },
            ]
        };
        RoadHighlightPaths {
            terrain: (0..17).map(make_path).collect(),
            bridges: (17..20).map(make_path).collect(),
        }
    }

    fn road_job_fixture_widths(
        paths: &RoadHighlightPaths,
        terrain_width: f32,
        bridge_width: f32,
    ) -> RoadHighlightWidths {
        RoadHighlightWidths {
            terrain: paths
                .terrain
                .iter()
                .map(|samples| vec![terrain_width; samples.len()])
                .collect(),
            bridges: paths
                .bridges
                .iter()
                .map(|samples| vec![bridge_width; samples.len()])
                .collect(),
        }
    }

    fn test_mesh_positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) => positions.clone(),
            other => panic!("expected Float32x3 mesh positions, got {other:?}"),
        }
    }

    #[derive(Debug, PartialEq)]
    struct TestMeshAttributes {
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        colors: Vec<[f32; 4]>,
    }

    fn test_mesh_attributes(mesh: &Mesh) -> TestMeshAttributes {
        let positions = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) => values.clone(),
            other => panic!("expected Float32x3 mesh positions, got {other:?}"),
        };
        let normals = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(values)) => values.clone(),
            other => panic!("expected Float32x3 mesh normals, got {other:?}"),
        };
        let colors = match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(bevy::mesh::VertexAttributeValues::Float32x4(values)) => values.clone(),
            other => panic!("expected Float32x4 mesh colors, got {other:?}"),
        };
        TestMeshAttributes {
            positions,
            normals,
            colors,
        }
    }

    fn assert_prepared_mesh_matches(prepared: &PreparedRoadMesh, expected: &Mesh) {
        let expected = test_mesh_attributes(expected);
        assert!(
            prepared.positions == expected.positions,
            "prepared road positions differ from the visual mesh oracle"
        );
        assert!(
            prepared.normals == expected.normals,
            "prepared road normals differ from the visual mesh oracle"
        );
        assert!(
            prepared.colors == expected.colors,
            "prepared road colors differ from the visual mesh oracle"
        );
    }

    fn projected_widths_for_test_pose(
        paths: &RoadHighlightPaths,
        transform: Transform,
        perspective: &PerspectiveProjection,
        viewport_height: f32,
    ) -> RoadHighlightWidths {
        road_highlight_widths(paths, |sample| {
            let depth = perspective_camera_depth(transform, sample.center);
            projected_surface_ribbon_width(
                depth.max(f32::EPSILON),
                perspective.fov,
                viewport_height,
                MIN_SCREEN_WIDTH_PX,
                MIN_ROAD_WIDTH_METERS,
            )
            .unwrap_or(MIN_ROAD_WIDTH_METERS)
        })
    }

    fn app_for_road_geometry_refresh(
        paths: RoadHighlightPaths,
        ground: PlanetMesh,
        epoch: crate::map::WorldEpoch,
        camera_transform: Transform,
        baseline_widths: RoadHighlightWidths,
        existing_mesh: Mesh,
    ) -> (App, Entity, Entity, Handle<Mesh>) {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(200),
            ))
            .insert_resource({
                let mut state = Exploration::default();
                state.set_planet_view_open(true);
                state
            })
            .insert_resource(RoadHighlightPaths {
                terrain: paths.terrain,
                bridges: paths.bridges,
            })
            .insert_resource(crate::map::CollisionTerrain(ground))
            .insert_resource(epoch);

        let mesh_handle = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(existing_mesh);
        let highlight_entity = app
            .world_mut()
            .spawn((
                Mesh3d(mesh_handle.clone()),
                RoadHighlightMesh { ready: true },
            ))
            .id();

        let size = UVec2::new(1200, 900);
        let perspective = PerspectiveProjection {
            far: 20_000_000.0,
            ..default()
        };
        let vertical_fov = perspective.fov;
        let mut camera = Camera {
            viewport: Some(Viewport {
                physical_size: size,
                ..default()
            }),
            ..default()
        };
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: size,
            scale_factor: 1.0,
        });
        camera.computed.clip_from_view = perspective.get_clip_from_view();
        let camera_entity = app
            .world_mut()
            .spawn((
                MainCamera,
                camera,
                Projection::Perspective(perspective),
                camera_transform,
                GlobalTransform::from(camera_transform),
            ))
            .id();

        app.insert_resource(RoadMeshRefresh {
            elapsed: 0.0,
            last_camera_position: Some(camera_transform.translation - Vec3::Z * 500.0),
            last_viewport_height: Some(size.y as f32),
            last_vertical_fov: Some(vertical_fov),
            last_widths: Some(baseline_widths.clone()),
            last_world_epoch: Some(epoch),
            observed_world_epoch: Some(epoch),
            request_world_epoch: None,
            job: None,
            pending_request: None,
            completed_geometry: None,
        });
        app.add_systems(Update, update_road_highlight_widths);
        (app, camera_entity, highlight_entity, mesh_handle)
    }

    #[test]
    fn first_road_geometry_commit_attaches_only_a_nonempty_mesh() {
        let paths = road_job_fixture_paths();
        let ground = PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let epoch = crate::map::WorldEpoch::new(6);
        let widths = road_job_fixture_widths(&paths, 4.0, 5.0);
        let camera_transform =
            Transform::from_translation(Vec3::Y * (shared::sphere::PLANET_RADIUS + 1_000.0))
                .looking_at(Vec3::ZERO, Vec3::Z);
        let (mut app, _, highlight, placeholder) = app_for_road_geometry_refresh(
            paths,
            ground,
            epoch,
            camera_transform,
            widths,
            build_visual_mesh(&[], &[]),
        );
        app.world_mut().entity_mut(highlight).remove::<Mesh3d>();
        app.world_mut()
            .entity_mut(highlight)
            .get_mut::<RoadHighlightMesh>()
            .unwrap()
            .ready = false;
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .remove(placeholder.id());
        assert!(app.world().get::<Mesh3d>(highlight).is_none());

        for _ in 0..8 {
            app.update();
            if app.world().get::<Mesh3d>(highlight).is_some() {
                break;
            }
        }

        assert!(
            app.world()
                .get::<RoadHighlightMesh>(highlight)
                .unwrap()
                .ready
        );
        let mesh = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&app.world().get::<Mesh3d>(highlight).unwrap().0)
            .expect("the completed road geometry is allocated");
        assert!(mesh.get_vertex_buffer_size() > 0);
    }

    #[test]
    fn road_mesh_attributes_are_prepared_before_the_atomic_asset_swap() {
        let paths = road_job_fixture_paths();
        let ground = PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let epoch = crate::map::WorldEpoch::new(20);
        let old_widths = road_job_fixture_widths(&paths, 4.0, 5.0);
        let old_triangles = build_highlight_triangles(&paths, &ground, &old_widths);
        let old_mesh = build_visual_mesh(
            &old_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; old_triangles.len()],
        );
        let old_attributes = test_mesh_attributes(&old_mesh);

        let camera_transform =
            Transform::from_translation(Vec3::Z * (shared::sphere::PLANET_RADIUS + 10_000.0))
                .looking_at(Vec3::ZERO, Vec3::Y);
        let perspective = PerspectiveProjection {
            far: 20_000_000.0,
            ..default()
        };
        let requested_widths =
            projected_widths_for_test_pose(&paths, camera_transform, &perspective, 900.0);
        let expected_triangles = build_highlight_triangles(&paths, &ground, &requested_widths);
        let expected_mesh = build_visual_mesh(
            &expected_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; expected_triangles.len()],
        );
        let expected_attributes = test_mesh_attributes(&expected_mesh);

        let (mut app, _, _, mesh_handle) = app_for_road_geometry_refresh(
            paths,
            ground,
            epoch,
            camera_transform,
            old_widths,
            old_mesh,
        );

        for _ in 0..8 {
            app.update();
            if app
                .world()
                .resource::<RoadMeshRefresh>()
                .completed_geometry
                .is_some()
            {
                break;
            }
        }

        let completed = app
            .world()
            .resource::<RoadMeshRefresh>()
            .completed_geometry
            .as_ref()
            .expect("the bounded path job reaches its completed state");
        assert_prepared_mesh_matches(&completed.prepared_mesh, &expected_mesh);
        assert_eq!(completed.widths, requested_widths);
        assert_eq!(
            test_mesh_attributes(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&mesh_handle)
                    .expect("the prior road mesh remains allocated")
            ),
            old_attributes,
            "staging the new mesh must not mutate the currently visible asset"
        );

        app.update();
        assert_eq!(
            test_mesh_attributes(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&mesh_handle)
                    .expect("the committed road mesh remains allocated")
            ),
            expected_attributes,
            "the atomic swap must commit the exact position, normal, and color output"
        );
    }

    #[test]
    fn road_geometry_job_is_bounded_atomic_and_coalesces_camera_reversals() {
        let paths = road_job_fixture_paths();
        let ground = PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let epoch = crate::map::WorldEpoch::new(7);
        let original_widths = road_job_fixture_widths(&paths, 4.0, 5.0);
        let first_camera_widths = road_job_fixture_widths(&paths, 8.0, 9.0);
        let latest_camera_widths = road_job_fixture_widths(&paths, 6.0, 7.0);
        let original_visible_mesh = build_highlight_triangles(&paths, &ground, &original_widths);
        let mut visible_mesh = original_visible_mesh.clone();

        let mut refresh = RoadMeshRefresh::default();
        refresh.observe_world_epoch(epoch);
        refresh.request_geometry(epoch, first_camera_widths.clone());
        refresh.start_pending_geometry();
        assert_eq!(refresh.advance_geometry(&paths, &ground, 8), 8);
        assert_eq!(visible_mesh, original_visible_mesh);

        refresh.request_geometry(epoch, original_widths.clone());
        refresh.request_geometry(epoch, latest_camera_widths.clone());
        assert_eq!(refresh.job.as_ref().unwrap().next_path, 8);
        assert_eq!(
            refresh.pending_request.as_ref().unwrap().widths,
            latest_camera_widths
        );

        assert_eq!(refresh.advance_geometry(&paths, &ground, 8), 8);
        assert_eq!(visible_mesh, original_visible_mesh);
        assert_eq!(refresh.advance_geometry(&paths, &ground, 8), 4);
        assert!(refresh.job.is_none());
        assert_eq!(refresh.last_widths, None);
        assert_eq!(visible_mesh, original_visible_mesh);

        let first_result = refresh.take_ready_geometry(epoch).unwrap();
        let first_triangles = build_highlight_triangles(&paths, &ground, &first_camera_widths);
        let first_mesh = build_visual_mesh(
            &first_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; first_triangles.len()],
        );
        assert_prepared_mesh_matches(&first_result.prepared_mesh, &first_mesh);
        visible_mesh = first_triangles;
        refresh.record_geometry_commit(first_result.epoch, first_result.widths);
        assert_ne!(visible_mesh, original_visible_mesh);

        refresh.start_pending_geometry();
        while refresh.completed_geometry.is_none() {
            let processed = refresh.advance_geometry(&paths, &ground, 8);
            assert!(processed <= 8);
        }
        let latest_result = refresh.take_ready_geometry(epoch).unwrap();
        let latest_triangles = build_highlight_triangles(&paths, &ground, &latest_camera_widths);
        let latest_mesh = build_visual_mesh(
            &latest_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; latest_triangles.len()],
        );
        assert_prepared_mesh_matches(&latest_result.prepared_mesh, &latest_mesh);
    }

    #[test]
    fn road_geometry_job_discards_partial_and_staged_work_on_cancel_or_epoch_change() {
        let paths = road_job_fixture_paths();
        let ground = PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let epoch = crate::map::WorldEpoch::new(11);
        let widths = road_job_fixture_widths(&paths, 4.0, 4.0);
        let mut refresh = RoadMeshRefresh::default();
        refresh.observe_world_epoch(epoch);
        refresh.request_geometry(epoch, widths.clone());
        refresh.start_pending_geometry();
        assert_eq!(refresh.advance_geometry(&paths, &ground, 8), 8);
        assert!(refresh.job.is_some());

        refresh.cancel_uncommitted_work();
        assert!(refresh.job.is_none());
        assert!(refresh.pending_request.is_none());
        assert!(refresh.completed_geometry.is_none());

        refresh.request_geometry(epoch, widths.clone());
        refresh.start_pending_geometry();
        while refresh.completed_geometry.is_none() {
            refresh.advance_geometry(&paths, &ground, 8);
        }
        assert!(refresh.completed_geometry.is_some());

        let next_epoch = crate::map::WorldEpoch::new(12);
        assert!(refresh.observe_world_epoch(next_epoch));
        assert!(refresh.job.is_none());
        assert!(refresh.pending_request.is_none());
        assert!(refresh.completed_geometry.is_none());
        assert_eq!(refresh.last_world_epoch, None);
        assert_eq!(refresh.last_widths, None);
    }

    #[test]
    fn runtime_keeps_last_mesh_while_geometry_progresses_and_commits_atomically() {
        let paths = road_job_fixture_paths();
        let paths = RoadHighlightPaths {
            terrain: (0..49)
                .map(|index| paths.terrain[index % paths.terrain.len()].clone())
                .collect(),
            bridges: paths.bridges,
        };
        let ground = PlanetMesh::new(shared::planet::unit_icosphere_tris(3));
        let epoch = crate::map::WorldEpoch::new(21);
        let old_widths = road_job_fixture_widths(&paths, 4.0, 5.0);
        let old_triangles = build_highlight_triangles(&paths, &ground, &old_widths);
        let old_mesh = build_visual_mesh(
            &old_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; old_triangles.len()],
        );
        let old_positions = test_mesh_positions(&old_mesh);

        let mut camera_transform =
            Transform::from_translation(Vec3::Z * (shared::sphere::PLANET_RADIUS + 10_000.0))
                .looking_at(Vec3::ZERO, Vec3::Y);
        let perspective = PerspectiveProjection {
            far: 20_000_000.0,
            ..default()
        };
        let first_widths =
            projected_widths_for_test_pose(&paths, camera_transform, &perspective, 900.0);
        let expected_first_triangles = build_highlight_triangles(&paths, &ground, &first_widths);
        let expected_first_mesh = build_visual_mesh(
            &expected_first_triangles,
            &vec![[[1.0, 1.0, 1.0, 1.0]; 3]; expected_first_triangles.len()],
        );
        let expected_first_positions = test_mesh_positions(&expected_first_mesh);

        let (mut app, camera_entity, highlight_entity, mesh_handle) = app_for_road_geometry_refresh(
            paths,
            ground,
            epoch,
            camera_transform,
            old_widths,
            old_mesh,
        );

        let mut replaced = false;
        let mut current_positions = old_positions.clone();
        app.update();
        assert!(
            test_mesh_positions(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&mesh_handle)
                    .unwrap()
            ) == old_positions,
            "the existing mesh remains visible during the first bounded geometry step"
        );
        for _ in 0..12 {
            camera_transform = Transform::from_translation(
                Vec3::Z * (camera_transform.translation.length() + 500.0),
            )
            .looking_at(Vec3::ZERO, Vec3::Y);
            app.world_mut()
                .entity_mut(camera_entity)
                .insert(camera_transform);
            app.update();

            let mesh = app
                .world()
                .resource::<Assets<Mesh>>()
                .get(&mesh_handle)
                .expect("the highlight mesh asset remains allocated");
            current_positions = test_mesh_positions(mesh);
            if current_positions != old_positions {
                replaced = true;
                break;
            }
            assert!(
                current_positions == old_positions,
                "the existing mesh must remain intact until replacement is ready"
            );
            assert!(
                app.world()
                    .get::<RoadHighlightMesh>(highlight_entity)
                    .unwrap()
                    .ready
            );
        }

        assert!(
            replaced,
            "the latest camera motion must not starve an active geometry job"
        );
        assert!(
            current_positions == expected_first_positions,
            "the complete first camera snapshot should replace the old mesh in one step"
        );
        assert_eq!(
            app.world()
                .resource::<RoadMeshRefresh>()
                .last_widths
                .as_ref(),
            Some(&first_widths),
            "the first complete snapshot commits atomically before the coalesced latest request"
        );
        assert!(
            app.world()
                .resource::<RoadMeshRefresh>()
                .pending_request
                .is_some()
        );

        let committed_positions = current_positions;
        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(false);
        app.update();
        assert!(app.world().resource::<RoadMeshRefresh>().job.is_none());
        assert!(
            app.world()
                .get::<RoadHighlightMesh>(highlight_entity)
                .unwrap()
                .ready
        );
        assert_eq!(
            test_mesh_positions(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&mesh_handle)
                    .unwrap()
            ),
            committed_positions,
            "leaving Planet view cancels unfinished work and retains its last mesh"
        );

        app.world_mut()
            .resource_mut::<Exploration>()
            .set_planet_view_open(true);
        app.update();
        assert!(app.world().resource::<RoadMeshRefresh>().job.is_some());
        assert!(
            app.world()
                .get::<RoadHighlightMesh>(highlight_entity)
                .unwrap()
                .ready
        );

        let next_epoch = crate::map::WorldEpoch::new(22);
        app.insert_resource(next_epoch);
        app.update();
        let refresh = app.world().resource::<RoadMeshRefresh>();
        assert_eq!(refresh.job.as_ref().map(|job| job.epoch), Some(next_epoch));
        assert!(refresh.completed_geometry.is_none());
        assert!(
            !app.world()
                .get::<RoadHighlightMesh>(highlight_entity)
                .unwrap()
                .ready
        );
        assert_eq!(
            test_mesh_positions(
                app.world()
                    .resource::<Assets<Mesh>>()
                    .get(&mesh_handle)
                    .unwrap()
            ),
            committed_positions,
            "an epoch change invalidates old display state without dropping its asset"
        );
    }
}
