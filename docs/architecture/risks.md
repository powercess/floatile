# 风险清单、依赖选型与需验证的假设

> 状态：Accepted
> 每条风险带：等级（高/中/低）、发生概率、影响、缓解措施、P0 验证项。
> 这份文档在 P0 结束必须复盘：哪些假设被推翻、锁定了哪些版本。

## 1. 风险清单

### R1. Wayland 能力受限（点击穿透/置顶/桌面附着）— 高 · 确定发生
- 核心协议不提供点击穿透；layer-shell 支持依赖合成器（wlroots 系支持，GNOME Mutter 不支持）。
- 缓解：能力探测 + 分级降级（layer-shell → XWayland → 普通窗口）；产品对「纯 Wayland 穿透」明确不承诺。
- P0 验证：至少 sway（wlroots）与 GNOME/Wayland 两个环境实测；产出矩阵回填。

### R2. Floatile UI IR/State Patch 的表达力与性能 — 中
- ADR-0001 取消第三方 `.slint`，Floatile 必须维护组件/IR/renderer；组件不足会迫使作者等待宿主升级，
  patch/绑定实现不当会造成 UI 卡顿或状态不一致。
- 缓解：P0 只冻结最小组件；组合 + 受限 Canvas/Path；schema 单源；patch 原子验证；不做运行时 VDOM；
  renderer spike 比较预编译通用组件与宿主从已验证 IR 生成定义；用 Reference/Rust/TypeScript clocks
  和恶意 IR/patch 建立行为/性能基线。
- P0 验证：嵌套布局/If/ForEach/动画可行性、IR 构建/缓存/销毁、首帧、1 KiB patch P95、30
  updates/s、10 实例与非法 IR/patch 回滚。

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
- 缓解：wasm 调用全部异步，不在 Slint 回调里阻塞；UI 回调只向容量 64 的桥做 `try_send`，过载
  丢弃并在 worker 聚合审计；fuel 按 guest 调用补充；共享 epoch ticker 强制每调用墙钟 deadline。
- ADR-0004 已拒绝用同步 WIT import 等待长任务。宿主 Operation 使用有界提交/完成/并发/结果预算，
  cancellation/deadline 产生唯一终态；completion 只带元数据并按 instance generation 非阻塞投递，
  旧代、满载和 actor 关闭均丢弃 retained result。
- 已验证：超大 fuel 的恶意无限循环被 25 ms 测试预算中断，同 Engine 已启动 peer 随后仍能处理事件；
  8 线程并发洪泛只保留队列容量内事件；Operation reference fixture 覆盖拒绝、超时、取消、迟到
  completion、实例重启、提交/结果/actor 队列过载和后续工作存活。FTUI 解析/校验/renderer 已移到
  准备线程。
- 剩余：Slint 1.17 `ComponentDefinition` 含 `Rc`、不可跨线程，宿主生成的有界源码仍须在 UI executor
  编译；Operation v1.1 WIT/SDK/guest dispatch 已接入首个 `storage:read` typed adapter，动态撤权和宿主重启恢复仍未实现；需回填真实插件
  编译时延与三平台 UI heartbeat，超预算时再决策通用预编译 renderer。

### R12. Slint 字体/SVG 传递依赖停止维护 — 高（公开分发/第三方资源前）
- Slint 1.17.1 经 `resvg/usvg` 带入 `rustybuzz 0.20.1`（RUSTSEC-2026-0206）与
  `ttf-parser 0.25.1`（RUSTSEC-2026-0192）；公告标为 unmaintained，当前无安全升级路径。
- 缓解：仅在内部 S1 原生时钟阶段做精确 advisory 例外，不接受第三方 `.slint`、字体或 SVG；持续
  跟踪 Slint/渲染依赖升级，禁止把例外扩展到其他版本或 crate。
- ADR-0001 后纯 `widget.ftui + wasm` 不编译第三方 Slint，也不允许第三方字体/SVG，因此可以在精确
  例外下验证内部统一 UI 垂直切片；公开分发、允许第三方字体/SVG 或扩大受影响解析路径前必须移除
  例外并让 advisory 检查通过，否则对应能力阻断。
- P0 验证：每次 Slint 升级运行 `cargo deny --locked check advisories` 并检查反向依赖图。

### R13. macOS 全局热键依赖已弃用的 Carbon API — 低
- 展示模式点击穿透后的恢复热键使用 Carbon `RegisterEventHotKey` + 事件处理器；该 API 无需
  辅助功能/输入监控授权（对比 `CGEventTap` 会触发授权弹窗），但 Carbon 在 macOS 15 已标记
  弃用、仍可正常工作。
- 缓解：热键注册失败时关闭点击穿透（沿用 Windows/X11 的「恢复热键失败即禁用穿透」降级）；
  若未来 macOS 移除 Carbon，迁移到 `CGEventTap` + 授权提示或 `NSEvent` 全局监听并重新实测。
- P0 验证：macOS 15.7.5（Apple M4）已实测 `RegisterEventHotKey` 注册成功（日志
  `global hotkey registered (Ctrl+Shift+E)`）；真实按键触发待人工复核。

### R14. TypeScript adapter/runtime 的体积与隔离 — 高
- 完整 TypeScript/JavaScript 语义可能显著增加单实例冷启动、包大小和 RSS；共享 runtime 又可能破坏
  实例故障隔离，Node/DOM 兼容层可能偷渡 ambient capability。
- 缓解：实现前独立 ADR；CLI 管理锁定工具链；同一 WIT/Broker；不提供 Node/DOM；对候选做单/10
  实例资源、trap、timeout、三平台和 contract vector 比较。
- P0 验证：TypeScript clock 与 Rust clock 行为一致且达到明确资源门；未达标则记录 P0 失败/降级，
  不发布伪 TypeScript 子集。

### R15. Rust/TypeScript SDK 语义漂移 — 高
- 两套手写 SDK 容易在组件、错误、权限默认值、事件顺序和版本支持上分叉，文档/Agent 示例会进一步
  放大差异。
- 缓解：UI schema/WIT/capability registry 单源生成；共享 contract/behavior vectors；每个文档示例
  CI 编译；同一稳定诊断 code。
- P0 验证：双 clock 与 capability deny/timeout/state patch vectors 全部一致。

### R16. UI IR 过早冻结 — 中
- v1 若混合渲染器细节、脚本或 Slint 名称，会把内部实现变成长期插件兼容负担；过度抽象又可能无法
  表达真实 Widget。
- 缓解：v1 只含稳定组件/State/Event/binding/有限 If/ForEach；Slint 版本不进入 manifest；编码与语义分离；新增
  组件按 minor，破坏语义按 major；Custom UI 必须新 ADR。
- P0 验证：至少 clock、system monitor、countdown 三种内部 fixture 证明模型，不用预留未验证控件。

## 2. 依赖选型

| 领域 | 选型 | 理由 | 风险备注 |
|------|------|------|----------|
| 宿主 GUI | Slint（winit 后端，可选软件渲染） | 原生 + GPU；只在宿主内 | 许可见 licensing.md；第三方资源见 R12 |
| 插件 UI | Floatile UI IR | 双 SDK 同一模型、可验证、renderer 可替换 | 表达力/性能见 R2；冻结风险见 R16 |
| 插件逻辑 | Wasmtime（component-model + async） | 成熟组件运行时、fuel/memory 限制 | 工具链见 R3 |
| 绑定生成 | wit-bindgen | 官方标准 | — |
| TypeScript adapter | 待 ADR | 保留普通 TypeScript 语义且进入同一 Component/Broker | R14/R15 |
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
| A1 | Floatile UI IR 在三平台可由 Slint host 渲染，State Patch 首帧/延迟达标 | F11 + patch/性能指标 | |
| A2 | `wasm32-wasip2` target 在 stable 可用且 wit-bindgen 组件链路顺畅 | clock.wasm 构建+运行 | |
| A3 | wasmtime async + fuel 能可靠终止恶意循环 | 安全验收 §3.3 | |
| A4 | winit 三端提供统一透明窗口路径 | F1 实测 | |
| A5 | XWayland 可覆盖 Wayland 点击穿透需求（能接受缩放差异） | F3 Wayland 降级实测 | |
| A6 | 空宿主性能达标（CPU/RSS/首帧） | 性能验收 | |
| A7 | keyring 三端 API 一致、Linux 可探测到 Secret Service | R4 验证项 | |
| A8 | 完整 TypeScript adapter 在资源目标内且无 ambient capability | TypeScript clock + 单/10 实例 + 安全测试 | |
| A9 | Rust/TypeScript SDK 可从单源保持行为与诊断一致 | 双 SDK contract/behavior vectors | |
| A10 | 最小组件 + Canvas 足以表达 P0 常见 Widget，无需第三方 Slint | clock/system-monitor/countdown fixtures | |

## 4. 版本锁定建议

- Rust：`rust-toolchain.toml` 固定为 P0 基线 1.97.1，升级走显式依赖/工具链变更。
- 关键依赖锁：`slint`、`wasmtime`、`wit-bindgen` 与未来 TypeScript adapter 各自记录兼容组；升级时
  跑 UI/WIT/双 SDK contract、三平台构建和资源回归，不能只用能编译作为通过。
- 其余依赖走 `Cargo.lock` + `cargo deny` 准入。
