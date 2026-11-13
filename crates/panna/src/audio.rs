//! Recording spoken comments from the system's default microphone, and playing them through its
//! default speakers. Each runs on a thread of its own, which holds the audio stream until it's
//! stopped or finished.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use panna_core::{MAX_SOUND_SECONDS, Sound};

/// How long to wait for the system to start a stream.
const START: Duration = Duration::from_secs(5);

/// A recording in progress.
#[derive(Debug)]
pub struct Recorder {
    stop: mpsc::Sender<()>,
    done: mpsc::Receiver<Result<Sound, String>>,
    started: Instant,
}

impl Recorder {
    /// Starts recording from the default microphone.
    pub fn start() -> Result<Self, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let recorded = Arc::new(Mutex::new(Vec::<i16>::new()));
            let stream = input(Arc::clone(&recorded));
            let (stream, rate, channels) = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            let started = Instant::now();
            // Until stopped, or the longest a recording may be.
            let _ = stop_rx.recv_timeout(Duration::from_secs(u64::from(MAX_SOUND_SECONDS)));
            drop(stream);
            let mut samples = std::mem::take(&mut *recorded.lock().unwrap_or_else(|e| e.into_inner()));
            // No longer than the time it took, should a device hand over sound faster than
            // it's spoken.
            let most = (started.elapsed().as_secs_f64() + 0.5) * f64::from(rate) * f64::from(channels);
            samples.truncate(most as usize);
            let _ = done_tx.send(Ok(mono(&samples, channels, rate)));
        });
        match ready_rx.recv_timeout(START) {
            Ok(Ok(())) => Ok(Self {
                stop: stop_tx,
                done: done_rx,
                started: Instant::now(),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("The microphone didn't start.".into()),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Stops recording, and gives what was recorded.
    pub fn finish(self) -> Result<Sound, String> {
        let _ = self.stop.send(());
        self.done
            .recv_timeout(START)
            .unwrap_or_else(|_| Err("The recording couldn't be finished.".into()))
    }
}

/// The channels mixed into one.
fn mono(samples: &[i16], channels: u16, rate: u32) -> Sound {
    let channels = usize::from(channels.max(1));
    Sound {
        rate,
        channels: 1,
        samples: samples
            .chunks(channels)
            .map(|frame| {
                (frame.iter().map(|&s| i32::from(s)).sum::<i32>() / frame.len().max(1) as i32) as i16
            })
            .collect(),
    }
}

type Stream = (cpal::Stream, u32, u16);

/// A stream from the default microphone, adding to `recorded`.
fn input(recorded: Arc<Mutex<Vec<i16>>>) -> Result<Stream, String> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or("No microphone was found.")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("The microphone can't be used: {e}"))?;
    let config = supported.config();
    let (rate, channels) = (config.sample_rate, config.channels);
    let most = rate as usize * usize::from(channels) * MAX_SOUND_SECONDS as usize;
    fn build<T: SizedSample>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        recorded: Arc<Mutex<Vec<i16>>>,
        most: usize,
    ) -> Result<cpal::Stream, cpal::Error>
    where
        i16: FromSample<T>,
    {
        device.build_input_stream(
            config,
            move |data: &[T], _: &_| {
                if let Ok(mut r) = recorded.lock()
                    && r.len() < most
                {
                    r.extend(data.iter().map(|&v| i16::from_sample(v)));
                }
            },
            |_| {},
            Some(START),
        )
    }
    let stream = match supported.sample_format() {
        SampleFormat::I8 => build::<i8>(&device, config, recorded, most),
        SampleFormat::I16 => build::<i16>(&device, config, recorded, most),
        SampleFormat::I32 => build::<i32>(&device, config, recorded, most),
        SampleFormat::U8 => build::<u8>(&device, config, recorded, most),
        SampleFormat::U16 => build::<u16>(&device, config, recorded, most),
        SampleFormat::U32 => build::<u32>(&device, config, recorded, most),
        SampleFormat::F32 => build::<f32>(&device, config, recorded, most),
        SampleFormat::F64 => build::<f64>(&device, config, recorded, most),
        other => {
            return Err(format!(
                "The microphone's sound format ({other}) isn't supported."
            ));
        }
    }
    .map_err(|e| format!("The microphone can't be used: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("The microphone didn't start: {e}"))?;
    Ok((stream, rate, channels))
}

/// A sound playing.
#[derive(Debug)]
pub struct Player {
    stop: mpsc::Sender<()>,
    finished: Arc<AtomicBool>,
    started: Instant,
    pub seconds: f32,
    /// What's playing, so pressing it again stops it.
    pub what: u64,
}

impl Player {
    /// Plays `sound` through the default speakers.
    pub fn start(sound: Sound, what: u64) -> Result<Self, String> {
        let seconds = sound.seconds();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let finished = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&finished);
        std::thread::spawn(move || {
            let stream = match output(sound, Arc::clone(&done)) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    done.store(true, Ordering::Relaxed);
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            while !done.load(Ordering::Relaxed) {
                match stop_rx.recv_timeout(Duration::from_millis(100)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
            // Let the last of it out of the speakers' buffer.
            std::thread::sleep(Duration::from_millis(150));
            drop(stream);
            done.store(true, Ordering::Relaxed);
        });
        match ready_rx.recv_timeout(START) {
            Ok(Ok(())) => Ok(Self {
                stop: stop_tx,
                finished,
                started: Instant::now(),
                seconds,
                what,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("The speakers didn't start.".into()),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::Relaxed)
    }

    pub fn stop(&self) {
        let _ = self.stop.send(());
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A stream to the default speakers playing `sound`, setting `done` at its end.
fn output(sound: Sound, done: Arc<AtomicBool>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No speakers were found.")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("The speakers can't be used: {e}"))?;
    let config = supported.config();
    let source = Arc::new(sound);
    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: cpal::StreamConfig,
        sound: Arc<Sound>,
        done: Arc<AtomicBool>,
    ) -> Result<cpal::Stream, cpal::Error> {
        let out_channels = usize::from(config.channels.max(1));
        let in_channels = usize::from(sound.channels.max(1));
        // Source frames for each frame played.
        let step = f64::from(sound.rate) / f64::from(config.sample_rate.max(1));
        let frames = sound.samples.len() / in_channels;
        let played = AtomicUsize::new(0);
        device.build_output_stream(
            config,
            move |data: &mut [T], _: &_| {
                let start = played.load(Ordering::Relaxed);
                for (i, frame) in data.chunks_mut(out_channels).enumerate() {
                    let at = ((start + i) as f64 * step) as usize;
                    for (c, out) in frame.iter_mut().enumerate() {
                        let v = if at < frames {
                            let channel = c.min(in_channels - 1);
                            sound
                                .samples
                                .get(at * in_channels + channel)
                                .map_or(0.0, |&s| f32::from(s) / 32768.0)
                        } else {
                            0.0
                        };
                        *out = T::from_sample(v);
                    }
                }
                let now = start + data.len() / out_channels;
                played.store(now, Ordering::Relaxed);
                if (now as f64 * step) as usize >= frames {
                    done.store(true, Ordering::Relaxed);
                }
            },
            |_| {},
            Some(START),
        )
    }
    let stream = match supported.sample_format() {
        SampleFormat::I8 => build::<i8>(&device, config, source, done),
        SampleFormat::I16 => build::<i16>(&device, config, source, done),
        SampleFormat::I32 => build::<i32>(&device, config, source, done),
        SampleFormat::U8 => build::<u8>(&device, config, source, done),
        SampleFormat::U16 => build::<u16>(&device, config, source, done),
        SampleFormat::U32 => build::<u32>(&device, config, source, done),
        SampleFormat::F32 => build::<f32>(&device, config, source, done),
        SampleFormat::F64 => build::<f64>(&device, config, source, done),
        other => return Err(format!("The speakers' sound format ({other}) isn't supported.")),
    }
    .map_err(|e| format!("The speakers can't be used: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("The speakers didn't start: {e}"))?;
    Ok(stream)
}

/// A tick four times a second, while recording or playing, to show the time.
pub fn ticks() -> futures_channel::mpsc::Receiver<()> {
    let (mut tx, rx) = futures_channel::mpsc::channel(1);
    std::thread::spawn(move || {
        while !tx.is_closed() {
            std::thread::sleep(Duration::from_millis(250));
            let _ = tx.try_send(());
        }
    });
    rx
}

/// A time as minutes and seconds, like 1:05.
pub fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_is_mixed_to_one_channel() {
        let sound = mono(&[100, 300, -50, 50], 2, 44_100);
        assert_eq!(sound.samples, [200, 0]);
        assert_eq!((sound.rate, sound.channels), (44_100, 1));
        assert_eq!(clock(65.4), "1:05");
    }
}
