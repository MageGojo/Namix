//! Trusted reverse-proxy client IP resolution.
//!
//! Forwarding headers are honored only when the socket peer is in the
//! configured allowlist. The `X-Forwarded-For` chain is walked from right to
//! left so a trusted edge which appends the real peer defeats client-supplied
//! spoofed entries.

use std::net::IpAddr;

use thiserror::Error;

use super::middleware::{MiddlewareFn, wrap_middleware};
use super::request::{ClientIp, Request};

#[derive(Clone, Debug)]
pub struct TrustedProxies {
    networks: Vec<TrustedNetwork>,
}

#[derive(Clone, Copy, Debug)]
struct TrustedNetwork {
    address: IpAddr,
    prefix: u8,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrustedProxyError {
    #[error("trusted proxy entry is empty")]
    Empty,
    #[error("invalid trusted proxy address or CIDR: {0}")]
    Invalid(String),
}

impl TrustedProxies {
    pub fn new(
        entries: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<Self, TrustedProxyError> {
        let networks = entries
            .into_iter()
            .map(|entry| TrustedNetwork::parse(entry.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { networks })
    }

    pub fn contains(&self, address: IpAddr) -> bool {
        self.networks
            .iter()
            .any(|network| network.contains(address))
    }

    pub fn middleware(self) -> MiddlewareFn {
        wrap_middleware(move |mut req: Request, next| {
            let proxies = self.clone();
            async move {
                if let Some(peer) = req.client_ip()
                    && proxies.contains(peer)
                    && let Some(client) = proxies.forwarded_client(&req)
                {
                    req.set_attr("namix.proxy.peer_ip", peer.to_string());
                    req.set(ClientIp(client));
                }
                next.run(req).await
            }
        })
    }

    fn forwarded_client(&self, req: &Request) -> Option<IpAddr> {
        let forwarded = req
            .header("x-forwarded-for")
            .into_iter()
            .flat_map(|value| value.split(','))
            .filter_map(parse_forwarded_ip)
            .collect::<Vec<_>>();
        if !forwarded.is_empty() {
            return forwarded
                .iter()
                .rev()
                .copied()
                .find(|address| !self.contains(*address))
                .or_else(|| forwarded.first().copied());
        }
        req.header("x-real-ip").and_then(parse_forwarded_ip)
    }
}

impl TrustedNetwork {
    fn parse(raw: &str) -> Result<Self, TrustedProxyError> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(TrustedProxyError::Empty);
        }
        let (address, prefix) = match raw.split_once('/') {
            Some((address, prefix)) => {
                let address = address
                    .parse::<IpAddr>()
                    .map_err(|_| TrustedProxyError::Invalid(raw.into()))?;
                let prefix = prefix
                    .parse::<u8>()
                    .ok()
                    .filter(|prefix| *prefix <= if address.is_ipv4() { 32 } else { 128 })
                    .ok_or_else(|| TrustedProxyError::Invalid(raw.into()))?;
                (address, prefix)
            }
            None => {
                let address = raw
                    .parse::<IpAddr>()
                    .map_err(|_| TrustedProxyError::Invalid(raw.into()))?;
                let prefix = if address.is_ipv4() { 32 } else { 128 };
                (address, prefix)
            }
        };
        Ok(Self { address, prefix })
    }

    fn contains(self, candidate: IpAddr) -> bool {
        match (self.address, candidate) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => prefix_matches(
                u32::from(network) as u128,
                u32::from(candidate) as u128,
                self.prefix,
                32,
            ),
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                prefix_matches(u128::from(network), u128::from(candidate), self.prefix, 128)
            }
            _ => false,
        }
    }
}

fn prefix_matches(network: u128, candidate: u128, prefix: u8, bits: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = bits - prefix;
    network >> shift == candidate >> shift
}

fn parse_forwarded_ip(raw: &str) -> Option<IpAddr> {
    raw.trim().trim_matches('"').parse::<IpAddr>().ok()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bytes::Bytes;
    use http::{HeaderMap, HeaderValue, Method, Uri};

    use super::*;
    use crate::core::content_type::ContentType;
    use crate::core::middleware::Next;
    use crate::core::response::Response;

    fn request(peer: IpAddr, forwarded: Option<&str>) -> Request {
        let mut headers = HeaderMap::new();
        if let Some(forwarded) = forwarded {
            headers.insert("x-forwarded-for", HeaderValue::from_str(forwarded).unwrap());
        }
        let mut request = Request::new(Method::GET, Uri::from_static("/"), headers, Bytes::new());
        request.set(ClientIp(peer));
        request
    }

    async fn show_ip(req: Request) -> Response {
        Response::new(
            http::StatusCode::OK,
            ContentType::Text,
            req.client_ip().unwrap().to_string(),
        )
    }

    async fn run(proxies: TrustedProxies, request: Request) -> Response {
        let middleware = proxies.middleware();
        middleware(
            request,
            Next::new(
                Arc::new(Vec::new()),
                0,
                Arc::new(|request| Box::pin(show_ip(request))),
            ),
        )
        .await
    }

    #[tokio::test]
    async fn trusted_peer_uses_rightmost_untrusted_forwarded_address() {
        let proxies = TrustedProxies::new(["127.0.0.1", "10.0.0.0/8"]).unwrap();
        let response = run(
            proxies,
            request(
                "127.0.0.1".parse().unwrap(),
                Some("198.51.100.99, 203.0.113.7, 10.1.2.3"),
            ),
        )
        .await;
        assert_eq!(response.into_status_headers_body().await.2, "203.0.113.7");
    }

    #[tokio::test]
    async fn untrusted_peer_cannot_spoof_forwarded_headers() {
        let proxies = TrustedProxies::new(["127.0.0.1"]).unwrap();
        let response = run(
            proxies,
            request("192.0.2.8".parse().unwrap(), Some("198.51.100.99")),
        )
        .await;
        assert_eq!(response.into_status_headers_body().await.2, "192.0.2.8");
    }

    #[test]
    fn rejects_invalid_cidr_prefixes() {
        assert!(matches!(
            TrustedProxies::new(["127.0.0.1/64"]),
            Err(TrustedProxyError::Invalid(_))
        ));
    }
}
