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
    let globe_angle =
        (shared::sphere::PLANET_RADIUS / shared::planet_view::PLANET_VIEW_FAR_RADIUS).asin();
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

pub(crate) fn sample_drag_frame(
    frames: &[DragInputFrame],
    fraction: f32,
) -> Option<DragInputFrame> {
    let last = frames.len().checked_sub(1)?;
    let index = (fraction.clamp(0.0, 1.0) * last as f32).round() as usize;
    frames.get(index.min(last)).copied()
}
