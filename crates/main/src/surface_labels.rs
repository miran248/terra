//! Textured surface ribbons shared by Planet-view and minimap labels.

use std::collections::{BTreeSet, HashMap};

use ab_glyph::{Font, FontArc, ScaleFont, point};
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use terra_geometry::sphere::great_circle_point;

pub(crate) const ATLAS_FONT_SIZE: f32 = 48.0;
pub(crate) const PLANET_LABEL_LAYER: usize = 3;
pub(crate) const MINIMAP_LABEL_LAYER: usize = 4;
const ATLAS_WIDTH: u32 = 1024;
const ATLAS_PADDING: u32 = 2;

#[derive(Clone, Copy, Debug)]
pub(crate) struct AtlasGlyph {
    pub advance: f32,
    pub bounds: Rect,
    uv_min: Vec2,
    uv_max: Vec2,
    has_ink: bool,
}

#[derive(Resource, Clone)]
pub(crate) struct SurfaceGlyphAtlas {
    texture: Handle<Image>,
    glyphs: HashMap<char, AtlasGlyph>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceLabelView {
    Planet,
    Minimap,
}

#[derive(Component, Clone, Debug)]
pub(crate) struct SurfaceLabelRibbon {
    pub marker_index: usize,
    pub view: SurfaceLabelView,
    pub label: String,
}

#[derive(Default)]
pub(crate) struct SurfaceRibbonMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub glyph_centers: Vec<Vec3>,
    pub glyph_corners: Vec<[Vec3; 4]>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfaceRibbonLayout {
    pub camera_right: Vec3,
    pub camera_up: Vec3,
    pub world_per_atlas_pixel: f32,
    pub label_offset_atlas_pixels: Vec2,
    pub clearance: f32,
}

struct RasterGlyph {
    character: char,
    advance: f32,
    bounds: Rect,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    atlas_x: u32,
    atlas_y: u32,
}

pub(crate) fn build_glyph_atlas(
    labels: impl IntoIterator<Item = impl AsRef<str>>,
    images: &mut Assets<Image>,
) -> Option<SurfaceGlyphAtlas> {
    static FONT_DATA: &[u8] = include_bytes!("../assets/fonts/MonaspaceNeon-Regular.otf");
    let font = FontArc::try_from_slice(FONT_DATA).ok()?;
    let scaled = font.as_scaled(ATLAS_FONT_SIZE);
    let characters = labels
        .into_iter()
        .flat_map(|label| label.as_ref().chars().collect::<Vec<_>>())
        .collect::<BTreeSet<_>>();
    let mut raster_glyphs = Vec::with_capacity(characters.len());

    for character in characters {
        let glyph_id = scaled.glyph_id(character);
        let advance = scaled.h_advance(glyph_id);
        let positioned = glyph_id.with_scale_and_position(ATLAS_FONT_SIZE, point(0.0, 0.0));
        let Some(outline) = font.outline_glyph(positioned) else {
            raster_glyphs.push(RasterGlyph {
                character,
                advance,
                bounds: Rect::default(),
                width: 0,
                height: 0,
                pixels: Vec::new(),
                atlas_x: 0,
                atlas_y: 0,
            });
            continue;
        };

        let px_bounds = outline.px_bounds();
        let min_x = px_bounds.min.x.floor();
        let max_x = px_bounds.max.x.ceil();
        let min_y = px_bounds.min.y.floor();
        let max_y = px_bounds.max.y.ceil();
        let width = (max_x - min_x).max(0.0) as u32;
        let height = (max_y - min_y).max(0.0) as u32;
        let mut pixels = vec![0; (width * height) as usize];
        outline.draw(|x, y, coverage| {
            let index = (y * width + x) as usize;
            if let Some(pixel) = pixels.get_mut(index) {
                *pixel = (coverage * 255.0).round() as u8;
            }
        });
        // Font-space y points up; atlas-space raster rows point down.
        let bounds = Rect::from_corners(Vec2::new(min_x, -max_y), Vec2::new(max_x, -min_y));
        raster_glyphs.push(RasterGlyph {
            character,
            advance,
            bounds,
            width,
            height,
            pixels,
            atlas_x: 0,
            atlas_y: 0,
        });
    }

    let mut x = ATLAS_PADDING;
    let mut y = ATLAS_PADDING;
    let mut row_height = 0;
    for glyph in raster_glyphs
        .iter_mut()
        .filter(|glyph| glyph.width > 0 && glyph.height > 0)
    {
        if x + glyph.width + ATLAS_PADDING > ATLAS_WIDTH {
            x = ATLAS_PADDING;
            y += row_height + ATLAS_PADDING;
            row_height = 0;
        }
        glyph.atlas_x = x;
        glyph.atlas_y = y;
        x += glyph.width + ATLAS_PADDING;
        row_height = row_height.max(glyph.height);
    }
    let height = (y + row_height + ATLAS_PADDING)
        .max(ATLAS_PADDING * 2)
        .next_power_of_two();
    if height > 2048 {
        return None;
    }
    let mut rgba = vec![0; (ATLAS_WIDTH * height * 4) as usize];
    let mut glyphs = HashMap::with_capacity(raster_glyphs.len());
    for glyph in raster_glyphs {
        let has_ink = glyph.width > 0 && glyph.height > 0;
        if has_ink {
            for row in 0..glyph.height {
                for column in 0..glyph.width {
                    let coverage = glyph.pixels[(row * glyph.width + column) as usize];
                    let pixel = ((glyph.atlas_y + row) * ATLAS_WIDTH + glyph.atlas_x + column) * 4;
                    rgba[pixel as usize..pixel as usize + 4]
                        .copy_from_slice(&[255, 255, 255, coverage]);
                }
            }
        }
        let (uv_min, uv_max) = if has_ink {
            (
                Vec2::new(
                    glyph.atlas_x as f32 / ATLAS_WIDTH as f32,
                    glyph.atlas_y as f32 / height as f32,
                ),
                Vec2::new(
                    (glyph.atlas_x + glyph.width) as f32 / ATLAS_WIDTH as f32,
                    (glyph.atlas_y + glyph.height) as f32 / height as f32,
                ),
            )
        } else {
            (Vec2::ZERO, Vec2::ZERO)
        };
        glyphs.insert(
            glyph.character,
            AtlasGlyph {
                advance: glyph.advance,
                bounds: glyph.bounds,
                uv_min,
                uv_max,
                has_ink,
            },
        );
    }

    let image = Image::new(
        Extent3d {
            width: ATLAS_WIDTH,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    Some(SurfaceGlyphAtlas {
        texture: images.add(image),
        glyphs,
    })
}

pub(crate) fn surface_label_material(
    atlas: &SurfaceGlyphAtlas,
    materials: &mut Assets<StandardMaterial>,
    color: Color,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        base_color_texture: Some(atlas.texture.clone()),
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        fog_enabled: false,
        unlit: true,
        ..default()
    })
}

pub(crate) fn ribbon_mesh(ribbon: &SurfaceRibbonMesh, alpha: f32) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    write_ribbon_mesh(&mut mesh, ribbon, alpha);
    mesh
}

pub(crate) fn write_ribbon_mesh(mesh: &mut Mesh, ribbon: &SurfaceRibbonMesh, alpha: f32) {
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, ribbon.positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, ribbon.normals.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, ribbon.uvs.clone());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vec![[1.0, 1.0, 1.0, alpha.clamp(0.0, 1.0)]; ribbon.positions.len()],
    );
    let indices = (0..ribbon.glyph_centers.len() as u32).flat_map(|glyph| {
        let base = glyph * 4;
        [base, base + 1, base + 2, base, base + 2, base + 3]
    });
    mesh.insert_indices(Indices::U32(indices.collect()));
}

pub(crate) fn build_surface_ribbon(
    atlas: &SurfaceGlyphAtlas,
    text: &str,
    anchor: Vec3,
    layout: SurfaceRibbonLayout,
    mut surface_radius: impl FnMut(Vec3) -> f32,
) -> SurfaceRibbonMesh {
    let anchor_direction = anchor.normalize_or(Vec3::Y);
    let camera_right = layout.camera_right.normalize_or(Vec3::X);
    let tangent = (camera_right - anchor_direction * camera_right.dot(anchor_direction))
        .normalize_or(anchor_direction.any_orthonormal_vector());
    let path_radius = anchor.length().max(f32::EPSILON);
    let mut terrain_clearance_radius = anchor.length().max(surface_radius(anchor_direction));
    let mut glyphs = Vec::new();
    let mut cursor = layout.label_offset_atlas_pixels.x;
    for character in text.chars() {
        let Some(glyph) = atlas.glyphs.get(&character).copied() else {
            continue;
        };
        if glyph.has_ink {
            let glyph_mid = glyph.bounds.center().x;
            let half_width = glyph.bounds.width() * 0.5;
            for local_x in [-half_width, 0.0, half_width] {
                let sample = great_circle_point(
                    anchor_direction,
                    tangent,
                    (cursor + glyph_mid + local_x) * layout.world_per_atlas_pixel,
                    path_radius,
                )
                .normalize_or(anchor_direction);
                terrain_clearance_radius = terrain_clearance_radius.max(surface_radius(sample));
            }
        }
        glyphs.push((glyph, cursor));
        cursor += glyph.advance;
    }
    // One radial envelope keeps the whole name a single smooth ribbon over
    // uneven terrain instead of stepping each character up and down.
    let ribbon_radius = terrain_clearance_radius + layout.clearance;
    let mut output = SurfaceRibbonMesh::default();

    for (glyph, cursor) in glyphs {
        if glyph.has_ink {
            let glyph_mid = glyph.bounds.center().x;
            let center_distance = (cursor + glyph_mid) * layout.world_per_atlas_pixel;
            let center_on_base =
                great_circle_point(anchor_direction, tangent, center_distance, ribbon_radius);
            let center_direction = center_on_base.normalize_or(anchor_direction);
            let glyph_tangent = (tangent * (center_distance / ribbon_radius).cos()
                - anchor_direction * (center_distance / ribbon_radius).sin())
            .normalize_or(tangent);
            let mut glyph_up = center_direction
                .cross(glyph_tangent)
                .normalize_or(layout.camera_up);
            if glyph_up.dot(layout.camera_up) < 0.0 {
                glyph_up = -glyph_up;
            }

            let glyph_center = center_direction * ribbon_radius
                + glyph_up
                    * (glyph.bounds.center().y + layout.label_offset_atlas_pixels.y)
                    * layout.world_per_atlas_pixel;
            let left = glyph.bounds.min.x - glyph_mid;
            let right = glyph.bounds.max.x - glyph_mid;
            let bottom = glyph.bounds.min.y - glyph.bounds.center().y;
            let top = glyph.bounds.max.y - glyph.bounds.center().y;
            let corners = [
                glyph_center
                    + glyph_tangent * (left * layout.world_per_atlas_pixel)
                    + glyph_up * (bottom * layout.world_per_atlas_pixel),
                glyph_center
                    + glyph_tangent * (right * layout.world_per_atlas_pixel)
                    + glyph_up * (bottom * layout.world_per_atlas_pixel),
                glyph_center
                    + glyph_tangent * (right * layout.world_per_atlas_pixel)
                    + glyph_up * (top * layout.world_per_atlas_pixel),
                glyph_center
                    + glyph_tangent * (left * layout.world_per_atlas_pixel)
                    + glyph_up * (top * layout.world_per_atlas_pixel),
            ];
            output.glyph_centers.push(glyph_center);
            output.glyph_corners.push(corners);
            for (index, position) in corners.into_iter().enumerate() {
                output.positions.push(position.to_array());
                output.normals.push(center_direction.to_array());
                let uv = match index {
                    0 => Vec2::new(glyph.uv_min.x, glyph.uv_max.y),
                    1 => glyph.uv_max,
                    2 => Vec2::new(glyph.uv_max.x, glyph.uv_min.y),
                    _ => glyph.uv_min,
                };
                output.uvs.push(uv.to_array());
            }
        }
    }
    output
}
