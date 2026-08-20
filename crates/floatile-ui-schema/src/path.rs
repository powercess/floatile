//! JSONPath 子集的解析与对 State schema 的解析。
//!
//! v1 只支持 `$.field.field...` 的 object 字段遍历，不支持脚本、filter、
//! 递归 descent 或动态 key。`parse` 把 `$.a.b` 解析为字段段；`resolve` 沿着
//! State 的 `Object` schema 定位目标 schema 并返回其 JSON 类型。

use crate::error::UiSchemaError;
use crate::schema::JsonSchema;

/// JSONPath 子集解析结果：根 `$` 之后的字段段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegments {
    segments: Vec<String>,
}

impl PathSegments {
    /// 解析 `$.a.b`；`$` 或 `$` 加空段也合法（返回空段，指向根）。
    pub fn parse(path: &str) -> Result<Self, UiSchemaError> {
        if !path.starts_with('$') {
            return Err(UiSchemaError::InvalidBindingPath(format!(
                "{path}: 必须以 `$` 开头"
            )));
        }
        let rest = &path[1..];
        if rest.is_empty() {
            return Ok(Self {
                segments: Vec::new(),
            });
        }
        if !rest.starts_with('.') {
            return Err(UiSchemaError::InvalidBindingPath(format!(
                "{path}: `$` 后必须紧跟 `.field`"
            )));
        }
        let mut segments = Vec::new();
        for field in rest[1..].split('.') {
            if field.is_empty() {
                return Err(UiSchemaError::InvalidBindingPath(format!(
                    "{path}: 存在空字段段"
                )));
            }
            // JSONPath 子集只允许纯标识符字段段，拒绝数组索引/通配/空白等。
            if field
                .chars()
                .any(|c| c == '[' || c == ']' || c == '*' || c.is_whitespace())
            {
                return Err(UiSchemaError::InvalidBindingPath(format!(
                    "{path}: 段 `{field}` 含非法字符"
                )));
            }
            segments.push(field.to_owned());
        }
        Ok(Self { segments })
    }

    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

/// 从 State 的根 schema 沿字段段解析目标 schema 及其 JSON 类型名。
///
/// 只允许 object 属性遍历；字段缺失、额外越过 array/标量均视为不合法。
pub fn resolve<'a>(
    schema: &'a JsonSchema,
    segments: &[String],
) -> Result<&'a JsonSchema, UiSchemaError> {
    let mut current = schema;
    for field in segments {
        match current {
            JsonSchema::Object {
                properties,
                additional_properties,
                ..
            } => {
                if let Some(sub) = properties.get(field) {
                    current = sub;
                } else if *additional_properties {
                    return Err(UiSchemaError::InvalidBindingPath(format!(
                        "`{field}` 在 additionalProperties schema 上无法静态解析类型"
                    )));
                } else {
                    return Err(UiSchemaError::InvalidBindingPath(format!(
                        "State 不存在字段 `{field}`"
                    )));
                }
            }
            _ => {
                return Err(UiSchemaError::InvalidBindingPath(format!(
                    "`{field}`：无法在非 object schema 上继续遍历"
                )));
            }
        }
    }
    Ok(current)
}

/// 目标 schema 的 JSON 类型名（`string`/`boolean`/`number`/`array`/`object`/`null`）。
pub fn json_type(schema: &JsonSchema) -> &'static str {
    match schema {
        JsonSchema::String { .. } => "string",
        JsonSchema::Boolean => "boolean",
        JsonSchema::Number | JsonSchema::Integer => "number",
        JsonSchema::Null => "null",
        JsonSchema::Array { .. } => "array",
        JsonSchema::Object { .. } => "object",
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn root_schema() -> JsonSchema {
        JsonSchema::Object {
            required: vec![],
            additional_properties: false,
            properties: BTreeMap::from([
                ("time".into(), JsonSchema::String { max_length: None }),
                (
                    "zones".into(),
                    JsonSchema::Array {
                        max_items: None,
                        items: Box::new(JsonSchema::String { max_length: None }),
                    },
                ),
            ]),
        }
    }

    #[test]
    fn parses_subset_paths() {
        assert!(PathSegments::parse("$.time").is_ok());
        assert!(PathSegments::parse("$.a.b.c").is_ok());
        assert_eq!(PathSegments::parse("$.time").unwrap().segments(), &["time"]);
        // 非法形式。
        assert!(PathSegments::parse("time").is_err());
        assert!(PathSegments::parse("$.a..b").is_err());
        assert!(PathSegments::parse("$a").is_err());
        assert!(PathSegments::parse("$.a[0]").is_err());
    }

    #[test]
    fn resolves_existing_fields() {
        let s = root_schema();
        let time = resolve(&s, &["time".to_owned()]).unwrap();
        assert_eq!(json_type(time), "string");
        let zones = resolve(&s, &["zones".to_owned()]).unwrap();
        assert_eq!(json_type(zones), "array");
        // 根路径。
        assert_eq!(json_type(resolve(&s, &[]).unwrap()), "object");
    }

    #[test]
    fn rejects_missing_or_unwalkable() {
        let s = root_schema();
        assert!(resolve(&s, &["nope".to_owned()]).is_err());
        // 越界遍历 array。
        assert!(resolve(&s, &["zones".to_owned(), "x".to_owned()]).is_err());
    }
}
