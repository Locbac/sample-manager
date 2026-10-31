# SCOUT 🎧
### Fast, Local, Native Sample Library Manager for Music Producers

[![Rust](https://img.shields.io/badge/language-Rust_2021-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS_%7C_Windows_%7C_Linux-blue.svg?style=flat-square)](https://github.com)
[![Audio Engine](https://img.shields.io/badge/audio-Symphonia_%2B_CPAL-purple.svg?style=flat-square)](https://github.com/pdeljanov/Symphonia)
[![GUI](https://img.shields.io/badge/gui-egui_%2B_wgpu-red.svg?style=flat-square)](https://github.com/emilk/egui)
[![License](https://img.shields.io/badge/license-MIT-green.svg?style=flat-square)](LICENSE)
[![Codecs](https://img.shields.io/badge/codecs-WAV_•_FLAC_•_MP3_•_AIFF_•_OGG_•_AAC-informational.svg?style=flat-square)](#supported-audio-formats)

**SCOUT** is an ultra-fast, local-first sample library browser built for music producers, beatmakers, and sound designers. It replaces sluggish, subscription-based sample managers with a blazing fast desktop application that runs 100% on your machine.

---

## ⚡ Why Scout?

- 🚀 **Zero Audition Delay**: Instant playback for WAV, FLAC, and MP3 files. Even 4-minute full songs and 100MB stem files load with zero UI freeze.
- 🗂️ **Smart Auto-Tagging**: Automatically analyzes folder paths and filenames to detect **BPM**, **Musical Key** (Pitch Class & Camelot wheel), and **Type** (Loop vs. One-Shot).
- 🏷️ **Comprehensive Sound Taxonomy**: 38 genres (Future Bass, Dubstep, Trap, UK Garage, Lo-Fi, etc.), 24 drum categories, 24 instruments, and 35 FX classes.
- 🎯 **Natural Number Sorting**: Sorts your samples the way you expect (`Perc 1`, `Perc 2`, `Perc 10`, `Perc 20` — not lexicographic `Perc 1, Perc 10, Perc 2`).
- 🎚️ **Producer Workflow Controls**:
  - Scroll over the **Volume** slider or **Pitch** control to tune samples on the fly.
  - Seamless loop auditioning for melody and drum loops.
  - One-click drag-and-drop straight into your DAW (Ableton Live, FL Studio, Logic Pro, Reaper, Bitwig, Studio One, etc.).
- ⌨️ **Vim & Keyboard-First Navigation**: Browse and audition hundreds of samples without ever touching your mouse. Press **`r`** to instantly pull up a random sound!

---

## 🎹 Producer's Quickstart Guide (No Coding Experience Needed)

You do **not** need to be a programmer to build and run Scout. Follow the simple steps below for your operating system:

### 🍏 macOS (Apple Silicon M1/M2/M3/M4 & Intel)

1. **Open Terminal**:
   - Press <kbd>Cmd</kbd> + <kbd>Space</kbd>, type `Terminal`, and hit <kbd>Enter</kbd>.
2. **Install Rust** (the safe, high-speed programming language Scout is written in):
   - Paste this line into Terminal and press <kbd>Enter</kbd>:
     ```bash
     curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
     ```
   - Press <kbd>1</kbd> when prompted to proceed with default installation.
   - Restart Terminal or run: `source "$HOME/.cargo/env"`
3. **Launch Scout**:
   - Navigate to the Scout folder and start the app:
     ```bash
     cargo run --release --bin scout-gui
     ```
   > **Pro Tip**: To create a standalone desktop app, run `cargo build --release --bin scout-gui`. The executable will be waiting for you inside the `target/release/` folder.

---

### 🪟 Windows (Windows 10 / 11)

1. **Install Rust**:
   - Download and run the official 64-bit installer from **[rustup.rs](https://rustup.rs/)**.
   - If prompted for Visual Studio C++ Build Tools, select option 1 to install the required Microsoft C++ components.
2. **Open PowerShell**:
   - Press <kbd>Win</kbd> + <kbd>X</kbd> and select **Windows Terminal** or **PowerShell**.
3. **Launch Scout**:
   - In PowerShell, run:
     ```powershell
     cargo run --release --bin scout-gui
     ```

---

### 🐧 Linux (Ubuntu, Debian, Fedora, Arch)

1. **Install Audio & Graphics Dependencies**:
   - **Ubuntu/Debian**:
     ```bash
     sudo apt update && sudo apt install -y build-essential libasound2-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev
     ```
   - **Fedora**:
     ```bash
     sudo dnf install -y alsa-lib-devel libxcb-devel
     ```
   - **Arch Linux**:
     ```bash
     sudo pacman -S base-devel alsa-lib
     ```
2. **Install Rust & Launch Scout**:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   cargo run --release --bin scout-gui
   ```

---

## 🎛️ Keyboard Shortcuts & Workflow Cheatsheet

Scout is designed to keep your hands on the keyboard so you can find the perfect sample in seconds:

| Shortcut | Action | Description |
| :---: | :--- | :--- |
| <kbd>r</kbd> | **Randomize Sample** | Auditions a random sample from your filtered search results for instant inspiration |
| <kbd>l</kbd> | **Retrigger Audition** | Triggers playback from the exact beginning (drum-pad style retrigger) |
| <kbd>Space</kbd> / <kbd>Enter</kbd> | **Play / Pause** | Toggles playback of the current sample |
| <kbd>j</kbd> / <kbd>↓</kbd> | **Next Sample** | Moves down to the next sample in the list |
| <kbd>k</kbd> / <kbd>↑</kbd> | **Previous Sample** | Moves up to the previous sample in the list |
| <kbd>h</kbd> | **Toggle Favorite [*]** | Pins the sample to your favorites for quick recall |
| <kbd>/</kbd> | **Search** | Instantly highlights the search box (type queries like `kick`, `140bpm`, `F# min`) |
| <kbd>Esc</kbd> or <kbd>Tab</kbd> | **Focus Sample List** | Leaves the search box so arrow keys navigate the sample list |
| <kbd>gg</kbd> / <kbd>G</kbd> | **Top / Bottom** | Jump to the very first or very last sample |
| <kbd>yy</kbd> | **Yank Sample** | Copies the sample file so you can paste it directly into your OS or DAW |
| **Mouse Wheel** | **Scroll Pitch / Vol** | Hover over Pitch or Volume and scroll your mouse wheel to adjust |

---

## 🔬 Reverse Engineering & Static Analysis of ADSR Sample Manager

To ensure Scout provides class-leading automated categorization out of the box, we conducted a clean-room static analysis of **ADSR Sample Manager** (macOS x86_64 binary):

### Methodology & Findings
1. **Binary Disassembly & Symbol Analysis**:
   - Inspected binary image segments, strings tables, and control-flow graphs using Capstone disassembly and static string extraction.
   - Identified that ADSR relies on a comprehensive, hand-curated rule dictionary containing multi-word token patterns, parent-directory inheritance, and exclusion rules.
2. **Taxonomy Extraction**:
   - Extracted and normalized ADSR's complete 91KB taxonomy dictionary (`adsr-tag-rules.json`), covering:
     - **38 Music Genres**: Future Bass, Dubstep, Drum & Bass, UK Garage, Trap, Hip Hop, Drill, Lo-Fi, House, Techno, Synthwave, Ambient, Pop, Soul, R&B, etc.
     - **24 Drum Categories**: Kick, Snare, Clap, Hi-Hat Closed/Open, 808, Sub, Percussion, Tom, Crash, Ride, Rim, Shaker, Snap, etc.
     - **24 Instruments**: Piano, Rhodes, Organ, Guitar Acoustic/Electric, Brass, Saxophone, Strings, Synth Pluck, Lead, Pad, Flute, Vocal, etc.
     - **48 Character & Style Descriptors**: Heavy, Deep, Dark, Warm, Clean, Distorted, Punchy, Ambient, Vinyl, Acoustic, etc.
     - **35 Sound FX Types**: Impact, Riser, Downlifter, Sweep, Noise, Glitch, Laser, Siren, Transition, etc.
3. **Engine Enhancements in Scout**:
   - **N-Gram Multi-Word Matching**: Correctly categorizes multi-word phrases (e.g. `future bass`, `drum loop`, `vocal chops`) while preventing false substring matches (e.g. `clapton.wav` will not be tagged as `clap`).
   - **Parent Folder Ancestry (up to 10 levels)**: Samples located deep within subfolders (e.g. `Sample Packs/Future Bass/Drums/Snares/01.wav`) inherit the correct `Future Bass`, `Drums`, and `Snare` tags automatically.
   - **Strict Provenance & No Resurrections**: User-added manual tags are never overwritten, and dismissed tags (`state = 'dismissed'`) are never resurrected by automated rescanning.
   - **Native SQLite FTS5 Engine**: Replaced legacy index formats with an ACID-compliant, local SQLite database supporting sub-millisecond full-text searches.

---

## 📦 Supported Audio Formats

Scout decodes audio natively with hardware-accelerated vectorization:
- **WAV** (PCM 16-bit, 24-bit, 32-bit float, ADPCM)
- **FLAC** (Lossless 16-bit / 24-bit)
- **MP3** (MPEG-1 / MPEG-2 Audio Layer III, CBR & VBR)
- **AIFF / AIFC** (Apple Audio Interchange File Format)
- **OGG / Vorbis**
- **AAC / M4A** (ISO-MP4 container)

---

## 🛠️ Project Structure

```text
sample-manager/
├── crates/
│   ├── audio/        # Audio playback engine & non-blocking decoding (CPAL + Symphonia)
│   ├── db/           # Embedded SQLite schema, migrations & natural-sort collation
│   ├── scanner/      # High-throughput filesystem walker & change detection
│   ├── tagging/      # Lexical rule engine, multi-word n-grams & musical key/BPM extractors
│   ├── search/       # SQLite FTS5 search query generator & filter builder
│   ├── import-adsr/  # Importer for existing ADSR Sample Manager databases
│   ├── gui/          # egui + wgpu desktop graphical user interface
│   └── cli/          # Headless command-line utility for automated scripting
├── Cargo.toml        # Workspace manifest with opt-level=3 audio profile configuration
├── TODO.md           # Future architecture roadmap (Serum preset management, etc.)
└── README.md         # Producer & developer documentation
```

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
Built with ❤️ for music producers and sound designers worldwide.
