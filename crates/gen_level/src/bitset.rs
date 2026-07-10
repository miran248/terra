pub struct BitSet(pub Vec<u64>);

impl BitSet {
    pub fn new(n: usize) -> Self { Self(vec![0; n.div_ceil(64)]) }
    pub fn insert(&mut self, i: usize) { self.0[i >> 6] |= 1u64 << (i & 63); }
    pub fn contains(&self, i: usize) -> bool { self.0[i >> 6] & (1u64 << (i & 63)) != 0 }
}
