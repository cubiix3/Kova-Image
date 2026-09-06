use crate::error::Error;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 32 * 1024 * 1024;
pub const MAX_DIMENSION: u32 = 32768;
pub const DECODE_BUDGET: u64 = 256 * 1024 * 1024;
pub const FRAME_BUDGET: usize = 128 * 1024 * 1024;
pub const CACHE_BUDGET: usize = 192 * 1024 * 1024;
pub const MAX_FRAMES: usize = 2000;

pub fn rgba_bytes(width: u32, height: u32) -> Result<usize, Error> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(Error::Dimensions)?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || pixels > MAX_PIXELS
    {
        return Err(Error::Dimensions);
    }
    usize::try_from(pixels.checked_mul(4).ok_or(Error::Dimensions)?).map_err(|_| Error::Dimensions)
}
pub fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(DECODE_BUDGET);
    limits
}

#[derive(Clone, Default)]
pub struct Generation(Arc<AtomicU64>);
impl Generation {
    pub fn next(&self) -> Ticket {
        let id = self.0.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        Ticket {
            generation: self.clone(),
            id,
        }
    }
}
#[derive(Clone)]
pub struct Ticket {
    generation: Generation,
    pub id: u64,
}
impl Ticket {
    pub fn is_current(&self) -> bool {
        self.generation.0.load(Ordering::Acquire) == self.id
    }
    pub fn check(&self) -> Result<(), Error> {
        if self.is_current() {
            Ok(())
        } else {
            Err(Error::Cancelled)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_overflow() {
        assert_eq!(rgba_bytes(100, 200), Ok(80000));
        for (w, h) in [(0, 1), (u32::MAX, u32::MAX), (10000, 10000), (32769, 1)] {
            assert_eq!(rgba_bytes(w, h), Err(Error::Dimensions));
        }
    }
    #[test]
    fn stale_requests_never_win() {
        let g = Generation::default();
        let old = g.next();
        let latest = g.next();
        assert_eq!(old.check(), Err(Error::Cancelled));
        assert!(latest.is_current());
    }
}
