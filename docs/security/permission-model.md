# 权限模型设计

> 状态：Proposed
> 原则：**零权限默认（deny-by-default）**。所有宿主能力调用必须经过 `PermissionBroker`，先校验后执行。
> 粒度：按 `pluginId + instanceId` 授权与审计。

## 1. 能力注册表（Capability Registry）

能力由宿主定义，插件通过 manifest `permissions` 声明。P0/MVP 最小集：

| capability | 含义 | 作用域参数（params） | 配额默认 |
|------------|------|----------------------|----------|
| `storage:read` | 读插件私有 KV | `keys: []`（默认全部私有键） | — |
| `storage:write` | 写插件私有 KV | `keys: []`；大小限制 | 64 KiB/实例 |
| `timer:schedule` | 请求宿主定时回调 | `maxPerMinute` | 60/min，活跃≤8 |
| `theme:subscribe` | 订阅主题变化、读 token | — | 1 订阅/实例 |
| `system:cpu` | 读本进程 CPU 占用 | `sampleRateHz` | 1 Hz |
| `system:memory` | 读本进程内存 | — | — |
| `notification:show` | 发系统通知 | `maxPerHour` | 30/h |
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
  → 1. capability 已注册？
  → 2. 该实例 grants 是否覆盖 capability？
  → 3. 作用域匹配（域名/键/配额）？
  → 4. 配额可用（频率、大小、数量）？
  → 5. 通过 → execute；失败 → 返回 permission denied + 审计记录
```

- **决策缓存**：同一实例、同一 capability、同 scope 的决策结果缓存（LRU），降低热路径开销；scope 敏感项不缓存（如 `file:read` 具体路径）。
- **降级**：能力探测（`CapabilityProbe`）结果为不可用时（如 Wayland 无穿透、X11 无合成器）→ 在授权前 `Deny::EnvironmentUnavailable`，并在审计中记录原因。

## 4. 敏感能力与二次确认

| 等级 | 能力 | 处理 |
|------|------|------|
| L0 低风险 | log, storage, timer, theme, metrics | 安装时由 manifest 声明即授予（可收窄） |
| L1 中风险 | notification, clipboard:write | 安装时提示，运行时首启二次确认 |
| L2 高风险 | network, clipboard:read, file:read, launcher | 每次实例激活时重新确认 + 会话内超时失效 |
| L3 特高风险 | network:localhost, 任意命令, 原生二进制 | 默认禁止；如未来支持需签名 + OS 级隔离 + 独立授权流程 |

P0/MVP 只实现 L0。

## 5. 资源配额与运行预算（Runtime Budget）

- **内存**：wasmtime 线性内存上限（默认 16 MiB/实例）+ 数据段上限。
- **CPU**：fuel 计量，默认每秒预算；超出触发 `trap`，宿主按配置决定终止或暂停。
- **频率**：能力调用频率（如 `system:cpu` 采样 ≤1Hz）；`maxPerMinute` 计时器上限。
- **后台预算**：widget 隐藏/最小化时，周期回调降频或挂起，累计后台秒数计入配额。

## 6. 审计（Audit）

- 所有敏感调用写审计日志（`tracing` target `floatile::audit` → SQLite `audit_log`）。
- 字段：时间、plugin_id、instance_id、capability、decision、reason、**redacted args**。
- 脱敏规则：
  - `storage:*` 的 value 不落盘（只记 key + 长度 hash）；
  - 网络请求的 Authorization/Cookie/自定义敏感头只记存在性（布尔），不记值；
  - `system:*` 结果不记具体数值（可记范围分桶）。
- 审计日志仅供宿主 UI 查看（用户可在权限中心筛选），插件读不到。

## 7. 与 WIT 的关系

- WIT interface 是「能力面的表达」，Permission Broker 是「能力面的裁决」。二者必须一致：新增 interface 必须同步注册新 capability，CI 校验「WIT 中 import 的能力 ⊆ 已注册能力」。

## 8. 未决问题

1. P0 是否引入「运行时权限提示 UI」（倾向 MVP 再做，P0 只有日志）。
2. `network:localhost` 的确认流程细节（V1 定）。
3. 配额是否跨实例共享（倾向按实例独立，避免一个插件多个实例拖垮宿主）。
