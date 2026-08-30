# Floatile P0 技术设计

> 版本：draft-0.2
> 状态：Proposed；插件 UI 边界由 ADR-0001 Accepted
> 范围：P0 技术可行性验证，不等同完整 MVP

## 1. P0 目标

- 在 Windows、macOS、Linux X11 验证透明、无边框、置顶、点击穿透和 Edit/Show。
- 在 Wayland 验证能力分级与显式降级。
- 验证多显示器、DPI、热插拔布局恢复和 SQLite 持久化。
- 交付硬编码 Reference Clock，作为窗口与性能基线。
- 验证统一 Floatile UI：Rust/TypeScript SDK 生成相同 `widget.ftui`，宿主使用 Slint 渲染但插件不
  接触 Slint。
- 交付 Rust 与 TypeScript 插件化时钟，通过相同 WIT、State/Event、权限与行为向量。
- 验证 Wasmtime Component、串行实例 actor、State Patch 和 deny-by-default PermissionBroker。
- 交付恶意插件拒绝、配额、trap、宿主存活和脱敏审计证据。
- 完成 F1–F13、平台矩阵、性能、风险假设和版本选择记录。

P0 不做：插件市场、签名/更新、网络/文件/命令/secret、跨插件通信、Sidecar、第三方 `.slint`、
HTML/WebView、原生插件、多种插件类型和完整无障碍。

## 2. 架构总览

```text
floatile-shell
├─ Slint/winit main loop · Canvas · Edit/Show · layout
├─ Floatile UI renderer ← validated widget.ftui + State snapshot
├─ PluginManager ← manifest/package validation
├─ floatile-runtime
│  ├─ per-instance actor / bounded queue / State
│  ├─ Operation completion bridge / generation filter
│  └─ Wasmtime Component / fuel / memory / timeout
├─ floatile-services
│  ├─ PermissionBroker → log/clock/timer/storage/metrics/theme
│  └─ bounded Operation registry / deadline / cancel / typed results
├─ floatile-store
└─ floatile-platform

plugin package
├─ manifest.json
├─ ui/widget.ftui
└─ logic/plugin.wasm
```

关键边界：

1. **统一 UI**：插件 View 构建期编译，运行期只发 State Patch；Slint 不是插件 API。
2. **唯一 ABI**：WIT 是 host/guest 唯一调用源；UI IR 不能携带 host function。
3. **唯一能力入口**：固有/声明能力都经 Broker 的 identity、scope、quota、environment 与 audit。
4. **宿主权威模式**：Edit/Show、点击穿透、窗口控件、布局和平台降级由 shell/platform 管理。
5. **实例 actor**：同一实例 callback 严格串行；资源和预算按实例隔离，编译产物可只读共享。
6. **零 ambient capability**：插件不获得 raw WASI 文件/网络/环境、Slint/窗口/服务/数据库句柄。

详细插件架构见 `../plugin-sdk/plugin-system-architecture.md`。

## 3. 线程模型

```text
[Slint main thread]
  input → declared UI event ───────┐
  render ← validated State update │ bounded channels
  control snapshot / command ─────┤
                                  ▼
[Tokio/runtime workers]
  instance actor → Wasmtime async callback
                 → WIT import → Broker → service
                 → host-ui State Patch → validate/commit → UI queue
  Broker → bounded Operation workers → metadata completion
         → generation filter / try_send → instance actor
[shell control/supervisor workers]
  SQLite desired state + verified Installation/Config Schema
  → bounded reconcile/snapshot; runtime lifecycle → observed registry
```

- Slint 回调只用 `try_send` 排队 event，不同步等待 WASM、Tokio、SQLite、审计持久化或任何不可信输入；
  UI→runtime 队列容量 64，满载事件立即丢弃，worker 聚合记录 `ui:event-queue` 脱敏审计。
- runtime 每实例 bounded queue 严格串行；timeout/cancel 后才能处理下一事件。
- 跨回调长任务由宿主 Operation worker 托管；提交、完成、并发和 retained result 分别有界。完成信号
  不携带 payload，runtime 仅向相同 instance generation 非阻塞投递；旧代、满队列和关闭 actor 的结果
  立即丢弃。v1.1 已从 WIT 单源接入通用 cancel、元数据 completion 与首个 `storage:read` typed adapter。
- State Patch 在 worker 上解析、原子应用和 schema 校验，runtime State 为权威；SDK mirror 只在 host
  确认后提交，主线程只应用已验证 snapshot/diff。
- worker 每轮最多转发 8 个 UI event，再让出 State 投影机会；queue full、UI 拥塞、shutdown 和
  cancellation 都有明确错误/丢弃策略与 tracing。
- 实例控制命令、reconcile 动作和控制面快照均使用有界 channel；Slint timer 只做 `try_recv/try_send`。
  observed lifecycle 只存在于当前进程，不覆盖 SQLite desired state；手动 retry 仅清除所选实例的已隔离
  fingerprint，并由后台 worker 重新复核最新 Installation/Config。
- 第三方运行时窗口在一次宿主拖动/缩放结束时构造领域层验证过的 `WidgetLayout`，经容量 64 的
  `try_send` 队列交给 instance supervisor worker 写入 SQLite；启动时同一 worker 按 `InstanceId`
  读取布局并随启动动作交给 UI 线程恢复，Slint/winit 回调不执行数据库 I/O。
- Windows 单实例守卫同时持有当前会话命名互斥体和手动复位 activation event。重复启动进程只设置
  event 后退出；主进程 Slint timer 以零超时消费并显示控制面，不引入文件轮询、阻塞 IPC 或 Shell
  中的 Win32 调用。
- Windows 插件/控制中心启动表面不创建内建 Clock 原生窗口。恢复热键通过 platform 的
  `RegisterHotKey(NULL, ...)` 注册到运行 winit 的 UI 线程消息队列，仍由 event-loop msg hook 派发；
  欢迎表面才拥有 Clock HWND、窗口事件和渲染性能通知。
- Windows 原生插件包选择器由 platform 封装 Win32 common dialog 及私有 owner 句柄，在独立 worker
  阻塞等待用户；Slint 线程只设置 busy 和接收路径。选择结果仍进入 instance-control worker 的统一
  包预算、校验和原子安装路径。
- Windows 当前用户开机启动由 platform 封装 HKCU Run 注册表值。控制面 worker 读取/写入精确宿主
  命令，Slint 线程不访问注册表；`--background` 在没有 desired-running 实例时进入无窗口事件循环，
  仍注册托盘、线程热键和单实例 activation，普通二次启动可重新显示管理中心。

## 4. UI、渲染与 DPI

- Slint 1.17/winit 只在宿主内；默认 GPU renderer，软件 renderer 是显式降级。
- `widget.ftui` v1 是静态组件树、State/Event schema、binding、有限 If/ForEach、动画与 asset ref；
  无脚本或通用表达式运行时。
- Rust/TypeScript 组件与 schema 从同一源生成。P0 组件集保持最小，定制通过组合与受限 Canvas/Path。
- IR→Slint 是宿主内部 renderer spike：比较预编译通用 renderer 与“由结构化已验证 IR 生成宿主
  控制的 Slint 定义”两条路径。不得接受/拼接插件 Slint 源；在实测前不宣称任一路径已可行。
- Widget 内尺寸和宿主布局均使用逻辑像素；scale factor/物理尺寸只由 host/platform 提供。
- 透明、穿透、置顶是宿主窗口能力，插件组件不能请求原生窗口改变。

## 5. 布局与持久化

- 坐标相对期望 monitor 工作区原点，保存逻辑矩形、物理尺寸、scale factor 与稳定 monitor key。
- layout v2 保存 instance/plugin/monitor/rect/DPI/lost_monitor/z/mode/version/time。
- monitor 缺失时运行态回主屏并标记 lost，保留原 monitor-local 数据以便重新接入恢复。
- Config、State、Storage 分离：Config/Storage 持久化，UI State 默认不持久化。

## 6. 插件加载管线

1. PluginManager 以有界流读取 dev 目录或 `.floatile`，校验规范路径、归档预算和 manifest。
2. 独立检查 `manifestVersion`、`engineApiVersion`、`uiApiVersion` 与插件 semver。
3. 验证 `widget.ftui` 的 component registry、State/Event schema、binding、If/ForEach、Canvas 与 assets。
4. 验证 WASM 是目标 Component world，无未知/ambient imports，并施加 Engine/Store limits。
5. capability registry 校验 permissions，构造仅可收窄的 instance grants。
6. shell 从已验证 UI IR 创建宿主组件；runtime 创建 instance actor/State/queue/Store。
7. host 调 constructor/start；插件通过 `host-ui.update-state` 完成首个 UI 状态。
8. 任一步失败都回滚实例与临时安装，不显示可用状态，并记录安全的结构化错误。

## 7. Reference Clock 与插件时钟

- Reference Clock 保持内建，用于窗口/UI/性能基线。
- Rust clock：`wasm32-wasip2` + Rust SDK，timer 每秒事件后更新 `state.time`。
- TypeScript clock：相同 View/State/Event/权限和行为测试；具体 adapter/runtime 先经 ADR 与资源门。
- 两个插件生成相同语义的 UI IR，使用同一 WIT world；语言不是能力或兼容维度。
- F11 比较首次显示、每秒更新、Edit/Show 通知、timer deny、suspend/resume、stop 与资源指标。

## 8. PermissionBroker 与服务

- 固有能力：UI State、限速日志、只读 wall clock；固定当前 instance scope，仍经 Broker。
- 声明能力：timer、private storage、process metrics、theme；manifest 是上限，grant 可收窄。
- P0 不实现网络/文件/命令/secret；没有 WIT interface，也没有临时 host function。
- allow/deny/scope/quota/unavailable/invalid input 都有稳定错误与脱敏 audit。
- ADR-0004 Operation 的 submit/cancel/take 全部经同一 Broker；registry 原始执行入口不跨 crate 公开。
  P0 WIT 尚不暴露 Operation，首个 typed capability 接入时必须联动版本与 contract tests。

## 9. Runtime 与安全

- Wasmtime 启用 component model、async、fuel/epoch interruption、memory/table/resource limits；fuel 在
  constructor/lifecycle/event/timer/cleanup 每次调用前独立补充，默认墙钟预算 2 s，由共享 Engine 的
  10 ms epoch ticker 驱动相对 deadline。
- 对 manifest/archive/UI/WASM/config/State/event/assets 先限字节/数量/深度，再解析和语义校验。
- 插件 trap、超时、超内存、恶意 patch/事件洪泛只终止/暂停当前实例，宿主和其他实例存活。
- Slint 只消费宿主生成/验证的 UI，不编译第三方 `.slint`；第三方字体/SVG 在 R12 退出前禁用。
- `.slint`、HTML/WebView、native sidecar 不能作为 dev-only 逃生口。

## 10. 可观测性与诊断

- tracing span 始终包含 plugin_id/instance_id/event/capability，不记录 secret 或完整 State/Storage value。
- `floatile::audit` 记录 capability decision、quota、invalid input、runtime trap 和 package validation。
- UI patch 只记录大小、字段数、错误路径 hash、queue latency 和结果。
- CLI/runtime 错误有稳定 code；`--json` 输出供 CI/Agent 使用，自由文本不作为自动化契约。

## 11. P0 交付物

- [ ] 平台矩阵与三端可运行 host，Wayland 有真实降级证据
- [ ] Reference Clock + Rust/TypeScript plugin clocks
- [ ] WIT/UI schema/manifest/capability 单源与生成 drift 检查
- [ ] IR→Slint renderer spike 与布局/缓存/错误/资源/恶意 IR 证据
- [ ] Wasmtime actor、State Patch、Broker、budget 与 audit
- [ ] `.floatile` dev 包 validate/build/inspect 与恶意 corpus
- [ ] 布局持久化与真实多屏/DPI/hot-plug 证据
- [ ] evil-plugin 安全测试与宿主存活
- [ ] CPU/RSS/首帧/帧率/patch latency/拒绝延迟数据
- [ ] 风险假设、TypeScript runtime ADR、许可 ADR 与版本锁定

## 12. 冻结与后续

P0 结束冻结：WIT v1 语义、UI IR v1 语义、Broker 决策模型、manifest/package v1、SQLite 骨架、
坐标/DPI 模型和双 SDK 行为向量。实现细节编码可以演进，但不能静默改变公开语义。

签名/商店、HTTP/credential、Custom UI、插件类型扩展与发布 SLO 在后续阶段分别决策。任何 Custom
UI 必须用新 ADR 证明统一 UI/Canvas 的真实缺口以及隔离、许可、兼容和迁移方案。
