//! Floatile UI 组件 registry v1（单一机器源）。
//!
//! 每个组件条目定义稳定名称、允许的 props（类型 + 是否可绑定）、可绑定的输入
//! 事件、子组件策略与组件类别。Rust/TypeScript SDK、CLI、runtime 与 renderer 都以
//! 本 registry 为准；不得在别处手写同名平行列表。新增组件/可选 prop 需要 bump
//! `uiApiVersion` minor；删除或改语义需要 bump major。

use std::sync::LazyLock;

use serde::Serialize;

use crate::error::UiSchemaError;
use crate::ir::PropValue;

/// 允许的字面量 JSON 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum JsonType {
    String,
    Boolean,
    Number,
    Integer,
    Object,
    Array,
    Null,
}

impl JsonType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Object => "object",
            Self::Array => "array",
            Self::Null => "null",
        }
    }

    /// 值是否符合该类型（`number` 也接受 `integer`）。
    fn matches(&self, value: &serde_json::Value) -> bool {
        match (self, value) {
            (Self::String, serde_json::Value::String(_)) => true,
            (Self::Boolean, serde_json::Value::Bool(_)) => true,
            (Self::Integer, serde_json::Value::Number(n)) => n.is_i64() || n.is_u64(),
            (Self::Number, serde_json::Value::Number(_)) => true,
            (Self::Object, serde_json::Value::Object(_)) => true,
            (Self::Array, serde_json::Value::Array(_)) => true,
            (Self::Null, serde_json::Value::Null) => true,
            _ => false,
        }
    }
}

/// 单个 prop 的 schema。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropSchema {
    pub name: &'static str,
    pub types: &'static [JsonType],
    /// 是否允许 State/Item 绑定（如 `Text.text`、`Toggle.checked`）。
    pub allow_binding: bool,
    /// 首次引入该 prop 的 UI API 1.x minor。
    pub introduced_minor: u64,
    pub optional: bool,
}

/// 子组件策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChildrenPolicy {
    Forbidden,
    One,
    Many,
}

/// 组件类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComponentKind {
    Element,
    If,
    ForEach,
}

/// 组件规格。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSpec {
    pub name: &'static str,
    /// 首次引入该组件的 UI API 1.x minor。
    pub introduced_minor: u64,
    pub props: Vec<PropSchema>,
    pub input_events: &'static [&'static str],
    pub children: ChildrenPolicy,
    pub kind: ComponentKind,
}

impl ComponentSpec {
    pub fn find_prop(&self, name: &str) -> Option<&PropSchema> {
        self.props.iter().find(|p| p.name == name)
    }

    pub fn declares_input_event(&self, name: &str) -> bool {
        self.input_events.contains(&name)
    }
}

/// 元素组件共享的可选样式 props。
const COMMON_ELEMENT_PROPS: &[PropSchema] = &[
    PropSchema {
        name: "padding",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "gap",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "width",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "height",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "opacity",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "radius",
        types: &[JsonType::Number],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "color",
        types: &[JsonType::String],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
    PropSchema {
        name: "border",
        types: &[JsonType::String],
        allow_binding: false,
        introduced_minor: 0,
        optional: true,
    },
];

/// 组件 registry（惰性构建一次，`'static` 生命周期）。
static REGISTRY: LazyLock<Vec<ComponentSpec>> = LazyLock::new(build_registry);

pub fn components() -> &'static [ComponentSpec] {
    &REGISTRY
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryContract<'a> {
    pub schema_version: u32,
    pub ui_api_version: &'static str,
    pub components: &'a [ComponentSpec],
}

/// Machine-readable single-source contract consumed by SDK code generators.
pub fn contract() -> RegistryContract<'static> {
    RegistryContract {
        schema_version: 1,
        ui_api_version: crate::UI_API_VERSION,
        components: components(),
    }
}

fn build_registry() -> Vec<ComponentSpec> {
    let mut specs = Vec::new();
    element(&mut specs, "Row", ChildrenPolicy::Many, &[], &[]);
    element(&mut specs, "Column", ChildrenPolicy::Many, &[], &[]);
    element(&mut specs, "Stack", ChildrenPolicy::Many, &[], &[]);
    element(
        &mut specs,
        "Grid",
        ChildrenPolicy::Many,
        &[PropSchema {
            name: "columns",
            types: &[JsonType::Integer],
            allow_binding: false,
            introduced_minor: 0,
            optional: true,
        }],
        &[],
    );
    element(&mut specs, "Scroll", ChildrenPolicy::Many, &[], &[]);
    element_since(
        &mut specs,
        "Responsive",
        4,
        ChildrenPolicy::Many,
        &[PropSchema {
            name: "breakpoint",
            types: &[JsonType::Number],
            allow_binding: false,
            introduced_minor: 4,
            optional: false,
        }],
        &[],
    );

    element(
        &mut specs,
        "Text",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "text",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "style",
                types: &[JsonType::String],
                allow_binding: false,
                introduced_minor: 0,
                optional: true,
            },
            PropSchema {
                name: "colorToken",
                types: &[JsonType::String],
                allow_binding: false,
                introduced_minor: 5,
                optional: true,
            },
        ],
        &[],
    );
    element(
        &mut specs,
        "Icon",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "name",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "size",
                types: &[JsonType::Number],
                allow_binding: false,
                introduced_minor: 0,
                optional: true,
            },
        ],
        &[],
    );
    element(
        &mut specs,
        "Image",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "asset",
                types: &[JsonType::String],
                allow_binding: false,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "width",
                types: &[JsonType::Number],
                allow_binding: false,
                introduced_minor: 0,
                optional: true,
            },
            PropSchema {
                name: "height",
                types: &[JsonType::Number],
                allow_binding: false,
                introduced_minor: 0,
                optional: true,
            },
        ],
        &[],
    );

    element(
        &mut specs,
        "Button",
        ChildrenPolicy::Forbidden,
        &[PropSchema {
            name: "label",
            types: &[JsonType::String],
            allow_binding: true,
            introduced_minor: 0,
            optional: false,
        }],
        &["activate"],
    );
    element(
        &mut specs,
        "Toggle",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "checked",
                types: &[JsonType::Boolean],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "accessibilityLabel",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 6,
                optional: true,
            },
        ],
        &["toggle"],
    );

    element(
        &mut specs,
        "Progress",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "value",
                types: &[JsonType::Number],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "accessibilityLabel",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 6,
                optional: true,
            },
        ],
        &[],
    );
    element_since(
        &mut specs,
        "Badge",
        1,
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "label",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "tone",
                types: &[JsonType::String],
                allow_binding: false,
                introduced_minor: 0,
                optional: true,
            },
        ],
        &[],
    );
    element(
        &mut specs,
        "Gauge",
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "value",
                types: &[JsonType::Number],
                allow_binding: true,
                introduced_minor: 0,
                optional: false,
            },
            PropSchema {
                name: "accessibilityLabel",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 6,
                optional: true,
            },
        ],
        &[],
    );
    element(
        &mut specs,
        "List",
        ChildrenPolicy::Many,
        &[PropSchema {
            name: "items",
            types: &[JsonType::Array],
            allow_binding: true,
            introduced_minor: 2,
            optional: true,
        }],
        &[],
    );
    element_since(
        &mut specs,
        "Sparkline",
        3,
        ChildrenPolicy::Forbidden,
        &[
            PropSchema {
                name: "values",
                types: &[JsonType::Array],
                allow_binding: true,
                introduced_minor: 3,
                optional: false,
            },
            PropSchema {
                name: "label",
                types: &[JsonType::String],
                allow_binding: true,
                introduced_minor: 3,
                optional: false,
            },
            PropSchema {
                name: "tone",
                types: &[JsonType::String],
                allow_binding: false,
                introduced_minor: 3,
                optional: true,
            },
        ],
        &[],
    );

    // 控制组件（Canvas/Path 在 renderer spike 通过前不启用）。
    specs.push(ComponentSpec {
        name: "If",
        introduced_minor: 0,
        props: Vec::new(),
        input_events: &[],
        children: ChildrenPolicy::Forbidden,
        kind: ComponentKind::If,
    });
    specs.push(ComponentSpec {
        name: "ForEach",
        introduced_minor: 0,
        props: Vec::new(),
        input_events: &[],
        children: ChildrenPolicy::Forbidden,
        kind: ComponentKind::ForEach,
    });
    specs
}

fn element(
    out: &mut Vec<ComponentSpec>,
    name: &'static str,
    children: ChildrenPolicy,
    props: &[PropSchema],
    events: &'static [&'static str],
) {
    element_since(out, name, 0, children, props, events);
}

fn element_since(
    out: &mut Vec<ComponentSpec>,
    name: &'static str,
    introduced_minor: u64,
    children: ChildrenPolicy,
    props: &[PropSchema],
    events: &'static [&'static str],
) {
    let mut all = Vec::with_capacity(COMMON_ELEMENT_PROPS.len() + props.len());
    all.extend_from_slice(COMMON_ELEMENT_PROPS);
    all.extend_from_slice(props);
    out.push(ComponentSpec {
        name,
        introduced_minor,
        props: all,
        input_events: events,
        children,
        kind: ComponentKind::Element,
    });
}

/// 按名称查找组件规格。
pub fn find(name: &str) -> Result<&'static ComponentSpec, UiSchemaError> {
    components()
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| UiSchemaError::UnknownComponent(name.to_owned()))
}

/// 校验一个字面量 prop 值（非绑定）符合该 prop 允许的类型。
pub fn validate_literal(
    spec: &ComponentSpec,
    prop: &PropSchema,
    value: &PropValue,
) -> Result<(), UiSchemaError> {
    let PropValue::Literal(json) = value else {
        // 绑定由调用方按 allow_binding 另行处理。
        return Ok(());
    };
    if !prop.types.iter().any(|t| t.matches(json)) {
        let expected: Vec<String> = prop.types.iter().map(|t| t.name().to_owned()).collect();
        return Err(UiSchemaError::InvalidPropType {
            prop: format!("{}.{}", spec.name, prop.name),
            expected,
        });
    }
    if spec.name == "Badge"
        && prop.name == "tone"
        && let Some(value) = json.as_str()
        && !["neutral", "info", "success", "warning", "danger"].contains(&value)
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "Badge.tone".to_owned(),
            expected: vec!["neutral|info|success|warning|danger".to_owned()],
        });
    }
    if spec.name == "List"
        && prop.name == "items"
        && let Some(items) = json.as_array()
        && (items.len() > crate::MAX_LIST_ITEMS || items.iter().any(|item| !item.is_string()))
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "List.items".to_owned(),
            expected: vec![format!(
                "string array with at most {} items",
                crate::MAX_LIST_ITEMS
            )],
        });
    }
    if spec.name == "Sparkline"
        && prop.name == "values"
        && let Some(items) = json.as_array()
        && (items.len() > crate::MAX_CHART_POINTS
            || items.iter().any(|item| item.as_f64().is_none()))
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "Sparkline.values".to_owned(),
            expected: vec![format!(
                "number array with at most {} points",
                crate::MAX_CHART_POINTS
            )],
        });
    }
    if spec.name == "Sparkline"
        && prop.name == "tone"
        && let Some(value) = json.as_str()
        && !["info", "success", "warning", "danger"].contains(&value)
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "Sparkline.tone".to_owned(),
            expected: vec!["info|success|warning|danger".to_owned()],
        });
    }
    if spec.name == "Grid"
        && prop.name == "columns"
        && let Some(columns) = json.as_u64()
        && !(1..=16).contains(&columns)
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "Grid.columns".to_owned(),
            expected: vec!["integer from 1 through 16".to_owned()],
        });
    }
    if spec.name == "Responsive"
        && prop.name == "breakpoint"
        && let Some(breakpoint) = json.as_f64()
        && !(240.0..=1920.0).contains(&breakpoint)
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: "Responsive.breakpoint".to_owned(),
            expected: vec!["number from 240 through 1920".to_owned()],
        });
    }
    if prop.name == "colorToken"
        && let Some(token) = json.as_str()
        && crate::theme::color_token(token).is_none()
    {
        return Err(UiSchemaError::InvalidPropType {
            prop: format!("{}.colorToken", spec.name),
            expected: crate::theme::COLOR_TOKENS
                .iter()
                .map(|token| token.name.to_owned())
                .collect(),
        });
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_expected_v1_components() {
        for name in [
            "Row", "Column", "Stack", "Grid", "Scroll", "Text", "Icon", "Image", "Button",
            "Toggle", "Progress", "Badge", "Gauge", "List", "If", "ForEach",
        ] {
            assert!(find(name).is_ok(), "missing component {name}");
        }
        // Canvas/Path 尚未启用。
        assert!(find("Canvas").is_err());
        assert!(find("Path").is_err());
    }

    #[test]
    fn component_input_events() {
        let button = find("Button").unwrap();
        assert!(button.declares_input_event("activate"));
        assert!(!button.declares_input_event("toggle"));
        let text = find("Text").unwrap();
        assert!(!text.declares_input_event("activate"));
    }

    #[test]
    fn literal_type_check() {
        let text = find("Text").unwrap();
        let prop = text.find_prop("text").unwrap();
        assert!(validate_literal(text, prop, &PropValue::Literal(serde_json::json!("hi"))).is_ok());
        assert!(validate_literal(text, prop, &PropValue::Literal(serde_json::json!(5))).is_err());
    }

    #[test]
    fn common_style_props_shared() {
        let column = find("Column").unwrap();
        assert!(column.find_prop("padding").is_some());
        assert!(column.find_prop("gap").is_some());
    }

    #[test]
    fn machine_contract_serializes_the_registry_source() {
        let value = serde_json::to_value(contract()).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["uiApiVersion"], crate::UI_API_VERSION);
        let specs = value["components"].as_array().unwrap();
        assert_eq!(specs.len(), components().len());
        assert!(specs.iter().any(|spec| spec["name"] == "Text"));
    }
}
