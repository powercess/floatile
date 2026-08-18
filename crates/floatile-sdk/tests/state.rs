//! `#[derive(State)]` 生成 schema/initial 的主机侧契约测试。
#![allow(clippy::unwrap_used)]

use floatile_sdk::{JsonSchema, State};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
struct TestState {
    time: String,
    count: u32,
    ratio: f64,
    running: bool,
    zones: Vec<String>,
    note: Option<String>,
}

#[test]
fn derived_schema_is_object_with_fields() {
    let schema = TestState::schema();
    match schema {
        JsonSchema::Object {
            required,
            properties,
            additional_properties,
        } => {
            assert!(!additional_properties, "State schema 必须拒绝未知字段");
            for name in ["time", "count", "ratio", "running", "zones"] {
                assert!(properties.contains_key(name), "缺少字段 {name}");
            }
            // note 是 Option → 不进 required。
            assert_eq!(required.len(), 5, "Option 字段不应进入 required");
            assert!(properties.contains_key("note"));
        }
        other => panic!("期望 Object schema，实际 {other:?}"),
    }
}

#[test]
fn derived_initial_uses_type_defaults() {
    let s = TestState::initial();
    assert_eq!(s.time, "");
    assert_eq!(s.count, 0);
    assert_eq!(s.ratio, 0.0);
    assert!(!s.running);
    assert!(s.zones.is_empty());
    assert!(s.note.is_none());
}

#[test]
fn derived_state_roundtrips_serde() {
    let s = TestState {
        time: "12:00:00".into(),
        count: 3,
        ratio: 0.5,
        running: true,
        zones: vec!["UTC".into(), "PST".into()],
        note: Some("x".into()),
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: TestState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}
