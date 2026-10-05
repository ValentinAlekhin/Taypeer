//! Each copy owns a Wayland source; destroying that source never clears a later foreign copy.
use super::Shared;
use crate::Event;
use rand_core::RngCore;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, Write},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    globals::registry_queue_init,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1 as ed, ext_data_control_manager_v1 as em,
    ext_data_control_offer_v1 as eo, ext_data_control_source_v1 as es,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1 as wd, zwlr_data_control_manager_v1 as wm,
    zwlr_data_control_offer_v1 as wo, zwlr_data_control_source_v1 as ws,
};
use zeroize::Zeroizing;
const TEXT: &str = "text/plain;charset=utf-8";
const SENSITIVE: &str = "x-kde-passwordManagerHint";
pub(super) struct Command {
    pub text: Zeroizing<String>,
    pub secret: bool,
    pub notify: bool,
    pub seconds: Option<u32>,
}
enum Native {
    Ext(em::ExtDataControlManagerV1, ed::ExtDataControlDeviceV1),
    Wlr(wm::ZwlrDataControlManagerV1, wd::ZwlrDataControlDeviceV1),
}
enum Source {
    Ext(es::ExtDataControlSourceV1),
    Wlr(ws::ZwlrDataControlSourceV1),
}
impl Source {
    fn destroy(self) {
        match self {
            Self::Ext(source) => source.destroy(),
            Self::Wlr(source) => source.destroy(),
        }
    }
}
enum Offer {
    Ext(eo::ExtDataControlOfferV1),
    Wlr(wo::ZwlrDataControlOfferV1),
}
impl Offer {
    fn destroy(self) {
        match self {
            Self::Ext(offer) => offer.destroy(),
            Self::Wlr(offer) => offer.destroy(),
        }
    }
}
struct OwnedCopy {
    id: u64,
    marker: String,
    text: Zeroizing<String>,
    secret: bool,
    deadline: Option<Instant>,
    source: Source,
}
struct Clipboard {
    native: Option<Native>,
    current: Option<OwnedCopy>,
    offers: HashMap<wayland_client::backend::ObjectId, (Offer, Vec<String>)>,
    selected: Option<wayland_client::backend::ObjectId>,
    next_id: u64,
    finished: bool,
}
impl Clipboard {
    fn discard(&mut self) {
        if let Some(copy) = self.current.take() {
            copy.source.destroy();
        }
    }
    fn cancelled(&mut self, id: u64) {
        if self.current.as_ref().is_some_and(|copy| copy.id == id) {
            self.discard();
        }
    }
    fn expired(&self, now: Instant) -> bool {
        self.current
            .as_ref()
            .and_then(|copy| copy.deadline)
            .is_some_and(|deadline| now >= deadline)
    }
    fn send(&self, id: u64, mime: &str, fd: std::os::fd::OwnedFd) {
        let Some(copy) = self.current.as_ref().filter(|copy| copy.id == id) else {
            return;
        };
        let bytes = if mime == TEXT || mime == "text/plain" {
            Zeroizing::new(copy.text.as_bytes().to_vec())
        } else if mime == SENSITIVE && copy.secret {
            Zeroizing::new(b"secret".to_vec())
        } else {
            Zeroizing::new(Vec::new())
        };
        // Receivers may stop reading. The transfer is bounded independently from the native event loop.
        std::thread::spawn(move || {
            let mut file = File::from(fd);
            if rustix::fs::fcntl_setfl(&file, rustix::fs::OFlags::NONBLOCK).is_err() {
                return;
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut remaining = bytes.as_slice();
            while !remaining.is_empty() && Instant::now() < deadline {
                match file.write(remaining) {
                    Ok(0) => break,
                    Ok(count) => remaining = &remaining[count..],
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
    }
    fn install(&mut self, command: Command, qh: &QueueHandle<Self>) -> io::Result<u64> {
        self.discard();
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let mut nonce = [0u8; 16];
        rand_core::OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| io::Error::other("clipboard ownership randomness unavailable"))?;
        let marker = format!(
            "application/x-taypeer-owned-{}",
            nonce
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let source = match self
            .native
            .as_ref()
            .ok_or_else(|| io::Error::other("clipboard unavailable"))?
        {
            Native::Ext(manager, device) => {
                let source = manager.create_data_source(qh, id);
                source.offer(TEXT.into());
                source.offer("text/plain".into());
                source.offer(marker.clone());
                if command.secret {
                    source.offer(SENSITIVE.into());
                }
                device.set_selection(Some(&source));
                Source::Ext(source)
            }
            Native::Wlr(manager, device) => {
                let source = manager.create_data_source(qh, id);
                source.offer(TEXT.into());
                source.offer("text/plain".into());
                source.offer(marker.clone());
                if command.secret {
                    source.offer(SENSITIVE.into());
                }
                device.set_selection(Some(&source));
                Source::Wlr(source)
            }
        };
        let deadline = if command.secret {
            command
                .seconds
                .map(|seconds| Instant::now() + Duration::from_secs(u64::from(seconds)))
        } else {
            None
        };
        self.current = Some(OwnedCopy {
            id,
            marker,
            text: command.text,
            secret: command.secret,
            deadline,
            source,
        });
        Ok(id)
    }
    fn confirmed(&self, id: u64) -> bool {
        let Some(copy) = self.current.as_ref().filter(|copy| copy.id == id) else {
            return false;
        };
        self.selected
            .as_ref()
            .and_then(|selected| self.offers.get(selected))
            .is_some_and(|(_, mimes)| mimes.contains(&copy.marker))
    }
}
pub(super) fn start(shared: Arc<Shared>) -> io::Result<mpsc::SyncSender<Command>> {
    start_connection(
        Connection::connect_to_env().map_err(io::Error::other)?,
        shared,
    )
}
fn start_connection(
    connection: Connection,
    shared: Arc<Shared>,
) -> io::Result<mpsc::SyncSender<Command>> {
    let (mut queue, mut state) = connect(&connection)?;
    let qh = queue.handle();
    let (send, commands) = mpsc::sync_channel::<Command>(8);
    std::thread::spawn(move || {
        loop {
            match commands.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => {
                    let notify = command.notify;
                    if shared.failed.load(std::sync::atomic::Ordering::Acquire)
                        || !shared.available.load(std::sync::atomic::Ordering::Acquire)
                    {
                        let _ = shared.send.send(Event::Clipboard {
                            success: false,
                            notify,
                        });
                        continue;
                    }
                    let id = state.install(command, &qh);
                    let acknowledged = queue.roundtrip(&mut state).is_ok();
                    let success = acknowledged && id.is_ok_and(|id| state.confirmed(id));
                    let _ = shared.send.send(Event::Clipboard { success, notify });
                    if !acknowledged {
                        break;
                    }
                    if !success {
                        state.discard();
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if queue.roundtrip(&mut state).is_err() || state.finished {
                break;
            }
            if state.expired(Instant::now()) {
                state.discard();
            }
        }
        state.discard();
        // Protocol loss revokes protected interaction; no GPUI frame is needed.
        shared.suspend(taypeer_runtime::session::LockReason::HostExited);
    });
    Ok(send)
}
fn connect(
    connection: &Connection,
) -> io::Result<(wayland_client::EventQueue<Clipboard>, Clipboard)> {
    let (globals, mut queue) =
        registry_queue_init::<Clipboard>(connection).map_err(io::Error::other)?;
    let qh = queue.handle();
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ())
        .map_err(io::Error::other)?;
    let native =
        if let Ok(manager) = globals.bind::<em::ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()) {
            let device = manager.get_data_device(&seat, &qh, ());
            Native::Ext(manager, device)
        } else {
            let manager = globals
                .bind::<wm::ZwlrDataControlManagerV1, _, _>(&qh, 1..=2, ())
                .map_err(io::Error::other)?;
            let device = manager.get_data_device(&seat, &qh, ());
            Native::Wlr(manager, device)
        };
    let mut state = Clipboard {
        native: Some(native),
        current: None,
        offers: HashMap::new(),
        selected: None,
        next_id: 1,
        finished: false,
    };
    queue.roundtrip(&mut state).map_err(io::Error::other)?;
    Ok((queue, state))
}
impl Dispatch<wl_registry::WlRegistry, wayland_client::globals::GlobalListContents> for Clipboard {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Clipboard: ignore wl_seat::WlSeat);
delegate_noop!(Clipboard: ignore em::ExtDataControlManagerV1);
delegate_noop!(Clipboard: ignore wm::ZwlrDataControlManagerV1);
macro_rules! devices {
    ($device:ty, $event:path, $offer:ty, $variant:ident) => {
        impl Dispatch<$device, ()> for Clipboard {
            fn event(state: &mut Self, _: &$device, event: $event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                use $event as E;
                match event {
                    E::DataOffer { id } => { state.offers.insert(id.id(), (Offer::$variant(id), Vec::new())); }
                    E::Selection { id } => {
                        if let Some(previous) = state.selected.take() { if let Some((offer, _)) = state.offers.remove(&previous) { offer.destroy(); } }
                        state.selected = id.map(|offer| offer.id());
                    }
                    E::PrimarySelection { id: Some(id) } => { state.offers.remove(&id.id()); id.destroy(); }
                    E::Finished => state.finished = true,
                    _ => {}
                }
            }
            wayland_client::event_created_child!(Clipboard, $device, [0 => ($offer, ())]);
        }
    }
}
devices!(
    ed::ExtDataControlDeviceV1,
    ed::Event,
    eo::ExtDataControlOfferV1,
    Ext
);
devices!(
    wd::ZwlrDataControlDeviceV1,
    wd::Event,
    wo::ZwlrDataControlOfferV1,
    Wlr
);
macro_rules! offers {
    ($offer:ty, $event:path) => {
        impl Dispatch<$offer, ()> for Clipboard {
            fn event(
                state: &mut Self,
                offer: &$offer,
                event: $event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                use $event as E;
                if let E::Offer { mime_type } = event
                    && (mime_type == SENSITIVE
                        || mime_type.starts_with("application/x-taypeer-owned-"))
                    && mime_type.len() <= 128
                    && let Some((_, mimes)) = state.offers.get_mut(&offer.id())
                    && mimes.len() < 64
                {
                    // Ownership and the sensitivity hint are the only metadata this adapter needs.
                    mimes.push(mime_type);
                }
            }
        }
    };
}
offers!(eo::ExtDataControlOfferV1, eo::Event);
offers!(wo::ZwlrDataControlOfferV1, wo::Event);
macro_rules! sources {
    ($source:ty, $event:path) => {
        impl Dispatch<$source, u64> for Clipboard {
            fn event(
                state: &mut Self,
                _: &$source,
                event: $event,
                id: &u64,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                use $event as E;
                match event {
                    E::Send { mime_type, fd } => state.send(*id, &mime_type, fd),
                    E::Cancelled => state.cancelled(*id),
                    _ => {}
                }
            }
        }
    };
}
sources!(es::ExtDataControlSourceV1, es::Event);
sources!(ws::ZwlrDataControlSourceV1, ws::Event);

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
