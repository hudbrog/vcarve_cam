//! Application-owned geometry contracts for the Flat V-carve CAM engine.
pub mod checks;
pub mod contours;
pub mod geometry;
pub mod job;
pub mod model;
pub mod motion;
pub mod operations;
mod plan_hash;
pub mod pocket;
pub mod post;
pub mod preview;
pub mod project;
mod routing;
pub mod sequence;
pub mod setup;
pub mod spike;
pub mod stock;
pub mod svg;
pub mod target;
mod timing;
pub mod tool_library;
pub mod toolpath;

pub mod vcarve;
pub mod verification;
