# Scout

Fast, local sample library manager for music producers and sound designers.

[![Rust](https://img.shields.io/badge/language-Rust_2021-orange.svg?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS_%7C_Windows_%7C_Linux-blue.svg?style=flat-square)](https://github.com/locbac/sample-manager)
[![Audio Engine](https://img.shields.io/badge/audio-Symphonia_%2B_CPAL-purple.svg?style=flat-square)](https://github.com/pdeljanov/Symphonia)
[![GUI](https://img.shields.io/badge/gui-egui_%2B_wgpu-red.svg?style=flat-square)](https://github.com/emilk/egui)
[![Nix](https://img.shields.io/badge/nix-flake_%2B_shell-blueviolet.svg?style=flat-square&logo=nixos)](flake.nix)
[![License](https://img.shields.io/badge/license-MIT-green.svg?style=flat-square)](LICENSE)

Scout is an open source desktop sample browser written in Rust. It indexes your audio samples into an embedded SQLite database, detects BPM and musical keys from filenames, and gives you keyboard-first navigation and instant playback without cloud accounts, subscriptions, or heavy web frameworks.

---

## Why Scout

- **Non-blocking playback**: Background streaming decoder (Symphonia + CPAL) starts playback immediately without UI stutter, even on multi-minute stems or 100MB files.
- **Automatic key and BPM detection**: Extracts tempo, pitch class, and Camelot wheel notation from filenames and folder structures. Distinguishes loops from one-shots.
- **Rule-based tagging**: Categorizes samples across 38 genres, 24 drum types, 24 instruments, and 35 FX classes using boundary-aware token matching.
- **Natural numerical sorting**: Numbers sort intuitively (`Kick 1`, `Kick 2`, `Kick 10` rather than `Kick 1`, `Kick 10`, `Kick 2`).
- **DAW drag-and-drop**: Drag samples directly into Ableton Live, FL Studio, Logic Pro, Bitwig, Reaper, or Studio One.
- **In-place pitch and volume tuning**: Hover over pitch or volume sliders and use the scroll wheel to tune samples in semitones before dropping them into your project.
- **Keyboard navigation**: Navigate using arrow keys or Vim bindings (`j`/`k` to browse, `r` for a random sample, `/` to search, `Space` to audition).

---

## Quickstart

You need a Rust toolchain installed to build Scout from source.

### macOS

1. Install Rust:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   ```
2. Run Scout:
   ```bash
   cargo run --release --bin scout-gui
   ```

To build a standalone release binary, run `cargo build --release --bin scout-gui`. The executable is produced at `target/release/scout-gui`.

---

### Windows

1. Install Rust via the installer at [rustup.rs](https://rustup.rs/) (including the MSVC C++ build tools if prompted).
2. Open PowerShell or Windows Terminal and run:
   ```powershell
   cargo run --release --bin scout-gui
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
2. Install Rust and run Scout:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   source "$HOME/.cargo/env"
   cargo run --release --bin scout-gui
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

Then run Scout inside the shell:
```bash
cargo run --release --bin scout-gui
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

To provide automatic categorization out of the box, Scout includes a clean-room implementation of the taxonomy patterns found in ADSR Sample Manager:

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
│   ├── gui/          # egui + wgpu desktop interface
│   └── cli/          # Headless CLI for scripting and maintenance
├── Cargo.toml        # Workspace manifest with release audio optimization profile
├── TODO.md           # Roadmap and planned features
└── README.md
```

---

## Building releases for all platforms

Scout includes an automated GitHub Actions release workflow (`.github/workflows/release.yml`) that compiles binaries across all supported operating systems and architectures.

### Automated GitHub Release (Recommended)

To create a new release and build binaries for all targets:

1. Tag the release:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
2. GitHub Actions will automatically spin up native runners and build:
   - **macOS Universal Binary** (`scout-macos-universal.zip`): Dual-architecture executable supporting both Apple Silicon (ARM64) and Intel (x86_64).
   - **Windows x86_64** (`scout-windows-x86_64.zip`): `scout-gui.exe` and `scout.exe`.
   - **Linux x86_64** (`scout-linux-x86_64.tar.gz`).
   - **Linux ARM64** (`scout-linux-arm64.tar.gz`).
   - **Checksums** (`SHA256SUMS.txt`).

A release will be automatically drafted and published on your GitHub repository with all binaries attached. You can also trigger this manually from the **Actions** tab on GitHub via `workflow_dispatch`.

### Building locally

If you want to compile release binaries manually on your local system:

- **macOS (Universal Binary for both ARM64 and Intel)**:
  ```bash
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --target aarch64-apple-darwin --bin scout-gui
  cargo build --release --target x86_64-apple-darwin --bin scout-gui
  lipo -create -output scout-gui \
    target/aarch64-apple-darwin/release/scout-gui \
    target/x86_64-apple-darwin/release/scout-gui
  ```

- **Windows (x86_64)**:
  ```powershell
  cargo build --release --bin scout-gui
  ```

- **Linux (x86_64 or ARM64)**:
  ```bash
  cargo build --release --bin scout-gui
  ```

---

## License

Licensed under the [MIT License](LICENSE).
