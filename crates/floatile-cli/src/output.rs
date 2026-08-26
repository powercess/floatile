//! CLI 自动化输出的共享、版本化契约。
//!
//! 人类可读文本不是自动化接口；Agent/CI 只依赖这里定义的 JSON 字段、稳定 code
//! 和进程退出码。错误详情必须由调用方先脱敏，不能直接放入 cargo stderr 或宿主路径。

use serde::Serialize;

pub const OUTPUT_SCHEMA_VERSION: u32 = 1;

/// 所有作者命令共享的 warning 形状。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandWarning {
    pub code: String,
    pub message: String,
}

/// 所有作者命令共享的失败形状；`phases` 保留各命令自己的类型化阶段。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandErrorReport<P: Serialize> {
    pub schema_version: u32,
    pub status: &'static str,
    pub severity: &'static str,
    pub code: String,
    pub detail: String,
    pub phases: P,
    pub warnings: Vec<CommandWarning>,
}

impl<P: Serialize> CommandErrorReport<P> {
    pub fn new(code: impl Into<String>, detail: impl Into<String>, phases: P) -> Self {
        Self {
            schema_version: OUTPUT_SCHEMA_VERSION,
            status: "error",
            severity: "error",
            code: code.into(),
            detail: detail.into(),
            phases,
            warnings: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn error_report_has_stable_versioned_shape() {
        let report = CommandErrorReport::new(
            "FCOMMAND_FAILED",
            "bounded public detail",
            serde_json::json!({ "build": false }),
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["schemaVersion"], OUTPUT_SCHEMA_VERSION);
        assert_eq!(value["status"], "error");
        assert_eq!(value["severity"], "error");
        assert_eq!(value["code"], "FCOMMAND_FAILED");
        assert_eq!(value["warnings"], serde_json::json!([]));
    }
}
