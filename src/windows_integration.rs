//! Windows shell integration. Native playback is isolated in `video`.
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::*;
#[cfg(windows)]
mod associations;
#[cfg(windows)]
pub use associations::*;
