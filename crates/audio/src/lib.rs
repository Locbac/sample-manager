//! crates/audio — audio decode and streaming preview.
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::probe::Hint;

#[derive(Clone)]
pub struct Decoded {
    pub samples: Vec<f32>, // interleaved f32 samples
    pub channels: u16,
    pub sample_rate: u32,
}

impl Decoded {
    pub fn duration_secs(&self) -> f32 {
        if self.channels == 0 || self.sample_rate == 0 {
            0.0
        } else {
            (self.samples.len() / self.channels as usize) as f32 / self.sample_rate as f32
        }
    }
}

/// Decode an audio file into memory. Capped to prevent unbounded RAM usage.
pub fn decode(path: &Path) -> Result<Decoded> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions::default(),
        &Default::default(),
    )?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow!("no decodable audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())?;

    let mut out: Vec<f32> = Vec::new();
    let mut channels = 0u16;
    let mut sr = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    // Max 20 minutes of stereo 48kHz to prevent memory exhaustion
    let max_samples = 48_000 * 2 * 60 * 20;

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
            Ok(decoded_ref) => {
                let spec = *decoded_ref.spec();
                channels = spec.channels.count() as u16;
                sr = spec.rate;

                if sample_buf.is_none() {
                    sample_buf = Some(SampleBuffer::<f32>::new(decoded_ref.capacity() as u64, spec));
                }

                if let Some(buf) = &mut sample_buf {
                    buf.copy_interleaved_ref(decoded_ref);
                    out.extend_from_slice(buf.samples());
                }

                if out.len() >= max_samples {
                    break;
                }
            }
            Err(SymError::DecodeError(_)) => continue, // skip corrupt frames
            Err(e) => return Err(e.into()),
        }
    }

    if channels == 0 {
        return Err(anyhow!("decoded 0 channels"));
    }

    Ok(Decoded {
        samples: out,
        channels,
        sample_rate: sr,
    })
}

pub struct Player {
    pub position: Arc<AtomicUsize>,
    pub playing: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

impl Player {
    pub fn play(decoded: Arc<Decoded>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no output device available"))?;

        let config = cpal::StreamConfig {
            channels: decoded.channels,
            sample_rate: cpal::SampleRate(decoded.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let position = Arc::new(AtomicUsize::new(0));
        let playing = Arc::new(AtomicBool::new(true));

        let pos_clone = position.clone();
        let playing_clone = playing.clone();
        let data = decoded.clone();

        let stream = device.build_output_stream(
            &config,
            move |output: &mut [f32], _| {
                if !playing_clone.load(Ordering::Relaxed) {
                    output.fill(0.0);
                    return;
                }
                let mut current = pos_clone.load(Ordering::Relaxed);
                for sample in output.iter_mut() {
                    if current < data.samples.len() {
                        *sample = data.samples[current];
                        current += 1;
                    } else {
                        *sample = 0.0;
                    }
                }
                pos_clone.store(current, Ordering::Relaxed);
                if current >= data.samples.len() {
                    playing_clone.store(false, Ordering::Relaxed);
                }
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )?;

        stream.play()?;

        Ok(Self {
            position,
            playing,
            _stream: stream,
        })
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn seek_seconds(&self, secs: f32, decoded: &Decoded) {
        let frame = (secs * decoded.sample_rate as f32) as usize;
        let sample_idx = frame * decoded.channels as usize;
        self.position
            .store(sample_idx.min(decoded.samples.len()), Ordering::Relaxed);
    }
}
