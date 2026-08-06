//! The models an agent may run on (ADR-0021).
//!
//! Until M14 slice 3 `AgentTraits.model` was seeded, patched and versioned by
//! nobody's benefit: no code read it, and the adapter was invoked without a
//! `--model` flag at all. So "the CEO runs on the strongest model" was a
//! sentence in a roadmap rather than a fact about the system, and the hire
//! dialog offered `claude-sonnet` / `claude-opus` / `claude-haiku` — none of
//! which is a model identifier.
//!
//! This module is the one place that knows which models exist. A model the
//! catalog does not name is refused at the API boundary rather than stored and
//! handed to a prompt later, which is the rule M16 already applies to language
//! codes: validate where the value enters, not where it finally breaks.

/// A model an agent may be put on.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Model {
    pub id: &'static str,
    /// Shown in the hire dialog. Not translated: a model name is a proper
    /// noun, like an archetype slug (ADR-0021).
    pub display_name: &'static str,
    /// Whether the model can read images. Every model here can, which makes
    /// the multimodal check against it vacuous *today* — see `AgentTraits`
    /// for why it is written down anyway.
    pub vision: bool,
}

/// Ordered strongest-first: the founding CEO takes the head of the list, and
/// the hire dialog offers them in this order.
const CATALOG: &[Model] = &[
    Model {
        id: "claude-opus-5",
        display_name: "Claude Opus 5",
        vision: true,
    },
    Model {
        id: "claude-opus-4-8",
        display_name: "Claude Opus 4.8",
        vision: true,
    },
    Model {
        id: "claude-sonnet-5",
        display_name: "Claude Sonnet 5",
        vision: true,
    },
    Model {
        id: "claude-haiku-4-5",
        display_name: "Claude Haiku 4.5",
        vision: true,
    },
];

/// The whole catalog, for the hire dialog.
pub fn catalog() -> &'static [Model] {
    CATALOG
}

pub fn lookup(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id == id)
}

pub fn is_known(id: &str) -> bool {
    lookup(id).is_some()
}

/// Whether this model can be given an image. An unknown model is treated as
/// sightless: we do not guess capabilities for something we do not ship.
pub fn supports_vision(id: &str) -> bool {
    lookup(id).is_some_and(|m| m.vision)
}

/// What the founding CEO runs on (M15). A lookup rather than a constant, so
/// "the strongest model" stays true as the catalog moves instead of being true
/// on the day it was typed.
pub fn strongest() -> &'static Model {
    // The catalog is a non-empty const; the fallback exists so this function
    // is total without an `unwrap` in non-test code.
    CATALOG.first().unwrap_or(&Model {
        id: "claude-opus-5",
        display_name: "Claude Opus 5",
        vision: true,
    })
}

/// What an archetype seeds its agents with unless tuned: the balanced middle
/// of the catalog, not its most expensive entry.
pub fn default_model() -> &'static Model {
    lookup("claude-sonnet-5").unwrap_or_else(|| strongest())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_sane() {
        for (i, m) in CATALOG.iter().enumerate() {
            assert!(!m.id.is_empty(), "model {i} has no id");
            assert_eq!(
                CATALOG.iter().filter(|o| o.id == m.id).count(),
                1,
                "duplicate model id `{}`",
                m.id
            );
        }
    }

    #[test]
    fn the_defaults_are_in_the_catalog() {
        // Guards the one way this module can lie: a default that names a model
        // the catalog does not carry would be refused by our own validation.
        assert!(is_known(strongest().id));
        assert!(is_known(default_model().id));
    }

    #[test]
    fn an_unknown_model_is_neither_known_nor_sighted() {
        assert!(!is_known("claude-sonnet"));
        assert!(!supports_vision("claude-sonnet"));
    }
}
