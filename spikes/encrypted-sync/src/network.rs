//! Synthetic process harness. Identity numbers are PUBLIC fixture seeds, never user keys.
use crate::{Fault, Id, MAX_BYTES, container::Store, digest};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, endpoint::presets};
use std::{fs, path::Path, time::Duration};
use tokio::time::timeout;
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const ALPN: &[u8] = b"pass2p-spike/container/3";
async fn endpoint(seed: u8, relay: &str) -> Result<Endpoint> {
    let mode = if relay == "direct" {
        RelayMode::Disabled
    } else {
        RelayMode::Custom(relay.parse::<iroh::RelayUrl>()?.into())
    };
    let mut builder = Endpoint::builder(presets::Minimal)
        .relay_mode(mode)
        .secret_key(SecretKey::from_bytes(&[seed; 32]))
        .alpns(vec![ALPN.to_vec()]);
    if relay != "direct" {
        // Only for the self-signed LOCAL test relay. Never an application default.
        if !relay.starts_with("https://127.0.0.1:") {
            return Err("test relay must be loopback".into());
        }
        builder = builder
            .clear_ip_transports()
            .ca_tls_config(iroh_relay::tls::CaTlsConfig::insecure_skip_verify());
    }
    let ep = builder.bind().await?;
    if relay != "direct" {
        timeout(Duration::from_secs(15), ep.online()).await?;
    }
    Ok(ep)
}
fn load_store(path: &str, root_file: &str) -> Result<Store> {
    let root: Id = serde_json::from_slice(&fs::read(root_file)?)?;
    Ok(Store::at(Path::new(path).to_owned(), root))
}
async fn run(args: &[String]) -> Result<()> {
    if args[1] == "relay" {
        let (_, url, _server) = iroh::test_utils::run_relay_server().await?;
        fs::write(&args[2], url.to_string())?;
        std::future::pending::<()>().await;
        return Ok(());
    }
    // mode, store, pinned-root-file, synthetic identity, rendezvous file, relay/direct, fault
    let store = load_store(&args[2], &args[3])?;
    let ep = endpoint(args[4].parse()?, &args[6]).await?;
    if args[1] == "listen" || args[1] == "admit-listen" {
        fs::write(&args[5], serde_json::to_vec(&ep.addr())?)?;
        let conn = timeout(Duration::from_secs(20), ep.accept())
            .await?
            .ok_or("accept stopped")?
            .await?;
        let peer = *conn.remote_id().as_bytes();
        let (mut send, mut recv) = conn.accept_bi().await?;
        let bytes = timeout(Duration::from_secs(10), recv.read_to_end(MAX_BYTES)).await??;
        let fault = match args[7].as_str() {
            "before-rename" => Fault::BeforeRename,
            "after-rename" => Fault::AfterRename,
            _ => Fault::None,
        };
        let accepted = if args[1] == "admit-listen" {
            let code = serde_json::from_slice(&bytes)?;
            let manager = ed25519_dalek::SigningKey::from_bytes(&[args[4].parse()?; 32]);
            store
                .redeem(
                    b"PUBLIC synthetic spike password",
                    &manager,
                    crate::container::Redemption {
                        code,
                        peer,
                        now: 1,
                        approved: args[7] == "approved",
                    },
                    fault,
                )
                .is_ok()
        } else {
            store.receive_container(peer, &bytes, fault).is_ok()
        };
        if accepted {
            send.write_all(&digest(&bytes)).await?;
        } else {
            send.write_all(&[0]).await?;
        }
        send.finish()?;
        let _ = timeout(Duration::from_secs(3), conn.closed()).await;
    } else if args[1] == "send"
        || args[1] == "truncate"
        || args[1] == "unreachable"
        || args[1] == "redeem"
    {
        let addr: EndpointAddr = serde_json::from_slice(&fs::read(&args[5])?)?;
        let bytes = if args[1] == "redeem" {
            fs::read(&args[7])?
        } else {
            store.export(*addr.id.as_bytes())?
        };
        let addr = if args[1] == "unreachable" {
            EndpointAddr::new(addr.id)
        } else {
            addr
        };
        let conn = timeout(Duration::from_secs(5), ep.connect(addr, ALPN)).await??;
        let (mut send, mut recv) = conn.open_bi().await?;
        let body = if args[1] == "truncate" {
            &bytes[..bytes.len() / 2]
        } else {
            &bytes
        };
        send.write_all(body).await?;
        send.finish()?;
        let ack = timeout(Duration::from_secs(10), recv.read_to_end(32)).await??;
        if ack.as_slice() != digest(&bytes) {
            return Err("no durable ACK".into());
        }
        conn.close(0u8.into(), b"complete");
    } else {
        return Err("mode".into());
    }
    ep.close().await;
    Ok(())
}
/// Runs one synthetic process scenario; never prints transport payloads on failure.
pub fn main_entry(args: &[String]) -> i32 {
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    if args.len() < 3
        || (args[1] != "relay" && args.len() < 8)
        || runtime.block_on(timeout_run(args))
    {
        eprintln!("synthetic network scenario did not complete");
        1
    } else {
        0
    }
}
async fn timeout_run(args: &[String]) -> bool {
    timeout(Duration::from_secs(45), run(args))
        .await
        .map_or(true, |r| r.is_err())
}
