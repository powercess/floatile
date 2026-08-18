# Floatile 插件系统架构

> 状态：Accepted（架构边界）；具体 API 字段在 P0 契约实现与测试后冻结
> 范围：P0/MVP Widget 插件
> 关联：FR-PLUGIN-01、FR-PERM-01、FR-PACK-01、F11、F12、ADR-0001

本文是插件系统整体架构的事实源。WIT 字段以 `wit/` 为唯一源，包字段以 `manifest-v1.md` 为事实
源，权限与审计以 `permission-model.md` 为事实源；本文定义这些部分必须如何组合，避免每层各自
形成一套插件模型。

## 1. 开发者模型

普通插件作者只需要理解四个概念：

| 概念 | 含义 | 不暴露的内部实现 |
|---|---|---|
| `State` | 驱动 UI 的可序列化当前状态 | Slint property、UI 线程句柄 |
| `View` | 由标准组件组成的静态 UI 结构 | Slint 源码、渲染后端 |
| `Event` | UI、计时器、模式、配置与生命周期事件 | WIT resource、Tokio channel |
| `Context` | 访问 State 与经 Broker 仲裁的宿主能力 | Wasmtime store、原生服务句柄 |

Rust 和 TypeScript 只是两种 SDK 表面。它们最终必须生成相同的 `widget.ftui`、WASM Component、
manifest 和权限语义。不得因语言不同增加能力、改变错误或形成两套教程。

## 2. 包与运行时总览

```text
Rust source / TypeScript source
           │
           ├── Floatile UI compiler ──> ui/widget.ftui
           ├── language adapter ──────> logic/plugin.wasm
           └── manifest compiler ─────> manifest.json
                                      │
                                  .floatile
                                      │ validate limits/digests/schema
                                      ▼
                            PluginManager / Runtime
                    ┌─────────────────┴─────────────────┐
                    │                                   │
             UI IR + instance State             Wasmtime Component
                    │                                   │
          bounded UI-thread patches              serialized event actor
                    │                                   │
                 Slint host                 WIT → PermissionBroker → services
```

不可破坏的边界：

1. 插件不能携带或访问宿主原生代码、Slint 对象、窗口句柄、文件描述符或 service 实现。
2. `wit/` 是 host/guest 调用契约的唯一源；`widget.ftui` 只描述 UI，不得另建宿主能力通道。
3. 所有 plugin→host 调用都带隐式的 `plugin_id + instance_id`，经过 `PermissionBroker` 后才能执行。
4. Slint 主线程只接收宿主验证后的有界 State Patch，不等待 WASM、I/O、SQLite 或 Tokio。
5. manifest、UI IR、State Patch、事件 payload、WASM、配置和 assets 均是不受信任输入。

## 3. 单向数据流

```text
Host/UI event
  → per-instance bounded queue
  → widget-instance.handle-event(event)
  → SDK handler/reducer
  → ctx.state.update(patch)
  → host-ui.update-state(patch-json)
  → Broker identity/quota check
  → schema validation + atomic apply
  → bounded invoke_from_event_loop
  → Slint binding refresh
```

### 3.1 UI 结构

- View 在构建期编译为 `widget.ftui`，运行期插件不能替换组件类型、创建任意宿主组件或执行 UI
  脚本。
- IR 可以包含 `If`、`ForEach`、State path 绑定和声明式动画；v1 不提供通用算术、字符串或函数
  表达式，派生值由插件计算进 State。
- 事件由 IR 声明稳定名称和 payload schema。宿主只转发已声明事件，未知事件在到达 WASM 前拒绝。
- 插件逻辑只产生 State Patch；不得使用 CSS selector、节点 ID 或 Slint property 名修改 UI。
- IR→Slint 的宿主实现不是公开契约。P0 必须比较两条内部路径：预编译的通用组件 renderer，或从
  已验证 IR 生成仅由宿主控制的 Slint 定义再编译。第二条路径也不得接受/拼接插件 Slint 文本，
  所有值必须通过结构化节点和转义边界生成。未完成 renderer spike 前不冻结 IR 布局与动画细节。

### 3.2 State Patch

- v1 语义采用 JSON Merge Patch；SDK 隐藏序列化细节。
- 宿主先复制并应用 patch，再对完整新 State 做 schema 校验；成功后原子替换，失败时旧 State 不变。
- P0 默认限制：单 patch ≤16 KiB、State ≤64 KiB、嵌套深度 ≤16、每实例 UI 更新 ≤30/s；参考
  时钟只需 1/s。具体常量必须进入 capability/budget registry 并可测试。
- patch 不自动持久化，不包含 secret，不允许 NaN/Infinity，字符串与数组必须受长度限制。
- UI 线程拥塞时可以合并仅含 State 的连续 patch，但不得跨越需要严格顺序的用户事件或生命周期
  事件；合并策略必须可观测。

runtime 中经过验证的完整 State 是 UI 权威副本。Rust/TypeScript SDK 可以保留 typed mirror 方便
业务代码读取，但 `update` 必须事务化：从 mirror 副本计算 patch → host 验证并确认 → SDK 才提交
mirror；host 拒绝时两边都保持旧值。constructor 从 host 接收 canonical initial State，不能各自用默认
值猜测。除创建/重启外，宿主不直接改插件 State；Config、mode、theme 通过独立 event/capability。

## 4. 实例与生命周期

每个 Widget 是独立 actor：同一实例任何时刻最多执行一个插件回调。宿主可以缓存已编译 Component
和只读 UI/assets，但 `Store`、线性内存、State、Config、Storage、Timer、Grant、事件队列与预算必须
按实例隔离。

```text
discover → validate → instantiate → start
                              │
                              ├─ ui event
                              ├─ timer event
                              ├─ mode/config event
                              ├─ suspend/resume
                              ▼
                             stop → drop
```

生命周期规则：

- `constructor(init)` 只建立内存状态，不做 I/O；配置已经宿主 schema 校验。
- `start` 是首次调用宿主能力的入口；失败时实例进入 failed，不显示伪成功 UI。
- `handle-event` 是唯一常规事件入口；WIT event variant 区分 UI、timer、mode、config、suspend、resume。
- `stop` 有短、可取消的清理预算，不保证在进程崩溃或强制终止时运行；持久数据必须在操作时提交。
- callback 超时、fuel 耗尽、memory limit、trap 和队列溢出都转成稳定 runtime error，并记录宿主存活。
- Show/Edit 属宿主权威状态；插件只接收通知，不得请求或覆盖点击穿透与宿主控件。

## 5. State、Config 与 Storage

| 数据 | 所有者 | 持久化 | 更新方式 |
|---|---|---|---|
| `Config` | 用户/宿主 | 是 | 设置 UI → schema 校验 → `config-changed` |
| `State` | runtime 权威、SDK typed mirror | 否 | `ctx.state.update` 成功后两边事务提交，只驱动 UI |
| `Storage` | 插件私有 KV | 是 | `ctx.storage`，需要权限、配额与 migration |
| `Secrets` | 宿主 | P0 不提供 | 未来只传 opaque reference，不给明文 |

不得自动持久化整个 State。插件升级需要数据迁移时，只迁移 Storage；Config schema 的兼容与默认值由
包版本和宿主管理。

## 6. 能力与 PermissionBroker

能力分两类，但都经过 Broker：

- **固有实例能力**：`host-ui.update-state`、受限 `host-log`。安装时不弹权限提示，但只能操作当前
  实例，且有固定 schema、脱敏和配额，不能被扩展为原生访问。
- **声明能力**：storage、timer、metrics、theme 等。manifest 声明是授权上限，用户/宿主可继续
  收窄；未声明、未知、超 scope、超 quota、环境不可用均拒绝并审计。

SDK 提供 `ctx.timer()` 等易用表面；CLI 可从静态使用生成权限候选，但生成结果不得自动扩大权限。
manifest 的显式权限声明与最终用户 grant 仍是权威来源。

## 7. 线程与异步

- Slint 主线程：渲染、输入、宿主窗口状态与应用经过验证的 State。
- Tokio/runtime worker：WASM async 调用、Timer、Storage 与其他宿主服务。
- 每实例 bounded queue 串行投递事件；队列默认容量、合并与丢弃策略必须写成测试。
- host import 不得持有 Slint 句柄；`host-ui` 只写 runtime 的 State 模型，再异步投递 UI。
- shutdown 先停止接收新事件，再取消 Timer/能力调用，执行有预算的 `stop`，最后 drop Store。

## 8. Rust 与 TypeScript 执行

### Rust

- 目标为 `wasm32-wasip2`，由 `floatile-sdk` re-export guest bindings 与安全包装。
- 插件作者不手写 WIT、不调用生成模块、不接触 raw handle。
- proc macro/build helper 生成 State/Event schema、UI IR 和 manifest 候选。

### TypeScript

- 对外承诺普通 TypeScript/JavaScript 语义，不引入“看似 TypeScript、实际是另一门语言”的公开子集。
- CLI 管理 TypeScript→Component 的工具链和锁定版本；项目不要求全局安装 wasm-tools 或理解 WIT。
- TypeScript adapter 必须产出同一 world，接受相同 fuel/memory/timeout/queue 限制，不得使用宿主内
  无 Broker 的第二套 JS API。
- 具体 JS runtime/编译后端在实现前另立 ADR；必须以单实例与 10 实例的包大小、冷启动、RSS、CPU、
  异常隔离和三平台构建数据做选择。未通过前 Rust 是参考实现，不能降低 TypeScript 公共语义来
  制造通过。

## 9. 版本轴

| 版本 | 管理内容 | 兼容规则 |
|---|---|---|
| `manifestVersion` | 包元数据与布局 | major/整数不识别即拒绝 |
| `engineApiVersion` | WIT lifecycle 与 host interfaces | major 必须匹配；minor 只增可选能力 |
| `uiApiVersion` | UI IR、组件、State/event schema | 独立于 WIT；同 major 向后兼容 |
| `sdkVersion` | 语言封装与工具 | 可以快于 ABI；不得静默改变 ABI |
| plugin `version` | 插件自身发布版本 | semver；权限增加需重新确认 |

宿主不得用 Slint 版本作为插件兼容字段。CLI 必须能够解释不兼容发生在哪个版本轴，并提供稳定错误码。

## 10. 错误与恢复

跨语言稳定错误类别至少包括：

```text
permission-denied · quota-exceeded · invalid-input · unavailable
timeout · cancelled · queue-full · plugin-trapped · incompatible-api
invalid-ui-state · internal
```

错误同时具有 `code`、安全的人类信息、可选 `path/event/capability`、可选修复建议与不可泄密的
details。自由文本不能作为测试或 Agent 判断依据。插件 trap 只终止对应实例；宿主与其他实例必须
存活。重复崩溃进入隔离/暂停状态，禁止无上限自动重启。

## 11. P0 性能与安全预算

P0 必须给出并测试：WASM memory、fuel、callback timeout、队列长度、State/patch 大小、UI 更新频率、
Canvas 指令/点数、asset 数量/大小、活跃 Timer 和宿主调用速率。默认值由安全文档定义；开发模式
可以显示预算消耗，但不得绕开绝对上限。

恶意 fixture 至少覆盖：非法 State path/type、超大/深层 patch、更新洪泛、事件洪泛、无限循环、
超内存、越权 capability、伪造 instance id、trap 后重复启动和宿主关闭期间调用。

renderer spike 还必须证明：嵌套布局/ForEach/If/动画的可实现性、构建/缓存成本、错误定位、销毁后
资源释放以及恶意 IR 不会生成无限/超大宿主 UI。无法满足时调整 UI IR，不得开放 raw Slint 绕过。

## 12. 明确非目标

P0/MVP 不包含：第三方 `.slint`、HTML/WebView、原生插件、网络/文件/命令能力、插件市场、签名与
自动更新、跨插件通信、secret 明文、自定义渲染后端。任何一项进入范围都需要同步需求、安全文档
和 ADR。
