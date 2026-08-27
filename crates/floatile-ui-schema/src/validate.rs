//! `widget.ftui` v1 文档的结构与预算校验（host/CLI/runtime 共用）。
//!
//! 校验顺序：版本 → IR 字节预算 → 事件声明 → initial State schema → 组件树
//! （registry、props、绑定路径/类型、If/ForEach、事件、节点/深度/绑定/asset 预算）。
//! 任一失败都返回稳定 code，不泄漏宿主内部结构；CLI 通过不代表 runtime 可跳过复验。

use std::collections::BTreeMap;

use crate::ir::{Binding, Component, EventSchema, PropValue, UiDocument};
use crate::path;
use crate::registry::{self, ChildrenPolicy, ComponentKind, JsonType};
use crate::schema::{self, JsonSchema};
use crate::{
    MAX_ASSET_REFS, MAX_BINDINGS, MAX_EVENT_DECLS, MAX_IR_BYTES, MAX_NODES, MAX_TREE_DEPTH,
    UiSchemaError,
};

/// 校验一个完整的 `widget.ftui` 文档。
pub fn validate_document(doc: &UiDocument) -> Result<(), UiSchemaError> {
    let ui_minor = check_api_version(&doc.ui_api_version)?;
    check_ir_size(doc)?;

    if doc.events.len() > MAX_EVENT_DECLS {
        return Err(UiSchemaError::LimitExceeded(format!(
            "事件声明 {} 超过上限 {MAX_EVENT_DECLS}",
            doc.events.len()
        )));
    }
    for (name, event) in &doc.events {
        check_event_name(name)?;
        check_schema_depth(&event.payload, 0, &format!("events.{name}.payload"))?;
    }

    // initial State 必须通过完整 schema（含深度限制）。
    schema::validate_value(&doc.state.schema, &doc.state.initial, "$", 0)?;
    check_schema_depth(&doc.state.schema, 0, "state.schema")?;

    let mut ctx = Ctx {
        doc_events: &doc.events,
        nodes: 0,
        bindings: 0,
        asset_refs: 0,
        ui_minor,
    };
    validate_component(&doc.root, &doc.state.schema, None, &mut ctx, 1)
}

/// 校验 uiApiVersion：只接受 major 1（`1` 或 `1.x.y`）。
fn check_api_version(version: &str) -> Result<u64, UiSchemaError> {
    let mut parts = version.split('.');
    if parts.next() != Some("1") {
        return Err(UiSchemaError::UnsupportedApiVersion(version.to_owned()));
    }
    match parts.next() {
        None => Ok(0),
        Some(minor) => minor
            .parse::<u64>()
            .map_err(|_| UiSchemaError::UnsupportedApiVersion(version.to_owned())),
    }
}

fn check_ir_size(doc: &UiDocument) -> Result<(), UiSchemaError> {
    let bytes = serde_json::to_vec(doc)
        .map_err(|e| UiSchemaError::InvalidState(format!("IR 序列化失败: {e}")))?;
    if bytes.len() > MAX_IR_BYTES {
        return Err(UiSchemaError::LimitExceeded(format!(
            "IR 大小 {} 字节超过上限 {MAX_IR_BYTES}",
            bytes.len()
        )));
    }
    Ok(())
}

fn check_event_name(name: &str) -> Result<(), UiSchemaError> {
    if name.is_empty() {
        return Err(UiSchemaError::UnknownEvent("空事件名".to_owned()));
    }
    Ok(())
}

/// 递归校验 schema 自身深度（防止病理深度的 schema 绕过值校验）。
fn check_schema_depth(schema: &JsonSchema, depth: usize, path: &str) -> Result<(), UiSchemaError> {
    if depth > schema::MAX_STATE_DEPTH {
        return Err(UiSchemaError::InvalidState(format!(
            "{path}: schema 超过最大深度 {}",
            schema::MAX_STATE_DEPTH
        )));
    }
    match schema {
        JsonSchema::Array { items, .. } => {
            check_schema_depth(items, depth + 1, path)?;
        }
        JsonSchema::Object { properties, .. } => {
            for (key, sub) in properties {
                check_schema_depth(sub, depth + 1, &format!("{path}.{key}"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

struct Ctx<'a> {
    doc_events: &'a BTreeMap<String, EventSchema>,
    nodes: usize,
    bindings: usize,
    asset_refs: usize,
    ui_minor: u64,
}

fn validate_component(
    comp: &Component,
    state_schema: &JsonSchema,
    item_schema: Option<&JsonSchema>,
    ctx: &mut Ctx<'_>,
    depth: usize,
) -> Result<(), UiSchemaError> {
    if depth > MAX_TREE_DEPTH {
        return Err(UiSchemaError::LimitExceeded(format!(
            "组件树深度 {depth} 超过上限 {MAX_TREE_DEPTH}"
        )));
    }
    ctx.nodes += 1;
    if ctx.nodes > MAX_NODES {
        return Err(UiSchemaError::LimitExceeded(format!(
            "组件节点数 {} 超过上限 {MAX_NODES}",
            ctx.nodes
        )));
    }
    let spec = registry::find(&comp.kind)?;
    if spec.introduced_minor > ctx.ui_minor {
        return Err(UiSchemaError::UnsupportedApiVersion(format!(
            "{} requires 1.{}.0",
            comp.kind, spec.introduced_minor
        )));
    }
    match spec.kind {
        ComponentKind::If => validate_if(comp, state_schema, ctx, depth),
        ComponentKind::ForEach => validate_foreach(comp, state_schema, ctx, depth),
        ComponentKind::Element => {
            validate_element(comp, spec, state_schema, item_schema, ctx, depth)
        }
    }
}

fn validate_element(
    comp: &Component,
    spec: &'static registry::ComponentSpec,
    state_schema: &JsonSchema,
    item_schema: Option<&JsonSchema>,
    ctx: &mut Ctx<'_>,
    depth: usize,
) -> Result<(), UiSchemaError> {
    // 元素组件不得携带 If/ForEach 专用字段。
    if comp.when.is_some()
        || comp.then.is_some()
        || comp.else_.is_some()
        || comp.items.is_some()
        || comp.key.is_some()
        || comp.template.is_some()
    {
        return Err(UiSchemaError::InvalidControl(format!(
            "`{}` 是元素组件，不能携带 If/ForEach 字段",
            comp.kind
        )));
    }

    for (name, value) in &comp.props {
        let prop = spec
            .find_prop(name)
            .ok_or_else(|| UiSchemaError::UnknownProp {
                component: comp.kind.clone(),
                prop: name.clone(),
            })?;
        if prop.introduced_minor > ctx.ui_minor {
            return Err(UiSchemaError::UnsupportedApiVersion(format!(
                "{}.{} requires 1.{}.0",
                comp.kind, name, prop.introduced_minor
            )));
        }
        match value {
            PropValue::Binding(binding) => {
                if !prop.allow_binding {
                    return Err(UiSchemaError::BindingTypeMismatch(format!(
                        "{}.{} 不允许绑定",
                        comp.kind, name
                    )));
                }
                validate_binding(binding, prop, state_schema, item_schema, &comp.kind, name)?;
                ctx.bindings += 1;
                if ctx.bindings > MAX_BINDINGS {
                    return Err(UiSchemaError::LimitExceeded(format!(
                        "绑定数 {} 超过上限 {MAX_BINDINGS}",
                        ctx.bindings
                    )));
                }
            }
            PropValue::Literal(_) => registry::validate_literal(spec, prop, value)?,
        }
    }

    // 必填 prop 缺失检查。
    for prop in &spec.props {
        if !prop.optional && !comp.props.contains_key(prop.name) {
            return Err(UiSchemaError::MissingProp {
                component: comp.kind.clone(),
                prop: prop.name.to_owned(),
            });
        }
    }

    // 子组件策略。
    if comp.kind == "List" && comp.props.contains_key("items") && !comp.children.is_empty() {
        return Err(UiSchemaError::InvalidChildren(
            "List 使用 items 时不能同时声明静态 children".to_owned(),
        ));
    }
    match spec.children {
        ChildrenPolicy::Forbidden => {
            if !comp.children.is_empty() {
                return Err(UiSchemaError::InvalidChildren(format!(
                    "`{}` 不允许子组件",
                    comp.kind
                )));
            }
        }
        ChildrenPolicy::One => {
            if comp.children.len() != 1 {
                return Err(UiSchemaError::InvalidChildren(format!(
                    "`{}` 需要一个子组件，实际 {}",
                    comp.kind,
                    comp.children.len()
                )));
            }
        }
        ChildrenPolicy::Many => {}
    }
    for child in &comp.children {
        validate_component(child, state_schema, item_schema, ctx, depth + 1)?;
    }

    // 输入事件绑定。
    for (input, emitted) in &comp.events {
        if !spec.declares_input_event(input) {
            return Err(UiSchemaError::UnknownInputEvent {
                component: comp.kind.clone(),
                event: input.clone(),
            });
        }
        let declared = ctx
            .doc_events
            .get(&emitted.emit)
            .ok_or_else(|| UiSchemaError::UnknownEvent(emitted.emit.clone()))?;
        schema::validate_value(
            &declared.payload,
            &emitted.payload,
            &format!("events.{}.payload", emitted.emit),
            0,
        )?;
    }

    // asset 引用预算（Image.asset）。
    if comp.kind == "Image" {
        ctx.asset_refs += 1;
        if ctx.asset_refs > MAX_ASSET_REFS {
            return Err(UiSchemaError::LimitExceeded(format!(
                "asset 引用数 {} 超过上限 {MAX_ASSET_REFS}",
                ctx.asset_refs
            )));
        }
    }

    Ok(())
}

fn validate_binding(
    binding: &Binding,
    prop: &registry::PropSchema,
    state_schema: &JsonSchema,
    item_schema: Option<&JsonSchema>,
    component: &str,
    prop_name: &str,
) -> Result<(), UiSchemaError> {
    let target_schema = match binding {
        Binding::State { bind } => {
            let segs = path::PathSegments::parse(bind)?;
            path::resolve(state_schema, segs.segments())?
        }
        Binding::Item { item } => {
            let item_schema = item_schema.ok_or_else(|| {
                UiSchemaError::InvalidItemBinding(format!(
                    "`{item}` 只能在 ForEach template 内使用"
                ))
            })?;
            if item == "value" {
                item_schema
            } else {
                let segs = path::PathSegments::parse(&format!("$.{item}"))?;
                path::resolve(item_schema, segs.segments())?
            }
        }
    };
    if component == "List" && prop_name == "items" {
        match target_schema {
            JsonSchema::Array {
                max_items: Some(max_items),
                items,
            } if *max_items <= crate::MAX_LIST_ITEMS
                && matches!(items.as_ref(), JsonSchema::String { .. }) => {}
            _ => {
                return Err(UiSchemaError::BindingTypeMismatch(format!(
                    "List.items 必须绑定 maxItems <= {} 的 string array",
                    crate::MAX_LIST_ITEMS
                )));
            }
        }
    }
    if component == "Sparkline" && prop_name == "values" {
        match target_schema {
            JsonSchema::Array {
                max_items: Some(max_items),
                items,
            } if *max_items <= crate::MAX_CHART_POINTS
                && matches!(items.as_ref(), JsonSchema::Number | JsonSchema::Integer) => {}
            _ => {
                return Err(UiSchemaError::BindingTypeMismatch(format!(
                    "Sparkline.values 必须绑定 maxItems <= {} 的 number array",
                    crate::MAX_CHART_POINTS
                )));
            }
        }
    }
    let target_type = path::json_type(target_schema).to_owned();
    let allowed = prop.types.iter().any(|t| {
        t.name() == target_type || (matches!(t, JsonType::Number) && target_type == "integer")
    });
    if !allowed {
        return Err(UiSchemaError::BindingTypeMismatch(format!(
            "{component}.{prop_name}: 绑定目标类型 {target_type} 不在允许集合"
        )));
    }
    Ok(())
}

fn validate_if(
    comp: &Component,
    state_schema: &JsonSchema,
    ctx: &mut Ctx<'_>,
    depth: usize,
) -> Result<(), UiSchemaError> {
    if !comp.props.is_empty() || !comp.events.is_empty() || !comp.children.is_empty() {
        return Err(UiSchemaError::InvalidControl(
            "If 不使用 props/events/children".to_owned(),
        ));
    }
    let Some(Binding::State { bind }) = &comp.when else {
        return Err(UiSchemaError::InvalidControl(
            "If 需要 when 布尔 State 绑定".to_owned(),
        ));
    };
    let segs = path::PathSegments::parse(bind)?;
    let target = path::resolve(state_schema, segs.segments())?;
    if path::json_type(target) != "boolean" {
        return Err(UiSchemaError::BindingTypeMismatch(format!(
            "If.when 必须绑定 boolean，实际 {}",
            path::json_type(target)
        )));
    }
    let Some(then) = &comp.then else {
        return Err(UiSchemaError::InvalidControl(
            "If 需要 then 分支".to_owned(),
        ));
    };
    validate_component(then, state_schema, None, ctx, depth + 1)?;
    if let Some(els) = &comp.else_ {
        validate_component(els, state_schema, None, ctx, depth + 1)?;
    }
    Ok(())
}

fn validate_foreach(
    comp: &Component,
    state_schema: &JsonSchema,
    ctx: &mut Ctx<'_>,
    depth: usize,
) -> Result<(), UiSchemaError> {
    if !comp.props.is_empty() || !comp.events.is_empty() || !comp.children.is_empty() {
        return Err(UiSchemaError::InvalidControl(
            "ForEach 不使用 props/events/children".to_owned(),
        ));
    }
    let Some(Binding::State { bind }) = &comp.items else {
        return Err(UiSchemaError::InvalidControl(
            "ForEach 需要 items 数组 State 绑定".to_owned(),
        ));
    };
    let segs = path::PathSegments::parse(bind)?;
    let target = path::resolve(state_schema, segs.segments())?;
    let JsonSchema::Array { items, .. } = target else {
        return Err(UiSchemaError::BindingTypeMismatch(format!(
            "ForEach.items 必须绑定 array，实际 {}",
            path::json_type(target)
        )));
    };
    if comp.key.is_none() {
        return Err(UiSchemaError::InvalidControl("ForEach 需要 key".to_owned()));
    }
    let Some(template) = &comp.template else {
        return Err(UiSchemaError::InvalidControl(
            "ForEach 需要 template".to_owned(),
        ));
    };
    validate_component(template, state_schema, Some(items), ctx, depth + 1)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{EmittedEvent, StateSchema};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn clock_schema() -> JsonSchema {
        JsonSchema::Object {
            required: vec!["time".into(), "running".into(), "zones".into()],
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
        }
    }

    fn text(text: PropValue) -> Component {
        Component {
            kind: "Text".into(),
            props: BTreeMap::from([("text".into(), text)]),
            children: vec![],
            events: BTreeMap::new(),
            when: None,
            then: None,
            else_: None,
            items: None,
            key: None,
            template: None,
        }
    }

    fn valid_doc() -> UiDocument {
        UiDocument {
            ui_api_version: crate::UI_API_VERSION.into(),
            state: StateSchema {
                initial: json!({"time": "--:--:--", "running": false, "zones": []}),
                schema: clock_schema(),
            },
            events: BTreeMap::new(),
            root: Component {
                kind: "Column".into(),
                props: BTreeMap::new(),
                children: vec![text(PropValue::Binding(Binding::State {
                    bind: "$.time".into(),
                }))],
                events: BTreeMap::new(),
                when: None,
                then: None,
                else_: None,
                items: None,
                key: None,
                template: None,
            },
        }
    }

    #[test]
    fn accepts_valid_minimal_clock() {
        assert!(validate_document(&valid_doc()).is_ok());
    }

    #[test]
    fn rejects_unknown_component() {
        let mut doc = valid_doc();
        doc.root.kind = "Canvas".into();
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::UnknownComponent(_))
        ));
    }

    #[test]
    fn rejects_unknown_prop() {
        let mut doc = valid_doc();
        doc.root
            .props
            .insert("nope".into(), PropValue::Literal(json!(1)));
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::UnknownProp { .. })
        ));
    }

    #[test]
    fn rejects_binding_to_missing_state_field() {
        let mut doc = valid_doc();
        doc.root.children[0] = text(PropValue::Binding(Binding::State {
            bind: "$.nope".into(),
        }));
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::InvalidBindingPath(_))
        ));
    }

    #[test]
    fn rejects_type_mismatched_binding() {
        // Text.text 期望 string；绑定到 boolean running。
        let mut doc = valid_doc();
        doc.root.children[0] = text(PropValue::Binding(Binding::State {
            bind: "$.running".into(),
        }));
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::BindingTypeMismatch(_))
        ));
    }

    #[test]
    fn rejects_missing_required_prop() {
        let mut doc = valid_doc();
        // 移除 Text.text。
        doc.root.children[0].props.clear();
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::MissingProp { .. })
        ));
    }

    #[test]
    fn rejects_malformed_binding_path() {
        let mut doc = valid_doc();
        doc.root.children[0] = text(PropValue::Binding(Binding::State {
            bind: "time".into(),
        }));
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::InvalidBindingPath(_))
        ));
    }

    #[test]
    fn rejects_binding_on_non_bindable_prop() {
        // Column.padding 不允许绑定。
        let mut doc = valid_doc();
        doc.root.props.insert(
            "padding".into(),
            PropValue::Binding(Binding::State {
                bind: "$.time".into(),
            }),
        );
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::BindingTypeMismatch(_))
        ));
    }

    #[test]
    fn rejects_invalid_initial_state() {
        let mut doc = valid_doc();
        doc.state.initial = json!({"time": 5, "running": true, "zones": []});
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::InvalidState(_))
        ));
    }

    #[test]
    fn rejects_unsupported_api_version() {
        let mut doc = valid_doc();
        doc.ui_api_version = "2.0.0".into();
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
    }

    #[test]
    fn accepts_foreach_with_item_binding() {
        let mut doc = valid_doc();
        doc.root.children = vec![Component {
            kind: "ForEach".into(),
            props: BTreeMap::new(),
            children: vec![],
            events: BTreeMap::new(),
            when: None,
            then: None,
            else_: None,
            items: Some(Binding::State {
                bind: "$.zones".into(),
            }),
            key: Some("value".into()),
            template: Some(Box::new(text(PropValue::Binding(Binding::Item {
                item: "value".into(),
            })))),
        }];
        assert!(validate_document(&doc).is_ok());
    }

    #[test]
    fn rejects_item_binding_outside_foreach() {
        let mut doc = valid_doc();
        doc.root.children[0] = text(PropValue::Binding(Binding::Item {
            item: "value".into(),
        }));
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::InvalidItemBinding(_))
        ));
    }

    #[test]
    fn accepts_button_with_declared_event() {
        let mut doc = valid_doc();
        doc.events = BTreeMap::from([(
            "toggle".into(),
            EventSchema {
                payload: JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::new(),
                    additional_properties: false,
                },
            },
        )]);
        doc.root.children = vec![Component {
            kind: "Button".into(),
            props: BTreeMap::from([("label".into(), PropValue::Literal(json!("Go")))]),
            children: vec![],
            events: BTreeMap::from([(
                "activate".into(),
                EmittedEvent {
                    emit: "toggle".into(),
                    payload: json!({}),
                },
            )]),
            when: None,
            then: None,
            else_: None,
            items: None,
            key: None,
            template: None,
        }];
        assert!(validate_document(&doc).is_ok());
    }

    #[test]
    fn rejects_emit_of_undeclared_event() {
        let mut doc = valid_doc();
        doc.root.children = vec![Component {
            kind: "Button".into(),
            props: BTreeMap::from([("label".into(), PropValue::Literal(json!("Go")))]),
            children: vec![],
            events: BTreeMap::from([(
                "activate".into(),
                EmittedEvent {
                    emit: "nope".into(),
                    payload: json!({}),
                },
            )]),
            when: None,
            then: None,
            else_: None,
            items: None,
            key: None,
            template: None,
        }];
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::UnknownEvent(_))
        ));
    }

    #[test]
    fn rejects_undeclared_input_event_on_component() {
        let mut doc = valid_doc();
        doc.root.children[0].events = BTreeMap::from([(
            "hover".into(),
            EmittedEvent {
                emit: "toggle".into(),
                payload: json!({}),
            },
        )]);
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::UnknownInputEvent { .. })
        ));
    }

    #[test]
    fn enforces_node_budget() {
        let mut doc = valid_doc();
        // 制造 300 个 Text 子组件。
        doc.root.children = (0..300)
            .map(|i| text(PropValue::Literal(json!(i.to_string()))))
            .collect();
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::LimitExceeded(_))
        ));
    }

    #[test]
    fn enforces_tree_depth() {
        let mut doc = valid_doc();
        // 嵌套 40 层 Column。
        let mut node = text(PropValue::Literal(json!("x")));
        for _ in 0..40 {
            node = Component {
                kind: "Column".into(),
                props: BTreeMap::new(),
                children: vec![node],
                events: BTreeMap::new(),
                when: None,
                then: None,
                else_: None,
                items: None,
                key: None,
                template: None,
            };
        }
        doc.root = node;
        assert!(matches!(
            validate_document(&doc),
            Err(UiSchemaError::LimitExceeded(_))
        ));
    }
    // ---- uiApiVersion 版本轴与契约正反例向量 ----
    //
    // 这些向量是 host/CLI/Rust/TypeScript 共用的稳定契约基准:相同文档在
    // 各语言 validate_document 必须得到相同 pass/fail/code。新增/删除组件、
    // prop、事件或版本语义变更时,必须同步本表与
    // `docs/plugin-sdk/ui-ir-v1.md` §13 contract tests。

    fn version_doc(version: &str) -> UiDocument {
        let mut d = valid_doc();
        d.ui_api_version = version.into();
        d
    }

    fn with_single(root: Component) -> UiDocument {
        let mut d = valid_doc();
        d.root.children = vec![root];
        d
    }

    #[test]
    fn contract_vector_ui_api_version_axis() {
        // 正例:当前支持的 major 1(含 minor/patch 与预发布——版本轴按 major 匹配)。
        assert!(validate_document(&version_doc("1.0.0")).is_ok());
        assert!(validate_document(&version_doc("1.2.3")).is_ok());
        assert!(validate_document(&version_doc("1.0.0-rc.1")).is_ok());
        // 反例:major 不匹配(2.x)拒绝。
        assert!(matches!(
            validate_document(&version_doc("2.0.0")),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
        assert!(matches!(
            validate_document(&version_doc("2.1.0-beta")),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
        // 反例:非版本结构。
        assert!(matches!(
            validate_document(&version_doc("banana")),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
    }

    #[test]
    fn contract_vector_component_minor_gate() {
        let badge = Component {
            kind: "Badge".into(),
            props: BTreeMap::from([("label".into(), PropValue::Literal(json!("ok")))]),
            ..Default::default()
        };
        let mut old = with_single(badge.clone());
        old.ui_api_version = "1.0.0".into();
        assert!(matches!(
            validate_document(&old),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
        let mut current = with_single(badge);
        current.ui_api_version = "1.1.0".into();
        assert!(validate_document(&current).is_ok());
    }

    #[test]
    fn contract_vector_list_items_minor_and_budget_gate() {
        let list = Component {
            kind: "List".into(),
            props: BTreeMap::from([(
                "items".into(),
                PropValue::Binding(Binding::State {
                    bind: "$.zones".into(),
                }),
            )]),
            ..Default::default()
        };
        let mut old = with_single(list.clone());
        old.ui_api_version = "1.1.0".into();
        assert!(matches!(
            validate_document(&old),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));
        let current = with_single(list.clone());
        assert!(validate_document(&current).is_ok());

        let mut unbounded = with_single(list);
        if let JsonSchema::Object { properties, .. } = &mut unbounded.state.schema {
            properties.insert(
                "zones".into(),
                JsonSchema::Array {
                    max_items: None,
                    items: Box::new(JsonSchema::String {
                        max_length: Some(64),
                    }),
                },
            );
        }
        assert!(matches!(
            validate_document(&unbounded),
            Err(UiSchemaError::BindingTypeMismatch(_))
        ));
    }

    #[test]
    fn contract_vector_sparkline_requires_bounded_numbers_and_label() {
        let sparkline = Component {
            kind: "Sparkline".into(),
            props: BTreeMap::from([
                (
                    "values".into(),
                    PropValue::Binding(Binding::State {
                        bind: "$.trend".into(),
                    }),
                ),
                ("label".into(), PropValue::Literal(json!("Usage trend"))),
            ]),
            ..Default::default()
        };
        let mut current = with_single(sparkline.clone());
        if let JsonSchema::Object { properties, .. } = &mut current.state.schema {
            properties.insert(
                "trend".into(),
                JsonSchema::Array {
                    max_items: Some(16),
                    items: Box::new(JsonSchema::Number),
                },
            );
        }
        current.state.initial["trend"] = json!([]);
        assert!(validate_document(&current).is_ok());

        let mut old = current.clone();
        old.ui_api_version = "1.2.0".into();
        assert!(matches!(
            validate_document(&old),
            Err(UiSchemaError::UnsupportedApiVersion(_))
        ));

        let mut missing_label = current;
        missing_label.root.children[0].props.remove("label");
        assert!(matches!(
            validate_document(&missing_label),
            Err(UiSchemaError::MissingProp { .. })
        ));
    }

    #[test]
    fn contract_vector_positive_components() {
        // 正例:registry 元素组件结构合法。
        for kind in [
            "Text", "Button", "Toggle", "Progress", "Badge", "Gauge", "Icon", "Image",
        ] {
            let root = Component {
                kind: kind.into(),
                props: BTreeMap::new(),
                children: vec![],
                events: BTreeMap::new(),
                when: None,
                then: None,
                else_: None,
                items: None,
                key: None,
                template: None,
            };
            // 结构本身合法(必填 prop 缺失的组件会单独触发 MissingProp,非未知组件)。
            let result = validate_document(&with_single(root));
            match result {
                Ok(()) | Err(UiSchemaError::MissingProp { .. }) => {}
                Err(other) => panic!("{kind} 应合法或仅缺 prop,实际 {other:?}"),
            }
        }
        // 正例:控制组件 If/ForEach 结构合法。
        for kind in ["If", "ForEach"] {
            let root = Component {
                kind: kind.into(),
                props: BTreeMap::new(),
                children: vec![],
                events: BTreeMap::new(),
                when: Some(Binding::State {
                    bind: "$.running".into(),
                }),
                then: Some(Box::new(text(PropValue::Literal(json!("x"))))),
                else_: None,
                items: None,
                key: None,
                template: None,
            };
            let result = validate_document(&with_single(root));
            match result {
                Ok(()) | Err(UiSchemaError::InvalidControl(_)) => {}
                Err(other) => panic!("{kind} 应合法或控制结构错误,实际 {other:?}"),
            }
        }
    }

    #[test]
    fn contract_vector_negative_unknowns() {
        // 反例:未知组件/未知 prop/未知输入事件均拒绝。
        let root = Component {
            kind: "BogusComponent".into(),
            props: BTreeMap::new(),
            children: vec![],
            events: BTreeMap::new(),
            when: None,
            then: None,
            else_: None,
            items: None,
            key: None,
            template: None,
        };
        assert!(matches!(
            validate_document(&with_single(root)),
            Err(UiSchemaError::UnknownComponent(_))
        ));

        let mut d = valid_doc();
        d.root.children[0]
            .props
            .insert("not-a-prop".into(), PropValue::Literal(json!("x")));
        assert!(matches!(
            validate_document(&d),
            Err(UiSchemaError::UnknownProp { .. })
        ));
    }
}
