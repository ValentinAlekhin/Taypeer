//! Resolve once, enforce the network boundary and pin the actual connection addresses.

use super::{IconError, validate};
use html5ever::{
    tendril::StrTendril,
    tokenizer::{
        BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, states::RawKind,
    },
};
use std::{
    io::Read,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::mpsc,
    time::{Duration, Instant},
};
use taypeer_core::ICON_LIMIT;
use url::{Host, Url};
use zeroize::Zeroizing;

const DEADLINE: Duration = Duration::from_secs(30);

struct Download {
    deadline: Instant,
    public_seen: bool,
    redirects: u8,
}

fn parse(input: &str) -> Result<Url, IconError> {
    let mut url = Url::parse(input).map_err(|_| IconError::InvalidUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(IconError::InvalidUrl);
    }
    url.set_fragment(None);
    Ok(url)
}

fn internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.octets()[0] == 0
                || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                || ip.octets()[0] >= 240
        }
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|ip| internal(IpAddr::V4(ip)))
            .unwrap_or_else(|| {
                ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_multicast()
            }),
    }
}

impl Download {
    fn remaining(&self) -> Result<Duration, IconError> {
        self.deadline
            .checked_duration_since(Instant::now())
            .ok_or(IconError::Timeout)
    }
    fn addresses(&mut self, url: &Url) -> Result<Vec<SocketAddr>, IconError> {
        let port = url.port_or_known_default().ok_or(IconError::InvalidUrl)?;
        let addresses = match url.host().ok_or(IconError::InvalidUrl)? {
            Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
            Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
            Host::Domain(host) => {
                let host = host.to_owned();
                let (send, receive) = mpsc::sync_channel(1);
                std::thread::spawn(move || {
                    let result = (host.as_str(), port)
                        .to_socket_addrs()
                        .map(|addresses| addresses.take(16).collect::<Vec<_>>());
                    let _ = send.send(result);
                });
                receive
                    .recv_timeout(self.remaining()?)
                    .map_err(|_| IconError::Timeout)?
                    .map_err(|_| IconError::Network)?
            }
        };
        if addresses.is_empty() {
            return Err(IconError::Network);
        }
        let has_internal = addresses.iter().any(|address| internal(address.ip()));
        if self.public_seen && has_internal {
            return Err(IconError::NetworkBoundary);
        }
        self.public_seen |= addresses.iter().any(|address| !internal(address.ip()));
        // Mixed DNS answers may otherwise select an internal address on retry.
        if self.public_seen && has_internal {
            return Err(IconError::NetworkBoundary);
        }
        Ok(addresses)
    }
    fn fetch(&mut self, mut url: Url) -> Result<(Url, Zeroizing<Vec<u8>>), IconError> {
        loop {
            let addresses = self.addresses(&url)?;
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(self.remaining()?)
                .connect_timeout(self.remaining()?.min(Duration::from_secs(10)))
                .resolve_to_addrs(url.host_str().ok_or(IconError::InvalidUrl)?, &addresses)
                .user_agent("Taypeer/0.1 icon-loader")
                .build()
                .map_err(|_| IconError::Network)?;
            let mut response = client.get(url.clone()).send().map_err(network_error)?;
            if response.status().is_redirection() {
                self.redirects += 1;
                if self.redirects > 5 {
                    return Err(IconError::Network);
                }
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|h| h.to_str().ok())
                    .ok_or(IconError::Network)?;
                let next = url.join(location).map_err(|_| IconError::InvalidUrl)?;
                url = parse(next.as_str())?;
                continue;
            }
            if !response.status().is_success() {
                return Err(IconError::Network);
            }
            if response.content_length().is_some_and(|n| n > ICON_LIMIT) {
                return Err(IconError::TooLarge);
            }
            let mut bytes = Zeroizing::new(Vec::new());
            (&mut response)
                .take(ICON_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    if self.remaining().is_err() {
                        IconError::Timeout
                    } else {
                        IconError::Network
                    }
                })?;
            self.remaining()?;
            if bytes.len() as u64 > ICON_LIMIT {
                return Err(IconError::TooLarge);
            }
            return Ok((url, bytes));
        }
    }
}
fn network_error(error: reqwest::Error) -> IconError {
    if error.is_timeout() {
        IconError::Timeout
    } else {
        IconError::Network
    }
}

pub(super) fn download(input: &str, favicon: bool) -> Result<Zeroizing<Vec<u8>>, IconError> {
    let url = parse(input)?;
    let mut download = Download {
        deadline: Instant::now() + DEADLINE,
        public_seen: false,
        redirects: 0,
    };
    if !favicon {
        let (_, bytes) = download.fetch(url)?;
        validate(&bytes)?;
        return Ok(bytes);
    }
    let mut page = url.clone();
    let page_result = download.fetch(url);
    if let Err(error @ (IconError::NetworkBoundary | IconError::Timeout)) = &page_result {
        return Err(*error);
    }
    if let Ok((final_url, html)) = page_result {
        page = final_url;
        let mut tokenizer = Tokenizer::new(Links::default(), Default::default());
        let mut queue = BufferQueue::default();
        queue.push_back(StrTendril::from_slice(&String::from_utf8_lossy(&html)));
        let _ = tokenizer.feed(&mut queue);
        tokenizer.end();
        let base = tokenizer
            .sink
            .base
            .as_deref()
            .and_then(|href| page.join(href).ok())
            .unwrap_or_else(|| page.clone());
        for link in tokenizer.sink.icons {
            let Ok(candidate) = base.join(&link) else {
                continue;
            };
            let Ok(candidate) = parse(candidate.as_str()) else {
                continue;
            };
            match download.fetch(candidate) {
                Ok((_, bytes)) if validate(&bytes).is_ok() => return Ok(bytes),
                Err(error @ (IconError::NetworkBoundary | IconError::Timeout)) => {
                    return Err(error);
                }
                _ => {}
            }
        }
    }
    let fallback = page
        .join("/favicon.ico")
        .map_err(|_| IconError::InvalidUrl)?;
    let (_, bytes) = download.fetch(fallback)?;
    validate(&bytes)?;
    Ok(bytes)
}

#[derive(Default)]
struct Links {
    icons: Vec<String>,
    base: Option<String>,
}
impl TokenSink for Links {
    type Handle = ();
    fn process_token(&mut self, token: Token, _: u64) -> TokenSinkResult<()> {
        let Token::TagToken(tag) = token else {
            return TokenSinkResult::Continue;
        };
        if tag.kind != TagKind::StartTag {
            return TokenSinkResult::Continue;
        }
        // A tokenizer sink must explicitly enter raw-text modes; otherwise JavaScript
        // strings containing markup could be mistaken for real favicon links.
        match tag.name.as_ref() {
            "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
            "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => {
                return TokenSinkResult::RawData(RawKind::Rawtext);
            }
            "textarea" | "title" => return TokenSinkResult::RawData(RawKind::Rcdata),
            _ => {}
        }
        let attribute = |key: &str| {
            tag.attrs
                .iter()
                .find(|a| a.name.local.as_ref() == key)
                .map(|a| a.value.as_ref())
        };
        if tag.name.as_ref() == "base" && self.base.is_none() {
            self.base = attribute("href").map(str::to_owned);
        }
        if tag.name.as_ref() == "link"
            && self.icons.len() < 8
            && attribute("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace().any(|word| {
                    word.eq_ignore_ascii_case("icon")
                        || word.eq_ignore_ascii_case("apple-touch-icon")
                })
            })
            && let Some(href) = attribute("href")
        {
            self.icons.push(href.to_owned());
        }
        TokenSinkResult::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_sources_cannot_switch_to_any_internal_address_spelling() {
        for input in [
            "http://127.0.0.1",
            "http://2130706433",
            "http://[::1]",
            "http://[::ffff:127.0.0.1]",
            "http://192.168.1.1",
            "http://169.254.169.254",
        ] {
            let mut download = Download {
                deadline: Instant::now() + DEADLINE,
                public_seen: true,
                redirects: 0,
            };
            assert_eq!(
                download.addresses(&parse(input).unwrap()),
                Err(IconError::NetworkBoundary)
            );
        }
        assert!(parse("file:///PUBLIC").is_err());
        assert!(parse("http://PUBLIC:PUBLIC@localhost").is_err());
        let expired = Download {
            deadline: Instant::now() - Duration::from_secs(1),
            public_seen: false,
            redirects: 0,
        };
        assert_eq!(expired.remaining(), Err(IconError::Timeout));
    }
}
