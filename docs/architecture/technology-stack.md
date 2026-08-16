# 技术栈与版本策略

> 状态：Accepted
> 范围：P0

## 1. 已选工具栈

| 层 | P0 选择 | 约束与理由 |
|---|---|---|
| 语言 | Rust 2024，MSRV 1.97 | 原生跨平台、WASM guest 同语言；工具链固定到 `rust-toolchain.toml`。 |
| UI | Slint 1.17.x + winit 0.30.x | 原生后端、透明窗口与运行时 `.slint` 解释能力；许可和动态编译性能是 P0 风险。 |
| 平台 API | windows-sys 0.52.x + x11rb 0.13.x（`randr`、`shape`） | 平台句柄、Windows 窗口操作，以及 X11 compositor/SHAPE/EWMH/RandR/热键探测与操作；只允许 `floatile-platform` 直接依赖。 |
| 插件 ABI | WIT + WASM Component Model，guest `wasm32-wasip2` | 版本化接口、无原生句柄；`wit/` 为唯一源。 |
| 插件 runtime | Wasmtime（计划在 S5 引入） | Component Model、异步调用、fuel 与资源限制；引入时固定兼容版本组。 |
| 异步 | Tokio（计划在需要后台服务时引入） | 后台 I/O/runtime；Slint 主线程只跑事件循环。 |
| 存储 | SQLite + rusqlite bundled | 单文件、事务、跨平台；migration 前向追加（v1 layout 表已落地） |
| 序列化/错误 | serde、serde_json、thiserror | 契约类型、结构化校验错误；应用入口可统一报告。 |
| 可观测性 | tracing、tracing-subscriber | 结构化 span；审计使用独立 target 并脱敏。 |
| 包与接口工具 | `wasm-tools`、`wit-bindgen`、zip、semver（按阶段引入） | 组件校验、绑定生成、包校验和版本兼容。 |
| 质量门禁 | rustfmt、Clippy、Cargo test、cargo-deny、GitHub Actions | 本地与 CI 使用相同 Cargo 命令。 |

Slint 官方同时支持编译期和运行时加载 `.slint`；Floatile 的第三方 UI 需要后者，因此 P0 必须实测
`slint_interpreter::Compiler` 的耗时与受限输入行为：
<https://docs.slint.dev/latest/docs/rust/slint_interpreter/>。

Cargo workspace 统一共享依赖与 lint，成员只通过 `workspace = true` 继承：
<https://doc.rust-lang.org/stable/cargo/reference/workspaces.html>。

## 2. 版本策略

- `rust-toolchain.toml` 固定完整 Rust patch 版本；`workspace.package.rust-version` 固定 MSRV minor。
- 应用仓库提交 `Cargo.lock`。本地和 CI 默认 `--locked`，依赖更新必须是显式变更。
- 所有直接依赖先进入根 `[workspace.dependencies]`；crate 不自行选择不同版本。
- Slint/winit 和未来 wasmtime/wit-bindgen 是兼容版本组；升级时同一变更完成编译、契约、三平台
  构建与关键验收，不跨功能提交顺带升级。
- Slint 1.17.1 当前有两项精确的 unmaintained 传递依赖例外，见 `risks.md` R12；该例外只允许
  内部 S1，进入 S5 前必须消除。
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
- `cargo-deny` 的 advisories/bans/sources 是持续门禁。licenses 检查在许可 ADR 前应保持失败，
  用于阻止分发，不得添加宽泛例外让它“变绿”。

## 4. 选型变更规则

替换 UI/runtime/存储/ABI、安全边界、任务系统或 CI 平台，必须新增 ADR，写明需求、候选项、
威胁与许可影响、迁移/回退方案和验证证据。仅新增普通库也必须说明为何标准库或已有依赖不足，
并检查维护状态、许可证、默认 feature、传递依赖与跨平台支持。
