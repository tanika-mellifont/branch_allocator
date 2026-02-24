//! Operations pertaining to walking the tree. This module is not aware of the crate root module.
use crate::{
    BranchAllocator, ENTRIES_PER_BRANCH, LEAVES_PER_BRANCH, STEMS_PER_BRANCH, branch::Branch,
    target::DEPTH,
};
#[derive(Clone)]
pub(crate) struct Cursor<'a, 'b> {
    allocator: &'b BranchAllocator<'a>,
    branch: usize,
    index: usize,
    depth: usize,
}
impl<'a, 'b> PartialEq for Cursor<'a, 'b> {
    fn eq(&self, other: &Self) -> bool {
        (self.allocator as *const BranchAllocator == other.allocator as *const BranchAllocator)
            & (self.branch == other.branch)
            & (self.index == other.index)
    }
}
impl<'a, 'b> Cursor<'a, 'b> {
    pub(crate) fn new(
        allocator: &'b BranchAllocator<'a>,
        branch: usize,
        index: usize,
        depth: usize,
    ) -> Option<Self> {
        if allocator.branches.get(branch).is_some()
            & (index < ENTRIES_PER_BRANCH)
            & (depth <= allocator.order)
        {
            Some(Self {
                allocator,
                branch,
                index,
                depth,
            })
        } else {
            None
        }
    }
    pub(crate) fn parent(&self) -> Option<Self> {
        if self.index == 0 {
            if self.branch == 0 {
                None
            } else {
                Some(Self {
                    allocator: self.allocator,
                    branch: (self.branch - 1) / (2 * LEAVES_PER_BRANCH),
                    index: STEMS_PER_BRANCH + ((self.branch - 1) % (2 * LEAVES_PER_BRANCH)) / 2,
                    depth: self.depth - 1,
                })
            }
        } else {
            Some(Self {
                allocator: self.allocator,
                branch: self.branch,
                index: (self.index - 1) / 2,
                depth: self.depth - 1,
            })
        }
    }
    fn outer(&self) -> Self {
        let mut current = self.clone();
        while current.depth % (DEPTH + 1) != 0 {
            current = current.parent().unwrap()
        }
        current
    }
    fn branch(&self) -> &Branch {
        self.allocator.branches.get(self.branch).unwrap()
    }
    fn lock_parents(&self) -> Option<Cursor<'a, 'b>> {
        let mut outer = self.outer();
        let mut last = outer.clone();
        while let Some(parent_cursor) = outer.parent() {
            let leaf_index = parent_cursor.index - STEMS_PER_BRANCH;
            let base_child = parent_cursor.branch * (2 * LEAVES_PER_BRANCH) + 1 + 2 * leaf_index;
            let is_lower = outer.branch == base_child;
            let parent_branch = &self.allocator.branches[parent_cursor.branch];
            'retry: loop {
                let (old, mut data) = parent_branch.load();
                if data.locked(parent_cursor.index).unwrap() {
                    return Some(last);
                }
                if is_lower {
                    data.uncoalesce_lower(parent_cursor.index);
                    data.lock_lower(parent_cursor.index);
                } else {
                    data.uncoalesce_upper(parent_cursor.index);
                    data.lock_upper(parent_cursor.index);
                }
                data.lock_parents(parent_cursor.index);
                match parent_branch.store(old, data) {
                    Ok(_) => break 'retry,
                    Err(_) => continue,
                }
            }
            last = parent_cursor.outer();
            outer = last.clone();
        }
        None
    }
    fn coalesce_to(&self, stop: Option<&Cursor<'a, 'b>>) {
        let mut cursor = self.outer();
        while let Some(parent) = cursor.parent() {
            if let Some(ref s) = stop {
                if cursor.branch == s.branch && cursor.index == s.index {
                    break;
                }
            }
            let leaf_index = parent.index - STEMS_PER_BRANCH;
            let base_child = parent.branch * (2 * LEAVES_PER_BRANCH) + 1 + 2 * leaf_index;
            let is_lower = cursor.branch == base_child;
            let parent_branch = &self.allocator.branches[parent.branch];
            loop {
                let (old, mut data) = parent_branch.load();
                if is_lower {
                    data.coalesce_lower(parent.index);
                } else {
                    data.coalesce_upper(parent.index);
                }
                match parent_branch.store(old, data) {
                    Ok(_) => break,
                    Err(_) => continue,
                }
            }
            cursor = parent.outer();
        }
    }
    fn uncoalesce_to(&self, stop: Option<&Cursor<'a, 'b>>) {
        let mut cursor = self.outer();
        while let Some(parent) = cursor.parent() {
            if let Some(ref s) = stop {
                if cursor.branch == s.branch && cursor.index == s.index {
                    break;
                }
            }
            let leaf_index = parent.index - STEMS_PER_BRANCH;
            let base_child = parent.branch * (2 * LEAVES_PER_BRANCH) + 1 + 2 * leaf_index;
            let is_lower = cursor.branch == base_child;
            let parent_branch = &self.allocator.branches[parent.branch];
            let mut exit = false;
            loop {
                let (old, mut data) = parent_branch.load();
                if is_lower {
                    if !data.lower_coalescing(parent.index).unwrap() {
                        return;
                    }
                    data.uncoalesce_lower(parent.index);
                    data.unlock_lower(parent.index);
                    if data.upper_locked(parent.index).unwrap() {
                        exit = true;
                    }
                } else {
                    if !data.upper_coalescing(parent.index).unwrap() {
                        return;
                    }
                    data.uncoalesce_upper(parent.index);
                    data.unlock_upper(parent.index);
                    if data.lower_locked(parent.index).unwrap() {
                        exit = true;
                    }
                }
                let mut current_slot = parent.index;
                while let Some(parent) = data.parent(current_slot) {
                    let lower = parent * 2 + 1;
                    let upper = parent * 2 + 2;
                    let sibling = if current_slot == lower { upper } else { lower };
                    if data.locked(sibling).unwrap() {
                        exit = true;
                        break;
                    }
                    data.unlock(parent);
                    current_slot = parent;
                }
                match parent_branch.store(old, data) {
                    Ok(_) => break,
                    Err(_) => continue,
                }
            }
            if exit {
                return;
            }
            cursor = parent.outer();
        }
    }
    fn unlock_branch(&self) -> bool {
        let branch = self.branch();
        'retry: loop {
            let (old, mut data) = branch.load();
            let mut exit = false;
            let mut current = self.index;
            'climb: while let Some(parent) = data.parent(current) {
                let lower = parent * 2 + 1;
                let upper = parent * 2 + 2;
                let sibling = if current == lower { upper } else { lower };
                if data.locked(sibling).unwrap() {
                    exit = true;
                    break 'climb;
                }
                data.unlock(parent);
                current = parent;
            }
            if self.depth < self.allocator.order {
                data.unlock_children(self.index);
            }
            data.unlock(self.index);
            match branch.store(old, data) {
                Ok(_) => return exit,
                Err(_) => continue 'retry,
            }
        }
    }
    fn is_allocated(&self) -> Option<bool> {
        let branch = self.branch();
        let (_, data) = branch.load();
        data.locked(self.index)
    }
    pub(crate) fn deallocate(&self, stop: Option<&Cursor<'a, 'b>>) -> Option<()> {
        if !self.is_allocated()? {
            return None;
        }
        self.coalesce_to(stop);
        let exit = self.unlock_branch();
        if !exit {
            self.uncoalesce_to(stop);
        }
        Some(())
    }
    pub(crate) fn allocate(&self) -> Option<()> {
        let branch = self.branch();
        'retry: loop {
            let (current, mut data) = branch.load();
            if !data.allocable(self.index).unwrap() {
                return None;
            }
            data.lock_parents(self.index);
            data.lock(self.index);
            if self.depth < self.allocator.order {
                data.lock_children(self.index);
            }
            match branch.store(current, data) {
                Ok(_) => break 'retry,
                Err(_) => continue 'retry,
            }
        }
        if self.branch == 0 {
            return Some(());
        }
        match self.lock_parents() {
            Some(last) => {
                self.deallocate(Some(&last))?;
                None
            }
            None => Some(()),
        }
    }
}
