mod blends;
mod clusters;
mod resolution;

pub(in crate::worldgen) use blends::mark_blends;
pub(in crate::worldgen) use clusters::absorb_small_clusters;
pub(in crate::worldgen) use resolution::resolve_transitions;
