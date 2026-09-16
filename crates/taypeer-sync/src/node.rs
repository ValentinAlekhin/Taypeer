use crate::{
    Backend, Command, EndpointAddr, Error, RelayUrl, Reply,
    wire::{self, Request},
};
use iroh::{
    Endpoint, RelayMode, SecretKey,
    endpoint::{Connection, RecvStream, SendStream, presets},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::Duration,
};
use taypeer_core::DatabaseId;
use taypeer_storage::ObjectReader;
use taypeer_trust::{CipherObject, Digest, PublicKey, TransportKey};
use tempfile::NamedTempFile;
use tokio::{
    io::AsyncWriteExt,
    task::{JoinHandle, JoinSet},
    time::timeout,
};

const ALPN: &[u8] = b"taypeer/sync/1";
const CONNECT: Duration = Duration::from_secs(15);
const MAX_CONNECTIONS: usize = 16;

/// Device-local relay preference. Product TLS verification cannot be disabled.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "url", rename_all = "snake_case")]
pub enum RelaySetting {
    /// Iroh's public relays, with direct path establishment where possible.
    #[default]
    Default,
    /// Direct connections only; unreachable peers remain unsynchronized.
    Disabled,
    /// Explicit user-selected relay with normal TLS verification.
    Custom(RelayUrl),
    /// Explicit relay with optional DER CA root and no direct IP transport.
    /// Certificate chains and host names are still verified by Rustls.
    RelayOnly {
        /// Explicit HTTPS relay route.
        url: RelayUrl,
        /// Optional private CA certificate in DER encoding (at most 64 KiB).
        root_der: Option<Vec<u8>>,
    },
}

/// Receipt counts refer to durable ciphertext objects, never applied document changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExchangeReport {
    /// Objects newly requested and durably acknowledged by the peer.
    pub sent: u64,
    /// Objects fetched and durably acknowledged locally.
    pub received: u64,
}

type ExchangeGates =
    std::collections::BTreeMap<(DatabaseId, [u8; 32]), Arc<tokio::sync::Mutex<()>>>;

/// One listening endpoint owned by the live CLI host. It contains no author/read key.
pub struct Node {
    endpoint: Endpoint,
    backend: Arc<dyn Backend>,
    task: Mutex<Option<JoinHandle<()>>>,
    exchanges: Mutex<ExchangeGates>,
}
impl Node {
    /// Bind the profile's persistent transport identity and start accepting exchanges.
    pub async fn bind(
        key: &TransportKey,
        relay: &RelaySetting,
        backend: Arc<dyn Backend>,
    ) -> Result<Self, Error> {
        let mode = match relay {
            RelaySetting::Default => RelayMode::Default,
            RelaySetting::Disabled => RelayMode::Disabled,
            RelaySetting::Custom(url) | RelaySetting::RelayOnly { url, .. } => {
                RelayMode::Custom(url.clone().into())
            }
        };
        let seed = key.secret_seed();
        let builder = match relay {
            RelaySetting::Default => Endpoint::builder(presets::N0),
            RelaySetting::Disabled | RelaySetting::Custom(_) | RelaySetting::RelayOnly { .. } => {
                Endpoint::builder(presets::Minimal)
            }
        };
        let builder = if let RelaySetting::RelayOnly { url, root_der } = relay {
            if !url.as_str().starts_with("https://") {
                return Err(Error::Protocol);
            }
            let builder = builder.clear_ip_transports();
            if let Some(root) = root_der {
                if root.is_empty() || root.len() > 64 * 1024 {
                    return Err(Error::Protocol);
                }
                builder.ca_tls_config(iroh_relay::tls::CaTlsConfig::custom_roots([root
                    .clone()
                    .into()]))
            } else {
                builder
            }
        } else {
            builder
        };
        let endpoint = builder
            .secret_key(SecretKey::from_bytes(&seed))
            .relay_mode(mode)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .map_err(|_| Error::Transport)?;
        Ok(Self::from_endpoint(endpoint, backend))
    }
    fn from_endpoint(endpoint: Endpoint, backend: Arc<dyn Backend>) -> Self {
        let listener = endpoint.clone();
        let handler = Arc::clone(&backend);
        let task = tokio::spawn(async move { serve(listener, handler).await });
        Self {
            endpoint,
            backend,
            task: Mutex::new(Some(task)),
            exchanges: Mutex::new(Default::default()),
        }
    }
    /// Current routes, suitable for an invitation or an admitted peer's address book.
    pub fn address(&self) -> EndpointAddr {
        self.endpoint.addr()
    }
    /// Wait for relay connectivity when an externally usable invitation route is needed.
    pub async fn online(&self) -> Result<(), Error> {
        timeout(CONNECT, self.endpoint.online())
            .await
            .map_err(|_| Error::Timeout)?;
        Ok(())
    }
    /// Explicit close waits for transport shutdown and terminates outstanding handlers.
    pub async fn close(&self) {
        self.endpoint.close().await;
        let task = self.task.lock().ok().and_then(|mut task| task.take());
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
    async fn connect(&self, address: EndpointAddr) -> Result<Connection, Error> {
        timeout(CONNECT, self.endpoint.connect(address, ALPN))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Transport)
    }
    /// Send an explicit control request, including one-time admission presentation.
    pub async fn request(&self, address: EndpointAddr, command: Command) -> Result<Reply, Error> {
        let connection = self.connect(address).await?;
        let result = rpc(&connection, command).await;
        connection.close(0_u8.into(), b"complete");
        result
    }
    /// Exchange missing ciphertext in both directions. Only complete durable objects
    /// are counted; callers report application/conflicts using the unlocked service.
    pub async fn exchange(
        &self,
        address: EndpointAddr,
        database: DatabaseId,
    ) -> Result<ExchangeReport, Error> {
        let gate = {
            let mut gates = self.exchanges.lock().map_err(|_| Error::State)?;
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
            Arc::clone(
                gates
                    .entry((database.clone(), *address.id.as_bytes()))
                    .or_default(),
            )
        };
        let _attempt = gate.lock().await;
        let connection = self.connect(address.clone()).await?;
        let result = self
            .exchange_connected(&connection, address, database)
            .await;
        connection.close(0_u8.into(), b"complete");
        result
    }
    async fn exchange_connected(
        &self,
        connection: &Connection,
        peer_address: EndpointAddr,
        database: DatabaseId,
    ) -> Result<ExchangeReport, Error> {
        let peer = public(connection);
        let backend = Arc::clone(&self.backend);
        let local_command = Command::Inventory {
            database: database.clone(),
            address: peer_address,
        };
        let local = match blocking(move || backend.command(peer, local_command)).await? {
            Reply::Inventory(metadata) => metadata,
            _ => return Err(Error::Protocol),
        };
        let needed = match rpc(connection, Command::Offer(local)).await? {
            Reply::Needed(ids) => ids,
            _ => return Err(Error::Protocol),
        };
        let mut report = ExchangeReport::default();
        for id in needed {
            let backend = Arc::clone(&self.backend);
            let db = database.clone();
            let (descriptor, reader) = blocking(move || backend.object(peer, &db, id)).await?;
            put(connection, database.clone(), descriptor, reader).await?;
            report.sent += 1;
        }
        let remote = match rpc(
            connection,
            Command::Inventory {
                database: database.clone(),
                address: self.address(),
            },
        )
        .await?
        {
            Reply::Inventory(metadata) => metadata,
            _ => return Err(Error::Protocol),
        };
        let descriptors = remote.manifest.body.objects.clone();
        let backend = Arc::clone(&self.backend);
        let needed = match blocking(move || backend.command(peer, Command::Offer(remote))).await? {
            Reply::Needed(ids) => ids,
            _ => return Err(Error::Protocol),
        };
        for id in needed {
            let descriptor = descriptors.get(&id).ok_or(Error::Protocol)?.clone();
            let backend = Arc::clone(&self.backend);
            let db = database.clone();
            let check = descriptor.clone();
            blocking(move || backend.authorize_object(peer, &db, &check)).await?;
            let file = fetch(connection, database.clone(), &descriptor).await?;
            let backend = Arc::clone(&self.backend);
            let db = database.clone();
            let receipt = blocking(move || backend.receive(peer, &db, descriptor, file)).await?;
            if receipt != id {
                return Err(Error::Protocol);
            }
            report.received += 1;
        }
        Ok(report)
    }
    /// Fetch a fully admitted initial inventory without needing a local database writer.
    /// The caller owns durable initial-download staging and validates the final manifest.
    pub async fn download_object(
        &self,
        address: EndpointAddr,
        database: DatabaseId,
        descriptor: CipherObject,
    ) -> Result<NamedTempFile, Error> {
        let connection = self.connect(address).await?;
        let result = fetch(&connection, database, &descriptor).await;
        connection.close(0_u8.into(), b"complete");
        result
    }
}

fn public(connection: &Connection) -> PublicKey {
    PublicKey::from_bytes(*connection.remote_id().as_bytes())
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| Error::State)?
}

async fn serve(endpoint: Endpoint, backend: Arc<dyn Backend>) {
    let mut handlers = JoinSet::new();
    loop {
        tokio::select! {
            _ = handlers.join_next(), if !handlers.is_empty() => {},
            incoming = endpoint.accept(), if handlers.len() < MAX_CONNECTIONS => {
                let Some(incoming) = incoming else { break; };
                let backend = Arc::clone(&backend);
                handlers.spawn(async move {
                    let connection = match timeout(CONNECT, incoming).await {
                        Ok(Ok(connection)) => connection,
                        _ => return,
                    };
                    // A malformed/failed stream closes this exchange without a success receipt.
                    if serve_connection(&connection, backend).await.is_err() {
                        connection.close(1_u8.into(), b"exchange failed");
                    }
                });
            }
        }
    }
    handlers.abort_all();
}
async fn serve_connection(connection: &Connection, backend: Arc<dyn Backend>) -> Result<(), Error> {
    loop {
        let (mut send, mut recv) = match timeout(wire::IDLE, connection.accept_bi()).await {
            Ok(Ok(streams)) => streams,
            Ok(Err(_)) => return Ok(()),
            Err(_) => return Err(Error::Timeout),
        };
        let request: Request = wire::read(&mut recv).await?;
        let peer = public(connection);
        match request {
            Request::Command(command) => {
                wire::eof(&mut recv).await?;
                let handler = Arc::clone(&backend);
                let result = blocking(move || handler.command(peer, *command)).await;
                wire::write(&mut send, &result).await?;
            }
            Request::Fetch { database, id } => {
                wire::eof(&mut recv).await?;
                let handler = Arc::clone(&backend);
                match blocking(move || handler.object(peer, &database, id)).await {
                    Ok((descriptor, reader)) => {
                        wire::write(&mut send, &Ok::<_, Error>(&descriptor)).await?;
                        send_body(&mut send, reader).await?;
                    }
                    Err(error) => wire::write(&mut send, &Err::<CipherObject, _>(error)).await?,
                }
            }
            Request::Put {
                database,
                descriptor,
            } => {
                let handler = Arc::clone(&backend);
                let db = database.clone();
                let check = descriptor.clone();
                blocking(move || handler.authorize_object(peer, &db, &check)).await?;
                let file = receive_body(&mut recv, descriptor.length).await?;
                let handler = Arc::clone(&backend);
                let result =
                    blocking(move || handler.receive(peer, &database, descriptor, file)).await;
                wire::write(&mut send, &result).await?;
            }
        }
        send.finish().map_err(|_| Error::Transport)?;
    }
}
async fn rpc(connection: &Connection, command: Command) -> Result<Reply, Error> {
    let (mut send, mut recv) = connection.open_bi().await.map_err(|_| Error::Transport)?;
    wire::write(&mut send, &Request::Command(Box::new(command))).await?;
    send.finish().map_err(|_| Error::Transport)?;
    let reply: Result<Reply, Error> = wire::read(&mut recv).await?;
    wire::eof(&mut recv).await?;
    reply
}
async fn put(
    connection: &Connection,
    database: DatabaseId,
    descriptor: CipherObject,
    reader: ObjectReader,
) -> Result<(), Error> {
    let (mut send, mut recv) = connection.open_bi().await.map_err(|_| Error::Transport)?;
    let id = descriptor.digest;
    wire::write(
        &mut send,
        &Request::Put {
            database,
            descriptor,
        },
    )
    .await?;
    send_body(&mut send, reader).await?;
    send.finish().map_err(|_| Error::Transport)?;
    let receipt: Result<Digest, Error> = wire::read(&mut recv).await?;
    wire::eof(&mut recv).await?;
    if receipt? != id {
        return Err(Error::Protocol);
    }
    Ok(())
}
async fn fetch(
    connection: &Connection,
    database: DatabaseId,
    descriptor: &CipherObject,
) -> Result<NamedTempFile, Error> {
    let (mut send, mut recv) = connection.open_bi().await.map_err(|_| Error::Transport)?;
    wire::write(
        &mut send,
        &Request::Fetch {
            database,
            id: descriptor.digest,
        },
    )
    .await?;
    send.finish().map_err(|_| Error::Transport)?;
    let actual: Result<CipherObject, Error> = wire::read(&mut recv).await?;
    if actual? != *descriptor {
        return Err(Error::Protocol);
    }
    receive_body(&mut recv, descriptor.length).await
}
async fn send_body(send: &mut SendStream, mut reader: ObjectReader) -> Result<(), Error> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let producer = tokio::task::spawn_blocking(move || {
        loop {
            let mut chunk = vec![0; wire::CHUNK];
            let count = reader
                .read(&mut chunk)
                .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
            if count == 0 {
                break;
            }
            chunk.truncate(count);
            tx.blocking_send(chunk).map_err(|_| Error::Transport)?;
        }
        Ok::<_, Error>(())
    });
    while let Some(bytes) = rx.recv().await {
        timeout(wire::IDLE, send.write_all(&bytes))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Transport)?;
    }
    producer.await.map_err(|_| Error::State)?
}
async fn receive_body(recv: &mut RecvStream, length: u64) -> Result<NamedTempFile, Error> {
    if length == 0 || length > wire::MAX_OBJECT {
        return Err(Error::Protocol);
    }
    let temp = NamedTempFile::new().map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
    let handle = temp
        .reopen()
        .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
    let mut output = tokio::fs::File::from_std(handle);
    let mut left = length;
    let mut buffer = vec![0; wire::CHUNK];
    while left != 0 {
        let size = left.min(buffer.len() as u64) as usize;
        let count = timeout(wire::IDLE, recv.read(&mut buffer[..size]))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::Transport)?
            .ok_or(Error::Protocol)?;
        if count == 0 {
            return Err(Error::Protocol);
        }
        output
            .write_all(&buffer[..count])
            .await
            .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
        left -= count as u64;
    }
    wire::eof(recv).await?;
    output
        .flush()
        .await
        .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
    output
        .sync_all()
        .await
        .map_err(|_| Error::Storage(taypeer_storage::Error::Io))?;
    Ok(temp)
}

#[cfg(test)]
mod tests;

impl Drop for Node {
    fn drop(&mut self) {
        if let Ok(task) = self.task.get_mut()
            && let Some(task) = task.take()
        {
            task.abort();
        }
    }
}
