//! Native protocol scenarios use a private socket pair, never the system clipboard.
use super::*;
use crate::{Event, state::PlatformState};
use std::{
    collections::VecDeque,
    os::{fd::AsFd, unix::net::UnixStream},
    sync::Mutex,
};
use wayland_protocols::ext::data_control::v1::server::{
    ext_data_control_device_v1 as sed, ext_data_control_manager_v1 as sem,
    ext_data_control_offer_v1 as seo, ext_data_control_source_v1 as ses,
};
use wayland_protocols_wlr::data_control::v1::server::{
    zwlr_data_control_device_v1 as swd, zwlr_data_control_manager_v1 as swm,
    zwlr_data_control_offer_v1 as swo, zwlr_data_control_source_v1 as sws,
};
use wayland_server::{
    Client, DataInit, Dispatch as ServerDispatch, Display, DisplayHandle, GlobalDispatch, New,
    Resource, protocol::wl_seat as seat,
};

#[derive(Clone)]
enum ServerSource {
    Ext(ses::ExtDataControlSourceV1),
    Wlr(sws::ZwlrDataControlSourceV1),
}
impl ServerSource {
    fn id(&self) -> wayland_server::backend::ObjectId {
        match self {
            Self::Ext(source) => source.id(),
            Self::Wlr(source) => source.id(),
        }
    }
    fn cancel(&self) {
        match self {
            Self::Ext(source) => source.cancelled(),
            Self::Wlr(source) => source.cancelled(),
        }
    }
    fn send(&self, mime: &str, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Self::Ext(source) => source.send(mime.into(), fd),
            Self::Wlr(source) => source.send(mime.into(), fd),
        }
    }
}
enum ServerDevice {
    Ext(sed::ExtDataControlDeviceV1),
    Wlr(swd::ZwlrDataControlDeviceV1),
}
enum Request {
    Snapshot(mpsc::Sender<Snapshot>),
    Foreign(mpsc::Sender<()>),
    Read(&'static str, UnixStream),
    RejectNext(mpsc::Sender<()>),
    Stop,
}
struct Snapshot {
    owned: bool,
    foreign: bool,
    mimes: Vec<String>,
    destroyed: usize,
}
#[derive(Default)]
struct Server {
    device: Option<ServerDevice>,
    source: Option<ServerSource>,
    mimes: HashMap<wayland_server::backend::ObjectId, Vec<String>>,
    foreign: bool,
    destroyed: usize,
    reject_next: bool,
}
impl Server {
    fn advertise(&self, handle: &DisplayHandle, mimes: &[String]) {
        match self.device.as_ref().expect("test device") {
            ServerDevice::Ext(device) => {
                let offer = device
                    .client()
                    .expect("test client")
                    .create_resource::<seo::ExtDataControlOfferV1, _, Self>(handle, 1, ())
                    .expect("offer");
                device.data_offer(&offer);
                for mime in mimes {
                    offer.offer(mime.clone());
                }
                device.selection(Some(&offer));
            }
            ServerDevice::Wlr(device) => {
                let offer = device
                    .client()
                    .expect("test client")
                    .create_resource::<swo::ZwlrDataControlOfferV1, _, Self>(handle, 1, ())
                    .expect("offer");
                device.data_offer(&offer);
                for mime in mimes {
                    offer.offer(mime.clone());
                }
                device.selection(Some(&offer));
            }
        }
    }
    fn replace(&mut self, source: Option<ServerSource>, handle: &DisplayHandle) {
        if self.reject_next {
            self.reject_next = false;
            if let Some(source) = source {
                source.cancel();
            }
            return;
        }
        if let Some(old) = self.source.take() {
            old.cancel();
        }
        self.source = source;
        self.foreign = false;
        if let Some(source) = &self.source {
            self.advertise(handle, &self.mimes[&source.id()]);
        }
    }
    fn destroy(&mut self, id: wayland_server::backend::ObjectId) {
        self.destroyed += 1;
        if self.source.as_ref().is_some_and(|source| source.id() == id) {
            self.source.take();
        }
        self.mimes.remove(&id);
    }
}
macro_rules! global {
    ($interface:ty) => {
        impl GlobalDispatch<$interface, ()> for Server {
            fn bind(
                _: &mut Self,
                _: &DisplayHandle,
                _: &Client,
                resource: New<$interface>,
                _: &(),
                init: &mut DataInit<'_, Self>,
            ) {
                init.init(resource, ());
            }
        }
    };
}
global!(seat::WlSeat);
global!(sem::ExtDataControlManagerV1);
global!(swm::ZwlrDataControlManagerV1);
macro_rules! no_requests {
    ($interface:ty, $request:ty) => {
        impl ServerDispatch<$interface, ()> for Server {
            fn request(
                _: &mut Self,
                _: &Client,
                _: &$interface,
                _: $request,
                _: &(),
                _: &DisplayHandle,
                _: &mut DataInit<'_, Self>,
            ) {
            }
        }
    };
}
no_requests!(seat::WlSeat, seat::Request);
no_requests!(seo::ExtDataControlOfferV1, seo::Request);
no_requests!(swo::ZwlrDataControlOfferV1, swo::Request);
macro_rules! manager {
    ($manager:ty, $request:path, $variant:ident) => {
        impl ServerDispatch<$manager, ()> for Server {
            fn request(
                state: &mut Self,
                _: &Client,
                _: &$manager,
                request: $request,
                _: &(),
                _: &DisplayHandle,
                init: &mut DataInit<'_, Self>,
            ) {
                use $request as R;
                match request {
                    R::CreateDataSource { id } => {
                        let source = init.init(id, ());
                        state.mimes.insert(source.id(), Vec::new());
                    }
                    R::GetDataDevice { id, .. } => {
                        let device = init.init(id, ());
                        device.selection(None);
                        state.device = Some(ServerDevice::$variant(device));
                    }
                    _ => {}
                }
            }
        }
    };
}
manager!(sem::ExtDataControlManagerV1, sem::Request, Ext);
manager!(swm::ZwlrDataControlManagerV1, swm::Request, Wlr);
macro_rules! device {
    ($device:ty, $request:path, $variant:ident) => {
        impl ServerDispatch<$device, ()> for Server {
            fn request(
                state: &mut Self,
                _: &Client,
                _: &$device,
                request: $request,
                _: &(),
                handle: &DisplayHandle,
                _: &mut DataInit<'_, Self>,
            ) {
                use $request as R;
                if let R::SetSelection { source } = request {
                    state.replace(source.map(ServerSource::$variant), handle);
                }
            }
        }
    };
}
device!(sed::ExtDataControlDeviceV1, sed::Request, Ext);
device!(swd::ZwlrDataControlDeviceV1, swd::Request, Wlr);
macro_rules! source {
    ($source:ty, $request:path) => {
        impl ServerDispatch<$source, ()> for Server {
            fn request(
                state: &mut Self,
                _: &Client,
                source: &$source,
                request: $request,
                _: &(),
                _: &DisplayHandle,
                _: &mut DataInit<'_, Self>,
            ) {
                use $request as R;
                match request {
                    R::Offer { mime_type } => state
                        .mimes
                        .get_mut(&source.id())
                        .expect("source")
                        .push(mime_type),
                    R::Destroy => state.destroy(source.id()),
                    _ => {}
                }
            }
        }
    };
}
source!(ses::ExtDataControlSourceV1, ses::Request);
source!(sws::ZwlrDataControlSourceV1, sws::Request);
struct Fixture {
    copies: mpsc::SyncSender<Command>,
    requests: mpsc::Sender<Request>,
    state: PlatformState,
    pending: Mutex<VecDeque<Event>>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new(ext: bool) -> Self {
        let (client_socket, server_socket) = UnixStream::pair().expect("private protocol socket");
        let (requests, commands) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let mut display = Display::<Server>::new().expect("test display");
            let mut handle = display.handle();
            handle
                .insert_client(server_socket, Arc::new(()))
                .expect("private client");
            handle.create_global::<Server, seat::WlSeat, _>(1, ());
            if ext {
                handle.create_global::<Server, sem::ExtDataControlManagerV1, _>(1, ());
            } else {
                handle.create_global::<Server, swm::ZwlrDataControlManagerV1, _>(2, ());
            }
            let mut state = Server::default();
            loop {
                display
                    .dispatch_clients(&mut state)
                    .expect("dispatch private client");
                display.flush_clients().expect("flush private client");
                match commands.recv_timeout(Duration::from_millis(2)) {
                    Ok(Request::Snapshot(reply)) => {
                        let mimes = state
                            .source
                            .as_ref()
                            .map(|source| state.mimes[&source.id()].clone())
                            .unwrap_or_default();
                        let _ = reply.send(Snapshot {
                            owned: state.source.is_some(),
                            foreign: state.foreign,
                            mimes,
                            destroyed: state.destroyed,
                        });
                    }
                    Ok(Request::Foreign(reply)) => {
                        if let Some(source) = state.source.take() {
                            source.cancel();
                        }
                        state.foreign = true;
                        state.advertise(&handle, &[TEXT.into()]);
                        display.flush_clients().expect("foreign offer");
                        let _ = reply.send(());
                    }
                    Ok(Request::Read(mime, socket)) => {
                        state
                            .source
                            .as_ref()
                            .expect("owned source")
                            .send(mime, socket.as_fd());
                        display.flush_clients().expect("transfer request");
                    }
                    Ok(Request::RejectNext(reply)) => {
                        state.reject_next = true;
                        let _ = reply.send(());
                    }
                    Ok(Request::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        let state = PlatformState::new(true);
        let copies = start_connection(
            Connection::from_socket(client_socket).expect("private connection"),
            state.source(),
        )
        .expect("clipboard adapter");
        Self {
            copies,
            requests,
            state,
            pending: Mutex::new(VecDeque::new()),
            server: Some(server),
        }
    }
    fn copy(&self, seconds: Option<u32>) {
        self.copies
            .send(Command {
                text: Zeroizing::new("PUBLIC-clipboard-synthetic".into()),
                secret: true,
                notify: true,
                seconds,
            })
            .expect("copy queue");
        assert!(matches!(
            self.event(),
            Event::Clipboard {
                success: true,
                notify: true
            }
        ));
    }
    fn event(&self) -> Event {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let mut pending = self.pending.lock().expect("fixture events");
            pending.extend(self.state.drain());
            if let Some(event) = pending.pop_front() {
                return event;
            }
            drop(pending);
            assert!(
                Instant::now() < deadline,
                "compositor acknowledgement deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn snapshot(&self) -> Snapshot {
        let (send, receive) = mpsc::channel();
        self.requests
            .send(Request::Snapshot(send))
            .expect("snapshot request");
        receive
            .recv_timeout(Duration::from_secs(3))
            .expect("snapshot")
    }
    fn wait(&self, condition: impl Fn(&Snapshot) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition(&self.snapshot()) {
            assert!(Instant::now() < deadline, "clipboard condition deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn foreign(&self) {
        let (send, receive) = mpsc::channel();
        self.requests
            .send(Request::Foreign(send))
            .expect("foreign copy");
        receive
            .recv_timeout(Duration::from_secs(3))
            .expect("foreign acknowledgement");
    }
    fn read(&self, mime: &'static str) -> Vec<u8> {
        use std::io::Read;
        let (mut reader, writer) = UnixStream::pair().expect("transfer socket");
        reader
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("transfer deadline");
        self.requests
            .send(Request::Read(mime, writer))
            .expect("transfer request");
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).expect("bounded transfer");
        bytes
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.requests.send(Request::Stop);
        if let Some(server) = self.server.take() {
            server.join().expect("private server shutdown");
        }
    }
}
fn owned_clipboard_scenario(ext: bool) {
    let fixture = Fixture::new(ext);
    fixture.copy(Some(1));
    assert!(
        fixture
            .snapshot()
            .mimes
            .iter()
            .any(|mime| mime == SENSITIVE)
    );
    assert_eq!(fixture.read(SENSITIVE), b"secret");
    assert_eq!(fixture.read(TEXT), b"PUBLIC-clipboard-synthetic");
    fixture.wait(|snapshot| !snapshot.owned && snapshot.destroyed == 1);
    fixture.copy(Some(1));
    fixture.foreign();
    fixture.wait(|snapshot| snapshot.foreign && snapshot.destroyed == 2);
    std::thread::sleep(Duration::from_millis(1100));
    assert!(fixture.snapshot().foreign);
    fixture.copy(None);
    std::thread::sleep(Duration::from_millis(1100));
    assert!(fixture.snapshot().owned);
    fixture.copy(Some(1));
    fixture.wait(|snapshot| !snapshot.owned && snapshot.destroyed == 4);
    let (send, receive) = mpsc::channel();
    fixture
        .requests
        .send(Request::RejectNext(send))
        .expect("deny clipboard ownership");
    receive
        .recv_timeout(Duration::from_secs(3))
        .expect("deny acknowledgement");
    fixture
        .copies
        .send(Command {
            text: Zeroizing::new("PUBLIC-denied-copy".into()),
            secret: true,
            notify: true,
            seconds: Some(1),
        })
        .expect("denied copy queue");
    assert!(matches!(
        fixture.event(),
        Event::Clipboard {
            success: false,
            notify: true
        }
    ));
    fixture.wait(|snapshot| !snapshot.owned && snapshot.destroyed == 5);
}
#[test]
fn ext_confirmed_secret_copy_clears_only_its_source() {
    owned_clipboard_scenario(true);
}
#[test]
fn wlr_fallback_preserves_later_foreign_copy() {
    owned_clipboard_scenario(false);
}

#[test]
#[ignore = "explicit native Wayland/logind acceptance; briefly replaces clipboard with PUBLIC text"]
fn native_wayland_secret_ownership_and_timed_clear() {
    let connection = Connection::connect_to_env().expect("native Wayland connection");
    let (mut observer, mut state) = connect(&connection).expect("native data-control observer");
    println!(
        "Native clipboard protocol: {}",
        match state.native.as_ref().expect("native protocol") {
            Native::Ext(..) => "ext-data-control-v1",
            Native::Wlr(..) => "wlr-data-control-v1",
        }
    );
    let mut platform = crate::linux::Platform::start().expect("native Wayland and logind adapter");
    assert!(
        platform.available(),
        "confirmed active logind session required before native clipboard test"
    );
    platform.set_clipboard_seconds(Some(1));
    platform.copy("PUBLIC-native-clipboard-probe".into(), true, true);
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut acknowledged = false;
    while !acknowledged {
        for event in platform.drain() {
            if let Event::Clipboard { success, .. } = event {
                assert!(success, "native compositor ownership acknowledgement");
                acknowledged = true;
            }
        }
        assert!(
            Instant::now() < deadline,
            "native clipboard acknowledgement deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    observer
        .roundtrip(&mut state)
        .expect("native selection observation");
    let (_, mimes) = state
        .selected
        .as_ref()
        .and_then(|selected| state.offers.get(selected))
        .expect("native owned selection metadata");
    assert!(
        mimes.iter().any(|mime| mime == SENSITIVE),
        "native sensitive MIME hint"
    );
    let marker = mimes
        .iter()
        .find(|mime| mime.starts_with("application/x-taypeer-owned-"))
        .expect("native ownership metadata")
        .clone();
    loop {
        observer
            .roundtrip(&mut state)
            .expect("native clear observation");
        let still_owned = state
            .selected
            .as_ref()
            .and_then(|selected| state.offers.get(selected))
            .is_some_and(|(_, mimes)| mimes.contains(&marker));
        if !still_owned {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "native owned clipboard clearing deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    println!("Confirmed native active session, sensitive copy ownership and timed source clearing");
}
