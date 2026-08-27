//! PP-M5 reference package evidence: generic HTTPS capability only and no embedded secret.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

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
    let _ = std::fs::remove_file(output);
}
