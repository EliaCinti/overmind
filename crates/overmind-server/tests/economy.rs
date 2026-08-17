//! How you pay is a fact the interface can read (ADR-0030, M20 slice A).
//!
//! The reading rule itself is unit-tested in `economy.rs` against the three
//! payloads that were actually observed. This is the other half: that the fact
//! reaches a client at all, and that not knowing is a state the API can express
//! rather than a hole it falls into.

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

async fn health(config: overmind_server::Config) -> Value {
    let state = overmind_server::init_with("sqlite::memory:", config)
        .await
        .expect("init in-memory db");
    let app = overmind_server::app(state);
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/health")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router responds");
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("read body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("health is json")
}

/// Nobody has asked the CLI yet — the binary does that at startup — so the
/// honest answer is that we do not know, with the reason attached.
///
/// The reason matters more than it looks: "unknown" alone sends a person to
/// read source code, while "not detected yet" or "a custom adapter is
/// configured" sends them somewhere useful.
#[tokio::test]
async fn before_anything_is_detected_the_api_says_it_does_not_know() {
    let body = health(overmind_server::Config::default()).await;
    let economy = &body["economy"];
    assert_eq!(economy["kind"], "unknown", "{body}");
    assert_eq!(economy["metered"], false, "{body}");
    assert!(
        economy["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "an unknown economy must say why: {body}"
    );
}

/// The escape hatch, for an adapter that is not Claude Code or a reading we got
/// wrong. `metered` is the field the rest of the system asks before promising a
/// ceiling in money, so it has to follow the declaration.
#[tokio::test]
async fn a_declared_economy_is_reported_and_carries_its_meaning() {
    let body = health(overmind_server::Config {
        economy_override: Some(overmind_server::economy::Economy::Key),
        ..overmind_server::Config::default()
    })
    .await;
    assert_eq!(body["economy"]["kind"], "key", "{body}");
    assert_eq!(
        body["economy"]["metered"], true,
        "a key is the only economy where the cap is money: {body}"
    );

    let body = health(overmind_server::Config {
        economy_override: Some(overmind_server::economy::Economy::Subscription {
            plan: Some("max".into()),
        }),
        ..overmind_server::Config::default()
    })
    .await;
    assert_eq!(body["economy"]["kind"], "subscription", "{body}");
    assert_eq!(body["economy"]["plan"], "max", "{body}");
    assert_eq!(
        body["economy"]["metered"], false,
        "under a plan the cap is an equivalent, not a charge: {body}"
    );
}

/// A custom adapter is not necessarily Claude Code, and `auth status` is not a
/// contract any adapter signed. Not knowing beats running somebody else's
/// binary with arguments we invented — and beats guessing the economy that
/// happens to be cheaper to implement.
#[tokio::test]
async fn a_custom_adapter_is_not_interrogated() {
    let economy = overmind_server::economy::detect(&overmind_server::Config {
        agent_cmd: Some("sh /somewhere/agent.sh".into()),
        ..overmind_server::Config::default()
    })
    .await;
    match economy {
        overmind_server::economy::Economy::Unknown { reason } => {
            assert!(reason.contains("OVERMIND_AGENT_CMD"), "{reason}");
        }
        other => panic!("a custom adapter must not be assumed into an economy: {other:?}"),
    }
}

/// A declaration outranks the probe, because it exists for the case where the
/// probe is wrong. Checked without a custom adapter set, so nothing else could
/// be producing this answer.
#[tokio::test]
async fn a_declaration_outranks_detection() {
    let economy = overmind_server::economy::detect(&overmind_server::Config {
        economy_override: Some(overmind_server::economy::Economy::Subscription { plan: None }),
        ..overmind_server::Config::default()
    })
    .await;
    assert_eq!(
        economy,
        overmind_server::economy::Economy::Subscription { plan: None }
    );
}
