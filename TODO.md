# SCOUT — Project Roadmap & TODO

This document tracks upcoming architecture milestones and feature roadmaps for SCOUT (Sample & Sound Library Manager).

---

## 1. Xfer Records Serum Preset & Asset Management (`.fxp` / `.fxb`)

### Architecture & Format Parser
- [ ] **VST V2 Preset Chunk Parser (`.fxp` / `.fxb`)**:
  - Implement zero-copy binary parser for standard VST 2.4 preset chunks (`CcnP` / `FPCh` opaque byte streams).
  - Extract the embedded Serum state data chunk and validate the magic header (`1347047248` / `'Xfer'`).
- [ ] **Serum Metadata & Parameter Extraction**:
  - **Author & Pack**: Parse preset author, description, and sound design comments.
  - **Category Auto-Detection**: Extract category indicators (`BA`, `LD`, `PD`, `PL`, `FX`, `CH`, `DR`) from filenames and internal metadata.
  - **Macro Mappings**: Read Macro 1-4 custom labels and target destinations.
  - **Oscillator Configuration**:
    - Osc A & Osc B wavetable names, transpose, detune, unison voices, warp modes (Bend, Sync, FM, AM, RM, PWM).
    - Sub oscillator waveform (Sine, Triangle, Saw, Pulse).
    - Noise generator sample path and state (`.wav` noise reference in `Noises/`).
  - **LFO Shapes & Envelopes**:
    - Extract LFO 1-8 curve point coordinates for preview rendering.
  - **Filter & FX Chain**:
    - Filter types (Ladder, Comb, Flanger, Reverb, Misc) and FX bypass states (Distortion, Chorus, Delay, Reverb, EQ, Comp).
- [ ] **Wavetable & Noise Asset Resolution**:
  - Scan user's `Library/Audio/Presets/Xfer Records/Serum Presets/` (macOS) and `%APPDATA%/Xfer/Serum Presets/` (Windows).
  - Cross-reference wavetable paths referenced by presets against indexed `.wav` files.
  - Single-cycle and 2048-sample wavetable cycle visualization in Scout's waveform display.
- [ ] **DAW Drag-and-Drop Workflow**:
  - Enable dragging `.fxp` preset files directly from Scout's UI into Serum plugin windows across major DAWs (Ableton Live, FL Studio, Logic Pro, Reaper, Bitwig, Cubase, Studio One).

---

## 2. Multi-Synth Preset Support (Vital, Phase Plant, Pigments)

- [ ] **Matt Tytel Vital (`.vital`, `.vitalbank`)**:
  - Decompress gzipped JSON preset structures.
  - Extract embedded wavetable audio slices, LFO spline curves, and macro names.
  - Index and audition Vital factory and user banks.
- [ ] **Kilohearts Phase Plant (`.phaseplant`)**:
  - Parse generator modules (Analog, Wavetable, Sample, Noise) and Snapin lane effects.
- [ ] **Arturia Pigments (`.pgmx`)**:
  - Parse JSON metadata and engine assignments (Harmonic, Wavetable, Sample, Utility).

---

## 3. Streaming Audio Engine & Peak Caching

- [ ] **Chunked Audio Streaming**:
  - For long tracks (>5 minutes) or stems (>100MB), stream audio on-demand into cpal output rather than holding entire uncompressed PCM in memory.
- [ ] **Persistent Waveform Peak Pyramids**:
  - Compute multi-resolution peak pyramids (360 points, 1000 points, 4000 points) on scan.
  - Store decimation in SQLite `contents.peak_data` BLOB for instantaneous $O(1)$ zero-cost waveform rendering on row selection.
- [ ] **Real-Time Spectrogram Preview**:
  - Add optional mini-spectrogram view (FFT frequency breakdown) to inspect sub-bass frequencies (30Hz–60Hz) and high-end air.

---

## 4. Audio Similarity & Machine Learning

- [ ] **Local ONNX Runtime Embeddings**:
  - Embed lightweight local ONNX model (e.g. CLAP or OpenL3 quantized) for audio timbre embeddings.
  - Vector index (HNSW / sqlite-vec) for $O(\log N)$ "Find Similar" queries without cloud dependencies.
- [ ] **Transient & Drum Hit Detection**:
  - Exact transient onset detection for automatic slice exporting and zero-crossing detection.

---

## 5. UI / UX Enhancements

- [ ] **Tag Taxonomy Customizer**:
  - UI modal to add custom genre tags, sound designer credits, and instrument groupings.
- [ ] **BPM Multiplier / Half-Time / Double-Time**:
  - Quick toggle to preview loops at $\times 0.5$ or $\times 2.0$ pitch/tempo.
- [ ] **Export to Ableton Drum Rack / Sampler**:
  - Context menu action to export selected one-shots directly into a `.adg` (Ableton Drum Rack) preset or SFZ instrument.
