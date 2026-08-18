//! `State` trait：schema 单源（由 `#[derive(State)]` 生成）。
//!
//! 结构体一次定义，自动获得：
//! - `State::schema() -> JsonSchema`（JSON Schema 由字段类型推导）
//! - `State::initial() -> Self`（每个字段用类型默认值）
//!
//! runtime 的 host-ui.update-state 使用 host 侧的 `validate_state_value`
//! 校验 State，插件 SDK 不在 guest 重复校验。

use floatile_ui_schema::JsonSchema;

pub trait State: serde::Serialize + serde::de::DeserializeOwned {
    fn schema() -> JsonSchema;
    fn initial() -> Self;
}
