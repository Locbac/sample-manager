//! crates/audio — audio decode and streaming preview with resilient device negotiation and resampled playback.
use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

pub enum AudioBackend {
    Cpal(cpal::Stream),
    Virtual(Arc<AtomicBool>),
}

pub struct Player {
    pub position: Arc<AtomicUsize>, // sample index (0..samples.len())
    pub seek_frame: Arc<AtomicI64>, // target sample index to seek to, -1 if none
    pub playing: Arc<AtomicBool>,
    pub looping: Arc<AtomicBool>,
    pub volume_bits: Arc<AtomicU32>,
    pub pitch_semitones: Arc<AtomicI32>,
    _backend: AudioBackend,
}

impl Player {
    pub fn play(decoded: Arc<Decoded>) -> Result<Self> {
        let position = Arc::new(AtomicUsize::new(0));
        let seek_frame = Arc::new(AtomicI64::new(-1));
        let playing = Arc::new(AtomicBool::new(true));
        let looping = Arc::new(AtomicBool::new(false));
        let volume_bits = Arc::new(AtomicU32::new(1.0f32.to_bits()));
        let pitch_semitones = Arc::new(AtomicI32::new(0));

        let pos_clone = position.clone();
        let seek_clone = seek_frame.clone();
        let playing_clone = playing.clone();
        let loop_clone = looping.clone();
        let vol_clone = volume_bits.clone();
        let pitch_clone = pitch_semitones.clone();
        let data_cpal = decoded.clone();
        let data_virtual = decoded.clone();

        // Attempt hardware audio stream via CPAL
        let stream_result = (|| -> Result<cpal::Stream> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| anyhow!("no output device available"))?;

            let default_config = device.default_output_config()?;
            let device_channels = default_config.channels() as usize;
            let device_sample_rate = default_config.sample_rate().0;
            let stream_config: cpal::StreamConfig = default_config.into();

            let sample_channels = data_cpal.channels.max(1) as usize;
            let sample_rate = data_cpal.sample_rate.max(1);
            let total_frames = data_cpal.total_frames();

            // Resampling factor
            let resample_base = (sample_rate as f64) / (device_sample_rate as f64);
            let mut current_frame_frac: f64 = 0.0;

            let stream = device.build_output_stream(
                &stream_config,
                move |output: &mut [f32], _| {
                    if !playing_clone.load(Ordering::Relaxed) {
                        output.fill(0.0);
                        return;
                    }

                    let is_loop = loop_clone.load(Ordering::Relaxed);
                    let vol = f32::from_bits(vol_clone.load(Ordering::Relaxed));
                    let pitch = pitch_clone.load(Ordering::Relaxed);
                    let pitch_speed = 2.0_f64.powf(pitch as f64 / 12.0);
                    let speed = resample_base * pitch_speed;

                    // Handle pending seek request or drift sync
                    let req = seek_clone.swap(-1, Ordering::Relaxed);
                    if req >= 0 {
                        current_frame_frac = (req as usize / sample_channels) as f64;
                    } else {
                        let target_sample_pos = pos_clone.load(Ordering::Relaxed);
                        let target_frame = (target_sample_pos / sample_channels) as f64;
                        if (target_frame - current_frame_frac).abs() > 100.0 {
                            current_frame_frac = target_frame;
                        }
                    }

                    if total_frames == 0 {
                        output.fill(0.0);
                        return;
                    }

                    let out_frames = output.len() / device_channels.max(1);
                    for f in 0..out_frames {
                        let mut frame_floor = current_frame_frac.floor() as usize;
                        if frame_floor >= total_frames {
                            if is_loop && total_frames > 0 {
                                current_frame_frac %= total_frames as f64;
                                frame_floor = current_frame_frac.floor() as usize;
                            } else {
                                playing_clone.store(false, Ordering::Relaxed);
                                for sample in &mut output[f * device_channels..] {
                                    *sample = 0.0;
                                }
                                break;
                            }
                        }

                        let frame_ceil = if is_loop && total_frames > 0 {
                            (frame_floor + 1) % total_frames
                        } else {
                            (frame_floor + 1).min(total_frames.saturating_sub(1))
                        };
                        let frac = (current_frame_frac - frame_floor as f64) as f32;

                        let max_idx = data_cpal.samples.len().saturating_sub(1);
                        for dev_ch in 0..device_channels {
                            let src_ch = if sample_channels == 1 {
                                0
                            } else {
                                dev_ch % sample_channels
                            };
                            let idx1 = (frame_floor * sample_channels + src_ch).min(max_idx);
                            let idx2 = (frame_ceil * sample_channels + src_ch).min(max_idx);
                            let s1 = data_cpal.samples[idx1];
                            let s2 = data_cpal.samples[idx2];
                            let val = s1 + frac * (s2 - s1);
                            output[f * device_channels + dev_ch] = (val * vol).clamp(-1.0, 1.0);
                        }

                        current_frame_frac += speed;
                    }

                    pos_clone.store(
                        (current_frame_frac as usize * sample_channels).min(data_cpal.samples.len()),
                        Ordering::Relaxed,
                    );
                },
                |err| eprintln!("audio stream error: {err}"),
                None,
            )?;

            stream.play()?;
            Ok(stream)
        })();

        let backend = match stream_result {
            Ok(s) => AudioBackend::Cpal(s),
            Err(e) => {
                eprintln!("Audio hardware stream failed ({e}), using high-precision virtual playback clock");
                let stop_signal = Arc::new(AtomicBool::new(false));
                let stop_clone = stop_signal.clone();
                let v_pos = position.clone();
                let v_seek = seek_frame.clone();
                let v_playing = playing.clone();
                let v_loop = looping.clone();
                let v_pitch = pitch_semitones.clone();
                let v_channels = data_virtual.channels.max(1) as usize;
                let v_sr = data_virtual.sample_rate.max(1);
                let v_total = data_virtual.samples.len();

                std::thread::spawn(move || {
                    let tick_dur = Duration::from_millis(25);
                    while !stop_clone.load(Ordering::Relaxed) {
                        std::thread::sleep(tick_dur);
                        let req = v_seek.swap(-1, Ordering::Relaxed);
                        if req >= 0 {
                            v_pos.store(req as usize, Ordering::Relaxed);
                        }
                        if !v_playing.load(Ordering::Relaxed) {
                            continue;
                        }
                        let pitch = v_pitch.load(Ordering::Relaxed);
                        let speed = 2.0_f64.powf(pitch as f64 / 12.0);
                        let samples_per_tick = ((v_sr as f64 * 0.025 * speed) as usize) * v_channels;
                        let cur = v_pos.load(Ordering::Relaxed);
                        let next = cur + samples_per_tick;
                        if next >= v_total {
                            if v_loop.load(Ordering::Relaxed) {
                                v_pos.store(0, Ordering::Relaxed);
                            } else {
                                v_pos.store(v_total, Ordering::Relaxed);
                                v_playing.store(false, Ordering::Relaxed);
                            }
                        } else {
                            v_pos.store(next, Ordering::Relaxed);
                        }
                    }
                });

                AudioBackend::Virtual(stop_signal)
            }
        };

        Ok(Self {
            position,
            seek_frame,
            playing,
            looping,
            volume_bits,
            pitch_semitones,
            _backend: backend,
        })
    }

    pub fn pause(&self) {
        self.playing.store(false, Ordering::Relaxed);
    }

    pub fn resume(&self) {
        self.playing.store(true, Ordering::Relaxed);
    }

    pub fn seek(&self, sample_idx: usize) {
        self.seek_frame.store(sample_idx as i64, Ordering::Relaxed);
        self.position.store(sample_idx, Ordering::Relaxed);
    }

    pub fn restart(&self) {
        self.seek(0);
        self.resume();
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
        self.seek(sample_idx.min(decoded.samples.len()));
    }

    pub fn seek_seconds(&self, secs: f32, decoded: &Decoded) {
        let frame = (secs * decoded.sample_rate as f32) as usize;
        let sample_idx = frame * decoded.channels as usize;
        self.seek(sample_idx.min(decoded.samples.len()));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let AudioBackend::Virtual(stop) = &self._backend {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_player_playback() {
        let decoded = Arc::new(Decoded {
            samples: vec![0.0; 44100 * 2], // 1 second of stereo audio
            channels: 2,
            sample_rate: 44100,
        });
        let player = Player::play(decoded.clone()).expect("Failed to create player");
        assert!(player.playing.load(Ordering::Relaxed));
        std::thread::sleep(Duration::from_millis(100));
        let pos = player.position.load(Ordering::Relaxed);
        assert!(pos > 0, "Position should advance during playback");
        player.pause();
        assert!(!player.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn test_decode_wav_pcm() {
        let path = Path::new("demo-samples/808_sub_kick_c.wav");
        if path.exists() {
            let decoded = decode(path).expect("Decode 808 PCM WAV should succeed");
            assert!(decoded.samples.len() > 0);
            assert_eq!(decoded.channels, 2);
            assert_eq!(decoded.sample_rate, 44100);
        }
    }
}
