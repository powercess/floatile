//! renderer 稳定错误分类。
//!
//! `code()` 返回稳定诊断 code(`RNDR_*`),自由文本不作为测试或 Agent 判断依据。

use floatile_ui_schema::UiSchemaError;

/// renderer 生成错误。
///
/// 所有错误都是对不可信 `widget.ftui` 的分类拒绝;不携带宿主内部结构、路径或敏感值。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RendererError {
    /// 输入 IR 未通过 ui-schema 校验(renderer 独立复验)。
    #[error("IR 未通过校验: {0}")]
    InvalidIr(#[from] UiSchemaError),
    /// 超出 renderer 节点/深度/绑定预算。
    #[error("renderer 预算超限: {0}")]
    BudgetExceeded(String),
    /// 未知或不支持映射的组件(registry 通过但 renderer 无法安全映射)。
    #[error("组件 `{0}` 无法安全渲染: {1}")]
    UnsupportedComponent(String, String),
    /// State 绑定无法映射为宿主属性。
    #[error("State 绑定无法映射: {0}")]
    BindingError(String),
    /// 生成的文本编码错误(不应发生;视为内部错误)。
    #[error("生成的 Slint 文本编码失败: {0}")]
    EncodeError(String),
}

impl RendererError {
    /// 稳定诊断 code(`RNDR_*`)。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidIr(_) => "RNDR_INVALID_IR",
            Self::BudgetExceeded(_) => "RNDR_BUDGET_EXCEEDED",
            Self::UnsupportedComponent(..) => "RNDR_UNSUPPORTED_COMPONENT",
            Self::BindingError(_) => "RNDR_BINDING_ERROR",
            Self::EncodeError(_) => "RNDR_ENCODE_ERROR",
        }
    }
}
