//! Concrete acquisition registry and scanner adapters.

pub mod escl;
pub mod file;
pub mod mock;
mod registry;
pub mod sane;
pub mod wia;

pub use registry::*;
