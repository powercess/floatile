//! 布局持久化模型：widget 实例的几何、层级与模式。
//!
//! 纯领域类型，无 I/O；校验规则可单测，供 `floatile-store` 持久化与
//! `floatile-shell` 恢复时共用。

use serde::{Deserialize, Serialize};

use crate::{InstanceId, LogicalRect, PluginId, WidgetMode};

/// 布局记录：一个 widget 实例在宿主上的几何与展示状态。
///
/// 坐标与尺寸均为逻辑像素；`monitor_key` 是显示器指纹（EDID/product 标识），
/// 原屏缺失时由恢复逻辑降级到主屏并标记 `lost_monitor`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetLayout {
    /// 运行时组件实例 ID。
    pub instance_id: InstanceId,
    /// 来源插件 ID（内建参考时钟等使用保留命名空间）。
    pub plugin_id: PluginId,
    /// 目标显示器指纹；空表示跟随主屏/未知。
    pub monitor_key: Option<String>,
    /// 逻辑像素矩形（位置 + 尺寸）。
    pub rect: LogicalRect,
    /// 层级（数值越大越靠上）。
    pub z: u32,
    /// 展示模式。
    pub mode: WidgetMode,
    /// 记录版本号，供向前迁移与并发冲突检测。
    pub version: u32,
    /// 更新时间（Unix 秒）。
    pub updated_at: u64,
}

/// 布局记录的序列化版本。
pub const LAYOUT_RECORD_VERSION: u32 = 1;

impl WidgetLayout {
    /// 校验布局记录；返回可读的错误原因。
    pub fn validate(&self) -> Result<(), LayoutValidationError> {
        if self.version == 0 {
            return Err(LayoutValidationError::InvalidVersion);
        }
        if self.rect.size.width <= 0.0 || self.rect.size.height <= 0.0 {
            return Err(LayoutValidationError::NonPositiveSize);
        }
        if self.z == 0 {
            return Err(LayoutValidationError::ZeroZ);
        }
        Ok(())
    }
}

/// 布局校验错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LayoutValidationError {
    #[error("布局记录版本无效（应为 {LAYOUT_RECORD_VERSION}）")]
    InvalidVersion,
    #[error("尺寸必须为正")]
    NonPositiveSize,
    #[error("层级 z 必须大于 0")]
    ZeroZ,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn sample() -> WidgetLayout {
        WidgetLayout {
            instance_id: InstanceId(1),
            plugin_id: PluginId("dev.floatile.clock".into()),
            monitor_key: Some("edid-abc123".into()),
            rect: LogicalRect {
                position: crate::LogicalPosition { x: 120.0, y: 80.0 },
                size: crate::LogicalSize {
                    width: 260.0,
                    height: 120.0,
                },
            },
            z: 10,
            mode: WidgetMode::Edit,
            version: LAYOUT_RECORD_VERSION,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn valid_layout_passes() {
        assert_eq!(sample().validate(), Ok(()));
    }

    #[test]
    fn zero_version_rejected() {
        let mut l = sample();
        l.version = 0;
        assert_eq!(l.validate(), Err(LayoutValidationError::InvalidVersion));
    }

    #[test]
    fn non_positive_size_rejected() {
        let mut l = sample();
        l.rect.size.width = 0.0;
        assert_eq!(l.validate(), Err(LayoutValidationError::NonPositiveSize));
        l = sample();
        l.rect.size.height = -1.0;
        assert_eq!(l.validate(), Err(LayoutValidationError::NonPositiveSize));
    }

    #[test]
    fn zero_z_rejected() {
        let mut l = sample();
        l.z = 0;
        assert_eq!(l.validate(), Err(LayoutValidationError::ZeroZ));
    }

    #[test]
    fn layout_serde_roundtrip() {
        let l = sample();
        let json = serde_json::to_string(&l).unwrap();
        let back: WidgetLayout = serde_json::from_str(&json).unwrap();
        assert_eq!(l, back);
    }
}
