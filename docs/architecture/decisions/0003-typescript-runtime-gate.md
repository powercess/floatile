# ADR-0003：TypeScript runtime 选型门暂不通过

> 状态：Accepted（no-go；未选择生产 runtime）
> 日期：2026-08-23
> 决策者：Floatile 项目

## 背景与需求

S5d、FR-PLUGIN-01/F11、R14/R15 要求 Floatile 提供普通 TypeScript/JavaScript 语义，产出与
Rust SDK 相同的 `floatile-widget` WIT world，并继续受 Wasmtime 的 fuel、epoch、内存上限和
Permission Broker 约束。公共 TypeScript SDK 实现前必须验证：无 Node/DOM/WASI ambient
capability、Rust/TypeScript clock 行为一致、单/10 实例资源、包大小、异常隔离和三平台构建。

本 ADR 只选择 runtime/adapter，不设计公共 TypeScript UI API。spike 的 `widget.ftui` 直接调用
Rust 参考时钟的 `build_ftui` 生成，以免在 UI schema codegen 落地前出现第二套手写组件语义。

## 候选

### A. jco + ComponentizeJS / StarlingMonkey

- `@bytecodealliance/jco` 1.31.0 + ComponentizeJS 0.22.0 + TypeScript 7.0.2。
- 支持普通 TypeScript/JavaScript；`jco guest-types` 从仓库 WIT 生成严格 guest types。
- `jco componentize --disable all` 可移除全部 WASI feature，组件只导入当前 world 的九个
  `floatile:widget/*` host interface。
- 官方 AOT/Weval 模式也纳入对照。

### B. componentize-qjs / QuickJS

- `componentize-qjs` 0.4.3，sync/async 与 opt-size 均实测。
- 普通 JavaScript 语义，组件约 1 MiB；`--stub-wasi` 后同样没有 WASI import。
- 当前版本的导出 resource method 只要带任意参数（scalar 或 variant）都会在进入 JavaScript 前
  trap；无参数 method 可工作。最小复现将原因定位为 method receiver 从 canonical 参数栈末尾而非
  首位取出，故问题在 adapter receiver lowering，而非 Clock 业务逻辑或 Permission Broker。

### C. Javy

Javy 提供 QuickJS core module/动态链接路径，但不是直接面向 Floatile WIT resource world 的
TypeScript Component adapter。补齐 resource、variant、异常与 host import 映射等于维护自有 adapter，
在 B 的现成路径尚未解决前不扩大实现面。

### D. AssemblyScript 或自定义 TypeScript 子集

拒绝。它们不能兑现普通 TypeScript/JavaScript 语义，会把 R14 的资源问题转化为公开语言兼容性问题，
违反 SDK 工具链门。

## Linux 实测

环境：Linux 7.1.8 x86_64（VMware，8 vCPU，AMD Ryzen 9 9950X3D）、Rust 1.97.1、
Wasmtime 47.0.3、Node 26.7.0、pnpm 11.3.0。release 测试串行运行；RSS 由
`floatile-platform::process_metrics` 读取，CPU 为 shell `time -p` 的进程 user/sys 时间。

| 候选 | 契约/安全 | Component | 单实例 | 10 实例 | 结论 |
|---|---|---:|---|---|---|
| StarlingMonkey | 行为、Broker deny、timeout、低内存失败、peer 存活均通过；0 ambient import | 12,653,655 B | startup 1,410 ms；首 tick 2,415 ms；RSS +521,994,240 B；user/sys 8.45/1.31 s | startup 13,834 ms；全部首 tick 14,837 ms；RSS +2,532,593,664 B；user/sys 79.30/22.28 s | 资源门失败 |
| StarlingMonkey + Weval AOT | 0 ambient import | 18,865,625 B | startup 1,975 ms；RSS +587,124,736 B | 未继续 | 超 16 MiB 单条目门且资源更差 |
| QuickJS opt-size | `constructor/start` 可用；`handle-event(variant)` adapter trap | 约 1.04 MiB | 无法完成行为向量 | 未继续 | 契约门失败 |

额外包证据：当前 `floatile-cli::package` 输出 4,104,438 B，能通过 8 MiB archive、16 MiB 单条目与
压缩比门；包体积不是 StarlingMonkey 的否决项，但压缩后的分发体积不能代表运行时 JIT/RSS 成本。

Linux 的 StarlingMonkey 组件已通过：

- UI `start` event、1 Hz timer、HH:MM:SS State Patch，与 Rust clock 相同行为；
- 无 timer grant 时 Broker deny 有审计且实例存活；
- 无限循环被 fuel/epoch 预算终止且同一 Engine peer 存活；低内存限制失败后宿主仍可启动 peer；
- WIT 导入白名单与导出 world 检查。

Windows/macOS 构建未验证。候选已在更早的资源/契约门失败，因此本 ADR 不用“计划中的 CI”冒充
三平台证据。

## 决策

**P0 暂不选择 TypeScript runtime，不发布 `@floatile/sdk`，F11 继续保持未完成。**

1. 不采用 StarlingMonkey：功能与包预算正确，但单/10 实例 RSS 与冷启动成本不符合轻量 Widget
   目标；AOT 不改善结论。
2. 不采用 componentize-qjs 0.4.3：体积合适，但不能执行现有 WIT lifecycle 的核心
   `handle-event(widget-event)`，不得通过修改 WIT 或降低 TypeScript 语义绕过。
3. 保留 `spikes/typescript-runtime`、固定 lock、WIT 生成类型和 ignored host tests，作为后续候选的
   可复现回归门；它不是公共 SDK 或可分发插件。
4. `wit/`、Permission Broker、16 MiB 单实例内存上限与 `.floatile` 安全预算均不因 TypeScript
   候选放宽。

## 重新打开条件

满足任一条件时新建后继 ADR：

- componentize-qjs 修复 resource method + variant lowering，并通过本 spike 的全部行为/安全向量；
- 出现另一个保持普通 TypeScript 语义、可直接生成同一 Component world 的 adapter；
- StarlingMonkey/Wasmtime 路径有可复现的共享编译产物或 runtime 改进，使单/10 实例资源降到项目
  明确冻结的 TypeScript 增量门。

后继选型仍必须补三平台构建、稳定态 CPU 和许可/NOTICE 清单。jco 使用
Apache-2.0 WITH LLVM-exception，ComponentizeJS 项目声明同一许可；生成组件内嵌引擎的完整分发许可
仍属于独立许可 ADR，当前 no-go 不授权发布。另因 Weval 0.4.1 的传递依赖 `decompress 4.2.1`
命中 GHSA-mp2f-45pm-3cg9 / GHSA-h39j-r5qq-r9mm / GHSA-jwp9-9v96-94mx，且 npm registry 尚无
advisory 所称的 4.2.2，spike lock 以本地禁用桩移除 AOT 链；不得在未修复前恢复。

## 后续验证：QuickJS receiver 候选修复

2026-08-23 在 `componentize-qjs` v0.4.3 源码上验证了最小候选修复：为 runtime 参数栈增加
从首位移出值的操作，并在导出 resource method 分发中用它读取 receiver；普通参数继续按原顺序
传给 JavaScript。上游扩展回归测试同时覆盖 `u32` 与 variant 参数，均通过。
修复已提交为 [`andreiltd/componentize-qjs#76`](https://github.com/andreiltd/componentize-qjs/pull/76)。

同一候选 CLI 生成的完整 Floatile Clock 未改 WIT、Broker 或限制，并通过：

- Rust 参考行为、timer deny 审计后实例存活、无限循环 fuel/epoch 隔离、低内存失败后 peer 存活；
- 七个 `floatile:widget/*` import、零 WASI import、16 MiB component 与默认 `.floatile` 包预算；
- component 3,268,502 B，package 855,374 B；
- release 单实例 startup 273 ms、首 tick 1,277 ms、RSS 增量 135,327,744 B；
- release 10 实例 startup 2,401 ms、全部首 tick 3,402 ms、RSS 增量 662,474,752 B。

2026-08-28 又从 PR #76 的固定 head
`7788644697ed08a841ad0910d4e99772c6fe7132` 构建 `componentize-qjs-cli --features opt-size`，并将
spike 同步到当前 `floatile:widget@1.2.0`/`uiApiVersion = 1.6.0`。生成组件精确白名单为九个
`floatile:widget/*@1.2.0` host interface，零 WASI import，并重新通过四个行为/隔离测试和包预算：

- component 1,053,110 B，package 440,106 B；
- release 单实例 startup 227 ms、首 tick 1,229 ms、RSS 增量 90,034,176 B；
- release 10 实例 startup 1,293 ms、全部首 tick 2,308 ms、RSS 增量 313,122,816 B。

随后将 QuickJS 参考时钟改为消费 private `sdk/typescript` 的
`defineWidget`/`createWidgetContract`，不再手写 resource export glue。组件 1,080,365 B、package
453,211 B，六条共享 lifecycle error 向量均由真实 SDK → adapter → Wasmtime 链路识别为 guest
rejection，既有行为、Broker deny、timeout、低内存与 peer 存活测试继续通过。

本次数据来自同一 Linux 测试机的一次串行冷运行，保留为候选比较证据，不冒充稳定态 CPU或
Windows/macOS 证据。固定 SHA 只用于复现未发布修复，不是 Floatile 的生产依赖。

这些数据显著优于本 ADR 的 StarlingMonkey 对照，但尚不改变 no-go 决策：修复未进入上游发布版，
Windows/macOS 构建、稳定态 CPU、许可/NOTICE 仍未完成；QuickJS component 还额外导出 runtime
初始化函数 `init`，虽不增加 host capability，仍需在生产契约门中决定是否允许或由 adapter 隐藏。

## 后果与下一步

- 好处：F11 不会因为“能编译”而被误标完成；R14/R15 的失败有可执行证据，不会以降低安全边界换
  取表面 TypeScript 支持。
- 代价：P0 的 TypeScript clock 与公共 SDK 继续阻塞。
- 本 PR：已将 QuickJS 问题最小化为任意 method 参数的 receiver 错位，形成可直接提交上游的修复与
  回归测试，并把 spike 改为 StarlingMonkey/QuickJS 共用同一宿主行为向量。
- 下一个 PR：在上游合并并发布固定版本后锁定该版本，补 Windows/macOS 构建、稳定态 CPU、
  QuickJS/生成 component 的许可与 NOTICE、额外 `init` export 契约处理；全部通过后新建后继 ADR，
  再决定是否启动公共 TypeScript SDK。
