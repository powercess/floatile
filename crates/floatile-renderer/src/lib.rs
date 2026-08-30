//! 宿主侧 UI renderer:把已验证 `widget.ftui`(UiDocument)结构化生成为
//! 宿主控制的 Slint 源码文本(ADR-0001 路径二变体)。
//!
//! 安全边界:
//! - 输入必须是已通过 `floatile_ui_schema::validate_document` 的 IR;本模块在生成前
//!   再次复验预算与结构(CLI 通过不代表 runtime 可跳过复验,renderer 同规则)。
//! - 输出是纯文本:所有字符串值经结构化编码器转义,绝不把插件 IR 的原始文本拼进
//!   Slint 语法位置;组件名、属性名、事件名由本模块生成,插件不能定义标识符。
//! - 组件映射是"降级优先":Slint 无 stable interpreter,任意运行时组件树不可直接渲染;
//!   对 build-time 已知的参考时钟,本模块生成有界、可编译的 Slint 组件,交给
//!   `slint-build` 编译(host-only,不依赖 interpreter/internal feature)。
//! - 恶意 IR 不会产生无限/超大宿主 UI:节点/深度/绑定预算在 validate 与本模块双层
//!   限制,转化为固定码错误,不泄漏宿主内部。

pub mod error;
pub mod render;

pub use error::RendererError;
pub use render::{
    BindingSlot, BindingValueType, CONTENT_COMPONENT_NAME, EventSlot,
    RUNTIME_WINDOW_COMPONENT_NAME, RenderedComponent, render_component,
};
