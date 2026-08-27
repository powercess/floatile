//! Connection-bound HTTPS request construction and transport.
//!
//! The guest selects only a validated manifest template, a granted Connection, and bounded query
//! values. Credential lookup and injection stay in this host-only module.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use floatile_core::{
    CapabilityParams, Connection, ConnectionHealth, ConnectionId, HttpTemplateDecl,
};
use futures_util::StreamExt;
use reqwest::header::{HeaderName, HeaderValue};
use url::Url;

use floatile_core::OperationFailure;

use crate::CredentialVault;

pub const MAX_QUERY_VALUE_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParam {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HttpServiceError {
    #[error("HTTPS request input is invalid")]
    InvalidInput,
    #[error("HTTPS template is not available")]
    TemplateUnavailable,
    #[error("Connection is not granted")]
    ConnectionNotGranted,
    #[error("Connection credential is unavailable")]
    CredentialUnavailable,
    #[error("HTTPS destination is not public")]
    DestinationNotPublic,
    #[error("HTTPS transport is unavailable")]
    Unavailable,
    #[error("HTTPS response violates the template")]
    ResponseRejected,
}

impl From<HttpServiceError> for OperationFailure {
    fn from(error: HttpServiceError) -> Self {
        match error {
            HttpServiceError::InvalidInput
            | HttpServiceError::TemplateUnavailable
            | HttpServiceError::ConnectionNotGranted
            | HttpServiceError::ResponseRejected => Self::Internal,
            HttpServiceError::CredentialUnavailable
            | HttpServiceError::DestinationNotPublic
            | HttpServiceError::Unavailable => Self::Unavailable,
        }
    }
}

pub struct HttpTransportRequest {
    url: Url,
    credential_header: HeaderName,
    credential: HeaderValue,
    max_response_bytes: usize,
    allowed_statuses: Vec<u16>,
}

pub trait HttpTransport: Send + Sync {
    fn execute(
        &self,
        request: HttpTransportRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, HttpServiceError>> + Send + 'static>>;
}

/// Production HTTPS transport: system DNS is resolved once, every result must be public, and the
/// selected address is pinned into a per-request rustls client. Redirects are disabled.
#[derive(Default)]
pub struct ReqwestHttpTransport;

impl HttpTransport for ReqwestHttpTransport {
    fn execute(
        &self,
        request: HttpTransportRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, HttpServiceError>> + Send + 'static>>
    {
        Box::pin(async move {
            let host = request
                .url
                .host_str()
                .ok_or(HttpServiceError::InvalidInput)?
                .to_owned();
            let port = request
                .url
                .port_or_known_default()
                .ok_or(HttpServiceError::InvalidInput)?;
            let addresses: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
                .await
                .map_err(|_| HttpServiceError::Unavailable)?
                .collect();
            if addresses.is_empty() || addresses.iter().any(|addr| !is_public_ip(addr.ip())) {
                return Err(HttpServiceError::DestinationNotPublic);
            }
            let pinned = SocketAddr::new(addresses[0].ip(), port);
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .resolve(&host, pinned)
                .build()
                .map_err(|_| HttpServiceError::Unavailable)?;
            let response = client
                .get(request.url)
                .header(request.credential_header, request.credential)
                .send()
                .await
                .map_err(|_| HttpServiceError::Unavailable)?;
            let status = response.status().as_u16();
            if !request.allowed_statuses.contains(&status)
                || response
                    .content_length()
                    .is_some_and(|length| length > request.max_response_bytes as u64)
            {
                return Err(HttpServiceError::ResponseRejected);
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| HttpServiceError::Unavailable)?;
                if body.len().saturating_add(chunk.len()) > request.max_response_bytes {
                    return Err(HttpServiceError::ResponseRejected);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse { status, body })
        })
    }
}

#[derive(Clone)]
pub struct HttpsService {
    templates: Arc<BTreeMap<String, HttpTemplateDecl>>,
    connections: Arc<BTreeMap<ConnectionId, Connection>>,
    vault: Arc<dyn CredentialVault>,
    transport: Arc<dyn HttpTransport>,
}

impl HttpsService {
    pub fn new(
        templates: Vec<HttpTemplateDecl>,
        granted_connections: Vec<Connection>,
        vault: Arc<dyn CredentialVault>,
        transport: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            templates: Arc::new(
                templates
                    .into_iter()
                    .map(|template| (template.id.clone(), template))
                    .collect(),
            ),
            connections: Arc::new(
                granted_connections
                    .into_iter()
                    .map(|connection| (connection.id(), connection))
                    .collect(),
            ),
            vault,
            transport,
        }
    }

    pub fn prepare(
        &self,
        template_id: &str,
        connection_id: ConnectionId,
        query: Vec<QueryParam>,
    ) -> Result<PreparedHttpOperation, HttpServiceError> {
        let template = self
            .templates
            .get(template_id)
            .ok_or(HttpServiceError::TemplateUnavailable)?;
        let connection = self
            .connections
            .get(&connection_id)
            .ok_or(HttpServiceError::ConnectionNotGranted)?;
        if connection.health() != ConnectionHealth::Healthy {
            return Err(HttpServiceError::CredentialUnavailable);
        }
        if query.len() > template.query_params.len() {
            return Err(HttpServiceError::InvalidInput);
        }
        let mut seen = BTreeSet::new();
        let mut url = Url::parse(&template.url).map_err(|_| HttpServiceError::InvalidInput)?;
        {
            let mut pairs = url.query_pairs_mut();
            for param in query {
                if !template.query_params.contains(&param.name)
                    || !seen.insert(param.name.clone())
                    || param.value.len() > MAX_QUERY_VALUE_BYTES
                    || param.value.contains('\0')
                {
                    return Err(HttpServiceError::InvalidInput);
                }
                pairs.append_pair(&param.name, &param.value);
            }
        }
        let header = HeaderName::from_bytes(template.credential_header.as_bytes())
            .map_err(|_| HttpServiceError::InvalidInput)?;
        let mut credential = None;
        self.vault
            .with_secret(connection.credential(), &mut |secret| {
                credential = HeaderValue::from_bytes(secret).ok();
            })
            .map_err(|_| HttpServiceError::CredentialUnavailable)?;
        let mut credential = credential.ok_or(HttpServiceError::CredentialUnavailable)?;
        credential.set_sensitive(true);
        let origin = url.origin().ascii_serialization();
        let request_params = CapabilityParams::Network {
            origins: vec![origin],
            max_requests_per_minute: 1,
            max_response_bytes: template.max_response_bytes,
            max_timeout_ms: template.timeout_ms,
        };
        let transport_request = HttpTransportRequest {
            url,
            credential_header: header,
            credential,
            max_response_bytes: usize::try_from(template.max_response_bytes)
                .map_err(|_| HttpServiceError::InvalidInput)?,
            allowed_statuses: template.allowed_statuses.clone(),
        };
        let transport = Arc::clone(&self.transport);
        Ok(PreparedHttpOperation {
            request_params,
            timeout: Duration::from_millis(template.timeout_ms),
            work: Box::pin(async move {
                transport
                    .execute(transport_request)
                    .await
                    .map_err(OperationFailure::from)
            }),
        })
    }
}

pub struct PreparedHttpOperation {
    pub request_params: CapabilityParams,
    pub timeout: Duration,
    pub work:
        Pin<Box<dyn Future<Output = Result<HttpResponse, OperationFailure>> + Send + 'static>>,
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_ipv4(mapped);
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

fn is_public_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use floatile_core::{CredentialRef, HttpTemplateDecl};

    use super::*;
    use crate::{CredentialVault, MemoryCredentialVault};

    struct InspectTransport {
        saw_secret: Arc<AtomicBool>,
    }

    impl HttpTransport for InspectTransport {
        fn execute(
            &self,
            request: HttpTransportRequest,
        ) -> Pin<Box<dyn Future<Output = Result<HttpResponse, HttpServiceError>> + Send + 'static>>
        {
            let saw_secret = Arc::clone(&self.saw_secret);
            Box::pin(async move {
                saw_secret.store(
                    request.credential.as_bytes() == b"Bearer host-only-secret",
                    Ordering::Release,
                );
                assert_eq!(
                    request.url.as_str(),
                    "https://api.example.com/v1/balance?account=A%26B"
                );
                Ok(HttpResponse {
                    status: 200,
                    body: br#"{"balance":42}"#.to_vec(),
                })
            })
        }
    }

    fn template() -> HttpTemplateDecl {
        HttpTemplateDecl {
            id: "balance".into(),
            method: "GET".into(),
            url: "https://api.example.com/v1/balance".into(),
            query_params: vec!["account".into()],
            credential_header: "authorization".into(),
            allowed_statuses: vec![200],
            max_response_bytes: 4096,
            timeout_ms: 2000,
        }
    }

    fn connection(health: ConnectionHealth) -> Connection {
        Connection::restore(
            ConnectionId(7),
            "example",
            "account@example.com",
            CredentialRef::new("cred://example/account").unwrap(),
            health,
            3,
            1,
            2,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn injects_secret_only_inside_host_transport_and_returns_typed_body() {
        let vault = Arc::new(MemoryCredentialVault::default());
        vault
            .put(
                connection(ConnectionHealth::Healthy).credential(),
                b"Bearer host-only-secret",
            )
            .unwrap();
        let saw_secret = Arc::new(AtomicBool::new(false));
        let service = HttpsService::new(
            vec![template()],
            vec![connection(ConnectionHealth::Healthy)],
            vault,
            Arc::new(InspectTransport {
                saw_secret: Arc::clone(&saw_secret),
            }),
        );
        let prepared = service
            .prepare(
                "balance",
                ConnectionId(7),
                vec![QueryParam {
                    name: "account".into(),
                    value: "A&B".into(),
                }],
            )
            .unwrap();
        let response = prepared.work.await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, br#"{"balance":42}"#);
        assert!(saw_secret.load(Ordering::Acquire));
        assert!(!format!("{response:?}").contains("host-only-secret"));
    }

    #[test]
    fn rejects_ungranted_unhealthy_and_undeclared_query_without_transport() {
        let vault = Arc::new(MemoryCredentialVault::default());
        let service = HttpsService::new(
            vec![template()],
            vec![connection(ConnectionHealth::Unavailable)],
            vault,
            Arc::new(InspectTransport {
                saw_secret: Arc::new(AtomicBool::new(false)),
            }),
        );
        assert!(matches!(
            service.prepare("balance", ConnectionId(8), Vec::new()),
            Err(HttpServiceError::ConnectionNotGranted)
        ));
        assert!(matches!(
            service.prepare("balance", ConnectionId(7), Vec::new()),
            Err(HttpServiceError::CredentialUnavailable)
        ));

        let healthy = HttpsService::new(
            vec![template()],
            vec![connection(ConnectionHealth::Healthy)],
            Arc::new(MemoryCredentialVault::default()),
            Arc::new(InspectTransport {
                saw_secret: Arc::new(AtomicBool::new(false)),
            }),
        );
        assert!(matches!(
            healthy.prepare(
                "balance",
                ConnectionId(7),
                vec![QueryParam {
                    name: "token".into(),
                    value: "forbidden".into()
                }]
            ),
            Err(HttpServiceError::InvalidInput)
        ));
    }

    #[test]
    fn ssrf_filter_rejects_non_public_addresses() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "192.0.2.1",
            "::1",
            "fe80::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "应拒绝 {ip}");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }
}
