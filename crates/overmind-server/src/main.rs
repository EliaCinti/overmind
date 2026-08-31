use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // "Which version is my friend actually running?" must not require a
    // login: `docker compose exec overmind overmind-server --version`.
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("overmind-server {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let db_url =
        std::env::var("OVERMIND_DB").unwrap_or_else(|_| "sqlite://overmind.sqlite".to_string());
    // A restore staged before the last stop is swapped in here, before
    // anything opens the database (ADR-0044).
    let config = overmind_server::Config::from_env();
    let restored = overmind_server::backup::swap_pending(&config, &db_url).await?;

    let state = overmind_server::init(&db_url).await?;
    if let Some(record) = &restored {
        // Written onto the chain it restored, as a system event.
        if let Err(e) = overmind_server::backup::note_restored(&state, record).await {
            eprintln!("restore: the restore could not be written to the audit chain: {e}");
        }
    }
    let _heartbeat = overmind_server::scheduler::spawn(state.clone());

    let addr: SocketAddr = std::env::var("OVERMIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7070".to_string())
        .parse()?;
    // How we pay (ADR-0030). Asked here rather than inside `init` because it
    // costs a subprocess, and a library that shells out when you construct it
    // makes every test pay for a fact none of them are about.
    let economy = overmind_server::economy::detect(&state.config).await;
    state.set_economy(economy.clone());

    let listener = tokio::net::TcpListener::bind(addr).await?;
    // The version in the first log line: `docker compose logs` answers
    // "what is running here" without a login or an exec.
    println!(
        "overmind-server {} listening on http://{addr} (db: {db_url})",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "paying with: {}",
        overmind_server::economy::describe(&economy)
    );
    // Say what is holding agent runs, at the one moment someone is looking.
    // A cage nobody can see the absence of is how "no run had ever changed a
    // file" went unnoticed for a month.
    println!(
        "agent confinement: {}",
        overmind_server::sandbox::announce(&state.config)
    );
    let mut asked_to_restart = state.restart.subscribe();
    axum::serve(listener, overmind_server::app(state))
        .with_graceful_shutdown(async move {
            let _ = asked_to_restart.recv().await;
        })
        .await?;
    // The only thing that ends the wait is a staged restore. Say so, and
    // leave: the image's restart policy brings the server back on the
    // restored data, and natively the person starts it again.
    println!(
        "overmind-server: a restore is staged — stopping, so the next start swaps it in \
         (restart the container, or run the server again)"
    );
    Ok(())
}
