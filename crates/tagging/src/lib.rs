//! crates/tagging — lexical tag sources (spec 4.1 items 1 and 2)
//! Rules are DATA (rules.default.toml) so users can edit them. Matching is
//! exact-token only: 'clap' never fires inside 'clapperboard'. The engine
//! emits Suggestions; persistence + resurrection rules live in scout-db.

use serde::Deserialize;
use std::collections::HashMap;

pub mod musical;
pub use musical::{extract_bpm, extract_is_loop, extract_key, extract_musical_info, MusicalInfo};

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    FilenameRule,
    FolderRule,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub tag: String,
    pub kind: String,
    pub source: Source,
    pub score: f32,   // raw rule weight, NOT a probability; UI maps to 0..1
    pub detail: String, // e.g. "kick@bd" — stored in file_tags.source_detail
}

#[derive(Debug, Deserialize)]
pub struct RuleSet {
    pub version: String,
    #[serde(default)]
    pub folder: FolderConfig,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Deserialize)]
pub struct FolderConfig {
    pub enabled: bool,
    pub weight: f32,      // base weight for folder-context hits
    pub max_depth: usize, // how many parents up we still trust
}

impl Default for FolderConfig {
    fn default() -> Self {
        Self { enabled: true, weight: 0.5, max_depth: 8 }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub tag: String,
    pub terms: Vec<String>,
    pub weight: f32,
    #[serde(default = "default_category")]
    pub kind: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub parent_kind: Option<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
}

fn default_category() -> String {
    "category".to_string()
}

pub struct Engine {
    pub ruleset: RuleSet,
    term_index: HashMap<String, Vec<usize>>, // normalized term -> rule indices
}

pub fn default_engine() -> &'static Engine {
    Engine::default_engine()
}

impl Engine {
    pub fn default_rules() -> Self {
        Self::from_toml(include_str!("../rules.default.toml")).expect("valid default rules.default.toml")
    }

    pub fn default_engine() -> &'static Engine {
        static ENGINE: std::sync::OnceLock<Engine> = std::sync::OnceLock::new();
        ENGINE.get_or_init(Self::default_rules)
    }

    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let rs: RuleSet = toml::from_str(text)?;
        let mut term_index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in rs.rules.iter().enumerate() {
            for t in &r.terms {
                let toks = tokenize(t);
                let key = if toks.is_empty() {
                    t.trim().to_ascii_lowercase()
                } else {
                    toks.join(" ")
                };
                if !key.is_empty() {
                    term_index.entry(key).or_default().push(i);
                }
            }
        }
        Ok(Self { ruleset: rs, term_index })
    }

    /// Filename + up to 'max_depth' parent folder names.
    pub fn suggest(&self, name: &str, rel_path: &str) -> Vec<Suggestion> {
        let mut out = Vec::new();
        for hit in self.match_tokens(&tokenize(name), 1.0, Source::FilenameRule) {
            out.push(hit);
        }
        if self.ruleset.folder.enabled {
            let parents: Vec<&str> = rel_path
                .split(|c| c == '/' || c == '\\')
                .filter(|s| !s.is_empty())
                .collect();
            for (depth, dir) in parents
                .iter()
                .rev()
                .skip(1) // [0] is the filename's own rel path entry handled above
                .take(self.ruleset.folder.max_depth)
                .enumerate()
            {
                let depth_factor = 1.0 / (depth as f32 * 0.3 + 1.0); // smooth parent decay
                let w = self.ruleset.folder.weight * depth_factor;
                for hit in self.match_tokens(&tokenize(dir), w, Source::FolderRule) {
                    out.push(hit);
                }
            }
        }
        out
    }

    fn match_tokens(&self, tokens: &[String], weight: f32, source: Source) -> Vec<Suggestion> {
        let mut found: HashMap<String, Suggestion> = HashMap::new();
        let clean_tokens: Vec<&str> = tokens.iter().filter(|t| !is_noise(t)).map(|s| s.as_str()).collect();

        // Generate 1-grams, 2-grams, and 3-grams
        let mut ngrams: Vec<(String, String)> = Vec::new(); // (ngram_key, original_display)
        let n = clean_tokens.len();
        for len in 1..=3.min(n) {
            for start in 0..=n - len {
                let slice = &clean_tokens[start..start + len];
                let joined = slice.join(" ");
                ngrams.push((joined.clone(), joined));
            }
        }

        // Also build token set for fast excludes lookup
        let token_set: std::collections::HashSet<String> = clean_tokens.iter().map(|&s| s.to_string()).collect();

        for (key, display) in &ngrams {
            if let Some(indices) = self.term_index.get(key) {
                for &i in indices {
                    let r = &self.ruleset.rules[i];

                    // Check exclude filters (e.g. dubstep excludes dub, cymatics excludes cymbal)
                    let excluded = r.excludes.iter().any(|ex| {
                        let ex_toks = tokenize(ex);
                        let ex_key = ex_toks.join(" ");
                        token_set.contains(&ex_key) || ngrams.iter().any(|(k, _)| k == &ex_key)
                    });
                    if excluded {
                        continue;
                    }

                    let score = (r.weight * weight).min(1.0);
                    found
                        .entry(r.tag.clone())
                        .and_modify(|s| {
                            if score > s.score {
                                s.score = score;
                                s.detail = format!("{}@{}", r.tag, display);
                            }
                        })
                        .or_insert_with(|| Suggestion {
                            tag: r.tag.clone(),
                            kind: r.kind.clone(),
                            source: source.clone(),
                            score,
                            detail: format!("{}@{}", r.tag, display),
                        });

                    // Parent category tag inheritance (e.g. snare -> drums, fm -> synth)
                    if let Some(parent_tag) = &r.parent {
                        let parent_score = (score * 0.9).min(1.0);
                        let parent_kind = r.parent_kind.clone().unwrap_or_else(|| "category".to_string());
                        found
                            .entry(parent_tag.clone())
                            .and_modify(|s| {
                                if parent_score > s.score {
                                    s.score = parent_score;
                                }
                            })
                            .or_insert_with(|| Suggestion {
                                tag: parent_tag.clone(),
                                kind: parent_kind,
                                source: source.clone(),
                                score: parent_score,
                                detail: format!("{}^{}", parent_tag, r.tag),
                            });
                    }
                }
            }
        }
        found.into_values().collect()
    }
}

/// Guards: single letters or generic pure numbers are noise, EXCEPT iconic music gear
/// numbers and synth models like '808', '909', '303', '707', 'cr78', 'mpc', 'dx7', 'fm8'.
fn is_noise(tok: &str) -> bool {
    let lower = tok.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "808" | "909" | "303" | "707" | "cr78" | "mpc" | "dx7" | "fm8" | "ms20" | "ms10"
    ) {
        return false;
    }
    tok.len() < 2 || tok.chars().all(|c| c.is_ascii_digit())
}

/// 'kick_01' -> [kick, 01] · 'BD-Deep' -> [bd, deep] · 'OpenHat' -> [open, hat]
/// 'hh909' -> [hh, 909] · 'G2_BASS_120bpm' -> [g, 2, bass, 120, bpm]
pub fn tokenize(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut prev: Option<char> = None;
    let flush = |buf: &mut String, out: &mut Vec<String>| {
        if !buf.is_empty() {
            out.push(buf.to_ascii_lowercase());
            buf.clear();
        }
    };
    for c in input.chars() {
        let is_sep = matches!(
            c,
            '_' | '-' | '.' | ' ' | '/' | '(' | ')' | '[' | ']' | '{' | '}' | '+' | ',' | '|'
                | '&' | '\'' | '"' | '~' | '!' | '='
        );
        if is_sep {
            flush(&mut buf, &mut out);
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let camel = p.is_lowercase() && c.is_uppercase();
            let digit_edge = p.is_ascii_digit() != c.is_ascii_digit();
            if camel || digit_edge {
                flush(&mut buf, &mut out);
            }
        }
        buf.push(c);
        prev = Some(c);
    }
    flush(&mut buf, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenizer_shapes() {
        assert_eq!(tokenize("kick_01.wav"), ["kick", "01", "wav"]);
        assert_eq!(tokenize("BD-Deep.wav"), ["bd", "deep", "wav"]);
        assert_eq!(tokenize("OpenHat A.wav"), ["open", "hat", "a", "wav"]);
        assert_eq!(tokenize("hh909.flac"), ["hh", "909", "flac"]);
        assert_eq!(tokenize("G2_BASS_120bpm.wav"), ["g", "2", "bass", "120", "bpm", "wav"]);
    }

    #[test]
    fn no_substring_false_positives() {
        let e = Engine::from_toml(include_str!("../rules.default.toml")).unwrap();
        // 'clap' must not fire inside 'clapperboard.wav'
        let s = e.suggest("clapperboard.wav", "SFX/clapperboard.wav");
        assert!(s.iter().all(|x| x.tag != "clap"));
    }

    #[test]
    fn multiword_and_genres() {
        let e = Engine::from_toml(include_str!("../rules.default.toml")).unwrap();
        let s = e.suggest("Lead.wav", "UK Garage/Lead.wav");
        assert!(s.iter().any(|x| x.tag == "uk garage" && x.kind == "genre"));

        let s2 = e.suggest("Wobble.wav", "Drum & Bass/Wobble.wav");
        assert!(s2.iter().any(|x| x.tag == "drum & bass" && x.kind == "genre"));

        let s3 = e.suggest("Lead - Future Bass.wav", "Packs/Lead - Future Bass.wav");
        assert!(s3.iter().any(|x| x.tag == "future bass" && x.kind == "genre"));
    }

    #[test]
    fn parent_inheritance_and_excludes() {
        let e = Engine::from_toml(include_str!("../rules.default.toml")).unwrap();
        // Snare matches instrument 'snare' AND inherits parent category 'drums'
        let s = e.suggest("Threat_Snare_01.wav", "Sample Packs/!Dubstep/!THREAT/Drums/Snare/Threat_Snare_01.wav");
        assert!(s.iter().any(|x| x.tag == "snare" && x.kind == "instrument"));
        assert!(s.iter().any(|x| x.tag == "drums" && x.kind == "category"));
        assert!(s.iter().any(|x| x.tag == "dubstep" && x.kind == "genre"));

        // 'dub' must be excluded by 'dubstep'
        assert!(s.iter().all(|x| x.tag != "dub"));
    }
}
