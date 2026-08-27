//! PP-M5 reference package evidence: generic HTTPS capability only and no embedded secret.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::{collections::BTreeSet, io::Read};

use floatile_ui_schema::JsonSchema;
use floatile_ui_schema::ir::{Component, PropValue, UiDocument};

#[test]
fn ai_balance_reference_builds_without_secret_material() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let project = root.join("plugins/ai-balance-wasm");
    let output = std::env::temp_dir().join(format!(
        "floatile-ai-balance-e2e-{}.floatile",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&output);
    let manifest = floatile_cli::build::build_project(&project, &output)
        .unwrap_or_else(|error| panic!("AI balance reference build failed: {error}"));
    assert!(
        manifest
            .permissions
            .iter()
            .any(|permission| permission.capability == "network:https")
    );
    assert_eq!(manifest.http_templates.len(), 1);

    let package = std::fs::read(&output).unwrap();
    for forbidden in [b"api_key".as_slice(), b"api-key", b"Bearer ", b"secret"] {
        assert!(
            !package
                .windows(forbidden.len())
                .any(|window| window == forbidden),
            "reference package contains forbidden credential marker"
        );
    }
    assert_pp_m6_ui_contract(&package);
    let _ = std::fs::remove_file(output);
}

fn assert_pp_m6_ui_contract(package: &[u8]) {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(package)).unwrap();
    let names = (0..archive.len())
        .map(|index| archive.by_index(index).unwrap().name().to_owned())
        .collect::<Vec<_>>();
    assert!(
        names
            .iter()
            .all(|name| !name.ends_with(".slint") && !name.ends_with(".html")),
        "PP-M6 reference package must not ship third-party Slint or HTML"
    );

    let mut ftui = String::new();
    archive
        .by_name("ui/widget.ftui")
        .unwrap()
        .read_to_string(&mut ftui)
        .unwrap();
    let document: UiDocument = serde_json::from_str(&ftui).unwrap();
    floatile_ui_schema::validate_document(&document).unwrap();
    assert_eq!(document.ui_api_version, floatile_ui_schema::UI_API_VERSION);

    let mut kinds = BTreeSet::new();
    let mut labeled_metrics = 0;
    let mut uses_theme_token = false;
    collect_ui_evidence(
        &document.root,
        &mut kinds,
        &mut labeled_metrics,
        &mut uses_theme_token,
    );
    for required in [
        "If",
        "Column",
        "Responsive",
        "List",
        "Badge",
        "Progress",
        "Sparkline",
        "Text",
    ] {
        assert!(
            kinds.contains(required),
            "missing PP-M6 component {required}"
        );
    }
    assert!(labeled_metrics >= 2, "metric/chart labels must be explicit");
    assert!(
        uses_theme_token,
        "reference must consume a host theme token"
    );

    let JsonSchema::Object { properties, .. } = &document.state.schema else {
        panic!("AI Balance state schema must be an object");
    };
    assert_bounded_array(properties.get("entries"), 256);
    assert_bounded_array(properties.get("trend"), 128);
}

fn collect_ui_evidence(
    component: &Component,
    kinds: &mut BTreeSet<String>,
    labeled_metrics: &mut usize,
    uses_theme_token: &mut bool,
) {
    kinds.insert(component.kind.clone());
    if matches!(component.kind.as_str(), "Progress" | "Gauge")
        && component.props.contains_key("accessibilityLabel")
    {
        *labeled_metrics += 1;
    }
    if component.kind == "Sparkline" && component.props.contains_key("label") {
        *labeled_metrics += 1;
    }
    *uses_theme_token |= matches!(
        component.props.get("colorToken"),
        Some(PropValue::Literal(value)) if value.as_str().is_some()
    );
    for child in &component.children {
        collect_ui_evidence(child, kinds, labeled_metrics, uses_theme_token);
    }
    for branch in [component.then.as_deref(), component.else_.as_deref()]
        .into_iter()
        .flatten()
    {
        collect_ui_evidence(branch, kinds, labeled_metrics, uses_theme_token);
    }
    if let Some(template) = component.template.as_deref() {
        collect_ui_evidence(template, kinds, labeled_metrics, uses_theme_token);
    }
}

fn assert_bounded_array(schema: Option<&JsonSchema>, limit: usize) {
    let Some(JsonSchema::Array {
        max_items: Some(max_items),
        ..
    }) = schema
    else {
        panic!("PP-M6 collection state must declare maxItems");
    };
    assert!(*max_items <= limit, "array budget exceeds {limit}");
}
