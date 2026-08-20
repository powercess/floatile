# Workspace 与 crate 边界设计

> 状态：Accepted
> 目标：依赖单向、UI/ABI 单源、平台差异收敛、Broker 成为唯一宿主能力入口
> 插件 UI 决策：ADR-0001

## 1. 分层

```text
插件作者侧
  TypeScript SDK ─┐
                  ├─ ui schema + WIT → widget.ftui + plugin.wasm
  floatile-sdk ───┘
                         │
  floatile-cli ──────────┤ validate / build / inspect
    floatile-renderer ───┘（IR → 宿主控制的 Slint 源码文本，host-only）
                         ▼
宿主侧
  floatile-shell → floatile-runtime → floatile-plugin-api
        │                 │                    │
        │                 ├────────────→ floatile-ui-schema
        │                 └────────────→ floatile-services → floatile-store
        │                                      │
        └────────→ floatile-platform ←─────────┘

共享纯模型
  floatile-core          host domain/layout/permission decision types
  floatile-ui-schema     guest-safe UI IR/State/Event schema types
```

图只表达允许方向，不表示每条依赖现在都已落地。新增反向依赖必须更新本文并在不可逆时新增 ADR。

## 2. 硬边界

1. `floatile-core` 是纯宿主领域模型，不做 I/O，不依赖 runtime/UI/platform。
2. `floatile-ui-schema` 是纯、guest-safe 共享 crate；不依赖 Slint、Wasmtime、Tokio、
   SQLite、平台 API 或宿主服务。
3. `floatile-platform` 是唯一允许直接依赖 windows-sys/objc2/x11rb/wayland-client 和平台 unsafe 的
   crate。
4. `floatile-plugin-api` 只包含从 `wit/` 生成的 host bindings 与薄契约封装，不实现 capability。
5. `floatile-runtime` 运行不可信 WASM、管理 actor/State/budget；不得执行未经 Broker 的原生能力。
6. `floatile-services` 拥有 `PermissionBroker` 与 capability 执行；授权、scope/quota、执行和脱敏审计
   不得拆成可绕过的公开入口。
7. `floatile-shell` 拥有 Slint 主线程、画布和 UI renderer；只消费已验证 UI IR 与 State，不等待 WASM。
8. `floatile-sdk` 和 TypeScript SDK 是 guest 作者表面，不依赖任何 host crate/API；只能通过 WIT 调宿主。
9. `floatile-cli` 可以验证/生成包，但不能链接宿主 capability 实现来“顺便执行”插件。

## 3. crate 职责

### 3.1 `floatile-core`

- `PluginId`、`InstanceId`、布局/DPI/模式等纯领域类型。
- manifest、capability/grant/scope/quota 的纯数据与决策输入；无文件/数据库访问。
- host/runtime 使用的稳定错误分类与版本值对象。
- 不放 UI IR/WIT 生成物，避免 guest 为 UI 类型依赖全部宿主 domain。

### 3.2 `floatile-ui-schema`

- `widget.ftui` v1 的纯类型：组件树、State/Event schema、binding、有限 If/ForEach、animation、asset ref。
- UI component registry 的机器可读 schema 与版本。
- 无 I/O 的结构/限制验证；输入字节读取和包路径检查属于 CLI/runtime。
- Rust SDK builder/macro 与 TypeScript codegen 的共同语义源。
- 必须在 host 与 `wasm32-wasip2` 编译；不得引入宿主依赖。

如最终采用 schema-first code generation 而不增加 crate，必须在实现计划中给出同等的单源与 host/guest
一致性证明；不得手写两套 UI 类型。

S5a 已实现：IR 类型、组件 registry v1、State/Event schema 模型与校验、JSONPath 绑定路径解析、
结构/预算校验与契约测试，host 与 `wasm32-wasip2` 均可编译。renderer spike（IR→Slint，见
`floatile-renderer`）已选定路径二变体并证明参考时钟生成物可经 `slint-build` 编译；
`uiApiVersion` 版本轴 contract vectors、animation/asset 预算的进一步落地与运行时第三方插件
UI 渲染（依赖 interpreter/运行时编译 ADR）仍待后续切片。

### 3.3 `floatile-shell`

- Slint/winit 事件循环、窗口/Canvas、Edit/Show 与布局编排。
- 把已验证 UI IR 映射到宿主 Slint 组件；插件不提供 `.slint`。
- 在主线程应用 runtime 发来的有界 State snapshot/patch，并把声明过的 UI event 投递 runtime。
- 不解析不可信包、不执行 WASM、不直接实现 plugin capability。
- 内建 Reference Clock，用于与插件化时钟对比行为/性能。

### 3.4 `floatile-platform`

- 平台 trait/probe、窗口标志、穿透、置顶、热键、monitor/DPI/hot-plug、系统指标采样。
- Windows/macOS/X11/Wayland 实现与所有平台 unsafe。
- 返回能力状态和降级原因；上层不得按 OS 名猜测。

### 3.5 `floatile-plugin-api`

- `wasmtime::component::bindgen!` 的 host bindings；`wit/` 为唯一源。
- WIT import/export 的薄 adapter traits 与 engine API 版本。
- 不实现 Broker、Storage、Timer、UI renderer 或 Wasmtime Engine。
- 与 `floatile-sdk` 的 binding/version 由 CI contract test 对齐。
- 当前 host async bindings 已迁移到 ADR-0001 目标契约，`floatile-runtime` 已接入；
  契约测试与 CLI 包校验落地前不得标记为统一插件契约 Implemented。

### 3.6 `floatile-runtime`

- Wasmtime Engine/Linker/Store、Component 验证/实例化、fuel/memory/epoch/timeout。
- 每实例 actor、bounded queue、严格串行 lifecycle、取消与 shutdown。
- 加载并验证 `widget.ftui`，维护 Config/State，原子应用已校验 State Patch。
- WIT host adapter 只持 `InstanceContext + PermissionBroker` 门面；不得持 service/OS raw handle。
- 把 UI snapshot/patch 通过有界通道发送 shell；不直接操作 Slint。
- trap/restart/isolation 与宿主存活保证。
- S5b 已实现：Wasmtime 47 + 空 WASI 上下文（零 ambient）、fuel/内存限制、串行 actor、State Patch
  原子应用、WIT adapter 经 Broker，`clock-wasm` 集成测试（start/1Hz/deny/fuel trap 存活）通过。

### 3.7 `floatile-services`

- `PermissionBroker`：registry、grant、scope、quota、环境能力、decision cache、redacted audit。
- Timer/Storage/Metrics/Theme/Clock/Log capability 的实现。
- 固有能力和声明能力使用同一 Broker 入口；固有能力只是固定 grant/scope。
- 后续 Notification/Keyring/HTTP 仍必须经 Broker；P0 不留可调用 stub 假装实现。
- S5b 已实现：Broker 决策/配额/脱敏审计（target `floatile::audit`）与七个能力服务；
  SQLite 审计持久化、decision cache 与真实容量数据待后续切片。

### 3.8 `floatile-store`

- SQLite open/migration/transaction。
- layout、plugin metadata、private KV、audit_log；新增前向 migration，禁止修改已发布 migration。
- 不做 permission 决策，不向 plugin 暴露连接/SQL/path。

### 3.9 `floatile-sdk`

- Rust guest SDK，目标 `wasm32-wasip2`。
- re-export 生成的 guest bindings，但普通作者只使用 `Widget/State/View/Event/Context`。
- UI builder/proc macro、State/Event schema、manifest capability 候选与 export glue。
- capability wrapper 保留稳定错误，不暴露 raw generated module/handle。
- `floatile-sdk-macros` proc-macro crate 已拆分：`#[derive(State)]` 生成 schema + initial。
- S5c 已实现：`Widget<State,Event>` trait、`View` builder、`Context` 运行时封装（state/log/
  clock/timer/storage/metrics/theme）与 `impl_export_widget!` 导出适配；clock-wasm 已改用作者
  SDK（作者不手写 WIT）。作者级 `Event` 类型化（`FromWidgetEvent`）已落地；build-time UI IR 生成
  仍待后续切片。

### 3.10 TypeScript SDK（非 Cargo workspace crate）

- `@floatile/sdk` 组件/State/Event/Context 类型和 JSX build transform。
- 由 CLI 管理的 TypeScript→WASM Component adapter；必须实现同一 WIT/world/error/budget。
- 不提供 Node/DOM/文件/网络等 ambient API；具体 runtime 选择需独立 ADR 与性能/隔离证据。

### 3.11 `floatile-cli`

- `new/dev/check/test/preview/build/inspect/migrate`。
- 生成 UI IR、State/Event schema、bindings 与 manifest；插件作者不编辑生成物。
- manifest/UI/WASM/assets/archive 的正反例校验与可复现打包。
- `--json --no-interactive` 是 CI/Agent 稳定接口。
- dev/test 使用 mock capability 或受控 runtime，不绕过生产 Broker 语义。

### 3.12 `floatile-renderer`

- host-only；依赖 `floatile-ui-schema`（含 `validate_document`），不依赖 Slint/Wasmtime 运行时。
- 把已验证 `widget.ftui` 结构化生成为宿主控制的 Slint 源码文本（ADR-0001 路径二变体），
  输出 `component <PluginUI>` 内容组件 + binding/event 槽位。
- 生成前独立复验预算/结构；所有字符串值经结构化转义，组件/属性/回调名由本 crate 生成。
- 参考时钟由 `floatile-shell/build.rs` 调本 crate 生成并通过 `slint-build` 编译（可编译证据）；
  运行时第三方插件 UI 渲染待 interpreter/运行时编译 ADR（本 crate 输出契约不变）。

## 4. 事实源

| 契约 | 唯一源 | 生成/消费方 |
|---|---|---|
| Host/guest ABI | `wit/` | plugin-api、Rust/TS adapter、runtime、contract tests |
| UI IR/components | UI schema source / `floatile-ui-schema` | Rust/TS SDK、CLI、runtime、shell renderer |
| Package metadata | manifest schema | CLI、PluginManager、docs examples |
| Capabilities | capability registry | manifest schema、Broker、SDK wrappers、docs/tests |
| Platform behavior | platform probe + platform matrix evidence | shell/services、docs |

任何修改必须一次同步全部消费者；CI 检查生成物无 drift。不得复制常量字符串作为“单源校验”。

## 5. 目录目标

```text
Cargo.toml
crates/
  floatile-core/
  floatile-ui-schema/       # IR/组件/State/Event schema 单源（S5a 已实现）
  floatile-shell/
  floatile-platform/
  floatile-plugin-api/
  floatile-runtime/
  floatile-services/
  floatile-store/
  floatile-sdk/
  floatile-renderer/          # IR → 宿主控制 Slint 源码，host-only（S5a renderer spike）
  floatile-cli/
wit/
  floatile-widget.wit
schemas/
  manifest-v1.schema.json
  floatile-ui-v1.schema.json
sdk/
  typescript/               # @floatile/sdk；引入前需 runtime ADR
plugins/
  clock-wasm/               # Rust 参考插件
  clock-typescript/         # TypeScript adapter 通过后加入
tests/fixtures/
  evil-plugin/
docs/
```

文档列出目标不代表目录已实现；状态必须以 requirements/init-plan 与代码为准。

## 6. 编译与测试矩阵

| 单元 | 目标 | 必须验证 |
|---|---|---|
| core/ui-schema | host + wasm compatible | pure tests、serde/schema vectors |
| platform | host 三端 | compile + real platform evidence |
| plugin-api/runtime/services/store/shell | host 三端 | check/clippy/test/release |
| Rust SDK/clock | `wasm32-wasip2` | component validate + host contract |
| TypeScript SDK/clock | selected adapter target | same contract vectors + resource data |
| CLI | host 三端 | package adversarial corpus + deterministic output |

完整门禁按 `docs/development/workflow.md`；CI 构建不能替代真实 UI/平台/性能/安全证据。

## 7. 冻结与变更

- 修改 WIT、UI IR、manifest、capability registry、crate 方向或线程模型必须同步架构与契约测试。
- 新 UI component 是公开兼容表面；先进入 schema、双 SDK、renderer、preview、文档和测试，再发布。
- 第三方 `.slint`、HTML/WebView、原生插件或第二 host API 需要新 ADR，不能作为“临时开发模式”。
- TypeScript runtime 选择、签名/分发与许可各自需要独立 ADR。
