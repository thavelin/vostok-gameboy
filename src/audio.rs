//! Sound: a lock-free ring buffer between the emulator thread and the sound
//! card, and the output device itself.

use rodio::{OutputStream, Sink, Source};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

pub const AUDIO_SAMPLE_RATE: u32 = crate::machine::SAMPLE_RATE;

/// Single-producer, single-consumer ring of interleaved stereo samples.
pub struct AudioBuffer {
    samples: Box<[AtomicU32]>,
    read_pos: AtomicUsize,
    write_pos: AtomicUsize,
    mask: usize,
    last_sample: AtomicU32,
}

impl AudioBuffer {
    /// Capacity is rounded up to a power of two.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2).next_power_of_two();
        Self {
            samples: (0..capacity).map(|_| AtomicU32::new(0)).collect(),
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
            mask: capacity - 1,
            last_sample: AtomicU32::new(0),
        }
    }

    pub fn capacity(&self) -> usize {
        self.mask + 1
    }

    /// Samples waiting to be played.
    pub fn available(&self) -> usize {
        let read = self.read_pos.load(Ordering::Acquire);
        let write = self.write_pos.load(Ordering::Acquire);
        write.wrapping_sub(read)
    }

    /// Adds a block from the emulator thread. A block that does not fit is
    /// dropped whole: when the emulator runs ahead of the sound card, a skip
    /// is better than overwriting sound that has not played yet.
    pub fn push_samples(&self, samples: &[f32]) -> bool {
        if self.available() + samples.len() > self.capacity() {
            return false;
        }
        let mut write = self.write_pos.load(Ordering::Relaxed);
        for &sample in samples {
            self.samples[write & self.mask].store(sample.to_bits(), Ordering::Relaxed);
            write = write.wrapping_add(1);
        }
        self.write_pos.store(write, Ordering::Release);
        true
    }

    /// Next sample for the sound card. When the buffer runs dry the last
    /// sample fades out instead of cutting to silence, which would click.
    #[inline]
    pub fn pop_sample(&self) -> f32 {
        let read = self.read_pos.load(Ordering::Relaxed);
        let write = self.write_pos.load(Ordering::Acquire);
        if read == write {
            let faded = f32::from_bits(self.last_sample.load(Ordering::Relaxed)) * 0.99;
            self.last_sample.store(faded.to_bits(), Ordering::Relaxed);
            return faded;
        }
        let bits = self.samples[read & self.mask].load(Ordering::Relaxed);
        self.read_pos.store(read.wrapping_add(1), Ordering::Release);
        self.last_sample.store(bits, Ordering::Relaxed);
        f32::from_bits(bits)
    }

    /// Throws away whatever is waiting (after a pause or a speed change).
    pub fn clear(&self) {
        self.read_pos
            .store(self.write_pos.load(Ordering::Acquire), Ordering::Release);
    }
}

/// Endless stereo source that plays whatever is in the buffer.
pub struct GameboyAudioSource {
    buffer: Arc<AudioBuffer>,
}

impl GameboyAudioSource {
    pub fn new(buffer: Arc<AudioBuffer>) -> Self {
        Self { buffer }
    }
}

impl Iterator for GameboyAudioSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<f32> {
        Some(self.buffer.pop_sample())
    }
}

impl Source for GameboyAudioSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        AUDIO_SAMPLE_RATE
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// The open sound device. Keep it alive for as long as sound should play, on
/// the thread that opened it.
pub struct AudioOut {
    _stream: OutputStream,
    sink: Sink,
    buffer: Arc<AudioBuffer>,
}

impl AudioOut {
    /// Opens the default output. Fails with a readable reason when there is
    /// no sound device; the game can still run silent.
    pub fn open(volume: f32) -> Result<Self, String> {
        let (stream, handle) =
            OutputStream::try_default().map_err(|error| format!("no sound output: {error}"))?;
        let sink = Sink::try_new(&handle).map_err(|error| format!("no sound output: {error}"))?;
        // ~190 ms of stereo: enough slack for a late frame, short enough to stay in sync.
        let buffer = Arc::new(AudioBuffer::new(16_384));
        sink.set_volume(volume.clamp(0.0, 1.0));
        sink.append(GameboyAudioSource::new(Arc::clone(&buffer)));
        Ok(Self {
            _stream: stream,
            sink,
            buffer,
        })
    }

    pub fn push(&self, samples: &[f32]) {
        self.buffer.push_samples(samples);
    }

    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn clear(&self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_come_out_in_order_then_fade() {
        let buffer = AudioBuffer::new(8);
        assert!(buffer.push_samples(&[0.1, 0.2, 0.3]));
        assert_eq!(buffer.available(), 3);
        assert_eq!(buffer.pop_sample(), 0.1);
        assert_eq!(buffer.pop_sample(), 0.2);
        assert_eq!(buffer.pop_sample(), 0.3);
        let faded = buffer.pop_sample();
        assert!(faded > 0.0 && faded < 0.3);
    }

    #[test]
    fn a_block_that_does_not_fit_is_dropped_whole() {
        let buffer = AudioBuffer::new(4);
        assert!(buffer.push_samples(&[1.0, 2.0, 3.0]));
        assert!(!buffer.push_samples(&[4.0, 5.0]));
        assert_eq!(buffer.available(), 3);
        assert_eq!(buffer.pop_sample(), 1.0);
        assert!(buffer.push_samples(&[4.0, 5.0]));
        // Wraps around the end of the ring.
        let rest: Vec<f32> = (0..4).map(|_| buffer.pop_sample()).collect();
        assert_eq!(rest, [2.0, 3.0, 4.0, 5.0]);
        buffer.push_samples(&[9.0]);
        buffer.clear();
        assert_eq!(buffer.available(), 0);
    }
}
