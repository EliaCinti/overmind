use std::net::SocketAddr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = std::env::var("OVERMIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:7070".to_string())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("overmind-server listening on http://{addr}");
    axum::serve(listener, overmind_server::app()).await?;
    Ok(())
}
