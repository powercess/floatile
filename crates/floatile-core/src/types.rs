use serde::{Deserialize, Serialize};

/// 强类型插件 ID（反向域名命名空间）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(pub String);

/// 运行时组件实例 ID，宿主分配，全局唯一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InstanceId(pub u64);

/// 宿主展示模式。
///
/// - `Edit`：显示组件边框/拖拽区/缩放手柄，关闭点击穿透。
/// - `Show`：隐藏宿主控制元素，可选点击穿透。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetMode {
    Edit,
    Show,
}

/// 逻辑像素坐标（与 DPI 无关，宿主按 scale factor 换算物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LogicalPosition {
    pub x: f32,
    pub y: f32,
}

/// 逻辑像素尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LogicalSize {
    pub width: f32,
    pub height: f32,
}

/// 逻辑像素矩形。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LogicalRect {
    pub position: LogicalPosition,
    pub size: LogicalSize,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn plugin_id_roundtrip_serde() {
        let id = PluginId("dev.floatile.clock".to_string());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"dev.floatile.clock\"");
        let back: PluginId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
