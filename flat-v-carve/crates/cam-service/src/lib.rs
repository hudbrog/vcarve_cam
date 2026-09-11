//! Pure service DTOs and projections shared by the local HTTP service and the
//! WebAssembly build. No filesystem, async runtime, or UI access lives here.
pub mod admission;
pub mod collection;
pub mod document;
pub mod export;
pub mod inspection;
pub mod sequence;
pub mod summary;
pub mod task;
pub mod verification;
