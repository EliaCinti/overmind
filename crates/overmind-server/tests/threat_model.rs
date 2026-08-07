//! The threat model is held to the code, not just written beside it (M10 slice 3).
//!
//! [`docs/THREAT-MODEL.md`](../../../docs/THREAT-MODEL.md) claims a set of
//! boundaries and, for each, names the test that would fail if it stopped being
//! true. That is only worth something if the names are real: a table pointing at
//! a test somebody renamed is exactly the kind of document that reads correctly
//! and means nothing.
//!
//! So this walks the table and checks every referenced test exists. It is a
//! cheap test for a failure mode this project has already hit three times in
//! code — `permissions`, `model`, and the cost ledger were all believed, all
//! documented, and all wired to nothing.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn src_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn threat_model() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/THREAT-MODEL.md")
        .canonicalize()
        .expect("docs/THREAT-MODEL.md must exist — the boundary is documented on purpose");
    std::fs::read_to_string(path).expect("read the threat model")
}

/// Backticked snake_case identifiers long enough to be test names, taken from
/// the last cell of each table row — the "Held by" column.
fn referenced_tests(doc: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in doc.lines() {
        let line = line.trim();
        if !line.starts_with('|') || line.starts_with("|---") {
            continue;
        }
        let Some(held_by) = line.trim_end_matches('|').rsplit('|').next() else {
            continue;
        };
        for token in held_by.split('`').skip(1).step_by(2) {
            // Test names: snake_case, no dots (that would be a filename), and
            // long enough not to be an ordinary word in backticks. Relying on
            // the lowercase convention is what keeps `GIT_CONFIG_GLOBAL` and
            // friends out; a test named in any other style would slip past
            // this check unnoticed, which is the one gap here.
            let looks_like_a_test = token.len() >= 12
                && token.contains('_')
                && token
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if looks_like_a_test {
                out.insert(token.to_string());
            }
        }
    }
    out
}

#[test]
fn every_boundary_the_threat_model_claims_names_a_test_that_exists() {
    let doc = threat_model();
    let referenced = referenced_tests(&doc);
    assert!(
        referenced.len() >= 10,
        "the table should reference the whole set of boundaries; found {}: {referenced:?}",
        referenced.len()
    );

    // Both surfaces: integration tests, and the unit tests that hold the
    // boundaries whose logic is pure enough to test directly.
    let mut all_sources = String::new();
    for dir in [tests_dir(), src_dir()] {
        for entry in std::fs::read_dir(&dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_some_and(|e| e == "rs") {
                all_sources.push_str(&std::fs::read_to_string(&path).expect("read source file"));
            }
        }
    }

    let missing: Vec<&String> = referenced
        .iter()
        .filter(|name| !all_sources.contains(&format!("fn {name}(")))
        .collect();
    assert!(
        missing.is_empty(),
        "THREAT-MODEL.md points at tests that no longer exist: {missing:?}\n\
         Either the test was renamed and the table is now fiction, or the \
         boundary was removed and the claim with it — both need a human."
    );
}

#[test]
fn the_threat_model_says_what_it_does_not_defend() {
    let doc = threat_model();
    // The absent section is the one that rots first: it is easy to add a
    // mechanism and forget that the limitation it *doesn't* cover is still
    // true. Not a spell-check — a check that the section exists at all.
    assert!(
        doc.contains("Who this does *not* defend against"),
        "a threat model without its limits is a sales page"
    );
    for expected in ["no authentication", "malicious adapter", "tamper-evident"] {
        assert!(
            doc.to_lowercase().contains(&expected.to_lowercase()),
            "the threat model no longer mentions `{expected}` — if that stopped \
             being a limitation, say so deliberately"
        );
    }
}
