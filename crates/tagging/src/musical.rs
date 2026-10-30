//! crates/tagging/src/musical.rs — Lexical extraction of BPM, musical keys, and loop/shot classification.


#[derive(Debug, Clone, PartialEq, Default)]
pub struct MusicalInfo {
    pub bpm: Option<f64>,
    pub key: Option<String>,      // Canonical key label e.g. "C#m", "Am", "G"
    pub camelot: Option<String>,  // Camelot label e.g. "12A", "8A", "9B"
    pub is_loop: Option<bool>,    // true for loops, false for shots
}

/// Camelot table: (Camelot, Canonical Label, Alternative Enharmonic Label)
static CAMELOT_ENTRIES: &[(&str, &str, &str)] = &[
    ("1A", "Abm", "G#m"),
    ("1B", "B", "Bmaj"),
    ("2A", "Ebm", "D#m"),
    ("2B", "F#", "Gb"),
    ("3A", "Bbm", "A#m"),
    ("3B", "Db", "C#"),
    ("4A", "Fm", "Fmin"),
    ("4B", "Ab", "G#"),
    ("5A", "Cm", "Cmin"),
    ("5B", "Eb", "D#"),
    ("6A", "Gm", "Gmin"),
    ("6B", "Bb", "A#"),
    ("7A", "Dm", "Dmin"),
    ("7B", "F", "Fmaj"),
    ("8A", "Am", "Amin"),
    ("8B", "C", "Cmaj"),
    ("9A", "Em", "Emin"),
    ("9B", "G", "Gmaj"),
    ("10A", "Bm", "Bmin"),
    ("10B", "D", "Dmaj"),
    ("11A", "F#m", "Gbm"),
    ("11B", "A", "Amaj"),
    ("12A", "C#m", "Dbm"),
    ("12B", "E", "Emaj"),
];

/// Extract BPM, Musical Key, and Loop status from filename and relative path.
pub fn extract_musical_info(name: &str, rel_path: &str) -> MusicalInfo {
    let bpm = extract_bpm(name, rel_path);
    let (key, camelot) = extract_key(name, rel_path)
        .map(|(k, c)| (Some(k), Some(c)))
        .unwrap_or((None, None));
    let is_loop = extract_is_loop(name, rel_path, bpm.is_some());

    MusicalInfo {
        bpm,
        key,
        camelot,
        is_loop: Some(is_loop),
    }
}

/// Extract BPM from filename or path (e.g. "synth_pluck_lead_140bpm.wav" -> 140.0).
pub fn extract_bpm(name: &str, rel_path: &str) -> Option<f64> {
    let clean_name = name.to_ascii_lowercase();
    let clean_path = rel_path.to_ascii_lowercase();

    // 1. Look for explicit "bpm" pattern: (\d+(?:\.\d+)?)\s*bpm or bpm\s*(\d+(?:\.\d+)?)
    if let Some(val) = find_explicit_bpm(&clean_name) {
        return Some(val);
    }
    if let Some(val) = find_explicit_bpm(&clean_path) {
        return Some(val);
    }

    // 2. Tokenized search for standalone numbers in valid BPM range if context implies loop/tempo
    let is_tempo_context = clean_name.contains("loop")
        || clean_name.contains("tempo")
        || clean_name.contains("beat")
        || clean_name.contains("groove")
        || clean_name.contains("arp")
        || clean_path.contains("loop")
        || clean_path.contains("tempo");

    if is_tempo_context {
        // Extract isolated numbers
        let tokens = crate::tokenize(name);
        for tok in tokens {
            if let Ok(num) = tok.parse::<f64>() {
                // Must be reasonable tempo and not iconic gear or bit depth/sample rate
                if (60.0..=220.0).contains(&num)
                    && tok != "808"
                    && tok != "909"
                    && tok != "303"
                    && tok != "707"
                    && tok != "16"
                    && tok != "24"
                    && tok != "32"
                    && tok != "44"
                    && tok != "48"
                    && tok != "96"
                {
                    return Some(num);
                }
            }
        }
    }

    None
}

fn find_explicit_bpm(s: &str) -> Option<f64> {
    // Look for occurrences of "bpm"
    let mut search_idx = 0;
    while let Some(pos) = s[search_idx..].find("bpm") {
        let actual_pos = search_idx + pos;
        search_idx = actual_pos + 3;

        // Check digits preceding "bpm" (e.g. "140bpm", "140.5bpm", "140 bpm")
        let prefix = s[..actual_pos].trim_end();
        let mut start = prefix.len();
        while start > 0 {
            let ch = prefix.as_bytes()[start - 1];
            if ch.is_ascii_digit() || ch == b'.' {
                start -= 1;
            } else {
                break;
            }
        }
        if start < prefix.len() {
            let candidate = &prefix[start..];
            if let Ok(val) = candidate.parse::<f64>() {
                if (40.0..=300.0).contains(&val) && val != 808.0 && val != 909.0 {
                    return Some(val);
                }
            }
        }

        // Check digits following "bpm" (e.g. "bpm140", "bpm_140", "bpm 140")
        let suffix = s[actual_pos + 3..].trim_start_matches(|c: char| c == '_' || c == '-' || c == ' ');
        let end = suffix
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(suffix.len());
        if end > 0 {
            let candidate = &suffix[..end];
            if let Ok(val) = candidate.parse::<f64>() {
                if (40.0..=300.0).contains(&val) && val != 808.0 && val != 909.0 {
                    return Some(val);
                }
            }
        }
    }
    None
}

/// Extract Musical Key and Camelot code (e.g. "C#m" -> ("C#m", "12A"), "8A" -> ("Am", "8A")).
pub fn extract_key(name: &str, rel_path: &str) -> Option<(String, String)> {
    let lower_name = name.to_ascii_lowercase();

    // 1. Check for Camelot codes directly in name (e.g. "8A", "12A", "5B", "11B")
    if let Some(res) = find_camelot(name) {
        return Some(res);
    }

    // 2. Check for explicit key tokens in filename
    let tokens = crate::tokenize(name);
    for tok in &tokens {
        if let Some(res) = parse_key_token(tok) {
            return Some(res);
        }
    }

    // 3. Check for note name after accidental or separator: e.g. "808_sub_kick_c.wav" -> "C"
    // Only if context is melodic / bass / tonally defined
    let is_tonal = lower_name.contains("808")
        || lower_name.contains("bass")
        || lower_name.contains("sub")
        || lower_name.contains("synth")
        || lower_name.contains("lead")
        || lower_name.contains("pluck")
        || lower_name.contains("melody")
        || lower_name.contains("chord")
        || lower_name.contains("pad")
        || lower_name.contains("rhodes")
        || lower_name.contains("piano")
        || lower_name.contains("key");

    if is_tonal {
        for tok in tokens.iter().rev() {
            if tok.len() == 1 {
                let c = tok.chars().next().unwrap();
                if ('a'..='g').contains(&c) {
                    let upper = c.to_ascii_uppercase().to_string();
                    if let Some((cam, canon, _)) = CAMELOT_ENTRIES.iter().find(|(_, canon, _)| *canon == upper) {
                        return Some((canon.to_string(), cam.to_string()));
                    }
                }
            } else if tok.len() == 2 && tok.ends_with(|c: char| c.is_ascii_digit()) {
                // e.g. "c2", "f3"
                let c = tok.chars().next().unwrap();
                if ('a'..='g').contains(&c) {
                    let upper = c.to_ascii_uppercase().to_string();
                    if let Some((cam, canon, _)) = CAMELOT_ENTRIES.iter().find(|(_, canon, _)| *canon == upper) {
                        return Some((canon.to_string(), cam.to_string()));
                    }
                }
            }
        }
    }

    // 4. Check folder path for Camelot or key cues if filename had none
    if let Some(res) = find_camelot(rel_path) {
        return Some(res);
    }

    let path_tokens = crate::tokenize(rel_path);
    for tok in &path_tokens {
        if let Some(res) = parse_key_token(tok) {
            return Some(res);
        }
    }

    None
}

fn find_camelot(s: &str) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    for (cam, canon, _) in CAMELOT_ENTRIES {
        let cam_bytes = cam.as_bytes();
        let cam_len = cam_bytes.len();

        let mut start = 0;
        while start + cam_len <= len {
            if bytes[start..start + cam_len].eq_ignore_ascii_case(cam_bytes) {
                // Check boundary before
                let boundary_before = start == 0 || {
                    let prev = bytes[start - 1];
                    !prev.is_ascii_alphanumeric()
                };
                // Check boundary after
                let boundary_after = start + cam_len == len || {
                    let next = bytes[start + cam_len];
                    !next.is_ascii_alphanumeric()
                };

                if boundary_before && boundary_after {
                    return Some((canon.to_string(), cam.to_string()));
                }
            }
            start += 1;
        }
    }

    None
}

fn parse_key_token(tok: &str) -> Option<(String, String)> {
    let lower = tok.to_ascii_lowercase();

    // Standard notation lookup table
    // (Pattern, Canonical Label, Camelot)
    const KEY_PATTERNS: &[(&str, &str, &str)] = &[
        // Minor keys
        ("c#m", "C#m", "12A"), ("c#min", "C#m", "12A"), ("c#minor", "C#m", "12A"), ("csm", "C#m", "12A"),
        ("dbm", "Dbm", "12A"), ("dbmin", "Dbm", "12A"), ("dbminor", "Dbm", "12A"),
        ("d#m", "D#m", "2A"),  ("d#min", "D#m", "2A"),  ("d#minor", "D#m", "2A"),  ("dsm", "D#m", "2A"),
        ("ebm", "Ebm", "2A"),  ("ebmin", "Ebm", "2A"),  ("ebminor", "Ebm", "2A"),
        ("f#m", "F#m", "11A"), ("f#min", "F#m", "11A"), ("f#minor", "F#m", "11A"), ("fsm", "F#m", "11A"),
        ("gbm", "Gbm", "11A"), ("gbmin", "Gbm", "11A"), ("gbminor", "Gbm", "11A"),
        ("g#m", "G#m", "1A"),  ("g#min", "G#m", "1A"),  ("g#minor", "G#m", "1A"),  ("gsm", "G#m", "1A"),
        ("abm", "Abm", "1A"),  ("abmin", "Abm", "1A"),  ("abminor", "Abm", "1A"),
        ("a#m", "A#m", "3A"),  ("a#min", "A#m", "3A"),  ("a#minor", "A#m", "3A"),  ("asm", "A#m", "3A"),
        ("bbm", "Bbm", "3A"),  ("bbmin", "Bbm", "3A"),  ("bbminor", "Bbm", "3A"),
        ("cm", "Cm", "5A"),    ("cmin", "Cm", "5A"),    ("cminor", "Cm", "5A"),
        ("dm", "Dm", "7A"),    ("dmin", "Dm", "7A"),    ("dminor", "Dm", "7A"),
        ("em", "Em", "9A"),    ("emin", "Em", "9A"),    ("eminor", "Em", "9A"),
        ("fm", "Fm", "4A"),    ("fmin", "Fm", "4A"),    ("fminor", "Fm", "4A"),
        ("gm", "Gm", "6A"),    ("gmin", "Gm", "6A"),    ("gminor", "Gm", "6A"),
        ("am", "Am", "8A"),    ("amin", "Am", "8A"),    ("aminor", "Am", "8A"),
        ("bm", "Bm", "10A"),   ("bmin", "Bm", "10A"),   ("bminor", "Bm", "10A"),

        // Major keys (explicit accidentals or explicit maj)
        ("c#", "C#", "3B"),    ("c#maj", "C#", "3B"),   ("c#major", "C#", "3B"),    ("cs", "C#", "3B"),
        ("db", "Db", "3B"),    ("dbmaj", "Db", "3B"),   ("dbmajor", "Db", "3B"),
        ("d#", "D#", "5B"),    ("d#maj", "D#", "5B"),   ("d#major", "D#", "5B"),    ("ds", "D#", "5B"),
        ("eb", "Eb", "5B"),    ("ebmaj", "Eb", "5B"),   ("ebmajor", "Eb", "5B"),
        ("f#", "F#", "2B"),    ("f#maj", "F#", "2B"),   ("f#major", "F#", "2B"),    ("fs", "F#", "2B"),
        ("gb", "Gb", "2B"),    ("gbmaj", "Gb", "2B"),   ("gbmajor", "Gb", "2B"),
        ("g#", "G#", "4B"),    ("g#maj", "G#", "4B"),   ("g#major", "G#", "4B"),    ("gs", "G#", "4B"),
        ("ab", "Ab", "4B"),    ("abmaj", "Ab", "4B"),   ("abmajor", "Ab", "4B"),
        ("a#", "A#", "6B"),    ("a#maj", "A#", "6B"),   ("a#major", "A#", "6B"),    ("as", "A#", "6B"),
        ("bb", "Bb", "6B"),    ("bbmaj", "Bb", "6B"),   ("bbmajor", "Bb", "6B"),
        ("cmaj", "C", "8B"),   ("cmajor", "C", "8B"),
        ("dmaj", "D", "10B"),  ("dmajor", "D", "10B"),
        ("emaj", "E", "12B"),  ("emajor", "E", "12B"),
        ("fmaj", "F", "7B"),   ("fmajor", "F", "7B"),
        ("gmaj", "G", "9B"),   ("gmajor", "G", "9B"),
        ("amaj", "A", "11B"),  ("amajor", "A", "11B"),
        ("bmaj", "B", "1B"),   ("bmajor", "B", "1B"),
    ];

    for &(pat, canon, cam) in KEY_PATTERNS {
        if lower == pat {
            return Some((canon.to_string(), cam.to_string()));
        }
    }

    None
}

/// Determine whether a sample is a loop (true) or one-shot (false).
pub fn extract_is_loop(name: &str, rel_path: &str, has_bpm: bool) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let lower_path = rel_path.to_ascii_lowercase();

    // Explicit one-shot indicators take precedence
    if lower_name.contains("oneshot")
        || lower_name.contains("one-shot")
        || lower_name.contains("one_shot")
        || lower_name.contains("shot")
        || lower_name.contains("hit")
        || lower_name.contains("stab")
        || lower_path.contains("one-shot")
        || lower_path.contains("oneshot")
    {
        return false;
    }

    // Explicit loop indicators
    if lower_name.contains("loop")
        || lower_path.contains("loop")
        || lower_name.contains("groove")
        || lower_path.contains("groove")
    {
        return true;
    }

    // Drum types without loop words are usually one-shots
    if lower_name.contains("kick")
        || lower_name.contains("snare")
        || lower_name.contains("clap")
        || lower_name.contains("tom")
        || lower_name.contains("perc")
        || lower_name.contains("rim")
    {
        return false;
    }

    // If BPM is detected, it's typically a loop or tempo-synced element
    if has_bpm {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bpm_extraction() {
        assert_eq!(extract_bpm("synth_pluck_lead_140bpm.wav", "synth_pluck_lead_140bpm.wav"), Some(140.0));
        assert_eq!(extract_bpm("FB_TOP_LOOP_1_155bpm.wav", "Loops/FB_TOP_LOOP_1_155bpm.wav"), Some(155.0));
        assert_eq!(extract_bpm("Perc 14.wav", "percs/Perc 14.wav"), None);
        assert_eq!(extract_bpm("808_sub_kick_c.wav", "808_sub_kick_c.wav"), None);
        assert_eq!(extract_bpm("loop_126.5bpm.wav", "loop_126.5bpm.wav"), Some(126.5));
        assert_eq!(extract_bpm("Beat_120_Bpm.wav", "Drums/Beat_120_Bpm.wav"), Some(120.0));
    }

    #[test]
    fn test_key_extraction() {
        assert_eq!(extract_key("pluck_melody_C#m_120bpm.wav", ""), Some(("C#m".into(), "12A".into())));
        assert_eq!(extract_key("rhodes_chord_Am_loop.wav", ""), Some(("Am".into(), "8A".into())));
        assert_eq!(extract_key("bass_hit_F#_shot.wav", ""), Some(("F#".into(), "2B".into())));
        assert_eq!(extract_key("vocal_chop_8A_128.wav", ""), Some(("Am".into(), "8A".into())));
        assert_eq!(extract_key("pad_5B_90bpm.wav", ""), Some(("Eb".into(), "5B".into())));
        assert_eq!(extract_key("guitar_loop_Dmin_110bpm.wav", ""), Some(("Dm".into(), "7A".into())));
        assert_eq!(extract_key("lead_Fmaj_130bpm.wav", ""), Some(("F".into(), "7B".into())));
        assert_eq!(extract_key("808_sub_kick_c.wav", ""), Some(("C".into(), "8B".into())));
        assert_eq!(extract_key("Perc 14.wav", ""), None);
    }

    #[test]
    fn test_loop_classification() {
        assert!(extract_is_loop("FB_TOP_LOOP_1_155bpm.wav", "Loops/FB_TOP_LOOP_1_155bpm.wav", true));
        assert!(extract_is_loop("rhodes_chord_Am_loop.wav", "", false));
        assert!(!extract_is_loop("snare_tight_snap.wav", "", false));
        assert!(!extract_is_loop("kick_punchy_acoustic.wav", "", false));
        assert!(!extract_is_loop("bass_hit_F#_shot.wav", "", false));
    }
}
