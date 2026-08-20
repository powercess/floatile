//! UI IR 校验的稳定错误分类。
//!
//! `code()` 返回稳定的诊断 code（`FTUI_*`），自由文本不作为测试或 Agent 判断依据；
//! 跨语言（Rust/TypeScript SDK、CLI、runtime）对同一错误使用同一 code。

/// UI IR 校验错误。
///
/// 所有错误都是对不可信 `widget.ftui` 输入的分类拒绝；不携带宿主内部结构、
/// 路径或敏感值。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum UiSchemaError {
    #[error("不支持的 uiApiVersion `{0}`，需要 major 1")]
    UnsupportedApiVersion(String),
    #[error("未知组件 `{0}`")]
    UnknownComponent(String),
    #[error("组件 `{component}` 的未知 prop `{prop}`")]
    UnknownProp { component: String, prop: String },
    #[error("组件 `{component}` 缺少必填 prop `{prop}`")]
    MissingProp { component: String, prop: String },
    #[error("prop `{prop}` 类型不合法：期望 {expected:?}")]
    InvalidPropType { prop: String, expected: Vec<String> },
    #[error("绑定路径不合法：{0}")]
    InvalidBindingPath(String),
    #[error("绑定类型不匹配：{0}")]
    BindingTypeMismatch(String),
    #[error("ForEach item 绑定不合法：{0}")]
    InvalidItemBinding(String),
    #[error("发出的未知事件 `{0}`")]
    UnknownEvent(String),
    #[error("组件 `{component}` 未声明输入事件 `{event}`")]
    UnknownInputEvent { component: String, event: String },
    #[error("事件 payload 不合法：{0}")]
    InvalidEventPayload(String),
    #[error("State 不合法：{0}")]
    InvalidState(String),
    #[error("超出预算限制：{0}")]
    LimitExceeded(String),
    #[error("子组件结构不合法：{0}")]
    InvalidChildren(String),
    #[error("If/ForEach 控制组件结构不合法：{0}")]
    InvalidControl(String),
}

impl UiSchemaError {
    /// 稳定诊断 code（`FTUI_*`）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedApiVersion(_) => "FTUI_UNSUPPORTED_API_VERSION",
            Self::UnknownComponent(_) => "FTUI_UNKNOWN_COMPONENT",
            Self::UnknownProp { .. } => "FTUI_UNKNOWN_PROP",
            Self::MissingProp { .. } => "FTUI_MISSING_PROP",
            Self::InvalidPropType { .. } => "FTUI_INVALID_PROP_TYPE",
            Self::InvalidBindingPath(_) => "FTUI_INVALID_BINDING_PATH",
            Self::BindingTypeMismatch(_) => "FTUI_BINDING_TYPE_MISMATCH",
            Self::InvalidItemBinding(_) => "FTUI_INVALID_ITEM_BINDING",
            Self::UnknownEvent(_) => "FTUI_UNKNOWN_EVENT",
            Self::UnknownInputEvent { .. } => "FTUI_UNKNOWN_INPUT_EVENT",
            Self::InvalidEventPayload(_) => "FTUI_INVALID_EVENT_PAYLOAD",
            Self::InvalidState(_) => "FTUI_INVALID_STATE",
            Self::LimitExceeded(_) => "FTUI_LIMIT_EXCEEDED",
            Self::InvalidChildren(_) => "FTUI_INVALID_CHILDREN",
            Self::InvalidControl(_) => "FTUI_INVALID_CONTROL",
        }
    }
}
