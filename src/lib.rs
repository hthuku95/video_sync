// lib.rs - Main library file that exports all modules
pub mod types;
pub mod core;
pub mod audio;
pub mod visual;
pub mod transform;
pub mod advanced;
pub mod export;
pub mod utils;

// Re-export commonly used types for convenience
pub use types::*;
pub use core::*;
pub use audio::*;
pub use visual::*;
pub use transform::*;
pub use advanced::*;
pub use export::*;
pub use utils::*;