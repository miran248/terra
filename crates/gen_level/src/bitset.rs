pub const FACE_FLAG_ROAD: u8 = 1;
pub const FACE_FLAG_TOWN: u8 = 2;
pub const FACE_FLAG_BRIDGE: u8 = 4;

pub struct BitSet(pub Vec<u64>);

impl BitSet {
    pub fn new(n: usize) -> Self { Self(vec![0; n.div_ceil(64)]) }
    pub fn insert(&mut self, i: usize) { self.0[i >> 6] |= 1u64 << (i & 63); }
    pub fn contains(&self, i: usize) -> bool { self.0[i >> 6] & (1u64 << (i & 63)) != 0 }
    pub fn iter<'a>(&'a self) -> impl Iterator<Item = usize> + 'a {
        (0..self.0.len() * 64).filter(|i| self.contains(*i))
    }
    pub fn drain_all(&mut self) { self.0.fill(0); }
}
