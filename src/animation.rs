use std::time::Duration;

/// Total presentations, not additional repetitions. None means forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loops(pub Option<u32>);

/// Frames that ask for 10 ms or less are shown for 100 ms, as browsers do.
/// Encoders write 0 or 1 centisecond for "as fast as possible", and playing
/// those literally runs the animation about ten times too fast.
pub fn frame_delay(numerator_ms: u32, denominator: u32) -> Duration {
    let micros = u64::from(numerator_ms).saturating_mul(1000) / u64::from(denominator.max(1));
    if micros <= 10_000 {
        return Duration::from_millis(100);
    }
    Duration::from_micros(micros.min(60_000_000))
}

#[derive(Default, Debug)]
pub struct Playback {
    pub frame: usize,
    completed: u32,
    pub finished: bool,
}
impl Playback {
    pub fn advance(&mut self, frames: usize, loops: Loops) -> bool {
        if frames < 2 || self.finished {
            return false;
        }
        if self.frame + 1 < frames {
            self.frame += 1;
            return true;
        }
        self.completed = self.completed.saturating_add(1);
        if loops.0.is_some_and(|total| self.completed >= total) {
            self.finished = true;
            return false;
        }
        self.frame = 0;
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_is_bounded_and_variable() {
        assert_eq!(frame_delay(0, 0), Duration::from_millis(100));
        assert_eq!(frame_delay(10, 1), Duration::from_millis(100));
        assert_eq!(frame_delay(20, 1), Duration::from_millis(20));
        assert_eq!(frame_delay(125, 2), Duration::from_micros(62500));
        assert_eq!(frame_delay(u32::MAX, 1), Duration::from_secs(60));
    }
    #[test]
    fn finite_and_infinite_loops() {
        let mut p = Playback::default();
        assert!(p.advance(2, Loops(Some(2))));
        assert!(p.advance(2, Loops(Some(2))));
        assert!(p.advance(2, Loops(Some(2))));
        assert!(!p.advance(2, Loops(Some(2))));
        assert_eq!(p.frame, 1);
        let mut p = Playback::default();
        for _ in 0..100 {
            assert!(p.advance(2, Loops(None)));
        }
        assert!(!p.advance(1, Loops(None)));
    }
}
