//! Native three-process acceptance through a loopback relay with verified TLS.
#![cfg(target_os = "macos")]

#[test]
#[ignore = "native-keychain: explicitly opt in; may display macOS access dialogs"]
fn three_cli_profiles_exchange_through_a_verified_relay_without_direct_paths() {
    use iroh_relay::server::{CertConfig, RelayConfig, Server, ServerConfig, TlsConfig};
    use std::net::Ipv4Addr;
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let (certificates, server_config) =
        iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let certificate = directory.path().join("PUBLIC-relay-ca.der");
    std::fs::write(&certificate, certificates[0].as_ref()).unwrap();
    let mut relay = RelayConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(TlsConfig::new(
        (Ipv4Addr::LOCALHOST, 0),
        CertConfig::Manual { server_config },
    ));
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    let server = runtime.block_on(Server::spawn(config)).unwrap();
    let url = format!("https://{}", server.https_addr().unwrap());
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-cli-p2p.py");
    let status = std::process::Command::new("python3")
        .arg(script)
        .arg(env!("CARGO_BIN_EXE_taypeer-cli"))
        .args(["--relay", &url])
        .arg("--relay-ca")
        .arg(certificate)
        .status()
        .unwrap();
    assert!(status.success(), "Three-process relay acceptance failed");
}
