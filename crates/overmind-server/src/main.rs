use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url =
        std::env::var("OVERMIND_DB").unwrap_or_else(|_| "sqlite://overmind.sqlite".to_string());
    let state = overmind_server::init(&db_url).await?;
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
    println!("overmind-server listening on http://{addr} (db: {db_url})");
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
    axum::serve(listener, overmind_server::app(state)).await?;
    Ok(())
}
