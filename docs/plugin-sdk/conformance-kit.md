# SDK Conformance Kit

> 状态：Implemented（Rust SDK 与 host runtime 自动化验证；TypeScript 尚未实现）
> 范围：PP-M7、FR-PLUGIN-01、F11、NFR-MAINT-01

`conformance/` 保存语言无关、版本化的 JSON 向量。Rust SDK、未来 TypeScript SDK 与宿主 runtime
必须消费同一文件；语言专用测试只能负责把向量映射到本语言 API，不得复制或改写预期语义。

## 生命周期向量

`sdk-lifecycle-v1.json` 固定以下字段：

- `schemaVersion`：向量文件格式；不识别的版本必须拒绝；
- `engineApiVersion`：必须与根 WIT、host bindings 和 guest SDK 一致；
- `id`：跨语言稳定用例名，不得重复；
- `callback`：`start` 或 `event`；
- `guestError`：WIT `widget-error` 的 kebab-case variant；
- `message`：该 variant 携带的稳定测试 payload，`internal` 为 `null`；
- `expectedHostOutcome`：宿主错误分类，不是语言专用异常名。

当前向量覆盖 `start/event × invalid-input/rejected/internal` 的完整组合。runtime 使用真实
WASM Component 执行同一批向量，必须把它们分类为 guest `Rejected`，并证明随后仍可启动和停止同行
实例。trap、fuel、epoch timeout、内存、Broker deny、Operation 和 State Patch 的既有安全测试将在后续
PP-M7 切片逐步登记为同目录的版本化向量。

新增语言 SDK 时，第一步必须解析这些文件并拒绝未知 `schemaVersion`、callback、error 或 outcome；
不能用“等价的本地测试”代替共享向量。

`floatile conformance --json --no-interactive` 会校验 CLI 内嵌的当前 suite 并输出稳定的 schema v1
报告；language adapter、CI 与 Agent 应消费该命令，而不是依赖仓库路径。`--deny-warnings` 与其他作者
命令语义一致；当前 suite 没有 warning。
