//! Target-dependent definitions underneath the rest of the crate. This module is not aware of any
//! other module.
#[cfg(all(target_has_atomic = "8", not(target_has_atomic = "16")))]
mod this {
    pub type inner = u8;
    pub type Atomic = core::sync::atomic::AtomicU8;
    pub const DEPTH: usize = 0;
}
#[cfg(all(target_has_atomic = "16", not(target_has_atomic = "32")))]
mod this {
    pub type Inner = u16;
    pub type Atomic = core::sync::atomic::AtomicU16;
    pub const DEPTH: usize = 1;
}
#[cfg(all(target_has_atomic = "32", not(target_has_atomic = "64")))]
mod this {
    pub type Inner = u32;
    pub type Atomic = core::sync::atomic::AtomicU32;
    pub const DEPTH: usize = 2;
}
#[cfg(all(target_has_atomic = "64", not(target_has_atomic = "128")))]
mod this {
    pub type Inner = u64;
    pub type Atomic = core::sync::atomic::AtomicU64;
    pub const DEPTH: usize = 3;
}
#[cfg(target_has_atomic = "128")]
mod this {
    pub type Inner = u128;
    pub type Atomic = core::sync::atomic::AtomicU128;
    pub const DEPTH: usize = 4;
}
pub use this::{Atomic, Inner, DEPTH};