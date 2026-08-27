# Floatile 项目需求基线

> 状态：Accepted
> 范围：P0
> 基线：0.1（从现有 P0 设计、验收、安全与插件文档归并）

本文仍是 P0 范围、需求和验收映射的事实源。P0 之后把 Floatile 演进为开发者插件平台的方向、
领域模型和里程碑见[插件平台长期演进路线](plugin-platform-roadmap.md)；长期路线不降低或替代本文
尚未完成的验收与分发门禁。

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

## 5. 当前实现差距（2026-08-26）

| 范围 | 状态 | 主要差距 |
|---|---|---|
| Workspace/核心类型 | 部分实现 | 12 个 Rust crate 与两个 WASM fixture 已落地；PP-M1 已增加 InstallationRef、PluginInstance、受限 Config、desired state 与 generation 纯模型，依赖边界尚缺自动架构测试 |
| S1 窗口与原生时钟 | 部分验证 | 透明、无边框、置顶、拖拽已有 Windows、Linux Xvfb 与 VMware Xfce/Xorg 证据；macOS 15.7.5（Apple M4）已实测无边框置顶窗口、布局持久化与恢复；Wayland 协议层已实测（headless weston：探测正确、置顶/穿透显式降级）；时钟按 UTC 秒数计算且未标注时区 |
| S2 桌面交互 | 部分验证 | Edit/Show、Windows 与 X11 点击穿透、恢复热键、拖拽与缩放已落地；Linux Xvfb 与 VMware Xfce/Xorg 已验证穿透往返，Xfce 已验证窗口重映射后的输入区重同步；物理多显示器、DPI 与热插拔降级（F7/F8）未验证 |
| 平台抽象 | 部分实现 | 能力状态包含明确降级原因；Windows 窗口能力、X11 compositor/SHAPE/EWMH/RandR 探测与 macOS 探测（`PlatformKind::MacOS`）、点击穿透（`NSWindow.ignoresMouseEvents`）、NSScreen 显示器枚举、mach 进程指标与 Carbon 全局热键已落地；尚无统一四平台 trait 与 Wayland 协议探测 |
| 布局/存储 | 部分验证 | 核心层已有强类型 monitor/DPI/物理坐标模型和平台无关恢复算法；SQLite v2 migration 已持久化物理尺寸、scale factor 与 `lost_monitor`；shell 已接入启动保存/恢复（位置/尺寸/模式）、拖动/缩放/模式/热键/退出保存与显示器变化重恢复，Xvfb+Openbox 与 macOS 单屏重启恢复已实测；真实多屏/DPI/热插拔与 Windows 实机验证待做 |
| 插件实例内核（PP-M1） | 已实现，部分验证 | SQLite v4 实例模型和 `floatile instance create/list/get/configure/start/stop/delete` 已打通；创建/配置与 shell 恢复共用安装目录完整性和 Config JSON Schema 复验。shell 控制面提供安装/实例列表、Schema 驱动配置、desired 启停/删除、observed `starting/running/failed/stopped` 与手动 retry；SQLite、安装文件和配置解析均在有界后台 worker。runtime 只有在 WASM `start()` 成功后才报告 running，晚期退出按稳定 code 隔离。Linux X11/Xvfb 自动证据已验证同包双窗口、单实例安装缺失失败隔离和恢复后手动 retry；Windows、macOS、Wayland 控制面交互与真实桌面多窗口仍未验证。 |
| Rust 作者闭环（PP-M4） | 已实现，Xvfb 验证 | Rust SDK 的 WIT 发行快照受根事实源 drift 测试约束，生成模板可从三个独立 Cargo 包快照在仓库外目录构建；`new/check/test/dev/preview/build/install/run/inspect` 已串行通过 schema v1 自动化验收，`preview/dev/run` 使用 shell 所属真实 renderer/Slint/Wasmtime/Broker 宿主，`run` 创建精确 Installation 的持久实例并推进 generation。公开 SDK registry 上传继续受 NFR-LEGAL-01/许可 ADR 阻断；Windows、macOS、Wayland 作者窗口交互未验证。 |
| 插件系统/WASM/SDK | 部分实现 | ADR-0001 已确定统一 UI、State Patch、串行 actor 与 Rust/TypeScript 同语义目标；`wit/floatile-widget.wit`、guest/host bindings 与 `clock.wasm` 已迁移到 ADR-0001 目标契约形状并通过 `wasm-tools validate`；`floatile-ui-schema`（IR/registry/schema 校验/绑定解析/预算/契约测试 + `uiApiVersion` 版本轴 contract vectors）已实现；`floatile-renderer`（host-only，IR→宿主控制 Slint 源码 + binding/event 槽位，多层预算/结构复验与结构化转义）已实现；ADR-0002 已决策并经 `floatile-shell::runtime_ui` 落地运行时第三方插件 UI 渲染（`slint-interpreter` 运行时编译 renderer 输出为独立原生窗口：字节/结构/预算复验前置、自窗口挂 `floatile-platform` 置顶、沿 renderer binding/event 槽位 State 投影与输入事件回投；headless F12 恶意 IR 拒绝 + Xvfb 编译/实例化/投影/事件往返全绿；`spawn_runtime_ui` 已接入 shell，按持久实例的真实 ID/Config 启动独立窗口）；FTUI 解析/校验/renderer 已移到准备线程，UI event 桥容量 64、非阻塞丢弃并聚合审计；`floatile-runtime` 已实现逐调用 fuel、默认 5 s epoch 墙钟预算、内存限制、串行 actor、State Patch 原子应用与 WIT adapter 接入 Broker，并覆盖无限循环 timeout 与同 Engine peer 存活；生成组件已实例化进 shell 窗口（`build.rs` 写入 gitignore 源路径、宿主 `slint!` import 嵌入 `Clock`，运行时沿 binding 槽位投影权威 State，Xvfb 下参考时钟首帧/1 Hz 更新已实测），宿主凭 `slint!` 编译生成物、不引入 `slint-build`（规避 RUSTSEC advisory）；PP-M3 Capability Registry 已在 core 单源化稳定名称、暴露/参数/风险/执行类型、WIT/SDK/CLI/审计映射，并驱动 manifest schema、CLI 与 Broker 固有授权及 drift 测试；Rust 作者 SDK（`Widget`/`View`/`Context`/`#[derive(State)]`/`impl_export_widget!`/`build_ftui`）已实现且 clock-wasm 已改用；CLI `new/validate/build` 命令（模板、`.floatile` 校验、manifest 生成 + 打包）已实现；ADR-0003 的 Linux TypeScript runtime spike 已完成但结论为 no-go（StarlingMonkey 资源门失败、componentize-qjs 0.4.3 契约门失败），因此 TypeScript SDK/F11 仍未完成；动画/asset 预算向量、Slint interpreter 生产编译时延与三平台 UI heartbeat 证据仍缺失 |
| Permission Broker / Operation（PP-M2） | 已实现，部分验证 | `floatile-services` Broker（deny-by-default 决策、scope/配额、脱敏审计 target `floatile::audit`）与 clock/log/timer/storage/metrics/theme 能力实现已完成并有测试；脱敏审计已落 SQLite `audit_log`（store v3 + shell 运行时经 `with_audit_listener`）；恶意插件 fixture（`plugins/evil-wasm`）与安全集成测试已实现（拒绝 + 审计 + 宿主存活）。ADR-0004 的 Operation registry 已实现按实例/generation 的 identity、有界提交/完成/并发/结果预算、deadline、取消、迟到结果与过载；engine API v1.1 从 WIT 单源正式增加通用 cancel、元数据 completion 和首个 `storage:read` typed submit/take，Rust SDK 与真实 guest 往返测试已接通。动态撤权、真实容量数据和后续 capability adapter 仍缺失 |
| 包工具链 | 部分实现 | `floatile-cli` 已实现 `.floatile` 包校验核心（zip 预算、路径穿越/碰撞/symlink/zip-bomb 拒绝、manifest/UI IR/WASM world 校验与正反例 corpus）、`build` 打包+自校验、`inspect` 完整复验后输出版本化 manifest/版本轴/权限/预算/entry digest JSON 契约、`check` 在自动清理临时目录中复用正式构建/校验链并输出 metadata/wasm/ui/manifest/package 阶段契约，并按组件实际导入的 WIT function 对照 Registry 与 manifest 权限声明，以及 `install` 原子安装引擎（staging/逐文件 fsync/每文件+聚合 digest/原子 rename/install.json/同版本拒绝/失败零残留，含非法包安装期拒绝）；`floatile-core::install` 提供 InstallMeta 与内容 digest 单源，`floatile-shell::plugin_manager` 按 digest 复核后加载已安装 dev 包；config.schema 结构校验、独立 manifest JSON Schema、PP-M4 作者预览/运行闭环已落地；签名仍待做 |
| 跨平台/性能证据 | 部分验证 | Windows、Linux Xvfb 与 VMware Xfce/Xorg 已回填 S1/S2 子集，两个 Linux X11 环境的 F3 穿透往返通过；Wayland 协议层（headless weston）首帧/CPU/RSS 与 F3 降级已回填；macOS 15.7.5 已回填 S1 子集（置顶 layer=3、无边框、布局恢复、首帧/RSS/CPU）；穿透/拖拽/缩放的交互实测待人工复核 |

## 6. 阶段门槛与未决策

进入 MVP 前必须完成 F1–F13 记录、风险假设复盘、许可 ADR、WIT/UI IR/manifest 版本冻结、
Rust/TypeScript SDK 一致性证据和三平台构建证据。以下问题不能由实现者私自假设：Slint 商业/开源
路线、TypeScript adapter/runtime、Wayland 产品承诺、插件签名信任模型、网络与凭证能力的上线阶段、
P0 性能目标是否成为发布 SLO。
