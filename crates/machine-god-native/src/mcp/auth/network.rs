use super::{Authority, DOCUMENT_LIMIT, McpAuthError, Result};
use crate::mcp::{
    endpoint::McpEndpoint,
    http::{McpHttpConnection, McpHttpControl, McpHttpError, McpHttpLimits},
};
use futures_util::future::{Either, select};
use machine_god_core::CancellationToken;
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

pub(super) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub json: bool,
}
impl Drop for Response {
    fn drop(&mut self) {
        self.body.fill(0);
    }
}
pub(super) struct Failure {
    pub error: McpAuthError,
    pub attempted: bool,
}
pub(super) enum Request {
    Metadata,
    Registration(Vec<u8>),
    Token(Vec<u8>, Option<String>),
}

impl Authority {
    pub(super) async fn request(
        &self,
        url: &str,
        request: Request,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> std::result::Result<Response, Failure> {
        let deadline = deadline.min(
            self.clock
                .now()
                .checked_add(Duration::from_secs(30))
                .ok_or(Failure {
                    error: McpAuthError::Deadline,
                    attempted: false,
                })?,
        );
        let make = async {
            let selected = self.network.admit(url, cancellation, deadline).await?;
            let endpoint = McpEndpoint::parse(url).map_err(|_| McpAuthError::Invalid)?;
            if selected.destination.endpoint() != &endpoint {
                return Err(McpAuthError::Invalid);
            }
            let (control, authorization) = match request {
                Request::Metadata => (McpHttpControl::oauth_metadata(), None),
                Request::Registration(bytes) => (
                    McpHttpControl::oauth_json(bytes.into_boxed_slice()).map_err(map)?,
                    None,
                ),
                Request::Token(bytes, authorization) => (
                    McpHttpControl::oauth_form(bytes.into_boxed_slice()).map_err(map)?,
                    authorization,
                ),
            };
            let headers: Vec<(&str, &[u8])> = authorization
                .as_ref()
                .map(|value| ("Authorization", value.as_bytes()))
                .into_iter()
                .collect();
            let connection = McpHttpConnection::with_clock(
                selected.destination,
                &headers,
                selected.trust,
                McpHttpLimits {
                    body_bytes: DOCUMENT_LIMIT as u64,
                    ..McpHttpLimits::default()
                },
                cancellation.clone(),
                deadline,
                self.clock.clone() as Arc<dyn crate::mcp::http::McpHttpClock>,
            )
            .map_err(map)?;
            Ok((connection, control))
        };
        let (connection, control) =
            self.bounded(make, cancellation, deadline)
                .await
                .map_err(|error| Failure {
                    error,
                    attempted: false,
                })?;
        let observer = connection.observation();
        let operation = async {
            let mut response = connection.control(control).await.map_err(map)?;
            if (300..400).contains(&response.status) {
                return Err(McpAuthError::Rejected);
            }
            let mut content_type = None;
            for (name, value) in response.headers.iter() {
                if name == "content-type" && content_type.replace(value).is_some() {
                    return Err(McpAuthError::Invalid);
                }
            }
            let json = content_type.is_some_and(|value| {
                value.split(|b| *b == b';').next().is_some_and(|media| {
                    media.trim_ascii().eq_ignore_ascii_case(b"application/json")
                })
            });
            let mut body = Vec::new();
            while let Some(chunk) = response.body.next_chunk().await.map_err(map)? {
                if chunk.len() > DOCUMENT_LIMIT.saturating_sub(body.len()) {
                    body.fill(0);
                    return Err(McpAuthError::Limit);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(Response {
                status: response.status,
                body,
                json,
            })
        };
        self.bounded(operation, cancellation, deadline)
            .await
            .map_err(|error| Failure {
                error,
                attempted: observer.was_attempted(),
            })
    }
    pub(super) fn check(&self, cancellation: &CancellationToken, deadline: Instant) -> Result<()> {
        if cancellation.is_cancelled() {
            Err(McpAuthError::Cancelled)
        } else if self.clock.now() >= deadline {
            Err(McpAuthError::Deadline)
        } else {
            Ok(())
        }
    }
    pub(super) async fn bounded<T>(
        &self,
        future: impl Future<Output = Result<T>>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<T> {
        self.check(cancellation, deadline)?;
        let cancel = cancellation.cancelled();
        let timer = self.clock.sleep_until(deadline);
        let stop = select(Box::pin(cancel), timer);
        let result = match select(Box::pin(future), Box::pin(stop)).await {
            Either::Left((result, _)) => result,
            Either::Right((Either::Left(_), _)) => Err(McpAuthError::Cancelled),
            Either::Right((Either::Right(_), _)) => Err(McpAuthError::Deadline),
        };
        self.check(cancellation, deadline)?;
        result
    }
}
fn map(error: McpHttpError) -> McpAuthError {
    match error {
        McpHttpError::Cancelled => McpAuthError::Cancelled,
        McpHttpError::Deadline => McpAuthError::Deadline,
        McpHttpError::Limit => McpAuthError::Limit,
        _ => McpAuthError::Network,
    }
}
