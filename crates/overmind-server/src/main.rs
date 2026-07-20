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
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("overmind-server listening on http://{addr} (db: {db_url})");
    axum::serve(listener, overmind_server::app(state)).await?;
    Ok(())
}
