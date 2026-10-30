# scoutkit — starter scaffold for the sample-library browser

This kit is a *starting point*, not a finished app. It exists so the hard
architectural decisions are already written down in compiling-adjacent form:

  1. Run the M1 spikes FIRST. The drag-out spike is the technical gate for
     the whole UI decision. If startDrag() does not hand real files to the
     target DAWs on macOS and Windows, stop and pick another shell.
  2. Fill in docs/re/* from real observation. The templates in M0 have
     decision rules for each experiment — record evidence, not vibes.
  3. Stand up the M2 core via the CLI before writing any UI. schema.sql +
     scanner + tagging + search are testable headless.
  4. Nothing here was copied from ADSR. Behavior parity notes live only in
     docs/re/, and the DB importer reads a user-owned copy of their data.

Version pinning
  Cargo versions in the workspace manifest were current-ish when this kit was
  written — run "cargo update" and re-pin whatever resolves, then record the
  pins. lofty in particular changes APIs between minor versions.

Known traps already flagged in the code
  - tauri-plugin-drag crashes on SMB/network paths upstream (issue #72).
    Stage network files to temp before starting the OS drag.
  - FTS5 tag search is refreshed by triggers; after a bulk re-tag, consider
    files_fts('rebuild') scheduled off-peak.
  - symphonia duration for VBR mp3 is an estimate until fully parsed.
