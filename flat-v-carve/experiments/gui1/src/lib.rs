pub mod app;
pub mod compute;
pub mod platform;
pub mod render;
pub mod state;

#[cfg(target_arch = "wasm32")]
mod web;
