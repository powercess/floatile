//! 作者项目：`floatile.toml` 解析、manifest 生成与模板（`new`）。
//!
//! `floatile.toml` 是作者维护的最小项目配置（manifest-v1 §3），CLI 结合 SDK 生成的
//! UI/State schema 与能力候选产生 `manifest.json`。作者不编辑 manifest 生成物。

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use floatile_core::constants::{ENGINE_API_VERSION, MANIFEST_VERSION};
use floatile_core::manifest::{
    BuildMeta, Entrypoints, Manifest, PackagePath, PermissionDecl, PluginKind, Publisher, Sizes,
    validate_manifest,
};
use floatile_core::types::LogicalSize;
use floatile_core::{CAPABILITY_REGISTRY, CapabilityDefinition, CapabilityParamKind};
use floatile_ui_schema::UI_API_VERSION;
use serde::Deserialize;

/// 作者项目配置（`floatile.toml` 的直接映射）。
#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub plugin: PluginCfg,
    #[serde(default)]
    pub widget: WidgetCfg,
    #[serde(default)]
    pub permissions: BTreeMap<String, PermissionCfg>,
    #[serde(default)]
    pub publisher: Option<PublisherCfg>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginCfg {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WidgetCfg {
    #[serde(default)]
    pub default_size: Option<[f32; 2]>,
    #[serde(default)]
    pub min_size: Option<[f32; 2]>,
    #[serde(default)]
    pub max_size: Option<[f32; 2]>,
    #[serde(default = "default_true")]
    pub resizable: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct PublisherCfg {
    pub id: String,
    pub name: String,
}

/// 能力参数（TOML 中的标量/数组 → JSON，交给 core 解析）。
#[derive(Debug, Clone, Deserialize)]
pub struct PermissionCfg {
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub max_bytes: Option<u64>,
    #[serde(default)]
    pub max_per_minute: Option<u32>,
    #[serde(default)]
    pub max_active: Option<u32>,
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
}

/// `floatile.toml` 的权限段短名 → 完整 capability 名（manifest-v1 §3）。
/// 未知段名返回 `None`，由调用方拒绝（不能静默忽略作者声明的能力）。
fn section_to_capabilities(section: &str) -> Option<Vec<&'static CapabilityDefinition>> {
    let capabilities: Vec<_> = CAPABILITY_REGISTRY
        .iter()
        .filter(|definition| definition.author_section == Some(section))
        .collect();
    (!capabilities.is_empty()).then_some(capabilities)
}

/// 解析 `floatile.toml`。
pub fn parse_floatile_toml(text: &str) -> Result<ProjectConfig, ProjectError> {
    toml::from_str(text).map_err(|e| ProjectError::InvalidToml(e.to_string()))
}

/// 从作者配置生成 `manifest.json` 的内容（JSON 字符串）。
pub fn generate_manifest(config: &ProjectConfig) -> Result<Manifest, ProjectError> {
    let default = [240.0f32, 120.0];
    let min = [160.0f32, 80.0];
    let max = [800.0f32, 600.0];
    let mut permissions = Vec::new();
    for (section, cfg) in &config.permissions {
        let capabilities = section_to_capabilities(section)
            .ok_or_else(|| ProjectError::UnknownCapability(section.clone()))?;
        for capability in capabilities {
            let mut params = serde_json::Map::new();
            match capability.params {
                CapabilityParamKind::None => {}
                CapabilityParamKind::Storage => {
                    if !cfg.keys.is_empty() {
                        params.insert(
                            "keys".into(),
                            serde_json::Value::Array(
                                cfg.keys
                                    .iter()
                                    .map(|k| serde_json::Value::String(k.clone()))
                                    .collect(),
                            ),
                        );
                    }
                    if let Some(v) = cfg.max_bytes {
                        params.insert("maxBytes".into(), serde_json::json!(v));
                    }
                }
                CapabilityParamKind::Timer => {
                    if let Some(v) = cfg.max_per_minute {
                        params.insert("maxPerMinute".into(), serde_json::json!(v));
                    }
                    if let Some(v) = cfg.max_active {
                        params.insert("maxActive".into(), serde_json::json!(v));
                    }
                }
                CapabilityParamKind::Metrics => {
                    if let Some(v) = cfg.sample_rate_hz {
                        params.insert("sampleRateHz".into(), serde_json::json!(v));
                    }
                }
            }
            permissions.push(PermissionDecl {
                capability: capability.name.to_owned(),
                params: if params.is_empty() {
                    None
                } else {
                    Some(serde_json::Value::Object(params))
                },
            });
        }
    }

    let size = |a: &Option<[f32; 2]>, d: [f32; 2]| {
        let v = a.unwrap_or(d);
        LogicalSize {
            width: v[0],
            height: v[1],
        }
    };

    let manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        id: floatile_core::types::PluginId(config.plugin.id.clone()),
        name: config.plugin.name.clone(),
        description: None,
        version: config.plugin.version.clone(),
        publisher: match &config.publisher {
            Some(p) => Publisher {
                id: p.id.clone(),
                name: p.name.clone(),
            },
            None => Publisher {
                id: config.plugin.id.clone(),
                name: config.plugin.name.clone(),
            },
        },
        engine_api_version: ENGINE_API_VERSION.to_owned(),
        ui_api_version: UI_API_VERSION.to_owned(),
        kind: PluginKind::Widget,
        entrypoints: Entrypoints {
            ui: PackagePath::parse("ui/widget.ftui")?,
            logic: PackagePath::parse("logic/plugin.wasm")?,
        },
        sizes: Sizes {
            default: size(&config.widget.default_size, default),
            min: size(&config.widget.min_size, min),
            max: size(&config.widget.max_size, max),
            resizable: config.widget.resizable,
        },
        permissions,
        config: None,
        storage: None,
        build: Some(BuildMeta {
            sdk: "rust".into(),
            sdk_version: "0.1.0".into(),
        }),
    };
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// 生成 `new` 的项目模板文件。
pub fn generate_template(dir: &Path, id: &str, name: &str) -> Result<(), ProjectError> {
    let manifest_dir = dir.join("src");
    std::fs::create_dir_all(&manifest_dir).map_err(|e| ProjectError::Io(e.to_string()))?;

    let floatile_toml = format!(
        r#"[plugin]
id = "{id}"
name = "{name}"
version = "0.1.0"

[widget]
default_size = [240, 120]
min_size = [160, 80]
max_size = [800, 600]

[permissions.timer]
max_per_minute = 60
max_active = 2
"#
    );
    let cargo_toml = r#"[package]
name = "clock"
version = "0.1.0"
edition = "2021"

[dependencies]
# NOTE: floatile-sdk 尚未发布（许可 ADR 未通过）。独立构建前需先发布 SDK，
# 或改用 path/patch 指向本地 SDK。workspace 内成员（如 plugins/clock-wasm）
# 可直接构建。
floatile-sdk = "0.1"
serde = { version = "1", features = ["derive"] }
"#
    .to_owned();
    let lib_rs = r#"use floatile_sdk::{Context, LogLevel, State, Widget, WidgetEvent, view};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, State)]
pub struct MyState {
    pub message: String,
}

#[derive(Default)]
struct MyWidget;

impl Widget for MyWidget {
    type State = MyState;
    type Event = WidgetEvent;

    fn view(_state: &Self::State) -> view::View {
        view::column(vec![view::text_bind("$.message")])
    }

    fn start(&mut self, _ctx: &mut Context<Self>) {}

    fn event(&mut self, _event: WidgetEvent, ctx: &mut Context<Self>) {
        let _ = ctx.log(LogLevel::Info, "event received");
    }
}

#[cfg(target_arch = "wasm32")]
floatile_sdk::impl_export_widget!(MyWidget);
"#
    .to_owned();

    write_file(&dir.join("floatile.toml"), &floatile_toml)?;
    write_file(&dir.join("Cargo.toml"), &cargo_toml)?;
    write_file(&manifest_dir.join("lib.rs"), &lib_rs)?;
    Ok(())
}

fn write_file(path: &Path, content: &str) -> Result<(), ProjectError> {
    let mut f = std::fs::File::create(path).map_err(|e| ProjectError::Io(e.to_string()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| ProjectError::Io(e.to_string()))
}

/// 项目解析/生成错误。
#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("floatile.toml 解析失败: {0}")]
    InvalidToml(String),
    #[error("manifest 生成失败: {0}")]
    InvalidManifest(#[from] floatile_core::ManifestError),
    #[error("未知能力段 `{0}`")]
    UnknownCapability(String),
    #[error("I/O 失败: {0}")]
    Io(String),
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sample_toml() -> &'static str {
        r#"[plugin]
id = "dev.floatile.clock"
name = "World Clock"
version = "0.1.0"

[widget]
default_size = [240, 120]

[permissions.timer]
max_per_minute = 60
max_active = 2
"#
    }

    #[test]
    fn parses_floatile_toml() {
        let config = parse_floatile_toml(sample_toml()).unwrap();
        assert_eq!(config.plugin.id, "dev.floatile.clock");
        assert_eq!(config.widget.default_size, Some([240.0, 120.0]));
    }

    #[test]
    fn generates_valid_manifest() {
        let config = parse_floatile_toml(sample_toml()).unwrap();
        let manifest = generate_manifest(&config).unwrap();
        assert_eq!(manifest.id.0, "dev.floatile.clock");
        assert_eq!(manifest.engine_api_version, ENGINE_API_VERSION);
        assert_eq!(manifest.permissions.len(), 1);
        // round-trip 序列化后可再校验。
        let json = serde_json::to_string(&manifest).unwrap();
        let back: Manifest = serde_json::from_str(&json).unwrap();
        validate_manifest(&back).unwrap();
    }

    #[test]
    fn rejects_unknown_capability() {
        let toml = r#"[plugin]
id = "dev.floatile.clock"
name = "x"
version = "0.1.0"
[permissions.network]
"#;
        let config = parse_floatile_toml(toml).unwrap();
        assert!(matches!(
            generate_manifest(&config),
            Err(ProjectError::UnknownCapability(_))
        ));
    }

    #[test]
    fn every_author_section_generates_registry_capabilities() {
        let toml = r#"[plugin]
id = "dev.floatile.all-capabilities"
name = "All Capabilities"
version = "0.1.0"

[permissions.storage]
keys = ["settings"]
max_bytes = 1024

[permissions.timer]
max_per_minute = 60
max_active = 2

[permissions.theme]

[permissions.metrics]
sample_rate_hz = 2
"#;
        let config = parse_floatile_toml(toml).unwrap();
        let manifest = generate_manifest(&config).unwrap();
        let actual: std::collections::BTreeSet<_> = manifest
            .permissions
            .iter()
            .map(|permission| permission.capability.as_str())
            .collect();
        let expected: std::collections::BTreeSet<_> = CAPABILITY_REGISTRY
            .iter()
            .filter(|definition| definition.author_section.is_some())
            .map(|definition| definition.name)
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            manifest
                .permissions
                .iter()
                .find(|permission| permission.capability == "system:memory")
                .unwrap()
                .params,
            None,
            "无参数能力不得继承同一作者段中其他能力的参数"
        );
    }

    #[test]
    fn new_writes_template_files() {
        let dir = std::env::temp_dir().join(format!("floatile-new-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        generate_template(&dir, "dev.floatile.clock", "World Clock").unwrap();
        assert!(dir.join("floatile.toml").exists());
        assert!(dir.join("Cargo.toml").exists());
        assert!(dir.join("src/lib.rs").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
