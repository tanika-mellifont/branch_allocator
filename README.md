# Branch Allocator

## Usage
The allocator manages a region of page frames, virtual memory space, or equivalent, using atomic
operations and compare-and-swap retry loops to allow allocation and deallocation using only
shared references without internal locking. It does not depend on std, and uses alloc only for
tests.

The allocator's performance is architecture-dependent, but will automatically select the largest
word size that the target can compare-and-swap atomically.

## Algorithm

Safe, concurrent allocation is achieved by attempting allocation and then undoing work if a
conflict is found, repeating until a subsection of the tree is allocated atomically. This wastes
work, but itself is no worse than spinning, except with the new advantage of allowing a core to
safely preempt itself from an interrupt context without deadlocking.

The algorithm is heavily inspired by Andrea Scarselli's bunch allocator, detailed in their
thesis "A Lock-Free Buddy System for Scalable Memory Allocation".

## License

Licensed under either of:
- Apache License, Version 2.0 (LICENSE-APACHE or http://apache.org/licenses/LICENSE-2.0)
- MIT license (LICENSE-MIT or http://opensource.org/licenses/MIT) at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.