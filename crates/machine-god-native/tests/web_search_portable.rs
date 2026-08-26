use std::future;
use std::time::Instant;

use machine_god_core::{BoxFuture, CancellationToken};
use machine_god_native::{
    MAX_WEB_SEARCH_QUERY_BYTES, WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS, WebSearchDeadline,
    WebSearchLimits, WebSearchRequest, WebSearchResponse, WebSearchSource, WebSearchTransport,
    WebSearchTransportError, WebSearchTransportErrorKind,
};

struct InertTransport;

impl WebSearchTransport for InertTransport {
    fn search(
        &self,
        _request: WebSearchRequest,
        _cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<WebSearchResponse, WebSearchTransportError>> {
        Box::pin(future::pending())
    }
}

struct InertDeadline;

impl WebSearchDeadline for InertDeadline {
    fn wait_until(&self, _deadline: Instant) -> BoxFuture<'_, Result<(), WebSearchTransportError>> {
        Box::pin(future::pending())
    }
}

fn accept_transport<T: WebSearchTransport>(_transport: &T) {}

fn accept_deadline<T: WebSearchDeadline>(_deadline: &T) {}

#[test]
fn provider_neutral_contract_is_available_without_native_http_or_tokio_authority() {
    assert_eq!(MAX_WEB_SEARCH_QUERY_BYTES, 4_096);
    assert_eq!(
        WebSearchLimits::default().max_active_requests(),
        WEB_SEARCH_DEFAULT_MAX_ACTIVE_REQUESTS
    );
    accept_transport(&InertTransport);
    accept_deadline(&InertDeadline);

    let source = WebSearchSource::new(
        "portable citation".to_owned(),
        "https://www.rust-lang.org/".to_owned(),
    )
    .unwrap();
    assert_eq!(format!("{source:?}"), "WebSearchSource { .. }");
    assert_eq!(
        WebSearchResponse::new(vec![source], false)
            .unwrap()
            .sources()
            .len(),
        1
    );
    assert_eq!(
        WebSearchTransportError::new(WebSearchTransportErrorKind::RuntimeRequired).kind(),
        WebSearchTransportErrorKind::RuntimeRequired
    );
}
