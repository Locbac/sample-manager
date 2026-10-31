---
Type: UI
Tags: [randomize, recents, workflow, navigation, shortcuts]
Assoc: "[[INDEX]]", "[[Audio Engine and Auditioning]]", "[[Keyboard Navigation and Focus State Machine]]"
Reference: "crates/gui/src/main.rs"
Created: 2026-10-31
---

# Random Discovery and Session Recents

Sample browsing often suffers from choice fatigue. LocSamp introduces two complementary discovery mechanics: an accumulating **Randomize View** and an in-memory **Session Recents** tab.

---

## 1. Accumulating Randomize View (<kbd>r</kbd>)

When browsing a huge collection (e.g. 50,000 samples) or a specific filtered search (e.g. `genre: Future Bass`, `bpm: 140..160`, `key: F# min`), pressing <kbd>r</kbd> switches into `ViewMode::Randomize`:

1. **First Press (<kbd>r</kbd>)**:
   - Picks a uniform random offset in `0..total_results` via SplitMix64 pseudo-random generator:
     ```rust
     let mut x = (nanos as u64).wrapping_add(0x9E3779B97F4A7C15);
     x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
     x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
     x ^= x >> 31;
     let rand_offset = (x % (self.total_results as u64)) as u32;
     ```
   - Queries `LIMIT 1 OFFSET rand_offset` using active filters.
   - Replaces visible rows with just this single sample (`file_rows.len() == 1`).
   - Auditions and loops/plays it automatically.

2. **Subsequent Presses (<kbd>r</kbd>)**:
   - Randomizes another sample from the filtered list.
   - **Appends** it to the active view! The table now displays sample #1 and sample #2.
   - Subsequent presses add sample #3, #4, $\dots$, $N$.
   - Automatically selects, scrolls to, and auditions the newest random sample.
   - You can use <kbd>j</kbd> and <kbd>k</kbd> to move back and forth between previous random picks.

3. **Exiting Randomize View**:
   - Press <kbd>Esc</kbd> or click the `[SHOW ALL RESULTS]` button in the table header.
   - Typing in the search bar or changing filters also automatically exits `Randomize` back to `ViewMode::Library`.

---

## 2. Session Recents Tab (<kbd>Shift</kbd>+<kbd>r</kbd> / <kbd>R</kbd>)

During an active session, LocSamp keeps an in-memory journal of every sample auditioned, clicked, or randomized:

- **Hotkey**: Press <kbd>Shift</kbd>+<kbd>R</kbd> (or capital <kbd>R</kbd>).
- **UI Access**:
  - Top Bar: `[RECENTS (N)] (Shift+R)` button.
  - Left Sidebar: `🕒 Recents (N)` under `LIBRARIES`.
  - Menu Bar: `Playback -> Session Recents (Shift+R)`.
- **Behavior**:
  - Displays all auditioned samples in chronological order.
  - Bounded to the last 1,000 samples to keep RAM minimal.
  - When navigating or auditioning samples from within the Recents tab, the list order is preserved: samples are never re-appended or re-ordered when played from within Recents.
  - Features a `[CLEAR RECENTS]` button to wipe history.
  - Fully in-memory: starts clean on each application launch.
