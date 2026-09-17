//! Taking credentials out of text an agent wrote.
//!
//! # Why this exists
//!
//! A help request is where an agent explains why it could not do something, and the most common
//! reason by a wide margin is that a credential did not work. So the field most likely to contain
//! a secret is the one whose entire purpose is to describe a failed authentication — and unlike a
//! transcript, it is persisted as JSON Lines, served over an unauthenticated HTTP API, and
//! rendered in a dashboard.
//!
//! The agent is not being careless when it pastes the token it tried. It is being helpful.
//!
//! # What this is not
//!
//! This is not a guarantee, and nothing downstream should treat it as one. Secrets have no
//! grammar, and a redactor that claimed to catch all of them would be worse than none, because it
//! would license writing them down. What it does is catch the shapes that are cheap to recognise
//! and common enough to be worth the occasional false positive.
//!
//! The durable protection is that credentials are named in `env_from` and read from the
//! environment, never written into config. This is the second line, not the first.

use std::borrow::Cow;

/// Longest a detail may be once redacted.
///
/// A help request is a summary for a human deciding what to do, not a log. Something arbitrarily
/// long is both a weight on the journal and a sign that a transcript was pasted in — which is
/// exactly where an unredacted secret would be hiding.
pub const MAX_DETAIL: usize = 4_000;

/// What replaces a redacted run.
const MASK: &str = "[redacted]";

/// Rewrites `text` with anything credential-shaped masked.
///
/// Borrows the original when nothing matched, which is the overwhelmingly common case.
#[must_use]
pub fn secrets(text: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut rest = text;
    let mut changed = false;

    while let Some((start, len)) = find_secret(rest) {
        changed = true;
        out.push_str(&rest[..start]);
        out.push_str(MASK);
        rest = &rest[start + len..];
    }

    if !changed {
        return Cow::Borrowed(text);
    }

    out.push_str(rest);
    Cow::Owned(out)
}

/// Caps `text` at [`MAX_DETAIL`], on a character boundary, saying that it did.
#[must_use]
pub fn clamp(text: &str) -> Cow<'_, str> {
    if text.len() <= MAX_DETAIL {
        return Cow::Borrowed(text);
    }

    let mut end = MAX_DETAIL;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }

    Cow::Owned(format!("{}\n\n[truncated]", &text[..end]))
}

/// Redacts, then caps. The order matters: cutting first could halve a token and leave the front
/// of it sitting in the text looking like prose.
#[must_use]
pub fn detail(text: &str) -> String {
    clamp(&secrets(text)).into_owned()
}

/// Offset and length of the next credential-shaped run.
fn find_secret(text: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();

    for (index, _) in text.char_indices() {
        // Only consider the start of a run, so a token is examined once rather than per character.
        if index > 0 && is_token_byte(bytes[index - 1]) {
            continue;
        }

        let run = token_run(text, index);
        if run == 0 {
            continue;
        }

        // Assignment first. `=` is a token byte because base64 pads with it, which means
        // `NAME=value` scans as a single run — and masking that whole run would take the name
        // with it. Knowing *which* credential failed is the entire value of the report.
        if let Some(found) = assignment(text, index) {
            return Some(found);
        }

        if looks_like_secret(&text[index..index + run]) {
            return Some((index, run));
        }
    }

    None
}

/// Length of the run of characters that can make up a variable name.
///
/// Narrower than [`token_run`] on purpose: a name stops at the `=` or `:` that follows it.
fn name_run(text: &str, start: usize) -> usize {
    text[start..]
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count()
}

fn token_run(text: &str, start: usize) -> usize {
    text[start..]
        .bytes()
        .take_while(|b| is_token_byte(*b))
        .count()
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'+' | b'/' | b'=' | b'~')
}

/// Whether a bare token is credential-shaped on its own.
fn looks_like_secret(token: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "sk-",
        "pk-",
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "xoxb-",
        "xoxp-",
    ];
    if token.len() > 12 && PREFIXES.iter().any(|p| token.starts_with(p)) {
        return true;
    }

    // A JWT: three base64url segments, the first announcing a JSON header.
    if token.starts_with("eyJ") && token.matches('.').count() == 2 {
        return true;
    }

    // A long high-entropy run. Conservative on purpose: prose does not produce thirty-two
    // character runs mixing cases and digits, and paths are excluded by the separator check.
    token.len() >= 32 && !token.contains('/') && mixed_enough(token)
}

/// Whether a run has the character mix of a key rather than of a word.
fn mixed_enough(token: &str) -> bool {
    let digits = token.bytes().filter(u8::is_ascii_digit).count();
    let upper = token.bytes().filter(u8::is_ascii_uppercase).count();
    let lower = token.bytes().filter(u8::is_ascii_lowercase).count();

    // A hex digest has only digits and one case, so it would fail the three-class test that
    // catches everything else. A commit hash masked in error is a fair price; the reverse
    // mistake cannot be undone once it is on disk.
    if digits >= 8 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }

    digits > 0 && upper > 0 && lower > 0
}

/// Matches `NAME=value` / `NAME: value` where `NAME` names a credential, returning the span of
/// the **value** alone.
fn assignment(text: &str, start: usize) -> Option<(usize, usize)> {
    let name_len = name_run(text, start);
    if name_len == 0 || !names_a_secret(&text[start..start + name_len]) {
        return None;
    }

    let bytes = text.as_bytes();
    let mut at = start + name_len;

    match bytes.get(at) {
        Some(b'=' | b':') => at += 1,
        _ => return None,
    }

    while bytes.get(at) == Some(&b' ') {
        at += 1;
    }

    let len = token_run(text, at);
    if len == 0 { None } else { Some((at, len)) }
}

/// Whether a variable name announces that it holds a credential.
fn names_a_secret(name: &str) -> bool {
    const MARKERS: [&str; 11] = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "APIKEY",
        "API_KEY",
        "_PAT",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "ACCESS_KEY",
        "AUTHORIZATION",
    ];

    let upper = name.to_ascii_uppercase();
    MARKERS.iter().any(|marker| upper.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_prose_is_left_alone_and_not_reallocated() {
        let text = "Push to refs/heads/fix/1543477 returned 401. The token expires after 30 days.";
        assert!(matches!(secrets(text), Cow::Borrowed(_)), "{text}");
    }

    #[test]
    fn a_github_token_is_masked_but_the_sentence_survives() {
        let clean = secrets("tried ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8 and got 401");

        assert!(!clean.contains("ghp_A1b2"), "{clean}");
        assert!(
            clean.contains("got 401"),
            "the useful part must survive: {clean}"
        );
    }

    #[test]
    fn an_openai_style_key_is_masked() {
        let clean = secrets("key sk-proj-abcdefghijklmnopqrstuvwxyz0123456789 rejected");
        assert!(!clean.contains("sk-proj-abcdef"), "{clean}");
    }

    #[test]
    fn a_jwt_is_masked() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let text = format!("token {jwt} expired");
        let clean = secrets(&text);

        assert!(!clean.contains("eyJhbGciOi"), "{clean}");
        assert!(clean.contains("expired"), "{clean}");
    }

    #[test]
    fn the_variable_name_survives_because_it_is_the_useful_part() {
        // Which credential failed *is* the report. Masking the name would leave a request saying
        // only that something, somewhere, was wrong.
        let clean = secrets("ADO_PAT=zq8Vv2Lm4Kp7Rt1Ns5Wx9Yb3Cd6Ef0Gh became invalid");

        assert!(clean.starts_with("ADO_PAT="), "{clean}");
        assert!(!clean.contains("zq8Vv2Lm"), "{clean}");
        assert!(clean.contains("became invalid"), "{clean}");
    }

    #[test]
    fn a_named_secret_is_masked_after_a_colon_too() {
        let clean = secrets("AZURE_CLIENT_SECRET: Abcd1234Efgh5678Ijkl");
        assert!(!clean.contains("Abcd1234"), "{clean}");
        assert!(clean.starts_with("AZURE_CLIENT_SECRET:"), "{clean}");
    }

    #[test]
    fn a_digest_is_masked_because_it_could_equally_be_a_key() {
        let clean = secrets("at 5f2e9c4b8a1d3e7f0c6b2a9d8e4f1c3b5a7d9e0f");
        assert!(clean.contains(MASK), "{clean}");
    }

    #[test]
    fn paths_and_urls_are_not_mistaken_for_keys() {
        for text in [
            "could not read prompts/analyst.md",
            "GET https://dev.azure.com/org/_apis/git/repositories returned 403",
            "the workspace is at /home/ada/work/layover-project",
        ] {
            assert!(
                matches!(secrets(text), Cow::Borrowed(_)),
                "should not have matched: {text}"
            );
        }
    }

    #[test]
    fn several_secrets_in_one_message_are_all_masked() {
        let clean = secrets(
            "tried ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8 then sk-abcdefghijklmnopqrstuvwxyz012345",
        );
        assert_eq!(clean.matches(MASK).count(), 2, "{clean}");
    }

    #[test]
    fn an_over_long_detail_is_cut_and_says_so() {
        let long = "a".repeat(MAX_DETAIL + 500);
        let capped = clamp(&long);

        assert!(capped.len() < long.len());
        assert!(
            capped.ends_with("[truncated]"),
            "a cut report has to admit it was cut"
        );
    }

    #[test]
    fn redaction_happens_before_the_cut() {
        let secret = "ghp_A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q7r8";
        let text = format!("{}{secret}", "x".repeat(MAX_DETAIL - 10));

        assert!(
            !detail(&text).contains("ghp_A1b2"),
            "the token survived being cut in half"
        );
    }
}
