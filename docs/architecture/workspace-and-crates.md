# Workspace 与 crate 边界设计

> 状态：Accepted
> 目标：依赖方向单向、跨平台差异收敛、插件 API 与宿主实现分离，保证长期可维护。

## 1. 依赖方向（禁止反向依赖）

```
floatile-sdk ──(guest, 编译到 wasm)──┬── 仅供插件开发者使用
                                     │
floatile-cli ◄──(打包/校验)──────────┤
                                     │
         floatile-shell ◄────────────┴── floatile-plugin-api (WIT host 绑定 + 契约类型)
              │  ▲                            ▲
              │  │                            │
   floatile-services ◄── floatile-core ◄── floatile-runtime
        │  │                                    │
        │  floatile-store                       │
        │       │                               │
        └───────┴── floatile-platform ◄─────────┘ (所有 OS 差异)
```

依赖规则：

- **每一条都是分层单向依赖**：`core ← runtime ← services/shell`。
- `floatile-platform` 是唯一允许依赖平台 crate（`windows-sys` / `objc2` / `x11rb` / `wayland-client`）的 crate。任何平台相关代码不得外泄。
- `floatile-plugin-api` 只依赖 `floatile-core`（类型）与生成的 WIT 绑定，是「契约 crate」，不得依赖任何运行时实现。
- `floatile-sdk` 是 guest 侧工具库，编译到 `wasm32-wasip2`，不得依赖宿主 crate。

## 2. 各 crate 职责

### 2.1 floatile-core（数据模型与纯逻辑）
- `PluginId` / `InstanceId` / `WidgetId`（强类型，禁止字符串互转滥用）。
- 坐标与 DPI 数据模型：`LogicalRect`、`PhysicalRect`、`MonitorKey`、`ScaleFactor`。
- `manifest` 结构体（serde）与校验（无 I/O）。
- 权限模型：`Capability`、`Grants`、`Scope`（纯逻辑，校验规则可单测）。
- 事件总线类型：`HostEvent`、`WidgetEvent`。
- 领域常量：`engineApiVersion`、包格式版本。

### 2.2 floatile-shell（宿主可执行 + 编排）
- `main`：初始化日志/审计、tokio、Slint 后端、画布。
- Canvas：widget 实例的创建/销毁/布局/层级。
- 编辑/展示模式状态机。
- 布局持久化编排（委托 `floatile-store`）。
- 窗口生命周期（创建/销毁/坐标同步），全部经由 `floatile-platform`。
- P0 的 Reference Clock（硬编码组件）。

### 2.3 floatile-platform（平台抽象）
- trait：`PlatformWindow`、`PlatformService`（窗口标志、穿透、置顶、桌面附着、监视器枚举、热插拔、系统指标）。
- impl：`windows.rs` / `macos.rs` / `linux_x11.rs` / `linux_wayland.rs`（feature-gate）。
- 能力探测：`CapabilityProbe::probe()` 返回 `PlatformCapabilities`（透明？穿透？layer-shell？），供上层降级。
- 平台矩阵文档即由此 crate 的探测逻辑驱动（可生成能力报告）。

### 2.4 floatile-plugin-api（宿主契约）
- 存放 WIT 宿主侧绑定（`wasmtime::component::bindgen!` 生成结果）与手工封装的「宿主能力 trait」。
- `HostCapability` trait 族：`HostStorage`、`HostTimer`、`HostMetrics`、`HostLog`。
- 声明这些 trait 必须实现「先过 Broker 再做事」。
- 供 `floatile-runtime` 实例化绑定，供 `floatile-sdk` 对齐签名（guest 与 host 共享同一 WIT 源）。

### 2.5 floatile-runtime（插件运行时）
- wasmtime `Engine` 配置（fuel、内存上限、异步、cache）。
- WIT 世界解析与 component 加载、实例化、资源生命周期。
- Slint 动态编译 `slint_interpreter::Compiler` + 组件实例化（作为 Widget Host）。
- `WidgetHost`：把 Slint 回调 ↔ wasm `handle_ui_event` 桥接。
- 资源配额与运行预算执行（调用频率、CPU、内存）。

### 2.6 floatile-services（宿主能力实现）
- `StorageService`（SQLite KV + 表结构）。
- `TimerService`（tokio 调度，按实例配额）。
- `MetricsService`（CPU/内存采样）。
- `NotificationService`（P0 预留）。
- `KeyringService` / `HttpBroker`（MVP/V1 实现，P0 只有 stub 接口）。
- 所有服务入口持 `Broker` 引用，先校验后执行。

### 2.7 floatile-store（持久化）
- SQLite 打开/迁移/事务封装。
- 表：`layout`、`plugin_meta`、`kv`、`audit_log`（见迁移 SQL）。
- 只被 `floatile-services` 与 `floatile-shell`（布局编排）使用。

### 2.8 floatile-sdk（插件开发者 SDK）
- 编译目标 `wasm32-wasip2`。
- 预置 wit-bindgen guest 绑定（re-export）。
- 便利宏/包装：`FloatWidget` trait + `#[float_widget]` 属性宏。
- 编译成组件的前置说明（使用 `wasm-tools component embed` 或 cargo config）。
- **发布时与 `floatile-plugin-api` 必须由同一 WIT 源生成，加 CI 检查签名一致性。**

### 2.9 floatile-cli（打包/校验工具）
- `floatile build`（收集 manifest/ui/logic/assets → 校验 → 打包 `.floatile` zip）。
- `floatile validate`（manifest 语义校验 + schema 校验）。
- `floatile sign`（V1 起；P0 只有骨架）。
- `floatile dev`（开发模式：监视目录、热重载触发，P0 骨架）。

## 3. 目录结构（cargo workspace）

```
Cargo.toml              # [workspace] members + 共享依赖版本表 [workspace.dependencies]
rust-toolchain.toml     # 锁定完整 Rust patch 版本 + wasm32-wasip2 target
crates/
  floatile-core/
  floatile-shell/       # src/main.rs 是宿主 bin
  floatile-platform/
  floatile-plugin-api/
  floatile-runtime/
  floatile-services/
  floatile-store/
  floatile-sdk/
  floatile-cli/
wit/                    # 单一 WIT 源（floatile:widget）
  floatile-widget.wit   # 或按目录拆分
plugins/
  clock/                # 硬编码参考组件（native Slint）
  clock-wasm/           # 插件化时钟（SDK 示例）
docs/
tests/
```

## 4. workspace 配置要点

- `[workspace.dependencies]` 统一定版本，单一事实源。
- rust-toolchain.toml：固定 P0 基线 `1.97.1`，targets 加 `wasm32-wasip2`。
- CI（GitHub Actions）三 OS 矩阵 + clippy `-D warnings` + `cargo test`。
- 依赖准入：`cargo-deny`（licenses 白名单、advisories、可疑来源 crate 阻断）。

## 5. 冻结规则

- WIT 源只放在 `wit/`，一次修改触发 `floatile-plugin-api` 与 `floatile-sdk` 两处重新生成。
- 任何新平台能力 → 先在 `floatile-platform` 加 trait 方法 + 能力探测，再决定是否需要权限。
- 任何跨 crate 类型移动都必须更新本文件，禁止绕过。

## 6. 编译矩阵（P0）

| crate | 目标 | 编译平台 |
|-------|------|----------|
| core / services / store | host | 三端 |
| platform | host（feature-gate: win/mac/x11/wayland） | 三端 |
| runtime / plugin-api | host | 三端 |
| shell | bin | 三端 |
| sdk | `wasm32-wasip2` | 任意（guest 交叉编译） |
| cli | host | 三端 |
