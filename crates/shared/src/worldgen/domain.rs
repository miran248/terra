use std::ops::{Deref, DerefMut, Index, IndexMut};

use crate::topology::{CellId, FaceId};

macro_rules! field {
    ($name:ident, $id:ty) => {
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub(super) struct $name<T>(Vec<T>);

        impl<T> Default for $name<T> {
            fn default() -> Self {
                Self(Vec::new())
            }
        }

        impl<T> $name<T> {
            pub(super) fn dense(&self) -> &[T] {
                &self.0
            }
        }

        impl<T> From<Vec<T>> for $name<T> {
            fn from(values: Vec<T>) -> Self {
                Self(values)
            }
        }

        impl<T> Deref for $name<T> {
            type Target = [T];

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl<T> DerefMut for $name<T> {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }

        impl<T> Index<$id> for $name<T> {
            type Output = T;

            fn index(&self, id: $id) -> &Self::Output {
                &self.0[id.index()]
            }
        }

        impl<T> IndexMut<$id> for $name<T> {
            fn index_mut(&mut self, id: $id) -> &mut Self::Output {
                &mut self.0[id.index()]
            }
        }

        impl<T> IntoIterator for $name<T> {
            type Item = T;
            type IntoIter = std::vec::IntoIter<T>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.into_iter()
            }
        }

        impl<'a, T> IntoIterator for &'a $name<T> {
            type Item = &'a T;
            type IntoIter = std::slice::Iter<'a, T>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.iter()
            }
        }
    };
}

field!(CellField, CellId);
field!(FaceField, FaceId);

/// Dense membership set whose identity domain is terrain cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CellSet(Vec<u64>);

pub(super) trait CellIndex {
    fn cell_index(self) -> usize;
}

impl CellIndex for CellId {
    fn cell_index(self) -> usize {
        self.index()
    }
}

impl CellIndex for usize {
    fn cell_index(self) -> usize {
        self
    }
}

impl CellSet {
    pub(super) fn new(cell_count: usize) -> Self {
        Self(vec![0; cell_count.div_ceil(64)])
    }

    pub(super) fn insert(&mut self, cell: impl CellIndex) {
        let index = cell.cell_index();
        self.0[index >> 6] |= 1u64 << (index & 63);
    }

    pub(super) fn contains(&self, cell: impl CellIndex) -> bool {
        let index = cell.cell_index();
        self.0
            .get(index >> 6)
            .is_some_and(|word| word & (1u64 << (index & 63)) != 0)
    }

    pub(super) fn remove(&mut self, cell: impl CellIndex) {
        let index = cell.cell_index();
        self.0[index >> 6] &= !(1u64 << (index & 63));
    }
}
