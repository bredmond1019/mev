//! The voice tripwire's banned-phrase list (Phase 12, Block C, Task 1).
//!
//! Loads `data/voice-tells.toml` — the data-driven phrase list `learn-ai/CLAUDE.md`'s "Voice
//! and tone" bullet names — into a typed [`VoiceTell`] list. Same shape as
//! [`crate::funnel`]'s `CtaVocabulary`: a `parse`/`load_default` pair backed by an
//! `include_str!`-embedded default, plus a `parse`-only path for tests to prove the list is
//! genuinely data-driven (an override list a test supplies works exactly like the shipped one).
//!
//! This module owns the data file and its loader only. The scanner that turns a phrase list
//! into `W_VOICE_TELL` diagnostics is `MV.12.C` Task 2's `voice.rs`.

use std::sync::OnceLock;

/// One banned phrase from the voice tripwire's data file, plus a note naming where the rule
/// comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceTell {
    /// The literal phrase to match — never a regex (see this module's doc comment: a bad
    /// regex from a content edit must never be able to panic or hang the validator).
    pub phrase: String,
    /// One line naming the rule's source, e.g. an `learn-ai/CLAUDE.md` bullet.
    pub note: String,
}

/// Typed error for a malformed voice-tells data file — never a panic on bad *input*. (The
/// shipped default file is trusted at compile time; see [`default_tells`].)
#[derive(Debug, thiserror::Error)]
pub enum VoiceTellError {
    #[error("malformed voice-tells TOML: {0}")]
    Toml(#[from] toml::de::Error),
}

#[derive(Debug, serde::Deserialize)]
struct RawVoiceTells {
    tells: Vec<RawVoiceTell>,
}

#[derive(Debug, serde::Deserialize)]
struct RawVoiceTell {
    phrase: String,
    note: String,
}

/// Parse a voice-tells list from raw TOML text — the override path tests use to prove the
/// list is genuinely data-driven rather than hardcoded.
pub fn parse(toml_source: &str) -> Result<Vec<VoiceTell>, VoiceTellError> {
    let raw: RawVoiceTells = toml::from_str(toml_source)?;
    Ok(raw
        .tells
        .into_iter()
        .map(|t| VoiceTell {
            phrase: t.phrase,
            note: t.note,
        })
        .collect())
}

/// Load the shipped default voice-tells list, embedded into the binary at compile time from
/// `data/voice-tells.toml`.
pub fn load_default() -> Result<Vec<VoiceTell>, VoiceTellError> {
    parse(DEFAULT_VOICE_TELLS_TOML)
}

const DEFAULT_VOICE_TELLS_TOML: &str = include_str!("../data/voice-tells.toml");

/// The shipped voice-tells list, parsed once and cached. `expect` is safe here: this is our
/// own compile-time-embedded file, not runtime input — a malformed shipped file is a build
/// defect that must fail loudly, not degrade silently at every call site. Runtime-supplied
/// lists should go through [`parse`] directly, which returns a typed error instead.
pub fn default_tells() -> &'static [VoiceTell] {
    static TELLS: OnceLock<Vec<VoiceTell>> = OnceLock::new();
    TELLS.get_or_init(|| {
        load_default().expect("data/voice-tells.toml (embedded at compile time) must be valid TOML")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shipped_list_parses_and_contains_the_three_seeded_phrases() {
        let tells = default_tells();
        let phrases: Vec<&str> = tells.iter().map(|t| t.phrase.as_str()).collect();
        assert!(
            phrases.contains(&"production-ready"),
            "expected 'production-ready' in shipped list, got {phrases:?}"
        );
        assert!(
            phrases.contains(&"game-changing"),
            "expected 'game-changing' in shipped list, got {phrases:?}"
        );
        assert!(
            phrases.contains(&"actually bites you"),
            "expected 'actually bites you' in shipped list, got {phrases:?}"
        );
        assert_eq!(
            tells.len(),
            3,
            "seed list must stay exactly the three explicit phrases, got {tells:?}"
        );
    }

    #[test]
    fn every_seeded_phrase_has_a_source_note() {
        for tell in default_tells() {
            assert!(
                !tell.note.trim().is_empty(),
                "phrase {:?} is missing a source note",
                tell.phrase
            );
        }
    }

    #[test]
    fn malformed_data_file_returns_typed_error_not_panic() {
        let result = parse("this is not valid toml at all [[[");
        assert!(
            matches!(result, Err(VoiceTellError::Toml(_))),
            "expected VoiceTellError::Toml, got {result:?}"
        );
    }

    #[test]
    fn missing_tells_key_returns_typed_error_not_panic() {
        let result = parse("not_tells = []");
        assert!(
            result.is_err(),
            "expected an error for a TOML doc missing the `tells` key, got {result:?}"
        );
    }

    #[test]
    fn override_path_loads_a_custom_list() {
        let custom = r#"
            [[tells]]
            phrase = "totally custom phrase"
            note = "override test"
        "#;
        let tells = parse(custom).expect("valid override TOML must parse");
        assert_eq!(tells.len(), 1);
        assert_eq!(tells[0].phrase, "totally custom phrase");
        assert_eq!(tells[0].note, "override test");
    }
}
