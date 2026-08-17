# P0 最小垂直切片实施计划

> 状态：Accepted
> 目标：以最小垂直切片跑通 P0 验收 F1–F13，暴露窗口层与 Wayland 风险。
> 原则：每次提交都可运行；每步结束跑一次「当前验收项」；三端差异只进 `floatile-platform`。
> 进度：S0 已完成；S1 有 Windows 与 Linux Xvfb 子集证据；S2/S3 部分实现；Wayland 协议层已在 headless weston 实测（F3/置顶显式降级）；物理 X11、sway/GNOME Wayland 与 macOS 尚未验证。

## 0. 当前基线（2026-08-16）

- Rust 1.97.1、`wasm32-wasip2`、rustfmt、Clippy、wasm-tools 已可用。
- Workspace、九个 crate、CI/依赖策略和工程文档已建立；Windows S2 窗口交互与 S3 SQLite layout CRUD 已部分落地。
- S1 已有 Windows 实测与 Linux Xvfb/Openbox/picom 证据；X11 合成器探测、无边框、置顶、拖拽和 `--perf` 诊断已落地。
- 物理 Linux X11、sway/GNOME Wayland、macOS 仍未验证；Wayland 协议层（headless weston 14.0.2）已验证探测与 F3/置顶显式降级；Win/macOS 后续使用实体机、CI 或远程环境补证。

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

### S5 — 最小 wasm 插件（clock.wasm）
- 做：wit/ 写入 widget WIT v1；floatile-plugin-api（bindgen host 绑定）；floatile-runtime（wasmtime engine + fuel/memory 配额）；floatile-sdk（guest 绑定 + 属性宏）；PermissionBroker 骨架（log/storage/timer/metrics）。
- 验收：F11；恶意循环/超内存被 fuel/memory trap（安全 §3.3/3.4）。

### S6 — .floatile 包 + 安装
- 做：floatile-cli（validate + build，zip + 路径穿越校验）；PluginManager 加载 dev 包。
- 验收：能安装 dev 包并运行插件时钟；manifest 校验失败拒绝安装。

### S7 — 恶意插件安全测试 + 审计
- 做：tests/fixtures/evil-plugin + 自动化断言；audit_log 落库。
- 验收：安全验收 §3 全部通过、宿主存活、审计留痕。

### S8 — P0 复盘
- 做：跑全部验收 F1–F13；回填 platform-matrix；复盘 risks.md 假设 A1–A7；锁定版本；产出 MVP 范围建议。

## 3. 每步的验证命令

```bash
cargo check --workspace
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo run -p floatile-shell -- --perf   # 诊断模式：采样 CPU/RSS/帧率
wasm-tools validate logic/plugin.wasm    # 校验组件
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
- 无业务 crate 泄漏平台 API；WIT 单源校验脚本就位。
- 三个目标平台均有可运行产物（Linux 必需，Win/macOS 至少 CI 构建通过）。

## 6. 下一步

在物理 X11 多屏环境验证 RandR 的 EDID key、负坐标、主屏、DPI 与拔插回归（F7/F8 实机验收）；
随后补齐 Windows/macOS monitor 实现和统一平台 trait。窗口风险完成实测前不扩展到 S5 插件层。
