#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeSet;

use floatile_sdk::{ENGINE_API_VERSION, WidgetError};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Suite {
    schema_version: u64,
    engine_api_version: String,
    vectors: Vec<Vector>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vector {
    id: String,
    callback: String,
    guest_error: String,
    message: Option<String>,
    expected_host_outcome: String,
}

fn suite() -> Suite {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/sdk-lifecycle-v1.json"
    )))
    .expect("SDK lifecycle conformance vectors must be valid JSON")
}

fn widget_error(vector: &Vector) -> WidgetError {
    match vector.guest_error.as_str() {
        "invalid-input" => WidgetError::InvalidInput(
            vector
                .message
                .clone()
                .expect("invalid-input vector requires a message"),
        ),
        "rejected" => WidgetError::Rejected(
            vector
                .message
                .clone()
                .expect("rejected vector requires a message"),
        ),
        "internal" => WidgetError::Internal,
        other => panic!("unknown guest error in conformance vector: {other}"),
    }
}

#[test]
fn lifecycle_vectors_match_the_generated_wit_contract() {
    let suite = suite();
    assert_eq!(suite.schema_version, 1);
    assert_eq!(suite.engine_api_version, ENGINE_API_VERSION);

    let mut ids = BTreeSet::new();
    for vector in &suite.vectors {
        assert!(ids.insert(&vector.id), "duplicate vector id: {}", vector.id);
        assert!(matches!(vector.callback.as_str(), "start" | "event"));
        assert_eq!(vector.expected_host_outcome, "rejected");
        let _ = widget_error(vector);
    }

    assert_eq!(
        suite
            .vectors
            .iter()
            .map(|vector| vector.guest_error.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["internal", "invalid-input", "rejected"]),
        "vectors must cover every WIT widget-error variant"
    );
}
