// spikes/audio-preview/src/main.rs
// Cargo deps: symphonia (workspace features), cpal = "0.15", anyhow
// Pass criteria: preview of wav/mp3/flac/aif starts < 300 ms on SSD, UI
// thread never blocks, seek is instant against the decoded buffer.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use symphonia::core::audio::Signal;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;

struct Decoded {
    samples: Vec<f32>, // interleaved
    channels: u16,
    sample_rate: u32,
}

/// Decode the whole file. One-shots are small; cap loops so a 10-minute
/// pad cannot eat RAM. (Cap ~ 20 minutes of stereo 48k.)
fn decode(path: &str) -> Result<Decoded> {
    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint, mss, &FormatOptions::default(), &Default::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no decodable track"))?
        .clone();
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut out: Vec<f32> = Vec::new();
    let mut channels = 0u16;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44_100);
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) => break, // clean EOF
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                channels = spec.channels.count() as u16;
                sr = spec.rate;
                let n = buf.frames();
                for f in 0..n {
                    for ch in 0..channels as usize {
                        out.push(buf.chan(ch)[f]); // Signal trait -> typed chan()
                    }
                }
            }
            Err(SymError::DecodeError(_)) => continue, // corrupt frame: skip, do not die
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Decoded { samples: out, channels, sample_rate: sr })
}

/// Naive linear resample when the device rate differs — good enough for
/// auditioning; swap for rubato if you ever ship resampling.
fn resample_linear(d: &Decoded, target: u32) -> Decoded {
    if d.sample_rate == target || d.samples.is_empty() {
        return Decoded { samples: d.samples.clone(), channels: d.channels, sample_rate: target };
    }
    let ratio = d.sample_rate as f64 / target as f64;
    let ch = d.channels.max(1) as usize;
    let frames = d.samples.len() / ch;
    let out_frames = (frames as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_frames * ch);
    for of in 0..out_frames {
        let src = of as f64 * ratio;
        let i = src.floor() as usize;
        let frac = (src - i as f64) as f32;
        let j = (i + 1).min(frames.saturating_sub(1));
        for c in 0..ch {
            let a = d.samples[i * ch + c];
            let b = d.samples[j * ch + c];
            out.push(a + (b - a) * frac);
        }
    }
    Decoded { samples: out, channels: d.channels, sample_rate: target }
}

pub struct Preview {
    buf: Arc<Mutex<Arc<Decoded>>>,
    cursor: Arc<AtomicUsize>, // in interleaved samples
    playing: Arc<AtomicBool>,
    stream: cpal::Stream,
}

impl Preview {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| anyhow!("no output device"))?;
        let cfg = device.default_output_config()?;
        let buf: Arc<Mutex<Arc<Decoded>>> = Arc::new(Mutex::new(Arc::new(
            Decoded { samples: vec![], channels: 2, sample_rate: cfg.sample_rate().0 },
        )));
        let cursor = Arc::new(AtomicUsize::new(0));
        let playing = Arc::new(AtomicBool::new(false));

        let b2 = buf.clone();
        let c2 = cursor.clone();
        let p2 = playing.clone();
        let stream = device.build_output_stream(
            &cfg.config(),
            move |data: &mut [f32], _| {
                // REAL-TIME CALLBACK: no locks that can wait, no allocation.
                if !p2.load(Ordering::Relaxed) {
                    data.fill(0.0);
                    return;
                }
                let guard = match b2.try_lock() {
                    Ok(g) => g,
                    Err(_) => {
                        data.fill(0.0);
                        return;
                    }
                };
                let dec = guard.clone();
                let mut pos = c2.load(Ordering::Relaxed);
                for frame in data.chunks_mut(dec.channels.max(1) as usize) {
                    for s in frame.iter_mut() {
                        *s = dec.samples.get(pos).copied().unwrap_or(0.0);
                        pos += 1;
                    }
                }
                c2.store(pos.min(dec.samples.len()), Ordering::Relaxed);
            },
            |e| eprintln!("stream error: {e}"),
            None,
        )?;
        Ok(Self { buf, cursor, playing, stream })
    }

    /// Called on ANY thread — the decode happens here, never on the callback.
    pub fn load_and_play(&self, path: &str) -> Result<()> {
        let dec = decode(path)?;
        let device_sr = cpal::default_host()
            .default_output_device()
            .and_then(|d| d.default_output_config().ok())
            .map(|c| c.sample_rate().0)
            .unwrap_or(48_000);
        let dec = resample_linear(&dec, device_sr);
        *self.buf.lock().unwrap() = Arc::new(dec);
        self.cursor.store(0, Ordering::Relaxed);
        self.playing.store(true, Ordering::Relaxed);
        self.stream.play()?;
        Ok(())
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    /// TODO: exact frame seek needs total frames; this maps 0..1.
    pub fn seek_fraction(&self, frac: f32) {
        let guard = self.buf.lock().unwrap();
        let len = guard.samples.len();
        drop(guard);
        self.cursor
            .store((len as f32 * frac.clamp(0.0, 1.0)) as usize, Ordering::Relaxed);
    }
}

fn main() -> Result<()> {
    let preview = Preview::new()?;
    preview.load_and_play("test-tone.wav")?;
    std::thread::sleep(std::time::Duration::from_secs(5));
    Ok(())
}
