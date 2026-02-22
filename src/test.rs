//! Testing for the entire crate. This module is aware of only the crate root module and the target
//! module, and uses only the public interface for testing.
use crate::{BranchAllocator, target::Atomic};
fn create(order: usize) -> BranchAllocator<'static> {
    let required = BranchAllocator::required(order);
    let storage: &'static mut [Atomic] = Vec::leak(Vec::from_fn(required, |_| Atomic::new(0)));
    BranchAllocator::new(storage, order).expect("failed to create allocator!")
}

fn allocate_ok(alloc: &BranchAllocator, index: usize, order: usize) {
    assert!(
        alloc.try_allocate(index, order).is_some(),
        "failed to allocate {}-ordered region containing index {}",
        order,
        index
    );
}

fn allocate_fails(alloc: &BranchAllocator, index: usize, order: usize) {
    assert!(
        alloc.try_allocate(index, order).is_none(),
        "unexpectedly succeeded to allocate {}-ordered region containing index {}",
        order,
        index
    );
}

fn deallocate_ok(alloc: &BranchAllocator, index: usize, order: usize) {
    assert!(
        alloc.deallocate(index, order).is_some(),
        "failed to deallocate {}-ordered region containing index {}",
        order,
        index
    );
}

fn deallocate_fails(alloc: &BranchAllocator, index: usize, order: usize) {
    assert!(
        alloc.deallocate(index, order).is_none(),
        "unexpectedly succeeded to deallocate {}-ordered region containing index {}",
        order,
        index
    );
}

#[test]
fn create_allocator() {
    let order = 10;
    let alloc = create(order);
    assert_eq!(alloc.order, order);
}

#[test]
fn allocate_and_free_single_frame() {
    let alloc = create(4); // 16 frames
    let frame = 5;
    allocate_ok(&alloc, frame, 0);
    allocate_fails(&alloc, frame, 0);
    deallocate_ok(&alloc, frame, 0);
    allocate_ok(&alloc, frame, 0);
}

#[test]
fn allocate_stem_node() {
    let alloc = create(4); // 16 frames
    let frame = 0; // block of order 3 covering frames 0..7
    allocate_ok(&alloc, frame, 3);
    let leaf_inside = 3;
    allocate_fails(&alloc, leaf_inside, 0);
    deallocate_ok(&alloc, frame, 3);
    allocate_ok(&alloc, leaf_inside, 0);
}

#[test]
fn range_and_order_mismatch() {
    let alloc = create(4);
    // order too large
    allocate_fails(&alloc, 0, 5);
    // order 0 on any valid frame should work
    allocate_ok(&alloc, 0, 0);
}

#[test]
fn out_of_bounds_index() {
    let alloc = create(4); // frames 0..15
    allocate_fails(&alloc, 16, 0);
    deallocate_fails(&alloc, 16, 0);
}

#[test]
fn multiple_allocations_and_frees() {
    let alloc = create(5); // 32 frames
    let frames: Vec<usize> = (0..32).collect();

    for &f in &frames {
        allocate_ok(&alloc, f, 0);
    }
    for &f in &frames {
        allocate_fails(&alloc, f, 0);
    }
    for &f in frames.iter().step_by(2) {
        deallocate_ok(&alloc, f, 0);
    }
    for &f in frames.iter().step_by(2) {
        allocate_ok(&alloc, f, 0);
    }
}

#[test]
fn coalescing_across_branches() {
    let alloc = create(5); // 32 frames
    let left = 0; // block of order 3 covering frames 0..7
    let right = 8; // block of order 3 covering frames 8..15
    let parent = 0; // block of order 4 covering frames 0..15

    allocate_ok(&alloc, left, 3);
    allocate_ok(&alloc, right, 3);
    deallocate_ok(&alloc, left, 3);
    deallocate_ok(&alloc, right, 3);
    allocate_ok(&alloc, parent, 4);
    allocate_fails(&alloc, left, 3);
    allocate_fails(&alloc, right, 3);
}

#[test]
fn concurrent_allocation() {
    use std::thread;
    let alloc = create(8); // 256 frames
    let frames: Vec<usize> = (0..256).collect();
    let alloc_ref = &alloc;
    thread::scope(|s| {
        for i in 0..8 {
            let frame = frames[i * 32];
            s.spawn(move || {
                for _ in 0..100 {
                    allocate_ok(alloc_ref, frame, 0);
                    deallocate_ok(alloc_ref, frame, 0);
                }
            });
        }
    });
}

#[test]
fn out_of_range_indices() {
    let alloc = create(4);
    allocate_fails(&alloc, 16, 0);
    deallocate_fails(&alloc, 16, 0);
    allocate_fails(&alloc, 1_000_000, 0);
    deallocate_fails(&alloc, 1_000_000, 0);
}

#[test]
fn invalid_orders() {
    let alloc = create(4);
    allocate_fails(&alloc, 0, 5);
    deallocate_fails(&alloc, 0, 5);
    allocate_fails(&alloc, 0, 10);
    allocate_ok(&alloc, 0, 0);
}

#[test]
fn double_allocation() {
    let alloc = create(4);
    let frame = 0;
    allocate_ok(&alloc, frame, 0);
    allocate_fails(&alloc, frame, 0);
}

#[test]
fn double_deallocation() {
    let alloc = create(4);
    let frame = 0;
    deallocate_fails(&alloc, frame, 0);
    allocate_ok(&alloc, frame, 0);
    deallocate_ok(&alloc, frame, 0);
    deallocate_fails(&alloc, frame, 0);
}

#[test]
fn allocate_with_index_not_matching_order() {
    let alloc = create(4);
    allocate_ok(&alloc, 0, 2); // block frames 0..3
    allocate_fails(&alloc, 0, 2); // same block already allocated
    allocate_fails(&alloc, 1, 2); // still inside that block
}

#[test]
fn deallocate_with_wrong_order() {
    let alloc = create(4);
    let frame = 0;
    allocate_ok(&alloc, frame, 0);
    deallocate_fails(&alloc, frame, 1);
    deallocate_ok(&alloc, frame, 0);
}

#[test]
fn allocate_after_partial_free() {
    let alloc = create(5);
    let stem_frame = 0;
    allocate_ok(&alloc, stem_frame, 3);
    let leaf_inside = 3;
    deallocate_fails(&alloc, leaf_inside, 0);
    deallocate_ok(&alloc, stem_frame, 3);
    allocate_ok(&alloc, leaf_inside, 0);
}

#[test]
fn concurrent_mixed_operations() {
    use std::thread;
    let alloc = create(8);
    let alloc_ref = &alloc;
    let frames: Vec<usize> = (0..256).collect();
    thread::scope(|s| {
        for i in 0..8 {
            let frame = frames[i * 32];
            s.spawn(move || {
                for _ in 0..50 {
                    // valid ops
                    let _ = alloc_ref.try_allocate(frame, 0);
                    let _ = alloc_ref.deallocate(frame, 0);
                    // invalid index
                    let _ = alloc_ref.try_allocate(9999, 0);
                    let _ = alloc_ref.deallocate(9999, 0);
                    // invalid order
                    let _ = alloc_ref.try_allocate(frame, 10);
                    let _ = alloc_ref.deallocate(frame, 10);
                }
            });
        }
    });
}

#[test]
fn allocate_all_frames_then_free_all() {
    let alloc = create(6); // 64 frames
    for f in 0..64 {
        allocate_ok(&alloc, f, 0);
    }
    for f in 0..64 {
        deallocate_ok(&alloc, f, 0);
    }
    allocate_ok(&alloc, 0, 6);
    deallocate_ok(&alloc, 0, 6);
}

#[test]
fn allocate_large_blocks_interleaved() {
    let alloc = create(6); // 64 frames
    for &base in &[0, 16, 32, 48] {
        allocate_ok(&alloc, base, 4);
    }
    allocate_fails(&alloc, 0, 5);
    deallocate_ok(&alloc, 0, 4);
    deallocate_ok(&alloc, 16, 4);
    allocate_ok(&alloc, 0, 5);
}

#[test]
fn allocate_max_order() {
    let alloc = create(10); // 1024 frames
    allocate_ok(&alloc, 0, 10);
    allocate_fails(&alloc, 500, 0);
    deallocate_ok(&alloc, 0, 10);
    allocate_ok(&alloc, 500, 0);
}

#[test]
fn stress_random_ops() {
    use std::thread;
    let alloc = create(8);
    let frames: Vec<usize> = (0..256).collect();
    let frames_ref = &frames;
    let orders = [0, 1, 2, 3, 4, 5, 6, 7, 8];
    // Simple xorshift RNG with a fixed seed.
    fn xorshift(state: &mut u32) -> u32 {
        let mut x = *state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *state = x;
        x
    }

    thread::scope(|s| {
        for _ in 0..4 {
            let alloc_ref = &alloc;
            s.spawn(move || {
                let mut rng = 123456789; // fixed seed per thread
                for _ in 0..100 {
                    let f_idx = (xorshift(&mut rng) as usize) % frames_ref.len();
                    let f = frames_ref[f_idx];
                    let o_idx = (xorshift(&mut rng) as usize) % orders.len();
                    let o = orders[o_idx];
                    if xorshift(&mut rng) % 2 == 0 {
                        let _ = alloc_ref.try_allocate(f, o);
                    } else {
                        let _ = alloc_ref.deallocate(f, o);
                    }
                }
            });
        }
    });
}