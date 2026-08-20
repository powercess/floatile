# Floatile 项目需求基线

> 状态：Accepted
> 范围：P0
> 基线：0.1（从现有 P0 设计、验收、安全与插件文档归并）

## 1. 产品问题

Floatile 要让用户在桌面上长期运行轻量、可布局的浮动 Widget，同时让第三方插件扩展行为而不能
直接获得宿主文件、网络、命令或原生窗口能力。P0 先回答“跨平台窗口能力和沙箱插件链路是否
可行”，不等同于完整产品 MVP。

主要参与者：桌面用户、Widget 插件开发者、宿主维护者。P0 优先保证宿主可控、能力可降级、
失败可观测，再追求插件数量或高级定制。

## 2. P0 功能需求

| ID | 需求 | 对应验收 |
|---|---|---|
| FR-WIN-01 | 宿主必须提供透明、无边框、置顶的 Widget 窗口，并明确不支持能力的降级行为。 | F1、F2、F13 |
| FR-MODE-01 | 宿主必须统一管理 Edit/Show 状态；插件不得自行切换点击穿透或宿主控件。 | F3、F4 |
| FR-LAYOUT-01 | 用户必须能拖拽、缩放 Widget，并以逻辑像素保存位置、尺寸、层级和屏幕标识。 | F5、F6、F9 |
| FR-DISPLAY-01 | 多显示器、DPI 与热插拔后必须按定义恢复布局；原屏缺失时降级到主屏并留痕。 | F7、F8 |
| FR-REF-01 | 宿主必须包含每秒更新的原生时钟，作为窗口与性能基线。 | F10 |
| FR-PLUGIN-01 | 宿主必须加载由统一 Floatile UI 与 WASM Component 组成的 Widget；Rust/TypeScript SDK 生成相同 `widget.ftui + plugin.wasm` 契约，host/guest 只通过版本化 WIT 通信。 | F11 |
| FR-PERM-01 | 插件所有宿主能力必须经 deny-by-default Broker 检查、配额和审计。 | F12 |
| FR-PACK-01 | dev 包安装必须校验 manifest、UI IR、WASM、assets、引用文件、规范化路径、归档结构与资源预算。 | F11、F12 |
| FR-PROBE-01 | 平台能力必须在运行时探测；产品逻辑消费能力结果，不根据 OS 名称猜测。 | F3、F13 |

详细操作标准以 `../architecture/p0-acceptance.md` 为准。

## 3. 非功能需求

| ID | 要求 |
|---|---|
| NFR-SEC-01 | 默认零权限；WASM 内存隔离；fuel/内存/调用频率有上限；拒绝不能拖垮宿主。 |
| NFR-SEC-02 | manifest、zip、UI IR、WASM、assets、配置、State Patch 与插件参数一律按不受信任输入处理。 |
| NFR-OBS-01 | 插件加载、能力决策、模式与窗口事件使用结构化 tracing；敏感参数必须脱敏。 |
| NFR-PERF-01 | release 构建达到 P0 验收定义的 CPU、RSS、首帧、帧率与拒绝路径目标。 |
| NFR-PORT-01 | Windows、macOS、Linux X11 可构建；Wayland 按能力矩阵分级，不伪装成完全一致。 |
| NFR-MAINT-01 | crate 依赖单向；平台差异只进 `floatile-platform`；WIT、UI schema、capability registry 各自只有一个源；Rust/TypeScript SDK 不得语义分叉。 |
| NFR-REPRO-01 | 固定 Rust 工具链、提交 `Cargo.lock`，CI 使用 `--locked`。 |
| NFR-LEGAL-01 | Slint 与仓库许可未决前禁止对外分发。 |

## 4. P0 非目标

插件市场、签名与自动更新、多画布、凭证托管、网络 Broker、跨插件通信、Sidecar、第三方 `.slint`、
HTML/WebView、原生插件和完整无障碍均不在 P0。接口可以预留，但不得以未实现 stub 宣称需求完成。

## 5. 当前实现差距（2026-08-18）

| 范围 | 状态 | 主要差距 |
|---|---|---|
| Workspace/核心类型 | 部分实现 | 多数 crate 仍为模板，依赖边界尚缺架构测试 |
| S1 窗口与原生时钟 | 部分验证 | 透明、无边框、置顶、拖拽已有 Windows、Linux Xvfb 与 VMware Xfce/Xorg 证据；macOS 15.7.5（Apple M4）已实测无边框置顶窗口、布局持久化与恢复；Wayland 协议层已实测（headless weston：探测正确、置顶/穿透显式降级）；时钟按 UTC 秒数计算且未标注时区 |
| S2 桌面交互 | 部分验证 | Edit/Show、Windows 与 X11 点击穿透、恢复热键、拖拽与缩放已落地；Linux Xvfb 与 VMware Xfce/Xorg 已验证穿透往返，Xfce 已验证窗口重映射后的输入区重同步；物理多显示器、DPI 与热插拔降级（F7/F8）未验证 |
| 平台抽象 | 部分实现 | 能力状态包含明确降级原因；Windows 窗口能力、X11 compositor/SHAPE/EWMH/RandR 探测与 macOS 探测（`PlatformKind::MacOS`）、点击穿透（`NSWindow.ignoresMouseEvents`）、NSScreen 显示器枚举、mach 进程指标与 Carbon 全局热键已落地；尚无统一四平台 trait 与 Wayland 协议探测 |
| 布局/存储 | 部分验证 | 核心层已有强类型 monitor/DPI/物理坐标模型和平台无关恢复算法；SQLite v2 migration 已持久化物理尺寸、scale factor 与 `lost_monitor`；shell 已接入启动保存/恢复（位置/尺寸/模式）、拖动/缩放/模式/热键/退出保存与显示器变化重恢复，Xvfb+Openbox 与 macOS 单屏重启恢复已实测；真实多屏/DPI/热插拔与 Windows 实机验证待做 |
| 插件系统/WASM/SDK | 部分实现 | ADR-0001 已确定统一 UI、State Patch、串行 actor 与 Rust/TypeScript 同语义目标；`wit/floatile-widget.wit`、guest/host bindings 与 `clock.wasm` 已迁移到 ADR-0001 目标契约形状并通过 `wasm-tools validate`；`floatile-ui-schema`（IR/registry/schema 校验/绑定解析/预算/契约测试）已实现；`floatile-runtime` 已实现 Wasmtime 加载、串行 actor、State Patch 原子应用与 WIT adapter 接入 Broker，`clock-wasm` 集成测试（start/1Hz tick/update-state）通过；manifest 模型与 capability 注册表（core）已实现；Rust 作者 SDK（`Widget`/`View`/`Context`/`#[derive(State)]`/`impl_export_widget!`/`build_ftui`）已实现且 clock-wasm 已改用；CLI `new/validate/build` 命令（模板、`.floatile` 校验、manifest 生成 + 打包）已实现；TypeScript SDK、renderer spike 与契约测试仍缺失 |
| Permission Broker | 部分实现 | `floatile-services` Broker（deny-by-default 决策、scope/配额、脱敏审计 target `floatile::audit`）与 clock/log/timer/storage/metrics/theme 能力实现已完成并有测试；SQLite audit 持久化、恶意插件 fixture 与真实容量数据仍缺失 |
| 包工具链 | 部分实现 | `floatile-cli` 已实现 `.floatile` 包校验核心（zip 预算、路径穿越/碰撞/symlink/zip-bomb 拒绝、manifest/UI IR/WASM world 校验与正反例 corpus）；build 打包、schema 文件与原子安装待做 |
| 跨平台/性能证据 | 部分验证 | Windows、Linux Xvfb 与 VMware Xfce/Xorg 已回填 S1/S2 子集，两个 Linux X11 环境的 F3 穿透往返通过；Wayland 协议层（headless weston）首帧/CPU/RSS 与 F3 降级已回填；macOS 15.7.5 已回填 S1 子集（置顶 layer=3、无边框、布局恢复、首帧/RSS/CPU）；穿透/拖拽/缩放的交互实测待人工复核 |

## 6. 阶段门槛与未决策

进入 MVP 前必须完成 F1–F13 记录、风险假设复盘、许可 ADR、WIT/UI IR/manifest 版本冻结、
Rust/TypeScript SDK 一致性证据和三平台构建证据。以下问题不能由实现者私自假设：Slint 商业/开源
路线、TypeScript adapter/runtime、Wayland 产品承诺、插件签名信任模型、网络与凭证能力的上线阶段、
P0 性能目标是否成为发布 SLO。
