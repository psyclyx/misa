#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let mut tickets = Vec::new();
    let mut listen = "127.0.0.1:8080".to_string();
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--ticket" | "-t" | "--daemon" | "-d" => tickets.push(arguments.next().ok_or("A daemon address or ticket is required")?),
            "--listen" | "-l" => listen = arguments.next().unwrap_or(listen),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let address: std::net::SocketAddr = listen.parse()?;

    misa_web::hub::serve(&tickets, address).await?;
    Ok(())
}
