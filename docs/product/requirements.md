# Floatile 项目需求基线

> 状态：Accepted
> 范围：P0
> 基线：0.1（从现有 P0 设计、验收、安全与插件文档归并）

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
| FR-PLUGIN-01 | 宿主必须加载一个 `.slint + .wasm` Widget，host/guest 只通过版本化 WIT 通信。 | F11 |
| FR-PERM-01 | 插件所有宿主能力必须经 deny-by-default Broker 检查、配额和审计。 | F12 |
| FR-PACK-01 | dev 包安装必须校验 manifest、引用文件、规范化路径与大小限制。 | F11、F12 |
| FR-PROBE-01 | 平台能力必须在运行时探测；产品逻辑消费能力结果，不根据 OS 名称猜测。 | F3、F13 |

详细操作标准以 `../architecture/p0-acceptance.md` 为准。

## 3. 非功能需求

| ID | 要求 |
|---|---|
| NFR-SEC-01 | 默认零权限；WASM 内存隔离；fuel/内存/调用频率有上限；拒绝不能拖垮宿主。 |
| NFR-SEC-02 | manifest、zip、Slint、WASM、配置与插件参数一律按不受信任输入处理。 |
| NFR-OBS-01 | 插件加载、能力决策、模式与窗口事件使用结构化 tracing；敏感参数必须脱敏。 |
| NFR-PERF-01 | release 构建达到 P0 验收定义的 CPU、RSS、首帧、帧率与拒绝路径目标。 |
| NFR-PORT-01 | Windows、macOS、Linux X11 可构建；Wayland 按能力矩阵分级，不伪装成完全一致。 |
| NFR-MAINT-01 | crate 依赖单向；平台差异只进 `floatile-platform`；WIT 只有一个源。 |
| NFR-REPRO-01 | 固定 Rust 工具链、提交 `Cargo.lock`，CI 使用 `--locked`。 |
| NFR-LEGAL-01 | Slint 与仓库许可未决前禁止对外分发。 |

## 4. P0 非目标

插件市场、签名与自动更新、多画布、主题系统、凭证托管、网络 Broker、跨插件通信、Sidecar、
完整无障碍均不在 P0。接口可以预留，但不得以未实现 stub 宣称需求完成。

## 5. 当前实现差距（2026-08-16）

| 范围 | 状态 | 主要差距 |
|---|---|---|
| Workspace/核心类型 | 部分实现 | 多数 crate 仍为模板，依赖边界尚缺架构测试 |
| S1 窗口与原生时钟 | 部分验证 | 透明、无边框、置顶、拖拽已有 Windows、Linux Xvfb 与 VMware Xfce/Xorg 证据；Wayland 协议层已实测（headless weston：探测正确、置顶/穿透显式降级），桌面会话与 macOS 未验证；时钟按 UTC 秒数计算且未标注时区 |
| S2 桌面交互 | 部分验证 | Edit/Show、Windows 与 X11 点击穿透、恢复热键、拖拽与缩放已落地；Linux Xvfb 与 VMware Xfce/Xorg 已验证穿透往返，Xfce 已验证窗口重映射后的输入区重同步；物理多显示器、DPI 与热插拔降级（F7/F8）未验证 |
| 平台抽象 | 部分实现 | 能力状态包含明确降级原因；Windows 窗口能力与 X11 compositor/SHAPE/EWMH/RandR 探测已落地；尚无统一四平台 trait、Wayland 协议探测及 macOS 实现 |
| 布局/存储 | 部分验证 | 核心层已有强类型 monitor/DPI/物理坐标模型和平台无关恢复算法；SQLite v2 migration 已持久化物理尺寸、scale factor 与 `lost_monitor`；shell 已接入启动保存/恢复（位置/尺寸/模式）、拖动/缩放/模式/热键/退出保存与显示器变化重恢复，Xvfb+Openbox 下重启恢复与删除已实测；真实多屏/DPI/热插拔与 Windows/macOS 实机验证待做 |
| WASM/WIT/SDK | 部分实现 | `wit/floatile-widget.wit` v1 契约、SDK guest 绑定、plugin-api host async 绑定与 clock.wasm 组件构建已落地（wasm-tools validate 通过，CI 有单源校验）；wasmtime runtime 加载执行、fuel/memory 配额与契约测试缺失 |
| Permission Broker | 未实现 | grants、配额、审计和恶意插件 fixture 缺失 |
| 包工具链 | 未实现 | validate/build、schema、路径与 zip-bomb 防护缺失 |
| 跨平台/性能证据 | 部分验证 | Windows、Linux Xvfb 与 VMware Xfce/Xorg 已回填 S1/S2 子集，两个 Linux X11 环境的 F3 穿透往返通过；Wayland 协议层（headless weston）首帧/CPU/RSS 与 F3 降级已回填；桌面会话与 macOS 未验证 |

## 6. 阶段门槛与未决策

进入 MVP 前必须完成 F1–F13 记录、风险 A1–A7 复盘、许可 ADR、WIT/manifest 版本冻结和三平台
构建证据。以下问题不能由实现者私自假设：Slint 商业/开源路线、Wayland 产品承诺、插件签名信任
模型、网络与凭证能力的上线阶段、P0 性能目标是否成为发布 SLO。
