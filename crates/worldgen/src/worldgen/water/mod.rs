mod classification;
mod normalization;
mod rivers;

pub(super) use classification::{
    classify_cover, classify_landform, classify_slope, classify_water_depth,
};
#[cfg(test)]
pub(super) use normalization::cell_zone;
pub(super) use normalization::{nearest_cell, normalize_water_bodies};
pub(super) use rivers::{cell_chain, paint_rivers};
