//! Mechanically transcribed representative validation corpus from pinned fx
//! b1774fbf6c7602b503026f96f6e960e946c692ef `json_schema.zig`.
//! Upstream cases derive from JSON Schema Test Suite
//! be54236db6e8e6bb2e098ed16fb4c61e73f5a9ac.
pub(super) const CASES: &[(&str, &str, bool)] = &[
    (r"true", r"null", true),
    (r"false", r"null", false),
    (r#"{"type":["string","null"]}"#, r"null", true),
    (r#"{"enum":[1,"x",null]}"#, r#""y""#, false),
    (r#"{"const":{"a":[1,2]}}"#, r#"{"a":[1,2]}"#, true),
    (r#"{"multipleOf":0.25}"#, r"1.5", true),
    (r#"{"multipleOf":0.25}"#, r"1.6", false),
    (r#"{"minimum":2,"maximum":4}"#, r"5", false),
    (
        r#"{"exclusiveMinimum":2,"exclusiveMaximum":4}"#,
        r"2",
        false,
    ),
    (r#"{"minLength":2,"maxLength":2}"#, r#""éa""#, true),
    (r#"{"pattern":"^\\p{Letter}+$"}"#, r#""π""#, true),
    (r#"{"pattern":"^[a-z]+$"}"#, r#""123""#, false),
    (r#"{"minItems":2,"maxItems":3}"#, r"[1]", false),
    (r#"{"uniqueItems":true}"#, r#"[{"a":1},{"a":1.0}]"#, false),
    (
        r#"{"contains":{"type":"integer"},"minContains":2,"maxContains":2}"#,
        r#"[1,2,"x"]"#,
        true,
    ),
    (
        r#"{"prefixItems":[{"type":"string"}],"items":false}"#,
        r#"["x",2]"#,
        false,
    ),
    (
        r#"{"minProperties":2,"maxProperties":2}"#,
        r#"{"a":1}"#,
        false,
    ),
    (r#"{"required":["a","b"]}"#, r#"{"a":1}"#, false),
    (
        r#"{"properties":{"a":{"type":"integer"}}}"#,
        r#"{"a":"x"}"#,
        false,
    ),
    (
        r#"{"properties":{"a":true},"additionalProperties":false}"#,
        r#"{"a":1,"b":2}"#,
        false,
    ),
    (
        r#"{"patternProperties":{"^x":{"type":"integer"}},"additionalProperties":false}"#,
        r#"{"x-value":1}"#,
        true,
    ),
    (r#"{"propertyNames":{"minLength":2}}"#, r#"{"a":1}"#, false),
    (
        r#"{"dependentRequired":{"card":["billing"]}}"#,
        r#"{"card":1}"#,
        false,
    ),
    (
        r#"{"dependentSchemas":{"card":{"required":["billing"]}}}"#,
        r#"{"card":1,"billing":2}"#,
        true,
    ),
    (
        r#"{"allOf":[{"type":"integer"},{"minimum":2}]}"#,
        r"1",
        false,
    ),
    (
        r#"{"anyOf":[{"type":"integer"},{"type":"string"}]}"#,
        r"true",
        false,
    ),
    (r#"{"not":{"type":"integer"}}"#, r#""x""#, true),
    (
        r#"{"allOf":[{"properties":{"known":true}}],"unevaluatedProperties":false}"#,
        r#"{"known":1}"#,
        true,
    ),
    (
        r#"{"prefixItems":[true],"unevaluatedItems":false}"#,
        r"[1,2]",
        false,
    ),
    (
        r#"{"format":"email","contentEncoding":"base64"}"#,
        r#""not asserted""#,
        true,
    ),
];
