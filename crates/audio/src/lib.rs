//! crates/audio — audio decode and streaming preview.
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
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

    pub fn total_frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
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
    pub position: Arc<AtomicUsize>, // sample index (0..samples.len())
    pub playing: Arc<AtomicBool>,
    pub looping: Arc<AtomicBool>,
    pub volume_bits: Arc<AtomicU32>,
    pub pitch_semitones: Arc<AtomicI32>,
    _stream: cpal::Stream,
}

impl Player {
    pub fn play(decoded: Arc<Decoded>) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no output device available"))?;

        let channels = decoded.channels;
        let config = cpal::StreamConfig {
            channels,
            sample_rate: cpal::SampleRate(decoded.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let position = Arc::new(AtomicUsize::new(0));
        let playing = Arc::new(AtomicBool::new(true));
        let looping = Arc::new(AtomicBool::new(false));
        let volume_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let pitch_semitones = Arc::new(AtomicI32::new(0));

        let pos_clone = position.clone();
        let playing_clone = playing.clone();
        let loop_clone = looping.clone();
        let vol_clone = volume_bits.clone();
        let pitch_clone = pitch_semitones.clone();
        let data = decoded.clone();

        let num_ch = channels as usize;
        let total_frames = data.total_frames();

        let mut current_frame_frac: f64 = 0.0;

        let stream = device.build_output_stream(
            &config,
            move |output: &mut [f32], _| {
                if !playing_clone.load(Ordering::Relaxed) {
                    output.fill(0.0);
                    return;
                }

                let is_loop = loop_clone.load(Ordering::Relaxed);
                let vol = f32::from_bits(vol_clone.load(Ordering::Relaxed));
                let pitch = pitch_clone.load(Ordering::Relaxed);
                let speed = 2.0_f64.powf(pitch as f64 / 12.0);

                // Sync current_frame_frac with atomic position if user sought
                let target_sample_pos = pos_clone.load(Ordering::Relaxed);
                let target_frame = (target_sample_pos / num_ch.max(1)) as f64;
                if (target_frame - current_frame_frac).abs() > 50.0 {
                    current_frame_frac = target_frame;
                }

                if total_frames == 0 {
                    output.fill(0.0);
                    return;
                }

                let mut out_idx = 0;
                while out_idx < output.len() {
                    let frame_floor = current_frame_frac.floor() as usize;
                    if frame_floor >= total_frames {
                        if is_loop {
                            current_frame_frac = 0.0;
                            continue;
                        } else {
                            // Audio finished
                            playing_clone.store(false, Ordering::Relaxed);
                            while out_idx < output.len() {
                                output[out_idx] = 0.0;
                                out_idx += 1;
                            }
                            break;
                        }
                    }

                    let frame_ceil = (frame_floor + 1).min(total_frames - 1);
                    let frac = (current_frame_frac - frame_floor as f64) as f32;

                    for ch in 0..num_ch {
                        if out_idx >= output.len() {
                            break;
                        }
                        let s1 = data.samples[frame_floor * num_ch + ch];
                        let s2 = data.samples[frame_ceil * num_ch + ch];
                        let val = s1 + frac * (s2 - s1);
                        output[out_idx] = (val * vol).clamp(-1.0, 1.0);
                        out_idx += 1;
                    }

                    current_frame_frac += speed;
                }

                pos_clone.store((current_frame_frac as usize * num_ch).min(data.samples.len()), Ordering::Relaxed);
            },
            |err| eprintln!("audio stream error: {err}"),
            None,
        )?;

        stream.play()?;

        Ok(Self {
            position,
            playing,
            looping,
            volume_bits,
            pitch_semitones,
            _stream: stream,
        })
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn set_loop(&self, looping: bool) {
        self.looping.store(looping, Ordering::Relaxed);
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume_bits.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn set_pitch(&self, semitones: i32) {
        self.pitch_semitones.store(semitones, Ordering::Relaxed);
    }

    pub fn seek_fraction(&self, frac: f32, decoded: &Decoded) {
        let frac = frac.clamp(0.0, 1.0);
        let sample_idx = (frac * decoded.samples.len() as f32) as usize;
        self.position.store(sample_idx.min(decoded.samples.len()), Ordering::Relaxed);
    }

    pub fn seek_seconds(&self, secs: f32, decoded: &Decoded) {
        let frame = (secs * decoded.sample_rate as f32) as usize;
        let sample_idx = frame * decoded.channels as usize;
        self.position
            .store(sample_idx.min(decoded.samples.len()), Ordering::Relaxed);
    }
}
