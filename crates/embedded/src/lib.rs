//! crates/embedded — metadata WITHOUT audio analysis (spec 4.1 item 3)
//! Runs in stage 2.5: cheap, bounded, and every returned field names its
//! origin so the DB can store source='embedded' + source_detail='wav.smpl'
//! etc. User-controllable: disable per-root if a library has hostile tags.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddedField {
    pub kind: EmbeddedKind,
    pub value: String,
    pub source_detail: &'static str, // which chunk/tag it came from
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EmbeddedKind {
    IsLoop,
    RootKey,   // raw MIDI unity note 0-127 (map to musical_keys downstream)
    Bpm,
    KeyLabel,  // textual key label e.g. "Am"
    Title,
    Comment,
}

/// Entry point: dispatch by extension. Unknown => empty, never an error.
pub fn read_embedded(path: &Path) -> Result<Vec<EmbeddedField>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "wav" => read_wav(path),
        "aif" | "aiff" | "mp3" | "flac" | "m4a" | "ogg" | "opus" => read_lofty(path),
        _ => Ok(vec![]),
    }
}

// ---------------------------------------------------------------------------
// WAV: RIFF walker -> smpl chunk (pitch + loop points)
// ---------------------------------------------------------------------------

fn read_wav(path: &Path) -> Result<Vec<EmbeddedField>> {
    let mut f = File::open(path)?;
    let mut riff = [0u8; 12];
    if f.read_exact(&mut riff).is_err() || &riff[0..4] != b"RIFF" {
        return Ok(vec![]);
    }
    if &riff[8..12] != b"WAVE" && &riff[8..12] != b"RF64" && &riff[8..12] != b"BW64" {
        return Ok(vec![]);
    }
    let mut out = Vec::new();
    let mut hdr = [0u8; 8];
    while f.read_exact(&mut hdr).is_ok() {
        let id = [&hdr[0], &hdr[1], &hdr[2], &hdr[3]]
            .map(|b| *b as char)
            .iter()
            .collect::<String>();
        let len = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64;
        match id.as_str() {
            "smpl" => {
                let mut buf = vec![0u8; len.min(1 << 16) as usize]; // cap: 64KB chunk max
                let n = f.read(&mut buf).unwrap_or(0);
                out.extend(parse_smpl(&buf[..n]));
            }
            _ => {
                // odd-sized chunks are padded to even offsets per RIFF
                let skip = len + (len & 1);
                if f.seek(SeekFrom::Current(skip as i64)).is_err() {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// smpl chunk layout (36-byte header then 24-byte loop records):
///   +12  midi_unity_note   (root key)
///   +28  num_sample_loops
///   loops: cue_id, type(0=forward), start, end, fraction, play_count
fn parse_smpl(buf: &[u8]) -> Vec<EmbeddedField> {
    let mut out = Vec::new();
    if buf.len() < 36 {
        return out;
    }
    let u32_at = |off: usize| -> u32 {
        u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
    };
    let note = u32_at(12);
    if note <= 127 && note > 0 {
        out.push(EmbeddedField {
            kind: EmbeddedKind::RootKey,
            value: note.to_string(),
            source_detail: "wav.smpl.unity_note",
        });
    }
    let n_loops = u32_at(28) as usize;
    let mut has_loop = false;
    let mut off = 36;
    for _ in 0..n_loops.min(64) {
        if off + 24 > buf.len() {
            break;
        }
        if u32_at(off + 4) == 0 {
            has_loop = true; // forward loop present
        }
        off += 24;
    }
    if has_loop {
        out.push(EmbeddedField {
            kind: EmbeddedKind::IsLoop,
            value: "1".into(),
            source_detail: "wav.smpl.loops",
        });
    }
    out
}

// ---------------------------------------------------------------------------
// id3 / vorbis / aiff via lofty
// ---------------------------------------------------------------------------

fn read_lofty(path: &Path) -> Result<Vec<EmbeddedField>> {
    use lofty::file::{AudioFile, TaggedFileExt};
    use lofty::probe::Probe;
    let probed = Probe::open(path)?.guess_file_type()?.read()?;
    let mut out = Vec::new();
    if let Some(tag) = probed.primary_tag().or_else(|| probed.first_tag()) {
        use lofty::tag::ItemKey;
        // ItemKey names vary by lofty version — pin and adjust. Candidates:
        // TempoBpm (TBPM/TMPO), KeySignature / InitialKey (TKEY).
        for (kind, keys, detail) in [
            (
                EmbeddedKind::Bpm,
                &[ItemKey::Bpm, ItemKey::IntegerBpm][..],
                "id3v2.TBPM",
            ),
            (
                EmbeddedKind::KeyLabel,
                &[ItemKey::InitialKey][..],
                "id3v2.TKEY",
            ),
            (
                EmbeddedKind::Title,
                &[ItemKey::TrackTitle][..],
                "tag.title",
            ),
        ] {
            for k in keys {
                if let Some(v) = tag.get_string(k) {
                    let v = v.trim();
                    if !v.is_empty() {
                        out.push(EmbeddedField { kind, value: v.to_string(), source_detail: detail });
                        break;
                    }
                }
            }
        }
    }
    // duration truth for content cache once hashed (cheap here: properties)
    let _ = probed.properties();
    Ok(out)
}

/// How fields land in the DB (call from the header job):
///   for f in read_embedded(path)? {
///       match f.kind {
///           IsLoop   => set files.is_loop = 1 (detail wav.smpl.loops)
///           Bpm      => files.bpm = parsed (bounded 20..=400, else ignore)
///           RootKey  => files.key_id = musical_keys lookup (major label +
///                       provenance note: mode unknown from smpl)
///           KeyLabel => tag via record_automatic_tag(source='embedded',
///                       detail=f.source_detail)
///           _        => tags as descriptors if mapped in user rules
///       }
///   }
/// Reanalysis NEVER clobbers user state — record_automatic_tag does that.
pub fn _doc_only() {}
