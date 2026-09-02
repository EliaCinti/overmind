//! Two writers at once is the first thing real use does (M23): a CEO turn
//! writes its messages while the person creates a project in another tab.
//! WAL keeps readers flowing, but SQLite still allows one writer at a time --
//! and without a busy timeout the second writer is answered "database is
//! locked" on the spot, which reached the browser as a 500 within the first
//! minute of dogfooding. The pool now waits its turn; this holds that.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn a_write_waits_for_the_writer_ahead_instead_of_failing() {
    // A file-backed database: the in-memory one is capped to a single
    // connection, which cannot contend with itself.
    let dir = std::env::temp_dir().join(format!(
        "overmind-contention-{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let db = format!("sqlite://{}", dir.join("db.sqlite").display());

    let config = overmind_server::Config {
        data_dir: dir.join("data"),
        ..overmind_server::Config::default()
    };
    let state = overmind_server::init_with(&db, config).await.expect("init");
    let app = common::claimed(overmind_server::app(state.clone()), &dir.join("data")).await;

    // One connection takes the write lock and sits on it, the way a
    // mid-flight conversation turn does.
    let mut holder = state.pool.acquire().await.expect("acquire");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *holder)
        .await
        .expect("take the write lock");

    // While the lock is held, a person creates a company. Fire it now;
    // it must wait, not fail.
    let post = tokio::spawn(async move {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/companies")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Contesa"}"#))
                .expect("build"),
        )
        .await
        .expect("router responds")
        .status()
    });

    // The writer ahead finishes after a beat -- far inside the busy timeout.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    sqlx::query("COMMIT")
        .execute(&mut *holder)
        .await
        .expect("release the write lock");
    drop(holder);

    let status = post.await.expect("join");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the second writer should have waited for the lock, not been told the database is locked"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
