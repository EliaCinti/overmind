//! M16: the company's language, and how it reaches the agents.
//!
//! What you read on screen comes from three places, and only one of them is a
//! dictionary in the frontend:
//!
//! 1. **UI chrome** — buttons, labels, empty states. A dictionary, in the web app.
//! 2. **Agent output** — chat replies, meeting transcripts, the reasoning behind
//!    a proposed team. That is the model's language, and this module is what
//!    governs it: one line appended to every prompt.
//! 3. **Server-composed prose** — notification bodies, approval summaries.
//!    Still English; the fix is to make them structured rather than to translate
//!    them here (see the roadmap, M16 slice D).
//!
//! Keeping the setting on the company rather than the browser is what makes (2)
//! possible at all: a per-tab preference cannot tell an agent what to write.

/// The languages the product offers, as `(code, endonym)`. The name is written
/// **in that language** — a speaker recognises "Italiano" faster than "Italian",
/// and a list of endonyms needs no translation of its own.
pub const SUPPORTED: &[(&str, &str)] = &[("en", "English"), ("it", "Italiano")];

/// The default for a company that never chose.
pub const DEFAULT: &str = "en";

/// Whether we know this code, so an unknown one can be refused rather than
/// silently stored and later fed into a prompt.
pub fn is_supported(code: &str) -> bool {
    SUPPORTED.iter().any(|(c, _)| *c == code)
}

/// The English name of a language, for use inside a prompt written in English.
fn english_name(code: &str) -> &'static str {
    match code {
        "it" => "Italian",
        _ => "English",
    }
}

/// The instruction appended to every agent prompt.
///
/// Emitted even for English: the point of a setting is that the outcome does
/// not depend on which language the last message happened to be written in.
pub fn prompt_line(code: &str) -> String {
    format!(
        "\n\nWrite everything you produce — replies, task descriptions, decisions, documents — in {}. This is the language the company works in, whatever language this prompt or the user's message is written in. Keep identifiers, code, file paths and archetype slugs unchanged.",
        english_name(code)
    )
}

/// The language a company works in. Falls back to the default rather than
/// failing: a missing row must never stop an agent from working.
pub async fn company_language(state: &crate::db::AppState, company_id: &str) -> String {
    sqlx::query_as::<_, (String,)>("SELECT language FROM companies WHERE id = ?")
        .bind(company_id)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        .map(|(l,)| l)
        .filter(|l| is_supported(l))
        .unwrap_or_else(|| DEFAULT.to_string())
}
