//! Branch-level operations to load and store atomics, and manipulate an unpacked view of the data.
//! This module is not aware of the cursor module, or the crate root module.
use core::sync::atomic::Ordering;
use crate::{ENTRIES_PER_BRANCH, STEMS_PER_BRANCH, target::{Atomic, Inner}};
pub(crate) struct Branch(Atomic);
impl Branch {
    pub(crate) fn load(&self) -> (Inner, Data) {
        let inner = self.0.load(Ordering::SeqCst);
        (inner, Data(inner))
    }
    pub(crate) fn store(&self, current: Inner, data: Data) -> Result<Inner, Inner> {
        self.0
            .compare_exchange(current, data.0, Ordering::SeqCst, Ordering::SeqCst)
    }
    pub(crate) fn zero(&self) {
        self.0.store(0, Ordering::Relaxed);
    }
}
pub(crate) struct Data(Inner);
impl Data {
    const BITS_PER_LEAF: usize = 5;
    const LEAF_MASK: Inner = 0b11111;
    const LOCKED: Inner = 1 << 0;
    const LOWER_LOCKED: Inner = 1 << 1;
    const UPPER_LOCKED: Inner = 1 << 2;
    const LOWER_COALESCING: Inner = 1 << 3;
    const UPPER_COALESCING: Inner = 1 << 4;
    fn stem_position(index: usize) -> Option<usize> {
        if index < STEMS_PER_BRANCH {
            Some(index)
        } else {
            None
        }
    }
    fn leaf_offset(index: usize) -> Option<usize> {
        if index >= STEMS_PER_BRANCH && index < ENTRIES_PER_BRANCH {
            let leaf_idx = index - STEMS_PER_BRANCH;
            Some(STEMS_PER_BRANCH + leaf_idx * Self::BITS_PER_LEAF)
        } else {
            None
        }
    }
    fn stem_bit(&self, index: usize) -> Option<bool> {
        Self::stem_position(index).map(|pos| (self.0 >> pos) & 1 != 0)
    }
    fn set_stem(&mut self, index: usize, value: bool) {
        let pos = Self::stem_position(index).unwrap();
        if value {
            self.0 |= 1 << pos;
        } else {
            self.0 &= !(1 << pos);
        }
    }
    fn leaf_bits(&self, index: usize) -> Option<Inner> {
        let offset = Self::leaf_offset(index)?;
        Some((self.0 >> offset) & Self::LEAF_MASK)
    }
    fn set_leaf(&mut self, index: usize, bits: Inner) -> Option<()> {
        let offset = Self::leaf_offset(index)?;
        self.0 = (self.0 & !(Self::LEAF_MASK << offset)) | ((bits & Self::LEAF_MASK) << offset);
        Some(())
    }
    fn modify_leaf(&mut self, index: usize, closure: impl FnOnce(Inner) -> Inner) -> Option<()> {
        let bits = self.leaf_bits(index)?;
        self.set_leaf(index, closure(bits))
    }
    pub(crate) fn lower(&self, index: usize) -> Option<usize> {
        let new = 2 * index + 1;
        if new < ENTRIES_PER_BRANCH {
            Some(new)
        } else {
            None
        }
    }
    pub(crate) fn upper(&self, index: usize) -> Option<usize> {
        let new = 2 * index + 2;
        if new < ENTRIES_PER_BRANCH {
            Some(new)
        } else {
            None
        }
    }
    pub(crate) fn locked(&self, index: usize) -> Option<bool> {
        if let Some(bit) = self.stem_bit(index) {
            Some(bit)
        } else {
            self.leaf_bits(index).map(|bits| (bits & Self::LOCKED) != 0)
        }
    }
    pub(crate) fn lower_locked(&self, index: usize) -> Option<bool> {
        self.leaf_bits(index).map(|bits| (bits & Self::LOWER_LOCKED) != 0)
    }
    pub(crate) fn upper_locked(&self, index: usize) -> Option<bool> {
        self.leaf_bits(index).map(|bits| (bits & Self::UPPER_LOCKED) != 0)
    }
    pub(crate) fn lower_coalescing(&self, index: usize) -> Option<bool> {
        self.leaf_bits(index).map(|bits| (bits & Self::LOWER_COALESCING) != 0)
    }
    pub(crate) fn upper_coalescing(&self, index: usize) -> Option<bool> {
        self.leaf_bits(index).map(|bits| (bits & Self::UPPER_COALESCING) != 0)
    }
    pub(crate) fn allocable(&self, index: usize) -> Option<bool> {
        if Self::stem_position(index).is_some() {
            self.stem_bit(index).map(|bit| !bit)
        } else {
            self.leaf_bits(index).map(|bits| bits == 0)
        }
    }
    pub(crate) fn lock(&mut self, index: usize) {
        if Self::stem_position(index).is_some() {
            self.set_stem(index, true)
        } else {
            self.set_leaf(index, Self::LOCKED | Self::LOWER_LOCKED | Self::UPPER_LOCKED).unwrap()
        }
    }
    pub(crate) fn unlock(&mut self, index: usize) {
        if Self::stem_position(index).is_some() {
            self.set_stem(index, false)
        } else {
            self.set_leaf(index, 0).unwrap()
        }
    }
    pub(crate) fn lock_lower(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit | Self::LOWER_LOCKED).unwrap()
    }
    pub(crate) fn lock_upper(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit | Self::UPPER_LOCKED).unwrap()
    }
    pub(crate) fn unlock_lower(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit & !Self::LOWER_LOCKED).unwrap()
    }
    pub(crate) fn unlock_upper(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit & !Self::UPPER_LOCKED).unwrap()
    }
    pub(crate) fn coalesce_lower(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit | Self::LOWER_COALESCING).unwrap()
    }
    pub(crate) fn coalesce_upper(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit | Self::UPPER_COALESCING).unwrap()
    }
    pub(crate) fn uncoalesce_lower(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit & !Self::LOWER_COALESCING).unwrap()
    }
    pub(crate) fn uncoalesce_upper(&mut self, index: usize) {
        self.modify_leaf(index, |bit| bit & !Self::UPPER_COALESCING).unwrap()
    }
    pub(crate) fn parent(&self, index: usize) -> Option<usize> {
        if index == 0 {
            None
        } else {
            Some((index - 1) / 2)
        }
    }
    pub(crate) fn lock_children(&mut self, index: usize) {
        if let Some(lower) = self.lower(index) {
            self.lock(lower);
            self.lock_children(lower);
        }
        if let Some(upper) = self.upper(index) {
            self.lock(upper);
            self.lock_children(upper);
        }
    }
    pub(crate) fn unlock_children(&mut self, index: usize) {
        if index < STEMS_PER_BRANCH {
            if let Some(lower) = self.lower(index) {
                self.unlock_children(lower);
            }
            if let Some(upper) = self.upper(index) {
                self.unlock_children(upper);
            }
        }
        self.unlock(index);
    }
    pub(crate) fn lock_parents(&mut self, mut index: usize) {
        while let Some(parent) = self.parent(index) {
            self.lock(parent);
            index = parent;
        }
    }
}