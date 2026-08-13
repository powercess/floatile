# 风险清单、依赖选型与需验证的假设

> 状态：Accepted
> 每条风险带：等级（高/中/低）、发生概率、影响、缓解措施、P0 验证项。
> 这份文档在 P0 结束必须复盘：哪些假设被推翻、锁定了哪些版本。

## 1. 风险清单

### R1. Wayland 能力受限（点击穿透/置顶/桌面附着）— 高 · 确定发生
- 核心协议不提供点击穿透；layer-shell 支持依赖合成器（wlroots 系支持，GNOME Mutter 不支持）。
- 缓解：能力探测 + 分级降级（layer-shell → XWayland → 普通窗口）；产品对「纯 Wayland 穿透」明确不承诺。
- P0 验证：至少 sway（wlroots）与 GNOME/Wayland 两个环境实测；产出矩阵回填。

### R2. Slint 动态编译 .slint 的成熟度与性能 — 中
- 动态加载需要 `slint_interpreter::Compiler`，运行时编译有 CPU/内存成本；动态组件属性绑定的方式受限。
- 缓解：P0 只做运行时编译验证 + 首帧时间测量；热重载（MVP）用增量重编译；必要时 MVP 阶段用「预编译组件 + 受限动态属性」。
- P0 验证：运行时编译 100 行 .slint 耗时、内存增量、首帧。

### R3. WASM Component Model 工具链稳定性（稳定版 Rust 下）— 中
- guest 目标 `wasm32-wasip2`；wit-bindgen 组件支持成熟度；`cargo-component` 非 stable。
- 缓解：P0 用 `wasm32-wasip2` target + wit-bindgen（纯 cargo 构建），不使用 cargo-component；CI 固定工具链版本；记录到文档。
- P0 验证：最小 `clock.wasm` 在三端加载运行。

### R4. keyring 在 Linux 的可用性 — 中
- Secret Service（gnome-keyring/kwallet）并非所有环境都有；CI/无桌面环境无 Keyring。
- 缓解：V1 提供降级（宿主管理加密文件凭证库，显式 opt-in）；能力不可用则拒绝对应网络权限。
- P0 验证：仅验证 API 不崩，明确降级路径；实际使用在 MVP/V1。

### R5. 多屏/DPI 差异 — 中
- Windows 混合 DPI、X11 每屏 DPI 缺失、Wayland 由合成器缩放；逻辑/物理像素换算易出错。
- 缓解：所有坐标走 `LogicalRect`；保存时记录 scale factor；热插拔重算。
- P0 验证：F7/F8 在三端实测。

### R6. 透明窗口在无合成器 X11 显示异常 — 低 · 确定发生（无合成器时）
- 缓解：能力探测到无合成器 → 回退不透明背景 + 记录标记。
- P0 验证：在 Xvfb/无合成器环境跑一次。

### R7. Windows 桌面附着（WorkerW）脆弱 — 低 · 已知不承诺
- 缓解：P0 标记为实验性，不作为验收项；产品文档不承诺。

### R8. macOS 桌面附着需要辅助功能授权 — 低
- 缓解：P0 不做桌面附着；记录到能力矩阵。

### R9. DNS rebinding 绕过私网校验 — 中（V1 网络能力时）
- 缓解：固定 IP 连接 + SNI 保域名 + 逐 hop 校验（见 http-broker.md §5）。
- P0 验证：无网络能力，仅保留设计；V1 前做 rebinding 测试。

### R10. Slint/winit 透明度在部分驱动（Wayland GL、老 X 驱动）异常 — 中
- 缓解：软件渲染器回退；GPU 信息诊断日志。
- P0 验证：记录各环境 GPU/渲染后端。

### R11. 异步 wasmtime + 主线程事件循环的死锁/卡顿 — 中
- 缓解：wasm 调用全部异步（`async_support`），不在 Slint 回调里阻塞；fuel 上限；调用超时。
- P0 验证：恶意插件无限循环测试不卡死 UI。

### R12. Slint 字体/SVG 传递依赖停止维护 — 高（进入第三方插件阶段前）
- Slint 1.17.1 经 `resvg/usvg` 带入 `rustybuzz 0.20.1`（RUSTSEC-2026-0206）与
  `ttf-parser 0.25.1`（RUSTSEC-2026-0192）；公告标为 unmaintained，当前无安全升级路径。
- 缓解：仅在内部 S1 原生时钟阶段做精确 advisory 例外，不接受第三方 `.slint`、字体或 SVG；持续
  跟踪 Slint/渲染依赖升级，禁止把例外扩展到其他版本或 crate。
- 退出条件：进入 S5 加载不受信任插件 UI/资源前必须移除例外并让 advisory 检查通过；否则 S5 阻断。
- P0 验证：每次 Slint 升级运行 `cargo deny --locked check advisories` 并检查反向依赖图。

## 2. 依赖选型

| 领域 | 选型 | 理由 | 风险备注 |
|------|------|------|----------|
| GUI | Slint（winit 后端，可选软件渲染） | 原生 + GPU；动态编译支持；Rust 生态 | 许可见 licensing.md；动态 API 成熟度见 R2 |
| 插件逻辑 | Wasmtime（component-model + async） | 成熟组件运行时、fuel/memory 限制 | 工具链见 R3 |
| 绑定生成 | wit-bindgen | 官方标准 | — |
| 异步 | Tokio | 标准 | 线程模型见 p0-design §3 |
| HTTP | reqwest + rustls | 生态成熟 | 重定向需自定义策略（http-broker §6） |
| DNS（Broker 用） | hickory-resolver | 可控解析 + 私网校验 | V1 才引入 |
| 存储 | SQLite + rusqlite（bundled） | 单文件、事务 | 迁移机制 MVP 起 |
| 凭证 | keyring | 三端系统凭证库 | Linux 降级见 R4 |
| 序列化 | serde / schemars / jsonschema | 类型→schema 同源 | — |
| 日志 | tracing (+ tracing-subscriber) | 结构化 + span | 审计独立 target |
| 签名/哈希 | ed25519-dalek + sha2（V1） | 生态成熟 | 签名校验 V1 |
| 打包 | zip（带路径穿越校验） | 规范 | 防 zip-bomb（manifest §5） |
| 平台 API | windows-sys / objc2 / x11rb / wayland-client | 官方绑定 | 收敛在 floatile-platform |
| 版本 | semver | 语义版本 | engineApiVersion 判断 |
| 审计依赖 | cargo-deny | 许可/漏洞/来源策略 | CI 强制 |

## 3. 需验证的假设（P0 每项给出结论）

| # | 假设 | 验证方式 | 结论（P0 后填） |
|---|------|----------|-----------------|
| A1 | Slint 动态 .slint 在 3 平台可用且首帧达标 | F11 + 性能指标 | |
| A2 | `wasm32-wasip2` target 在 stable 可用且 wit-bindgen 组件链路顺畅 | clock.wasm 构建+运行 | |
| A3 | wasmtime async + fuel 能可靠终止恶意循环 | 安全验收 §3.3 | |
| A4 | winit 三端提供统一透明窗口路径 | F1 实测 | |
| A5 | XWayland 可覆盖 Wayland 点击穿透需求（能接受缩放差异） | F3 Wayland 降级实测 | |
| A6 | 空宿主性能达标（CPU/RSS/首帧） | 性能验收 | |
| A7 | keyring 三端 API 一致、Linux 可探测到 Secret Service | R4 验证项 | |

## 4. 版本锁定建议

- Rust：`rust-toolchain.toml` 固定为 P0 基线 1.97.1，升级走显式依赖/工具链变更。
- 关键依赖锁：`slint`、`wasmtime`、`wit-bindgen` 三者版本联动；升级时三者一起升并跑 WIT 一致性校验。
- 其余依赖走 `Cargo.lock` + `cargo deny` 准入。
