//! A lock-free buddy allocator for `no_std`.
//!
//! The allocator manages a region of page frames, virtual memory space, or equivalent, using atomic
//! operations and compare-and-swap retry loops to allow allocation and deallocation using only
//! shared references without internal locking. It does not depend on `std`, and uses `alloc` only for
//! tests.
//!
//! The allocator's performance is architecture-dependent, but will automatically select the largest
//! word size that the target can compare-and-swap atomically.
//!
//! # Algorithm
//!
//! Safe, concurrent allocation is achieved by attempting allocation and then undoing work if a
//! conflict is found, repeating until a subsection of the tree is allocated atomically. This wastes
//! work, but itself is no worse than spinning, except with the new advantage of allowing a core to
//! safely preempt itself from an interrupt context without deadlocking.
//!
//! The algorithm is heavily inspired by Andrea Scarselli's bunch allocator, detailed in their
//! thesis "A Lock-Free Buddy System for Scalable Memory Allocation".
//!
//! # License
//!
//! This crate is dual-licensed under the MIT and Apache 2.0 licenses.
//! See the LICENSE-* files in the repository root for details.
#![no_std]
mod branch;
mod cursor;
mod target;
#[cfg(test)]
mod test;
pub use crate::target::Atomic;
use crate::{branch::Branch, cursor::Cursor, target::DEPTH};
use core::slice;
const ENTRIES_PER_BRANCH: usize = (1 << (DEPTH + 1)) - 1;
const LEAVES_PER_BRANCH: usize = 1 << DEPTH;
const STEMS_PER_BRANCH: usize = (1 << DEPTH) - 1;
/// An allocator. Stores its order, and a shared reference to a slice of `Atomic`s, which is used to
/// store an internal tree.
#[derive(Clone)]
pub struct BranchAllocator<'a> {
    branches: &'a [Branch],
    order: usize,
}
impl<'a> BranchAllocator<'a> {
    /// Calculate the space required (in words) for the allocator's internal storage.
    ///
    /// # Arguments
    /// * `order` - Count of layers in the allocator's internal tree. An `order`-ordered allocator
    /// can manage 2^`order` distinct blocks.
    ///
    /// # Returns
    /// The length, in words (`branch_allocator::target::Atomic`, which may be any of `u8`, `u16`,
    /// `u32`, `u64`, or `u128`), of the slice that an `order`-ordered allocator requires to track its internal tree.
    ///
    /// # Example
    /// ```
    /// use branch_allocator::{BranchAllocator, Atomic};
    /// const ORDER: usize = 14; // 2 ^ 14 = 16384 pages / page frames; 64MiB managed area.
    /// static STORAGE: [Atomic; BranchAllocator::required(ORDER)] =
    ///     [const { branch_allocator::Atomic::new(0) }; _];
    /// fn initialise() {
    ///     let allocator = BranchAllocator::new(&STORAGE, ORDER).unwrap();
    ///     // ...
    /// }
    /// ```
    pub const fn required(order: usize) -> usize {
        let mut count = 0;
        let mut depth = 0;
        while depth <= order {
            count += 1 << depth;
            depth += DEPTH + 1;
        }
        count
    }
    /// Create a new branch allocator. Fails if `storage` is not large enough to track an `order`-
    /// -ordered allocator. All blocks start as fully deallocated, and invalid regions should be
    /// allocated immediately to avoid allocating them later.
    ///
    /// # Arguments
    /// * `storage` - Space to store the allocator's internal tree.
    /// * `order` - Count of layers in the allocator's internal tree. An `order`-ordered allocator
    /// can manage 2^`order` distinct blocks.
    ///
    /// # Returns
    /// * `Some(Self)` - If `storage` is large enough to manage an area of the given order.
    /// * `None` - If it is not.
    ///
    /// # Example
    /// ```
    /// use branch_allocator::{BranchAllocator, Atomic};
    /// const ORDER: usize = 14; // 2 ^ 14 = 16384 pages / page frames; 64mib managed area.
    /// static STORAGE: [Atomic; BranchAllocator::required(ORDER)] =
    ///     [const { Atomic::new(0) }; _];
    /// fn initialise() {
    ///     let allocator = BranchAllocator::new(&STORAGE, ORDER).unwrap();
    ///     // ...
    /// }
    /// ```
    pub fn new(storage: &'a [Atomic], order: usize) -> Option<Self> {
        if storage.len() < Self::required(order) {
            return None;
        }
        // SAFETY: The branch slice is created in place of the provided storage, being the same size
        // or smaller, with alignment checked above.
        let branches =
            unsafe { slice::from_raw_parts(storage.as_ptr() as *mut Branch, storage.len()) };
        for branch in branches {
            branch.zero();
        }
        Some(BranchAllocator { branches, order })
    }
    fn depth_of(global: usize) -> usize {
        (usize::BITS - (global + 1).leading_zeros() - 1) as usize
    }
    fn cursor(&self, global: usize) -> Option<Cursor<'_, 'a>> {
        let total_nodes = 2 * (1 << self.order) - 1;
        if global >= total_nodes {
            return None;
        }
        let depth = Self::depth_of(global);
        let branch_depth = depth - (depth % (DEPTH + 1));
        let mut root = global;
        while Self::depth_of(root) > branch_depth {
            root = (root - 1) / 2;
        }
        let mut branch_idx = 0;
        let mut d = 0;
        while d < branch_depth {
            branch_idx += 1 << d;
            d += DEPTH + 1;
        }
        let first_at_depth = (1 << branch_depth) - 1;
        branch_idx += root - first_at_depth;
        if branch_idx >= self.branches.len() {
            return None;
        }
        let mut path = [0; (!0usize).count_ones() as usize];
        let mut len = 0;
        let mut cur = global;
        while cur != root {
            path[len] = cur;
            len += 1;
            cur = (cur - 1) / 2;
        }
        let mut slot = 0;
        for i in (0..len).rev() {
            let node = path[i];
            let parent = (node - 1) / 2;
            if node == 2 * parent + 1 {
                slot = 2 * slot + 1;
            } else {
                slot = 2 * slot + 2;
            }
        }
        Cursor::new(self, branch_idx, slot, depth)
    }
    fn block_leaf(&self, index: usize) -> Option<usize> {
        if index < (1 << self.order) {
            Some((1 << self.order) - 1 + index)
        } else {
            None
        }
    }
    /// Attempt to allocate an `order`-ordered region containing the `index`'th block.
    ///
    /// # Arguments
    /// * `index` - The index of the block within the managed area that the region must contain.
    /// * `order` - The order of the requested region. An `order`-ordered region is 2^`order`x
    /// larger than a block.
    ///
    /// # Returns
    /// * `Some(())` - If the allocation succeeded.
    /// * `None` - If the requested region, or a subregion of it, is already allocated.
    pub fn try_allocate(&self, index: usize, order: usize) -> Option<()> {
        let leaf = self.block_leaf(index)?;
        let depth = Self::depth_of(leaf);
        let target_depth = self.order.checked_sub(order)?;
        if depth < target_depth {
            return None;
        }
        let mut cursor = self.cursor(leaf)?;
        for _ in 0..(depth - target_depth) {
            cursor = cursor.parent()?;
        }
        cursor.allocate()
    }
    /// Deallocate the previously-allocated region containg the `index`'th block.
    ///
    /// # Arguments
    /// * `index` - The index of the block from which to walk up the tree to find the region. This
    /// is the same as the `index` used for
    /// `BranchAllocator::try_allocate`.
    /// * `order` - The order of the region to be deallocated. This is the same as the `order` used
    /// for `BranchAllocator::try_allocate`.
    ///
    /// # Returns
    /// * `Some(())` - The region specified by `index` and `order` was found and verified as
    /// allocated, then successfully deallocated.
    /// * `None` - The region could not be found within the managed area, or was not marked as allocated.
    pub fn deallocate(&self, index: usize, order: usize) -> Option<()> {
        let leaf = self.block_leaf(index)?;
        let depth = Self::depth_of(leaf);
        let target_depth = self.order.checked_sub(order)?;
        if depth < target_depth {
            return None;
        }
        let mut cursor = self.cursor(leaf)?;
        for _ in 0..(depth - target_depth) {
            cursor = cursor.parent()?;
        }
        cursor.deallocate(None)
    }
}
