use bevy::prelude::*;

const DRAG_MARGIN: f32 = 8.0;
pub(crate) const DEFAULT_POINTS_PER_STROKE: usize = 29;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DragInputFrame {
    pub position: Vec2,
    pub pressed: bool,
}

/// Cursor/button samples for a far-zoom antipode drag. The route stays just
/// outside the projected globe and splits into valid captured strokes when a
/// single viewport-height drag cannot cover the required angular distance.
pub(crate) fn opposite_side_drag(
    start: Vec3,
    width: f32,
    height: f32,
    points_per_stroke: usize,
) -> Option<Vec<DragInputFrame>> {
    let _start_direction = start.try_normalize()?;
    let viewport = Vec2::new(width, height);
    if !viewport.is_finite() || viewport.min_element() <= 2.0 * DRAG_MARGIN || points_per_stroke < 2
    {
        return None;
    }

    let fov_y = PerspectiveProjection::default().fov;
    let radians_per_pixel = shared::planet_view::orbit_radians_per_logical_pixel(
        shared::planet_view::PLANET_VIEW_FAR_RADIUS,
        height,
        fov_y,
    );
    if !radians_per_pixel.is_finite() || radians_per_pixel <= 0.0 {
        return None;
    }

    let x = width - DRAG_MARGIN;
    let normalized_x = x / width * 2.0 - 1.0;
    let ray_angle = (normalized_x * (width / height) * (fov_y * 0.5).tan()).atan();
    let globe_angle = (terra_geometry::sphere::PLANET_RADIUS
        / shared::planet_view::PLANET_VIEW_FAR_RADIUS)
        .asin();
    if ray_angle <= globe_angle {
        return None;
    }

    let mut remaining_pixels = std::f32::consts::PI / radians_per_pixel;
    let pixels_per_stroke = height - 2.0 * DRAG_MARGIN;
    let mut frames = Vec::new();
    while remaining_pixels > 0.0 {
        let stroke_pixels = remaining_pixels.min(pixels_per_stroke);
        for sample in 0..points_per_stroke {
            let progress = sample as f32 / (points_per_stroke - 1) as f32;
            frames.push(DragInputFrame {
                position: Vec2::new(x, DRAG_MARGIN + progress * stroke_pixels),
                pressed: true,
            });
        }
        let end = Vec2::new(x, DRAG_MARGIN + stroke_pixels);
        frames.push(DragInputFrame {
            position: end,
            pressed: false,
        });
        remaining_pixels -= stroke_pixels;
        if remaining_pixels > 1e-3 {
            // Reposition only while released, then begin a fresh captured stroke.
            frames.push(DragInputFrame {
                position: Vec2::new(x, DRAG_MARGIN),
                pressed: false,
            });
        }
    }
    Some(frames)
}

pub(crate) fn surface_drag_toward_direction(
    start: Transform,
    target_direction: Vec3,
    start_cursor: Vec2,
    width: f32,
    height: f32,
    vertical_fov: f32,
) -> Option<Vec<DragInputFrame>> {
    const MAX_PATH_FRAMES: usize = 2_000;
    const MAX_REANCHORS: usize = 24;
    const TARGET_TOLERANCE_RADIANS: f32 = 0.01;
    const CURSOR_STEP_PIXELS: [f32; 4] = [8.0, 16.0, 32.0, 48.0];

    let viewport = Vec2::new(width, height);
    let direction = start.translation.try_normalize()?;
    let target = target_direction.try_normalize()?;
    if !viewport.is_finite()
        || viewport.min_element() <= 2.0 * DRAG_MARGIN
        || !start_cursor.is_finite()
        || start_cursor.x < DRAG_MARGIN
        || start_cursor.x > width - DRAG_MARGIN
        || start_cursor.y < DRAG_MARGIN
        || start_cursor.y > height - DRAG_MARGIN
        || !vertical_fov.is_finite()
        || !(0.0..std::f32::consts::PI).contains(&vertical_fov)
    {
        return None;
    }

    let mut anchor = shared::planet_view::planet_surface_hit_direction(
        start,
        start_cursor,
        viewport,
        vertical_fov,
    )?;
    let mut camera = start;
    let mut cursor = start_cursor;
    let mut frames = Vec::new();
    frames.push(DragInputFrame {
        position: cursor,
        pressed: true,
    });
    let center = viewport * 0.5;
    let mut reanchors = 0;
    while camera
        .translation
        .normalize_or(direction)
        .angle_between(target)
        > TARGET_TOLERANCE_RADIANS
    {
        if frames.len() >= MAX_PATH_FRAMES {
            return None;
        }
        let direction = camera.translation.normalize_or(Vec3::Y);
        let current_error = direction.angle_between(target);
        let best_step = (0..16)
            .flat_map(|index| {
                let angle = std::f32::consts::TAU * index as f32 / 16.0;
                let axis = Vec2::new(angle.cos(), angle.sin());
                CURSOR_STEP_PIXELS
                    .into_iter()
                    .map(move |distance| cursor + axis * distance)
            })
            .filter(|candidate| {
                candidate.x >= DRAG_MARGIN
                    && candidate.x <= width - DRAG_MARGIN
                    && candidate.y >= DRAG_MARGIN
                    && candidate.y <= height - DRAG_MARGIN
            })
            .filter_map(|candidate| {
                let hit = shared::planet_view::planet_surface_hit_direction(
                    camera,
                    candidate,
                    viewport,
                    vertical_fov,
                )?;
                let rotation = shared::planet_view::planet_surface_drag_rotation(anchor, hit)?;
                let moved_direction = (rotation * direction).normalize_or(direction);
                let error = moved_direction.angle_between(target);
                (error + 1e-5 < current_error).then_some((candidate, rotation, error))
            })
            .min_by(|left, right| left.2.total_cmp(&right.2));

        if let Some((next_cursor, rotation, _)) = best_step {
            frames.push(DragInputFrame {
                position: next_cursor,
                pressed: true,
            });
            camera.translation = rotation * camera.translation;
            camera.rotation = rotation * camera.rotation;
            cursor = next_cursor;
            continue;
        }

        if reanchors >= MAX_REANCHORS {
            return None;
        }
        frames.push(DragInputFrame {
            position: cursor,
            pressed: false,
        });
        cursor = center;
        anchor = shared::planet_view::planet_surface_hit_direction(
            camera,
            cursor,
            viewport,
            vertical_fov,
        )?;
        frames.push(DragInputFrame {
            position: cursor,
            pressed: false,
        });
        frames.push(DragInputFrame {
            position: cursor,
            pressed: true,
        });
        reanchors += 1;
    }

    if camera
        .translation
        .normalize_or(direction)
        .angle_between(target)
        > TARGET_TOLERANCE_RADIANS + 0.001
    {
        return None;
    }
    frames.push(DragInputFrame {
        position: cursor,
        pressed: false,
    });
    Some(frames)
}
