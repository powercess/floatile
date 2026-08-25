# WIT 插件 API v1

> 状态：Proposed（ADR-0001 目标契约）；WIT、host/guest bindings 与 clock fixture 已迁移到目标形状并通过
> `wasm-tools validate`，runtime adapter 与 contract tests 待实现
> 唯一源：`wit/`；本文解释语义，不复制一套可独立修改的绑定
> engine API：`floatile:widget@1.x`
> 关联：ADR-0001、FR-PLUGIN-01、FR-PERM-01、F11、F12

## 0. 当前实现与迁移门

`wit/floatile-widget.wit@1.0.0`、`floatile-sdk` guest bindings、`floatile-plugin-api` host async
bindings 与 `plugins/clock-wasm` 已迁移到 ADR-0001 目标契约形状：包含 `host-ui`/`host-clock`、canonical
initial State、统一 `start/handle-event/stop` lifecycle 与稳定 `widget-error`。host/guest 均从 `wit/`
单一源生成，engine version 一致，clock fixture 通过 `wasm-tools validate`，验证了 stable Rust、
wit-bindgen、Wasmtime bindgen 与 Component 构建链路。

项目尚未对外分发插件，现有 `1.0.0` 不是兼容承诺。`floatile-runtime` 已实现 Wasmtime adapter 并经
`PermissionBroker` 接入全部七个接口，`clock-wasm` 集成测试（start/1Hz tick/update-state）通过；仍缺
§9 的契约测试、恶意插件 fixture 与 CLI 包校验。这些落地前不得把 WIT/SDK 标记为 ADR-0001 契约
已 Implemented，也不得接入 shell 制造第二套适配层。

## 1. v1 目标

WIT 只负责 WASM plugin 与 host 的跨边界调用。UI 结构由 `widget.ftui` 描述，Slint 不进入 WIT，
插件也不能获得 UI 节点、窗口、renderer、service 或原生句柄。

公开 SDK 将 WIT 包装成 `State / View / Event / Context`；普通作者不得手写 WIT 或直接调用生成
模块。v1 必须支持：

1. 宿主创建、启动、串行投递事件、停止并销毁一个实例；
2. 插件通过 State Patch 更新当前实例 UI；
3. log、clock、timer、storage、metrics、theme 全部由 Broker 仲裁；
4. 所有拒绝、配额、输入、环境和内部错误使用稳定 variant；
5. host/guest 都从同一 WIT 生成，CI 验证版本和签名一致。

## 2. Interface 分类

| interface | SDK 表面 | grant | P0 |
|---|---|---|---|
| `host-ui` | `ctx.state` | 固有实例能力；严格实例/schema/quota scope | 必须 |
| `host-log` | `ctx.log` | 固有实例能力；限速与脱敏 | 必须 |
| `host-clock` | `ctx.clock` | 固有只读能力；不暴露系统句柄 | 必须 |
| `host-timer` | `ctx.timer` | `timer:schedule` | 必须 |
| `host-storage` | `ctx.storage` | `storage:read/write` | 必须 |
| `host-metrics` | `ctx.metrics` | `system:cpu/memory` | 必须 |
| `host-theme` | `ctx.theme` | `theme:subscribe` | 必须 |
| `widget-contract` | `Widget` lifecycle | host 调 guest export | 必须 |

能力即接口不等于导入即授权。world 提供可链接的接口，manifest + user grants 决定有效授权；每次调用
仍经过 Broker。固有能力也只能操作当前实例并执行固定预算，不能绕过 Broker。

## 3. 建议的 v1 形状

以下用于说明拟定形状；实现时 `wit/` 文件是唯一可编译事实源。字段或 case 调整必须同步本文、
bindings、runtime adapter、版本和 contract tests。

```wit
package floatile:widget@1.0.0;

interface host-ui {
    variant ui-error {
        not-allowed,
        invalid-json,
        schema-mismatch(string),
        patch-too-large,
        state-too-large,
        update-rate-exceeded,
        queue-full,
        internal,
    }

    // JSON Merge Patch；宿主原子应用并验证完整 State。
    update-state: func(patch-json: string) -> result<_, ui-error>;
}

interface host-log {
    enum log-level { debug, info, warn, error }
    variant log-error { rate-exceeded, message-too-large }
    log: func(level: log-level, message: string) -> result<_, log-error>;
}

interface host-clock {
    record wall-time {
        unix-millis: u64,
        utc-offset-minutes: s32,
    }
    now: func() -> wall-time;
}

interface host-timer {
    type timer-id = u32;
    variant timer-error {
        not-allowed,
        budget-exceeded,
        invalid-delay,
        invalid-timer-id,
        unavailable,
    }
    schedule: func(delay-ms: u64) -> result<timer-id, timer-error>;
    cancel: func(timer-id: timer-id) -> result<_, timer-error>;
}

interface host-storage {
    variant storage-error {
        not-allowed,
        invalid-key,
        quota-exceeded,
        unavailable,
        internal,
    }
    get: func(key: string) -> result<option<string>, storage-error>;
    set: func(key: string, value: string) -> result<_, storage-error>;
    delete: func(key: string) -> result<_, storage-error>;
}

interface host-metrics {
    record memory-snapshot { rss-kib: u64, virtual-kib: u64 }
    variant metrics-error { not-allowed, rate-exceeded, unavailable }
    cpu-percent: func() -> result<f64, metrics-error>;
    memory: func() -> result<memory-snapshot, metrics-error>;
}

interface host-theme {
    type subscription-id = u32;
    variant theme-error {
        not-allowed,
        unknown-token,
        invalid-subscription,
        unavailable,
    }
    get-token: func(name: string) -> result<option<string>, theme-error>;
    subscribe: func() -> result<subscription-id, theme-error>;
    unsubscribe: func(id: subscription-id) -> result<_, theme-error>;
}

interface widget-contract {
    record widget-init {
        config-json: string,
        initial-state-json: string,
    }

    record ui-event {
        name: string,
        payload-json: string,
    }

    enum widget-mode { edit, show }

    variant widget-event {
        ui(ui-event),
        timer(u32),
        mode-changed(widget-mode),
        config-changed(string),
        theme-changed(string),
        suspend,
        resume,
    }

    variant widget-error {
        invalid-input(string),
        rejected(string),
        internal,
    }

    resource widget-instance {
        constructor(init: widget-init);
        start: func() -> result<_, widget-error>;
        handle-event: func(event: widget-event) -> result<_, widget-error>;
        stop: func();
    }
}

world floatile-widget {
    import host-ui;
    import host-log;
    import host-clock;
    import host-timer;
    import host-storage;
    import host-metrics;
    import host-theme;
    export widget-contract;
}
```

## 4. 生命周期语义

- host 完成 manifest、UI IR、WASM 和 config 校验后才调用 constructor。
- constructor 的 initial State 是 host 从已验证 UI IR 取得的 canonical JSON；SDK 以它初始化 typed
  mirror，不能自行猜默认值。
- constructor 不做 I/O；第一次 capability 调用从 `start` 开始。
- 同一 instance 的 `start/handle-event/stop` 严格串行；runtime 不重入 guest。
- UI callback、timer、mode、config、theme、suspend/resume 统一进入 `widget-event`，避免每增加宿主事件都
  扩展 resource method。
- `stop` 只有短预算且可被取消；资源释放最终由 resource drop 保证。插件不能依赖 stop 持久化
  尚未提交的数据。
- guest `widget-error` 是业务拒绝；trap、fuel、timeout、memory、queue overflow 由 runtime error
  模型报告，不能伪装成 guest 返回值。

## 5. UI State Patch 语义

`host-ui.update-state` 是完成 F11 的唯一 plugin→UI 通道：

1. runtime 从当前实例取得 State 与 UI IR 内嵌 schema；插件不能传 instance id。
2. 验证 UTF-8/JSON、patch 大小、深度与更新频率。
3. 在副本上应用 JSON Merge Patch，对完整 State 做 schema 与总大小校验。
4. 成功后原子替换 runtime State，并把有界更新投递 Slint 主线程。
5. 任一步失败时旧 State 不变，返回具体 `ui-error` 并写脱敏审计。

host runtime State 是权威副本。SDK mirror 只在 `update-state` 成功返回后提交同一修改；失败必须回滚
本地候选，避免 UI 与 guest 逻辑 split-brain。

禁止把 JSON pointer、Slint property、组件 ID 或任意表达式作为更新目标。字段合法性完全由 State
schema 决定。P0 选择 JSON 是为了跨语言和诊断简单；未来更换编码不改变 State Patch 语义。

## 6. 事件语义

- UI event 名与 payload schema 来自 `widget.ftui`；未知事件和错误 payload 在进入 WASM 前拒绝。
- Timer 是一次性：周期由 SDK 在 tick 后重新 schedule；SDK 可以提供 `every` 便利包装，但仍受
  active timer 与 max/minute 配额。
- Config 已由 manifest 指定 schema 校验，再作为完整 JSON 发送；插件必须明确处理版本/default。
- Theme change 发送有大小上限的 token snapshot JSON；只有已订阅实例接收，SDK 解析为 typed snapshot，
  非法 payload 在进入 guest 前拒绝。纯 UI theme token 由 renderer 自动更新，不要求插件回写 State。
- mode 只是通知，插件不得借此调用窗口穿透、置顶或 Edit/Show 切换。
- suspend 后普通 timer/event 可以合并或暂停；resume 必须按文档提供最新 config/theme snapshot。

## 7. 异步与线程

host imports 由 Wasmtime async adapter 实现。异步不代表同一实例并行；runtime 的 per-instance
actor 等待当前回调完成或超时后才取下一个 event。任何 host import 都不得阻塞 Slint 主线程。

`host-ui` 写 runtime State 并异步投递 UI；不能直接调用 Slint。Storage、Timer、Metrics、Theme 由
services 实现，adapter 只能持有 Broker/instance context，不能持有原生能力句柄。

ADR-0004 已选择“capability-specific typed submit/take-result + 通用 Operation ID/cancel + 仅元数据
completion signal”作为未来长任务合约，并由宿主 spike 验证队列、deadline、取消、generation 与
过载语义。该合约**不属于当前 v1.0 WIT**：`wit/floatile-widget.wit`、bindings、SDK 与
`engineApiVersion` 在本 spike 中均未变化。下一 PP-M2 切片必须先修改 WIT 唯一源并完成 §8 的全部
联动；在此之前不得暴露临时 import 或把宿主 Rust API 描述为插件 API。

## 8. 版本与兼容

- WIT package/world 版本对应 `engineApiVersion`，不对应 `uiApiVersion` 或 SDK 包版本。
- major 不一致拒绝；同 major 的 minor 只允许增加可选 interface/function/case，并需要兼容测试。
- capability 不存在或环境不支持时，SDK 提供显式 unavailable；宿主不得伪造成功。
- WIT 改动必须：更新 `wit/` → host/guest bindings → runtime adapter → API 文档 → version →
  supported/rejected contract tests → clock/evil fixtures。

## 9. 必须的 contract tests

P0 至少验证：

- host 与 guest bindings 来自同一 WIT，engine API 一致；
- 支持的版本加载、major 不匹配拒绝、未知 minor capability 显式降级；
- lifecycle 顺序、同实例不并发、stop/drop 可取消；
- State Patch 正常、类型错误、未知字段、超大、过深、洪泛、queue full 与原子回滚；
- UI event schema 正常/错误/未知事件；
- capability allow/deny/scope/quota/unavailable 与审计脱敏；
- trap/fuel/memory/timeout 后宿主和其他实例存活。

## 10. 非目标

v1 不提供文件、网络、命令、原生窗口、Slint、DOM、跨插件通信、secret 或 raw WASI ambient
capability。没有 interface 就没有路径；不得用临时 host function 绕开 WIT/Broker。
