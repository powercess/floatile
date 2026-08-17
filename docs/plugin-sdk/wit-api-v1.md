# WIT 插件 API v1 草案

> 状态：Implemented（契约与绑定）
> 契约定义于 `wit/` 目录，单一事实源。
> 宿主与 guest 都由同一 WIT 生成绑定（`wasmtime::component::bindgen!` / wit-bindgen）。
> 版本：`floatile:widget@1.0.0`（对应 `engineApiVersion = "1.0.0"`）。
> 已落地：`wit/floatile-widget.wit`、`floatile-sdk`（guest 绑定 + `export_widget!`）、
> `floatile-plugin-api`（host async 绑定）、`plugins/clock-wasm` 组件构建（wasm-tools
> validate 通过）。未落地：wasmtime runtime 加载执行（S5b）、Broker 配额（S5c）。

## 1. 设计原则

1. **能力即接口**：每个宿主能力（storage、timer、metrics…）是独立 interface；`world` 里显式 import 才算获得。
2. **P0 最小能力集**：`log` / `storage` / `timer` / `metrics` / `theme(subscribe)`。网络、文件、通知、命令等能力在 V1 才加入，P0 里**根本没有对应接口 = 无攻击面**。
3. **跨边界数据用 JSON 字符串**（v1 简化）：UI 事件与配置都以 JSON 字符串传递，语义结构化在 WIT 里约定 schema 版本。避免 v1 就引入复杂的 interface 类型演进问题。
4. **资源模型**：`widget-instance` 是 resource，宿主创建实例、插件管理内部状态，生命周期由宿主强控（销毁时宿主调用 drop）。
5. **异步语义**：宿主接口标注 `async`；wasmtime 在 `async_support` 下执行。P0 的调用频率与耗时受配额限制（见 security 文档）。

## 2. 文件：`wit/floatile-widget.wit`

```wit
package floatile:widget@1.0.0;

// ---------- 宿主能力（插件 import） ----------

interface host-log {
  log: func(level: log-level, message: string);
}

enum log-level {
  debug,
  info,
  warn,
  error,
}

interface host-storage {
  get: func(key: string) -> result<option<string>, storage-error>;
  set: func(key: string, value: string) -> result<_, storage-error>;
  delete: func(key: string) -> result<_, storage-error>;
}

variant storage-error {
  not-allowed,      // 无 storage:read/write 权限
  quota-exceeded,   // 超出单实例配额（默认 64 KiB）
  internal,
}

interface host-timer {
  /// 请求宿主在 delay-ms 后调用 widget-instance.on-tick(timer-id)。
  /// 需 timer:schedule 权限；返回计时器句柄。
  schedule: func(delay-ms: u64) -> result<timer-id, timer-error>;
  cancel: func(timer-id: timer-id) -> result<_, timer-error>;
}

type timer-id = u32;

variant timer-error {
  not-allowed,       // 无 timer:schedule
  budget-exceeded,   // 超 maxPerMinute / 活跃计时器上限
  invalid-timer-id,
}

interface host-metrics {
  /// 需 system:cpu 权限。返回 0.0..100.0 的占用率（进程级，非整机，避免信息泄露面）。
  cpu-percent: func() -> result<f64, metrics-error>;
  /// 需 system:memory 权限。只返回本进程内存快照（RSS/虚拟），不暴露其他进程信息。
  memory: func() -> result<memory-snapshot, metrics-error>;
}

record memory-snapshot {
  rss-kib: u64,
  virtual-kib: u64,
}

variant metrics-error {
  not-allowed,
  unavailable,
}

interface host-theme {
  get-token: func(name: string) -> result<option<string>, theme-error>;
  /// 订阅主题变化；宿主通过 widget-instance.on-theme-changed 通知。
  subscribe: func() -> result<subscription-id, theme-error>;
  unsubscribe: func(id: subscription-id) -> result<_, theme-error>;
}

type subscription-id = u32;

variant theme-error {
  not-allowed,
  unknown-token,
  invalid-subscription,
}

// ---------- 插件实现（宿主 import） ----------

interface widget-contract {
  resource widget-instance {
    /// 宿主创建实例：config 为 manifest.config.schema.json 校验后的 JSON 字符串。
    constructor(config: string);
    /// UI 事件（回调名 + JSON 参数），宿主从 Slint 回调桥接而来。
    handle-ui-event: func(event: ui-event);
    /// 计时器到期回调。
    on-tick: func(timer-id: timer-id);
    /// 主题变化回调（仅订阅后触发）。
    on-theme-changed: func();
    /// 宿主要求展示模式切换（插件可做内部状态调整，非必须）。
    on-mode-changed: func(mode: widget-mode);
    /// 宿主销毁实例前的最后通知。
    destroy: func();
  }

  record ui-event {
    name: string,
    /// JSON 序列化参数，schema 由 SDK 文档约定。
    payload: string,
  }

  enum widget-mode {
    edit,
    show,
  }
}

// ---------- world ----------

world floatile-widget {
  import host-log;
  import host-storage;
  import host-timer;
  import host-metrics;
  import host-theme;

  export widget-contract;
}
```

## 3. 调用链示例（P0 时钟）

```
[Slint: 按钮 click]  ──桥接──> host 调 widget-instance.handle-ui-event({name:"start",payload:"{}"})
                                 └─> wasm (plugin) 调 host-timer.schedule(1000)
                                       └─> Broker: timer:schedule 权限 + maxPerMinute 配额
                                            └─> tokio sleep(1s)
                                                 └─> 宿主调 widget-instance.on-tick(id)
                                                      └─> 插件算时间，调 host-log / host-metrics
                                                           └─> 宿主把返回值 set 到 Slint 属性
```

- 宿主写回 UI：P0 用「宿主直接 set Slint 属性」（宿主持有组件句柄），插件通过 `handle-ui-event` 返回值（`ui-event` 响应或专用 `set-property`）→ 简化后为：**宿主在回调里按需 set 属性**，见 §4。

## 4. UI（.slint）与 wasm 逻辑的桥接约定

P0 桥接规则（写入 SDK 文档）：

- `.slint` 中声明的 `callback <name>(<json-args>)` 由宿主注册，回调触发时调用 `widget-instance.handle-ui-event({name, payload})`。
- 插件需要把数据写给 UI 时，通过 `host-ui.set-property(name, json)` 接口（**宿主校验属性名白名单：只允许 manifest/初始化时声明的属性**，防止插件篡改宿主属性）。
- 属性白名单在实例化时确定：`ui/set-properties: [...]` 从 manifest 读取（manifest 增加 `ui.properties` 字段，见 §5）。

### 补充接口 host-ui（在 v1 中与 host-theme 并列加入）

```wit
interface host-ui {
  /// 向本实例的 Slint 组件写入属性值（JSON 字符串）。
  /// 属性必须在 manifest.ui.properties 白名单中，否则返回 not-allowed。
  set-property: func(name: string, value: string) -> result<_, ui-error>;
}

variant ui-error {
  not-allowed,
  invalid-json,
  no-such-property,
}
```

> 说明：`host-ui` 属 v1 扩展点，P0 的最小插件可只用 `handle-ui-event` 回传 + 宿主预绑定静态属性。为保持文档完整，此处先并列记录，P0 实现若不需要则标记 deprecated 而不删除。

## 5. manifest 关联字段（新增）

```json
{
  "ui": {
    "properties": [
      { "name": "time_text", "type": "string" },
      { "name": "cpu", "type": "number" }
    ]
  }
}
```

## 6. 版本与演进策略

- world/interface 包版本 = `1.0.0`；对 `engineApiVersion` 语义：
  - **major** 不匹配 → 拒绝加载。
  - **minor** 兼容 → 宿主做降级（例如 host-ui 为 v1.1 新增，v1.0 宿主拒绝需要该接口的插件）。
- 每个 interface 独立版本化，未来某个能力接口升级不影响其他。
- WIT 变更流程：改 `wit/` → 重新生成 host + guest 绑定 → 更新 `engineApiVersion` → CI 里做「SDK 与 Host 绑定必须来自同一 commit 的 WIT」校验。

## 7. 未决问题（P0 评审确认）

1. `host-ui.set-property` 是否在 P0 落地，还是仅 `handle-ui-event` 回传（倾向：P0 只做后者，`host-ui` 留在 v1.1）。
2. 计时器粒度：仅一次性（delay）还是需要周期（interval）？倾向 v1 先只做一次性，周期由插件自建（每次 on-tick 重新 schedule）。
3. `ui-event` 是否需要返回值（`result<string, ui-event-error>`）？倾向 v1 返回 `result<_, ui-error>` 简化。
4. `metrics` 是否在 v1 暴露整机信息（当前只暴露进程级，作为默认保守值）。
