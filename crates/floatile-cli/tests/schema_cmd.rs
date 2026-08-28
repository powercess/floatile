#![allow(clippy::unwrap_used)]

use std::process::Command;

#[test]
fn ui_schema_command_emits_the_single_source_registry() {
    let output =
        std::env::temp_dir().join(format!("floatile-ui-registry-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&output);
    let result = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["schema", "ui", output.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(result.status.success(), "stderr: {:?}", result.stderr);
    let contract: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(contract["schemaVersion"], 1);
    assert_eq!(contract["uiApiVersion"], floatile_ui_schema::UI_API_VERSION);
    assert!(
        contract["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["name"] == "Text")
    );
    let _ = std::fs::remove_file(output);
}

#[test]
fn manifest_schema_legacy_argument_remains_supported() {
    let output = std::env::temp_dir().join(format!(
        "floatile-manifest-schema-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    let result = Command::new(env!("CARGO_BIN_EXE_floatile"))
        .args(["schema", output.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(result.status.success(), "stderr: {:?}", result.stderr);
    let schema: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
    let _ = std::fs::remove_file(output);
}
