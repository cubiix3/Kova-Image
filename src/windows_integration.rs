//! The only application module that calls unsafe Windows APIs.
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::*;
