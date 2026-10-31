---
Type: Packaging
Tags: [macos, packaging, bundle, nix, windows, linux]
Assoc: "[[INDEX]]", "[[Architecture Overview]]", "[[Performance and Benchmarks]]"
Reference: "scripts/build-macos-app.sh", "flake.nix"
Created: 2026-10-31
---

# Cross-Platform Packaging

LocSamp compiles to standalone native binaries across macOS (ARM64 & x86_64), Linux, and Windows.

---

## 1. macOS `.app` Application Bundle

On macOS, running a raw binary from Finder or Launchpad causes `launchd` to invoke the process with the working directory set to `/` (the read-only system partition). Attempting to open `locsamp.db` relatively would panic with `EACCES`.

### Database Path Resolution (`default_db_path`)
LocSamp resolves user paths dynamically based on platform standards:
- **macOS**: `~/Library/Application Support/LocSamp/locsamp.db`
- **Windows**: `%APPDATA%\LocSamp\locsamp.db`
- **Linux**: `~/.local/share/locsamp/locsamp.db`
- **Dev fallback**: `./locsamp.db` if writable in current working directory.

### Automated Bundle Script (`scripts/build-macos-app.sh`)
```bash
./scripts/build-macos-app.sh
```
This script:
1. Compiles the optimized release binary (`cargo build --release --bin locsamp`).
2. Creates the bundle directory structure: `dist/LocSamp.app/Contents/{MacOS,Resources}`.
3. Generates `Info.plist` with bundle identifier `com.locsamp.app` and high-DPI retina display flags (`NSHighResolutionCapable`).
4. Generates an application icon with audio waveform graphics (`AppIcon.icns`).

---

## 2. Nix Flake & `nix-shell`

LocSamp includes a reproducible Nix development and build environment:
- [`flake.nix`](file:///Users/amon/Projects/AI/Sandboxes/cloud/work/sample-manager/flake.nix) provides native derivations for `x86_64-linux`, `aarch64-linux`, `x86_64-darwin`, and `aarch64-darwin`.
- Automatically supplies ALSA development libraries, libudev, pkg-config, and Wayland/X11 dependencies on Linux.
- Simply run:
  ```bash
  nix develop
  # or
  nix-shell
  ```
