//! IR → 宿主控制的 Slint 源码文本生成。
//!
//! 将已验证 `UiDocument` 递归展开为单个 `component PluginContent` 的 Slint 定义。
//! 输出除源码文本外,还给出:
//! - `bindings`:State 路径 → 生成的属性名(runtime 按下发 State 逐项 set property);
//! - `events`:输入事件名 → 生成的回调名(runtime 绑定 callback 转发 handle_event)。

use std::collections::BTreeMap;

use floatile_ui_schema::ir::{Binding, Component, PropValue, UiDocument};
use floatile_ui_schema::path::PathSegments;
use floatile_ui_schema::{MAX_BINDINGS, MAX_NODES, MAX_TREE_DEPTH, validate_document};

use crate::RendererError;

/// 一个 State 绑定:实例路径 → 生成的 Slint 属性名。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BindingSlot {
    /// 规范 JSONPath(`$.time`)。
    pub path: String,
    /// 生成的宿主属性名(如 `prop_time`)。
    pub prop: String,
    /// Slint 投影值类型；运行时必须按此类型转换权威 State。
    pub value_type: BindingValueType,
}

/// renderer 与 shell 共享的有限 State 投影类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingValueType {
    String,
    Boolean,
    Number,
    StringList,
    NumberList,
}

/// 一个输入事件:声明事件名 → 生成的回调名。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct EventSlot {
    /// 声明事件名(顶层 `events` 键,如 `toggle`)。
    pub event: String,
    /// 生成的回调名(如 `emit_toggle`)。
    pub callback: String,
}

/// 生成的组件文本与绑定/事件槽位。
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedComponent {
    /// `component PluginContent` 的 Slint 源码文本。
    pub source: String,
    /// 全部 State 绑定槽位(去重)。
    pub bindings: Vec<BindingSlot>,
    /// 全部输入事件槽位(去重)。
    pub events: Vec<EventSlot>,
}

/// 生成开始,先复验输入。
pub fn render_component(doc: &UiDocument) -> Result<RenderedComponent, RendererError> {
    // renderer 独立复验(CLI/runtime 通过不代表本层可跳过)。
    validate_document(doc)?;
    let mut ctx = Ctx {
        ui_minor: doc
            .ui_api_version
            .split('.')
            .nth(1)
            .and_then(|minor| minor.parse().ok())
            .unwrap_or_default(),
        ..Default::default()
    };
    let body = render_node(&doc.root, &mut ctx, 0)?;
    let bindings: Vec<BindingSlot> = ctx.bindings.into_values().collect();
    let events: Vec<EventSlot> = ctx.events.values().cloned().collect();
    let source = wrap_component(&body, &bindings, &events);
    Ok(RenderedComponent {
        source,
        bindings,
        events,
    })
}

/// 生成期上下文:属性/回调命名与预算复验。
#[derive(Default)]
struct Ctx {
    bindings: BTreeMap<String, BindingSlot>,
    events: BTreeMap<String, EventSlot>,
    nodes: usize,
    callback_counter: usize,
    ui_minor: u64,
}

fn wrap_component(body: &str, bindings: &[BindingSlot], callbacks: &[EventSlot]) -> String {
    let mut out = String::new();
    out.push_str("// 由 floatile-renderer 生成;宿主控制,勿手编。\n");
    // `export` 让宿主的 `slint!` 能 `import` 本组件并把权威 State 投影到其绑定槽位。
    out.push_str("export component ClockPluginUI inherits Rectangle {\n");
    for slot in bindings {
        let (kind, default) = match slot.value_type {
            BindingValueType::String => ("string", "\"\""),
            BindingValueType::Boolean => ("bool", "false"),
            BindingValueType::Number => ("float", "0"),
            BindingValueType::StringList => ("[string]", "[]"),
            BindingValueType::NumberList => ("[float]", "[]"),
        };
        out.push_str(&format!(
            "    in property <{kind}> {}: {default};\n",
            slot.prop
        ));
    }
    for slot in callbacks {
        out.push_str(&format!("    callback {};\n", slot.callback));
    }
    out.push_str("    background: transparent;\n");
    out.push_str(body);
    out.push_str("}\n");
    out
}

fn render_node(comp: &Component, ctx: &mut Ctx, depth: usize) -> Result<String, RendererError> {
    if depth > MAX_TREE_DEPTH {
        return Err(RendererError::BudgetExceeded(format!(
            "组件树深度 {depth} 超过 renderer 上限 {MAX_TREE_DEPTH}"
        )));
    }
    ctx.nodes += 1;
    if ctx.nodes > MAX_NODES {
        return Err(RendererError::BudgetExceeded(format!(
            "组件节点数 {} 超过 renderer 上限 {MAX_NODES}",
            ctx.nodes
        )));
    }
    if ctx.bindings.len() > MAX_BINDINGS {
        return Err(RendererError::BudgetExceeded(format!(
            "绑定数 {} 超过 renderer 上限 {MAX_BINDINGS}",
            ctx.bindings.len()
        )));
    }

    match comp.kind.as_str() {
        "Column" => render_layout(comp, ctx, depth, "VerticalLayout"),
        "Row" => render_layout(comp, ctx, depth, "HorizontalLayout"),
        "Stack" => render_layout(comp, ctx, depth, "StackLayout"),
        "Grid" => render_grid(comp, ctx, depth),
        "Responsive" => render_responsive(comp, ctx, depth),
        "List" => render_list(comp, ctx, depth),
        "Sparkline" => render_sparkline(comp, ctx),
        "Text" => render_text(comp, ctx),
        "Button" => render_button(comp, ctx),
        "Toggle" => render_toggle(comp, ctx),
        "Progress" => render_meter(comp, ctx, false),
        "Badge" => render_badge(comp, ctx),
        "Gauge" => render_meter(comp, ctx, true),
        "If" => render_if(comp, ctx, depth),
        "ForEach" => render_foreach(comp, ctx, depth),
        kind => Err(RendererError::UnsupportedComponent(
            kind.to_owned(),
            "renderer 暂不映射该组件;请移除或等待后续切片".to_owned(),
        )),
    }
}

/// Responsive:窄窗口纵向、宽窗口横向；分支互斥且复用同一受验证子树。
fn render_responsive(
    comp: &Component,
    ctx: &mut Ctx,
    depth: usize,
) -> Result<String, RendererError> {
    let breakpoint = comp
        .props
        .get("breakpoint")
        .and_then(|value| match value {
            PropValue::Literal(value) => value.as_f64(),
            PropValue::Binding(_) => None,
        })
        .ok_or_else(|| RendererError::BindingError("Responsive 缺少 breakpoint".to_owned()))?;
    let mut children = String::new();
    for child in &comp.children {
        children.push_str(&render_node(child, ctx, depth + 1)?);
        children.push('\n');
    }
    let props = render_layout_props(comp)?;
    Ok(format!(
        "if root.width < {breakpoint}px: VerticalLayout {{\n{props}{children}}}\nif root.width >= {breakpoint}px: HorizontalLayout {{\n{props}{children}}}\n"
    ))
}

fn render_sparkline(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    let label = match comp.props.get("label") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            format!(
                "root.{}",
                binding_slot(bind, BindingValueType::String, ctx)?.prop
            )
        }
        Some(PropValue::Literal(value)) => encode_string(value.as_str().ok_or_else(|| {
            RendererError::BindingError("Sparkline.label 必须是 string".to_owned())
        })?)?,
        _ => {
            return Err(RendererError::BindingError(
                "Sparkline 缺少 label prop".to_owned(),
            ));
        }
    };
    let color = match comp.props.get("tone") {
        Some(PropValue::Literal(value)) => match value.as_str() {
            Some("success") => "#247a45",
            Some("warning") => "#a86600",
            Some("danger") => "#a33a3a",
            Some("info") | None => "#2f6feb",
            Some(other) => {
                return Err(RendererError::BindingError(format!(
                    "Sparkline.tone 不支持 `{other}`"
                )));
            }
        },
        None => "#2f6feb",
        Some(PropValue::Binding(_)) => {
            return Err(RendererError::BindingError(
                "Sparkline.tone 不允许绑定".to_owned(),
            ));
        }
    };
    let bars = match comp.props.get("values") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let values = format!(
                "root.{}",
                binding_slot(bind, BindingValueType::NumberList, ctx)?.prop
            );
            format!(
                "        for value in {values}: Rectangle {{\n            width: 4px;\n            height: parent.height * ((value < 0 ? 0 : (value > 100 ? 100 : value)) / 100);\n            background: {color};\n        }}\n"
            )
        }
        Some(PropValue::Literal(value)) => {
            let values = value.as_array().ok_or_else(|| {
                RendererError::BindingError("Sparkline.values 必须是 number array".to_owned())
            })?;
            let mut bars = String::new();
            for value in values {
                let value = value.as_f64().ok_or_else(|| {
                    RendererError::BindingError("Sparkline.values 必须是 number array".to_owned())
                })?;
                let value = value.clamp(0.0, 100.0);
                bars.push_str(&format!(
                    "        Rectangle {{ width: 4px; height: parent.height * {value} / 100; background: {color}; }}\n"
                ));
            }
            bars
        }
        Some(PropValue::Binding(Binding::Item { .. })) | None => {
            return Err(RendererError::BindingError(
                "Sparkline.values 缺少受支持的值".to_owned(),
            ));
        }
    };
    Ok(format!(
        "Rectangle {{\n    height: 48px;\n    accessible-role: image;\n    accessible-label: {label};\n    HorizontalLayout {{\n        spacing: 2px;\n        alignment: end;\n{bars}    }}\n}}\n"
    ))
}

/// Grid:把声明式 columns 转为 Slint 的显式 Row 分组，避免忽略列数语义。
fn render_grid(comp: &Component, ctx: &mut Ctx, depth: usize) -> Result<String, RendererError> {
    let columns = comp
        .props
        .get("columns")
        .and_then(|value| match value {
            PropValue::Literal(value) => value.as_u64(),
            PropValue::Binding(_) => None,
        })
        .unwrap_or(1) as usize;
    let props = render_layout_props(comp)?;
    let mut rows = String::new();
    for row in comp.children.chunks(columns) {
        rows.push_str("    Row {\n");
        for child in row {
            let rendered = render_node(child, ctx, depth + 1)?;
            for line in rendered.lines() {
                rows.push_str("        ");
                rows.push_str(line);
                rows.push('\n');
            }
        }
        rows.push_str("    }\n");
    }
    Ok(format!("GridLayout {{\n{props}{rows}}}\n"))
}

fn render_list(comp: &Component, ctx: &mut Ctx, depth: usize) -> Result<String, RendererError> {
    let Some(items) = comp.props.get("items") else {
        return render_layout(comp, ctx, depth, "VerticalLayout");
    };
    if !comp.children.is_empty() {
        return Err(RendererError::BindingError(
            "List 使用 items 时不能同时声明静态 children".to_owned(),
        ));
    }
    match items {
        PropValue::Binding(Binding::State { bind }) => {
            let slot = binding_slot(bind, BindingValueType::StringList, ctx)?;
            Ok(format!(
                "VerticalLayout {{\n    for item in root.{}: Text {{ text: item; }}\n}}\n",
                slot.prop
            ))
        }
        PropValue::Binding(Binding::Item { .. }) => Err(RendererError::BindingError(
            "List.items 不支持 ForEach item 绑定".to_owned(),
        )),
        PropValue::Literal(value) => {
            let array = value.as_array().ok_or_else(|| {
                RendererError::BindingError("List.items 必须是字符串数组".to_owned())
            })?;
            let mut children = String::new();
            for item in array {
                let text = item.as_str().ok_or_else(|| {
                    RendererError::BindingError("List.items 必须是字符串数组".to_owned())
                })?;
                children.push_str(&format!("    Text {{ text: {}; }}\n", encode_string(text)?));
            }
            Ok(format!("VerticalLayout {{\n{children}}}\n"))
        }
    }
}

/// 布局容器:展开 children,并映射公共样式 props。
fn render_layout(
    comp: &Component,
    ctx: &mut Ctx,
    depth: usize,
    layout: &str,
) -> Result<String, RendererError> {
    let mut children = String::new();
    for child in &comp.children {
        children.push_str(&render_node(child, ctx, depth + 1)?);
        children.push('\n');
    }
    let props = render_layout_props(comp)?;
    Ok(format!("{layout} {{\n{props}{children}}}\n"))
}

/// 布局的公共样式 props 映射(Slint layout spacing/padding 语义)。
fn render_layout_props(comp: &Component) -> Result<String, RendererError> {
    let mut out = String::new();
    if let Some(PropValue::Literal(v)) = comp.props.get("padding")
        && let Some(px) = v.as_f64()
    {
        out.push_str(&format!("    padding: {px}px;\n"));
    }
    if let Some(PropValue::Literal(v)) = comp.props.get("gap")
        && let Some(px) = v.as_f64()
    {
        out.push_str(&format!("    spacing: {px}px;\n"));
    }
    Ok(out)
}

/// Text:绑定或字面量 + 文本样式 prop。
fn render_text(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    let Some(prop) = comp.props.get("text") else {
        return Err(RendererError::BindingError(
            "Text 缺少 text prop".to_owned(),
        ));
    };
    match prop {
        PropValue::Binding(Binding::State { bind }) => {
            let slot = binding_slot(bind, BindingValueType::String, ctx)?;
            let mut out = String::from("    Text {\n");
            out.push_str(&format!("        text: root.{0};\n", slot.prop));
            out.push_str(&text_style_props(comp)?);
            out.push_str("    }\n");
            Ok(out)
        }
        PropValue::Binding(Binding::Item { item }) => {
            // ForEach template 内的 item 绑定由 item 变量提供,非根属性。
            let mut out = String::from("    Text {\n");
            out.push_str(&format!("        text: {item};\n"));
            out.push_str(&text_style_props(comp)?);
            out.push_str("    }\n");
            Ok(out)
        }
        PropValue::Literal(v) => {
            let text = encode_string(&value_to_string(v)?)?;
            let mut out = String::from("    Text {\n");
            out.push_str(&format!("        text: {text};\n"));
            out.push_str(&text_style_props(comp)?);
            out.push_str("    }\n");
            Ok(out)
        }
    }
}

fn text_style_props(comp: &Component) -> Result<String, RendererError> {
    let mut out = String::new();
    if let Some(PropValue::Literal(v)) = comp.props.get("colorToken")
        && let Some(token) = v.as_str()
    {
        let color = floatile_ui_schema::theme::color_token(token)
            .ok_or_else(|| RendererError::EncodeError(format!("未知宿主主题 token `{token}`")))?;
        out.push_str(&format!("        color: {};\n", color.value));
    } else if let Some(PropValue::Literal(v)) = comp.props.get("color")
        && let Some(s) = v.as_str()
    {
        out.push_str(&format!("        color: {0};\n", color_literal(s)?));
    }
    if let Some(PropValue::Literal(v)) = comp.props.get("opacity")
        && let Some(f) = v.as_f64()
    {
        out.push_str(&format!("        opacity: {f};\n"));
    }
    Ok(out)
}

/// Button:TouchArea + 绘制矩形 + 标签,点击发出声明事件。
///
/// Slint 无 Button 内建基础组件(需 std-widgets 主题),宿主用基础元素绘制,
/// 保持与 registry 相同的输入事件语义(`activate`)。
fn render_button(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    let label = match comp.props.get("label") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let slot = binding_slot(bind, BindingValueType::String, ctx)?;
            format!("root.{0}", slot.prop)
        }
        Some(PropValue::Binding(Binding::Item { item })) => item.clone(),
        Some(PropValue::Literal(v)) => encode_string(&value_to_string(v)?)?,
        None => {
            return Err(RendererError::BindingError(
                "Button 缺少 label prop".to_owned(),
            ));
        }
    };
    let callback = event_callback(comp, ctx, "activate")?;
    let callback_name = callback.callback.clone();
    let mut out = String::from("    TouchArea {\n");
    out.push_str("        accessible-role: button;\n");
    out.push_str(&format!("        accessible-label: {label};\n"));
    out.push_str("        Rectangle {\n");
    out.push_str("            border-radius: 4px;\n");
    out.push_str("            background: #2a2f3a;\n");
    out.push_str("            Text {\n");
    out.push_str(&format!("                text: {label};\n"));
    out.push_str("                color: white;\n");
    out.push_str("                horizontal-alignment: center;\n");
    out.push_str("                vertical-alignment: center;\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str(&format!(
        "        clicked => {{ root.{callback_name}(); }}\n"
    ));
    out.push_str("    }\n");
    Ok(out)
}

/// Toggle:TouchArea + 按 checked 状态换色,点击发出 `toggle` 事件。
fn render_toggle(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    let checked = match comp.props.get("checked") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let slot = binding_slot(bind, BindingValueType::Boolean, ctx)?;
            format!("root.{0}", slot.prop)
        }
        Some(PropValue::Binding(Binding::Item { item })) => item.clone(),
        Some(PropValue::Literal(v)) => {
            let b = v.as_bool().ok_or_else(|| {
                RendererError::BindingError("Toggle checked 必须是布尔值".to_owned())
            })?;
            b.to_string()
        }
        None => {
            return Err(RendererError::BindingError(
                "Toggle 缺少 checked prop".to_owned(),
            ));
        }
    };
    let accessibility_label = render_accessibility_label(comp, ctx)?;
    let callback = event_callback(comp, ctx, "toggle")?;
    let callback_name = callback.callback.clone();
    let mut out = String::from("    TouchArea {\n");
    out.push_str("        accessible-role: switch;\n");
    out.push_str(&format!(
        "        accessible-label: {accessibility_label};\n"
    ));
    out.push_str(&format!("        accessible-checked: {checked};\n"));
    out.push_str("        Rectangle {\n");
    out.push_str("            border-radius: 2px;\n");
    out.push_str("            border-width: 1px;\n");
    out.push_str("            border-color: #4a90e2;\n");
    out.push_str(&format!("            background: {};\n", tied(checked)));
    // 状态指示圆点(Toggle 语义的视觉呈现,值随 checked 绑定)。
    out.push_str("        }\n");
    out.push_str(&format!(
        "        clicked => {{ root.{callback_name}(); }}\n"
    ));
    out.push_str("    }\n");
    Ok(out)
}

/// 布尔绑定转 Slint 颜色:true 亮色表示开启,false 暗色表示关闭。
fn tied(checked: String) -> String {
    format!("(if {checked} ? #3a7dff : #2a2f3a)")
}

/// Progress/Gauge:按绑定数值绘制填充条(Gauge 为环形语义的简化水平填充)。
fn render_meter(comp: &Component, ctx: &mut Ctx, _gauge: bool) -> Result<String, RendererError> {
    let value = match comp.props.get("value") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let slot = binding_slot(bind, BindingValueType::Number, ctx)?;
            format!(
                "(root.{0} < 0 ? 0 : (root.{0} > 100 ? 100 : root.{0}))",
                slot.prop
            )
        }
        Some(PropValue::Binding(Binding::Item { .. })) => {
            return Err(RendererError::BindingError(
                "meter 不支持 ForEach item 绑定".to_owned(),
            ));
        }
        Some(PropValue::Literal(v)) => {
            let f = v
                .as_f64()
                .ok_or_else(|| RendererError::BindingError("meter value 必须是数字".to_owned()))?;
            format!("{}", f.clamp(0.0, 100.0))
        }
        None => {
            return Err(RendererError::BindingError(
                "meter 缺少 value prop".to_owned(),
            ));
        }
    };
    let accessibility_label = render_accessibility_label(comp, ctx)?;
    let mut out = String::from("    Rectangle {\n");
    out.push_str("        accessible-role: progress-indicator;\n");
    out.push_str(&format!(
        "        accessible-label: {accessibility_label};\n"
    ));
    out.push_str(&format!("        accessible-value: {value};\n"));
    out.push_str("        accessible-value-minimum: 0;\n");
    out.push_str("        accessible-value-maximum: 100;\n");
    out.push_str("        border-radius: 2px;\n");
    out.push_str("        background: #2a2f3a;\n");
    out.push_str("        width: 100%;\n");
    out.push_str("        height: 8px;\n");
    out.push_str("        Rectangle {\n");
    out.push_str("            border-radius: 2px;\n");
    out.push_str("            background: #4a90e2;\n");
    out.push_str(&format!(
        "            width: parent.width * ({value} / 100);\n"
    ));
    out.push_str("            height: 8px;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    Ok(out)
}

fn render_string_prop(
    comp: &Component,
    prop: &str,
    ctx: &mut Ctx,
) -> Result<String, RendererError> {
    match comp.props.get(prop) {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let slot = binding_slot(bind, BindingValueType::String, ctx)?;
            Ok(format!("root.{0}", slot.prop))
        }
        Some(PropValue::Binding(Binding::Item { item })) => Ok(item.clone()),
        Some(PropValue::Literal(value)) => encode_string(&value_to_string(value)?),
        None => Err(RendererError::BindingError(format!(
            "{} 缺少 {prop} prop",
            comp.kind
        ))),
    }
}

fn render_accessibility_label(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    if ctx.ui_minor < 6 && !comp.props.contains_key("accessibilityLabel") {
        return encode_string(&comp.kind);
    }
    render_string_prop(comp, "accessibilityLabel", ctx)
}

/// Badge：宿主语义 tone 映射到固定主题色，不接受插件提供的颜色源码。
fn render_badge(comp: &Component, ctx: &mut Ctx) -> Result<String, RendererError> {
    let label = match comp.props.get("label") {
        Some(PropValue::Binding(Binding::State { bind })) => {
            let slot = binding_slot(bind, BindingValueType::String, ctx)?;
            format!("root.{0}", slot.prop)
        }
        Some(PropValue::Binding(Binding::Item { item })) => item.clone(),
        Some(PropValue::Literal(value)) => encode_string(&value_to_string(value)?)?,
        None => {
            return Err(RendererError::BindingError(
                "Badge 缺少 label prop".to_owned(),
            ));
        }
    };
    let tone = match comp.props.get("tone") {
        None => "neutral",
        Some(PropValue::Literal(value)) => value
            .as_str()
            .ok_or_else(|| RendererError::BindingError("Badge tone 必须是字符串".to_owned()))?,
        Some(PropValue::Binding(_)) => {
            return Err(RendererError::BindingError(
                "Badge tone 不允许运行时绑定".to_owned(),
            ));
        }
    };
    let background = match tone {
        "neutral" => "#3b4351",
        "info" => "#245ea8",
        "success" => "#247a45",
        "warning" => "#8a6418",
        "danger" => "#9b3030",
        other => {
            return Err(RendererError::EncodeError(format!(
                "Badge tone `{other}` 不在允许集合"
            )));
        }
    };
    Ok(format!(
        "    Rectangle {{\n        border-radius: 8px;\n        background: {background};\n        height: 20px;\n        Text {{\n            text: {label};\n            color: white;\n            horizontal-alignment: center;\n            vertical-alignment: center;\n        }}\n    }}\n"
    ))
}

/// If:when 布尔绑定 → Slint `if` 结构。
fn render_if(comp: &Component, ctx: &mut Ctx, depth: usize) -> Result<String, RendererError> {
    let Some(Binding::State { bind }) = &comp.when else {
        return Err(RendererError::BindingError(
            "If 缺少 when 布尔 State 绑定".to_owned(),
        ));
    };
    let slot = binding_slot(bind, BindingValueType::Boolean, ctx)?;
    let Some(then) = &comp.then else {
        return Err(RendererError::BindingError("If 缺少 then 分支".to_owned()));
    };
    let then_text = render_node(then, ctx, depth + 1)?;
    let mut out = String::new();
    out.push_str(&format!("if root.{0}: ", slot.prop));
    out.push_str(&then_text);
    if let Some(els) = &comp.else_ {
        let else_text = render_node(els, ctx, depth + 1)?;
        out.push_str(&format!("if !root.{0}: ", slot.prop));
        out.push_str(&else_text);
    }
    Ok(out)
}

/// ForEach:items 数组绑定 → Slint `for` 循环(模板内 item 绑定)。
fn render_foreach(comp: &Component, ctx: &mut Ctx, depth: usize) -> Result<String, RendererError> {
    let Some(Binding::State { bind }) = &comp.items else {
        return Err(RendererError::BindingError(
            "ForEach 缺少 items 数组 State 绑定".to_owned(),
        ));
    };
    let slot = binding_slot(bind, BindingValueType::String, ctx)?;
    let Some(template) = &comp.template else {
        return Err(RendererError::BindingError(
            "ForEach 缺少 template".to_owned(),
        ));
    };
    // 模板内的 item 绑定(`{"item": "field"}`)由 for 变量提供,进入独立命名空间。
    let template_text = render_node(template, ctx, depth + 1)?;
    let mut out = String::new();
    out.push_str(&format!(
        "for item[{0}] in root.{1}: {{\n",
        item_key(comp)?,
        slot.prop
    ));
    out.push_str(&template_text);
    out.push_str("}\n");
    Ok(out)
}

fn item_key(comp: &Component) -> Result<String, RendererError> {
    if let Some(key) = &comp.key {
        if key.is_empty() {
            return Err(RendererError::BindingError(
                "ForEach key 不能为空".to_owned(),
            ));
        }
        Ok(key.clone())
    } else {
        Ok("item".to_owned())
    }
}

/// 登记一个 State 绑定并返回生成的属性名(同路径复用同一属性)。
fn binding_slot(
    bind: &str,
    value_type: BindingValueType,
    ctx: &mut Ctx,
) -> Result<BindingSlot, RendererError> {
    PathSegments::parse(bind).map_err(|e| RendererError::BindingError(format!("{bind}: {e}")))?;
    if let Some(slot) = ctx.bindings.get(bind) {
        if slot.value_type != value_type {
            return Err(RendererError::BindingError(format!(
                "{bind} 被用于不兼容的投影类型"
            )));
        }
        return Ok(slot.clone());
    }
    if ctx.bindings.len() >= MAX_BINDINGS {
        return Err(RendererError::BudgetExceeded(format!(
            "绑定数超过 renderer 上限 {MAX_BINDINGS}"
        )));
    }
    let prop = format!("prop_{}", path_prop_name(bind));
    let slot = BindingSlot {
        path: bind.to_owned(),
        prop,
        value_type,
    };
    ctx.bindings.insert(bind.to_owned(), slot.clone());
    Ok(slot)
}

/// 从规范 JSONPath 派生稳定属性名(`$.a.b` → `a_b`),点号/元字符转义为 `_`。
fn path_prop_name(bind: &str) -> String {
    let segments = PathSegments::parse(bind)
        .map(|s| s.segments().join("_"))
        .unwrap_or_else(|_| {
            bind.trim_start_matches("$.")
                .replace(['.', ' ', '-', '['], "_")
        });
    if segments.is_empty() {
        "state".to_owned()
    } else {
        segments
    }
}

/// 登记输入事件并返回生成的回调名(同事件复用同一回调)。
fn event_callback(
    comp: &Component,
    ctx: &mut Ctx,
    input_event: &str,
) -> Result<EventSlot, RendererError> {
    let Some(emitted) = comp.events.get(input_event) else {
        return Err(RendererError::BindingError(format!(
            "组件 {} 未声明输入事件 `{input_event}`",
            comp.kind
        )));
    };
    let event = emitted.emit.clone();
    if let Some(slot) = ctx.events.get(&event) {
        return Ok(slot.clone());
    }
    let callback = format!("emit_{}", ctx.callback_counter);
    ctx.callback_counter += 1;
    let slot = EventSlot { event, callback };
    ctx.events.insert(slot.event.clone(), slot.clone());
    Ok(slot)
}

/// 字符串值编码为 Slint 双引号字符串字面量(结构化转义)。
fn encode_string(s: &str) -> Result<String, RendererError> {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                return Err(RendererError::EncodeError(format!(
                    "字符串含控件字符 U+{:04X},拒绝嵌入 Slint 文本",
                    c as u32
                )));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// 颜色字符串编码为 Slint 颜色字面量(仅接受受限形式)。
fn color_literal(s: &str) -> Result<String, RendererError> {
    let s = s.trim();
    let valid = s.starts_with('#')
        && matches!(s.len(), 4 | 7 | 9)
        && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    if valid {
        Ok(s.to_owned())
    } else {
        Err(RendererError::EncodeError(format!(
            "颜色 `{s}` 不是受限 #RRGGBB[AA] 字面量"
        )))
    }
}

/// 把字面量 JSON 值转字符串(仅标量)。
fn value_to_string(v: &serde_json::Value) -> Result<String, RendererError> {
    match v {
        serde_json::Value::String(s) => Ok(s.clone()),
        serde_json::Value::Null => Ok(String::new()),
        other => Err(RendererError::EncodeError(format!(
            "prop 字面量不支持非字符串值 {other}"
        ))),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use floatile_ui_schema::ir::Component;
    use serde_json::json;

    fn doc(root: Component) -> UiDocument {
        UiDocument {
            ui_api_version: floatile_ui_schema::UI_API_VERSION.into(),
            state: floatile_ui_schema::ir::StateSchema {
                initial: json!({"time": "00:00:00", "running": false, "items": [], "trend": []}),
                schema: floatile_ui_schema::JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::from([
                        (
                            "time".into(),
                            floatile_ui_schema::JsonSchema::String {
                                max_length: Some(32),
                            },
                        ),
                        ("running".into(), floatile_ui_schema::JsonSchema::Boolean),
                        (
                            "items".into(),
                            floatile_ui_schema::JsonSchema::Array {
                                max_items: Some(16),
                                items: Box::new(floatile_ui_schema::JsonSchema::String {
                                    max_length: Some(64),
                                }),
                            },
                        ),
                        (
                            "trend".into(),
                            floatile_ui_schema::JsonSchema::Array {
                                max_items: Some(16),
                                items: Box::new(floatile_ui_schema::JsonSchema::Number),
                            },
                        ),
                    ]),
                    additional_properties: false,
                },
            },
            events: BTreeMap::new(),
            root,
        }
    }

    fn doc_with_items() -> UiDocument {
        UiDocument {
            ui_api_version: floatile_ui_schema::UI_API_VERSION.into(),
            state: floatile_ui_schema::ir::StateSchema {
                initial: json!({"time": "00:00:00", "running": false, "items": []}),
                schema: floatile_ui_schema::JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::from([
                        (
                            "time".into(),
                            floatile_ui_schema::JsonSchema::String {
                                max_length: Some(32),
                            },
                        ),
                        ("running".into(), floatile_ui_schema::JsonSchema::Boolean),
                        (
                            "items".into(),
                            floatile_ui_schema::JsonSchema::Array {
                                max_items: Some(16),
                                items: Box::new(floatile_ui_schema::JsonSchema::String {
                                    max_length: Some(64),
                                }),
                            },
                        ),
                    ]),
                    additional_properties: false,
                },
            },
            events: BTreeMap::new(),
            root: Component::default(),
        }
    }

    fn text_bind(path: &str) -> Component {
        Component {
            kind: "Text".into(),
            props: BTreeMap::from([(
                "text".into(),
                PropValue::Binding(Binding::State { bind: path.into() }),
            )]),
            ..Default::default()
        }
    }

    fn column(children: Vec<Component>) -> Component {
        Component {
            kind: "Column".into(),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn renders_column_text_with_binding_slot() {
        let root = column(vec![text_bind("$.time")]);
        let rendered = render_component(&doc(root)).unwrap();
        assert!(
            rendered
                .source
                .contains("component ClockPluginUI inherits Rectangle")
        );
        assert!(rendered.source.contains("VerticalLayout"));
        assert!(rendered.source.contains("text: root.prop_time;"));
        assert_eq!(
            rendered.bindings,
            vec![BindingSlot {
                path: "$.time".into(),
                prop: "prop_time".into(),
                value_type: BindingValueType::String,
            }]
        );
    }

    #[test]
    fn reuses_binding_slot_for_same_path() {
        let root = column(vec![text_bind("$.time"), text_bind("$.time")]);
        let rendered = render_component(&doc(root)).unwrap();
        assert_eq!(rendered.bindings.len(), 1);
    }

    #[test]
    fn escapes_literal_text() {
        let root = Component {
            kind: "Text".into(),
            props: BTreeMap::from([(
                "text".into(),
                PropValue::Literal(json!("a \"quoted\" \\ path\nline")),
            )]),
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(
            rendered
                .source
                .contains(r#"text: "a \"quoted\" \\ path\nline";"#)
        );
        assert!(!rendered.source.contains("a \"quoted\" \\ path\n"));
    }

    #[test]
    fn rejects_unknown_component() {
        // 未知组件(不在 registry)由 validate_document 先拒绝,renderer 不改写该语义。
        let root = Component {
            kind: "Canvas".into(),
            ..Default::default()
        };
        let err = render_component(&doc(root)).unwrap_err();
        assert_eq!(err.code(), "RNDR_INVALID_IR");
    }

    #[test]
    fn rejects_registry_but_unmapped_component() {
        // registry 通过、但 renderer 尚未映射的组件(如 Scroll)由 renderer 层拒绝。
        let root = Component {
            kind: "Scroll".into(),
            ..Default::default()
        };
        let err = render_component(&doc(root)).unwrap_err();
        assert_eq!(err.code(), "RNDR_UNSUPPORTED_COMPONENT");
    }

    #[test]
    fn rejects_unvalidated_missing_text_prop() {
        let root = Component {
            kind: "Text".into(),
            ..Default::default()
        };
        assert!(render_component(&doc(root)).is_err());
    }

    #[test]
    fn renders_button_event_slot() {
        let mut d = doc(Component {
            kind: "Button".into(),
            props: BTreeMap::from([("label".into(), PropValue::Literal(json!("Go")))]),
            events: BTreeMap::from([(
                "activate".into(),
                floatile_ui_schema::ir::EmittedEvent {
                    emit: "toggle".into(),
                    payload: json!({}),
                },
            )]),
            ..Default::default()
        });
        d.events.insert(
            "toggle".into(),
            floatile_ui_schema::ir::EventSchema {
                payload: floatile_ui_schema::JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::new(),
                    additional_properties: false,
                },
            },
        );
        let rendered = render_component(&d).unwrap();
        assert!(rendered.source.contains("TouchArea"));
        assert!(rendered.source.contains("clicked => { root.emit_0(); }"));
        assert_eq!(
            rendered.events,
            vec![EventSlot {
                event: "toggle".into(),
                callback: "emit_0".into(),
            }]
        );
    }
    #[test]
    fn renders_if_when_else_branches() {
        // If 生成 `if root.prop_running:` + then/else 分支。
        let root = Component {
            kind: "If".into(),
            when: Some(Binding::State {
                bind: "$.running".into(),
            }),
            then: Some(Box::new(text_bind("$.time"))),
            else_: Some(Box::new(Component {
                kind: "Text".into(),
                props: BTreeMap::from([("text".into(), PropValue::Literal(json!("stopped")))]),
                ..Default::default()
            })),
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        let src = &rendered.source;
        assert!(src.contains("if root.prop_running:"), "缺少 if 分支: {src}");
        assert!(
            src.contains("if !root.prop_running:"),
            "缺少 else 分支: {src}"
        );
        assert!(
            src.contains("text: root.prop_time;"),
            "then 分支应渲染 time 绑定"
        );
        assert!(src.contains(r#"text: "stopped";"#), "else 分支应渲染字面量");
        assert!(
            rendered.bindings.iter().any(|b| b.path == "$.running"),
            "when 绑定应登记"
        );
    }

    #[test]
    fn renders_foreach_items_template() {
        // ForEach 生成 `for item[N] in root.prop_items:` 循环,模板内 item 绑定。
        let template_root = Component {
            kind: "Column".into(),
            children: vec![Component {
                kind: "Text".into(),
                props: BTreeMap::from([(
                    "text".into(),
                    PropValue::Binding(Binding::Item {
                        item: "value".into(),
                    }),
                )]),
                ..Default::default()
            }],
            ..Default::default()
        };
        let root = Component {
            kind: "ForEach".into(),
            items: Some(Binding::State {
                bind: "$.items".into(),
            }),
            key: Some("value".into()),
            template: Some(Box::new(template_root)),
            ..Default::default()
        };
        let mut d = doc_with_items();
        d.root = root;
        let rendered = render_component(&d).unwrap();
        let src = &rendered.source;
        assert!(
            src.contains("for item[value] in root.prop_items:"),
            "缺少 for: {src}"
        );
        assert!(src.contains("text: value;"), "模板 item 绑定应渲染");
    }

    #[test]
    fn renders_toggle_event_slot() {
        let root = Component {
            kind: "Toggle".into(),
            props: BTreeMap::from([
                (
                    "checked".into(),
                    PropValue::Binding(Binding::State {
                        bind: "$.running".into(),
                    }),
                ),
                (
                    "accessibilityLabel".into(),
                    PropValue::Literal(json!("Timer running")),
                ),
            ]),
            events: BTreeMap::from([(
                "toggle".into(),
                floatile_ui_schema::ir::EmittedEvent {
                    emit: "toggle".into(),
                    payload: json!({}),
                },
            )]),
            ..Default::default()
        };
        let mut d = doc(root);
        d.events.insert(
            "toggle".into(),
            floatile_ui_schema::ir::EventSchema {
                payload: floatile_ui_schema::JsonSchema::Object {
                    required: vec![],
                    properties: BTreeMap::new(),
                    additional_properties: false,
                },
            },
        );
        let rendered = render_component(&d).unwrap();
        assert!(rendered.source.contains("TouchArea"));
        assert!(rendered.source.contains("accessible-role: switch"));
        assert!(
            rendered
                .source
                .contains("accessible-checked: root.prop_running")
        );
        assert!(
            rendered
                .source
                .contains("accessible-label: \"Timer running\"")
        );
        // checked 绑定经受限三元映射为颜色(布尔 → 亮/暗色),无直接属性引用。
        assert!(
            rendered
                .source
                .contains("root.prop_running ? #3a7dff : #2a2f3a")
        );
        assert_eq!(
            rendered.events,
            vec![EventSlot {
                event: "toggle".into(),
                callback: "emit_0".into(),
            }]
        );
    }

    #[test]
    fn renders_typed_boolean_and_number_slots() {
        let root = column(vec![
            Component {
                kind: "If".into(),
                when: Some(Binding::State {
                    bind: "$.running".into(),
                }),
                then: Some(Box::new(text_bind("$.time"))),
                ..Default::default()
            },
            Component {
                kind: "Progress".into(),
                props: BTreeMap::from([
                    (
                        "value".into(),
                        PropValue::Binding(Binding::State {
                            bind: "$.percent".into(),
                        }),
                    ),
                    (
                        "accessibilityLabel".into(),
                        PropValue::Literal(json!("Timer progress")),
                    ),
                ]),
                ..Default::default()
            },
        ]);
        let mut document = doc(root);
        if let floatile_ui_schema::JsonSchema::Object { properties, .. } =
            &mut document.state.schema
        {
            properties.insert("percent".into(), floatile_ui_schema::JsonSchema::Number);
        }
        document.state.initial["percent"] = json!(42.0);
        let rendered = render_component(&document).unwrap();
        assert!(rendered.source.contains("in property <bool> prop_running"));
        assert!(rendered.source.contains("in property <float> prop_percent"));
        assert!(
            rendered
                .source
                .contains("accessible-role: progress-indicator")
        );
        assert!(
            rendered
                .source
                .contains("accessible-label: \"Timer progress\"")
        );
        assert!(rendered.source.contains(
            "width: parent.width * ((root.prop_percent < 0 ? 0 : (root.prop_percent > 100 ? 100 : root.prop_percent)) / 100)"
        ));
    }

    #[test]
    fn renders_badge_with_host_owned_tone() {
        let root = Component {
            kind: "Badge".into(),
            props: BTreeMap::from([
                (
                    "label".into(),
                    PropValue::Binding(Binding::State {
                        bind: "$.time".into(),
                    }),
                ),
                ("tone".into(), PropValue::Literal(json!("success"))),
            ]),
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(rendered.source.contains("background: #247a45"));
        assert!(rendered.source.contains("text: root.prop_time"));
    }

    #[test]
    fn rejects_badge_tone_outside_host_palette() {
        let root = Component {
            kind: "Badge".into(),
            props: BTreeMap::from([
                ("label".into(), PropValue::Literal(json!("unsafe"))),
                (
                    "tone".into(),
                    PropValue::Literal(json!("url(plugin-input)")),
                ),
            ]),
            ..Default::default()
        };
        let error = render_component(&doc(root)).unwrap_err();
        assert_eq!(error.code(), "RNDR_INVALID_IR");
    }

    #[test]
    fn renders_bounded_string_list_model() {
        let root = Component {
            kind: "List".into(),
            props: BTreeMap::from([(
                "items".into(),
                PropValue::Binding(Binding::State {
                    bind: "$.items".into(),
                }),
            )]),
            ..Default::default()
        };
        let mut document = doc_with_items();
        document.root = root;
        let rendered = render_component(&document).unwrap();
        assert!(
            rendered
                .source
                .contains("in property <[string]> prop_items")
        );
        assert!(rendered.source.contains("for item in root.prop_items"));
        assert_eq!(
            rendered.bindings[0].value_type,
            BindingValueType::StringList
        );
    }

    #[test]
    fn renders_grid_columns_as_explicit_rows() {
        let child = |label: &str| Component {
            kind: "Text".into(),
            props: BTreeMap::from([("text".into(), PropValue::Literal(json!(label)))]),
            ..Default::default()
        };
        let root = Component {
            kind: "Grid".into(),
            props: BTreeMap::from([("columns".into(), PropValue::Literal(json!(2)))]),
            children: vec![child("one"), child("two"), child("three")],
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(rendered.source.contains("GridLayout"));
        assert_eq!(rendered.source.matches("Row {").count(), 2);
    }

    #[test]
    fn renders_accessible_bounded_sparkline() {
        let root = Component {
            kind: "Sparkline".into(),
            props: BTreeMap::from([
                (
                    "values".into(),
                    PropValue::Binding(Binding::State {
                        bind: "$.trend".into(),
                    }),
                ),
                ("label".into(), PropValue::Literal(json!("Usage trend"))),
                ("tone".into(), PropValue::Literal(json!("info"))),
            ]),
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(rendered.source.contains("in property <[float]> prop_trend"));
        assert!(rendered.source.contains("accessible-role: image"));
        assert!(
            rendered
                .source
                .contains("accessible-label: \"Usage trend\"")
        );
        assert_eq!(
            rendered.bindings[0].value_type,
            BindingValueType::NumberList
        );
    }

    #[test]
    fn renders_responsive_layout_from_host_width() {
        let root = Component {
            kind: "Responsive".into(),
            props: BTreeMap::from([("breakpoint".into(), PropValue::Literal(json!(420)))]),
            children: vec![text_bind("$.time")],
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(rendered.source.contains("if root.width < 420px"));
        assert!(rendered.source.contains("if root.width >= 420px"));
        assert_eq!(rendered.bindings.len(), 1);
    }

    #[test]
    fn resolves_named_text_color_from_host_registry() {
        let root = Component {
            kind: "Text".into(),
            props: BTreeMap::from([
                ("text".into(), PropValue::Literal(json!("balance"))),
                ("colorToken".into(), PropValue::Literal(json!("accent"))),
            ]),
            ..Default::default()
        };
        let rendered = render_component(&doc(root)).unwrap();
        assert!(rendered.source.contains("color: #89b4fa"));
        assert!(!rendered.source.contains("accent"));
    }
}
