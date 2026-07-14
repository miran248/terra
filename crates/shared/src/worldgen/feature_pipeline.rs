mod bridges;
mod transitions;

pub(super) use bridges::{build_bridges, paint_features, widen_band_sym};
pub(super) use transitions::{absorb_small_clusters, mark_blends, resolve_transitions};
