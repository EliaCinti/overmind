//! Credentials known by value (ADR-0051 decision 6).
//!
//! The sign-in's scrubber recognised a secret by the literal `sk-ant`, so a
//! credential from any vendor that does not share Anthropic's prefix was
//! neither scraped nor -- the dangerous half -- redacted. Every credential
//! Overmind holds or was started with is registered here by its exact value,
//! and [`scrub`] blots out every occurrence, whatever it looks like.
//!
//! The shape rule stays beside it, for the one moment a credential exists and
//! is not yet known: the sign-in transcript, where the CLI prints a token
//! before anybody has scraped it.

use std::sync::RwLock;

/// Every credential registered so far. Process-wide rather than per instance:
/// a value known to one is not less secret in another, and the scrubber is
/// called from places that hold no state at all.
static KNOWN: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Shorter than this is not registered: a value that short would blot out
/// ordinary words, and no credential any provider issues is anywhere near it.
const MIN_LEN: usize = 12;

/// What a blotted-out credential reads as.
const REDACTED: &str = "…[redacted]";

/// Remember a credential so it is blotted out wherever it appears.
pub fn register(value: &str) {
    let value = value.trim();
    if value.chars().count() < MIN_LEN {
        return;
    }
    let Ok(mut known) = KNOWN.write() else {
        return;
    };
    if !known.iter().any(|k| k == value) {
        known.push(value.to_string());
        // Longest first, so a value that contains another is replaced whole
        // rather than leaving the remainder of itself behind.
        known.sort_by_key(|k| std::cmp::Reverse(k.len()));
    }
}

/// Text fit to log or show: every registered credential removed, and anything
/// shaped like an Anthropic token as well.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    if let Ok(known) = KNOWN.read() {
        for value in known.iter() {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), REDACTED);
            }
        }
    }
    by_shape(&out)
}

/// Blot out anything shaped like an Anthropic credential.
///
/// Learned the hard way (27 Aug 2026): "its last words" on a failed token
/// scrape carried the token itself — the one the scraper had not recognized
/// — into `docker compose logs` and a pasted bug report. Anything shaped
/// like a long `sk-ant-…` run is replaced before the text leaves; a lost
/// character or an unknown subtype must not defeat it, so the match is the
/// shape, not an exact prefix list.
fn by_shape(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let is_tok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        // A run of token characters containing "sk-ant" and long enough to
        // be a credential is not for anyone's eyes.
        if is_tok(chars[i]) {
            let start = i;
            while i < chars.len() && is_tok(chars[i]) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            // How far a credential runs from here, following it across the
            // whitespace a TUI redraw inserts. `strip_ansi` turns a carriage
            // return mid-token into a newline, so the marker can land in a run
            // far shorter than a secret while the body sits in the next one --
            // and demanding length of THIS run alone printed the prefix, then
            // printed the secret as a run with no marker in it.
            // Bounded: a credential torn by a redraw is one or two pieces, not
            // fifty. Without a cap, one `sk-ant` followed by a run of base64-ish
            // lines would swallow all of them — and this text exists so somebody
            // can read what went wrong, so a scrubber that eats the diagnostic
            // has traded one failure for another.
            const MAX_PIECES: usize = 4;
            let mut end = i;
            for _ in 0..MAX_PIECES {
                let mut j = end;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let body = j;
                while j < chars.len() && is_tok(chars[j]) {
                    j += 1;
                }
                // A body has no spaces and is long; prose after a prefix is
                // neither, which is what keeps "set sk-ant-… as the key" whole.
                if j - body > 20 {
                    end = j;
                } else {
                    break;
                }
            }
            // Long enough to hold a secret, or trailed by something that is.
            // `sk-ant-` alone is seven characters and a sentence, not a token.
            if run.contains("sk-ant") && (run.len() > 10 || end > i) {
                out.push_str("sk-ant-…[redacted]");
                i = end;
            } else {
                out.push_str(&run);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{register, scrub};

    /// A registered value goes wherever it appears, shaped like anything.
    #[test]
    fn a_registered_value_is_blotted_out_whatever_its_shape() {
        let odd = "xai-0f9e8d7c6b5a49382716-unit";
        register(odd);
        let shown = scrub(&format!("refused {odd} twice: {odd}"));
        assert!(!shown.contains(odd), "{shown}");
        assert_eq!(shown, "refused …[redacted] twice: …[redacted]");
    }

    /// A value too short to be a credential is not one: registering it must
    /// not eat ordinary words out of every log line.
    #[test]
    fn a_short_value_is_not_registered() {
        register("error");
        assert_eq!(scrub("an error occurred"), "an error occurred");
    }
}
