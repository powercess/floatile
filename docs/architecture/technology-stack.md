# 技术栈与版本策略

> 状态：Accepted
> 范围：P0

## 1. 已选工具栈

| 层 | P0 选择 | 约束与理由 |
|---|---|---|
| 语言 | Rust 2024，MSRV 1.97 | 原生跨平台、WASM guest 同语言；工具链固定到 `rust-toolchain.toml`。 |
| 宿主 UI | Slint 1.17.x + winit 0.30.x | 原生 renderer 与窗口；ADR-0001 规定 Slint 只在宿主内使用，插件不提供 `.slint`。 |
| 插件 UI | Floatile UI IR v1（`floatile-ui-schema` 已实现） | 版本化静态组件树 + State/Event schema；Rust/TypeScript SDK 同源生成；IR→Slint renderer 路径需 P0 spike。 |
| 平台 API | windows-sys 0.52.x + tray-icon 0.24.x + x11rb 0.13.x（`randr`、`shape`）+ objc2 0.6.x（`app-kit`/`foundation`）+ mach2 0.6.x | 平台句柄、Windows 窗口与通知区操作、X11 compositor/SHAPE/EWMH/RandR/热键探测，以及 macOS NSWindow/NSScreen/进程指标与 Carbon 全局热键；只允许 `floatile-platform` 直接依赖。`tray-icon` 本阶段仅在 Windows target 启用并关闭默认 GTK/libxdo features。 |
| 插件 ABI | WIT + WASM Component Model，guest `wasm32-wasip2` | 版本化接口、无原生句柄；`wit/` 为唯一源。 |
| 插件 runtime | Wasmtime 47 + wasmtime-wasi p2（S5b 已引入，空 WASI 上下文实现零 ambient） | Component Model、异步调用、fuel 与资源限制；引入时固定兼容版本组。 |
| TypeScript adapter | private SDK + ADR-0003 no-go runtime | TypeScript 作者契约、Rust Registry 同源 UI builder、WIT 1.2 adapter 与共享 conformance vectors 已实现；componentize-qjs 未发布修复候选已过三系统构建和 Linux 行为/安全/资源门，但上游发布、许可/NOTICE 与额外 `init` export 未决。暂不启用 CLI 模板或公共发布。 |
| 异步 | Tokio（runtime/services 已引入，S5b） | 后台 I/O/runtime；Slint 主线程只跑事件循环。 |
| 存储 | SQLite + rusqlite bundled | 单文件、事务、跨平台；migration 前向追加（v1 layout 表已落地） |
| 序列化/错误 | serde、serde_json、thiserror；manifest JSON Schema 单源使用 schemars + jsonschema | 契约类型、结构化校验错误；类型→schema 同源生成独立 manifest.schema.json 产物 |
| 可观测性 | tracing、tracing-subscriber | 结构化 span；审计使用独立 target 并脱敏。 |
| 包与接口工具 | `wasm-tools`、`wit-bindgen`、zip、semver（按阶段引入） | 组件校验、绑定生成、包校验和版本兼容。 |
| 质量门禁 | rustfmt、Clippy、Cargo test、cargo-deny、GitHub Actions | 本地与 CI 使用相同 Cargo 命令。 |

ADR-0001 禁止 P0/MVP 插件携带第三方 `.slint`。P0 性能验证聚焦 UI IR 解析/校验、State Patch、
宿主 Slint renderer 与 Wasmtime 调用；不得为了复用动态编译器重新暴露 Slint 源码。

Cargo workspace 统一共享依赖与 lint，成员只通过 `workspace = true` 继承：
<https://doc.rust-lang.org/stable/cargo/reference/workspaces.html>。

## 2. 版本策略

- `rust-toolchain.toml` 固定完整 Rust patch 版本；`workspace.package.rust-version` 固定 MSRV minor。
- 应用仓库提交 `Cargo.lock`。本地和 CI 默认 `--locked`，依赖更新必须是显式变更。
- 所有直接依赖先进入根 `[workspace.dependencies]`；crate 不自行选择不同版本。
- Slint/winit 和未来 wasmtime/wit-bindgen 是兼容版本组；升级时同一变更完成编译、契约、三平台
  构建与关键验收，不跨功能提交顺带升级。
- Slint 1.17.1 当前有两项精确的 unmaintained 传递依赖例外，见 `risks.md` R12；统一 UI 使纯
  `widget.ftui + wasm` 不接收第三方字体/SVG，但公开分发和任何第三方字体/SVG 前仍必须消除例外。
- 常规依赖使用兼容版本范围 + lockfile；只有确认上游兼容风险且有 ADR 时才用精确 `=` 固定。
- 禁止 `*` 版本和无 `rev`/`tag` 的 Git 依赖；新增 Git 依赖需 ADR 和 cargo-deny source 例外。
- workspace crate 均 `publish = false`，直到发布与许可策略单独通过评审。

## 3. 工具选择边界

- P0 不引入通用任务运行器；标准入口保持为可移植的 Cargo 命令。重复的跨平台复杂流程出现后，
  优先新增 workspace `xtask`，不堆积平台相关 shell 脚本。
- 平台 API 按目标使用 `windows-sys`、`objc2`、`x11rb`、`wayland-client`，但只允许
  `floatile-platform` 直接依赖。
- 网络能力未进入 P0；未来固定使用 rustls 路径，禁止为了便利默认引入系统 OpenSSL 或绕开
  HTTP Broker。
- 不使用 `cargo-component` 作为 P0 必需链路；先验证 stable Rust + `wasm-tools` + `wit-bindgen`。
- 不把 Node/DOM/WebView 作为 TypeScript SDK 的隐式宿主环境；adapter/runtime 不能引入 WIT/Broker
  之外的 ambient capability。
- `cargo-deny` 的 advisories/bans/sources 是持续门禁。licenses 检查在许可 ADR 前应保持失败，
  用于阻止分发，不得添加宽泛例外让它“变绿”。

## 4. 选型变更规则

替换 UI/runtime/存储/ABI、安全边界、任务系统或 CI 平台，必须新增 ADR，写明需求、候选项、
威胁与许可影响、迁移/回退方案和验证证据。仅新增普通库也必须说明为何标准库或已有依赖不足，
并检查维护状态、许可证、默认 feature、传递依赖与跨平台支持。
