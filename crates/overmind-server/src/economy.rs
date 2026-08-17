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
    Key,
    /// A subscription pays. Spend is quota inside a window we cannot see; the
    /// plan's name is reported when the CLI gives one, because "max" is more
    /// use to a person than "subscription".
    Subscription { plan: Option<String> },
    /// We could not tell — said rather than assumed. Assuming the wrong one
    /// either bills someone who thought they were on a plan, or promises a
    /// dollar ceiling that is not enforcing anything.
    Unknown { reason: String },
}

impl Economy {
    /// Does the cap correspond to money that will actually be charged?
    pub fn is_metered(&self) -> bool {
        matches!(self, Economy::Key)
    }
}

/// Read the economy out of `claude auth status --json`.
///
/// Split from the probe so the rule can be tested against the payloads that
/// were actually observed, rather than against shapes we imagined.
pub fn read(status: &Value) -> Economy {
    if status.get("loggedIn").and_then(Value::as_bool) != Some(true) {
        return Economy::Unknown {
            reason: "the agent CLI is not signed in".into(),
        };
    }
    // Presence, not truthiness: the field is absent when no key is in play, and
    // its value names *where* the key came from rather than what it is.
    if status.get("apiKeySource").is_some_and(|v| !v.is_null()) {
        return Economy::Key;
    }
    if let Some(plan) = status.get("subscriptionType").and_then(Value::as_str) {
        return Economy::Subscription {
            plan: Some(plan.to_string()),
        };
    }
    // Signed in, no key, no plan named. A shape we have not seen; saying so
    // beats picking the branch that happens to be cheaper to implement.
    Economy::Unknown {
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
                reason: format!("could not run the agent CLI: {e}"),
            };
        }
        Err(_) => {
            return Economy::Unknown {
                reason: "the agent CLI did not answer within 20s".into(),
            };
        }
    };
    match serde_json::from_slice::<Value>(&out.stdout) {
        Ok(status) => read(&status),
        Err(e) => Economy::Unknown {
            reason: format!("the agent CLI's answer was not JSON: {e}"),
        },
    }
}

/// One line for the startup log — and it says what the cap *means*, not only
/// which economy won, because that is the part a reader is about to act on.
pub fn describe(economy: &Economy) -> String {
    match economy {
        Economy::Key => "an API key — the budget cap is a ceiling in real money".into(),
        Economy::Subscription { plan } => format!(
            "a subscription{} — the budget cap is an equivalent, not a charge, and the plan's own quota is not visible from here",
            plan.as_deref()
                .map(|p| format!(" ({p})"))
                .unwrap_or_default()
        ),
        Economy::Unknown { reason } => {
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
        Economy::Key => serde_json::json!({ "kind": "key", "metered": true }),
        Economy::Subscription { plan } => serde_json::json!({
            "kind": "subscription", "metered": false, "plan": plan
        }),
        Economy::Unknown { reason } => serde_json::json!({
            "kind": "unknown", "metered": false, "reason": reason
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
        assert_eq!(read(&observed), Economy::Key);
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
        assert_eq!(read(&observed), Economy::Key);
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

    #[test]
    fn not_signed_in_is_not_a_guess() {
        let out = read(&json!({ "loggedIn": false }));
        assert!(matches!(out, Economy::Unknown { .. }), "{out:?}");
        assert!(!out.is_metered());
    }

    /// Signed in, and the CLI named neither. We have not seen this; the point is
    /// that meeting it does not silently become whichever branch came first.
    #[test]
    fn a_shape_we_do_not_recognise_is_unknown() {
        let out = read(&json!({ "loggedIn": true, "authMethod": "something-new" }));
        assert!(matches!(out, Economy::Unknown { .. }), "{out:?}");
    }

    /// Only a key means money, and `is_metered` is what the rest of the system
    /// asks before promising a dollar ceiling.
    #[test]
    fn only_a_key_is_metered() {
        assert!(Economy::Key.is_metered());
        assert!(!Economy::Subscription { plan: None }.is_metered());
        assert!(
            !Economy::Unknown {
                reason: String::new()
            }
            .is_metered()
        );
    }
}
