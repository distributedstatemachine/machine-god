//! Bounded localhost OAuth and modern MCP producer, with exact request evidence.

use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener as AsyncListener, TcpStream},
};

pub(super) struct Server {
    listener: AsyncListener,
    pub origin: String,
}
struct Request {
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Server {
    pub(super) fn new(fixture: &Fixture) -> Self {
        Self {
            listener: AsyncListener::from_std(fixture.listener.try_clone().unwrap()).unwrap(),
            origin: format!(
                "http://127.0.0.1:{}",
                fixture.listener.local_addr().unwrap().port()
            ),
        }
    }
    async fn reply(&self, status: u16, body: Value) -> Request {
        let (mut socket, _) = self.listener.accept().await.unwrap();
        let request = read(&mut socket).await;
        let bytes = serde_json::to_vec(&body).unwrap();
        drop(body);
        let body = bytes;
        socket.write_all(format!("HTTP/1.1 {status} Status\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        request
    }
    pub(super) async fn discovery(&self) -> String {
        let resource = self.reply(200, json!({"resource":format!("{}/mcp", self.origin),"authorization_servers":[self.origin],"scopes_supported":["read"]})).await;
        assert_eq!(
            resource.target,
            "GET /.well-known/oauth-protected-resource/mcp HTTP/1.1"
        );
        assert!(!resource.headers.contains_key("authorization"));
        let issuer = self.reply(200, json!({"issuer":self.origin,"authorization_endpoint":format!("{}/authorize",self.origin),
            "token_endpoint":format!("{}/token",self.origin),"registration_endpoint":format!("{}/register",self.origin),
            "revocation_endpoint":format!("{}/revoke",self.origin),"code_challenge_methods_supported":["S256"],
            "token_endpoint_auth_methods_supported":["none"],"grant_types_supported":["authorization_code","refresh_token"],
            "scopes_supported":["offline_access"],"authorization_response_iss_parameter_supported":true})).await;
        assert_eq!(
            issuer.target,
            "GET /.well-known/oauth-authorization-server HTTP/1.1"
        );
        assert!(!issuer.headers.contains_key("authorization"));
        let registration = self
            .reply(
                201,
                json!({"client_id":"local-client","token_endpoint_auth_method":"none"}),
            )
            .await;
        assert_eq!(registration.target, "POST /register HTTP/1.1");
        let value: Value = serde_json::from_slice(&registration.body).unwrap();
        assert_eq!(value["application_type"], "native");
        assert_eq!(value["token_endpoint_auth_method"], "none");
        assert_eq!(value["redirect_uris"].as_array().unwrap().len(), 1);
        value["redirect_uris"][0].as_str().unwrap().to_owned()
    }
    pub(super) async fn token(&self) -> BTreeMap<String, String> {
        let request = self.reply(200, json!({"access_token":"accepted-access-secret","refresh_token":"accepted-refresh-secret","expires_in":3600,"token_type":"Bearer"})).await;
        assert_eq!(request.target, "POST /token HTTP/1.1");
        assert!(!request.headers.contains_key("authorization"));
        let fields = form(&request);
        assert_eq!(fields["grant_type"], "authorization_code");
        assert_eq!(fields["code"], "local-code");
        assert_eq!(fields["resource"], format!("{}/mcp", self.origin));
        assert_eq!(fields["client_id"], "local-client");
        assert_eq!(fields["code_verifier"].len(), 64);
        fields
    }
    pub(super) async fn activate(&self, accepted: bool) {
        let request = self.reply(if accepted { 200 } else { 403 }, json!({"jsonrpc":"2.0","id":1,"result":{
            "resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"resources":{}}}})).await;
        let request = authenticated(&request);
        assert_eq!(request["id"], 1);
        assert_eq!(request["method"], "server/discover");
    }
    pub(super) async fn resource(&self) {
        let list = self.reply(200, json!({"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","resources":[{"uri":"memo://private","name":"Private note"}],"ttlMs":60000}})).await;
        assert_eq!(authenticated(&list)["method"], "resources/list");
        let read = self.reply(200, json!({"jsonrpc":"2.0","id":3,"result":{"resultType":"complete","contents":[{"uri":"memo://private","text":"authenticated private result"}]}})).await;
        let read = authenticated(&read);
        assert_eq!(read["method"], "resources/read");
        assert_eq!(read["params"]["uri"], "memo://private");
    }
    pub(super) async fn logout(&self, accepted: bool) {
        for (hint, token) in [
            ("refresh_token", "accepted-refresh-secret"),
            ("access_token", "accepted-access-secret"),
        ] {
            let request = self
                .reply(if accepted { 200 } else { 500 }, json!({}))
                .await;
            assert_eq!(request.target, "POST /revoke HTTP/1.1");
            let fields = form(&request);
            assert_eq!(fields["token_type_hint"], hint);
            assert_eq!(fields["token"], token);
            assert_eq!(fields["client_id"], "local-client");
            assert!(!request.headers.contains_key("authorization"));
        }
    }
}

fn form(request: &Request) -> BTreeMap<String, String> {
    assert_eq!(
        request.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    url::form_urlencoded::parse(&request.body)
        .into_owned()
        .collect()
}
fn authenticated(request: &Request) -> Value {
    assert_eq!(request.target, "POST /mcp HTTP/1.1");
    assert_eq!(
        request.headers["authorization"],
        "Bearer accepted-access-secret"
    );
    serde_json::from_slice(&request.body).unwrap()
}
async fn read(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let end = loop {
        let mut chunk = [0; 1024];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() <= 64 * 1024);
        if let Some(index) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let mut lines = std::str::from_utf8(&bytes[..end]).unwrap().split("\r\n");
    let target = lines.next().unwrap().to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<BTreeMap<_, _>>();
    let length = headers
        .get("content-length")
        .map_or(0, |value| value.parse::<usize>().unwrap());
    assert!(length <= 32 * 1024);
    while bytes.len() < end + length {
        let mut chunk = [0; 1024];
        let count = socket.read(&mut chunk).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&chunk[..count]);
    }
    assert_eq!(bytes.len(), end + length);
    Request {
        target,
        headers,
        body: bytes[end..].to_vec(),
    }
}
