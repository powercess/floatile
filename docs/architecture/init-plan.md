# P0 最小垂直切片实施计划

> 状态：Accepted
> 目标：以最小垂直切片跑通 P0 验收 F1–F13，暴露窗口层与 Wayland 风险。
> 原则：每次提交都可运行；每步结束跑一次「当前验收项」；三端差异只进 `floatile-platform`。
> 进度：S0 已完成；S1 有 Windows 与 Linux Xvfb 子集证据；S2/S3 部分实现；Wayland 协议层已在 headless weston 实测；ADR-0001 与插件/SDK 架构已确定，S5 实现尚未进入 dev。

## 0. 当前基线（2026-08-18）

- Rust 1.97.1、`wasm32-wasip2`、rustfmt、Clippy、wasm-tools 已可用。
- Workspace、九个 crate、CI/依赖策略和工程文档已建立；Windows S2 窗口交互与 S3 SQLite layout CRUD 已部分落地。
- S1 已有 Windows 实测与 Linux Xvfb/Openbox/picom 证据；X11 合成器探测、无边框、置顶、拖拽和 `--perf` 诊断已落地。
- 物理 Linux X11、sway/GNOME Wayland、macOS 仍未验证；Wayland 协议层（headless weston 14.0.2）已验证探测与 F3/置顶显式降级；Win/macOS 后续使用实体机、CI 或远程环境补证。
- ADR-0001 已把插件 UI 从第三方 `.slint` 改为统一 `widget.ftui` + State Patch；插件系统、SDK、WIT、
  manifest、安全与 crate 文档已形成实施约束，但对应代码不算完成。
- `agent/s4-plugin-wit` 的实验分支证明 stable Rust 可构建/validate Component，但其契约早于 ADR-0001，
  缺少正式 UI State 输出和统一生命周期；评审/合入前必须按新 WIT/UI schema 重整，不能按原样作为 F11。

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
  Xvfb+Openbox 下拖拽→重启恢复与删除清库已实测；`kv/audit_log` 仍未实现，真实多屏/DPI/热插拔
  实机验证待做。

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

#### S5b — Runtime actor + Broker

- Wasmtime Engine/Store limits、每实例 bounded serial actor、timeout/cancel/shutdown。
- State Patch 原子验证与有界 UI 投递；shell renderer 从已验证 IR 构建 Slint host UI。
- Broker 固有能力（UI/log/clock）与 timer 最小声明能力；allow/deny/quota/audit。
- 验收：Rust clock 1 Hz 更新；deny、超 patch、队列洪泛、fuel/内存 trap 后宿主存活。

#### S5c — Rust SDK 与作者闭环

- `Widget<State, Event>`、View builder/macro、Context wrapper、test harness。
- `floatile new/dev/check/test/preview/build/inspect` 的 Rust 最小闭环与稳定 JSON 诊断。
- 验收：作者不编辑 WIT/manifest/UI IR；Reference Clock 行为与插件 clock 对比。

#### S5d — TypeScript SDK

- 先用 ADR 选择 TypeScript adapter/runtime；禁止公开非标准 TypeScript 子集或 Broker 外 ambient API。
- 与 Rust 共用 UI/component/capability/error/behavior vectors；实现 TSX 构建期 View 和同一 WIT world。
- 验收：Rust/TypeScript clocks 行为一致；单/10 实例 CPU/RSS/冷启动/包大小和三平台构建记录。

### S6 — `.floatile` 包 + 安装

- 做：有界流式 validate/build，manifest/UI/WASM/assets、路径穿越/碰撞/symlink/zip-bomb、digest 与
  原子安装；PluginManager 加载 dev 包。
- 验收：合法 Rust/TS clock 包可安装运行；恶意 corpus 全拒绝且不留下半安装状态。

### S7 — 恶意插件安全测试 + 审计
- 做：tests/fixtures/evil-plugin + 非法 UI/State/event/package corpus + 自动化断言；audit_log 落库。
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
| macOS | 待定 | 待定 |

## 5. 完成定义（DoD）

- P0 验收 F1–F13 全绿（或明确降级说明）。
- platform-matrix 实测回填完成。
- risks.md 假设表有结论。
- 无业务 crate 泄漏平台 API；WIT/UI schema/capability/manifest 单源与双 SDK contract tests 就位。
- 三个目标平台均有可运行产物（Linux 必需，Win/macOS 至少 CI 构建通过）。

## 6. 下一步

两条独立风险线都进入 P0 关键路径：

1. 平台线继续在物理 X11 验证 EDID key、负坐标、DPI/拔插，并补 Windows/macOS monitor 与统一 trait。
2. 插件线先做 S5a 的 UI schema + 新 WIT + manifest contract tests；不得直接在旧实验 WIT 上实现
   runtime。S5b 必须以 State Patch + Broker + 恶意路径为完整垂直切片，不能只做到 Component 能加载。

两条线都需要真实证据；任一线的 CI 编译不能替代对应平台/安全验收。
