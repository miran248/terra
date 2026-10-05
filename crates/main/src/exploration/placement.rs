use super::*;
use bevy::ecs::system::SystemParam;

#[derive(SystemParam)]
pub(super) struct Placement<'w, 's> {
    spatial: SpatialQuery<'w, 's>,
    ground: Query<'w, 's, (), (With<Ground>, Without<crate::chunks::WorldObstacle>)>,
    water: Option<Res<'w, Liquid>>,
}

impl Placement<'_, '_> {
    pub fn wet(&self, position: Vec3, bottom: f32) -> bool {
        self.water.as_ref().is_some_and(|w| {
            position.length() - bottom < w.0.facet_radius(position.normalize(), 0.0)
        })
    }
    pub fn support(
        &self,
        entity: Entity,
        position: Vec3,
        heading: Vec3,
        kind: Kind,
    ) -> Option<Vec3> {
        let up = position.normalize();
        let offsets = shared::car_prototype::support_origins(position, heading);
        offsets
            .into_iter()
            .filter_map(|origin| {
                self.spatial.cast_ray(
                    origin,
                    Dir3::new(-up).unwrap(),
                    // Include the plane collision envelope and speculative contact margin.
                    if kind == Kind::Plane { 1.25 } else { 0.75 },
                    false,
                    &SpatialQueryFilter::from_excluded_entities([entity]),
                )
            })
            .filter(|h| self.ground.contains(h.entity) && h.normal.dot(up) > 0.7)
            .min_by(|a, b| a.distance.total_cmp(&b.distance))
            .map(|h| h.normal)
    }
    /// Fit the attitude across the footprint, so crossing a facet boundary
    /// cannot lift an axle by adopting just the nearest triangle's slope.
    pub fn car_attitude(&self, entity: Entity, position: Vec3, heading: Vec3) -> Option<Vec3> {
        let up = position.normalize();
        let origins = shared::car_prototype::support_origins(position, heading);
        let mut points = [Vec3::ZERO; 4];
        for (point, origin) in points.iter_mut().zip(origins.into_iter().skip(1)) {
            let origin = origin + up * 2.0;
            let hit = self.spatial.cast_ray(
                origin,
                Dir3::new(-up).ok()?,
                5.0,
                false,
                &SpatialQueryFilter::from_excluded_entities([entity]),
            )?;
            if !self.ground.contains(hit.entity) || hit.normal.dot(up) <= 0.7 {
                return None;
            }
            *point = origin - up * hit.distance;
        }
        let across = points[0] + points[2] - points[1] - points[3];
        let along = points[0] + points[1] - points[2] - points[3];
        let normal = across.cross(along).try_normalize()?;
        (normal.dot(up) > 0.7).then_some(normal)
    }

    pub fn clearance(&self, entity: Entity, position: Vec3) -> f32 {
        let solid = self
            .spatial
            .cast_ray(
                position,
                Dir3::new(-position.normalize()).unwrap(),
                5000.0,
                false,
                &SpatialQueryFilter::from_excluded_entities([entity]),
            )
            .map_or(5000.0, |h| h.distance);
        let liquid = self.water.as_ref().map_or(5000.0, |w| {
            let r = w.0.facet_radius(position.normalize(), 0.0);
            if r > 0.0 {
                (position.length() - r).max(0.0)
            } else {
                5000.0
            }
        });
        solid.min(liquid)
    }
    pub fn obstacle(&self, entity: Entity) -> bool {
        !self.ground.contains(entity)
    }
    pub fn locate(
        &self,
        origin: Vec3,
        heading: Vec3,
        kind: Option<Kind>,
        excluded: &[Entity],
        radius: f32,
    ) -> Option<(Vec3, Vec3)> {
        let shape = Self::shape(kind);
        let candidates = if kind.is_some() {
            shared::placement::PlacementCandidateSearch::vehicle(origin, heading, radius)
        } else {
            shared::placement::PlacementCandidateSearch::fixed_heading(origin, heading, radius)
        };
        for (candidate, heading) in candidates {
            if let Some(p) = self.at_with_shape(candidate, heading, kind, excluded, &shape) {
                return Some((p, tangent(heading, p.normalize())));
            }
        }
        None
    }
    pub fn at(
        &self,
        origin: Vec3,
        heading: Vec3,
        kind: Option<Kind>,
        excluded: &[Entity],
    ) -> Option<Vec3> {
        let shape = Self::shape(kind);
        self.at_with_shape(origin, heading, kind, excluded, &shape)
    }

    fn shape(kind: Option<Kind>) -> Collider {
        kind.map_or_else(
            || crate::asset_collision::actor_body("actor.player").0,
            Kind::collider,
        )
    }

    pub(super) fn at_with_shape(
        &self,
        origin: Vec3,
        heading: Vec3,
        kind: Option<Kind>,
        excluded: &[Entity],
        shape: &Collider,
    ) -> Option<Vec3> {
        let up = origin.normalize();
        let filter = SpatialQueryFilter::from_excluded_entities(excluded.iter().copied());
        let floor = self.spatial.cast_ray(
            origin + up * 150.0,
            Dir3::new(-up).ok()?,
            origin.length() + 150.0,
            false,
            &filter,
        )?;
        if !self.ground.contains(floor.entity) {
            return None;
        }
        let slope = if kind == Some(Kind::Plane) {
            8.0_f32
        } else {
            40.0
        };
        if floor.normal.dot(up) < slope.to_radians().cos() {
            return None;
        }
        let height = kind.map_or(crate::map::player_half_height() + 0.08, |k| {
            k.height() + 0.08
        });
        let center = origin + up * (150.0 - floor.distance + height);
        if self.wet(center, height) {
            return None;
        }
        let forward = tangent(heading, up);
        let rotation = facing(forward, up);
        if !self
            .spatial
            .shape_intersections(shape, center, rotation, &filter)
            .is_empty()
        {
            return None;
        }
        // Sample the footprint/corridor densely for support and sweep the full body
        // between samples, so thin obstacles cannot fit between probe positions.
        let width = kind.map_or(0.25, |k| if k == Kind::Plane { 4.0 } else { 0.9 });
        let length: f32 = kind.map_or(0.25, |k| if k == Kind::Plane { 50.0 } else { 1.6 });
        let start = kind.map_or(-0.25, |k| if k == Kind::Plane { -2.5 } else { -1.6 });
        let side = forward.cross(up);
        let mut previous: Option<Vec3> = None;
        let steps = ((length - start) / 0.5).ceil() as usize;
        for i in 0..=steps {
            let d = start + (length - start) * i as f32 / steps as f32;
            let sample = center + forward * d;
            let local_up = sample.normalize();
            let sample = if kind == Some(Kind::Plane) {
                let h = self.spatial.cast_ray(
                    sample + local_up * 150.0,
                    Dir3::new(-local_up).ok()?,
                    300.0,
                    false,
                    &filter,
                )?;
                if !self.ground.contains(h.entity)
                    || h.normal.dot(local_up) < slope.to_radians().cos()
                {
                    return None;
                }
                let adjusted = sample + local_up * (150.0 - h.distance + height);
                let from = previous.unwrap_or(center);
                let delta = adjusted - from;
                let rise = delta.dot(local_up).abs();
                let run = (delta - local_up * delta.dot(local_up)).length();
                if rise > run * slope.to_radians().tan() + 0.1 {
                    return None;
                }
                adjusted
            } else {
                sample
            };
            let rotation = facing(tangent(forward, local_up), local_up);
            let mut surface = None;
            for x in [-width, 0.0, width] {
                let point = sample + side * x;
                let h = self.spatial.cast_ray(
                    point + local_up * 2.0,
                    Dir3::new(-local_up).ok()?,
                    4.0 + height,
                    false,
                    &filter,
                )?;
                if !self.ground.contains(h.entity)
                    || h.normal.dot(local_up) < slope.to_radians().cos()
                    || (h.distance - (2.0 + height)).abs() > 0.25
                {
                    return None;
                }
                if self.wet(point + local_up * (2.0 - h.distance + height), height) {
                    return None;
                }
                if x == 0.0 {
                    surface = Some(point + local_up * (2.0 - h.distance + height));
                }
            }
            if kind == Some(Kind::Plane) {
                let next = surface?;
                if !self
                    .spatial
                    .shape_intersections(shape, next, rotation, &filter)
                    .is_empty()
                {
                    return None;
                }
                if let Some(prev) = previous {
                    let delta: Vec3 = next - prev;
                    if self
                        .spatial
                        .cast_shape(
                            shape,
                            prev,
                            rotation,
                            Dir3::new(delta).ok()?,
                            &ShapeCastConfig::from_max_distance(delta.length()),
                            &filter,
                        )
                        .is_some()
                    {
                        return None;
                    }
                }
                previous = Some(next);
            }
        }
        Some(center)
    }
}
