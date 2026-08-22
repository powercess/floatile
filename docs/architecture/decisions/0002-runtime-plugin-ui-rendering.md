# ADR-0002：运行时第三方插件 UI 渲染路径

> 状态：Implemented
> 日期：2026-08-21
> 决策者：Floatile 项目

## 背景与需求

ADR-0001 已决策：插件 UI 是版本化 `widget.ftui`（统一 Floatile UI IR），Slint 只是宿主内部实现，
宿主不编译第三方 `.slint`。当前参考时钟的实现路径是**构建期**：`floatile-shell/build.rs` 调
`floatile_clock_wasm::__floatile_ftui_json()` 生成 widget.ftui，经 `floatile-renderer` 转成
`ClockPluginUI.slnt` 源码文本，宿主 `slint!` 宏在编译期嵌入该组件，运行时沿静态 binding 槽位投影
权威 State。

该路径只证明「一个预编译的参考时钟能被宿主渲染」，不满足：

- **FR-PLUGIN-01 / F11**：宿主必须加载「由统一 Floatile UI 与 WASM Component 组成的 Widget」——
  第三方插件安装后的 `widget.ftui` 必须能被渲染进窗口。
- **FR-PACK-01**：dev 包安装后必须可运行（S6 已实现安装 + PluginManager 加载已安装插件的
  `ui_bytes`，但没有任何运行时把 UI 实例化进窗口）。

`requirements.md` §5、`init-plan.md` S5a、`workspace-and-crates.md` §3.3 均把「运行时第三方插件 UI
渲染」标注为 S5a 剩余项，并注明「依赖 interpreter/运行时编译 ADR」。本 ADR 即该决策。

关联：FR-PLUGIN-01、FR-PACK-01、NFR-SEC-01/02、NFR-MAINT-01、F11、ADR-0001、R2。

## 候选方案

### A. Slint interpreter（`slint-interpreter`）运行时编译

把 renderer 生成的 `.slnt` 源码文本交给 `slint_interpreter::ComponentCompiler` 在进程内运行时编译，
`ComponentDefinition::create()` 实例化，沿 binding 槽位 set/get property。

- 优点：与现有 renderer 输出契约零改动（同一源码文本）；不引入 `slint-build`（规避
  RUSTSEC-2024-0436 及其图像/AVIF 依赖链，与现有 build.rs 注释一致）；组件能力与 `slint!`
  宏产物等价；无第三方 `.slint` 接触面（仍只编译**宿主 renderer 生成的受限源码**）。
- 缺点：slint-interpreter 是运行时组件（i-slint-compiler 已在宏路径进依赖树，本路径把它
  变为运行时依赖）；性能（编译耗时）需实测；仍是 Slint 许可面的一部分（许可 ADR 独立决策）。
- 版本：1.17.1，与 `slint` 完全同版本，share 同一 compiler/core。

### B. 运行时调用 `slint-build` 或子进程编译

宿主 runtime 以 `slint-build` 或编译子进程把 renderer 输出在运行时编成代码再加载。

- 优点：复用宏路径的编译产物形态。
- 缺点：`slint-build` 需在 cargo 构建上下文运行（`compile_evidence.rs` 注释已记录
  `NotRunViaCargo` 问题）；引入 RUSTSEC-2024-0436 排除依赖链；运行时编译子进程是新的
  不受信任执行面与平台差异面（Windows/macOS 路径、工具链定位、并发编译）；失败路径复杂。
- 结论：与「运行时轻量、无脚本、bounded」目标冲突，排除。

### C. 自定义 UI 解释器（自研 IR → 原生渲染）

放弃 IR→Slint 映射，把 widget.ftui 直接解释为 Slint 组件树之外的渲染（如 femtovg/软件渲染）。

- 优点：彻底脱离 Slint 许可与编译面。
- 缺点：重写 renderer 与组件语义、动画、布局、输入命中测试、无障碍与三平台后端；NFR-PERF-01
  首帧/帧率目标需重新验证；完全推翻 S5a 已验证的 renderer 契约测试。P0 无此需求证据。
- 结论：作为未来替换 renderer 的独立 ADR 才有意义，不是本决策的候选。

### D. 保持构建期嵌入（不决策）

维持只有参考时钟可渲染的现状。

- 缺点：F11/F12 无法完成，插件开发者无法运行自己的浮窗，与 P0 验收直接冲突。
- 结论：拒绝。

## 决策

选择 **A：Slint interpreter 运行时编译 renderer 生成的源码**。

1. `floatile-runtime`/`floatile-shell` 在**运行时**读取已安装插件的 `widget.ftui`，先经
   `floatile-ui-schema::validate_document` 复验，再经 `floatile-renderer::render_component`
   生成源码文本，最后用 `slint_interpreter::Compiler` 编译并以 `create()` 实例化为
   **独立原生窗口**（`ComponentHandle::window()` + `WinitWindowAccessor`），复用
   `floatile-platform` 窗口能力（无边框/透明/置顶/穿透）。
2. **接入形态（spike-2 修订）**：评估了 Slint `ComponentContainer` + `component-factory`
   嵌入式路径，但它是 **Slint 1.17 实验性 API**——生产 `builtin()`（`i-slint-compiler/
   typeregister.rs` 672 行）显式移除 `ComponentContainer`/`component-factory`，仅在
   `SLINT_ENABLE_EXPERIMENTAL_FEATURES` 编译环境变量下可用，官方注释「Do not use in
   production code!」。依赖它会污染 floatile-shell 的编译面并违反本仓库稳定 API 纪律，
  故**拒绝**。
  formal 接入改为 **interpreter 自窗口（稳定 API）**：`slint_interpreter` 编译的
   `ComponentDefinition::create()` 生成的 `ComponentInstance` 自带独立 window adapter
   （`ComponentHandle::window()`），经 `slint::winit_030::WinitWindowAccessor::
   with_winit_window` 取原生 `winit::window::Window` 并复用 `floatile-platform` 的
   无边框/透明/置顶/点击穿透能力，无需迁移 S1–S4 平台层。代价是插件 UI 与宿主框架为
   分离窗口，需在 shell 侧协调其外观/输入（架构变更，另见 shell 切片计划）。
3. 编译的**唯一输入**仍是宿主 renderer 生成的受限源码（属性/回调名由 renderer 生成，
   字符串走结构化转义）；插件永不提供 `.slint`，interpreter 不被当作不受信任源码编译器。
4. binding/event 槽位契约不变：`RenderedComponent.bindings/events` 仍是权威 State 投影与
   输入事件回投的唯一事实源（shell 运行时不再依赖 build.rs 的 `plugin_meta.json` 静态槽位）。
5. 参考时钟保留为 `slint!` 静态嵌入路径作为内建基线（S1–S4 平台窗口证据依赖它）；第三方
   已安装插件走运行时 interpreter 路径。两条路径共享 renderer 生成契约。
6. interpreter 依赖为 `slint-interpreter = "1.17"`（与 `slint` 同版本），不新增 `slint-build`/
   图像/AVIF 链。Cargo.lock 新增条目需过 cargo-deny advisories/bans 门禁。

### 明确不做

- 不做自定义 UI 解释器（方案 C）与运行时编译子进程（方案 B）；未来替换 renderer 需新 ADR。
- 本 ADR 不引入 TypeScript runtime——S5d 的 TS SDK 仍需独立 runtime ADR 与性能/隔离证据。
- 本 ADR 不解决 Slint 许可——许可路线（licensing.md 候选 A–D）独立决策，interpreter 与
  `slint!` 宏同属 Slint 1.17 许可面，不改变许可口径。

## 后果

- **收益**：插件作者安装的 `.floatile` 可被宿主渲染；F11/F12 的 UI 渲染链闭合；renderer 契约、
  runtime actor/State 投递全部复用，改动集中在 shell/runtime 的接线层。
- **代价**：`slint-interpreter` 成为 shell 正式依赖（当前 spike 仅 dev 使用）；新增运行时编译
  入口需按不受信任输入预算（IR/大小/深度在 validate_document 已限）；实例化与首帧性能需实测
  回填到性能验收表。
- **兼容性**：插件包格式（manifest v1 / widget.ftui v1 / uiApiVersion）不变；renderer 输出文本
  形态不变；`slint!` 静态路径保留，无迁移成本。
- **安全**：interpreter 编译的是 renderer 输出的受限源码而非任意输入；IR 预算/结构校验前置不变；
  恶意 UI（超大/过深/越界绑定）仍在 validate_document + renderer 预算层被拒，不达 interpreter。
- **验证**：renderer 契约向量保证任意合法 IR 的生成文本一致；interpreter 自窗口路径对同一
  renderer 输出的编译+投影在 Xvfb 实测通过（见证据）；恶意 UI fixture 在运行时路径仍被拒且宿主
  存活（裁单实例隔离由 runtime 已实现的 actor/预算层保证）。

## 证据

- spike `crates/floatile-shell/tests/runtime_render_spike.rs`（本 ADR 附带）：
  - `runtime_compiles_renderer_output`：renderer 输出的 `ClockPluginUI.slnt` 源码文本可由
    `slint-interpreter` 运行时编译，产物组件名与 renderer 声明一致——无头 CI 可直接跑（纯编译，
    无 backend 需求）。
  - `runtime_instance_projects_state`：Xvfb 下 `definition.create()` 实例化、binding 槽位
    `set/get_property` 往返成功；无头环境明确 SKIP 并留痕。
  - 环境：Linux x64（video: VMware SVGA II），`xvfb-run -a cargo test -p floatile-shell
    --test runtime_render_spike` → 2 passed。
- spike-2 `crates/floatile-shell/tests/runtime_embed_spike.rs`（接入形态判据，dev 依赖验证）：
  - `host_window_can_embed_interpreter_factory`：interpreter 编译的 `ComponentDefinition` 可
    桥接为 `slint::ComponentFactory`（编译通过）——证明 Slint 公共导出与 interpreter 类型互操作，
    但该路径依赖实验性 `ComponentContainer`，正式接入不采用（见决策 2）。
  - `interpreter_instance_has_own_window`：interpreter 内容组件（Rectangle, 非 Window）由
    `create()` 获得自己的 window adapter——证实「自窗口」接入形态成立。
- spike-3 `crates/floatile-shell/tests/own_window_spike.rs`（最终接入判据，Xvfb 全绿）：
  `interpreter_window_exposes_native_window`：interpreter 自窗口经 `WinitWindowAccessor::
  with_winit_window` 取原生 winit 窗口（复用 floatile-platform 运行时能力的前置）+ 沿 renderer
  binding 槽位 `set/get_property` 投影往返成功——「自窗口 + 原生句柄 + State 投影」三断言成立。
  - 环境：Linux x64（video: VMware SVGA II），`xvfb-run -a cargo test -p floatile-shell
    --test own_window_spike` → 1 passed。
- 依赖重叠：`slint-interpreter` 1.17.1 与现有 `slint!` 宏共用 `i-slint-compiler`/`i-slint-core`/
  `i-slint-common`；Cargo.lock 增量仅 interpreter 自身及其薄封装，无新增 crate 类风险。
- renderer 契约：`floatile-renderer/tests/compile_evidence.rs`（5 用例全绿）继续约束生成文本的
  结构/转义/预算；interpreter 路径不改变这些断言。
- **实现落点（2026-08-21）**：`floatile-shell::runtime_ui` 落地 ADR 决策 A——`parse_document`
  （字节/结构/预算复验，恶意 IR 前置拒绝）、`compile_component`（interpreter 运行时编译）、
  `RuntimePluginWindow::create_on_ui_thread`（自窗口 + `floatile-platform` 置顶）、沿 renderer
  binding/event 槽位的 State 投影与输入事件回投；`spawn_runtime_ui` 编排 runtime worker 投影。
  `slint-interpreter` 升为 shell 正式依赖（1.17 同版本，无 slint-build 图像链）。证据：
  `runtime_ui` 单测（headless，F12 拒绝 + 编译）与 `tests/runtime_ui_window.rs`（Xvfb：编译+
  实例化+State 投影往返+事件回投）全绿；instance 化与首帧性能仍待回填性能验收表。
- **运行边界加固（2026-08-22）**：`spawn_runtime_ui` 在专用线程完成 FTUI 解析、复验和 renderer
  生成，再由 Slint local task 启动；生产路径不再嵌套 Tokio `block_on`。输入事件桥改为容量 64 的
  非阻塞 `try_send`，过载立即丢弃、worker 聚合脱敏审计，每轮最多转发 8 个事件。Slint 1.17 的
  `ComponentDefinition` 为 `!Send`，因此 interpreter 编译与实例化仍在同一 UI executor；这是待性能
  实测的已知限制，不得通过跨线程 `unsafe` 绕过。
