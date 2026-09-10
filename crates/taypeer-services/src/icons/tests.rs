//! Loopback-only synthetic HTTP responses. No public website is contacted.
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><path d="M1 1 L20 20" stroke="#336699"/></svg>"##;

struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn start(response: fn(&str) -> Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let request_log = requests.clone();
        let stopped = stop.clone();
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        let mut byte = [0];
                        while bytes.len() < 8192 && !bytes.ends_with(b"\r\n\r\n") {
                            if socket.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            bytes.push(byte[0]);
                        }
                        let request = String::from_utf8_lossy(&bytes).into_owned();
                        let response = response(&request);
                        request_log.lock().unwrap().push(request);
                        let _ = socket.write_all(&response);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn response(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn explicitly_requested_local_favicon_follows_html_without_credentials() {
    let server = Server::start(|request| {
        if request.starts_with("GET /page ") {
            response(
                r#"<html><head><script>var ignored = '<link rel="icon" href="/fake.svg">';</script><base href="/assets/"><link href="public.svg" rel="shortcut icon"></head></html>"#,
            )
        } else {
            response(SVG)
        }
    });
    let bytes = favicon(&format!("{}/page", server.url)).unwrap();
    assert_eq!(bytes.as_slice(), SVG.as_bytes());
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /page "));
    assert!(requests[1].starts_with("GET /assets/public.svg "));
    for request in requests.iter() {
        let request = request.to_ascii_lowercase();
        assert!(!request.contains("authorization:"));
        assert!(!request.contains("cookie:"));
        assert!(!request.contains("referer:"));
    }
}

#[test]
fn redirect_loop_and_oversized_body_are_bounded() {
    let server = Server::start(|_| {
        b"HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    });
    assert_eq!(from_url(&server.url).unwrap_err(), IconError::Network);
    assert_eq!(server.requests.lock().unwrap().len(), 6);
    let large = Server::start(|_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".to_vec()
    });
    assert_eq!(from_url(&large.url).unwrap_err(), IconError::TooLarge);
}

#[test]
fn image_validation_rejects_oversized_images_external_svg_and_active_content() {
    validate(SVG.as_bytes()).unwrap();
    for svg in [
        r#"<svg xmlns="http://www.w3.org/2000/svg"><image href="file:///PUBLIC.svg"/></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>PUBLIC</script></svg>"#,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"/>"#,
        r#"<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///PUBLIC'>]><svg>&x;</svg>"#,
    ] {
        assert!(validate(svg.as_bytes()).is_err());
    }
    assert_eq!(
        validate(&vec![0; 1024 * 1024 + 1]),
        Err(IconError::TooLarge)
    );
    assert_eq!(
        validate(b"PUBLIC not an image"),
        Err(IconError::InvalidImage)
    );
}

#[test]
fn every_bundled_key_has_a_valid_local_svg() {
    for key in taypeer_core::LUCIDE_KEYS {
        let icon = taypeer_core::LucideKey::try_from((*key).to_owned()).unwrap();
        validate(icon.svg().as_bytes()).unwrap();
    }
    assert!(serde_json::from_str::<taypeer_core::LucideKey>(r#""PUBLIC invalid""#).is_err());
}

#[test]
fn group_batch_skips_existing_icons_and_keeps_successful_siblings_on_retry() {
    use crate::{
        BinaryEdit, BinaryRequest, BinaryTarget, DatabaseService, EditableEntry, IconInput,
    };
    use taypeer_core::OperationId;
    let server = Server::start(|request| {
        if request.starts_with("GET /page ") {
            response(r#"<link rel="icon" href="/PUBLIC.svg">"#)
        } else {
            response(SVG)
        }
    });
    let mut service = DatabaseService::new();
    let database = service.create_database("PUBLIC batch").unwrap();
    let session = service.unlock(&database, crate::DEMO_PASSWORD).unwrap();
    let parent = service
        .create_group(&session, "PUBLIC parent".into(), None)
        .unwrap()
        .value
        .id;
    let child = service
        .create_group(&session, "PUBLIC child".into(), Some(parent.clone()))
        .unwrap()
        .value
        .id;
    for (index, group) in [&parent, &parent, &parent, &child].into_iter().enumerate() {
        service.start_create_entry(&session, group.clone()).unwrap();
        service
            .update_draft(
                &session,
                EditableEntry {
                    title: format!("PUBLIC {index}"),
                    url: (index != 1).then(|| format!("{}/page", server.url)),
                    ..Default::default()
                },
            )
            .unwrap();
        let entry = service.save_draft(&session).unwrap().value;
        if index == 2 {
            service
                .edit_binary(
                    &session,
                    &BinaryRequest {
                        target: BinaryTarget::Entry(entry),
                        edit: BinaryEdit::Icon(IconInput::Lucide(
                            "key-round".to_owned().try_into().unwrap(),
                        )),
                        review: None,
                    },
                    &OperationId::new("PUBLIC custom"),
                )
                .unwrap();
        }
    }
    let op = OperationId::new("PUBLIC batch");
    let results = service
        .group_favicons(&session, &parent, false, false, &op)
        .unwrap()
        .value;
    assert_eq!(results.len(), 3);
    assert_eq!(results.iter().filter(|r| r.error.is_some()).count(), 1);
    assert_eq!(results.iter().filter(|r| r.skipped).count(), 1);
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    service
        .group_favicons(&session, &parent, false, false, &op)
        .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 2);
    service
        .group_favicons(
            &session,
            &parent,
            true,
            false,
            &OperationId::new("PUBLIC recursive"),
        )
        .unwrap();
    assert_eq!(server.requests.lock().unwrap().len(), 4);
}
