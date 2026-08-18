# ADR-0001：插件使用统一 Floatile UI，Slint 仅作为宿主实现

> 状态：Accepted
> 日期：2026-08-18
> 决策者：Floatile 项目

## 背景与需求

Floatile P0 原设计要求插件同时提供 `.slint + .wasm`，宿主在进程内动态编译第三方 `.slint`。
这个方案可以直接获得 Slint 的表达能力，但会把 Slint 语法、版本、许可和不可信源码编译面暴露给
每个插件作者，也会迫使 Rust SDK、TypeScript SDK、文档和 AI Agent 同时理解 Floatile 与 Slint
两套模型。

插件 SDK 的首要产品要求是：普通开发者只理解 `State / View / Event / Context` 四个概念；Rust 与
TypeScript 使用同一组件、生命周期、权限和错误语义；AI Agent 可以通过结构化 schema、诊断和
截图闭环稳定生成插件。安全上，插件不得获得 Slint 对象、宿主窗口句柄或任意 UI 节点访问能力。

关联：FR-PLUGIN-01、FR-PERM-01、FR-PACK-01、NFR-SEC-02、NFR-MAINT-01、F11、F12、R2、R12。

## 候选方案

### A. 插件直接提供 `.slint`

- 优点：表达能力最大，宿主适配工作较少。
- 缺点：开发者必须学习 Slint；运行时编译不可信源码；直接承受 Slint 语法兼容、许可与 R12
  传递依赖风险；Rust/TypeScript SDK 无法提供完整的一致抽象。

### B. 标准 UI 与自定义 Slint 双模式

- 优点：普通插件简单，复杂插件保留逃生口。
- 缺点：长期维护两套 UI、调试、兼容和安全模型；`eject` 后不可逆；安装器与用户需要理解两类
  风险；P0 在没有真实需求证据时提前冻结较大公开表面。

### C. 统一 Floatile UI IR

- 优点：一种组件与数据流；构建期验证；Rust/TypeScript 同源生成；宿主只接收有限、版本化、
  无脚本的 UI IR；Slint 可被替换而不改变插件 API；最适合机器生成和长期兼容。
- 缺点：Floatile 必须维护组件集、UI 编译器和 IR；极端自定义效果需要 Canvas/Path 或未来新
  扩展；P0 需要先验证 IR 到 Slint 的性能。

### D. HTML/CSS/WebView

- 优点：Web 开发者熟悉、生态丰富。
- 缺点：内存和启动成本高，安全面大，与轻量桌面 Widget 和 Rust/TypeScript 对称 SDK 目标冲突。

## 决策

选择 C：P0/MVP 只公开统一 Floatile UI。

1. 插件 UI 构建产物是版本化 `widget.ftui`，包含静态组件树、State schema、事件 schema、资源引用
   和绑定；不得包含脚本、原生句柄或任意文件导入。
2. UI 结构在构建时确定。运行时采用单向数据流：`Event → plugin → State Patch → host validation →
   UI render`。插件不得按名称查找或直接修改宿主 UI 节点。
3. Slint 是 `floatile-shell`/`floatile-runtime` 内部渲染实现，不是 P0/MVP 插件契约。插件包不包含
   第三方 `.slint`，宿主不编译插件提供的 `.slint`。
4. Rust 与 TypeScript SDK 从同一 UI schema、WIT 与 capability registry 生成公开类型；两种语言
   必须具有相同组件、生命周期、权限、错误和行为语义。
5. 标准组件组合、受预算约束的 Canvas/Path 与宿主认可的组件包提供定制能力。P0/MVP 不提供
   `ui eject` 或 Custom Slint profile。
6. 未来只有真实插件证明标准 UI 无法满足需求，并且新的 ADR 定义隔离、兼容、许可与迁移边界后，
   才可以增加自定义 UI profile；不得静默把 `.slint` 重新加入包格式。

## 后果

- FR-PLUGIN-01、F11、manifest v1、WIT v1、runtime 和 SDK 文档必须从 `.slint + .wasm` 改为
  `widget.ftui + plugin.wasm`。
- `host-ui` 成为正式 WIT interface。所有 State Patch 都经过实例身份、schema、大小、深度、频率
  和 UI 线程队列检查；它是固有能力但仍由 `PermissionBroker` 仲裁与审计。
- UI IR 需要独立版本 `uiApiVersion`，不能与 `engineApiVersion` 或 `manifestVersion` 混用。
- `floatile-runtime` 不再动态编译第三方 `.slint`；它负责验证 UI IR、维护实例 State、把有界 patch
  投递到 Slint 主线程并运行不可信 WASM。
- R2 从“第三方 Slint 动态编译成熟度”改为“UI IR 渲染与 State Patch 性能”；R12 仍阻断第三方
  字体/SVG，但不再阻断纯 `widget.ftui + wasm` 垂直切片。
- P0 可以先以 JSON 编码实现 `widget.ftui`，但文件名、schema 版本和语义从 v1 起受兼容策略约束；
  改为二进制编码必须保持语义兼容或 bump major。

## 证据

- 当前 `floatile-shell` 已证明宿主拥有 Slint 组件与主线程投递边界。
- 当前 WIT 草案只有事件输入，没有可完成 F11 的正式 UI 输出路径；State Patch 补齐该闭环。
- `docs/architecture/risks.md` R12 已记录第三方 Slint/字体/SVG 进入宿主前必须退出 advisory 例外。
- P0 仍需用参考时钟验证 UI IR 构建、State Patch、1 Hz 更新、错误路径、CPU/RSS 与首帧指标。
