# Spike: native drag-out (THE gate for Tauri)

Goal: prove a real OS drag session can carry FILE URLs out of the webview
into a DAW. Everything else about the UI shell is negotiable; this is not.

Chosen approach: crabnebula tauri-plugin-drag (works on tauri v2 windows;
Linux implementation is GTK-based and works with wry, NOT with bare winit).

## Files here

  src-tauri/src/main.rs   plugin registration + permissions note
  ui/main.js              pointer tracking -> startDrag({ item: [paths] })

## Test protocol (record into docs/re/alternatives.md)

| host | DAW | single file | multi-file | notes |
| --- | --- | --- | --- | --- |
| macOS | Logic / Ableton Live |  |  |  |
| Windows | FL Studio / Reaper / Ableton |  |  |  |
| Linux (optional) | Reaper |  |  | webkit2gtk quirks expected |

For each: does the DAW accept the drop? Does it COPY or REFERENCE the file?
Does drop position in an arrangement matter (time/marker)?

## Known caveats (check drag-rs issue #72 before shipping)

  - Dragging files living on SMB/NAS paths can CRASH the process.
    Mitigation in the real app: if root is on a network mount, stage to temp
    dir first, drag the temp copy.
  - HTML5 drag events and the native session fight each other; start the
    native drag from pointermove AFTER a movement threshold, and do not mark
    rows draggable=true.
  - Large multi-selection: pass every path in one startDrag call, not N calls.

## If this fails on a target OS

Plan B: Electron (webContents.startDrag is first-class).
Plan C: fully-native Rust UI (egui/winit) + the raw 'drag' crate.
Do not proceed to M3 on a shell that has not passed this spike.
