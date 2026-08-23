//! TypeScript runtime spike 的真实 `.floatile` 包预算证据。
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use floatile_cli::{
    PackageLimits, generate_manifest, package, parse_floatile_toml, validate_package,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
#[ignore = "requires spikes/typescript-runtime pnpm build"]
fn component_fits_current_default_package_budget() {
    let root = workspace_root();
    let spike = root.join("spikes/typescript-runtime");
    let target = root.join("target/typescript-runtime-spike");
    let config = parse_floatile_toml(
        &std::fs::read_to_string(spike.join("floatile.toml")).expect("读取 floatile.toml"),
    )
    .expect("解析 floatile.toml");
    let mut manifest = generate_manifest(&config).expect("生成 manifest");
    let build = manifest.build.as_mut().expect("应包含 build metadata");
    build.sdk = "typescript-runtime-spike".into();
    build.sdk_version = "0.1.0".into();

    let ftui = std::fs::read_to_string(target.join("widget.ftui")).expect("读取 widget.ftui");
    let component = std::env::var_os("FLOATILE_TYPESCRIPT_CLOCK_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|| target.join("clock-typescript-starlingmonkey.wasm"));
    let wasm = std::fs::read(component).expect("读取 component");
    let out = std::env::temp_dir().join(format!(
        "floatile-typescript-runtime-spike-{}.floatile",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);

    package(&manifest, &ftui, &wasm, &out).expect("TypeScript clock 应能打包并自校验");
    let bytes = std::fs::read(&out).expect("读取 .floatile");
    let validated = validate_package(&bytes, &PackageLimits::default()).expect("包应通过默认预算");
    assert_eq!(
        validated.manifest.id.0,
        "dev.floatile.clock-typescript-spike"
    );
    assert!(bytes.len() <= PackageLimits::default().max_archive_bytes);
    println!(
        "{{\"componentBytes\":{},\"packageBytes\":{}}}",
        wasm.len(),
        bytes.len()
    );

    let _ = std::fs::remove_file(out);
}
