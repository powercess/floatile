# ADR-0004：长任务采用 Broker 化 Operation

> 状态：Implemented（v1.1 首个 `storage:read` typed contract 已落地）
> 日期：2026-08-26
> 决策者：Floatile 项目

## 背景与需求

PP-M2、PP-G3、PP-G5 和 R11 要求网络、同步与其他可能跨越一次插件回调的工作不能阻塞 Slint
主线程或串行实例 actor。长任务必须继续经过 `PermissionBroker`，并具备按实例隔离的队列、并发、
deadline、取消、审计、唯一终态和迟到结果处理。

当前 `engineApiVersion = 1.0` 的 WIT 只有短时 host imports。本文选择后续合约方向，但本次 spike
不修改 `wit/`、host/guest bindings、SDK 或版本号，因而不把 Operation 宣称为插件可用 API。

## 候选方案

### A. 同步 WIT import 等待长任务

拒绝。Wasmtime adapter 虽可异步等待宿主 future，但实例 actor 在 import 返回前仍不能结束当前插件
回调，网络超时会占住回调预算、推迟生命周期事件，也容易把 Tokio 等待错误地传回 UI 路径。

### B. 宿主后台任务 + Operation ID + completion signal

采用。capability-specific submit 在一次 Broker 调用中完成授权与有界入队，并立即返回宿主生成的
`operation-id`。完成信号只带 ID、capability 和稳定终态；结果保留在宿主内，由同一 capability 的
typed `take-result` 一次性领取。取消使用通用 Operation ID，但执行和结果类型仍属于具体 capability。

### C. Component Model 原生 async/future

暂缓。Component Model 已定义 `async func`、`future<T>` 和由 runtime 调度的异步 Canonical ABI，
方向上能减少自定义轮询；但本仓库锁定的 Wasmtime 47.0.3 将 component async 支持标为
“very incomplete”。在取消、资源释放、guest toolchain 与三平台行为没有稳定证据前，不把长期公共
ABI 建立在该实现上。后续可在保持 Broker、owner、预算和审计语义的前提下替换传输机制。

### D. 通用 JSON-RPC / capability bus

拒绝。通用字符串 method 与 JSON payload 会绕过 WIT 类型、capability registry、SDK codegen 和
契约版本检查，也扩大 secret 进入事件、日志或 State 的风险。

## 决策

1. 长任务使用宿主拥有的 `Operation`。Operation owner 固定为
   `plugin-id + instance-id + generation`，ID 由宿主生成，插件不能选择 owner。
2. 所有 submit 必须通过 `PermissionBroker` 的单一授权并执行入口；registry 的原始执行原语不对
   runtime、shell 或 WIT adapter 公开。cancel 与 typed take 也重新经过 capability 授权。
3. 每实例 registry 使用有界提交队列、完成队列、并发许可和 retained-result 上限。deadline 从提交
   时开始并包含排队时间；无效 deadline、满载或服务关闭立即返回稳定错误，已在队列中过期或提交时
   被拒绝的 work 不执行。
4. 每项 Operation 只产生一个 `succeeded | timeout | cancelled | unavailable | internal |
   result-dropped` 终态。成功 payload 不进入通用完成事件；typed result 只能领取一次。
5. runtime completion bridge 只做 owner/generation 比较和非阻塞投递。旧 generation、actor queue 满或
   actor 已关闭时丢弃成功结果，不能让重启后的实例收到旧工作结果。
6. instance stop/delete 或 Broker drop 取消 active operations。权限撤销、Connection 轮换、宿主重启、
   retry/idempotency 和持久 Operation 继续由后续 capability 切片定义，不能从本 spike 推断已完成。
7. 审计只记录 capability、Operation ID、动作、稳定终态/失败码、delivery disposition 和脱敏尺寸
   元数据，不记录请求或结果 payload。当前审计 schema 不新增 completion decision 类型；spike 暂以
   既有 allow/deny 记录终态及 `delivered | stale-generation | queue-full | actor-closed` 处置。

正式 WIT 已按以下类型化形状从唯一源生成全部绑定；v1.1 首个 capability 是 `storage:read`：

```text
host-<capability>.submit(request) -> result<operation-id, error>
host-operation.cancel(operation-id) -> result<_, error>
widget.on-operation-completed(operation-completion)
host-<capability>.take-result(operation-id) -> result<typed-result, error>
```

`operation-completion` 只含 ID、capability 与终态元数据。不得把上述草图复制成第二份 WIT，也不得
通过临时 host function 暴露第二条路径。

## 后果

- 长任务生命周期与 payload 类型解耦，完成队列可以统一处理背压和 generation，而 secret 保持在
  capability service/Broker 边界内。
- typed take 需要每个异步 capability 增加 submit/result adapter 和 contract vectors；这是显式成本，
  但避免通用动态 payload 形成第二套 ABI。
- 进程内 Operation ID、结果与队列不承诺跨宿主重启恢复。需要持久恢复的同步任务必须另立存储、
  幂等和重放设计。
- 将来迁移到 Component Model `future<T>` 时，必须证明取消、deadline、资源释放、Broker 再授权、
  generation 丢弃和双 SDK 行为不退化；迁移需新增 ADR，不静默改变公开语义。

## 证据

本 spike 在 `floatile-core`、`floatile-services` 和 `floatile-runtime` 落地了宿主领域模型、有界 registry、
Broker 入口与 completion bridge。reference fixture 覆盖：成功与 one-shot typed take、拒绝且不执行、
deadline、主动取消、唯一终态、旧 generation、当前 generation、提交过载、retained-result 过载、
actor queue 满/关闭、无效配置/owner/deadline、审计脱敏和拒绝后宿主存活。

验证命令：

```text
cargo test -p floatile-runtime --test operation_spike --locked
cargo test -p floatile-core -p floatile-services --locked
cargo clippy -p floatile-core -p floatile-services -p floatile-runtime --all-targets --locked -- -D warnings
```

权威工具链资料：

- [Component Model async Canonical ABI](https://component-model.bytecodealliance.org/advanced/canonical-abi.html)
- [Component Model async design](https://component-model.bytecodealliance.org/design/async.html)
- [Wasmtime 47 configuration source](https://docs.wasmtime.dev/api/src/wasmtime/config.rs.html)
