# Floatile P0 技术设计文档

> 版本：draft-0.1
> 状态：Proposed
> 范围：P0（技术可行性验证）。目标不是堆砌功能，而是暴露窗口层和 Wayland 风险，为 MVP 打下可验证的垂直切片。

## 1. P0 目标与非目标

### 目标（Do）
- 在 Windows、macOS、Linux(X11) 上验证「透明无边框 + Always-on-top + 点击穿透 + 编辑/展示模式切换」。
- 在 Wayland 上验证能力分级与降级策略，明确能做什么、不能做什么。
- 验证多显示器、DPI 缩放、显示器热插拔下的布局恢复。
- 验证基础拖拽、缩放与布局持久化。
- 验证 Slint 运行时动态加载 `.slint` UI。
- 交付一个硬编码时钟组件（Reference Widget）。
- 交付一个最小 `.slint + .wasm` 插件（Wasmtime + Component Model + WIT）。
- 验证插件无法绕过 Permission Broker 调用原生能力。
- 产出三端能力矩阵（见 `docs/platform-matrix/platform-matrix.md`）。

### 非目标（Don't）
- 不做插件市场、多画布、主题系统、凭证托管、跨插件通信、Sidecar。
- 不做安装器/签名/更新（只在 MVP 做）。
- 不做全部四种插件类型，P0 只有 Widget 型。
- 不做完备的无障碍与高对比度（P0 只记录降级行为）。

## 2. 架构总览

P0 采用与最终架构一致的分层，但每层只实现「最小可用」：

```
floatile-shell (bin)
├─ canvas · layout · edit/show mode · persistence
├─ widget-host       —— 一个 widget 的 Slint 实例宿主
├─ plugin-manager    —— 安装/加载/校验（P0 只支持 dev 目录加载）
├─ permission-broker —— 唯一原生能力入口
└─ runtime
   ├─ slint-runtime   —— 动态编译 .slint
   └─ wasm-runtime    —— wasmtime 组件运行时
      └─ platform     —— 平台抽象（transparency / click-through / AOT / edit mode）
```

关键决策（P0 就定死，后续不得放宽）：

1. **能力入口唯一化**：插件触达的任何宿主能力（存储、计时器、指标、文件、网络、命令）都必须经由 `PermissionBroker` 路由。P0 只实现日志、存储、计时器、CPU/内存指标四类能力，且全部过 Broker。
2. **双模式**：`EditMode`（可交互、显示边框/手柄）与 `ShowMode`（可选点击穿透、隐藏控制元素）。点击穿透与可交互天然冲突，由宿主在模式切换时统一管理，插件不得自行切换。
3. **零权限默认**：插件未声明任何能力时，只能获得「纯渲染 + 被动 UI 事件回传」。
4. **WIT 即契约**：宿主与插件之间只有 WIT 定义的接口；宿主不暴露任何原始函数指针、模块或原生句柄给插件。

## 3. 线程模型与事件循环

- 主线程只跑 Slint/winit 事件循环。
- Tokio 多线程 runtime 在后台运行，承载 wasmtime(异步)、存储、网络。
- 跨线程边界使用 `tokio::sync::mpsc` + Slint `invoke_from_event_loop` 回投 UI 线程。
- **禁止在 Slint 回调里做阻塞 I/O 或同步等待 wasm**。

```
[Slint main loop]  <--invoke_from_event_loop--   [Tokio runtime]
        |                                            |
        +-------- widget host (edit frame, handles)   +-- wasmtime async component call
        +-------- permission broker (sync checks)     +-- sqlite / keyring / http
```

## 4. 渲染与 DPI

- Slint 默认 winit 后端（OpenGL/Metal/D3D），GPU 加速；软件渲染器作为 Linux 无 GPU/驱动异常时的降级后端。
- DPI 缩放：交给 Slint/winit 的 scale factor；所有布局以逻辑像素存储，保存时同时记录物理尺寸与 scale factor。
- 透明窗口：Slint 窗口背景设 `Color::TRANSPARENT`，窗口管理器层面按平台矩阵实现（见平台矩阵文档）。

## 5. 布局模型与持久化

- 逻辑坐标（逻辑像素）+ 所属 monitor（以 EDID/product 指纹标识）+ scale factor。
- 持久化到 SQLite（`floatile-store`）：`layout` 表（instance_id, plugin_id, monitor_key, x, y, w, h, z, mode, updated_at）。
- 热插拔恢复：启动与 `MonitorListChanged` 事件时重算；找不到原 monitor 时落到主屏并标记 `lost_monitor`。

## 6. 插件加载管线（P0 最小）

1. `PluginManager::discover(path)` 读取 `manifest.json`（P0 允许 dev 目录，跳过签名）。
2. 校验 `engineApiVersion` 兼容性（语义版本）。
3. 解析权限声明 → 构建 `Grants`。
4. Slint 运行时编译 `ui/widget.slint`（`slint_interpreter::Compiler`）。
5. wasmtime 引擎（`component-model` feature）加载 `logic/plugin.wasm`，实例化 component。
6. 将编译好的 Slint 组件放入画布，宿主绑定 UI 事件回调 → wasm 的 `handle_ui_event`。
7. 每次宿主能力调用先过 `PermissionBroker`。

## 7. 硬编码时钟（Reference Widget）

- 不经过插件管线，直接内建一个时钟 Slint 组件，作为：
  - 验证透明/置顶/点击穿透/编辑模式的基准载体；
  - 后续对比「插件化时钟」的行为与性能基线。

## 8. 最小插件（clock.wasm）

- 语言：Rust，target `wasm32-wasip2`，wit-bindgen 生成 guest 绑定。
- 职责：注册 `timer:schedule`（1s 周期），`get_local_time()` 返回格式化的时间字符串，宿主把值写回 Slint 属性。
- 目录：`plugins/clock-wasm`（作为 SDK 使用示例）。

## 9. 安全边界（P0 实测项）

- **插件无法读到宿主内存**：wasmtime 隔离。
- **插件无法直接碰文件/网络/命令**：P0 未实现这些能力 → 无接口 = 无路径。
- **插件无法绕过 Broker**：Broker 是宿主能力唯一入口，P0 以「恶意插件测试」形式证明（见验收标准）。
- **.slint 不是安全边界**：动态编译的 .slint 在宿主进程内执行，只能调用宿主暴露的回调/属性，但仍应视为不可信输入，后续用受限执行 + 回调白名单约束。

## 10. 可观测性

- `tracing` 全链路：事件（event）、span（instance_id/plugin_id）。
- 审计日志独立 target `floatile::audit`，可配置输出到 SQLite `audit_log` 表。
- P0 埋点：能力调用（permission 决策 + redacted 参数）、模式切换、窗口创建/移动、插件加载/实例化、崩溃（host）计数。

## 11. 交付物清单（P0 结束时应产出）

- [ ] 平台矩阵文档（实测数据回填）
- [ ] 三端可运行的 host（Win/macOS/X11，Wayland 有降级）
- [ ] 硬编码时钟 + 插件化时钟
- [ ] 布局持久化 + 热插拔恢复
- [ ] 恶意插件安全测试用例 + 结果
- [ ] P0 验收指标实测记录
- [ ] 依赖/假设复盘（哪些假设被推翻，哪些工具链版本锁定）

## 12. 与后续阶段的关系

- P0 冻结的：WIT 语义、Broker 决策模型、SQLite 表结构骨架、坐标/DPI 数据模型。
- P0 不冻结的：插件包格式细节、证书/签名格式、HTTP Broker 协议（MVP 再定）。
