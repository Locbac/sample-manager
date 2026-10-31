# LocSamp

Fast, local sample library manager for music producers and sound designers.

[![Rust](https://img.shields.io/badge/language-Rust_2021-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS_%7C_Windows_%7C_Linux-blue.svg?style=flat-square)](https://github.com/locbac/sample-manager)
[![Audio Engine](https://img.shields.io/badge/audio-Symphonia_%2B_CPAL-purple.svg?style=flat-square)](https://github.com/pdeljanov/Symphonia)
[![GUI](https://img.shields.io/badge/gui-egui_%2B_wgpu-red.svg?style=flat-square)](https://github.com/emilk/egui)
[![Nix](https://img.shields.io/badge/nix-flake_%2B_shell-blueviolet.svg?style=flat-square&logo=nixos)](flake.nix)
[![License](https://img.shields.io/badge/license-MIT-green.svg?style=flat-square)](LICENSE)

LocSamp is an open source desktop sample browser written in Rust. It indexes your audio samples into an embedded SQLite database, detects BPM and musical keys from filenames, and gives you keyboard-first navigation and instant playback without cloud accounts, subscriptions, or heavy web frameworks.

---

## Why LocSamp

- **Non-blocking playback**: Background streaming decoder (Symphonia + CPAL) starts playback immediately without UI stutter, even on multi-minute stems or 100MB files.
- **Automatic key and BPM detection**: Extracts tempo, pitch class, and Camelot wheel notation from filenames and folder structures. Distinguishes loops from one-shots.
- **Rule-based tagging**: Categorizes samples across 38 genres, 24 drum types, 24 instruments, and 35 FX classes using boundary-aware token matching.
- **Natural numerical sorting**: Numbers sort intuitively (`Kick 1`, `Kick 2`, `Kick 10` rather than `Kick 1`, `Kick 10`, `Kick 2`).
- **DAW drag-and-drop**: Drag samples directly into Ableton Live, FL Studio, Logic Pro, Bitwig, Reaper, or Studio One.
- **In-place pitch and volume tuning**: Hover over pitch or volume sliders and use the scroll wheel to tune samples in semitones before dropping them into your project.
- **Keyboard navigation**: Navigate using arrow keys or Vim bindings (`j`/`k` to browse, `r` for a random sample, `/` to search, `Space` to audition).

---

## Quickstart

You need a Rust toolchain installed to build LocSamp from source.

### macOS

1. Install Rust:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   ```
2. Run LocSamp:
   ```bash
   cargo run --release --bin locsamp
   ```

To build a standalone macOS `.app` bundle you can drag to `/Applications`, run:
```bash
./scripts/build-macos-app.sh
```
This generates `dist/LocSamp.app`.

---

### Windows

1. Install Rust via the installer at [rustup.rs](https://rustup.rs/) (including the MSVC C++ build tools if prompted).
2. Open PowerShell or Windows Terminal and run:
   ```powershell
   cargo run --release --bin locsamp
   ```

---

### Linux

1. Install system development and audio libraries:
   - **Debian / Ubuntu**:
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
2. Install Rust and run LocSamp:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   cargo run --release --bin locsamp
   ```

---

### Nix

If you use Nix or NixOS, enter the pre-configured developer environment with all audio and graphical dependencies:

```bash
# Using Nix flakes
nix develop

# Using classic nix-shell
nix-shell
```

Then run LocSamp inside the shell:
```bash
cargo run --release --bin locsamp
```

---

## Keyboard shortcuts

| Shortcut | Action | Description |
| :--- | :--- | :--- |
| <kbd>r</kbd> | Random sample | Auditions a random sample from the current filtered results |
| <kbd>l</kbd> | Retrigger | Plays current sample from the beginning |
| <kbd>Space</kbd> / <kbd>Enter</kbd> | Play / Pause | Toggles playback |
| <kbd>j</kbd> / <kbd>↓</kbd> | Next | Moves to the next sample |
| <kbd>k</kbd> / <kbd>↑</kbd> | Previous | Moves to the previous sample |
| <kbd>h</kbd> | Favorite | Toggles favorite status |
| <kbd>/</kbd> | Search | Focuses search bar (supports queries like `kick`, `140bpm`, `F# min`) |
| <kbd>Esc</kbd> / <kbd>Tab</kbd> | Focus list | Returns focus to sample list from search |
| <kbd>gg</kbd> / <kbd>G</kbd> | First / Last | Jumps to start or end of list |
| <kbd>yy</kbd> | Yank file | Copies sample file path to clipboard |
| **Scroll wheel** | Pitch / Volume | Hover over pitch or volume controls and scroll to adjust |

---

## Tagging taxonomy and ADSR analysis

To provide automatic categorization out of the box, LocSamp includes a clean-room implementation of the taxonomy patterns found in ADSR Sample Manager:

1. **Taxonomy dictionary**:
   Includes rules for 38 genres, 24 drum types, 24 instruments, 48 character descriptors, and 35 sound FX types.
2. **N-gram token matching**:
   Matches multi-word phrases (`future bass`, `drum loop`, `vocal chops`) while avoiding false substring collisions (e.g. `clapton.wav` is not tagged as `clap`, and `strap.wav` is not tagged as `trap`).
3. **Parent folder inheritance**:
   Traverses up to 10 directory levels so nested files automatically inherit tags from their parent folder structure (e.g. `Packs/Future Bass/Drums/Snares/01.wav` inherits `Future Bass`, `Drums`, and `Snare`).
4. **Persistent user provenance**:
   User-added tags are never overwritten by scans. Dismissed tags remain dismissed and are not resurrected during library rescans.
5. **Embedded SQLite FTS5**:
   Metadata, tags, and file paths are indexed in a local SQLite database with full-text search, keeping query times under a millisecond.

---

## Supported audio formats

Decoded via [Symphonia](https://github.com/pdeljanov/Symphonia):
- **WAV** (PCM 16-bit, 24-bit, 32-bit float, ADPCM)
- **FLAC** (Lossless 16-bit / 24-bit)
- **MP3** (MPEG-1 / MPEG-2 Audio Layer III, CBR & VBR)
- **AIFF / AIFC**
- **OGG / Vorbis**
- **AAC / M4A**

---

## Project structure

```text
sample-manager/
├── crates/
│   ├── audio/        # Audio playback engine and non-blocking decoding (CPAL + Symphonia)
│   ├── db/           # Embedded SQLite schema, migrations, and natural-sort collation
│   ├── scanner/      # Filesystem walker and change detection
│   ├── tagging/      # Lexical rule engine, multi-word n-grams, and key/BPM extractors
│   ├── search/       # SQLite FTS5 query generator and filter builder
│   ├── import-adsr/  # Importer for existing ADSR Sample Manager databases
│   ├── gui/          # egui + wgpu desktop interface (locsamp binary)
│   └── cli/          # Headless CLI for scripting and maintenance (locsamp-cli)
├── Cargo.toml        # Workspace manifest with release audio optimization profile
├── TODO.md           # Roadmap and planned features
└── README.md
```

---

## License

Licensed under the [MIT License](LICENSE).
