//! Append-only, hash-chained audit log.
//!
//! Every event stores the hash of the previous event; the hash covers the
//! event's own stored representation. Any mutation of a past row breaks
//! verification from that point on (see ADR-0006).

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnection;

/// prev_hash of the first event in the chain.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Hash over the stored representation of an event. Hashing exactly what is
/// stored (payload as TEXT) avoids JSON re-canonicalization pitfalls.
pub fn compute_hash(
    seq: i64,
    prev_hash: &str,
    kind: &str,
    company_id: Option<&str>,
    task_id: Option<&str>,
    created_at: &str,
    payload: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_le_bytes());
    for part in [
        prev_hash,
        kind,
        company_id.unwrap_or(""),
        task_id.unwrap_or(""),
        created_at,
        payload,
    ] {
        hasher.update([0x1f]); // field separator: unambiguous framing of variable-length fields
        hasher.update(part.as_bytes());
    }
    hex::encode(hasher.finalize())
}

/// Append one event. Must be called inside the same transaction as the domain
/// write it describes, so state change and audit trail commit atomically.
pub async fn append(
    conn: &mut SqliteConnection,
    company_id: Option<&str>,
    task_id: Option<&str>,
    kind: &str,
    payload: &Value,
) -> Result<i64, sqlx::Error> {
    let last: Option<(i64, String)> =
        sqlx::query_as("SELECT seq, hash FROM audit_events ORDER BY seq DESC LIMIT 1")
            .fetch_optional(&mut *conn)
            .await?;
    let (seq, prev_hash) = match last {
        Some((s, h)) => (s + 1, h),
        None => (1, GENESIS_HASH.to_string()),
    };
    let created_at = chrono::Utc::now().to_rfc3339();
    let payload_text = payload.to_string();
    let hash = compute_hash(
        seq,
        &prev_hash,
        kind,
        company_id,
        task_id,
        &created_at,
        &payload_text,
    );
    sqlx::query(
        "INSERT INTO audit_events (seq, company_id, task_id, kind, payload, created_at, prev_hash, hash)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(seq)
    .bind(company_id)
    .bind(task_id)
    .bind(kind)
    .bind(payload_text)
    .bind(&created_at)
    .bind(&prev_hash)
    .bind(&hash)
    .execute(&mut *conn)
    .await?;
    Ok(seq)
}

#[derive(Debug, Serialize)]
pub struct ChainReport {
    pub valid: bool,
    pub events_checked: i64,
    pub first_invalid_seq: Option<i64>,
}

/// Walk the whole chain: check linkage (prev_hash) and recompute every hash.
pub async fn verify(pool: &SqlitePool) -> Result<ChainReport, sqlx::Error> {
    type Row = (
        i64,            // seq
        Option<String>, // company_id
        Option<String>, // task_id
        String,         // kind
        String,         // payload
        String,         // created_at
        String,         // prev_hash
        String,         // hash
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT seq, company_id, task_id, kind, payload, created_at, prev_hash, hash
         FROM audit_events ORDER BY seq ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut expected_prev = GENESIS_HASH.to_string();
    let mut checked = 0i64;
    for (seq, company_id, task_id, kind, payload, created_at, prev_hash, hash) in rows {
        let recomputed = compute_hash(
            seq,
            &prev_hash,
            &kind,
            company_id.as_deref(),
            task_id.as_deref(),
            &created_at,
            &payload,
        );
        if prev_hash != expected_prev || recomputed != hash {
            return Ok(ChainReport {
                valid: false,
                events_checked: checked,
                first_invalid_seq: Some(seq),
            });
        }
        expected_prev = hash;
        checked += 1;
    }
    Ok(ChainReport {
        valid: true,
        events_checked: checked,
        first_invalid_seq: None,
    })
}
