use super::*;
use crate::EngineLimits;
use serde_json::json;
use std::cell::Cell;
use std::num::NonZeroUsize;

fn limits(depth: usize, nodes: usize) -> EngineLimits {
    EngineLimits {
        max_json_depth: NonZeroUsize::new(depth).unwrap(),
        max_json_nodes: NonZeroUsize::new(nodes).unwrap(),
        ..EngineLimits::default()
    }
}

#[test]
fn container_depth_is_inclusive_and_scalars_do_not_add_depth() {
    for value in [Value::Null, json!([null]), json!({"key": true})] {
        assert_eq!(validate_json_roots([&value], limits(1, 2)), Ok(()));
    }
    for value in [json!([[]]), json!({"key": {}})] {
        assert_eq!(
            validate_json_roots([&value], limits(1, 3)),
            Err(JsonLimitViolation::Depth)
        );
        assert_eq!(validate_json_roots([&value], limits(2, 3)), Ok(()));
    }
}

#[test]
fn nodes_include_roots_and_values_but_not_object_keys() {
    let value = json!({"one": [null, true], "two": 42});
    assert_eq!(validate_json_roots([&value], limits(2, 5)), Ok(()));
    assert_eq!(
        validate_json_roots([&value], limits(2, 4)),
        Err(JsonLimitViolation::Nodes)
    );
    assert_eq!(validate_json_roots([], limits(1, 1)), Ok(()));
}

#[test]
fn budget_accumulates_across_roots_and_stops_requesting_roots_on_failure() {
    let value = json!([null]);
    let mut budget = JsonValidationBudget::new(limits(1, 4));
    budget.validate(&value).unwrap();
    budget.validate(&value).unwrap();
    assert_eq!(
        budget.validate(&Value::Null),
        Err(JsonLimitViolation::Nodes)
    );
    assert_eq!(budget.nodes, 5);

    let visited = Cell::new(0);
    let roots = [&Value::Null, &Value::Null, &Value::Null]
        .into_iter()
        .inspect(|_| {
            visited.set(visited.get() + 1);
        });
    assert_eq!(
        validate_json_roots(roots, limits(1, 1)),
        Err(JsonLimitViolation::Nodes)
    );
    assert_eq!(visited.get(), 2);
}

#[test]
fn node_failure_precedes_depth_failure_and_overflow_is_checked() {
    let value = json!([[]]);
    assert_eq!(
        validate_json_roots([&value], limits(1, 1)),
        Err(JsonLimitViolation::Nodes)
    );
    let mut budget = JsonValidationBudget {
        nodes: usize::MAX,
        max_nodes: usize::MAX,
        max_container_depth: 1,
    };
    assert_eq!(
        budget.validate(&Value::Null),
        Err(JsonLimitViolation::Nodes)
    );
    assert_eq!(budget.nodes, usize::MAX);
}

#[test]
fn serialized_size_matches_compact_utf8_escaping_and_inclusive_limits() {
    let value = json!({"text": "é\n\u{0000}", "values": [null, true, 123]});
    let expected = serde_json::to_vec(&value).unwrap().len();
    assert_eq!(
        serialized_json_size_bounded(&value, expected).unwrap(),
        Some(expected)
    );
    assert_eq!(
        serialized_json_size_bounded(&value, expected - 1).unwrap(),
        None
    );
    assert_eq!(serialized_json_size_bounded(&value, 0).unwrap(), None);

    let values = [Value::Null, Value::Bool(true)];
    let slice: &[Value] = &values;
    assert_eq!(serialized_json_size_bounded(slice, 11).unwrap(), Some(11));
}

#[test]
fn serialization_failure_is_distinct_from_a_byte_limit() {
    struct Fails;
    impl Serialize for Fails {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("synthetic serialization failure"))
        }
    }
    for limit in [0, usize::MAX] {
        let error = serialized_json_size_bounded(&Fails, limit).unwrap_err();
        assert!(!error.is_io());
        assert!(
            error
                .to_string()
                .contains("synthetic serialization failure")
        );
    }
}

#[test]
fn byte_counter_rejects_whole_write_and_preserves_overflow_classification() {
    let mut counter = JsonByteCounter {
        bytes: 0,
        limit: 3,
        exceeded: false,
    };
    assert_eq!(counter.write(b"ab").unwrap(), 2);
    assert!(counter.write(b"cd").is_err());
    assert_eq!(counter.bytes, 2);
    assert!(counter.exceeded);
    assert!(counter.flush().is_ok());

    let mut counter = JsonByteCounter {
        bytes: usize::MAX,
        limit: usize::MAX,
        exceeded: false,
    };
    assert!(counter.write(b"x").is_err());
    assert_eq!(counter.bytes, usize::MAX);
    assert!(!counter.exceeded);
}

#[test]
fn validation_and_destruction_of_deep_mixed_trees_do_not_recurse() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            let mut value = Value::Null;
            for depth in 0..20_000 {
                value = if depth % 2 == 0 {
                    Value::Array(vec![Value::Bool(false), value, Value::Null])
                } else {
                    let mut fields = serde_json::Map::new();
                    fields.insert("child".into(), value);
                    fields.insert("sibling".into(), Value::String("text".into()));
                    Value::Object(fields)
                };
            }
            let result = validate_json_roots([&value], limits(64, 65_536));
            // Always reclaim the tree before asserting, including a failing test.
            drop_json_value_iterative(value);
            assert_eq!(result, Err(JsonLimitViolation::Depth));
        })
        .unwrap()
        .join()
        .unwrap();
}
