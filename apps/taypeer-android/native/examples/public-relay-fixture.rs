//! Local TLS relay for synthetic Android instrumentation; all emitted data is public.
use iroh_relay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
use std::{
    io::{BufRead, Write},
    net::Ipv4Addr,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (certificates, server_config) =
        iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let mut relay = RelayConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(TlsConfig::new(
        (Ipv4Addr::LOCALHOST, 0),
        CertConfig::Manual { server_config },
    ));
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    let server = Server::spawn(config).await?;
    let address = server
        .https_addr()
        .ok_or("PUBLIC relay TLS address missing")?;
    println!(
        "{}",
        serde_json::json!({ "url": format!("https://127.0.0.1:{}", address.port()), "root_der": certificates[0].as_ref() })
    );
    std::io::stdout().flush()?;
    // The orchestrator retains stdin; newline or EOF terminates this fixture.
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)
    })
    .await??;
    server.shutdown().await?;
    Ok(())
}
