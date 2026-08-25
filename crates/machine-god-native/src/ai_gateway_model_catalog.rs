//! Bounded AI Gateway model-catalog adapter over an injected byte transport.

use machine_god_core::{
    AvailableModel, BoxFuture, CancellationToken, ModelCatalog, ModelCatalogAccess,
    ModelCatalogProvider, ProviderError, ProviderErrorKind, PublicCatalogReason,
};
use serde::Deserialize;
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::future::{Future, poll_fn};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

/// Stable provider identifier for the AI Gateway catalog.
pub const AI_GATEWAY_MODEL_CATALOG_PROVIDER_NAME: &str = "vercel_ai_gateway";
/// Inclusive maximum response-body size accepted by the catalog adapter.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES: usize = 256 * 1024;
/// Maximum JSON container depth, with the top-level object at depth zero.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_JSON_DEPTH: usize = 32;
/// Maximum number of JSON values in one catalog response.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES: usize = 16_384;
/// Maximum number of raw entries in the response `data` array.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES: usize = 1_024;
/// Maximum number of accepted language-model entries.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_MODELS: usize = 512;
/// Maximum aggregate bytes across accepted model identifiers.
pub const AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES: usize = 24 * 1024;
/// Total catalog deadline, starting when the provider future is first polled.
pub const AI_GATEWAY_MODEL_CATALOG_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Catalog access attempted by an injected transport call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayModelCatalogRequestAccess {
    /// Include the transport's configured bearer credential.
    Authenticated,
    /// Make an anonymous catalog request.
    Public,
}

/// Initial catalog access selected by native credential discovery.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayModelCatalogAccessMode {
    /// Attempt authenticated access, with one anonymous fallback on 401/403.
    Authenticated,
    /// Make only one anonymous request.
    PublicOnly,
}

/// One complete HTTP-style catalog response from an injected transport.
pub struct AiGatewayModelCatalogTransportResponse {
    status: u16,
    body: Vec<u8>,
}

impl AiGatewayModelCatalogTransportResponse {
    /// Creates an owned response. The provider independently enforces its body limit.
    #[must_use]
    pub const fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body }
    }

    /// Returns the numeric response status.
    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    /// Returns the response bytes.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    fn into_parts(self) -> (u16, Vec<u8>) {
        (self.status, self.body)
    }
}

impl fmt::Debug for AiGatewayModelCatalogTransportResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogTransportResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

/// Stable failure category produced by an injected catalog transport.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AiGatewayModelCatalogTransportErrorKind {
    /// Network or HTTP transport failed.
    Transport,
    /// The peer response violated HTTP framing or another transport protocol rule.
    MalformedResponse,
    /// A fixed transport resource bound was exceeded.
    ResourceLimit,
    /// Polling requires an active Tokio runtime.
    RuntimeRequired,
    /// Cooperative cancellation won.
    Cancelled,
}

/// Fixed, data-free catalog transport failure.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct AiGatewayModelCatalogTransportError {
    kind: AiGatewayModelCatalogTransportErrorKind,
}

impl AiGatewayModelCatalogTransportError {
    /// Creates a fixed transport failure.
    #[must_use]
    pub const fn new(kind: AiGatewayModelCatalogTransportErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(self) -> AiGatewayModelCatalogTransportErrorKind {
        self.kind
    }
}

impl fmt::Debug for AiGatewayModelCatalogTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogTransportError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for AiGatewayModelCatalogTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            AiGatewayModelCatalogTransportErrorKind::Transport => {
                "AI Gateway model catalog transport failed"
            }
            AiGatewayModelCatalogTransportErrorKind::MalformedResponse => {
                "AI Gateway model catalog transport response is malformed"
            }
            AiGatewayModelCatalogTransportErrorKind::ResourceLimit => {
                "AI Gateway model catalog transport resource limit exceeded"
            }
            AiGatewayModelCatalogTransportErrorKind::RuntimeRequired => {
                "AI Gateway model catalog transport requires an active Tokio runtime"
            }
            AiGatewayModelCatalogTransportErrorKind::Cancelled => {
                "AI Gateway model catalog request cancelled"
            }
        })
    }
}

impl std::error::Error for AiGatewayModelCatalogTransportError {}

/// Runtime- and HTTP-client-neutral model-catalog transport.
pub trait AiGatewayModelCatalogTransport: Send + Sync + 'static {
    /// Waits until the supplied absolute deadline.
    ///
    /// Implementations must arrange a wakeup no later than `deadline` and
    /// must not resolve this future before that instant. The provider polls
    /// this authority independently from [`Self::get`], so a request future
    /// that remains pending cannot defeat the total catalog deadline.
    fn wait_until(&self, deadline: Instant) -> BoxFuture<'_, ()>;

    /// Fetches one complete catalog response at the requested access level.
    fn get(
        &self,
        access: AiGatewayModelCatalogRequestAccess,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<AiGatewayModelCatalogTransportResponse, AiGatewayModelCatalogTransportError>,
    >;
}

/// AI Gateway catalog provider with strict parsing and bounded authentication fallback.
pub struct AiGatewayModelCatalogProvider {
    access_mode: AiGatewayModelCatalogAccessMode,
    transport: Arc<dyn AiGatewayModelCatalogTransport>,
}

impl AiGatewayModelCatalogProvider {
    /// Creates a provider over an explicitly injected transport.
    #[must_use]
    pub fn new(
        access_mode: AiGatewayModelCatalogAccessMode,
        transport: Arc<dyn AiGatewayModelCatalogTransport>,
    ) -> Self {
        Self {
            access_mode,
            transport,
        }
    }
}

impl fmt::Debug for AiGatewayModelCatalogProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AiGatewayModelCatalogProvider")
            .field("access_mode", &self.access_mode)
            .field("transport", &"<redacted>")
            .finish()
    }
}

impl ModelCatalogProvider for AiGatewayModelCatalogProvider {
    fn name(&self) -> &str {
        AI_GATEWAY_MODEL_CATALOG_PROVIDER_NAME
    }

    fn list_models(
        &self,
        cancellation: CancellationToken,
    ) -> BoxFuture<'_, Result<ModelCatalog, ProviderError>> {
        Box::pin(async move {
            check_cancelled(&cancellation)?;
            let deadline = Instant::now()
                .checked_add(AI_GATEWAY_MODEL_CATALOG_REQUEST_TIMEOUT)
                .ok_or_else(resource_limit_error)?;
            let (response, access) = match self.access_mode {
                AiGatewayModelCatalogAccessMode::PublicOnly => {
                    let response = request_transport(
                        self.transport.as_ref(),
                        AiGatewayModelCatalogRequestAccess::Public,
                        deadline,
                        &cancellation,
                    )
                    .await?;
                    (
                        response,
                        ModelCatalogAccess::PublicOnly {
                            reason: PublicCatalogReason::NoCredential,
                        },
                    )
                }
                AiGatewayModelCatalogAccessMode::Authenticated => {
                    let response = request_transport(
                        self.transport.as_ref(),
                        AiGatewayModelCatalogRequestAccess::Authenticated,
                        deadline,
                        &cancellation,
                    )
                    .await?;
                    if matches!(response.status(), 401 | 403) {
                        drop(response);
                        check_cancelled(&cancellation)?;
                        check_deadline(deadline)?;
                        let public = request_transport(
                            self.transport.as_ref(),
                            AiGatewayModelCatalogRequestAccess::Public,
                            deadline,
                            &cancellation,
                        )
                        .await?;
                        (
                            public,
                            ModelCatalogAccess::PublicOnly {
                                reason: PublicCatalogReason::AuthenticatedCredentialRejected,
                            },
                        )
                    } else {
                        (response, ModelCatalogAccess::Authenticated)
                    }
                }
            };

            check_cancelled(&cancellation)?;
            check_deadline(deadline)?;
            let (status, body) = response.into_parts();
            if status != 200 {
                return Err(status_error(status));
            }
            let models = parse_catalog(&body, &cancellation, deadline)?;
            check_cancelled(&cancellation)?;
            check_deadline(deadline)?;
            Ok(ModelCatalog::new(models, access))
        })
    }
}

async fn request_transport(
    transport: &dyn AiGatewayModelCatalogTransport,
    access: AiGatewayModelCatalogRequestAccess,
    deadline: Instant,
    cancellation: &CancellationToken,
) -> Result<AiGatewayModelCatalogTransportResponse, ProviderError> {
    check_cancelled(cancellation)?;
    check_deadline(deadline)?;
    let mut deadline_reached = transport.wait_until(deadline);
    let mut request = transport.get(access, deadline, cancellation.clone());
    let mut cancelled = Box::pin(cancellation.cancelled());
    poll_fn(|context| {
        if cancelled.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(cancelled_error()));
        }
        if deadline_reached.as_mut().poll(context).is_ready() {
            return Poll::Ready(Err(resource_limit_error()));
        }
        match request.as_mut().poll(context) {
            Poll::Ready(result) => {
                if cancellation.is_cancelled() {
                    Poll::Ready(Err(cancelled_error()))
                } else if Instant::now() >= deadline {
                    Poll::Ready(Err(resource_limit_error()))
                } else {
                    Poll::Ready(result.map_err(map_transport_error))
                }
            }
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ParseFailure {
    Malformed,
    ResourceLimit,
    Cancelled,
    Deadline,
}

struct ParseContext<'a> {
    nodes: Cell<usize>,
    failure: Cell<Option<ParseFailure>>,
    cancellation: &'a CancellationToken,
    deadline: Instant,
}

impl ParseContext<'_> {
    fn consume_node<E: serde::de::Error>(&self) -> Result<(), E> {
        if self.cancellation.is_cancelled() {
            self.failure.set(Some(ParseFailure::Cancelled));
            return Err(E::custom("catalog parsing cancelled"));
        }
        if Instant::now() >= self.deadline {
            self.failure.set(Some(ParseFailure::Deadline));
            return Err(E::custom("catalog deadline exceeded"));
        }
        let Some(nodes) = self.nodes.get().checked_add(1) else {
            self.failure.set(Some(ParseFailure::ResourceLimit));
            return Err(E::custom("catalog JSON node limit exceeded"));
        };
        if nodes > AI_GATEWAY_MODEL_CATALOG_MAX_JSON_NODES {
            self.failure.set(Some(ParseFailure::ResourceLimit));
            return Err(E::custom("catalog JSON node limit exceeded"));
        }
        self.nodes.set(nodes);
        Ok(())
    }

    fn check_depth<E: serde::de::Error>(&self, depth: usize) -> Result<(), E> {
        if depth >= AI_GATEWAY_MODEL_CATALOG_MAX_JSON_DEPTH {
            self.failure.set(Some(ParseFailure::ResourceLimit));
            Err(E::custom("catalog JSON depth limit exceeded"))
        } else {
            Ok(())
        }
    }

    fn fail<E: serde::de::Error>(&self, failure: ParseFailure, message: &str) -> E {
        self.failure.set(Some(failure));
        E::custom(message)
    }
}

#[derive(Debug)]
struct Candidate {
    id: String,
    released: i64,
    has_tool_use: bool,
}

#[derive(Clone, Copy)]
struct CatalogSeed<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> DeserializeSeed<'de> for CatalogSeed<'_> {
    type Value = Vec<Candidate>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(CatalogVisitor {
            context: self.context,
        })
    }
}

struct CatalogVisitor<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> Visitor<'de> for CatalogVisitor<'_> {
    type Value = Vec<Candidate>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a model catalog object containing a data array")
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut data = None;
        while let Some(key) = entries.next_key::<String>()? {
            if key == "data" {
                if data.is_some() {
                    return Err(self
                        .context
                        .fail(ParseFailure::Malformed, "duplicate catalog data field"));
                }
                data = Some(entries.next_value_seed(DataSeed {
                    context: self.context,
                })?);
            } else {
                let _: &RawValue = entries.next_value()?;
            }
        }
        data.ok_or_else(|| {
            self.context
                .fail(ParseFailure::Malformed, "missing catalog data field")
        })
    }
}

#[derive(Clone, Copy)]
struct DataSeed<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> DeserializeSeed<'de> for DataSeed<'_> {
    type Value = Vec<Candidate>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(DataVisitor {
            context: self.context,
        })
    }
}

struct DataVisitor<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> Visitor<'de> for DataVisitor<'_> {
    type Value = Vec<Candidate>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded model catalog data array")
    }

    fn visit_seq<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut raw_entries = 0_usize;
        let mut accepted = Vec::new();
        let mut accepted_id_bytes = 0_usize;
        let mut ids = BTreeSet::new();
        while let Some(candidate) = entries.next_element_seed(EntrySeed {
            context: self.context,
        })? {
            raw_entries = raw_entries.checked_add(1).ok_or_else(|| {
                self.context.fail(
                    ParseFailure::ResourceLimit,
                    "catalog raw entry limit exceeded",
                )
            })?;
            if raw_entries > AI_GATEWAY_MODEL_CATALOG_MAX_RAW_ENTRIES {
                return Err(self.context.fail(
                    ParseFailure::ResourceLimit,
                    "catalog raw entry limit exceeded",
                ));
            }
            let Some(candidate) = candidate else {
                continue;
            };
            if AvailableModel::new(candidate.id.clone()).is_err() {
                return Err(self.context.fail(
                    ParseFailure::Malformed,
                    "catalog contains an invalid model identifier",
                ));
            }
            if !ids.insert(candidate.id.clone()) {
                return Err(self.context.fail(
                    ParseFailure::Malformed,
                    "catalog contains a duplicate model identifier",
                ));
            }
            accepted_id_bytes = accepted_id_bytes
                .checked_add(candidate.id.len())
                .ok_or_else(|| {
                    self.context.fail(
                        ParseFailure::ResourceLimit,
                        "catalog model identifier byte limit exceeded",
                    )
                })?;
            if accepted_id_bytes > AI_GATEWAY_MODEL_CATALOG_MAX_MODEL_ID_BYTES
                || accepted.len() >= AI_GATEWAY_MODEL_CATALOG_MAX_MODELS
            {
                return Err(self.context.fail(
                    ParseFailure::ResourceLimit,
                    "catalog accepted model limit exceeded",
                ));
            }
            accepted.push(candidate);
        }
        Ok(accepted)
    }
}

#[derive(Clone, Copy)]
struct EntrySeed<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> DeserializeSeed<'de> for EntrySeed<'_> {
    type Value = Option<Candidate>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = <&RawValue>::deserialize(deserializer)?;
        if first_non_whitespace(raw.get().as_bytes()) != Some(b'{') {
            return Ok(None);
        }

        let mut entry = serde_json::Deserializer::from_str(raw.get());
        let candidate = EntryObjectSeed {
            context: self.context,
        }
        .deserialize(&mut entry)
        .map_err(|_| {
            self.context
                .fail(ParseFailure::Malformed, "malformed catalog entry object")
        })?;
        entry.end().map_err(|_| {
            self.context.fail(
                ParseFailure::Malformed,
                "trailing data in catalog entry object",
            )
        })?;
        Ok(candidate)
    }
}

#[derive(Clone, Copy)]
struct EntryObjectSeed<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> DeserializeSeed<'de> for EntryObjectSeed<'_> {
    type Value = Option<Candidate>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_map(EntryObjectVisitor {
            context: self.context,
        })
    }
}

struct EntryObjectVisitor<'a> {
    context: &'a ParseContext<'a>,
}

impl<'de> Visitor<'de> for EntryObjectVisitor<'_> {
    type Value = Option<Candidate>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded catalog entry object")
    }

    fn visit_map<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut id: Option<&RawValue> = None;
        let mut model_type: Option<&RawValue> = None;
        let mut released: Option<&RawValue> = None;
        let mut tags: Option<&RawValue> = None;
        while let Some(key) = entries.next_key::<String>()? {
            match key.as_str() {
                "id" => {
                    if id.is_some() {
                        return Err(self.context.fail(
                            ParseFailure::Malformed,
                            "duplicate recognized catalog entry field",
                        ));
                    }
                    id = Some(entries.next_value()?);
                }
                "type" => {
                    if model_type.is_some() {
                        return Err(self.context.fail(
                            ParseFailure::Malformed,
                            "duplicate recognized catalog entry field",
                        ));
                    }
                    model_type = Some(entries.next_value()?);
                }
                "released" => {
                    if released.is_some() {
                        return Err(self.context.fail(
                            ParseFailure::Malformed,
                            "duplicate recognized catalog entry field",
                        ));
                    }
                    released = Some(entries.next_value()?);
                }
                "tags" => {
                    if tags.is_some() {
                        return Err(self.context.fail(
                            ParseFailure::Malformed,
                            "duplicate recognized catalog entry field",
                        ));
                    }
                    tags = Some(entries.next_value()?);
                }
                _ => {
                    let _: &RawValue = entries.next_value()?;
                }
            }
        }

        let is_language = match raw_json_string(model_type)? {
            Some(value) => value.eq_ignore_ascii_case("language"),
            None => true,
        };
        if !is_language {
            return Ok(None);
        }
        let Some(id) = raw_json_string(id)? else {
            return Ok(None);
        };
        let released = released
            .and_then(|value| serde_json::from_str::<i64>(value.get()).ok())
            .unwrap_or(0);
        let has_tool_use = raw_tags_have_tool_use(tags)?;
        Ok(Some(Candidate {
            id,
            released,
            has_tool_use,
        }))
    }
}

fn raw_json_string<E: serde::de::Error>(raw: Option<&RawValue>) -> Result<Option<String>, E> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if first_non_whitespace(raw.get().as_bytes()) != Some(b'"') {
        return Ok(None);
    }
    serde_json::from_str(raw.get())
        .map(Some)
        .map_err(|_| E::custom("malformed string in catalog entry"))
}

fn raw_tags_have_tool_use<E: serde::de::Error>(raw: Option<&RawValue>) -> Result<bool, E> {
    let Some(raw) = raw else {
        return Ok(false);
    };
    if first_non_whitespace(raw.get().as_bytes()) != Some(b'[') {
        return Ok(false);
    }
    let mut deserializer = serde_json::Deserializer::from_str(raw.get());
    let has_tool_use = TagsSeed
        .deserialize(&mut deserializer)
        .map_err(|_| E::custom("malformed tags in catalog entry"))?;
    deserializer
        .end()
        .map_err(|_| E::custom("trailing data in catalog tags"))?;
    Ok(has_tool_use)
}

#[derive(Clone, Copy)]
struct TagsSeed;

impl<'de> DeserializeSeed<'de> for TagsSeed {
    type Value = bool;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(TagsVisitor)
    }
}

struct TagsVisitor;

impl<'de> Visitor<'de> for TagsVisitor {
    type Value = bool;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a catalog tags array")
    }

    fn visit_seq<A>(self, mut entries: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut has_tool_use = false;
        while let Some(raw) = entries.next_element::<&RawValue>()? {
            if !has_tool_use && first_non_whitespace(raw.get().as_bytes()) == Some(b'"') {
                let tag: String = serde_json::from_str(raw.get())
                    .map_err(|_| <A::Error as serde::de::Error>::custom("malformed catalog tag"))?;
                has_tool_use = tag.eq_ignore_ascii_case("tool-use");
            }
        }
        Ok(has_tool_use)
    }
}

fn scan_raw_value<E: serde::de::Error>(
    raw: &str,
    depth: usize,
    context: &ParseContext<'_>,
) -> Result<(), E> {
    let mut scanner = RawJsonScanner {
        bytes: raw.as_bytes(),
        cursor: 0,
        context,
    };
    scanner.scan_value(depth)?;
    scanner.skip_whitespace();
    if scanner.cursor != scanner.bytes.len() {
        return Err(context.fail(
            ParseFailure::Malformed,
            "trailing data in raw catalog value",
        ));
    }
    Ok(())
}

struct RawJsonScanner<'raw, 'context, 'cancel> {
    bytes: &'raw [u8],
    cursor: usize,
    context: &'context ParseContext<'cancel>,
}

impl RawJsonScanner<'_, '_, '_> {
    fn scan_value<E: serde::de::Error>(&mut self, depth: usize) -> Result<(), E> {
        self.context.consume_node()?;
        self.skip_whitespace();
        match self.bytes.get(self.cursor).copied() {
            Some(b'{') => self.scan_object(depth),
            Some(b'[') => self.scan_array(depth),
            Some(b'"') => self.skip_string(),
            Some(_) => self.skip_primitive(),
            None => Err(self
                .context
                .fail(ParseFailure::Malformed, "empty raw catalog value")),
        }
    }

    fn scan_object<E: serde::de::Error>(&mut self, depth: usize) -> Result<(), E> {
        self.context.check_depth(depth)?;
        self.cursor += 1;
        self.skip_whitespace();
        if self.consume_byte(b'}') {
            return Ok(());
        }
        loop {
            if self.bytes.get(self.cursor) != Some(&b'"') {
                return Err(self.context.fail(
                    ParseFailure::Malformed,
                    "catalog object key is not a string",
                ));
            }
            self.skip_string()?;
            self.skip_whitespace();
            if !self.consume_byte(b':') {
                return Err(self
                    .context
                    .fail(ParseFailure::Malformed, "catalog object is missing a colon"));
            }
            self.scan_value(depth + 1)?;
            self.skip_whitespace();
            if self.consume_byte(b'}') {
                return Ok(());
            }
            if !self.consume_byte(b',') {
                return Err(self.context.fail(
                    ParseFailure::Malformed,
                    "catalog object is missing a separator",
                ));
            }
            self.skip_whitespace();
        }
    }

    fn scan_array<E: serde::de::Error>(&mut self, depth: usize) -> Result<(), E> {
        self.context.check_depth(depth)?;
        self.cursor += 1;
        self.skip_whitespace();
        if self.consume_byte(b']') {
            return Ok(());
        }
        loop {
            self.scan_value(depth + 1)?;
            self.skip_whitespace();
            if self.consume_byte(b']') {
                return Ok(());
            }
            if !self.consume_byte(b',') {
                return Err(self.context.fail(
                    ParseFailure::Malformed,
                    "catalog array is missing a separator",
                ));
            }
            self.skip_whitespace();
        }
    }

    fn skip_string<E: serde::de::Error>(&mut self) -> Result<(), E> {
        self.cursor += 1;
        while let Some(byte) = self.bytes.get(self.cursor).copied() {
            self.cursor += 1;
            match byte {
                b'"' => return Ok(()),
                b'\\' => {
                    if self.cursor >= self.bytes.len() {
                        break;
                    }
                    self.cursor += 1;
                }
                _ => {}
            }
        }
        Err(self
            .context
            .fail(ParseFailure::Malformed, "unterminated catalog string"))
    }

    fn skip_primitive<E: serde::de::Error>(&mut self) -> Result<(), E> {
        let start = self.cursor;
        while let Some(byte) = self.bytes.get(self.cursor).copied() {
            if matches!(byte, b' ' | b'\n' | b'\r' | b'\t' | b',' | b']' | b'}') {
                break;
            }
            self.cursor += 1;
        }
        if self.cursor == start {
            Err(self
                .context
                .fail(ParseFailure::Malformed, "empty catalog primitive"))
        } else {
            Ok(())
        }
    }

    fn skip_whitespace(&mut self) {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
        {
            self.cursor += 1;
        }
    }

    fn consume_byte(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.cursor) == Some(&expected) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
}

fn first_non_whitespace(bytes: &[u8]) -> Option<u8> {
    bytes
        .iter()
        .copied()
        .find(|byte| !matches!(byte, b' ' | b'\n' | b'\r' | b'\t'))
}

fn parse_catalog(
    body: &[u8],
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<AvailableModel>, ProviderError> {
    if body.len() > AI_GATEWAY_MODEL_CATALOG_MAX_BODY_BYTES {
        return Err(resource_limit_error());
    }
    let context = ParseContext {
        nodes: Cell::new(0),
        failure: Cell::new(None),
        cancellation,
        deadline,
    };
    check_cancelled(cancellation)?;
    check_deadline(deadline)?;
    let raw = std::str::from_utf8(body).map_err(|_| parse_terminal_error(&context))?;
    scan_raw_value::<serde_json::Error>(raw, 0, &context)
        .map_err(|_| parse_terminal_error(&context))?;
    let mut deserializer = serde_json::Deserializer::from_slice(body);
    let mut candidates = CatalogSeed { context: &context }
        .deserialize(&mut deserializer)
        .map_err(|_| parse_terminal_error(&context))?;
    deserializer
        .end()
        .map_err(|_| parse_terminal_error(&context))?;
    check_cancelled(cancellation)?;
    check_deadline(deadline)?;
    candidates.sort_by(compare_candidates);
    check_cancelled(cancellation)?;
    check_deadline(deadline)?;
    let mut models = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        check_cancelled(cancellation)?;
        check_deadline(deadline)?;
        models.push(AvailableModel::new(candidate.id).map_err(|_| malformed_response_error())?);
    }
    Ok(models)
}

fn compare_candidates(left: &Candidate, right: &Candidate) -> Ordering {
    right
        .has_tool_use
        .cmp(&left.has_tool_use)
        .then_with(|| tier_rank(&left.id).cmp(&tier_rank(&right.id)))
        .then_with(|| provider_rank(&left.id).cmp(&provider_rank(&right.id)))
        .then_with(|| right.released.cmp(&left.released))
        .then_with(|| left.id.cmp(&right.id))
}

fn provider_rank(id: &str) -> u8 {
    if id.starts_with("anthropic/") {
        0
    } else if id.starts_with("openai/") {
        1
    } else if id.starts_with("google/") {
        2
    } else if id.starts_with("xai/") {
        3
    } else if id.starts_with("deepseek/") {
        4
    } else if id.starts_with("meta/") {
        5
    } else if id.starts_with("mistral/") {
        6
    } else if id.starts_with("alibaba/") {
        7
    } else {
        8
    }
}

fn tier_rank(id: &str) -> u8 {
    if contains_ascii_case_insensitive(id, "preview") || contains_ascii_case_insensitive(id, "beta")
    {
        4
    } else if contains_ascii_case_insensitive(id, "haiku")
        || contains_ascii_case_insensitive(id, "mini")
        || contains_ascii_case_insensitive(id, "lite")
    {
        3
    } else if contains_ascii_case_insensitive(id, "flash") {
        2
    } else {
        let premium = ["opus", "sonnet", "gpt-5", "o1", "o3", "o4", "pro", "grok-4"]
            .iter()
            .any(|term| contains_ascii_case_insensitive(id, term));
        u8::from(!premium)
    }
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn parse_error(failure: Option<ParseFailure>) -> ProviderError {
    match failure.unwrap_or(ParseFailure::Malformed) {
        ParseFailure::Malformed => malformed_response_error(),
        ParseFailure::ResourceLimit | ParseFailure::Deadline => resource_limit_error(),
        ParseFailure::Cancelled => cancelled_error(),
    }
}

fn parse_terminal_error(context: &ParseContext<'_>) -> ProviderError {
    if context.cancellation.is_cancelled() {
        cancelled_error()
    } else if Instant::now() >= context.deadline {
        resource_limit_error()
    } else {
        parse_error(context.failure.get())
    }
}

fn status_error(status: u16) -> ProviderError {
    match status {
        401 | 403 => authentication_rejected_error(),
        429 => ProviderError::new(
            ProviderErrorKind::RateLimited,
            "RateLimited",
            "AI Gateway model catalog rate limit reached",
            true,
        ),
        500 | 502 | 503 | 504 => gateway_unavailable_error(true),
        501 | 505..=599 => gateway_unavailable_error(false),
        _ => ProviderError::new(
            ProviderErrorKind::Unavailable,
            "Unavailable",
            "AI Gateway model catalog is unavailable",
            false,
        ),
    }
}

fn map_transport_error(error: AiGatewayModelCatalogTransportError) -> ProviderError {
    match error.kind() {
        AiGatewayModelCatalogTransportErrorKind::Transport => transport_failure_error(),
        AiGatewayModelCatalogTransportErrorKind::MalformedResponse => malformed_response_error(),
        AiGatewayModelCatalogTransportErrorKind::ResourceLimit => resource_limit_error(),
        AiGatewayModelCatalogTransportErrorKind::RuntimeRequired => ProviderError::new(
            ProviderErrorKind::Transport,
            "RuntimeRequired",
            "AI Gateway model catalog requires an active Tokio runtime",
            false,
        ),
        AiGatewayModelCatalogTransportErrorKind::Cancelled => cancelled_error(),
    }
}

fn authentication_rejected_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Authentication,
        "AuthenticationRejected",
        "AI Gateway model catalog authentication failed",
        false,
    )
}

fn gateway_unavailable_error(retryable: bool) -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Unavailable,
        "GatewayUnavailable",
        "AI Gateway model catalog gateway is unavailable",
        retryable,
    )
}

fn malformed_response_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Protocol,
        "MalformedResponse",
        "AI Gateway model catalog response is malformed",
        false,
    )
}

fn resource_limit_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Protocol,
        "ResourceLimit",
        "AI Gateway model catalog resource limit exceeded",
        false,
    )
}

fn cancelled_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Cancelled,
        "Cancelled",
        "AI Gateway model catalog request cancelled",
        false,
    )
}

fn transport_failure_error() -> ProviderError {
    ProviderError::new(
        ProviderErrorKind::Transport,
        "TransportFailure",
        "AI Gateway model catalog transport failed",
        true,
    )
}

fn check_deadline(deadline: Instant) -> Result<(), ProviderError> {
    if Instant::now() >= deadline {
        Err(resource_limit_error())
    } else {
        Ok(())
    }
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), ProviderError> {
    if cancellation.is_cancelled() {
        Err(cancelled_error())
    } else {
        Ok(())
    }
}
