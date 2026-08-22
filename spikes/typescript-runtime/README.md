# TypeScript runtime spike

这是 S5d 的运行时选型证据，不是已发布的 `@floatile/sdk`。它只隔离验证
“普通 TypeScript → 同一 WIT world → Floatile Wasmtime/Permission Broker”链路；UI
暂时直接复用 Rust 参考时钟由 `floatile-ui-schema`/Rust SDK 生成的 `widget.ftui`，避免在
公共 TypeScript UI codegen 落地前手写第二套组件语义。

固定工具链：Node.js ≥20、pnpm 11.3.0、`@bytecodealliance/jco` 1.31.0、
ComponentizeJS/StarlingMonkey 后端、TypeScript 7.0.2。构建关闭 ComponentizeJS 的全部
WASI feature；生成组件只能导入 `floatile:widget/*`，仍由 Permission Broker 仲裁。
Weval/AOT 已被 ADR-0003 判定体积和 RSS 更差；其 npm 链还包含未发布修复版的
`decompress` advisory，因此 lock 用本地禁用桩替换 Weval，构建脚本不提供 AOT 路径。

```text
cd spikes/typescript-runtime
pnpm install --frozen-lockfile
pnpm test
```

`pnpm build` 从仓库 `wit/` 生成 TypeScript guest types，严格类型检查并输出到
`target/typescript-runtime-spike/`；`pnpm test` 继续运行宿主行为/Broker 拒绝/包预算证据。
`pnpm measure` 在 release 下串行输出单/10 实例启动、首 tick 与 RSS（CPU 需在外层用
`time -p`/平台 profiler 采样）。QuickJS 对照与资源数据记录在 ADR-0003；结论为 no-go，
因此公共 SDK、TSX View codegen、CLI TypeScript 模板仍未开始。
