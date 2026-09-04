//! Secret scrubbing and prompt truncation (design doc §15). Runs at the
//! normalizer boundary, before anything reaches the event log — the on-disk
//! log must never contain a secret to leak later.

use regex::Regex;
use std::sync::OnceLock;

const PROMPT_TRUNCATE_LEN: usize = 200;
const LONG_STRING_THRESHOLD: usize = 300;

struct SecretPatterns {
    patterns: Vec<Regex>,
}

fn secret_patterns() -> &'static SecretPatterns {
    static CELL: OnceLock<SecretPatterns> = OnceLock::new();
    CELL.get_or_init(|| SecretPatterns {
        patterns: vec![
            // Common API key / token prefixes.
            Regex::new(r"sk-[A-Za-z0-9_\-]{10,}").unwrap(),
            Regex::new(r"ghp_[A-Za-z0-9]{20,}").unwrap(),
            Regex::new(r"gho_[A-Za-z0-9]{20,}").unwrap(),
            Regex::new(r"github_pat_[A-Za-z0-9_]{20,}").unwrap(),
            Regex::new(r"(?i)bearer\s+[A-Za-z0-9._\-]{15,}").unwrap(),
            Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
            Regex::new(r"FlyV1\s+[A-Za-z0-9_,=./\-]{20,}").unwrap(),
            // Absolute home paths — not a secret per se, but leaks machine layout.
            Regex::new(r"/(root|home/[A-Za-z0-9_\-]+)(/[A-Za-z0-9_\-./]+)*").unwrap(),
            // Emails.
            Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap(),
            // Long high-entropy-looking tokens (hex/base64-ish, 32+ chars, no spaces).
            Regex::new(r"\b[A-Za-z0-9_\-]{32,}\b").unwrap(),
        ],
    })
}

/// Redact known secret shapes inside a string, replacing each match with a
/// fixed-width marker so length changes don't themselves leak information.
pub fn scrub_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for re in &secret_patterns().patterns {
        out = re.replace_all(&out, "[REDACTED-SECRET]").into_owned();
    }
    out
}

/// Truncate a routine prompt body. Per §15, routine prompts must never render
/// raw regardless of length — unlike `redact_field`, this ALWAYS appends the
/// redaction marker, even when the prompt is already shorter than the cap.
pub fn redact_routine_prompt(prompt: &str) -> String {
    let scrubbed = scrub_secrets(prompt);
    let total = scrubbed.chars().count();
    let truncated: String = scrubbed.chars().take(PROMPT_TRUNCATE_LEN).collect();
    format!("{truncated}...[REDACTED-LEN={total}]")
}

/// General-purpose field redactor for anything entering the event log or a
/// fixture snapshot: scrub secrets, then truncate anything long enough to be
/// a transcript/prompt body rather than a short label.
pub fn redact_field(value: &str) -> String {
    let scrubbed = scrub_secrets(value);
    if scrubbed.chars().count() > LONG_STRING_THRESHOLD {
        truncate_marked(&scrubbed, PROMPT_TRUNCATE_LEN)
    } else {
        scrubbed
    }
}

fn truncate_marked(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}...[REDACTED-LEN={total}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_api_key_prefixes() {
        let input = "token=sk-abcdefghij1234567890 rest of text";
        let out = scrub_secrets(input);
        assert!(!out.contains("sk-abcdefghij1234567890"));
        assert!(out.contains("[REDACTED-SECRET]"));
    }

    #[test]
    fn scrubs_github_tokens() {
        let out = scrub_secrets("ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(!out.contains("ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    }

    #[test]
    fn scrubs_bearer_tokens() {
        let out = scrub_secrets("Authorization: Bearer abc123.def456.ghi789xyz");
        assert!(!out.to_lowercase().contains("bearer abc123"));
    }

    #[test]
    fn scrubs_emails() {
        let out = scrub_secrets("contact breydenerrett@gmail.com for access");
        assert!(!out.contains("breydenerrett@gmail.com"));
    }

    #[test]
    fn scrubs_absolute_home_paths() {
        let out = scrub_secrets("token in scratchpad/.fly_token at /root/.fly/bin/flyctl");
        assert!(!out.contains("/root/.fly/bin/flyctl"));
    }

    #[test]
    fn routine_prompt_always_truncated_even_when_short() {
        let short = "run the daily loop";
        let out = redact_routine_prompt(short);
        assert!(out.contains("[REDACTED-LEN="));
        assert_ne!(out, short);
    }

    #[test]
    fn short_field_untouched_when_no_secret() {
        let short = "SPORTS LAB";
        assert_eq!(redact_field(short), short);
    }

    #[test]
    fn long_non_secret_field_still_truncated() {
        // Spaced-out words, not a single long token, so the high-entropy
        // secret pattern correctly does NOT fire here — this exercises the
        // separate "just long" truncation path, not secret scrubbing.
        let long = "the quick brown fox jumps over the lazy dog ".repeat(15);
        let total_chars = long.chars().count();
        let out = redact_field(&long);
        assert!(out.chars().count() < total_chars);
        assert!(out.contains(&format!("[REDACTED-LEN={total_chars}]")));
    }

    #[test]
    fn long_high_entropy_token_is_caught_as_a_secret_not_just_truncated() {
        // A long unbroken alnum run looks like a token/hash — scrub_secrets
        // should catch it as [REDACTED-SECRET], which is a stronger and
        // sufficient redaction than the generic length-truncation marker.
        let long = "a".repeat(500);
        let out = redact_field(&long);
        assert!(!out.contains(&"a".repeat(32)));
        assert!(out.contains("[REDACTED-SECRET]"));
    }
}
