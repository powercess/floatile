//! `widget.ftui` v1 文档的 IR 类型。
//!
//! IR 是构建期生成的静态 UI 文档：组件树、State/Event schema、绑定、有限
//! If/ForEach、资源引用。v1 采用 canonical JSON 编码，本模块类型与其一一对应，
//! 是 Rust/TypeScript SDK、CLI、runtime 与 renderer 共享的单源结构。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::schema::JsonSchema;

/// 组件树节点。
///
/// `props`、`children`、`events` 与 If/ForEach 专用字段（`when/then/else`、
/// `items/key/template`）的语义由 `registry` 决定；未使用的字段必须为空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Component {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub props: BTreeMap<String, PropValue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Component>,
    /// 输入事件绑定：输入事件名 → 发出的声明事件 + payload。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub events: BTreeMap<String, EmittedEvent>,
    /// If：`when` 布尔 State 绑定。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<Box<Component>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub else_: Option<Box<Component>>,
    /// ForEach：`items` 数组 State 绑定 + 稳定 `key` + 模板。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<Box<Component>>,
}

/// prop 值：字面量 或 绑定。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    Literal(serde_json::Value),
    Binding(Binding),
}

/// v1 绑定：State 绑定 `{"bind": "$.path"}` 或 ForEach item 绑定 `{"item": "field"}`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Binding {
    State { bind: String },
    Item { item: String },
}

/// 组件输入事件 → 发出的声明事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmittedEvent {
    pub emit: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

/// State 声明：canonical initial 值 + schema。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateSchema {
    pub initial: serde_json::Value,
    pub schema: JsonSchema,
}

/// 事件声明：稳定名称 → payload schema。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventSchema {
    pub payload: JsonSchema,
}

/// `widget.ftui` v1 文档根。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiDocument {
    #[serde(rename = "uiApiVersion")]
    pub ui_api_version: String,
    pub state: StateSchema,
    pub events: BTreeMap<String, EventSchema>,
    pub root: Component,
}
