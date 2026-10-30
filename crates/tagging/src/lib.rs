//! crates/tagging — lexical tag sources (spec 4.1 items 1 and 2)
//! Rules are DATA (rules.default.toml) so users can edit them. Matching is
//! exact-token only: 'clap' never fires inside 'clapperboard'. The engine
//! emits Suggestions; persistence + resurrection rules live in scout-db.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    FilenameRule,
    FolderRule,
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub tag: String,
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
        Self { enabled: true, weight: 0.35, max_depth: 2 }
    }
}

#[derive(Debug, Deserialize)]
pub struct Rule {
    pub tag: String,
    pub terms: Vec<String>,
    pub weight: f32,
}

pub struct Engine {
    pub ruleset: RuleSet,
    term_index: HashMap<String, Vec<usize>>, // term -> rule indices
}

impl Engine {
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        let rs: RuleSet = toml::from_str(text)?;
        let mut term_index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in rs.rules.iter().enumerate() {
            for t in &r.terms {
                term_index.entry(t.to_ascii_lowercase()).or_default().push(i);
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
            let parents: Vec<&str> = rel_path.split('/').collect();
            for (depth, dir) in parents
                .iter()
                .rev()
                .skip(1) // [0] is the filename's own rel path entry handled above
                .take(self.ruleset.folder.max_depth)
                .enumerate()
            {
                let depth_factor = 1.0 / (depth as f32 + 1.0); // parent > grandparent
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
        for tok in tokens {
            if is_noise(tok) {
                continue;
            }
            if let Some(indices) = self.term_index.get(tok) {
                for &i in indices {
                    let r = &self.ruleset.rules[i];
                    let score = (r.weight * weight).min(1.0);
                    found
                        .entry(r.tag.clone())
                        .and_modify(|s| s.score = s.score.max(score))
                        .or_insert_with(|| Suggestion {
                            tag: r.tag.clone(),
                            source: source.clone(),
                            score,
                            detail: format!("{}@{}", r.tag, tok),
                        });
                }
            }
        }
        found.into_values().collect()
    }
}

/// Guards: pure numbers (01, 808, 124bpm) are legal tokens but too weak to
/// tag on their own; single letters likewise.
fn is_noise(tok: &str) -> bool {
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
}
