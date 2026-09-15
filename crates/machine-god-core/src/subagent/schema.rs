use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    let mut schema = json!({"type":"object","required":required,"additionalProperties":false});
    schema["properties"] = properties;
    schema
}
fn string(bytes: usize) -> Value {
    json!({"type":"string","minLength":1,"maxLength":bytes,"description":"UTF-8 byte bound; NUL is rejected"})
}
fn options(values: &[&str]) -> Value {
    json!({"type":"string","enum":values})
}
fn exactly_one(properties: Value, names: &[&str]) -> Value {
    let mut schema = object(properties, &[]);
    schema["oneOf"] = Value::Array(
        names
            .iter()
            .map(|name| json!({"required":[name]}))
            .collect(),
    );
    schema
}
pub(super) fn input_schema() -> Value {
    let id = json!({"type":"string","minLength":1,"maxLength":255,"pattern":"^[A-Za-z0-9._-]+$","description":"Not . or .."});
    let effort =
        json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9._-]+$"});
    let permission = options(&["ask", "auto", "yolo"]);
    let positive_ms = json!({"type":"integer","minimum":1,"maximum":i64::MAX});
    let notifications = object(
        json!({
            "terminal":object(json!({"completed":{"type":"boolean","default":true},"failed":{"type":"boolean","default":true},"cancelled":{"type":"boolean","default":true}}),&[]),
            "started":{"type":"boolean","default":false},
            "milestones":{"type":"array","items":string(128),"maxItems":32,"uniqueItems":true},
            "report_interval_ms":positive_ms,"report_duration_ms":positive_ms,
            "stop_conditions":{"type":"array","items":options(&["terminal","duration_elapsed"]),"maxItems":8,"uniqueItems":true,"default":["terminal"]}
        }),
        &[],
    );
    let mut create = object(
        json!({"name":string(128),"mode":options(&["one_off","persistent"]),
        "prompt":string(65536),"model":string(256),"effort":effort,"permission_mode":permission,"notifications":notifications}),
        &["name", "mode"],
    );
    create["allOf"] =
        json!([{"if":{"properties":{"mode":{"const":"one_off"}}},"then":{"required":["prompt"]}}]);
    let wait = object(
        json!({"until":options(&["settled"]),"after_generation":{"type":"integer","minimum":0},
        "timeout_ms":{"type":"integer","minimum":1,"maximum":60000}}),
        &["until", "timeout_ms"],
    );
    let inspect = object(
        json!({"id":id,"sections":{"type":"array","minItems":1,"maxItems":6,"uniqueItems":true,
        "items":options(&["status","messages","tool_activity","events","configuration","relationship"])},
        "cursor":{"type":"string","pattern":"^v1:[0-9]+:[0-9]+$"},
        "limit":{"type":"integer","minimum":1,"maximum":100,"default":50},"wait":wait}),
        &["id", "sections"],
    );
    let message = exactly_one(
        json!({"send":object(json!({"id":id,"content":string(65536)}),&["id","content"]),
        "milestone":object(json!({"name":string(128)}),&["name"])}),
        &["send", "milestone"],
    );
    let relationship = object(
        json!({"id":id,"action":options(&["attach","detach","reparent"]),"parent_id":id}),
        &["id", "action"],
    );
    let mut configure = object(
        json!({"id":id,"name":string(128),"model":string(256),"effort":effort,
        "permission_mode":permission,"notifications":notifications}),
        &["id"],
    );
    configure["anyOf"] = json!([{"required":["name"]},{"required":["model"]},{"required":["effort"]},{"required":["permission_mode"]},{"required":["notifications"]}]);
    let lifecycle = object(
        json!({"id":id,"action":options(&["cancel","resume","close","reopen"])}),
        &["id", "action"],
    );
    object(
        json!({"command":exactly_one(json!({"create":create,"inspect":inspect,"message":message,
        "relationship":relationship,"configure":configure,"lifecycle":lifecycle}), &["create","inspect","message","relationship","configure","lifecycle"])}),
        &["command"],
    )
}
