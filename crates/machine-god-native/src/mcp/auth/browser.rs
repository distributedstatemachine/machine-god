use super::{
    Authority, McpAuthBrowser, McpAuthBrowserRequest, McpAuthError, Result,
    codec::{Credentials, McpAuthChallenge, McpAuthConfig, Registration, Secret},
    discovery::Discovery,
    token,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use machine_god_core::CancellationToken;
use sha2::{Digest, Sha256};
use std::{
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
const CALLBACK_BODY: &str = "Authorization received. Return to machine-god.";

impl Authority {
    pub(super) async fn authorize(
        &self,
        config: &McpAuthConfig,
        challenge: &McpAuthChallenge,
        previous: Option<&str>,
        browser: &dyn McpAuthBrowser,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Credentials> {
        let deadline = deadline.min(
            self.clock
                .now()
                .checked_add(Duration::from_secs(5 * 60))
                .ok_or(McpAuthError::Deadline)?,
        );
        self.check(cancellation, deadline)?;
        let discovery = self
            .discover(config, challenge, previous, cancellation, deadline)
            .await?;
        // This socket owns callback authority before registration fixes its URI.
        let listener = self
            .bounded(
                async {
                    TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
                        .await
                        .map_err(|_| McpAuthError::Network)
                },
                cancellation,
                deadline,
            )
            .await?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener
                .local_addr()
                .map_err(|_| McpAuthError::Network)?
                .port()
        );
        let registration = self
            .registration(
                config,
                &discovery.metadata,
                &redirect,
                cancellation,
                deadline,
            )
            .await?;
        let (verifier, state, request) =
            self.browser_request(&discovery, &registration, &redirect)?;
        if !self
            .bounded(
                browser.approve(&request, cancellation, deadline),
                cancellation,
                deadline,
            )
            .await?
        {
            return Err(McpAuthError::Denied);
        }
        self.bounded(
            browser.launch(&request, cancellation, deadline),
            cancellation,
            deadline,
        )
        .await?;
        let code = self
            .callback(
                &listener,
                state.text(),
                &discovery.metadata.issuer,
                discovery.metadata.require_issuer,
                cancellation,
                deadline,
            )
            .await?;
        drop(listener);
        let mut credentials = Credentials {
            identity: config.identity.clone(),
            resource: discovery.resource,
            issuer: discovery.metadata.issuer,
            registration,
            access: Secret::new(b"pending")?,
            refresh: None,
            scope: discovery.scope,
            expires_ms: 0,
            authorization_endpoint: discovery.metadata.authorization,
            token_endpoint: discovery.metadata.token,
            revocation_endpoint: discovery.metadata.revocation,
        };
        let resource = credentials.resource.clone();
        self.token(
            &mut credentials,
            &[
                ("grant_type", "authorization_code"),
                ("code", code.text()),
                ("redirect_uri", &redirect),
                ("code_verifier", verifier.text()),
                ("resource", &resource),
            ],
            cancellation,
            deadline,
        )
        .await?;
        credentials.validate()?;
        Ok(credentials)
    }
    fn browser_request(
        &self,
        discovery: &Discovery,
        registration: &Registration,
        redirect: &str,
    ) -> Result<(Secret, Secret, McpAuthBrowserRequest)> {
        let mut entropy = [0u8; 80];
        if let Err(error) = self.entropy.fill(&mut entropy) {
            entropy.fill(0);
            return Err(error);
        }
        let verifier = Secret::new(URL_SAFE_NO_PAD.encode(&entropy[..48]).as_bytes())?;
        let state = Secret::new(URL_SAFE_NO_PAD.encode(&entropy[48..]).as_bytes())?;
        entropy.fill(0);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.bytes()));
        let fields = token::form(&[
            ("response_type", "code"),
            ("client_id", registration.id.text()),
            ("redirect_uri", redirect),
            ("resource", &discovery.resource),
            ("state", state.text()),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("scope", &discovery.scope),
        ]);
        let separator = if discovery.metadata.authorization.contains('?') {
            '&'
        } else {
            '?'
        };
        let url = format!("{}{separator}{fields}", discovery.metadata.authorization);
        if url.len() > 32 * 1024 {
            return Err(McpAuthError::Limit);
        }
        Ok((
            verifier,
            state,
            McpAuthBrowserRequest {
                url: url.into(),
                issuer: discovery.metadata.issuer.clone(),
            },
        ))
    }
    async fn callback(
        &self,
        listener: &TcpListener,
        state: &str,
        issuer: &str,
        required_issuer: bool,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<Secret> {
        let (mut stream, address) = self
            .bounded(
                async { listener.accept().await.map_err(|_| McpAuthError::Network) },
                cancellation,
                deadline,
            )
            .await?;
        if !address.ip().is_loopback() {
            return Err(McpAuthError::Invalid);
        }
        let deadline = deadline.min(
            self.clock
                .now()
                .checked_add(Duration::from_secs(30))
                .ok_or(McpAuthError::Deadline)?,
        );
        self.bounded(async {
            let mut bytes = Vec::new();
            let target = loop {
                let mut buffer = [0u8; 1024];
                let read = stream.read(&mut buffer).await.map_err(|_| McpAuthError::Network)?;
                if read == 0 { return Err(McpAuthError::Invalid); }
                if read > (16 * 1024usize).saturating_sub(bytes.len()) { return Err(McpAuthError::Limit); }
                bytes.extend_from_slice(&buffer[..read]);
                let mut headers = [httparse::EMPTY_HEADER; 64];
                let mut request = httparse::Request::new(&mut headers);
                if request.parse(&bytes).map_err(|_| McpAuthError::Invalid)?.is_complete() {
                    if request.method != Some("GET") || request.headers.iter().any(|h| h.name.eq_ignore_ascii_case("transfer-encoding")) { return Err(McpAuthError::Invalid); }
                    break request.path.ok_or(McpAuthError::Invalid)?.to_owned();
                }
            };
            let code = callback_code(&target, state, issuer, required_issuer)?;
            bytes.fill(0);
            let response = format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{CALLBACK_BODY}", CALLBACK_BODY.len());
            stream.write_all(response.as_bytes()).await.map_err(|_| McpAuthError::Network)?;
            stream.shutdown().await.map_err(|_| McpAuthError::Network)?;
            Ok(code)
        }, cancellation, deadline).await
    }
}
pub(super) fn callback_code(
    target: &str,
    state: &str,
    issuer: &str,
    required_issuer: bool,
) -> Result<Secret> {
    let query = target
        .strip_prefix("/callback?")
        .filter(|s| !s.contains('#'))
        .ok_or(McpAuthError::Invalid)?;
    let mut fields = std::collections::BTreeMap::new();
    // Reject malformed escapes before the URL decoder's lossy recovery.
    for (index, byte) in query.bytes().enumerate() {
        if byte == b'%'
            && !query
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|s| s.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(McpAuthError::Invalid);
        }
    }
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(McpAuthError::Invalid);
        }
    }
    if fields.contains_key("error") {
        return Err(McpAuthError::Rejected);
    }
    if fields.get("state").map(String::as_str) != Some(state) {
        return Err(McpAuthError::StateMismatch);
    }
    let returned = fields.get("iss").map(String::as_str);
    if returned.is_some_and(|value| value != issuer) || required_issuer && returned.is_none() {
        return Err(McpAuthError::IssuerMismatch);
    }
    Secret::new(fields.get("code").ok_or(McpAuthError::Invalid)?.as_bytes())
}
