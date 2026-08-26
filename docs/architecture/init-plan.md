# P0 最小垂直切片实施计划

> 状态：Accepted
> 目标：以最小垂直切片跑通 P0 验收 F1–F13，暴露窗口层与 Wayland 风险。
> 原则：每次提交都可运行；每步结束跑一次「当前验收项」；三端差异只进 `floatile-platform`。
> 进度：S0 已完成；S1 有 Windows、Linux 与 macOS 子集证据；S2/S3 部分实现；Wayland 协议层已在
> headless weston 实测；ADR-0001 与插件/SDK 架构已确定，WIT/Component 链路已迁移到 ADR-0001 目标契约。

## 0. 当前基线（2026-08-20）

- Rust 1.97.1、`wasm32-wasip2`、rustfmt、Clippy、wasm-tools 已可用。
- Workspace、九个宿主/SDK crate、一个 WASM clock fixture、CI/依赖策略和工程文档已建立；Windows
  S2 窗口交互与 S3 SQLite layout CRUD 已部分落地。
- S1 已有 Windows 实测与 Linux Xvfb/Openbox/picom 证据；X11 合成器探测、无边框、置顶、拖拽和 `--perf` 诊断已落地。
- 物理 Linux X11 与 sway/GNOME Wayland 仍未验证；Wayland 协议层（headless weston 14.0.2）已验证
  探测与 F3/置顶显式降级；macOS 15.7.5 已验证探测、无边框置顶窗口和布局恢复，点击穿透/拖拽/
  缩放交互及真实多屏仍待复核。
- ADR-0001 已把插件 UI 从第三方 `.slint` 改为统一 `widget.ftui` + State Patch；插件系统、SDK、WIT、
  manifest、安全与 crate 文档已形成实施约束，但对应代码不算完成。
- `wit/`、guest/host bindings 与 `clock.wasm` 已迁移到 ADR-0001 目标契约（`host-ui`/`host-clock`、canonical State、
  统一 `start/handle-event/stop` 与稳定 guest error）并通过 `wasm-tools validate`；`floatile-ui-schema`
  S5a 切片已实现（IR/registry/schema 校验/绑定解析/预算校验/契约测试 + `uiApiVersion` 版本轴 vectors，
  host+wasm 可编译）。manifest 模型与 capability 注册表已在 `floatile-core` 实现。S5b 的 runtime actor +
  Broker + clock 集成测试已落地。`floatile-renderer`（host-only）已实现 S5a renderer spike 路径二变体：
  从已验证 IR 结构化生成宿主控制 Slint 源码 + binding/event 槽位，参考时钟生成物经 `slint-build` 编译
  通过。CLI 包校验联动（S6）、恶意插件 fixture（S7）已落地；运行时第三方插件 UI 渲染实现切片已按
  ADR-0002 用 `slint-interpreter` 落地（`floatile-shell::runtime_ui`：解析/复验/渲染/interpreter
  编译/自窗口/State 投影/事件回投，Xvfb 全绿，F12 恶意 IR 前置拒绝），可作 F11 统一插件契约完成证据。

## 1. 已完成脚手架

```
1. 固定 Rust 1.97.1 + target：`wasm32-wasip2`
2. 安装 wasm-tools（cargo install wasm-tools 或 binstall）
3. Cargo workspace + `Cargo.lock` + `.gitignore`
4. 创建 crates/* 九个目录（每个先空 lib/bin crate，仅 core 有骨架类型）
5. `rust-toolchain.toml`：1.97.1 + wasm32-wasip2
6. .github/workflows/ci.yml：三 OS × (fmt, clippy -D warnings, test, release build)
7. cargo-deny（licenses/advisories 准入）
8. docs 引用关系校验：README 指向 docs 索引
```

## 2. 垂直切片里程碑（每步有验收点）

### S1 — 透明无边框窗口（占位）
- 依赖：slint(winit)，floatile-shell bin。
- 做：创建透明无边框窗口、置顶、可拖拽。
- 验收：F1/F2/F5 首屏跑通；记录基线（RSS/CPU/首帧）。
- 关键验证：Linux 合成器是否存在 → 记录降级分支。

### S2 — floatile-platform 平台抽象
- 做：`Platform` trait + 四平台 impl（穿透/置顶/模式切换/监视器枚举）；`CapabilityProbe`。
- 验收：F3（穿透/编辑模式）、F7（多屏）。Wayland 走降级路径，产出矩阵回填。
- 工程约束：业务 crate 不得出现平台 API。
- 当前进度：Linux X11 子路径已实现 compositor/SHAPE/EWMH/RandR 实探测、点击穿透与幂等恢复热键；Xvfb 和 VMware Xfce/Xorg 单输出证据已回填，Xfce 已验证窗口重映射后的输入区重同步；真实多屏/DPI/热插拔与 Wayland 仍未验证。

### S3 — 画布 + 布局持久化
- 做：floatile-store（SQLite 迁移 v1：layout/kv/audit_log）、画布坐标模型、拖拽/缩放、热插拔恢复。
- 验收：F5/F6/F8/F9。
- 当前进度：核心层 monitor-local 布局恢复、主屏降级/原屏回归和边界钳制已实现；SQLite v1
  `layout` 可前向迁移到 v2 的 DPI/物理尺寸/`lost_monitor` 字段。shell 已接入启动保存/恢复
  （位置/尺寸/模式）、拖动/缩放/模式切换/热键/退出保存、显示器变化（Focused/Occluded）重恢复，
  Xvfb+Openbox 下拖拽→重启恢复与删除清库已实测；`kv` 仍未实现（`audit_log` 表已在 migration
  v3 落地，见 S7），真实多屏/DPI/热插拔实机验证待做。

### S4 — 硬编码时钟（Reference Widget）
- 做：内建时钟组件 + 每秒更新 + 编辑模式控件。
- 验收：F10；性能基线（空闲 CPU、首帧）。

### S5 — 统一 UI + 沙箱插件垂直切片

#### S5a — UI/WIT/manifest 单源契约

- 新增 `floatile-ui-schema` 或经评审的等价 schema-first 单源；定义最小组件、State/Event schema 与
  `widget.ftui` v1。
- 按 `wit-api-v1.md` 落地统一 lifecycle 与 `host-ui.update-state`；生成 host/Rust guest bindings，
  为 TypeScript adapter 输出同一 contract schema。
- manifest schema 改为 `widget.ftui + plugin.wasm`，实现版本轴与正反例 contract vectors。
- 验收：生成物无 drift；非法 UI/binding/patch/event/version 被拒；无 Slint/host handle 泄漏。
- 当前基线已有 ADR-0001 目标契约 `floatile:widget@1.0.0`：Rust guest/host bindings 与 `plugins/clock-wasm`
  已按统一 `start/handle-event/stop` lifecycle、`host-ui`/`host-clock` 与 canonical State 迁移并通过
  `wasm-tools validate`。`floatile-ui-schema` 的 IR/registry/schema 校验/绑定解析/预算校验与契约测试
  已落地（S5a 切片）。manifest 模型与 capability 注册表已在 `floatile-core` 实现。`floatile-renderer`
  已落地 S5a renderer spike 路径二变体（参考时钟生成物经 `slint-build` 编译通过）；`uiApiVersion`
  版本轴/正反例 contract vectors 已补。S5a 剩余：CLI 包校验（zip/路径/资源预算）联动与运行时第三方
  插件 UI 渲染实现（ADR-0002：`slint-interpreter` 运行时编译 renderer 输出）已在后续切片落地——
  前者见 S6，后者由 `floatile-shell::runtime_ui` 实现（Xvfb 编译+实例化+State 投影+事件回投全绿，
  F12 恶意 IR 前置拒绝），统一插件契约可标记为已实现。

#### S5b — Runtime actor + Broker

- Wasmtime Engine/Store limits、每实例 bounded serial actor、timeout/cancel/shutdown。
- State Patch 原子验证与有界 UI 投递；shell renderer 从已验证 IR 构建 Slint host UI。
- Broker 固有能力（UI/log/clock）与 timer 最小声明能力；allow/deny/quota/audit。
- 验收：Rust clock 1 Hz 更新；deny、超 patch、队列洪泛、fuel/内存 trap 后宿主存活。
- 已实现：`floatile-runtime`（Wasmtime 47 + 空 WASI 上下文、逐 guest 调用 fuel 补充、16 MiB
  默认内存限制、2 s 默认墙钟预算 + 10 ms epoch interruption、串行 actor、State Patch
  原子应用、WIT adapter 经 Broker）；`floatile-services` Broker 与 clock/log/timer/storage/metrics/theme
  能力；`clock-wasm` 集成测试（start/1Hz tick/update-state/deny 存活/fuel trap 存活）。renderer 侧：
  `floatile-renderer` 生成的 `ClockPluginUI` 组件已实例化进 shell 窗口：`build.rs` 把生成组件写到
  gitignore 的源路径，宿主 `slint!` 经 `import` 嵌入 `Clock` 窗口，运行时沿 renderer binding 槽位把
  权威 State 投影到宿主属性（Xvfb 下参考时钟已实测首帧与 1 Hz 更新），输入事件经 runtime
  `handle_event(WidgetEvent::Ui)` 回投（集成测试覆盖）。UI→runtime 桥使用容量 64 的 `try_send`
  队列，满载立即丢弃、worker 聚合脱敏审计，并以每轮 8 个事件的批次避免 State 投影饥饿；并发洪泛、
  fuel/墙钟超时、内存超限与同 Engine peer 存活均有回归测试。剩余：三平台交互洪泛与生产负载精度实测。

#### S5c — Rust SDK 与作者闭环

- `Widget<State, Event>`、View builder/macro、Context wrapper、test harness。
- `floatile new/dev/check/test/preview/build/inspect` 的 Rust 最小闭环与稳定 JSON 诊断。
- 验收：作者不编辑 WIT/manifest/UI IR；Reference Clock 行为与插件 clock 对比。
- 已实现：`#[derive(State)]`（schema 单源）、`Widget`/`View`/`Context`/`impl_export_widget!`，
  clock-wasm 改用作者 SDK 并通过 runtime 集成测试；`build_ftui`（宿主侧生成 widget.ftui）；
  `floatile new/validate/build` 命令（模板、`.floatile` 校验、manifest 生成 + zip 打包 + 自校验）。
  作者级 `Event` 类型化（`FromWidgetEvent`）已落地；`floatile-runtime::harness`（作者级
  `WidgetHarness`：grant/start/emit/wait_for_state/advance_time/audit，含 clock 集成测试）与
  `floatile test` 无头冒烟命令（build → 提取 → 生命周期/State/宿主存活 + 稳定 JSON）已落地；
  `floatile inspect` 已落地完整包复验与版本化 manifest/版本轴/权限/预算/entry digest JSON 契约；
  `floatile check` 已在自动清理临时目录中复用正式构建/校验链并输出五阶段稳定 JSON；剩余代码能力
  使用静态分析、`dev` 预览接入物理窗口、`preview` 截图与 CLI 其余命令 JSON 诊断统一。

#### S5d — TypeScript SDK

- 先用 ADR 选择 TypeScript adapter/runtime；禁止公开非标准 TypeScript 子集或 Broker 外 ambient API。
- 与 Rust 共用 UI/component/capability/error/behavior vectors；实现 TSX 构建期 View 和同一 WIT world。
- 验收：Rust/TypeScript clocks 行为一致；单/10 实例 CPU/RSS/冷启动/包大小和三平台构建记录。
- 当前结论：ADR-0003 已完成 Linux 候选 spike，但 StarlingMonkey 资源门失败、componentize-qjs 0.4.3
  契约门失败，因此 **no-go**；公共 SDK/TSX/CLI 模板尚未开始，F11 不得标记完成。

### S6 — `.floatile` 包 + 安装

- 做：有界流式 validate/build，manifest/UI/WASM/assets、路径穿越/碰撞/symlink/zip-bomb、digest 与
  原子安装；PluginManager 加载 dev 包。
- 已实现：`floatile-cli` 包校验核心（zip 预算、路径安全、manifest/UI IR/WASM world 校验、正反例
  corpus）；`floatile build` 打包+自校验；原子安装引擎（`install` 子命令：staging/逐文件 fsync/
  digest/原子 rename / install.json / 同版本拒绝 / 失败零残留，含非法包安装期拒绝）；
  `floatile-core::install`（InstallMeta + content_digest 单源）；`floatile-shell::plugin_manager`
  按 digest 复核加载已安装 dev 包并接入参考时钟。config.schema 结构/边界校验与独立 manifest
  JSON Schema 单源产物已落地。多插件并存加载策略已落地：`plugin_manager::list_installed` 枚举
  存储中全部已安装插件、每 id 取最高版本并逐一 digest 复核，篡改整体拒绝、按 id 稳定排序。
  剩余：schema 产物在 `floatile schema` CLI 的发布面接入与签名。
- 验收：合法 Rust/TS clock 包可安装运行；恶意 corpus 全拒绝且不留下半安装状态。

### S7 — 恶意插件安全测试 + 审计
- 做：tests/fixtures/evil-plugin + 非法 UI/State/event/package corpus + 自动化断言；audit_log 落库。
- 已实现：`plugins/evil-wasm` 对抗性 fixture（初始 State `mode` 选择攻击：未声明能力调用 /
  超限/类型错误/未知字段 State Patch / 无限 CPU 循环 / 超限内存申请 / 伪造事件洪泛）；
  `floatile-runtime` 安全集成测试断言「拒绝 + 审计落 SQLite + 宿主存活」（deny、bad-patch、
  fuel trap、StoreLimits trap、forged flood）；`floatile-store` 增加 audit_log 表（migration v3）
  与 AuditStore；shell 运行时把 Broker 脱敏审计落到 layout.db（`with_audit_listener`）。剩余：
  非法 package corpus 的安装期拒绝（并入 S6）与真实容量数据。
- 验收：安全验收 §3 全部通过、宿主存活、审计留痕。

### S8 — P0 复盘
- 做：跑全部验收 F1–F13；回填 platform-matrix；复盘 risks.md 全部假设；锁定版本；产出 MVP 范围建议。

## 3. 每步的验证命令

```bash
cargo check --workspace
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo run -p floatile-shell -- --perf   # 诊断模式：采样 CPU/RSS/帧率
wasm-tools validate logic/plugin.wasm    # 校验组件
# S5 后增加：CLI contract/package/UI/Agent JSON diagnostics 门禁
```

## 4. 平台验证分工

| 平台 | 环境 | 验证人 |
|------|------|--------|
| Linux X11 | 当前机 + 无合成器环境（Xvfb 场景） | 本地 |
| Linux Wayland | sway（wlroots）与 GNOME/Wayland | 本地（有则测） |
| Windows | 本机或 CI | 待定 |
| macOS | macOS 15.7.5 / Apple M4 子集已测 | 继续补穿透、拖拽、缩放、多屏/DPI/热插拔 |

## 5. 完成定义（DoD）

- P0 验收 F1–F13 全绿（或明确降级说明）。
- platform-matrix 实测回填完成。
- risks.md 假设表有结论。
- 无业务 crate 泄漏平台 API；WIT/UI schema/capability/manifest 单源与双 SDK contract tests 就位。
- 三个目标平台均有可运行产物（Linux 必需，Win/macOS 至少 CI 构建通过）。

## 6. 下一步

两条独立风险线都进入 P0 关键路径：

1. 平台线继续在物理 X11 验证 EDID key、负坐标、DPI/拔插，并补 Windows/macOS monitor 与统一 trait。
2. 插件线：WIT/guest-host bindings/clock 已迁移到 ADR-0001 目标契约，`floatile-ui-schema`、manifest/
   capability 纯模型与 S5b runtime+Broker（含 clock 集成测试）已落地；继续 CLI 包校验 + 版本轴/正反例
   contract vectors，再做 S5c Rust SDK 作者闭环与 renderer spike（恶意路径不能只做到 Component 能加载）。

两条线都需要真实证据；任一线的 CI 编译不能替代对应平台/安全验收。
