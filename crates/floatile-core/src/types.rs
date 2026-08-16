use serde::{Deserialize, Serialize};

/// 强类型插件 ID（反向域名命名空间）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(pub String);

/// 显示器稳定标识（优先使用 EDID/product 指纹，平台不可用时使用明确的降级键）。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MonitorKey(pub String);

impl MonitorKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for MonitorKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 逻辑像素到物理像素的缩放因子。
///
/// 该类型只允许有限正数，避免无效 DPI 数据进入布局换算。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct ScaleFactor(f64);

impl ScaleFactor {
    pub fn new(value: f64) -> Result<Self, ScaleFactorError> {
        if value.is_finite() && value > 0.0 {
            Ok(Self(value))
        } else {
            Err(ScaleFactorError(value))
        }
    }

    pub fn get(self) -> f64 {
        self.0
    }
}

impl TryFrom<f64> for ScaleFactor {
    type Error = ScaleFactorError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ScaleFactor> for f64 {
    fn from(value: ScaleFactor) -> Self {
        value.get()
    }
}

/// 无效缩放因子。
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("scale factor 必须为有限正数，实际为 {0}")]
pub struct ScaleFactorError(pub f64);

/// 虚拟桌面中的物理像素坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PhysicalPosition {
    pub x: i32,
    pub y: i32,
}

/// 物理像素或毫米尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PhysicalSize {
    pub width: u32,
    pub height: u32,
}

/// 虚拟桌面中的物理像素矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PhysicalRect {
    pub position: PhysicalPosition,
    pub size: PhysicalSize,
}

/// 运行时组件实例 ID，宿主分配，全局唯一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(pub u64);

/// 宿主展示模式。
///
/// - `Edit`：显示组件边框/拖拽区/缩放手柄，关闭点击穿透。
/// - `Show`：隐藏宿主控制元素，可选点击穿透。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WidgetMode {
    Edit,
    Show,
}

impl WidgetMode {
    /// 在 `Edit` 与 `Show` 之间切换。
    pub fn toggle(self) -> Self {
        match self {
            Self::Edit => Self::Show,
            Self::Show => Self::Edit,
        }
    }
}

/// 逻辑像素坐标（与 DPI 无关，宿主按 scale factor 换算物理像素）。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LogicalPosition {
    pub x: f32,
    pub y: f32,
}

/// 逻辑像素尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LogicalSize {
    pub width: f32,
    pub height: f32,
}

/// 尺寸约束（缩放手柄最小/最大边界，逻辑像素）。
///
/// `min` 与 `max` 按约定已归一：`min.width <= max.width` 且 `min.height <= max.height`，
/// 不变量由构造时 `with_min`/`with_max` 的顺序保证（后设置者覆盖先设置者时自动对调）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SizeConstraints {
    pub min: LogicalSize,
    pub max: LogicalSize,
}

impl Default for SizeConstraints {
    fn default() -> Self {
        Self {
            min: LogicalSize {
                width: 120.0,
                height: 60.0,
            },
            max: LogicalSize {
                width: 2000.0,
                height: 1200.0,
            },
        }
    }
}

impl SizeConstraints {
    /// 以给定最小/最大尺寸构造；若方向颠倒则对调以保证不变量。
    pub fn new(min: LogicalSize, max: LogicalSize) -> Self {
        let (min, max) = normalize(min, max);
        Self { min, max }
    }

    /// 将尺寸钳制到约束范围内（含边界）。
    pub fn clamp(&self, size: LogicalSize) -> LogicalSize {
        let width = size.width.clamp(self.min.width, self.max.width);
        let height = size.height.clamp(self.min.height, self.max.height);
        LogicalSize { width, height }
    }
}

fn normalize(min: LogicalSize, max: LogicalSize) -> (LogicalSize, LogicalSize) {
    let width_pair = ordered(min.width, max.width);
    let height_pair = ordered(min.height, max.height);
    (
        LogicalSize {
            width: width_pair.0,
            height: height_pair.0,
        },
        LogicalSize {
            width: width_pair.1,
            height: height_pair.1,
        },
    )
}

fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// 逻辑像素矩形。
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
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

    #[test]
    fn scale_factor_accepts_finite_positive_values() {
        let scale = ScaleFactor::new(1.25).unwrap();
        assert_eq!(scale.get(), 1.25);
        let json = serde_json::to_string(&scale).unwrap();
        assert_eq!(serde_json::from_str::<ScaleFactor>(&json).unwrap(), scale);
    }

    #[test]
    fn scale_factor_rejects_invalid_values_and_deserialization() {
        assert!(ScaleFactor::new(0.0).is_err());
        assert!(ScaleFactor::new(-1.0).is_err());
        assert!(ScaleFactor::new(f64::NAN).is_err());
        assert!(ScaleFactor::new(f64::INFINITY).is_err());
        assert!(serde_json::from_str::<ScaleFactor>("0").is_err());
    }

    #[test]
    fn widget_mode_toggles_between_edit_and_show() {
        assert_eq!(WidgetMode::Edit.toggle(), WidgetMode::Show);
        assert_eq!(WidgetMode::Show.toggle(), WidgetMode::Edit);
    }

    #[test]
    fn size_constraints_clamp_below_min() {
        let c = SizeConstraints::default();
        let size = LogicalSize {
            width: 40.0,
            height: 30.0,
        };
        let clamped = c.clamp(size);
        assert_eq!(clamped, c.min);
    }

    #[test]
    fn size_constraints_clamp_above_max() {
        let c = SizeConstraints::default();
        let size = LogicalSize {
            width: 5000.0,
            height: 5000.0,
        };
        let clamped = c.clamp(size);
        assert_eq!(clamped, c.max);
    }

    #[test]
    fn size_constraints_pass_through_in_range() {
        let c = SizeConstraints::default();
        let size = LogicalSize {
            width: 300.0,
            height: 150.0,
        };
        assert_eq!(c.clamp(size), size);
    }

    #[test]
    fn size_constraints_normalize_inverted_bounds() {
        let min = LogicalSize {
            width: 500.0,
            height: 400.0,
        };
        let max = LogicalSize {
            width: 100.0,
            height: 50.0,
        };
        let c = SizeConstraints::new(min, max);
        assert_eq!(c.min.width, 100.0);
        assert_eq!(c.min.height, 50.0);
        assert_eq!(c.max.width, 500.0);
        assert_eq!(c.max.height, 400.0);
    }
}
