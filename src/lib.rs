//! Open Scanline clean-room scanner library.
//!
//! Binary entry: `open-scanline` (see `main.rs` / `cli`).

pub mod batch;
pub mod cli;
mod composition;
pub mod config;
pub mod core;
pub mod device;
mod domain;
mod error;
pub mod escl;
pub mod export;
pub mod features;
pub mod film;
pub mod gui;
pub mod i18n;
pub mod icc;
pub mod imaging;
mod inbound;
mod infrastructure;
pub mod manufacturers;
pub mod ml;
pub mod ocr;
pub mod packaging;
pub mod pipeline;
pub mod platform;
pub mod plugin;
pub mod process;
pub mod sane;
pub mod scan;
pub mod twain;
pub mod wia;
mod workflows;

pub use export::{ExportOptions, OcrEngine};

pub const VERSION: &str = "1.0.0";
pub const APP_NAME: &str = "open-scanline";
