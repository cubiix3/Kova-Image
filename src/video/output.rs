use crate::error::Error;
use std::collections::VecDeque;
use windows::Win32::{Media::Audio::*, System::Com::*};

const RATE: usize = 48_000;
const CHANNELS: usize = 2;
const MAX_SAMPLES: usize = RATE * CHANNELS * 2;

fn fail(e: windows::core::Error) -> Error {
    Error::Io(format!("Video audio output failed: {e}"))
}

struct Chunk {
    start: i64,
    data: Vec<f32>,
    offset: usize,
}
#[derive(Default)]
struct Samples {
    chunks: VecDeque<Chunk>,
    count: usize,
}
impl Samples {
    fn push(&mut self, pts: f64, samples: &[f32]) -> Result<(), Error> {
        if !pts.is_finite() || pts.abs() > 604810.0 || !samples.len().is_multiple_of(CHANNELS) {
            return Err(Error::Corrupted("Invalid decoded audio timing".into()));
        }
        if samples.len() > MAX_SAMPLES.saturating_sub(self.count) || self.chunks.len() >= 256 {
            return Err(Error::Io(
                "Video audio buffering exceeds two seconds".into(),
            ));
        }
        let mut data = Vec::new();
        data.try_reserve_exact(samples.len())
            .map_err(|_| Error::MemoryBudget)?;
        data.extend_from_slice(samples);
        self.count += samples.len();
        self.chunks.push_back(Chunk {
            start: (pts * RATE as f64).round() as i64,
            data,
            offset: 0,
        });
        Ok(())
    }
    fn end(&self) -> i64 {
        self.chunks
            .iter()
            .map(|c| c.start + (c.data.len() / CHANNELS) as i64)
            .max()
            .unwrap_or(0)
    }
    fn discard_before(&mut self, cursor: i64) {
        while let Some(chunk) = self.chunks.front_mut() {
            let target = ((cursor - chunk.start).max(0) as usize).saturating_mul(CHANNELS);
            let offset = target.min(chunk.data.len()).max(chunk.offset);
            self.count -= offset - chunk.offset;
            chunk.offset = offset;
            if offset == chunk.data.len() {
                self.chunks.pop_front();
            } else {
                break;
            }
        }
    }
    fn render(&mut self, start: i64, output: &mut [f32]) {
        for (frame, samples) in output.chunks_exact_mut(CHANNELS).enumerate() {
            let cursor = start + frame as i64;
            samples.fill(0.0);
            self.discard_before(cursor);
            if let Some(chunk) = self.chunks.front_mut()
                && cursor >= chunk.start
            {
                samples.copy_from_slice(&chunk.data[chunk.offset..chunk.offset + CHANNELS]);
                chunk.offset += CHANNELS;
                self.count -= CHANNELS;
            }
        }
    }
}

pub(super) struct Output {
    client: IAudioClient,
    render: IAudioRenderClient,
    capacity: u32,
    queue: Samples,
    clock: IAudioClock,
    frequency: f64,
    volume: ISimpleAudioVolume,
    started: bool,
}

impl Output {
    pub fn new(volume: f64, muted: bool) -> Result<Self, Error> {
        // SAFETY: worker has a COM runtime; endpoint and render buffer stay on it.
        unsafe {
            let devices: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
            let device = devices
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .map_err(fail)?;
            let client: IAudioClient = device.Activate(CLSCTX_INPROC_SERVER, None).map_err(fail)?;
            let format = WAVEFORMATEX {
                wFormatTag: 3,
                nChannels: CHANNELS as u16,
                nSamplesPerSec: RATE as u32,
                nAvgBytesPerSec: (RATE * CHANNELS * 4) as u32,
                nBlockAlign: (CHANNELS * 4) as u16,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                    2_500_000,
                    0,
                    &format,
                    None,
                )
                .map_err(fail)?;
            let capacity = client.GetBufferSize().map_err(fail)?;
            if capacity == 0 || capacity as usize > RATE {
                return Err(Error::MemoryBudget);
            }
            let render = client.GetService().map_err(fail)?;
            let clock: IAudioClock = client.GetService().map_err(fail)?;
            let frequency = clock.GetFrequency().map_err(fail)? as f64;
            if frequency <= 0.0 {
                return Err(Error::Io("Invalid audio clock frequency".into()));
            }
            let session: ISimpleAudioVolume = client.GetService().map_err(fail)?;
            session
                .SetMasterVolume(volume as f32, std::ptr::null())
                .map_err(fail)?;
            session.SetMute(muted, std::ptr::null()).map_err(fail)?;
            Ok(Self {
                client,
                render,
                capacity,
                queue: Samples::default(),
                started: false,
                clock,
                frequency,
                volume: session,
            })
        }
    }

    pub fn push(&mut self, pts: f64, samples: &[f32]) -> Result<(), Error> {
        self.queue.push(pts, samples)
    }

    pub fn playhead(&self) -> Result<f64, Error> {
        let mut position = 0;
        // SAFETY: the worker-owned clock writes a single position value.
        unsafe {
            self.clock.GetPosition(&mut position, None).map_err(fail)?;
        }
        Ok(position as f64 / self.frequency)
    }

    pub fn controls(&mut self, paused: bool, volume: f64, muted: bool) -> Result<(), Error> {
        // SAFETY: synchronous calls on the worker-owned client.
        unsafe {
            self.volume
                .SetMasterVolume(volume as f32, std::ptr::null())
                .map_err(fail)?;
            self.volume.SetMute(muted, std::ptr::null()).map_err(fail)?;
            if paused && self.started {
                self.client.Stop().map_err(fail)?;
                self.started = false;
            } else if !paused && !self.started {
                self.client.Start().map_err(fail)?;
                self.started = true;
            }
        }
        Ok(())
    }

    pub fn pump(&mut self, paused: bool) -> Result<(), Error> {
        if paused {
            return Ok(());
        }
        // SAFETY: GetBuffer owns exactly count stereo f32 frames until ReleaseBuffer.
        unsafe {
            let padding = self.client.GetCurrentPadding().map_err(fail)?;
            let available = self
                .capacity
                .checked_sub(padding)
                .ok_or_else(|| Error::Io("Invalid video audio buffer size".into()))?;
            let cursor = (self.playhead()? * RATE as f64).round() as i64 + i64::from(padding);
            self.queue.discard_before(cursor);
            let count = (self.queue.end() - cursor).max(0).min(i64::from(available)) as u32;
            if count != 0 {
                let pointer = self.render.GetBuffer(count).map_err(fail)?;
                if pointer.is_null() {
                    let _ = self
                        .render
                        .ReleaseBuffer(count, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32);
                    return Err(Error::Io("No video audio buffer".into()));
                }
                let buffer = std::slice::from_raw_parts_mut(
                    pointer.cast::<f32>(),
                    count as usize * CHANNELS,
                );
                self.queue.render(cursor, buffer);
                self.render.ReleaseBuffer(count, 0).map_err(fail)?;
                if !self.started {
                    self.client.Start().map_err(fail)?;
                    self.started = true;
                }
            }
        }
        Ok(())
    }

    pub fn reset(&mut self, paused: bool) -> Result<(), Error> {
        // SAFETY: stopping precedes Reset and resource release.
        unsafe {
            self.client.Stop().map_err(fail)?;
            self.client.Reset().map_err(fail)?;
            if !paused {
                self.client.Start().map_err(fail)?;
            }
        }
        self.started = !paused;
        self.queue = Samples::default();
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        // SAFETY: client remains live until after Stop.
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timestamps_preserve_gaps_and_discard_late_samples() {
        let mut samples = Samples::default();
        samples
            .push(2.0 / RATE as f64, &[1.0, 1.0, 2.0, 2.0])
            .unwrap();
        let mut output = [9.0; 8];
        samples.render(0, &mut output);
        assert_eq!(output, [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
        samples.push(0.0, &[3.0, 3.0, 4.0, 4.0, 5.0, 5.0]).unwrap();
        samples.render(2, &mut output[..2]);
        assert_eq!(&output[..2], &[5.0, 5.0]);
        assert_eq!(samples.count, 0);
    }
    #[test]
    fn audio_queue_is_bounded() {
        let mut samples = Samples::default();
        samples.push(0.0, &vec![0.0; MAX_SAMPLES]).unwrap();
        assert!(samples.push(2.0, &[1.0, 1.0]).is_err());
    }
}
