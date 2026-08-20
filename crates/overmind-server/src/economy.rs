//! How this Overmind pays for the work it does (ADR-0030).
//!
//! Since [ADR-0012](../../docs/adr/0012-budgets-and-governance.md) the budget
//! has been money: cents reserved, cents spent, a monthly cap. That is exactly
//! right when an API key is paying, and beside the point under a subscription,
//! where there is no dollar to run out of — only a window, a quota inside it,
//! and a refusal at the end. The cap survives either way (it is also the brake
//! on a looping agent), but what it *promises* is different, and an interface
//! that says the same thing in both is wrong in one of them.
//!
//! So the economy is detected rather than configured. A setting that can
//! disagree with reality is a setting that will, and here the disagreement is a
//! bill.
//!
//! # Reading the CLI's answer
//!
//! `claude auth status --json`, measured in all three states that occur
//! (2026-08-17, claude 2.1.233):
//!
//! | | `authMethod` | `apiKeySource` | `subscriptionType` |
//! |---|---|---|---|
//! | a key and a login together | `claude.ai` | present | `null` |
//! | a login alone | `claude.ai` | **absent** | `"max"` |
//! | a key alone (the image) | `api_key` | present | absent |
//!
//! **`authMethod` does not discriminate** — it answers `claude.ai` in two
//! opposite situations, and reading it would have produced exactly the wrong
//! answer on a developer's machine, which is where both credentials usually
//! live. What discriminates is the *presence* of `apiKeySource` and a non-null
//! `subscriptionType`.
//!
//! The precedence rule falls out of that rather than being imposed: when both
//! are present the key wins, which is what the CLI itself warns — *"claude.ai
//! connectors are disabled because ANTHROPIC_API_KEY or another auth source is
//! set and takes precedence over your claude.ai login"*. Note that the
//! subscription fields go `null` in that state: the CLI is telling us the plan
//! is not the thing paying.

use serde_json::Value;

use crate::db::Config;

/// Which economy this Overmind is running in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Economy {
    /// An API key pays. Spend is money, and the cap is a ceiling in dollars.
    Key {
        /// There is a claude.ai login here too, and the key is winning.
        ///
        /// Worth its own field rather than a footnote: this is the state where
        /// somebody signed in, believes their plan is covering the work, and is
        /// being billed instead. The CLI warns about it in a log line nobody
        /// reads, which is not the same as being told.
        overrides_login: bool,
    },
    /// A subscription pays. Spend is quota inside a window we cannot see; the
    /// plan's name is reported when the CLI gives one, because "max" is more
    /// use to a person than "subscription".
    Subscription { plan: Option<String> },
    /// We could not tell — said rather than assumed. Assuming the wrong one
    /// either bills someone who thought they were on a plan, or promises a
    /// dollar ceiling that is not enforcing anything.
    Unknown { kind: UnknownKind, reason: String },
}

/// Why the economy is unknown — machine-readable, because the interface acts
/// on the difference (M22). "Not signed in" has a remedy the UI can name;
/// "custom adapter" is deliberate and must stay quiet; everything else is a
/// probe that failed. A UI string-matching the English `reason` would break
/// the day the sentence improves, which is exactly the defect this avoids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownKind {
    /// The default adapter answered: nobody is signed in. Remediable, and the
    /// interface should say how before a first turn burns on discovering it.
    NotSignedIn,
    /// A custom `OVERMIND_AGENT_CMD` is configured and was deliberately never
    /// interrogated. Not a problem to warn about.
    CustomAdapter,
    /// The probe ran and could not be read: a failure, a timeout, an
    /// unrecognised shape, or simply not asked yet.
    Unreadable,
}

impl UnknownKind {
    pub fn slug(&self) -> &'static str {
        match self {
            UnknownKind::NotSignedIn => "not_signed_in",
            UnknownKind::CustomAdapter => "custom_adapter",
            UnknownKind::Unreadable => "unreadable",
        }
    }
}

impl Economy {
    /// Does the cap correspond to money that will actually be charged?
    pub fn is_metered(&self) -> bool {
        matches!(self, Economy::Key { .. })
    }
}

/// Read the economy out of `claude auth status --json`.
///
/// Split from the probe so the rule can be tested against the payloads that
/// were actually observed, rather than against shapes we imagined.
pub fn read(status: &Value) -> Economy {
    if status.get("loggedIn").and_then(Value::as_bool) != Some(true) {
        return Economy::Unknown {
            kind: UnknownKind::NotSignedIn,
            reason: "the agent CLI is not signed in".into(),
        };
    }
    // Presence, not truthiness: the field is absent when no key is in play, and
    // its value names *where* the key came from rather than what it is.
    if status.get("apiKeySource").is_some_and(|v| !v.is_null()) {
        // Here `authMethod` earns its keep — not for telling a key from a plan,
        // which it cannot do, but for telling whether there is a login *behind*
        // the key. With a key alone the CLI answers `api_key`; with a key over a
        // login it answers `claude.ai`, naming the thing it is overriding.
        let overrides_login = status.get("authMethod").and_then(Value::as_str) == Some("claude.ai");
        return Economy::Key { overrides_login };
    }
    if let Some(plan) = status.get("subscriptionType").and_then(Value::as_str) {
        return Economy::Subscription {
            plan: Some(plan.to_string()),
        };
    }
    // Signed in, no key, no plan named. A shape we have not seen; saying so
    // beats picking the branch that happens to be cheaper to implement.
    Economy::Unknown {
        kind: UnknownKind::Unreadable,
        reason: "signed in, but the CLI named neither an API key nor a plan".into(),
    }
}

/// Ask the adapter CLI how it is authenticated.
///
/// **Run as the agent, not as the server.** In the image the server is root and
/// the agent's credentials live in the agent's home; a probe run as the server
/// would answer confidently about the wrong home directory — and would report
/// "not signed in" for a perfectly well signed-in agent.
///
/// Not caged: this is our own command with our own arguments, like `git` and
/// the memory server, which [ADR-0023](../../docs/adr/0023-os-level-sandboxing.md)
/// leaves outside on the same reasoning.
pub async fn detect(config: &Config) -> Economy {
    if let Some(declared) = &config.economy_override {
        return declared.clone();
    }
    // A custom adapter is not necessarily Claude Code, and `auth status` is not
    // a contract any adapter signed. Better to not know than to run somebody
    // else's binary with arguments we invented.
    if config.agent_cmd.is_some() {
        return Economy::Unknown {
            kind: UnknownKind::CustomAdapter,
            reason:
                "a custom OVERMIND_AGENT_CMD is configured, so Overmind cannot ask it how it pays"
                    .into(),
        };
    }

    let mut cmd = crate::sandbox::as_agent(config, "claude");
    cmd.args(["auth", "status", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let waited = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        cmd.spawn()?.wait_with_output().await
    })
    .await;

    let out = match waited {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            return Economy::Unknown {
                kind: UnknownKind::Unreadable,
                reason: format!("could not run the agent CLI: {e}"),
            };
        }
        Err(_) => {
            return Economy::Unknown {
                kind: UnknownKind::Unreadable,
                reason: "the agent CLI did not answer within 20s".into(),
            };
        }
    };
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(status) => read(&status),
        Err(e) => Economy::Unknown {
            kind: UnknownKind::Unreadable,
            reason: format!("the agent CLI's answer was not JSON: {e}"),
        },
    }
}

/// Where a subscription stands in the window that is governing it right now.
///
/// Not a percentage — that lives only in the status line, which a headless run
/// never invokes (measured). What a headless run *does* get is better than
/// nothing and arguably better than a percentage: which window applies, when it
/// resets, and whether we are still allowed inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanWindow {
    /// `five_hour` or `seven_day` — a plan has both, and this names the one
    /// doing the limiting at the moment.
    pub window: String,
    /// Unix epoch seconds at which this window resets. The countdown a person
    /// actually wants: "back at 14:30" beats "62% used".
    pub resets_at: i64,
    pub health: PlanHealth,
}

/// The adapter's own taxonomy, not our reading of its prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanHealth {
    Allowed,
    /// `allowed_warning` — still working, close to the edge.
    Warning,
    /// `blocked` or `rejected` — the plan has run out for this window.
    Exhausted,
}

impl PlanHealth {
    fn read(status: &str) -> Option<Self> {
        match status {
            "allowed" => Some(PlanHealth::Allowed),
            "allowed_warning" => Some(PlanHealth::Warning),
            "blocked" | "rejected" => Some(PlanHealth::Exhausted),
            // A value the CLI grew after we were written. Silence beats
            // guessing which side of the line it falls on.
            _ => None,
        }
    }
}

/// The last plan report in an adapter's output, if it made one.
///
/// The adapter emits a `rate_limit_event` on a stream run, which is why the
/// default command asks for `stream-json`: the plan's state then **rides along
/// with work already being done** rather than costing a call of its own — the
/// same bargain ADR-0026 made for memory watermarks.
///
/// The last one wins: a long run may report more than once, and the newest is
/// the one still true.
pub fn plan_window_in(output: &str) -> Option<PlanWindow> {
    output
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("rate_limit_event"))
        .find_map(|v| read_plan_window(v.get("rate_limit_info")?))
}

/// One `rate_limit_info` object, as measured on 2026-08-17.
pub fn read_plan_window(info: &Value) -> Option<PlanWindow> {
    Some(PlanWindow {
        window: info
            .get("rateLimitType")
            .and_then(Value::as_str)?
            .to_string(),
        resets_at: info.get("resetsAt").and_then(Value::as_i64)?,
        health: PlanHealth::read(info.get("status").and_then(Value::as_str)?)?,
    })
}

/// What we know about each of a plan's windows, keyed by its name.
///
/// A plan has **both** a five-hour and a seven-day limit, and a run reports
/// whichever is governing it at that moment — so they are learned separately
/// and kept separately. A window nobody has reported yet is *absent*, not
/// assumed healthy: "we have not heard" and "you are fine" are different
/// sentences, and only one of them is true before the first report.
pub type PlanWindows = std::collections::BTreeMap<String, PlanWindow>;

/// The window names a plan actually has, in the order a person thinks about
/// them: the one that bites first, then the one behind it.
pub const PLAN_WINDOWS: [&str; 2] = ["five_hour", "seven_day"];

/// A window's reset moment as a readable UTC timestamp.
///
/// For the durable `title`/`body` of a notification and for a chat message,
/// which are written once and read later; the *client* words the countdown,
/// because "tra 2 ore" and "in 2 hours" differ in more than vocabulary and only
/// the reader's locale knows which is right (M16).
pub fn reset_time(window: &PlanWindow) -> String {
    chrono::DateTime::from_timestamp(window.resets_at, 0)
        .map(|t| t.format("%H:%M UTC").to_string())
        .unwrap_or_else(|| "an unknown time".to_string())
}

/// How a plan window reaches a client.
pub fn window_as_json(window: &PlanWindow) -> Value {
    serde_json::json!({
        "window": window.window,
        "resets_at": window.resets_at,
        "health": match window.health {
            PlanHealth::Allowed => "allowed",
            PlanHealth::Warning => "warning",
            PlanHealth::Exhausted => "exhausted",
        },
    })
}

/// One line for the startup log — and it says what the cap *means*, not only
/// which economy won, because that is the part a reader is about to act on.
pub fn describe(economy: &Economy) -> String {
    match economy {
        Economy::Key {
            overrides_login: true,
        } => "an API key — which is overriding a claude.ai login you are signed into. Unset ANTHROPIC_API_KEY to let the plan pay. The budget cap is a ceiling in real money".into(),
        Economy::Key {
            overrides_login: false,
        } => "an API key — the budget cap is a ceiling in real money".into(),
        Economy::Subscription { plan } => format!(
            "a subscription{} — the budget cap is an equivalent, not a charge. Of the plan itself, runs report which window is limiting and when it resets, not how much is left",
            plan.as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        ),
        Economy::Unknown { reason, .. } => {
            format!(
                "unknown — {reason}. The budget cap still brakes a looping agent, but do not read it as a promise"
            )
        }
    }
}

/// How this reaches a client.
///
/// Deliberately narrow. The CLI also returns `email`, `orgId` and `orgName`,
/// and Overmind has no use for any of them — putting somebody's address in a
/// response the browser reads would be collecting it for the sake of having
/// asked.
pub fn as_json(economy: &Economy) -> Value {
    match economy {
        Economy::Key { overrides_login } => serde_json::json!({
            "kind": "key", "metered": true, "overrides_login": overrides_login
        }),
        Economy::Subscription { plan } => serde_json::json!({
            "kind": "subscription", "metered": false, "plan": plan
        }),
        Economy::Unknown { kind, reason } => serde_json::json!({
            "kind": "unknown", "metered": false, "reason": reason,
            "unknown_kind": kind.slug()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The state on a developer's machine: both credentials present. The key is
    /// what will be billed, and the CLI says so by nulling the plan fields.
    #[test]
    fn a_key_beside_a_login_is_a_key() {
        let observed = json!({
            "loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty",
            "apiKeySource": "ANTHROPIC_API_KEY", "email": null, "orgId": null,
            "orgName": null, "subscriptionType": null
        });
        assert_eq!(
            read(&observed),
            Economy::Key {
                overrides_login: true
            },
            "a login is being overridden here, and the person deserves to know"
        );
    }

    /// The same machine with the key unset. `authMethod` has not moved — which
    /// is the whole reason it cannot be the thing we read.
    #[test]
    fn a_login_alone_is_a_subscription_and_names_the_plan() {
        let observed = json!({
            "loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty",
            "email": "someone@example.com", "orgId": "3e091994", "orgName": "an org",
            "subscriptionType": "max"
        });
        assert_eq!(
            read(&observed),
            Economy::Subscription {
                plan: Some("max".into())
            }
        );
    }

    /// Inside the image, where only a key exists.
    #[test]
    fn a_key_alone_is_a_key() {
        let observed = json!({
            "loggedIn": true, "authMethod": "api_key", "apiProvider": "firstParty",
            "apiKeySource": "ANTHROPIC_API_KEY"
        });
        assert_eq!(
            read(&observed),
            Economy::Key {
                overrides_login: false
            },
            "no login to override in the image — only a key"
        );
    }

    /// `authMethod` answers `claude.ai` for both a key-and-login and a login
    /// alone, so a reading built on it is right half the time. Held as a test
    /// because it is the mistake this module exists to have already made.
    #[test]
    fn the_auth_method_field_is_not_the_discriminator() {
        let with_key = json!({
            "loggedIn": true, "authMethod": "claude.ai",
            "apiKeySource": "ANTHROPIC_API_KEY", "subscriptionType": null
        });
        let without = json!({
            "loggedIn": true, "authMethod": "claude.ai", "subscriptionType": "max"
        });
        assert_eq!(
            with_key.get("authMethod"),
            without.get("authMethod"),
            "the fixtures must agree here, or this test proves nothing"
        );
        assert_ne!(read(&with_key), read(&without));
    }

    /// The browser acts on `unknown_kind`, so it is a wire contract, not a
    /// debug detail: if the field vanished, the sign-in notice would silently
    /// never show again (M22).
    #[test]
    fn the_unknown_kind_reaches_the_wire() {
        let v = as_json(&read(&json!({ "loggedIn": false })));
        assert_eq!(v["kind"], "unknown");
        assert_eq!(v["unknown_kind"], "not_signed_in");
    }

    #[test]
    fn not_signed_in_is_not_a_guess() {
        let out = read(&json!({ "loggedIn": false }));
        // The kind is what lets the interface offer the remedy (M22) — and
        // only here: every other unknown must NOT invite a sign-in.
        assert!(
            matches!(
                out,
                Economy::Unknown {
                    kind: UnknownKind::NotSignedIn,
                    ..
                }
            ),
            "{out:?}"
        );
        assert!(!out.is_metered());
    }

    /// Signed in, and the CLI named neither. We have not seen this; the point is
    /// that meeting it does not silently become whichever branch came first.
    #[test]
    fn a_shape_we_do_not_recognise_is_unknown() {
        let out = read(&json!({ "loggedIn": true, "authMethod": "something-new" }));
        assert!(
            matches!(
                out,
                Economy::Unknown {
                    kind: UnknownKind::Unreadable,
                    ..
                }
            ),
            "{out:?}"
        );
    }

    /// Only a key means money, and `is_metered` is what the rest of the system
    /// asks before promising a dollar ceiling.
    #[test]
    fn only_a_key_is_metered() {
        assert!(
            Economy::Key {
                overrides_login: false
            }
            .is_metered()
        );
        assert!(!Economy::Subscription { plan: None }.is_metered());
        assert!(
            !Economy::Unknown {
                kind: UnknownKind::Unreadable,
                reason: String::new()
            }
            .is_metered()
        );
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;

    /// The event exactly as measured on 2026-08-17, from a headless
    /// `--output-format stream-json --verbose` run on a subscription.
    const OBSERVED: &str = r#"{"type":"system","subtype":"init","model":"claude-opus-5"}
{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1786983000,"rateLimitType":"five_hour","overageStatus":"rejected","overageDisabledReason":"out_of_credits","isUsingOverage":false},"uuid":"7cde8dec","session_id":"dd7de845"}
{"type":"assistant","message":{}}
{"type":"result","subtype":"success","total_cost_usd":0.01}"#;

    #[test]
    fn a_stream_run_reports_which_window_is_governing_and_when_it_resets() {
        let window = plan_window_in(OBSERVED).expect("the observed run reports a window");
        assert_eq!(window.window, "five_hour");
        assert_eq!(window.resets_at, 1_786_983_000);
        assert_eq!(window.health, PlanHealth::Allowed);
    }

    /// The adapter's own taxonomy, read rather than interpreted. `blocked` and
    /// `rejected` are what a plan running out looks like — which is why
    /// exhaustion can be recognised exactly here, and could not be recognised
    /// at all from the prose of a failed `-p json` run.
    #[test]
    fn the_status_vocabulary_is_the_adapters_and_not_ours() {
        assert_eq!(PlanHealth::read("allowed"), Some(PlanHealth::Allowed));
        assert_eq!(
            PlanHealth::read("allowed_warning"),
            Some(PlanHealth::Warning)
        );
        assert_eq!(PlanHealth::read("blocked"), Some(PlanHealth::Exhausted));
        assert_eq!(PlanHealth::read("rejected"), Some(PlanHealth::Exhausted));
        // A value the CLI grows after us is not silently sorted onto a side.
        assert_eq!(PlanHealth::read("something_new"), None);
    }

    /// A long run can report more than once, and the newest is the one still
    /// true.
    #[test]
    fn the_last_report_wins() {
        let two = format!(
            "{}\n{}",
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1,"rateLimitType":"five_hour"}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":2,"rateLimitType":"seven_day"}}"#
        );
        let window = plan_window_in(&two).expect("a window");
        assert_eq!(window.window, "seven_day");
        assert_eq!(window.resets_at, 2);
        assert_eq!(window.health, PlanHealth::Warning);
    }

    /// Under an API key there is no such event at all, and the absence must
    /// read as "no plan window" rather than as a parse failure.
    #[test]
    fn a_run_that_says_nothing_about_a_plan_reports_no_window() {
        assert_eq!(
            plan_window_in(r#"{"type":"result","subtype":"success","total_cost_usd":0.01}"#),
            None
        );
        assert_eq!(plan_window_in(""), None);
    }

    /// An event missing the fields we need is not half a window.
    #[test]
    fn an_incomplete_report_is_no_report() {
        assert_eq!(
            plan_window_in(r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#),
            None
        );
    }

    #[test]
    fn a_window_reaches_a_client_with_its_health_named() {
        let json = window_as_json(&PlanWindow {
            window: "five_hour".into(),
            resets_at: 1_786_983_000,
            health: PlanHealth::Exhausted,
        });
        assert_eq!(json["window"], "five_hour");
        assert_eq!(json["resets_at"], 1_786_983_000_i64);
        assert_eq!(json["health"], "exhausted");
    }

    #[test]
    fn the_observed_event_is_json_we_can_actually_parse() {
        // Guards the fixture itself: a typo here would make every test above
        // pass against a string that is not the thing we measured.
        let lines: Vec<&str> = OBSERVED.lines().collect();
        assert_eq!(lines.len(), 4);
        for line in lines {
            serde_json::from_str::<Value>(line).expect("every observed line is json");
        }
    }
}

#[cfg(test)]
mod override_tests {
    use super::*;
    use serde_json::json;

    /// The one state this slice exists for: signed in with a plan, billed to a
    /// key. Nothing is broken, which is precisely why it goes unnoticed.
    #[test]
    fn a_key_over_a_login_is_reported_as_overriding_it() {
        let both = json!({
            "loggedIn": true, "authMethod": "claude.ai",
            "apiKeySource": "ANTHROPIC_API_KEY", "subscriptionType": null
        });
        assert_eq!(
            read(&both),
            Economy::Key {
                overrides_login: true
            }
        );
    }

    /// A key with nothing behind it — the image — is not overriding anything,
    /// and saying it were would be a warning that trains people to ignore
    /// warnings.
    #[test]
    fn a_key_alone_is_not_overriding_anything() {
        let alone = json!({
            "loggedIn": true, "authMethod": "api_key",
            "apiKeySource": "ANTHROPIC_API_KEY"
        });
        assert_eq!(
            read(&alone),
            Economy::Key {
                overrides_login: false
            }
        );
    }

    /// Both are metered — what is shadowed changes the sentence, never the
    /// arithmetic.
    #[test]
    fn overriding_or_not_a_key_still_means_money() {
        for overrides_login in [true, false] {
            assert!(Economy::Key { overrides_login }.is_metered());
        }
    }

    /// The startup line names the fix, not just the fact. "You are on a key"
    /// tells someone nothing they can do; "unset ANTHROPIC_API_KEY" does.
    #[test]
    fn the_startup_line_says_how_to_get_the_plan_back() {
        let said = describe(&Economy::Key {
            overrides_login: true,
        });
        assert!(said.contains("ANTHROPIC_API_KEY"), "{said}");
        assert!(said.contains("overriding"), "{said}");

        let quiet = describe(&Economy::Key {
            overrides_login: false,
        });
        assert!(
            !quiet.contains("overriding"),
            "a key with nothing behind it must not cry wolf: {quiet}"
        );
    }

    #[test]
    fn the_client_is_told_which_of_the_two_it_is() {
        let loud = as_json(&Economy::Key {
            overrides_login: true,
        });
        assert_eq!(loud["overrides_login"], true);
        assert_eq!(loud["metered"], true);
        let quiet = as_json(&Economy::Key {
            overrides_login: false,
        });
        assert_eq!(quiet["overrides_login"], false);
    }
}

#[cfg(test)]
mod exhaustion_tests {
    use super::*;

    /// Exhaustion is a status, not a sentence. This is the whole reason the
    /// pause path could be wired at all: M18 left it open "once we can
    /// recognise it reliably", and prose in an unknown language was never going
    /// to be reliable.
    #[test]
    fn a_blocked_window_is_recognised_without_reading_any_prose() {
        let blocked = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"blocked","resetsAt":1786983000,"rateLimitType":"seven_day"}}"#;
        let window = plan_window_in(blocked).expect("a window");
        assert_eq!(window.health, PlanHealth::Exhausted);
        assert_eq!(window.window, "seven_day");
    }

    /// `allowed_warning` is close to the edge, not over it. Pausing a room on a
    /// warning would stop work that was still permitted — the opposite of the
    /// "transient and external" reasoning that makes pausing right at all.
    #[test]
    fn a_warning_is_not_an_exhaustion() {
        let warned = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed_warning","resetsAt":1,"rateLimitType":"five_hour"}}"#;
        let window = plan_window_in(warned).expect("a window");
        assert_ne!(window.health, PlanHealth::Exhausted);
        assert_eq!(window.health, PlanHealth::Warning);
    }

    /// The reset moment is written down readably for the durable record, and
    /// left to the client to turn into a countdown (M16).
    #[test]
    fn a_reset_moment_is_readable_in_the_durable_record() {
        let said = reset_time(&PlanWindow {
            window: "five_hour".into(),
            resets_at: 1_786_983_000,
            health: PlanHealth::Exhausted,
        });
        assert!(said.contains("UTC"), "{said}");
        assert!(said.contains(':'), "{said}");
    }

    /// A timestamp we cannot make sense of does not become a confident lie
    /// about when work resumes.
    #[test]
    fn an_impossible_reset_moment_says_it_does_not_know() {
        let said = reset_time(&PlanWindow {
            window: "five_hour".into(),
            resets_at: i64::MAX,
            health: PlanHealth::Exhausted,
        });
        assert!(said.contains("unknown"), "{said}");
    }
}
