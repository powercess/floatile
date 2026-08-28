//! 运行时第三方插件 UI 渲染（ADR-0002 实现切片）。
//!
//! 把已安装插件的 `widget.ftui` 在**运行时**（非构建期）经 `slint-interpreter`
//! 编译成独立原生窗口，复用 `floatile-platform` 的窗口能力（无边框/透明/置顶），
//! 并沿 renderer 的 binding 槽位把 runtime 的权威 State 投影到窗口属性、把声明
//! 的输入事件经 callback 回投给 runtime。这是 FR-PLUGIN-01/F11/F12 的「运行时插件
//! UI 渲染链」闭合点；参考时钟仍保留为 `slint!` 构建期基线（S1–S4 平台窗口证据
//! 依赖它），两条路径共享 renderer 生成契约。
//!
//! 安全边界（NFR-SEC-01/02）：
//! - interpreter 编译的**唯一输入**是宿主 renderer 生成的受限源码，插件永不提供
//!   `.slint`；interpreter 不被当作不受信任源码编译器。
//! - `widget.ftui` 先经 `MAX_IR_BYTES` 字节预算，再 `validate_document` 结构/预算
//!   复验，再由 renderer 独立复验——恶意/超大/超深/未知绑定在到达 interpreter
//!   之前被拒，宿主存活（F12 前置）。
//! - 投影失败只记录，绝不 panic、不部分改写权威 State。

use std::path::PathBuf;
use std::sync::Arc;

use floatile_platform::{PlatformCapabilities, set_always_on_top};
use floatile_renderer::{
    BindingSlot, BindingValueType, EventSlot, RenderedComponent, render_component,
};
use floatile_ui_schema::path::PathSegments;
use floatile_ui_schema::{MAX_IR_BYTES, UiDocument, validate_document};
use serde_json::Value;
use slint::winit_030::WinitWindowAccessor;
use slint_interpreter::{Compiler, ComponentDefinition, ComponentInstance, Value as UiValue};

/// renderer 声明的宿主组件名（单一事实源，随 renderer 命名演进）。
pub const PLUGIN_COMPONENT_NAME: &str = "ClockPluginUI";

/// 运行时 UI 渲染错误。`code()` 返回稳定诊断 code（`RUI_*`），自由文本不作判断依据。
#[derive(Debug, thiserror::Error)]
pub enum RuntimeUiError {
    #[error("widget.ftui 超过字节预算 {0} (max {MAX_IR_BYTES})")]
    IrTooLarge(usize),
    #[error("widget.ftui 不是合法 JSON: {0}")]
    Parse(String),
    #[error("widget.ftui 未通过结构/预算校验: {0}")]
    InvalidIr(#[from] floatile_ui_schema::error::UiSchemaError),
    #[error("renderer 无法安全渲染: {0}")]
    Render(#[from] floatile_renderer::RendererError),
    #[error("interpreter 编译失败: {0}")]
    Compile(String),
    #[error("编译产物缺少插件组件 `{PLUGIN_COMPONENT_NAME}`")]
    MissingComponent,
    #[error("组件实例化失败: {0}")]
    Instantiate(String),
    #[error("state 投影失败: {0}")]
    Projection(String),
    #[error("输入事件回调注册失败: {0}")]
    Callback(String),
    #[error("manifest 授权构造失败: {0}")]
    Grant(String),
    #[error("持久实例与已加载 Installation 不匹配: {0}")]
    InstanceIdentity(String),
    #[error("runtime 实例启动失败: {0}")]
    Runtime(String),
}

impl RuntimeUiError {
    /// 稳定诊断 code（`RUI_*`）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::IrTooLarge(_) => "RUI_IR_TOO_LARGE",
            Self::Parse(_) => "RUI_PARSE",
            Self::InvalidIr(_) => "RUI_INVALID_IR",
            Self::Render(_) => "RUI_RENDER",
            Self::Compile(_) => "RUI_COMPILE",
            Self::MissingComponent => "RUI_MISSING_COMPONENT",
            Self::Instantiate(_) => "RUI_INSTANTIATE",
            Self::Projection(_) => "RUI_PROJECTION",
            Self::Callback(_) => "RUI_CALLBACK",
            Self::Grant(_) => "RUI_GRANT",
            Self::InstanceIdentity(_) => "RUI_INSTANCE_IDENTITY",
            Self::Runtime(_) => "RUI_RUNTIME",
        }
    }
}

/// 解析 + 复验 + 渲染安装包里的 `widget.ftui`（运行时路径，双层预算）。
///
/// 通过即得到可供 interpreter 编译的宿主受限源码与 binding/event 槽位；失败返回
/// 稳定 `RUI_*` code，不达 interpreter（F12 前置拒绝）。
/// 解析 + 复验安装包里的 `widget.ftui`（运行时路径，双层预算）。
///
/// 通过即得到已通过 `validate_document` 的 IR 文档（供 renderer 渲染、供 orchestrator
/// 取 canonical initial State 与 State schema）；失败返回稳定 `RUI_*` code，不达
/// interpreter（F12 前置拒绝）。
pub fn parse_document(ui_bytes: &[u8]) -> Result<UiDocument, RuntimeUiError> {
    if ui_bytes.len() > MAX_IR_BYTES {
        return Err(RuntimeUiError::IrTooLarge(ui_bytes.len()));
    }
    let doc: UiDocument =
        serde_json::from_slice(ui_bytes).map_err(|e| RuntimeUiError::Parse(e.to_string()))?;
    validate_document(&doc)?;
    Ok(doc)
}

/// 解析 + 复验 + 渲染安装包里的 `widget.ftui`（`parse_document` → `render_component`）。
pub fn render_ftui(ui_bytes: &[u8]) -> Result<RenderedComponent, RuntimeUiError> {
    let doc = parse_document(ui_bytes)?;
    render_component(&doc).map_err(Into::into)
}

/// 运行时用 `slint-interpreter` 编译 renderer 生成的源码（ADR-0002 决策 A）。
///
/// 这个同步包装仅供无头单测与独立工具使用；生产启动路径调用
/// `compile_component_async`，由 Slint local executor 推进，不在事件循环中嵌套
/// `block_on`。
pub fn compile_component(
    rendered: &RenderedComponent,
) -> Result<ComponentDefinition, RuntimeUiError> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| RuntimeUiError::Compile(format!("tokio runtime: {e}")))?;
    runtime.block_on(compile_component_async(rendered))
}

/// 在当前 Slint local executor 编译宿主生成的有界源码，不嵌套阻塞 Tokio runtime。
/// Slint 1.17 的编译产物是 `!Send`，且无自定义 file loader 时该 future 实际会同步
/// 完成，因此这一步仍必须留在 UI executor；重型不受信任 IR 处理已在前一阶段移出。
async fn compile_component_async(
    rendered: &RenderedComponent,
) -> Result<ComponentDefinition, RuntimeUiError> {
    let compiler = Compiler::default();
    let compiled = compiler
        .build_from_source(rendered.source.clone(), PathBuf::from("runtime-plugin-ui"))
        .await;
    if compiled.has_errors() {
        let messages: Vec<String> = compiled.diagnostics().map(|d| format!("{d:?}")).collect();
        return Err(RuntimeUiError::Compile(messages.join("; ")));
    }
    compiled
        .component(PLUGIN_COMPONENT_NAME)
        .ok_or(RuntimeUiError::MissingComponent)
}

/// 输入事件回投 sink：`(事件名, payload_json)`，UI 线程调用、运行时 worker 投递。
type EventSink = Arc<dyn Fn(&str, String) + Send + Sync + 'static>;

/// 一个已实例化的运行时插件窗口。只能由 UI（事件循环）线程创建与操作。
///
/// 创建后沿 renderer binding 槽位把权威 State 投影为窗口属性；`weak()` 交出跨线程
/// 弱引用，供 runtime worker 经 `upgrade_in_event_loop` 在 UI 线程投影（匹配现有
/// 参考时钟的投递模型，Slint 主线程不阻塞、不接受不受信任同步工作）。
pub struct RuntimePluginWindow {
    instance: ComponentInstance,
    bindings: Vec<BindingSlot>,
}

impl RuntimePluginWindow {
    /// 在 UI 线程实例化编译产物，并应用宿主窗口能力。
    ///
    /// 无边框/透明/初始置顶由 `apply_window_options` 在 winit attrs hook 统一承担
    /// （同一事件循环创建的所有窗口均生效，含 interpreter 窗口）；这里按 ADR-0001
    /// S1 经验再显式应用一次置顶——Slint 会在组件属性同步时重写创建前的窗口级别，
    /// 需在原生窗口可用后再次调用。
    pub fn create_on_ui_thread(
        definition: &ComponentDefinition,
        bindings: Vec<BindingSlot>,
        caps: &PlatformCapabilities,
    ) -> Result<Self, RuntimeUiError> {
        use slint_interpreter::ComponentHandle;
        let instance = definition
            .create()
            .map_err(|e| RuntimeUiError::Instantiate(e.to_string()))?;
        let window = instance.window();
        let caps = *caps;
        let _ = window.with_winit_window(move |w: &slint::winit_030::winit::window::Window| {
            if let Err(error) = set_always_on_top(w, caps.always_on_top.is_available()) {
                tracing::warn!(%error, "runtime plugin window always-on-top apply failed");
            }
        });
        Ok(Self { instance, bindings })
    }

    /// 沿 renderer binding 槽位把权威 State 投影进本窗口（UI 线程）。
    ///
    /// 末位是尽力而为的展示投影：返回值供调用方记录，不回滚、不 panic、不部分改写
    /// runtime 的权威 State（NFR-SEC-01）。
    pub fn project_state(&self, state: &Value) -> Result<(), RuntimeUiError> {
        for slot in &self.bindings {
            let value = project_binding_value(slot, state)?.into_ui_value();
            self.instance
                .set_property(&slot.prop, value)
                .map_err(|e| RuntimeUiError::Projection(format!("{}: {e}", slot.prop)))?;
        }
        Ok(())
    }

    /// 注册输入事件回投：声明事件 → interpreter callback → sink。
    ///
    /// `sink(name, payload_json)` 在 UI 线程被调用；调用方（runtime worker）负责把
    /// 事件投递到插件实例（`WidgetHandle::handle_event`）。事件槽位由 renderer 生成，
    /// 事件名由 renderer 决定，不把插件自由文本拼进回调注册。
    pub fn register_events(
        &self,
        events: &[EventSlot],
        sink: EventSink,
    ) -> Result<(), RuntimeUiError> {
        for slot in events {
            let event_name = slot.event.clone();
            let callback_name = slot.callback.clone();
            let sink = Arc::clone(&sink);
            self.instance
                .set_callback(&callback_name, move |args: &[UiValue]| {
                    let payload_json = serde_json::to_string(&serialize_callback_args(args))
                        .unwrap_or_else(|_| "[]".to_owned());
                    sink(&event_name, payload_json);
                    UiValue::Void
                })
                .map_err(|e| RuntimeUiError::Callback(format!("{}: {e:?}", slot.callback)))?;
        }
        Ok(())
    }

    /// 跨线程弱引用：`slint::Weak<ComponentInstance>` 是 Send，可交给 worker 投影。
    pub fn weak(&self) -> slint::Weak<ComponentInstance> {
        use slint_interpreter::ComponentHandle;
        self.instance.as_weak()
    }

    /// 底层 interpreter 实例句柄（宿主内部；用于投影校验与事件触发，不暴露给插件）。
    pub fn instance(&self) -> &ComponentInstance {
        &self.instance
    }
}

#[derive(Debug)]
enum ProjectedValue {
    String(String),
    Boolean(bool),
    Number(f64),
    StringList(Vec<String>),
    NumberList(Vec<f64>),
}

impl ProjectedValue {
    fn into_ui_value(self) -> UiValue {
        match self {
            Self::String(value) => UiValue::String(value.into()),
            Self::Boolean(value) => UiValue::Bool(value),
            Self::Number(value) => UiValue::Number(value),
            Self::StringList(values) => UiValue::Model(slint::ModelRc::new(slint::VecModel::from(
                values
                    .into_iter()
                    .map(|value| UiValue::String(value.into()))
                    .collect::<Vec<_>>(),
            ))),
            Self::NumberList(values) => UiValue::Model(slint::ModelRc::new(slint::VecModel::from(
                values.into_iter().map(UiValue::Number).collect::<Vec<_>>(),
            ))),
        }
    }
}

fn project_binding_value(
    slot: &BindingSlot,
    state: &Value,
) -> Result<ProjectedValue, RuntimeUiError> {
    let segments = PathSegments::parse(&slot.path)
        .map_err(|error| RuntimeUiError::Projection(format!("{}: {error}", slot.prop)))?;
    let mut current = state;
    for segment in segments.segments() {
        current = current.get(segment).ok_or_else(|| {
            RuntimeUiError::Projection(format!("{}: State 字段 `{segment}` 缺失", slot.prop))
        })?;
    }
    match slot.value_type {
        BindingValueType::String => current
            .as_str()
            .map(|value| ProjectedValue::String(value.to_owned()))
            .ok_or_else(|| RuntimeUiError::Projection(format!("{}: 期望 string", slot.prop))),
        BindingValueType::Boolean => current
            .as_bool()
            .map(ProjectedValue::Boolean)
            .ok_or_else(|| RuntimeUiError::Projection(format!("{}: 期望 boolean", slot.prop))),
        BindingValueType::Number => current
            .as_f64()
            .map(ProjectedValue::Number)
            .ok_or_else(|| RuntimeUiError::Projection(format!("{}: 期望 number", slot.prop))),
        BindingValueType::StringList => {
            let items = current.as_array().ok_or_else(|| {
                RuntimeUiError::Projection(format!("{}: 期望 string array", slot.prop))
            })?;
            if items.len() > floatile_ui_schema::MAX_LIST_ITEMS {
                return Err(RuntimeUiError::Projection(format!(
                    "{}: List 项数超过 {}",
                    slot.prop,
                    floatile_ui_schema::MAX_LIST_ITEMS
                )));
            }
            let values = items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_owned).ok_or_else(|| {
                        RuntimeUiError::Projection(format!("{}: List item 期望 string", slot.prop))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ProjectedValue::StringList(values))
        }
        BindingValueType::NumberList => {
            let items = current.as_array().ok_or_else(|| {
                RuntimeUiError::Projection(format!("{}: 期望 number array", slot.prop))
            })?;
            if items.len() > floatile_ui_schema::MAX_CHART_POINTS {
                return Err(RuntimeUiError::Projection(format!(
                    "{}: Sparkline 采样点超过 {}",
                    slot.prop,
                    floatile_ui_schema::MAX_CHART_POINTS
                )));
            }
            let values = items
                .iter()
                .map(|item| {
                    item.as_f64().ok_or_else(|| {
                        RuntimeUiError::Projection(format!(
                            "{}: Sparkline point 期望 number",
                            slot.prop
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(ProjectedValue::NumberList(values))
        }
    }
}

/// 把 interpreter 回调参数序列化为 JSON（供 `ui-event.payload-json`）。
///
/// Slint `Value` 大多可直接 JSON 序列化；不能序列化的标量走显式降级，绝不 panic。
fn serialize_callback_args(args: &[UiValue]) -> Vec<Value> {
    args.iter()
        .map(|value| match value {
            UiValue::String(s) => Value::String(s.to_string()),
            UiValue::Number(n) => serde_json::Number::from_f64(*n)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            UiValue::Bool(b) => Value::Bool(*b),
            UiValue::Void => Value::Null,
            other => Value::String(format!("{other:?}")),
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use floatile_ui_schema::ir::{Component, PropValue};
    use floatile_ui_schema::schema::JsonSchema;
    use std::collections::BTreeMap;

    fn clock_ftui() -> UiDocument {
        UiDocument {
            ui_api_version: floatile_ui_schema::UI_API_VERSION.into(),
            state: floatile_ui_schema::ir::StateSchema {
                initial: serde_json::json!({"time": "00:00:00", "running": false}),
                schema: JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::from([
                        (
                            "time".into(),
                            JsonSchema::String {
                                max_length: Some(32),
                            },
                        ),
                        ("running".into(), JsonSchema::Boolean),
                    ]),
                    additional_properties: false,
                },
            },
            events: BTreeMap::new(),
            root: Component {
                kind: "Column".into(),
                children: vec![Component {
                    kind: "Text".into(),
                    props: BTreeMap::from([(
                        "text".into(),
                        PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                            bind: "$.time".into(),
                        }),
                    )]),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    fn ftui_bytes(doc: &UiDocument) -> Vec<u8> {
        serde_json::to_vec(doc).unwrap()
    }

    #[test]
    fn render_ftui_rejects_oversized_ir() {
        // F12 前置：超过 MAX_IR_BYTES 的 ftui 在解析前即被拒，到达不了 interpreter。
        let huge = vec![b' '; MAX_IR_BYTES + 1];
        let err = render_ftui(&huge).unwrap_err();
        assert_eq!(err.code(), "RUI_IR_TOO_LARGE");
    }

    #[test]
    fn render_ftui_rejects_invalid_json() {
        let err = render_ftui(b"not-json").unwrap_err();
        assert_eq!(err.code(), "RUI_PARSE");
    }

    #[test]
    fn render_ftui_rejects_unknown_binding() {
        // 绑定指向未声明的 State 路径：schema 校验拒绝，宿主存活。
        let mut doc = clock_ftui();
        doc.root = Component {
            kind: "Text".into(),
            props: BTreeMap::from([(
                "text".into(),
                PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                    bind: "$.nonexistent".into(),
                }),
            )]),
            ..Default::default()
        };
        let err = render_ftui(&ftui_bytes(&doc)).unwrap_err();
        assert_eq!(err.code(), "RUI_INVALID_IR");
    }

    #[test]
    fn render_ftui_rejects_excessive_depth() {
        // 超过 MAX_TREE_DEPTH 的嵌套树：预算拒绝。
        let mut node = Component {
            kind: "Column".into(),
            children: vec![],
            ..Default::default()
        };
        for _ in 0..(floatile_ui_schema::MAX_TREE_DEPTH + 2) {
            node = Component {
                kind: "Column".into(),
                children: vec![node],
                ..Default::default()
            };
        }
        let mut doc = clock_ftui();
        doc.root = node;
        let err = render_ftui(&ftui_bytes(&doc)).unwrap_err();
        assert_eq!(err.code(), "RUI_INVALID_IR");
    }

    #[test]
    fn render_ftui_produces_expected_component() {
        let rendered = render_ftui(&ftui_bytes(&clock_ftui())).unwrap();
        assert_eq!(rendered.bindings.len(), 1);
        assert_eq!(rendered.bindings[0].path, "$.time");
        assert!(rendered.source.contains(PLUGIN_COMPONENT_NAME));
    }

    #[test]
    fn typed_projection_preserves_boolean_and_number_values() {
        let state = serde_json::json!({
            "loading": true,
            "percent": 42.5,
            "items": ["one", "two"],
            "trend": [10.0, 42.5]
        });
        let boolean = BindingSlot {
            path: "$.loading".into(),
            prop: "prop_loading".into(),
            value_type: BindingValueType::Boolean,
        };
        let number = BindingSlot {
            path: "$.percent".into(),
            prop: "prop_percent".into(),
            value_type: BindingValueType::Number,
        };
        let list = BindingSlot {
            path: "$.items".into(),
            prop: "prop_items".into(),
            value_type: BindingValueType::StringList,
        };
        let trend = BindingSlot {
            path: "$.trend".into(),
            prop: "prop_trend".into(),
            value_type: BindingValueType::NumberList,
        };
        assert!(matches!(
            project_binding_value(&boolean, &state).unwrap(),
            ProjectedValue::Boolean(true)
        ));
        assert!(matches!(
            project_binding_value(&number, &state).unwrap(),
            ProjectedValue::Number(value) if value == 42.5
        ));
        assert!(matches!(
            project_binding_value(&list, &state).unwrap(),
            ProjectedValue::StringList(values) if values == ["one", "two"]
        ));
        assert!(matches!(
            project_binding_value(&list, &state)
                .unwrap()
                .into_ui_value(),
            UiValue::Model(_)
        ));
        assert!(matches!(
            project_binding_value(&trend, &state).unwrap(),
            ProjectedValue::NumberList(values) if values == [10.0, 42.5]
        ));
    }

    #[test]
    fn compile_component_succeeds_without_display() {
        // ADR-0002 核心断言：纯编译不需要窗口 backend，无头 CI 可直接跑。
        let rendered = render_ftui(&ftui_bytes(&clock_ftui())).unwrap();
        let definition = compile_component(&rendered)
            .unwrap_or_else(|error| panic!("{error}\n{}", rendered.source));
        assert_eq!(definition.name(), PLUGIN_COMPONENT_NAME);
    }

    #[test]
    fn compile_typed_page_state_and_metrics_without_display() {
        let mut document = clock_ftui();
        document.state.initial = serde_json::json!({
            "time": "ok",
            "running": true,
            "percent": 42.5,
            "items": ["one", "two"],
            "trend": [10.0, 42.5]
        });
        if let JsonSchema::Object { properties, .. } = &mut document.state.schema {
            properties.insert("percent".into(), JsonSchema::Number);
            properties.insert(
                "items".into(),
                JsonSchema::Array {
                    max_items: Some(16),
                    items: Box::new(JsonSchema::String {
                        max_length: Some(64),
                    }),
                },
            );
            properties.insert(
                "trend".into(),
                JsonSchema::Array {
                    max_items: Some(16),
                    items: Box::new(JsonSchema::Number),
                },
            );
        }
        document.root = Component {
            kind: "Column".into(),
            children: vec![
                Component {
                    kind: "If".into(),
                    when: Some(floatile_ui_schema::ir::Binding::State {
                        bind: "$.running".into(),
                    }),
                    then: Some(Box::new(Component {
                        kind: "Badge".into(),
                        props: BTreeMap::from([
                            (
                                "label".into(),
                                PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                                    bind: "$.time".into(),
                                }),
                            ),
                            (
                                "tone".into(),
                                PropValue::Literal(serde_json::json!("success")),
                            ),
                        ]),
                        ..Default::default()
                    })),
                    ..Default::default()
                },
                Component {
                    kind: "Progress".into(),
                    props: BTreeMap::from([
                        (
                            "value".into(),
                            PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                                bind: "$.percent".into(),
                            }),
                        ),
                        (
                            "accessibilityLabel".into(),
                            PropValue::Literal(serde_json::json!("Completion")),
                        ),
                    ]),
                    ..Default::default()
                },
                Component {
                    kind: "List".into(),
                    props: BTreeMap::from([(
                        "items".into(),
                        PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                            bind: "$.items".into(),
                        }),
                    )]),
                    ..Default::default()
                },
                Component {
                    kind: "Grid".into(),
                    props: BTreeMap::from([(
                        "columns".into(),
                        PropValue::Literal(serde_json::json!(2)),
                    )]),
                    children: vec![
                        Component {
                            kind: "Text".into(),
                            props: BTreeMap::from([(
                                "text".into(),
                                PropValue::Literal(serde_json::json!("one")),
                            )]),
                            ..Default::default()
                        },
                        Component {
                            kind: "Text".into(),
                            props: BTreeMap::from([(
                                "text".into(),
                                PropValue::Literal(serde_json::json!("two")),
                            )]),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                Component {
                    kind: "Sparkline".into(),
                    props: BTreeMap::from([
                        (
                            "values".into(),
                            PropValue::Binding(floatile_ui_schema::ir::Binding::State {
                                bind: "$.trend".into(),
                            }),
                        ),
                        (
                            "label".into(),
                            PropValue::Literal(serde_json::json!("Usage trend")),
                        ),
                    ]),
                    ..Default::default()
                },
                Component {
                    kind: "Responsive".into(),
                    props: BTreeMap::from([(
                        "breakpoint".into(),
                        PropValue::Literal(serde_json::json!(420)),
                    )]),
                    children: vec![Component {
                        kind: "Text".into(),
                        props: BTreeMap::from([(
                            "text".into(),
                            PropValue::Literal(serde_json::json!("responsive")),
                        )]),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let rendered = render_ftui(&ftui_bytes(&document)).unwrap();
        let definition = compile_component(&rendered)
            .unwrap_or_else(|error| panic!("{error}\n{}", rendered.source));
        assert_eq!(definition.name(), PLUGIN_COMPONENT_NAME);
    }

    #[tokio::test]
    async fn runtime_ui_preparation_runs_off_the_calling_thread() {
        let caller = thread::current().id();
        let prepared = prepare_runtime_ui(ftui_bytes(&clock_ftui())).await.unwrap();
        assert_ne!(prepared.worker_thread, caller);
        assert_eq!(prepared.rendered.bindings.len(), 1);
    }
}
// ---- 运行时插件窗口编排（ADR-0002 接线层；main() 与集成测试共用）----

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use floatile_core::PluginInstance;
use floatile_core::capability::{
    CapabilityId, EffectiveGrant, Grant, Grants, InstanceGrant, TrustLevel, narrow_instance,
    parse_capability_params,
};
use floatile_core::instance::InstallationRef;
use floatile_core::manifest::Manifest;
use floatile_core::types::{InstanceId, PluginId};
use floatile_plugin_api::exports::floatile::widget::widget_contract::{UiEvent, WidgetEvent};
use floatile_runtime::{WidgetConfig, WidgetManager};
use floatile_services::{
    AuditEvent, AuditListener, CredentialVault, HttpsService, ReqwestHttpTransport,
};

use crate::plugin_manager::InstalledPlugin;

struct PreparedRuntimeUi {
    rendered: RenderedComponent,
    initial_state: Value,
    state_schema: floatile_ui_schema::schema::JsonSchema,
    #[cfg(test)]
    worker_thread: thread::ThreadId,
}

/// 在专用后台线程解析、校验并渲染不受信任 FTUI。tokio oneshot 的 Receiver 可由
/// Slint local executor 直接 await，不要求调用线程存在 Tokio runtime。
async fn prepare_runtime_ui(ui_bytes: Vec<u8>) -> Result<PreparedRuntimeUi, RuntimeUiError> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    thread::Builder::new()
        .name("floatile-ui-prepare".to_owned())
        .spawn(move || {
            let result = (|| {
                let doc = parse_document(&ui_bytes)?;
                let rendered = render_component(&doc).map_err(RuntimeUiError::Render)?;
                Ok(PreparedRuntimeUi {
                    rendered,
                    initial_state: doc.state.initial,
                    state_schema: doc.state.schema,
                    #[cfg(test)]
                    worker_thread: thread::current().id(),
                })
            })();
            let _ = result_tx.send(result);
        })
        .map_err(|error| RuntimeUiError::Runtime(format!("启动 UI 准备线程失败: {error}")))?;
    result_rx
        .await
        .map_err(|_| RuntimeUiError::Runtime("UI 准备线程未返回结果".to_owned()))?
}

/// UI 回调到 runtime worker 的事件队列上限。UI 线程只做一次 `try_send`。
const EVENT_QUEUE_CAPACITY: usize = 64;
/// 每轮最多转发的事件数，确保持续输入下 State 投影仍有调度机会。
const EVENT_BATCH_SIZE: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventEnqueueOutcome {
    Enqueued,
    DroppedFull,
    Closed,
}

struct EventBridgeSender {
    sender: mpsc::SyncSender<(String, String)>,
    dropped: Arc<AtomicU64>,
}

impl EventBridgeSender {
    fn new(sender: mpsc::SyncSender<(String, String)>, dropped: Arc<AtomicU64>) -> Self {
        Self { sender, dropped }
    }

    fn try_send(&self, name: &str, payload: String) -> EventEnqueueOutcome {
        match self.sender.try_send((name.to_owned(), payload)) {
            Ok(()) => EventEnqueueOutcome::Enqueued,
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::AcqRel);
                EventEnqueueOutcome::DroppedFull
            }
            Err(mpsc::TrySendError::Disconnected(_)) => EventEnqueueOutcome::Closed,
        }
    }
}

/// 在 worker 线程聚合记录过载，避免 UI 回调同步调用持久化 listener。
fn flush_event_overload_audit(
    plugin_id: &str,
    instance: InstanceId,
    dropped: &AtomicU64,
    listener: Option<&AuditListener>,
) {
    let count = dropped.swap(0, Ordering::AcqRel);
    if count == 0 {
        return;
    }
    let event = AuditEvent {
        plugin: plugin_id.to_owned(),
        instance: instance.0,
        capability: "ui:event-queue".to_owned(),
        decision: "deny".to_owned(),
        reason: Some("QueueFull".to_owned()),
        detail: format!("dropped={count}"),
    };
    if let Some(listener) = listener {
        listener(&event);
    }
    tracing::event!(
        target: "floatile::audit",
        tracing::Level::INFO,
        plugin_id,
        instance_id = instance.0,
        capability = "ui:event-queue",
        decision = "deny",
        reason = "QueueFull",
        detail = %event.detail,
    );
}

/// 一个已启动的运行时插件窗口会话：持有窗口（drop 即关闭）与投影/事件 worker。
///
/// 必须在 UI（事件循环）线程构造（`create_on_ui_thread`）；构造后 worker 线程负责
/// runtime（Wasmtime actor）与沿 binding 槽位的 State 投影、输入事件回投。
/// 会话须保持存活至程序结束，否则窗口关闭。
pub struct RuntimeUiSession {
    /// 保持窗口存活。
    _window: RuntimePluginWindow,
    stop: mpsc::SyncSender<()>,
    lifecycle: mpsc::Receiver<RuntimeUiLifecycleEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

/// runtime worker 向 Slint 线程发布的 observed lifecycle。错误码稳定，detail 只用于
/// 宿主诊断；控制面不得依赖自由文本判断状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeUiLifecycleEvent {
    Running,
    Failed { code: &'static str, detail: String },
    Stopped,
}

impl RuntimeUiSession {
    /// 非阻塞读取一个 lifecycle 事件。Slint timer 每轮有界调用，不等待 worker。
    pub fn try_lifecycle_event(&self) -> Option<RuntimeUiLifecycleEvent> {
        self.lifecycle.try_recv().ok()
    }
}

impl Drop for RuntimeUiSession {
    fn drop(&mut self) {
        // 会话结束：UI 线程只做非阻塞停止通知；join 转交 reaper。窗口随 `_window` 关闭。
        if let Some(worker) = self.worker.take() {
            reap_runtime_worker(&self.stop, worker);
        }
    }
}

fn reap_runtime_worker(stop: &mpsc::SyncSender<()>, worker: thread::JoinHandle<()>) {
    let _ = stop.try_send(());
    if let Err(error) = thread::Builder::new()
        .name("floatile-runtime-reaper".to_owned())
        .spawn(move || {
            let _ = worker.join();
        })
    {
        // spawn 失败会 drop 闭包并分离原 worker；仍不得回到 UI 线程同步 join。
        tracing::warn!(%error, "failed to spawn runtime worker reaper");
    }
}

/// 从 manifest 声明能力构造实例授权（单一来源：`parse_capability_params` 校验）。
/// 未知能力跳过并告警（deny-by-default，该能力在运行时被 Broker 拒绝）。P0 安装包
/// 在 CLI 层已校验能力，`from_name` 对合法包恒为 `Some`。
fn manifest_grants(
    id: &PluginId,
    manifest: &Manifest,
    instance: InstanceId,
) -> Result<InstanceGrant, RuntimeUiError> {
    let mut caps = Vec::new();
    for decl in &manifest.permissions {
        let Some(capability) = CapabilityId::from_name(&decl.capability) else {
            tracing::warn!(
                plugin_id = %id.0,
                capability = %decl.capability,
                "manifest 声明未知能力，跳过授权（运行时默认拒绝）"
            );
            continue;
        };
        let params = parse_capability_params(capability, decl.params.as_ref())
            .map_err(|e| RuntimeUiError::Grant(format!("{}: {e}", capability.name())))?;
        caps.push(Grant {
            capability,
            params,
            effective: EffectiveGrant::DerivedFromInstall,
        });
    }
    let plugin = Grants {
        plugin: id.clone(),
        caps: caps.clone(),
        trust: TrustLevel::Dev,
    };
    narrow_instance(&plugin, instance, caps).map_err(|e| RuntimeUiError::Grant(e.to_string()))
}

fn validate_runtime_instance(
    plugin: &InstalledPlugin,
    instance: &PluginInstance,
) -> Result<(InstanceId, u64, String), RuntimeUiError> {
    let actual_installation = InstallationRef::from_install_meta(&plugin.meta)
        .map_err(|error| RuntimeUiError::InstanceIdentity(error.to_string()))?;
    if actual_installation != *instance.installation() {
        return Err(RuntimeUiError::InstanceIdentity(format!(
            "instance {} expects {}@{}",
            instance.id().0,
            instance.installation().plugin().0,
            instance.installation().version()
        )));
    }
    let config_json = serde_json::to_string(instance.config())
        .map_err(|error| RuntimeUiError::Runtime(format!("序列化实例配置失败: {error}")))?;
    Ok((instance.id(), instance.generation(), config_json))
}

/// 启动一个已安装插件的运行时窗口（FR-PLUGIN-01/F11 运行时 UI 渲染链闭合）。
///
/// - 专用准备线程解析/复验/渲染 `widget.ftui`；Slint local executor 异步编译宿主
///   生成源码，然后只在 UI 线程实例化独立原生窗口、注册输入事件回投；
/// - 派 worker 线程运行 Wasmtime 实例，沿 renderer binding 槽位把权威 State 投影到
///   窗口（经 `Weak::upgrade_in_event_loop`，Slint 主线程不阻塞）、把声明事件回投给
///   实例。
///
/// 任一失败：本插件不启动，宿主与其插件存活（F12 隔离）；返回稳定 `RUI_*` code。
pub async fn spawn_runtime_ui(
    plugin: InstalledPlugin,
    instance: PluginInstance,
    caps: PlatformCapabilities,
    audit_listener: Option<AuditListener>,
) -> Result<RuntimeUiSession, RuntimeUiError> {
    spawn_runtime_ui_with_https(plugin, instance, caps, audit_listener, None).await
}

/// Compose only the Connections explicitly granted to this instance. The caller owns the vault;
/// no secret is read from SQLite or copied into plugin config/state.
pub fn compose_instance_https(
    store: &floatile_store::Store,
    instance: InstanceId,
    manifest: &Manifest,
    vault: Arc<dyn CredentialVault>,
) -> Result<HttpsService, RuntimeUiError> {
    let connection_store = store.connections();
    let grants = connection_store
        .grants_for_instance(instance)
        .map_err(|error| {
            RuntimeUiError::Runtime(format!("读取 Connection grants 失败: {error}"))
        })?;
    let mut connections = Vec::with_capacity(grants.len());
    for grant in grants {
        let connection = connection_store
            .get(grant.connection_id)
            .map_err(|error| RuntimeUiError::Runtime(format!("读取 Connection 失败: {error}")))?
            .ok_or_else(|| RuntimeUiError::Runtime("Connection grant 引用不存在".to_owned()))?;
        connections.push(connection);
    }
    Ok(HttpsService::new(
        manifest.http_templates.clone(),
        connections,
        vault,
        Arc::new(ReqwestHttpTransport),
    ))
}

pub async fn spawn_runtime_ui_with_https(
    plugin: InstalledPlugin,
    instance: PluginInstance,
    caps: PlatformCapabilities,
    audit_listener: Option<AuditListener>,
    https: Option<HttpsService>,
) -> Result<RuntimeUiSession, RuntimeUiError> {
    let id = plugin.manifest.id.clone();
    let (instance_id, generation, config_json) = validate_runtime_instance(&plugin, &instance)?;
    // 1. 后台解析 + 复验 + 渲染（双层预算，恶意 IR 在此被拒，不达 interpreter）。
    let prepared = prepare_runtime_ui(plugin.ui_bytes).await?;
    let rendered = prepared.rendered;
    let bindings = rendered.bindings.clone();
    let events = rendered.events.clone();
    let initial_state = prepared.initial_state;
    let state_schema = prepared.state_schema;

    // 2. interpreter 产物含 Rc、不可跨线程；在 Slint local executor 异步编译，随后
    // 在同一 UI 线程实例化窗口，不使用嵌套 block_on。
    let definition = compile_component_async(&rendered).await?;
    let window = RuntimePluginWindow::create_on_ui_thread(&definition, rendered.bindings, &caps)?;

    // 3. 输入事件回投通道：UI 线程 sink → worker 转发给实例。
    let (event_tx, event_rx) = mpsc::sync_channel::<(String, String)>(EVENT_QUEUE_CAPACITY);
    let dropped_events = Arc::new(AtomicU64::new(0));
    let event_bridge = EventBridgeSender::new(event_tx, Arc::clone(&dropped_events));
    let sink: EventSink = Arc::new(move |name: &str, payload: String| {
        let _ = event_bridge.try_send(name, payload);
    });
    window.register_events(&events, sink)?;

    // 4. 实例授权（manifest 是上限，实例只可收窄）。
    let grants = manifest_grants(&id, &plugin.manifest, instance_id)?;

    // 5. 派 worker 运行 runtime + 投影 + 事件回投。
    let (stop, stop_rx) = mpsc::sync_channel::<()>(1);
    let (lifecycle_tx, lifecycle_rx) = mpsc::sync_channel::<RuntimeUiLifecycleEvent>(4);
    let weak = window.weak();
    let wasm = plugin.wasm;
    let worker = thread::Builder::new()
        .name(format!("floatile-runtime-{}-{}", id.0, instance_id.0))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(error) => {
                    tracing::warn!(%error, "failed to build tokio runtime");
                    let _ = lifecycle_tx.try_send(RuntimeUiLifecycleEvent::Failed {
                        code: "RUI_RUNTIME",
                        detail: error.to_string(),
                    });
                    return;
                }
            };
            runtime.block_on(async move {
                let overload_audit_listener = audit_listener.clone();
                let manager = match WidgetManager::new() {
                    Ok(m) => m,
                    Err(error) => {
                        tracing::warn!(%error, "failed to create widget manager");
                        let _ = lifecycle_tx.try_send(RuntimeUiLifecycleEvent::Failed {
                            code: "RUI_RUNTIME",
                            detail: error.to_string(),
                        });
                        return;
                    }
                }
                .with_audit_listener(audit_listener);
                let config = WidgetConfig {
                    plugin: id.clone(),
                    instance: instance_id,
                    generation,
                    wasm,
                    initial_state,
                    state_schema,
                    config_json,
                    grants,
                };
                let mut handle = match manager.spawn_with_https(config, https) {
                    Ok(h) => h,
                    Err(error) => {
                        tracing::warn!(%error, "failed to spawn runtime widget");
                        let _ = lifecycle_tx.try_send(RuntimeUiLifecycleEvent::Failed {
                            code: "RUI_RUNTIME",
                            detail: error.to_string(),
                        });
                        return;
                    }
                };
                if let Err(error) = handle.start().await {
                    tracing::warn!(%error, "runtime widget start failed");
                    let _ = lifecycle_tx.try_send(RuntimeUiLifecycleEvent::Failed {
                        code: "RUI_RUNTIME",
                        detail: error.to_string(),
                    });
                    let _ = handle.shutdown().await;
                    return;
                }
                let _ = lifecycle_tx.try_send(RuntimeUiLifecycleEvent::Running);
                tracing::info!(plugin_id = %id.0, "runtime plugin window started");

                let mut stopped_by_host = false;
                let mut terminal_failure = None;
                loop {
                    flush_event_overload_audit(
                        &id.0,
                        instance_id,
                        &dropped_events,
                        overload_audit_listener.as_ref(),
                    );
                    if matches!(
                        stop_rx.try_recv(),
                        Ok(()) | Err(mpsc::TryRecvError::Disconnected)
                    ) {
                        stopped_by_host = true;
                        break;
                    }
                    // 事件回投：UI 线程 sink → 本实例 handle_event。
                    for _ in 0..EVENT_BATCH_SIZE {
                        let (name, payload_json) = match event_rx.try_recv() {
                            Ok(event) => event,
                            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                                break;
                            }
                        };
                        let event = WidgetEvent::Ui(UiEvent {
                            name: name.clone(),
                            payload_json,
                        });
                        if let Err(error) = handle.handle_event(event).await {
                            tracing::warn!(%error, event = %name, "event forwarding rejected");
                        }
                    }
                    let next = tokio::time::timeout(
                        Duration::from_millis(200),
                        handle.ui_updates().recv(),
                    )
                    .await;
                    let Some(update) = (match next {
                        Ok(update) => update,
                        Err(_) => continue,
                    }) else {
                        terminal_failure = Some((
                            "RUI_RUNTIME_CLOSED",
                            "runtime state update channel closed unexpectedly".to_owned(),
                        ));
                        break;
                    };
                    // 沿 renderer binding 槽位解析为类型化值，在 UI 线程投影。
                    let mut projections = Vec::new();
                    for slot in &bindings {
                        match project_binding_value(slot, &update.state) {
                            Ok(value) => projections.push((slot.prop.clone(), value)),
                            Err(error) => tracing::warn!(
                                seq = update.seq,
                                %error,
                                "runtime state rejected by shell projection"
                            ),
                        }
                    }
                    if projections.is_empty() {
                        continue;
                    }
                    let weak = weak.clone();
                    if let Err(error) = weak.upgrade_in_event_loop(move |instance| {
                        for (prop, value) in projections {
                            let _ = instance.set_property(&prop, value.into_ui_value());
                        }
                    }) {
                        tracing::debug!(%error, "event loop delivery failed; stopping bridge");
                        terminal_failure = Some(("RUI_UI_CLOSED", error.to_string()));
                        break;
                    }
                }

                flush_event_overload_audit(
                    &id.0,
                    instance_id,
                    &dropped_events,
                    overload_audit_listener.as_ref(),
                );

                if let Err(error) = handle.shutdown().await {
                    tracing::warn!(%error, "runtime widget shutdown failed");
                }
                let event = match terminal_failure {
                    Some((code, detail)) => RuntimeUiLifecycleEvent::Failed { code, detail },
                    None if stopped_by_host => RuntimeUiLifecycleEvent::Stopped,
                    None => RuntimeUiLifecycleEvent::Failed {
                        code: "RUI_RUNTIME_CLOSED",
                        detail: "runtime worker exited unexpectedly".to_owned(),
                    },
                };
                let _ = lifecycle_tx.try_send(event);
            });
        })
        .map_err(|e| RuntimeUiError::Runtime(e.to_string()))?;

    Ok(RuntimeUiSession {
        _window: window,
        stop,
        lifecycle: lifecycle_rx,
        worker: Some(worker),
    })
}

#[cfg(test)]
const TEST_INSTANCE_ID: InstanceId = InstanceId(1);

#[cfg(test)]
mod grants_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU64;

    fn installed_plugin() -> InstalledPlugin {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "manifestVersion": 1,
            "id": "dev.floatile.clock",
            "name": "World Clock",
            "version": "0.2.0",
            "engineApiVersion": "1.0.0",
            "uiApiVersion": "1.0.0",
            "type": "widget",
            "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
            "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
            "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
            "permissions": []
        }))
        .unwrap();
        InstalledPlugin {
            manifest,
            meta: floatile_core::install::InstallMeta {
                manifest_version: 1,
                id: "dev.floatile.clock".into(),
                version: "0.2.0".into(),
                engine_api_version: "1.0.0".into(),
                ui_api_version: "1.0.0".into(),
                installed_at: 0,
                source: "clock.floatile".into(),
                trust: floatile_core::install::InstallationTrust::Unsigned,
                files: std::collections::BTreeMap::new(),
                digest: "00".repeat(32),
            },
            wasm: Vec::new(),
            ui_bytes: Vec::new(),
        }
    }
    #[test]
    fn manifest_grants_derives_timer_capability() {
        let manifest: Manifest = serde_json::from_str(
                &serde_json::json!({
                    "manifestVersion": 1,
                    "id": "dev.floatile.clock",
                    "name": "World Clock",
                    "version": "0.2.0",
                    "engineApiVersion": "1.0.0",
                    "uiApiVersion": "1.0.0",
                    "type": "widget",
                    "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
                    "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
                    "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
                    "permissions": [{ "capability": "timer:schedule", "params": { "maxPerMinute": 30, "maxActive": 2 } }]
                })
                .to_string(),
            )
            .unwrap();
        let grants = manifest_grants(
            &PluginId("dev.floatile.clock".into()),
            &manifest,
            TEST_INSTANCE_ID,
        )
        .unwrap();
        assert_eq!(grants.instance, TEST_INSTANCE_ID);
        assert_eq!(grants.caps.len(), 1);
        assert_eq!(
            grants.caps[0].capability,
            CapabilityId::TimerSchedule,
            "manifest 声明应派生为 timer 授权"
        );
    }

    #[test]
    fn manifest_grants_skips_unknown_capability_deny_by_default() {
        let manifest: Manifest = serde_json::from_str(
                &serde_json::json!({
                    "manifestVersion": 1,
                    "id": "dev.floatile.clock",
                    "name": "World Clock",
                    "version": "0.2.0",
                    "engineApiVersion": "1.0.0",
                    "uiApiVersion": "1.0.0",
                    "type": "widget",
                    "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
                    "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
                    "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
                    "permissions": [{ "capability": "network:fetch", "params": null }]
                })
                .to_string(),
            )
            .unwrap();
        // 未知能力：跳过授权（运行时 Broker 拒绝），不阻止宿主加载。
        let grants = manifest_grants(
            &PluginId("dev.floatile.clock".into()),
            &manifest,
            TEST_INSTANCE_ID,
        )
        .unwrap();
        assert!(grants.caps.is_empty());
    }

    #[test]
    fn manifest_grants_rejects_invalid_params() {
        let manifest: Manifest = serde_json::from_str(
                &serde_json::json!({
                    "manifestVersion": 1,
                    "id": "dev.floatile.clock",
                    "name": "World Clock",
                    "version": "0.2.0",
                    "engineApiVersion": "1.0.0",
                    "uiApiVersion": "1.0.0",
                    "type": "widget",
                    "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
                    "publisher": { "id": "dev.floatile", "name": "Floatile Labs" },
                    "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
                    "permissions": [{ "capability": "timer:schedule", "params": { "bogus": 1 } }]
                })
                .to_string(),
            )
            .unwrap();
        let err = manifest_grants(
            &PluginId("dev.floatile.clock".into()),
            &manifest,
            TEST_INSTANCE_ID,
        )
        .unwrap_err();
        assert_eq!(err.code(), "RUI_GRANT");
    }

    #[test]
    fn event_bridge_drops_immediately_when_bounded_queue_is_full() {
        let (tx, rx) = mpsc::sync_channel(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let bridge = EventBridgeSender::new(tx, Arc::clone(&dropped));

        assert_eq!(
            bridge.try_send("tick", "[]".into()),
            EventEnqueueOutcome::Enqueued
        );
        assert_eq!(
            bridge.try_send("tick", "[]".into()),
            EventEnqueueOutcome::DroppedFull
        );
        assert_eq!(dropped.load(Ordering::Acquire), 1);
        assert_eq!(rx.try_recv().unwrap(), ("tick".into(), "[]".into()));
    }

    #[test]
    fn concurrent_event_flood_stays_within_queue_capacity() {
        const CAPACITY: usize = 4;
        const PRODUCERS: usize = 8;
        const EVENTS_PER_PRODUCER: usize = 32;
        let (tx, rx) = mpsc::sync_channel(CAPACITY);
        let dropped = Arc::new(AtomicU64::new(0));
        let bridge = Arc::new(EventBridgeSender::new(tx, Arc::clone(&dropped)));
        let barrier = Arc::new(std::sync::Barrier::new(PRODUCERS));

        let producers: Vec<_> = (0..PRODUCERS)
            .map(|producer| {
                let bridge = Arc::clone(&bridge);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    for event in 0..EVENTS_PER_PRODUCER {
                        let _ = bridge.try_send(&format!("p{producer}-{event}"), "[]".into());
                    }
                })
            })
            .collect();
        for producer in producers {
            producer.join().unwrap();
        }

        assert_eq!(rx.try_iter().count(), CAPACITY);
        assert_eq!(
            dropped.load(Ordering::Acquire),
            u64::try_from(PRODUCERS * EVENTS_PER_PRODUCER - CAPACITY).unwrap()
        );
    }

    #[test]
    fn event_bridge_flushes_aggregated_overload_audit_without_payloads() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&records);
        let listener: AuditListener = Arc::new(move |event| {
            captured.lock().unwrap().push(event.clone());
        });
        let dropped = AtomicU64::new(3);

        flush_event_overload_audit(
            "dev.floatile.clock",
            TEST_INSTANCE_ID,
            &dropped,
            Some(&listener),
        );

        let records = records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].capability, "ui:event-queue");
        assert_eq!(records[0].decision, "deny");
        assert_eq!(records[0].detail, "dropped=3");
        assert_eq!(dropped.load(Ordering::Acquire), 0);
    }

    #[test]
    fn runtime_worker_shutdown_does_not_join_on_calling_thread() {
        let (stop, stop_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let _ = stop_rx.recv();
            thread::sleep(Duration::from_millis(250));
        });
        let started = std::time::Instant::now();
        reap_runtime_worker(&stop, worker);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "UI-side shutdown must not wait for worker join"
        );
    }

    #[test]
    fn runtime_context_preserves_same_installation_instance_ids_and_configs() {
        let plugin = installed_plugin();
        let installation = InstallationRef::from_install_meta(&plugin.meta).unwrap();
        let first = PluginInstance::restore(
            InstanceId(2),
            installation.clone(),
            floatile_core::InstanceConfig::new(serde_json::json!({"timezone": "UTC"})).unwrap(),
            floatile_core::InstanceDesiredState::Running,
            1,
            0,
            0,
        )
        .unwrap();
        let second = PluginInstance::restore(
            InstanceId(3),
            installation,
            floatile_core::InstanceConfig::new(serde_json::json!({"timezone": "Asia/Shanghai"}))
                .unwrap(),
            floatile_core::InstanceDesiredState::Running,
            1,
            0,
            0,
        )
        .unwrap();

        let first_context = validate_runtime_instance(&plugin, &first).unwrap();
        let second_context = validate_runtime_instance(&plugin, &second).unwrap();

        assert_eq!(first_context.0, InstanceId(2));
        assert_eq!(second_context.0, InstanceId(3));
        assert_eq!(first_context.1, 1);
        assert_eq!(second_context.1, 1);
        assert_eq!(first_context.2, r#"{"timezone":"UTC"}"#);
        assert_eq!(second_context.2, r#"{"timezone":"Asia/Shanghai"}"#);
    }

    #[test]
    fn runtime_context_rejects_mismatched_installation_identity() {
        let plugin = installed_plugin();
        let different_installation = InstallationRef::new(
            PluginId("dev.floatile.clock".into()),
            "0.2.0",
            floatile_core::InstallationDigest::from_bytes([0xff; 32]),
        )
        .unwrap();
        let instance = PluginInstance::restore(
            InstanceId(2),
            different_installation,
            floatile_core::InstanceConfig::empty(),
            floatile_core::InstanceDesiredState::Running,
            1,
            0,
            0,
        )
        .unwrap();

        let error = validate_runtime_instance(&plugin, &instance).unwrap_err();
        assert_eq!(error.code(), "RUI_INSTANCE_IDENTITY");
    }
}
