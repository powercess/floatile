# 三端能力矩阵

> 状态：Proposed
> 性质：能力预测，不是实测结果。P0 结束必须回填第 5 节。
> 语义：`预期支持` · `条件支持` · `预期不支持`。任何结论都以带环境的实测记录为准。

## 1. 总览矩阵

| 能力 | Windows 10/11 | macOS 12+ | Linux X11 | Linux Wayland |
|------|---------------|-----------|-----------|---------------|
| 无边框窗口 | 预期支持 | 预期支持 | 预期支持 | 预期支持 |
| 透明/半透明窗口 | 预期支持：DWM / `WS_EX_LAYERED` | 预期支持：NSWindow 非不透明 | 条件支持：依赖合成器 | 预期支持：ARGB surface |
| Always-on-top | 预期支持：`HWND_TOPMOST` | 预期支持：floating level | 预期支持：`_NET_WM_STATE_ABOVE` | 条件支持：layer-shell/合成器 |
| 点击穿透 | 预期支持：window hit-test | 预期支持：`ignoresMouseEvents` | 预期支持：XShape input region | 预期不支持：核心协议无统一能力 |
| 桌面附着 | 条件支持：WorkerW hack，脆弱 | 条件支持：授权与行为风险 | 预期支持：BELOW/DOCK | 条件支持：layer-shell background |
| 多显示器 | 预期支持 | 预期支持 | 预期支持 | 预期支持 |
| DPI 缩放 | 预期支持：per-monitor v2 | 预期支持：`backingScaleFactor` | 条件支持：无统一每屏 DPI | 预期支持：合成器缩放 |
| 显示器热插拔恢复 | 预期支持：`WM_DISPLAYCHANGE` | 预期支持：`NSScreen` 通知 | 条件支持：RandR/桌面环境差异 | 预期支持：output 事件 |
| 快捷键注册 | 预期支持 | 预期支持 | 条件支持：依赖桌面环境 | 条件支持：依赖全局快捷键协议 |
| 透明 + 点击穿透同时可用 | 预期支持 | 预期支持 | 预期支持 | 预期不支持 |

## 2. 关键能力详解

### 2.1 点击穿透（Click-through）

| 平台 | 机制 | 备注 |
|------|------|------|
| Windows | 设置 `WS_EX_TRANSPARENT`（不参与 hit-test）+ `WS_EX_LAYERED`（每像素 Alpha）；交互时清除该样式 | 窗口整体生效，无法按像素区域 |
| macOS | `window.setIgnoresMouseEvents(true)` | 同上，整体生效 |
| X11 | XShape 将输入区域设置为空 shape；`_NET_WM_WINDOW_TYPE_DOCK` 辅助 | 整体生效；无合成器时仍可穿透（输入区域独立于渲染） |
| Wayland | **核心协议不提供**。无标准机制让普通窗口忽略输入。可选：layer-shell 提供部分（`exclusive_zone` 只影响布局抢占，不解决点击穿透） | **P0 最大风险，必须降级** |

**Wayland 降级策略**（P0 必须实现并实测）：
1. 检测合成器能力（是否支持 `wlr-layer-shell`、`zwlr_foreign_toplevel` 等）。
2. 支持 layer-shell → 用 layer-shell 实现置顶与桌面附着，但点击穿透仍受限 → 降级为「编辑模式常驻边框 + 显式最小化控件」。
3. 不支持 → 走 XWayland（通过 `GDK_BACKEND`/环境检测），在 XWayland 内可正常点击穿透，但存在缩放/合成差异。
4. 最坏情况：纯 Wayland 且无 layer-shell → 禁用点击穿透，始终可交互，提供「收起/悬浮态」替代，并明确标记降级原因。

### 2.2 透明窗口

| 平台 | 机制 | 注意 |
|------|------|------|
| Windows | `WS_EX_LAYERED` + per-pixel alpha，或 DWM 下的 `SetLayeredWindowAttributes`；Slint 侧设置透明背景 | 需要 DWM 合成 |
| macOS | `NSWindow.isOpaque = false`，背景透明；`NSWindow.CollectionBehavior` 控制 Spaces | 全屏切换 Spaces 时需处理 |
| X11 | 需要合成器（compositor），否则透明显示为黑色 | 无合成器时降级为不透明背景 |
| Wayland | ARGB 表面 + xdg-shell；合成器负责混合 | 由合成器决定 |

### 2.3 Always-on-top 与桌面附着

- Windows：`HWND_TOPMOST`（置顶）。桌面附着用 WorkerW 是已知脆弱 hack，P0 标记为「可选实验」，不作为验收项。
- macOS：`NSWindow.Level = .floating/.statusBar`；桌面附着需要辅助功能授权，P0 不做。
- X11：`_NET_WM_STATE_ABOVE` / `_NET_WM_STATE_BELOW`。
- Wayland：`wlr-layer-shell` 的 `layer`（top/overlay/bottom/background）提供置顶与背景；非 wlroots 合成器（GNOME Mutter）不支持 → 走 XWayland 或普通窗口。

## 3. 与 Slint 的关系

- 透明背景、窗口标志、Always-on-top 等基础项尽量通过 winit 的 `WindowAttributesExt*` 设置。
- 无法在 winit 表达的（Windows click-through、macOS ignoresMouseEvents、X11 XShape）通过 `raw-window-handle` 获取原始窗口句柄后，在 `floatile-platform` 内用平台 API 直接调用，**不得外泄到业务层**。

## 4. 降级策略汇总

| 场景 | 降级行为 |
|------|---------|
| 无合成器（X11） | 背景不透明、置顶不可用、无动画；记录环境标记 |
| 纯 Wayland 无 layer-shell | 点击穿透禁用、置顶降级为普通窗口；提供手动置顶控件 |
| 无 GPU / 驱动异常 | Slint 软件渲染器回退；记录 GPU 信息到诊断日志 |
| 显示器热插拔找不到原屏 | 布局落回主屏并标记 `lost_monitor`，不丢数据 |
| 高 DPI 混合 | 以窗口所在屏的 scale factor 为准，跨屏拖动时重算逻辑尺寸 |

## 5. P0 实测回填栏

> 每个平台跑一遍：透明 → 置顶 → 点击穿透 → 编辑模式 → 拖拽 → 多屏 → 热插拔 → 热插拔后布局恢复。
> 记录：实测结果、环境（WM/合成器/版本）、截图、性能。

| 日期 | 平台/环境 | 透明 | 置顶 | 穿透 | 编辑模式 | 多屏 | 热插拔 | 备注 |
|------|-----------|------|------|------|----------|------|--------|------|
| 2026-08-13 | Windows 11，DWM 合成桌面，GPU 由 Slint 默认后端（dev 构建，commit 待定） | ✅ 无边框实测（`WS_POPUP`，`CAPTION/BORDER/SYSMENU` 清除）；圆角外角落像素 Alpha=0 透明生效 | ✅ 探测返回 `always_on_top=true`，实测 ex-style 含 `WS_EX_TOPMOST`，窗口盖过普通窗口 | ✅ Edit 模式 `TRANSPARENT=false`、Show 模式 `TRANSPARENT=true`（+LAYERED）；模式切换与全局热键 Ctrl+Shift+E 实测联动 | ✅ 编辑控件（边框/设置/展示/删除/缩放手柄）显示，Show 模式隐藏；拖拽（WM 拖动）与缩放（手柄 274x158→442x222）实测 | 未测 | 未测 | 探测日志：`kind=Windows click_through=true always_on_top=true`；窗口进程存活无崩溃；winit 0.30 顶层窗口 `with_decorations(false)` 不生效，由 `floatile-platform` 创建后强制移除（已记录）；穿透的视觉 Alpha 混合需在真实使用场景复核 |
