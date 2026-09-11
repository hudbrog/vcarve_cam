pub mod alloc_probe;
pub mod app;
pub mod compute;
#[cfg(not(target_arch = "wasm32"))]
pub mod file_io;
pub mod overlay;
pub mod pages;
pub mod paging;
pub mod perf;
pub mod pick;
pub mod platform;
pub mod recovery;
pub mod render;
pub mod sim;
#[cfg(not(target_arch = "wasm32"))]
pub mod sim_probe;
pub mod sim_setup;
pub mod state;
pub mod stock_preview;
pub mod stock_render;

/// Whole-process Rust heap accounting for the measurement report. The counter
/// is observational only: nothing in the application branches on it.
#[global_allocator]
static ALLOCATOR: crate::alloc_probe::Counting = crate::alloc_probe::Counting;

#[cfg(target_arch = "wasm32")]
mod web;
