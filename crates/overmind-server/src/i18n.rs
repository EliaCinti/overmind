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

/// An approval's durable summary, in the company's language (M23, carried).
/// The inbox words notifications from kind+params and never reads these;
/// the approvals list does, and an Italian company used to read "Start …"
/// there. Server-composed prose is kept to these two sentences on purpose --
/// anything richer belongs in structured params the client can word.
pub fn start_task_summary(code: &str, title: &str) -> String {
    match code {
        "it" => format!("Avvia «{title}»"),
        _ => format!("Start \"{title}\""),
    }
}

/// The quiet chip a compaction leaves in the thread (ADR-0040).
pub fn chat_compacted_notice(code: &str, covered: usize) -> String {
    match code {
        "it" => format!(
            "Conversazione compattata: {covered} messaggi precedenti riassunti (e salvati in memoria). Il filo resta leggibile qui."
        ),
        _ => format!(
            "Conversation compacted: {covered} earlier messages summarized (and stored to memory). The thread stays readable here."
        ),
    }
}

/// What became of a start the CEO asked for, said by the server in its own
/// voice (ADR-0046). The CEO's words are written before the start is tried, so
/// it cannot report this itself — and reporting "running" for all five
/// outcomes is how a company sat at zero in progress for a day.
pub fn start_outcome(code: &str, title: &str, outcome: &str) -> String {
    match (code, outcome) {
        ("it", "no_such_task") => format!(
            "«{title}» non è partito: sulla lavagna non c'è nessun task aperto con questo titolo."
        ),
        ("it", "nobody_on_it") => {
            format!("«{title}» non è partito: il task c'è, ma non è incaricato a nessuno.")
        }
        ("it", "asked") => format!(
            "«{title}» non è ancora in corso: l'avvio è nella tua posta in arrivo e aspetta la tua approvazione."
        ),
        ("it", "waiting") => format!(
            "«{title}» non è partito da sé: chi lo ha in carico propone soltanto, quindi deve avviarlo una persona."
        ),
        ("it", "already_running") => {
            format!("«{title}» era già in corso: non l'ho fatto ripartire una seconda volta.")
        }
        ("it", _) => format!("«{title}» non è partito. Il motivo è nel log del server."),
        (_, "no_such_task") => {
            format!("\"{title}\" did not start: no open task on the board carries that title.")
        }
        (_, "nobody_on_it") => {
            format!("\"{title}\" did not start: the task is there, but nobody is on it.")
        }
        (_, "asked") => format!(
            "\"{title}\" is not running yet: the start is in your inbox, waiting for your approval."
        ),
        (_, "waiting") => format!(
            "\"{title}\" did not start on its own: whoever holds it only proposes, so a person has to start it."
        ),
        (_, "already_running") => {
            format!("\"{title}\" was already under way: I did not run it a second time.")
        }
        (_, _) => format!("\"{title}\" did not start. The reason is in the server's log."),
    }
}
/// A start the budget gate refused, with the two numbers and the remedy
/// (ADR-0046). The gate has always worked; it simply never said so to anyone
/// — six refusals on the owner's own company in two hours, recorded on the
/// chain and nowhere else.
pub fn start_over_budget(code: &str, title: &str, limit_cents: i64, observed_cents: i64) -> String {
    // `governance::euros`, like every other budget line in the codebase: nobody
    // should read cents in one refusal and euros in the next.
    let cap = crate::governance::euros(limit_cents);
    let spent = crate::governance::euros(observed_cents);
    match code {
        "it" => format!(
            "«{title}» non è partito: chi lo ha in carico è oltre il suo tetto mensile — {spent} contro {cap}. Alza il tetto dalla sua scheda, oppure aspetta il mese nuovo."
        ),
        _ => format!(
            "\"{title}\" did not start: whoever holds it is over their monthly cap — {spent} against {cap}. Raise it from their card, or wait for the new month."
        ),
    }
}
/// The meeting-request approval's summary, in the company's language.
pub fn meeting_request_summary(code: &str, convener: &str, topic: &str) -> String {
    match code {
        "it" => format!("{convener} chiede di riunirsi su «{topic}»"),
        _ => format!("{convener} asks to meet about \"{topic}\""),
    }
}

/// Why a run stopped short of done, for its handoff (ADR-0050 §4).
pub enum Stop {
    TimedOut(u64),
    Ceiling,
    /// It stopped itself before its bounds and wrote its own handoff.
    HandedOff,
}

/// The handoff a run stopped short hands on, shown to the person on the task
/// and to the next attempt. `own` is the run's own file when it wrote one;
/// otherwise `changed` and `said` are what Overmind read from what it left.
/// `code` says whether the changes stay behind in a worktree the next attempt
/// will not have -- said plainly, so it does not look for them.
pub struct Handoff<'a> {
    pub stop: &'a Stop,
    pub own: Option<&'a str>,
    pub changed: Option<&'a [String]>,
    pub code_run: bool,
    pub said: &'a str,
}

pub fn handoff(code: &str, h: &Handoff) -> String {
    let it = code == "it";
    let reason = match (h.stop, it) {
        (Stop::TimedOut(s), true) => format!("è scaduto il tempo ({s}s)"),
        (Stop::TimedOut(s), false) => format!("it timed out after {s}s"),
        (Stop::Ceiling, true) => "ha raggiunto il tetto di spesa".to_string(),
        (Stop::Ceiling, false) => "it reached its budget ceiling".to_string(),
        (Stop::HandedOff, true) => "si è fermato da solo prima dei suoi limiti".to_string(),
        (Stop::HandedOff, false) => "it stopped itself before its bounds".to_string(),
    };
    if let Some(own) = h.own {
        return if it {
            format!("Il run si è fermato perché {reason}. Il suo handoff:\n\n{own}")
        } else {
            format!("The run stopped because {reason}. Its own handoff:\n\n{own}")
        };
    }
    let changed = match h.changed {
        None if it => "- (non leggibile)".to_string(),
        None => "- (could not be read)".to_string(),
        Some([]) if it => "- niente".to_string(),
        Some([]) => "- nothing".to_string(),
        Some(files) => files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    let kept = match (h.code_run, it) {
        (true, true) => {
            "\n\nQuesti cambi erano nella sua worktree e non sono in quella del prossimo tentativo, che riparte dal repository."
        }
        (true, false) => {
            "\n\nThese changes were in its worktree and are not in the next attempt's, which starts from the repository again."
        }
        (false, _) => "",
    };
    let said = if h.said.trim().is_empty() {
        if it { "(niente)" } else { "(nothing)" }
    } else {
        h.said
    };
    // A fence longer than any run of backticks in what it said, so its own
    // code blocks cannot close ours.
    let longest = said.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    if it {
        format!(
            "Il run si è fermato perché {reason}, prima di scrivere un handoff suo. Overmind l'ha scritto da ciò che ha lasciato.\n\n## Cosa ha cambiato\n{changed}{kept}\n\n## L'ultima cosa che ha detto\n{fence}\n{said}\n{fence}"
        )
    } else {
        format!(
            "The run stopped because {reason}, before writing a handoff of its own. Overmind wrote this from what it left behind.\n\n## What it changed\n{changed}{kept}\n\n## The last thing it said\n{fence}\n{said}\n{fence}"
        )
    }
}

#[cfg(test)]
mod summary_tests {
    use super::{meeting_request_summary, start_task_summary};

    #[test]
    fn summaries_speak_the_companys_language_and_default_to_english() {
        assert_eq!(start_task_summary("it", "Listino"), "Avvia «Listino»");
        assert_eq!(
            start_task_summary("en", "Price list"),
            "Start \"Price list\""
        );
        assert_eq!(start_task_summary("xx", "X"), "Start \"X\"");
        assert_eq!(
            meeting_request_summary("it", "Nico", "budget"),
            "Nico chiede di riunirsi su «budget»"
        );
        assert_eq!(
            meeting_request_summary("en", "Nico", "budget"),
            "Nico asks to meet about \"budget\""
        );
    }
}
