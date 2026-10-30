// spikes/drag-out/ui/main.js
// npm dep: @crabnebula/tauri-plugin-drag (v2 bindings)
import { startDrag } from "@crabnebula/tauri-plugin-drag";

const THRESHOLD_PX = 6;

// In the real app these come from the selection set: every selected row's
// absolute path. For the spike, hardcode real files you can verify in a DAW.
let currentSelection = ["/abs/path/to/kick_01.wav"];

export function attachRowDrag(rowEl, pathsForRow) {
  rowEl.addEventListener("pointerdown", (down) => {
    // Only the primary button starts a drag.
    if (down.button !== 0) return;
    const startX = down.clientX;
    const startY = down.clientY;
    let dragging = false;

    function onMove(move) {
      if (dragging) return;
      const dx = Math.abs(move.clientX - startX);
      const dy = Math.abs(move.clientY - startY);
      if (dx + dy < THRESHOLD_PX) return;
      dragging = true;
      cleanup();
      const paths = currentSelection.includes(pathsForRow[0])
        ? currentSelection
        : pathsForRow;
      // icon: a rendered 48px PNG works best; empty string is also fine.
      // onResult reports completed | cancelled per host OS.
      startDrag({ item: paths, icon: "" }, (result) => {
        console.log("drag finished:", result);
      }).catch((err) => {
        // EXPECT failure paths here in the spike: this try/catch is where
        // "view X on OS Y cannot drag out" gets recorded as data.
        console.error("startDrag failed:", err);
      });
    }
    function onUp() {
      cleanup();
    }
    function cleanup() {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    }
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  });
}

export function setSelection(paths) {
  currentSelection = paths;
}
