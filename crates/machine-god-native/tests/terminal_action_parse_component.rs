use machine_god_core::{
    TerminalActionRequest, TerminalProfile, TerminalReturnCondition, TerminalWritePayload,
};
use machine_god_native::{decode_terminal_action, terminal_action_requested_cwd};
use serde_json::{Value, json};

fn parse(value: &Value) -> TerminalActionRequest {
    decode_terminal_action(value, "/trusted/workspace").unwrap()
}

#[test]
fn all_twelve_public_actions_normalize_and_roundtrip() {
    for value in [
        json!({"action":"exec","command":"printf ready"}),
        json!({"action":"start"}),
        json!({"action":"read","session_id":"terminal-a","cursor_segment":1}),
        json!({"action":"screen","session_id":"terminal-a"}),
        json!({"action":"write","session_id":"terminal-a","lease":"acquire"}),
        json!({"action":"wait","session_id":"terminal-a","return_when":{"kind":"exit"},"wait_ceiling_ms":1000}),
        json!({"action":"monitor","session_id":"terminal-a","monitor":{"kind":"remove","monitor_id":"monitor-a"}}),
        json!({"action":"inspect","session_id":"terminal-a"}),
        json!({"action":"list"}),
        json!({"action":"resize","session_id":"terminal-a","rows":24,"columns":80}),
        json!({"action":"signal","session_id":"terminal-a","signal":"interrupt"}),
        json!({"action":"close","session_id":"terminal-a","close_policy":"force"}),
    ] {
        let request = parse(&value);
        request.validate().unwrap();
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<TerminalActionRequest>(encoded).unwrap(),
            request
        );
    }
}

#[test]
fn pinned_top_level_contract_and_no_model_authority() {
    for value in [
        json!(null),
        json!([]),
        json!({}),
        json!({"action":null}),
        json!({"action":"unknown"}),
        json!({"action":"exec"}),
        json!({"action":"start","session_id":"s"}),
        json!({"action":"start","profile":"clean","shell":{}}),
        json!({"action":"start","unknown":null}),
        json!({"action":"list","session_id":"s"}),
        json!({"action":"list","lifecycle":"running"}),
        json!({"action":"resize","session_id":"s","rows":null,"columns":80}),
        json!({"action":"read","session_id":1,"cursor_segment":1}),
        json!({"action":"read","session_id":"s","cursor":{"segment":1,"offset":0}}),
        json!({"action":"start","authority":null}),
        json!({"action":"start","owner":"s"}),
        json!({"action":"write","session_id":"s","lease":"acquire","writer":"a"}),
        json!({"action":"exec","command":"true","timeout_ms":1000}),
    ] {
        assert!(
            decode_terminal_action(&value, "/trusted").is_err(),
            "{value}"
        );
    }
    assert_eq!(
        parse(&json!({"action":"start"})),
        parse(
            &json!({"action":"start","session_id":null,"signal":null,"profile":null,"shell":null})
        )
    );
}

#[test]
fn cwd_is_retained_for_separate_preparation_and_never_used_as_authority() {
    for cwd in [".", "child", "../sibling", "/outside/requested"] {
        let value = json!({"action":"start","cwd":cwd});
        assert_eq!(
            terminal_action_requested_cwd(&value).unwrap().as_deref(),
            Some(cwd)
        );
        let TerminalActionRequest::Start { request } = parse(&value) else {
            panic!()
        };
        assert_eq!(request.cwd, "/trusted/workspace");
    }
    assert_eq!(
        terminal_action_requested_cwd(&json!({"action":"start","cwd":null})).unwrap(),
        None
    );
    for cwd in [
        json!(0),
        json!(""),
        json!("secret\0path"),
        json!("x".repeat(4097)),
    ] {
        let value = json!({"action":"start","cwd":cwd});
        assert!(terminal_action_requested_cwd(&value).is_err());
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
    for cwd in ["", "relative", "bad\0cwd"] {
        assert!(decode_terminal_action(&json!({"action":"start"}), cwd).is_err());
    }
    assert!(decode_terminal_action(&json!({"action":"list"}), "unused").is_ok());
}

#[test]
fn exec_preserves_optional_profiles_for_pinned_shell_resolution() {
    let omitted = parse(&json!({"action":"exec","command":"true"}));
    assert_eq!(
        omitted,
        parse(&json!({"action":"exec","command":"true","profile":null}))
    );
    for (profile, expected) in [
        (json!("user"), TerminalProfile::User),
        (json!(1), TerminalProfile::User),
        (json!("1"), TerminalProfile::User),
        (json!("clean"), TerminalProfile::Clean),
        (json!(0), TerminalProfile::Clean),
        (json!("0"), TerminalProfile::Clean),
    ] {
        let value = json!({"action":"exec","command":"true","cwd":"relative","profile":profile});
        assert_eq!(
            terminal_action_requested_cwd(&value).unwrap().as_deref(),
            Some("relative")
        );
        let TerminalActionRequest::Exec { request } = parse(&value) else {
            panic!()
        };
        assert_eq!(request.profile, Some(expected));
        assert_eq!(request.cwd, "/trusted/workspace");
    }
    for profile in [json!(2), json!("unknown"), json!(true)] {
        let value = json!({"action":"exec","command":"true","profile":profile});
        assert!(terminal_action_requested_cwd(&value).is_err());
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
}

#[test]
fn command_profile_shell_and_wait_defaults() {
    let TerminalActionRequest::Exec { request } = parse(&json!({"action":"exec","command":"true"}))
    else {
        panic!()
    };
    assert_eq!(request.profile, None);
    for value in [
        json!({"action":"exec","command":""}),
        json!({"action":"exec","command":"x".repeat(65_537)}),
        json!({"action":"exec","command":"true","profile":"unknown"}),
    ] {
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
    let TerminalActionRequest::Start { request } = parse(
        &json!({"action":"start","command":"true","profile":"user","backend":"tmux","dimensions":{"rows":25,"columns":90}}),
    ) else {
        panic!()
    };
    assert_eq!(request.return_when, Some(TerminalReturnCondition::Started));
    assert_eq!(request.profile, Some(TerminalProfile::User));
    assert_eq!(
        parse(&json!({"action":"start","command":""})),
        parse(&json!({"action":"start"}))
    );
    for shell in [
        json!({}),
        json!({"kind":"user_login","path":"ignored","clean_start":true}),
        json!({"kind":"executable","path":"/bin/bash","clean_start":true}),
    ] {
        parse(&json!({"action":"start","shell":shell}));
    }
    for condition in [
        json!({"kind":"started"}),
        json!({"kind":"exit"}),
        json!({"kind":"quiet","duration_ms":2}),
        json!({"kind":"match","pattern":"ready"}),
    ] {
        parse(
            &json!({"action":"wait","session_id":"s","return_when":condition,"wait_ceiling_ms":1000}),
        );
        parse(&json!({"action":"start","return_when":condition,"wait_ceiling_ms":1000}));
    }
    for value in [
        json!({"action":"start","return_when":{"kind":"exit"}}),
        json!({"action":"start","wait_ceiling_ms":0}),
        json!({"action":"wait","session_id":"s","return_when":{"kind":"quiet","duration_ms":0},"wait_ceiling_ms":1}),
    ] {
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
}

#[test]
fn all_write_forms_leases_and_boundaries() {
    for write in [
        json!({"kind":"text","text":"hello\0world"}),
        json!({"kind":"paste","text":"😀"}),
        json!({"kind":"keys","keys":["arrow_up","enter"]}),
        json!({"kind":"controls","controls":"cD?"}),
        json!({"kind":"controls","controls":[99,68,63]}),
    ] {
        parse(&json!({"action":"write","session_id":"s","write":write}));
    }
    let TerminalActionRequest::Write { request, .. } = parse(
        &json!({"action":"write","session_id":"s","write":{"kind":"controls","controls":"cD?"}}),
    ) else {
        panic!()
    };
    assert_eq!(
        request.payload,
        Some(TerminalWritePayload::Controls {
            controls: vec![99, 68, 63]
        })
    );
    for lease in ["acquire", "release", "revoke"] {
        parse(&json!({"action":"write","session_id":"s","lease":lease}));
        assert!(decode_terminal_action(&json!({"action":"write","session_id":"s","lease":lease,"write":{"kind":"text","text":"x"}}), "/trusted").is_err());
    }
    for write in [
        json!({"kind":"text","text":""}),
        json!({"kind":"keys","keys":[]}),
        json!({"kind":"keys","keys":["f1"]}),
        json!({"kind":"controls","controls":[256]}),
        json!({"kind":"controls","controls":"é"}),
        json!({"kind":"keys","keys":null}),
    ] {
        assert!(
            decode_terminal_action(
                &json!({"action":"write","session_id":"s","write":write}),
                "/trusted"
            )
            .is_err()
        );
    }
    assert!(
        decode_terminal_action(&json!({"action":"write","session_id":"s"}), "/trusted").is_err()
    );
}

fn definition(condition: Value) -> Value {
    let mut value = json!({"check_interval_ms":1000,"notify":{"kind":"on_match"},"lifetime":{"kind":"until_match"}});
    value["condition"] = condition;
    value
}

#[test]
fn all_thirteen_monitor_conditions_and_five_operations() {
    let conditions = [
        json!({"kind":"process_exit"}),
        json!({"kind":"exit_code","exit_code":0}),
        json!({"kind":"signal","signal":"terminate"}),
        json!({"kind":"output_contains","pattern":"ready"}),
        json!({"kind":"output_matches","pattern":"r.*y"}),
        json!({"kind":"output_quiet","duration_ms":100}),
        json!({"kind":"screen_matches","pattern":"ready"}),
        json!({"kind":"tcp_ready","host":"localhost","port":80}),
        json!({"kind":"http_ready","pattern":"https://example.test/"}),
        json!({"kind":"path_exists","path":"/requested/path"}),
        json!({"kind":"path_changed","path":"relative"}),
        json!({"kind":"path_size","path":"file","minimum_bytes":0}),
        json!({"kind":"custom_probe","command":"true","cwd":"/requested"}),
    ];
    for condition in conditions {
        let definition = definition(condition);
        parse(&json!({"action":"start","initial_monitors":[definition]}));
        for kind in ["add", "update"] {
            parse(
                &json!({"action":"monitor","session_id":"s","monitor":{"kind":kind,"monitor_id":"m","definition":definition}}),
            );
        }
    }
    for kind in ["pause", "resume", "remove"] {
        parse(
            &json!({"action":"monitor","session_id":"s","monitor":{"kind":kind,"monitor_id":"m"}}),
        );
    }
}

#[test]
fn monitor_notification_and_lifetime_vocabulary() {
    for notify in [
        json!({"kind":"on_match"}),
        json!({"kind":"on_state_change"}),
        json!({"kind":"on_exit"}),
        json!({"kind":"every_check"}),
        json!({"kind":"every_n_checks","count":2}),
        json!({"kind":"interval","interval_ms":100}),
    ] {
        for lifetime in [
            json!({"kind":"until_match"}),
            json!({"kind":"until_session_end"}),
            json!({"kind":"duration","duration_ms":10}),
        ] {
            let mut definition = definition(json!({"kind":"process_exit"}));
            definition["notify"] = notify.clone();
            definition["lifetime"] = lifetime;
            parse(&json!({"action":"start","initial_monitors":[definition]}));
        }
    }
}

#[test]
fn composite_strings_duplicates_and_initial_schedule_promotion() {
    for (field, value) in [
        ("shell", json!({})),
        ("dimensions", json!({"rows":24,"columns":80})),
        ("return_when", json!({"kind":"started"})),
        (
            "initial_monitors",
            json!([definition(json!({"kind":"process_exit"}))]),
        ),
    ] {
        let mut direct = json!({"action":"start"});
        direct[field] = value.clone();
        let mut encoded = direct.clone();
        encoded[field] = Value::String(value.to_string());
        assert_eq!(parse(&direct), parse(&encoded));
    }
    for value in [
        "null",
        "42",
        "\"nested string\"",
        "{",
        "{\"kind\":\"started\",\"kind\":\"exit\"}",
        "{\"kind\":\"started\",\"extra\":{\"a\":1,\"a\":2}}",
    ] {
        assert!(
            decode_terminal_action(&json!({"action":"start","return_when":value}), "/trusted")
                .is_err()
        );
    }
    let promoted = json!({"condition":{"kind":"path_exists","path":"file","check_interval_ms":1000},"notify":{"kind":"on_match"},"lifetime":{"kind":"until_match"}});
    assert_eq!(
        parse(&json!({"action":"start","initial_monitors":[promoted]})),
        parse(
            &json!({"action":"start","initial_monitors":[definition(json!({"kind":"path_exists","path":"file"}))]})
        )
    );
    assert!(decode_terminal_action(&json!({"action":"monitor","session_id":"s","monitor":{"kind":"add","definition":promoted}}), "/trusted").is_err());
}

#[test]
fn bounds_and_redacted_failures() {
    for value in [
        json!({"action":"read","session_id":"s","cursor_segment":0}),
        json!({"action":"screen","session_id":"../escape"}),
        json!({"action":"screen","session_id":"x".repeat(256)}),
        json!({"action":"inspect","session_id":"s","max_events":0}),
        json!({"action":"resize","session_id":"s","rows":4096,"columns":4096}),
        json!({"action":"resize","session_id":"s","rows":65536,"columns":1}),
        json!({"action":"signal","session_id":"s","signal":9}),
        json!({"action":"close","session_id":"s","close_policy":"immediate"}),
        json!({"action":"start","initial_monitors":vec![definition(json!({"kind":"process_exit"}));33]}),
        json!({"action":"start","command":"private secret".repeat(10000)}),
        json!({"action":"start","initial_monitors":[definition(json!({"kind":"path_exists","path":""}))]}),
    ] {
        let error = decode_terminal_action(&value, "/trusted").unwrap_err();
        assert_eq!(error.to_string(), "invalid terminal action arguments");
        assert_eq!(format!("{error:?}"), "TerminalActionParseError");
    }
    let mut deep = Value::Null;
    for _ in 0..70 {
        deep = json!([deep]);
    }
    assert!(decode_terminal_action(&json!({"action":"start","shell":deep}), "/trusted").is_err());
}

#[test]
fn cwd_extraction_validates_the_entire_request_before_resolution() {
    for value in [
        json!({"action":"read","session_id":"s","cursor_segment":1,"cwd":"outside"}),
        json!({"action":"close","session_id":"s","close_policy":"force","cwd":"outside"}),
        json!({"action":"start","cwd":"outside","shell":{"kind":"executable"}}),
        json!({"action":"start","cwd":"outside","initial_monitors":[definition(json!({"kind":"tcp_ready","host":"host","port":0}))]}),
    ] {
        assert!(terminal_action_requested_cwd(&value).is_err());
    }
    let value = json!({"action":"start","cwd":[46,47,99,104,105,108,100]});
    assert_eq!(
        terminal_action_requested_cwd(&value).unwrap().as_deref(),
        Some("./child")
    );
}

#[test]
fn zig_integer_coercions_preserve_exact_ids_and_bounds() {
    for segment in [
        json!(1),
        json!(1.0),
        json!("1"),
        json!("1.0"),
        json!("10e-1"),
        json!("+01"),
        json!("0_1"),
    ] {
        assert_eq!(
            parse(&json!({"action":"read","session_id":"s","cursor_segment":segment})),
            parse(&json!({"action":"read","session_id":"s","cursor_segment":1}))
        );
    }
    for value in [
        json!(u64::MAX),
        json!(u64::MAX.to_string()),
        json!("18446744073709551615.0"),
    ] {
        parse(&json!({"action":"read","session_id":"s","cursor_segment":value}));
    }
    parse(&json!({"action":"read","session_id":"s","cursor_segment":9_007_199_254_740_992.0}));
    for value in [
        json!("18446744073709551616"),
        json!("1.01"),
        json!(0.5),
        json!(-1),
        json!("1e100"),
        json!("1e-100"),
        json!("_1"),
        json!("1_"),
        json!("nan"),
    ] {
        assert!(
            decode_terminal_action(
                &json!({"action":"read","session_id":"s","cursor_segment":value}),
                "/trusted"
            )
            .is_err()
        );
    }
    parse(&json!({"action":"resize","session_id":"s","rows":"2.4e1","columns":80.0}));
    parse(
        &json!({"action":"start","return_when":"{\"kind\":\"quiet\",\"duration_ms\":\"1000.0\"}","wait_ceiling_ms":"1e3"}),
    );
}

#[test]
fn zig_enum_ordinals_and_utf8_byte_arrays() {
    assert_eq!(
        parse(&json!({"action":"start","profile":0,"backend":"1"})),
        parse(&json!({"action":"start","profile":"clean","backend":"tmux"}))
    );
    assert_eq!(
        parse(&json!({"action":"signal","session_id":[115],"signal":1})),
        parse(&json!({"action":"signal","session_id":"s","signal":"interrupt"}))
    );
    assert_eq!(
        parse(
            &json!({"action":"write","session_id":"s","lease":"1","write":{"kind":2,"controls":["99",68.0,"6.3e1"]}})
        ),
        parse(
            &json!({"action":"write","session_id":"s","write":{"kind":"controls","controls":"cD?"}})
        )
    );
    parse(&json!({"action":"exec","command":[116,114,117,101]}));
    parse(&json!({"action":"start","shell":{"kind":1,"path":[47,98,105,110,47,98,97,115,104]}}));
    parse(&json!({"action":"write","session_id":"s","write":{"kind":1,"keys":[0,"6"]}}));
    for value in [
        json!({"action":1}),
        json!({"action":"1"}),
        json!({"action":"start","profile":2}),
        json!({"action":"signal","session_id":"s","signal":1.0}),
        json!({"action":"exec","command":[255]}),
        json!({"action":"start","cwd":[256]}),
    ] {
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
}

#[test]
fn normalized_read_inspect_list_and_lifecycle_fields_are_explicit() {
    let TerminalActionRequest::Read { session_id, cursor } =
        parse(&json!({"action":"read","session_id":"s","cursor_segment":2}))
    else {
        panic!()
    };
    assert_eq!(session_id.as_str(), "s");
    assert_eq!(cursor.segment(), 2);
    assert_eq!(cursor.offset(), 0);
    let TerminalActionRequest::Inspect { events, .. } = parse(
        &json!({"action":"inspect","session_id":"s","after_event_id":5,"acknowledge_event_id":7}),
    ) else {
        panic!()
    };
    assert_eq!(events.after_event_id, 5);
    assert_eq!(events.acknowledge_event_id, Some(7));
    assert_eq!(events.max_events, 64);
    assert_eq!(
        parse(&json!({"action":"list","task_id":"","workspace_root":""})),
        parse(&json!({"action":"list"}))
    );
    let TerminalActionRequest::List { filters } = parse(
        &json!({"action":"list","task_id":"task","workspace_root":"/workspace","backend":"native"}),
    ) else {
        panic!()
    };
    assert_eq!(filters.task_id.as_deref(), Some("task"));
    assert_eq!(filters.workspace_root.as_deref(), Some("/workspace"));
    assert_eq!(filters.lifecycle, None);
    for signal in ["hangup", "interrupt", "quit", "terminate", "kill"] {
        parse(&json!({"action":"signal","session_id":"s","signal":signal}));
    }
    for policy in ["graceful", "force"] {
        parse(&json!({"action":"close","session_id":"s","close_policy":policy}));
    }
}

#[test]
fn remaining_composites_and_known_inactive_nested_fields() {
    for (field, value, action) in [
        ("write", json!({"kind":"text","text":"x"}), "write"),
        (
            "monitor",
            json!({"kind":"remove","monitor_id":"m"}),
            "monitor",
        ),
    ] {
        let mut direct = json!({"action":action,"session_id":"s"});
        direct[field] = value.clone();
        let mut encoded = direct.clone();
        encoded[field] = Value::String(value.to_string());
        assert_eq!(parse(&direct), parse(&encoded));
    }
    parse(&json!({"action":"start","return_when":{"kind":"started","duration_ms":0,"pattern":""}}));
    parse(
        &json!({"action":"monitor","session_id":"s","monitor":{"kind":"add","monitor_id":"","definition":definition(json!({"kind":"process_exit","exit_code":-1}))}}),
    );
    for value in [
        json!({"action":"start","return_when":{"kind":"started","duration_ms":false}}),
        json!({"action":"start","shell":{"kind":"user_login","clean_start":null}}),
        json!({"action":"start","shell":{"kind":"user_login","unknown":null}}),
    ] {
        assert!(decode_terminal_action(&value, "/trusted").is_err());
    }
    let mut promoted =
        definition(json!({"kind":"path_exists","path":"file","check_interval_ms":500}));
    promoted["check_interval_ms"] = Value::Null;
    assert!(
        decode_terminal_action(
            &json!({"action":"start","initial_monitors":[promoted]}),
            "/trusted"
        )
        .is_err()
    );
}

#[test]
fn binary128_integer_coercion_matches_pinned_zig_oracle() {
    // Checked with Zig 0.16.0: std.json.Value -> Stringify -> typed u64,
    // the same path as terminal.zig at b1774fbf6c7602b503026f96f6e960e946c692ef.
    // The long values are 1 + 2^-113 (binary128 halfway), its neighbors,
    // and 1 - 2^-114 (the lower halfway). Ties round to the even integer.
    let cases = [
        ("1.0000000000000000000000000000000000000001", Some(1)),
        (
            "1.00000000000000000000000000000000009629649721936179265279889712924636592690508241076940976199693977832794189453125",
            Some(1),
        ),
        (
            "1.000000000000000000000000000000000096296497219361792652798897129246365926905082410769409761996939778327941894531249999999",
            Some(1),
        ),
        (
            "1.000000000000000000000000000000000096296497219361792652798897129246365926905082410769409761996939778327941894531250000001",
            None,
        ),
        (
            "0.999999999999999999999999999999999951851751390319103673600551435376817036547458794615295119001530110836029052734375",
            Some(1),
        ),
        ("1e-5000", Some(0)),
        ("1e-4966", Some(0)),
        ("1e-4965", None),
        ("1e5000", None),
        ("-0", Some(0)),
        ("-0.0", Some(0)),
        ("-1e-5000", Some(0)),
        (
            "18446744073709551615.0000000000000000000000000000001",
            Some(u64::MAX),
        ),
        ("18446744073709551615.9999999999999999999999999999999", None),
        ("1.1", None),
        ("1e", None),
        ("1e+", None),
        ("1e-", None),
        ("1__0", Some(10)),
        ("1.0__0", None),
        ("1.0_0e0_0", Some(1)),
        ("0x1.0", Some(1)),
        ("0x1e0", Some(480)),
        ("0x1p0", None),
        ("0x1.0p0", Some(1)),
        ("0x1.0p_0", None),
        ("0x1._0", None),
        ("0e999999999999999999999999999999999999999999", Some(0)),
    ];
    for (spelling, expected) in cases {
        let actual = decode_terminal_action(
            &json!({"action":"read","session_id":"s","cursor_segment":1,"cursor_offset":spelling}),
            "/trusted",
        );
        match (actual, expected) {
            (Ok(TerminalActionRequest::Read { cursor, .. }), Some(expected)) => {
                assert_eq!(cursor.offset(), expected, "{spelling}");
            }
            (Err(_), None) => {}
            _ => panic!("binary128 differential mismatch: {spelling}"),
        }
    }
}

#[test]
fn binary128_conversion_preserves_field_bounds_and_nested_composites() {
    parse(
        &json!({"action":"resize","session_id":"s","rows":"24.00000000000000000000000000000000000001","columns":80}),
    );
    parse(
        &json!({"action":"start","return_when":"{\"kind\":\"quiet\",\"duration_ms\":\"1.0000000000000000000000000000000000000001\"}","wait_ceiling_ms":1}),
    );
    for spelling in [
        "-1.0",
        "65536.0",
        "65535.999999999999999999999999999999999999999",
        "1e5000",
    ] {
        assert!(
            decode_terminal_action(
                &json!({"action":"resize","session_id":"s","rows":spelling,"columns":80}),
                "/trusted"
            )
            .is_err()
        );
    }
    // Parse rounding may produce zero, but positive session/wait bounds remain.
    assert!(decode_terminal_action(&json!({"action":"wait","session_id":"s","return_when":{"kind":"exit"},"wait_ceiling_ms":"1e-5000"}), "/trusted").is_err());
    assert!(decode_terminal_action(&json!({"action":"signal","session_id":"s","signal":"1.0000000000000000000000000000000000000001"}), "/trusted").is_err());
}

#[test]
fn binary128_maximum_argument_stress_is_bounded_and_released() {
    let spelling = format!("1.{}1", "0".repeat(60_000));
    let arguments =
        json!({"action":"read","session_id":"s","cursor_segment":1,"cursor_offset":spelling});
    let allocations = allocation_counter::measure(|| {
        let TerminalActionRequest::Read { cursor, .. } = parse(&arguments) else {
            panic!()
        };
        assert_eq!(cursor.offset(), 1);
    });
    assert_eq!(allocations.bytes_current, 0);
    assert!(allocations.bytes_max < 8 * 1024 * 1024, "{allocations:?}");
    let oversized = json!({"action":"read","session_id":"s","cursor_segment":1,"cursor_offset":format!("1.{}1", "0".repeat(65_536))});
    let rejected = allocation_counter::measure(|| {
        assert!(decode_terminal_action(&oversized, "/trusted").is_err());
    });
    assert_eq!(rejected.bytes_current, 0);
    assert!(rejected.bytes_total < 256 * 1024, "{rejected:?}");
}

#[test]
fn full_command_bounds_allow_exact_64k_even_when_json_escaped() {
    use machine_god_core::MAX_TERMINAL_ACTION_COMMAND_BYTES;
    let command = "\u{1b}".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES);
    for action in ["exec", "start"] {
        let input = json!({"action":action,"command":command});
        let request = parse(&input);
        let actual = match request {
            TerminalActionRequest::Exec { request } => request.command,
            TerminalActionRequest::Start { request } => request.command.unwrap(),
            _ => panic!(),
        };
        assert_eq!(actual, command);
        assert!(
            decode_terminal_action(
                &json!({"action":action,"command":format!("{command}x")}),
                "/trusted"
            )
            .is_err()
        );
    }
}

#[test]
fn complete_envelope_fits_32_maximum_probes_and_control_arrays() {
    use machine_god_core::{
        MAX_TERMINAL_ACTION_COMMAND_BYTES, MAX_TERMINAL_ACTION_TEXT_BYTES,
        MAX_TERMINAL_INITIAL_MONITORS,
    };
    let command = "\u{1b}".repeat(MAX_TERMINAL_ACTION_COMMAND_BYTES);
    let cwd = format!("/{}", "x".repeat(MAX_TERMINAL_ACTION_TEXT_BYTES - 1));
    let monitor = definition(json!({"kind":"custom_probe","command":command,"cwd":cwd}));
    let monitors = vec![monitor; MAX_TERMINAL_INITIAL_MONITORS];
    let input = json!({"action":"start","command":command,"cwd":cwd,"shell":{"kind":"executable","path":cwd},"initial_monitors":serde_json::to_string(&monitors).unwrap()});
    let TerminalActionRequest::Start { request } = parse(&input) else {
        panic!()
    };
    assert_eq!(
        request.initial_monitors.len(),
        MAX_TERMINAL_INITIAL_MONITORS
    );
    assert_eq!(
        request.command.unwrap().len(),
        MAX_TERMINAL_ACTION_COMMAND_BYTES
    );
    // The byte-array input form must not be constrained by the old JSON cap.
    let command_bytes = vec![27; MAX_TERMINAL_ACTION_COMMAND_BYTES];
    parse(&json!({"action":"exec","command":command_bytes}));
    let controls = vec!["6.7e1"; machine_god_core::MAX_TERMINAL_WRITE_ITEMS];
    parse(
        &json!({"action":"write","session_id":"s","write":{"kind":"controls","controls":controls}}),
    );
    parse(&json!({"action":"write","session_id":"s","write":{"kind":"text","text":vec![67;8192]}}));
}

#[test]
fn serialized_envelope_rejects_before_allocating_decoded_fields() {
    let oversized = json!({"action":"exec","command":"\u{1b}".repeat(machine_god_native::MAX_TERMINAL_ACTION_ARGUMENT_BYTES / 6 + 1)});
    let allocations = allocation_counter::measure(|| {
        assert!(decode_terminal_action(&oversized, "/trusted").is_err());
    });
    assert_eq!(allocations.bytes_current, 0);
    assert!(allocations.bytes_total < 4096, "{allocations:?}");
}

#[test]
fn nested_composite_is_rejected_before_cwd_preparation_and_normalization() {
    let nested = format!("{}null{}", "[".repeat(100), "]".repeat(100));
    let input = json!({"action":"start","cwd":"/must-not-resolve","shell":nested});
    let allocations = allocation_counter::measure(|| {
        assert!(terminal_action_requested_cwd(&input).is_err());
    });
    assert_eq!(allocations.bytes_current, 0);
    assert!(allocations.bytes_total < 32 * 1024, "{allocations:?}");
}

#[test]
fn schema_node_budget_fits_all_32_full_probe_byte_arrays() {
    use machine_god_core::{
        MAX_TERMINAL_ACTION_COMMAND_BYTES, MAX_TERMINAL_ACTION_TEXT_BYTES,
        MAX_TERMINAL_INITIAL_MONITORS,
    };
    let command = vec![27; MAX_TERMINAL_ACTION_COMMAND_BYTES];
    let cwd = vec![47; MAX_TERMINAL_ACTION_TEXT_BYTES];
    let monitor = definition(json!({"kind":"custom_probe","command":command,"cwd":cwd}));
    let input = json!({"action":"start","command":command,"cwd":cwd,"initial_monitors":vec![monitor;MAX_TERMINAL_INITIAL_MONITORS]});
    let TerminalActionRequest::Start { request } = parse(&input) else {
        panic!()
    };
    assert_eq!(
        request.initial_monitors.len(),
        MAX_TERMINAL_INITIAL_MONITORS
    );
    assert_eq!(
        request.command.unwrap().len(),
        MAX_TERMINAL_ACTION_COMMAND_BYTES
    );
}
