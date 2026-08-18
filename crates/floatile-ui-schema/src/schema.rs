//! `widget.ftui` 的 State/Event JSON Schema 模型与值校验。
//!
//! 这是 UI schema 单一源的组成部分：Rust/TypeScript SDK、CLI、runtime 与 renderer
//! 都以同一套 schema 语义校验不可信 State/event payload。本模块不执行 I/O，可同时
//! 在 host 与 `wasm32-wasip2` 编译。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::UiSchemaError;

/// 受支持的最大 State 嵌套深度。
pub const MAX_STATE_DEPTH: usize = 16;

/// JSON Schema（v1 子集）。
///
/// 不支持脚本、`$ref`、任意表达式或动态 key；`Default` 用于构建时文档骨架，
/// 是宽松策略，运行期校验仍以 `additional_properties: false` + required 为准。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JsonSchema {
    #[serde(rename = "string")]
    String {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<usize>,
    },
    #[serde(rename = "boolean")]
    Boolean,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "integer")]
    Integer,
    #[serde(rename = "null")]
    Null,
    #[serde(rename = "array")]
    Array {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<usize>,
        items: Box<JsonSchema>,
    },
    #[serde(rename = "object")]
    Object {
        #[serde(default)]
        required: Vec<String>,
        #[serde(default)]
        properties: BTreeMap<String, JsonSchema>,
        #[serde(default = "default_true")]
        additional_properties: bool,
    },
}

fn default_true() -> bool {
    true
}

impl Default for JsonSchema {
    fn default() -> Self {
        Self::Object {
            required: Vec::new(),
            properties: BTreeMap::new(),
            additional_properties: true,
        }
    }
}

/// 校验一个 JSON 值是否符合 schema，并施加最大深度限制。
///
/// 返回的 `path` 定位到出错字段（如 `$.zones[0]`），供诊断使用；不泄漏宿主内部结构。
pub fn validate_value(
    schema: &JsonSchema,
    value: &serde_json::Value,
    path: &str,
    depth: usize,
) -> Result<(), UiSchemaError> {
    if depth > MAX_STATE_DEPTH {
        return Err(UiSchemaError::InvalidState(format!(
            "{path}: 超过最大深度 {MAX_STATE_DEPTH}"
        )));
    }
    match (schema, value) {
        (JsonSchema::Null, serde_json::Value::Null) => Ok(()),
        (JsonSchema::Boolean, serde_json::Value::Bool(_)) => Ok(()),
        (JsonSchema::Integer, serde_json::Value::Number(n))
            if n.as_i64().is_some() || n.as_u64().is_some() =>
        {
            Ok(())
        }
        (JsonSchema::Number, serde_json::Value::Number(_)) => Ok(()),
        (JsonSchema::String { max_length }, serde_json::Value::String(s)) => {
            if let Some(max) = max_length
                && s.chars().count() > *max
            {
                return Err(UiSchemaError::InvalidState(format!(
                    "{path}: 字符串长度 {} 超过上限 {max}",
                    s.chars().count()
                )));
            }
            Ok(())
        }
        (JsonSchema::Array { max_items, items }, serde_json::Value::Array(arr)) => {
            if let Some(max) = max_items
                && arr.len() > *max
            {
                return Err(UiSchemaError::InvalidState(format!(
                    "{path}: 数组长度 {} 超过上限 {max}",
                    arr.len()
                )));
            }
            for (i, item) in arr.iter().enumerate() {
                validate_value(items, item, &format!("{path}[{i}]"), depth + 1)?;
            }
            Ok(())
        }
        (
            JsonSchema::Object {
                required,
                properties,
                additional_properties,
            },
            serde_json::Value::Object(map),
        ) => {
            if !*additional_properties {
                for key in map.keys() {
                    if !properties.contains_key(key) {
                        return Err(UiSchemaError::InvalidState(format!(
                            "{path}: 未知字段 `{key}`"
                        )));
                    }
                }
            }
            for name in required {
                if !map.contains_key(name) {
                    return Err(UiSchemaError::InvalidState(format!(
                        "{path}: 缺少必填字段 `{name}`"
                    )));
                }
            }
            for (key, sub) in map {
                let sub_path = if path.is_empty() {
                    format!("${key}")
                } else {
                    format!("{path}.{key}")
                };
                match properties.get(key) {
                    Some(sub_schema) => {
                        validate_value(sub_schema, sub, &sub_path, depth + 1)?;
                    }
                    None if *additional_properties => {
                        // 动态字段：只施加深度限制。
                        ensure_depth(&sub_path, depth + 1)?;
                    }
                    None => {}
                }
            }
            Ok(())
        }
        _ => Err(UiSchemaError::InvalidState(format!(
            "{path}: 类型不匹配，期望 {}，实际 {}",
            type_name(schema),
            json_type_name(value)
        ))),
    }
}

fn ensure_depth(path: &str, depth: usize) -> Result<(), UiSchemaError> {
    if depth > MAX_STATE_DEPTH {
        return Err(UiSchemaError::InvalidState(format!(
            "{path}: 超过最大深度 {MAX_STATE_DEPTH}"
        )));
    }
    Ok(())
}

fn type_name(schema: &JsonSchema) -> &'static str {
    match schema {
        JsonSchema::String { .. } => "string",
        JsonSchema::Boolean => "boolean",
        JsonSchema::Number => "number",
        JsonSchema::Integer => "integer",
        JsonSchema::Null => "null",
        JsonSchema::Array { .. } => "array",
        JsonSchema::Object { .. } => "object",
    }
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_nested_object_with_limits() {
        let schema = JsonSchema::Object {
            required: vec!["time".into(), "running".into()],
            properties: BTreeMap::from([
                (
                    "time".into(),
                    JsonSchema::String {
                        max_length: Some(32),
                    },
                ),
                ("running".into(), JsonSchema::Boolean),
                (
                    "zones".into(),
                    JsonSchema::Array {
                        max_items: Some(16),
                        items: Box::new(JsonSchema::String {
                            max_length: Some(64),
                        }),
                    },
                ),
            ]),
            additional_properties: false,
        };
        let ok = json!({"time": "12:00:00", "running": true, "zones": ["UTC", "PST"]});
        assert!(validate_value(&schema, &ok, "$", 0).is_ok());

        // 未知字段被拒。
        let extra = json!({"time": "x", "running": true, "zones": [], "nope": 1});
        assert!(matches!(
            validate_value(&schema, &extra, "$", 0),
            Err(UiSchemaError::InvalidState(_))
        ));

        // 缺少必填字段被拒。
        let missing = json!({"time": "x", "zones": []});
        assert!(validate_value(&schema, &missing, "$", 0).is_err());

        // 类型错误被拒并定位 path。
        let wrong = json!({"time": "x", "running": "yes", "zones": []});
        match validate_value(&schema, &wrong, "$", 0) {
            Err(UiSchemaError::InvalidState(msg)) => {
                assert!(msg.contains("$.running"), "got: {msg}");
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }

        // 数组长度超限被拒。
        let too_many = json!({"time": "x", "running": true, "zones": (0..17).map(|i| i.to_string()).collect::<Vec<_>>()});
        assert!(validate_value(&schema, &too_many, "$", 0).is_err());

        // 字符串超长被拒。
        let long = json!({"time": "a".repeat(33), "running": true, "zones": []});
        assert!(validate_value(&schema, &long, "$", 0).is_err());
    }

    #[test]
    fn enforces_max_depth() {
        let inner = JsonSchema::Number;
        let schema = (0..20).fold(inner, |acc, _| JsonSchema::Array {
            max_items: None,
            items: Box::new(acc),
        });
        // 构造 20 层嵌套数组。
        let mut value = serde_json::Value::Array(vec![json!(1)]);
        for _ in 0..19 {
            value = serde_json::Value::Array(vec![value]);
        }
        assert!(validate_value(&schema, &value, "$", 0).is_err());
    }
}
