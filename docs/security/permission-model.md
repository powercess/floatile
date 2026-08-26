# 权限模型设计

> 状态：Accepted（安全边界）；默认配额在 P0 恶意插件测试后冻结
> 原则：**零权限默认（deny-by-default）**。所有宿主能力调用必须经过 `PermissionBroker`，先校验后执行。
> 粒度：按 `pluginId + instanceId` 授权与审计。

## 1. 能力注册表（Capability Registry）

能力由宿主定义。固有实例能力无需用户确认，但仍经过 Broker 的实例 scope、schema、配额与审计；
声明能力必须同时出现在 manifest 与有效 grants 中。

P0 能力的机器可读事实源是 `floatile_core::CAPABILITY_REGISTRY`。每项记录稳定 ID/名称、固有或声明
暴露方式、参数族、风险等级、执行形态、WIT interface、SDK 表面、作者配置段和审计脱敏策略。
Capability serde、manifest permission JSON Schema、CLI 作者段展开和 Broker 固有 grant 均消费该注册表；
契约测试校验枚举顺序/名称唯一性、WIT capability interface 覆盖、manifest 声明集合和 Broker 默认拒绝。

### 1.1 固有实例能力

| capability | 含义 | 强制 scope/配额 |
|---|---|---|
| `ui:update-state` | 更新当前实例 State | 仅当前实例；schema；patch/state 大小、深度、频率与 UI 队列上限 |
| `log:write` | 写插件日志 | 当前实例 span；消息长度/频率；参数脱敏 |
| `clock:read` | 当前 wall time 与 UTC offset | 只读；不暴露系统/平台句柄 |

固有不等于 ambient：插件不能传入或伪造 `instance_id`，不能操作其他实例，也不能请求扩大固定 scope。

### 1.2 声明能力

P0/MVP 最小集：

| capability | 含义 | 作用域参数（params） | 配额默认 |
|------------|------|----------------------|----------|
| `storage:read` | 读插件私有 KV | `keys: []`（默认全部私有键） | — |
| `storage:write` | 写插件私有 KV | `keys: []`；大小限制 | 64 KiB/实例 |
| `timer:schedule` | 请求宿主定时回调 | `maxPerMinute` | 60/min，活跃≤8 |
| `theme:subscribe` | 订阅主题变化、读 token | — | 1 订阅/实例 |
| `system:cpu` | 读本进程 CPU 占用 | `sampleRateHz` | 1 Hz |
| `system:memory` | 读本进程内存 | — | — |
| `notification:show` | 发系统通知（V1+） | `maxPerHour` | 30/h |
| `clipboard:read/write` | 剪贴板（V1+） | — | 需显式授权 |
| `launcher:open-url` | 打开 URL（V1+） | `schemes: [https]`；域名白名单 | — |
| `launcher:open-approved-app` | 打开已批准应用（V1+） | 应用路径白名单 | — |
| `network:https://<domain>/*` | 受控 HTTP（V1+，经 HTTP Broker） | 域名模式 | — |
| `network:websocket:wss://<domain>/*` | 受控 WebSocket（V1+） | 域名模式 | — |
| `network:localhost` | 本地回环访问（单独授权） | — | 高危，需二次确认 |
| `file:read:user-selected` | 读用户显式选择的文件（V1+） | 文件范围 | 仅会话内 |
| `publish:<topic>` / `subscribe:<topic>` | 跨插件通信（V2+） | 主题名 | — |

未注册的能力名 → manifest 校验失败。

## 2. 权限对象模型

```rust
pub struct Grant {
    pub capability: Capability,
    pub params: ScopeParams,      // 域名/键/配额等
    pub effective: EffectiveGrant, // Prompted | Explicit | DerivedFromInstall
}

pub struct Grants {
    pub plugin: PluginId,
    pub instances: Vec<InstanceGrant>, // 每实例可收窄，不可放宽
    pub trust: TrustLevel,             // none | dev | signed-untrusted | signed-trusted
}
```

- **实例级收窄**：安装时授予的是「上限」，运行时可被权限管理器收窄，不得放宽。
- **密钥/凭证永远不进入插件**：插件只能持有 credential reference（`cred://<id>`），由宿主注入（见 HTTP Broker）。

## 3. 决策模型（Check → Decide → Execute）

```
check(plugin_id, instance_id, capability, args)
  → 1. runtime context 是否绑定有效 plugin_id + instance_id？
  → 2. capability 已注册且 WIT interface/function 匹配？
  → 3. 固有 scope 或该实例 grants 是否覆盖 capability？
  → 4. 参数/schema/作用域匹配（State、键、域名、数量等）？
  → 5. 配额和 runtime budget 可用？环境能力可用？
  → 6. 通过 → execute；失败 → 具体错误 + 脱敏审计
```

- **决策缓存**：同一实例、同一 capability、同 scope 的决策结果缓存（LRU），降低热路径开销；scope 敏感项不缓存（如 `file:read` 具体路径）。
- **降级**：能力探测（`CapabilityProbe`）结果为不可用时（如 Wayland 无穿透、X11 无合成器）→ 在授权前 `Deny::EnvironmentUnavailable`，并在审计中记录原因。

## 4. 敏感能力与二次确认

| 等级 | 能力 | 处理 |
|------|------|------|
| 固有 | ui state, log, clock | 不提示；固定当前实例 scope 与硬预算，不可放宽 |
| L0 低风险 | storage, timer, theme, metrics | 安装时由 manifest 声明即授予（可收窄） |
| L1 中风险 | notification, clipboard:write | 安装时提示，运行时首启二次确认 |
| L2 高风险 | network, clipboard:read, file:read, launcher | 每次实例激活时重新确认 + 会话内超时失效 |
| L3 特高风险 | network:localhost, 任意命令, 原生二进制 | 默认禁止；如未来支持需签名 + OS 级隔离 + 独立授权流程 |

P0/MVP 只实现固有能力与 L0。

## 5. 资源配额与运行预算（Runtime Budget）

- **内存**：wasmtime 线性内存上限（默认 16 MiB/实例）+ 数据段上限。
- **CPU**：fuel 计量，默认每秒预算；超出触发 `trap`，宿主按配置决定终止或暂停。
- **频率**：能力调用频率（如 `system:cpu` 采样 ≤1Hz）；`maxPerMinute` 计时器上限。
- **后台预算**：widget 隐藏/最小化时，周期回调降频或挂起，累计后台秒数计入配额。
- **事件队列**：每实例有界、串行；溢出返回 `queue-full`，不得无限分配或阻塞 UI 线程。
- **异步 Operation**：每 instance generation 的提交队列、完成队列、并发数和 retained result 数均有
  硬上限；无效 deadline、满载或关闭必须在执行 work 前失败。旧 generation、actor 满载/关闭时丢弃
  payload，不能无限保留等待 guest。
- **UI State**：单 patch、完整 State、嵌套深度、每秒更新次数有硬上限；schema 校验后原子应用。
- **UI IR/Canvas**：节点、binding、If/ForEach 展开量、Canvas 指令/点数、asset 数量与解码后尺寸有硬上限。

## 6. 审计（Audit）

- 所有敏感调用写审计日志（`tracing` target `floatile::audit`，并经宿主注入的
  `AuditListener` 持久化到 SQLite `audit_log`——store migration v3 + shell 运行时
  `with_audit_listener`；已实现并有安全集成测试断言「拒绝 + 审计落库 + 宿主存活」）。
- 字段：时间、plugin_id、instance_id、capability、decision、reason、**redacted args**。
- 脱敏规则：
  - `storage:*` 的 value 不落盘（只记 key + 长度 hash）；
  - `ui:update-state` 不记 patch/State 内容，只记字节数、字段数、结果与错误路径 hash；
  - `log:write` 本身进入插件日志而非 capability 参数审计；超限/拒绝另记审计；
  - 网络请求的 Authorization/Cookie/自定义敏感头只记存在性（布尔），不记值；
  - `system:*` 结果不记具体数值（可记范围分桶）。
  - Operation 只记 ID、capability、动作、稳定终态/失败码、delivery disposition 和脱敏尺寸；请求/
    结果 payload 不进入 completion signal、audit detail 或插件 State。
- 审计日志仅供宿主 UI 查看（用户可在权限中心筛选），插件读不到。

## 7. 与 WIT 的关系

- WIT interface 是「能力面的表达」，Permission Broker 是「能力面的裁决」。二者必须一致：新增
  interface/function 必须同步 capability registry、scope、quota、错误、审计脱敏与恶意 fixture。
- CI 校验 WIT host imports 都有 registry 项；固有能力也必须有固定 scope，不能以“内部接口”为由
  绕过 Broker。
- `widget.ftui` 只描述 UI。它不能声明能力或创建第二条 host 调用路径；manifest permissions 才是
  安装授权上限。
- ADR-0004 的 Operation submit 必须在一次 Broker 入口中完成 check→execute；cancel 与 typed
  `take-result` 同样重新授权。v1.1 已接入 `storage:read` 的 typed submit/take、通用 cancel 和元数据
  completion；后续能力仍必须联动 schema、SDK、版本、contract vectors 与恶意 fixture，禁止通用
  JSON-RPC/capability bus。

## 8. 未决问题

1. P0 是否引入「运行时权限提示 UI」（倾向 MVP 再做，P0 只有日志和开发者面板）。
2. `network:localhost` 的确认流程细节（V1 定）。
3. 默认预算数值在 clock/evil/10-instance 数据后冻结；隔离与计量必须先按实例实现，宿主还可以增加
   全局上限，防止插件通过创建大量实例规避预算。
