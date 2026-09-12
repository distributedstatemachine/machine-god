//! Bounded modern localhost producer; all connections remain owned by the test.
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

pub(super) const NAME: &str = "mcp_remote_lookup";

async fn request(socket: &mut TcpStream, method: &str) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut scratch = [0; 4096];
        let count = socket.read(&mut scratch).await.unwrap();
        assert_ne!(count, 0);
        bytes.extend_from_slice(&scratch[..count]);
        assert!(bytes.len() <= 256 * 1024);
        let Some(head) = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let headers = std::str::from_utf8(&bytes[..head]).unwrap();
        assert!(headers.starts_with("POST /mcp HTTP/1.1\r\n"));
        let length: usize = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .unwrap()
            .1
            .trim()
            .parse()
            .unwrap();
        if bytes.len() < head + 4 + length {
            continue;
        }
        assert_eq!(bytes.len(), head + 4 + length);
        let request = machine_god_core::json::from_slice(&bytes[head + 4..]).unwrap();
        assert_eq!(request["method"], method);
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert!(request["id"].as_i64().is_some_and(|id| id > 0));
        return request;
    }
}

async fn reply(listener: &TcpListener, method: &str, mut result: Value) -> Value {
    let (mut socket, _) = listener.accept().await.unwrap();
    let received = request(&mut socket, method).await;
    result["resultType"] = "complete".into();
    let body =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":received["id"],"result":result})).unwrap();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(&body).await.unwrap();
    socket.shutdown().await.unwrap();
    received
}

pub(super) async fn startup(listener: &TcpListener, revision: &str) -> TcpStream {
    assert_eq!(reply(listener, "server/discover", json!({"supportedVersions":["2026-07-28"],"capabilities":{"tools":{"listChanged":true}}})).await["id"], 1);
    assert_eq!(reply(listener, "tools/list", json!({"tools":[{
        "name":"lookup","description":format!("Configured lookup {revision}"),
        "inputSchema":{"type":"object","properties":{"revision":{"type":"string"}},"required":["revision"],"additionalProperties":false}
    }],"ttlMs":300_000})).await["id"], 2);
    let (mut socket, _) = listener.accept().await.unwrap();
    let request = request(&mut socket, "subscriptions/listen").await;
    assert_eq!(request["id"], 3);
    assert_eq!(
        request["params"]["notifications"],
        json!({"toolsListChanged":true})
    );
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let ack = json!({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{
        "_meta":{"io.modelcontextprotocol/subscriptionId":3},"notifications":{"toolsListChanged":true}}});
    socket
        .write_all(format!("data: {ack}\n\n").as_bytes())
        .await
        .unwrap();
    socket
}

pub(super) async fn call(listener: &TcpListener, id: i64, revision: &str) {
    let result = json!({"content":[{"type":"text","text":"x".repeat(70_000)}],
        "structuredContent":{"revision":revision,"marker":"configured-archive-end"}});
    let request = reply(listener, "tools/call", result).await;
    assert_eq!(request["id"], id);
    assert_eq!(request["params"]["name"], "lookup");
    assert_eq!(request["params"]["arguments"], json!({"revision":revision}));
}

pub(super) async fn closed(mut socket: TcpStream) {
    let mut byte = [0];
    assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
}
