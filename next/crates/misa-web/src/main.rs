#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut ticket = None;
    let mut listen = "127.0.0.1:8080".to_string();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--ticket" | "-t" => ticket = arguments.next(),
            "--listen" | "-l" => listen = arguments.next().unwrap_or(listen),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let address: std::net::SocketAddr = listen.parse()?;

    let ticket = ticket.ok_or("pass --ticket misa:<endpoint id>:<session>")?;
    misa_web::attach(&ticket, address).await?;
    Ok(())
}
