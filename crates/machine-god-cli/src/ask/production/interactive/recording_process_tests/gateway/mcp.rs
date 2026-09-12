//! Modern producer fixture using the parent's bounded HTTP and worker owner.
use serde_json::{Value, json};
use std::{io, sync::Mutex};

#[derive(Default)]
pub(super) struct Requests(Mutex<Vec<Value>>);
impl Requests {
    pub(super) fn snapshot(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    pub(super) fn reply(&self, bytes: &[u8]) -> io::Result<String> {
        let request =
            machine_god_core::json::from_slice(bytes).map_err(|_| io::ErrorKind::InvalidData)?;
        assert_eq!(
            request["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
            "2026-07-28"
        );
        assert!(request["id"].as_i64().is_some_and(|id| id > 0));
        let mut captured = self.0.lock().unwrap();
        assert!(captured.len() < 16);
        captured.push(request.clone());
        drop(captured);
        let mut result = match request["method"].as_str() {
            Some("server/discover") => {
                json!({"supportedVersions":["2026-07-28"],"capabilities":{"tools":{},"resources":{},"prompts":{}}})
            }
            Some("tools/list") => json!({"tools":[],"ttlMs":300_000}),
            Some("resources/list") => {
                json!({"resources":[{"uri":"test://fixed","name":"MCP process resource"},{"uri":"test://confirm","name":"MCP confirmation resource"}],"ttlMs":300_000})
            }
            Some("resources/read") => read(&request),
            _ => return Err(io::ErrorKind::InvalidData.into()),
        };
        if result.get("resultType").is_none() {
            result["resultType"] = "complete".into();
        }
        serde_json::to_string(&json!({"jsonrpc":"2.0","id":request["id"],"result":result}))
            .map_err(|_| io::ErrorKind::InvalidData.into())
    }
}

fn read(request: &Value) -> Value {
    let uri = request["params"]["uri"].as_str().unwrap();
    assert!(matches!(uri, "test://fixed" | "test://confirm"));
    if uri == "test://confirm" {
        if let Some(answer) = request["params"].get("inputResponses") {
            assert_eq!(answer, &json!({"confirm":{"action":"cancel"}}));
            assert_eq!(
                request["params"]["requestState"],
                json!({"exact":"MCP original state"})
            );
            return json!({"contents":[{"uri":uri,"text":"MCP cancellation acknowledged"}]});
        }
        return json!({"resultType":"input_required","requestState":{"exact":"MCP original state"},"inputRequests":{"confirm":{"method":"elicitation/create","params":{"message":"MCP process confirmation","requestedSchema":{"type":"object","properties":{}}}}}});
    }
    json!({"contents":[{"uri":uri,"text":"MCP process resource content"}]})
}
