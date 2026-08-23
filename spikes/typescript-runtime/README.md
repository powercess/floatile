# TypeScript runtime spike

这是 S5d 的运行时选型证据，不是已发布的 `@floatile/sdk`。它只隔离验证
“普通 TypeScript → 同一 WIT world → Floatile Wasmtime/Permission Broker”链路；UI
暂时直接复用 Rust 参考时钟由 `floatile-ui-schema`/Rust SDK 生成的 `widget.ftui`，避免在
公共 TypeScript UI codegen 落地前手写第二套组件语义。

固定工具链：Node.js ≥20、pnpm 11.3.0、`@bytecodealliance/jco` 1.31.0、
ComponentizeJS/StarlingMonkey、`componentize-qjs` 0.4.3、TypeScript 7.0.2。两个后端生成的
组件只能导入 `floatile:widget/*`，仍由 Permission Broker 仲裁；StarlingMonkey 构建关闭全部
WASI feature，QuickJS 构建使用 `--stub-wasi --opt-size --sync`。
Weval/AOT 已被 ADR-0003 判定体积和 RSS 更差；其 npm 链还包含未发布修复版的
`decompress` advisory，因此 lock 用本地禁用桩替换 Weval，构建脚本不提供 AOT 路径。

```text
cd spikes/typescript-runtime
pnpm install --frozen-lockfile
pnpm test
```

`pnpm build` 从仓库 `wit/` 生成 TypeScript guest types，严格类型检查并输出到
`target/typescript-runtime-spike/clock-typescript-starlingmonkey.wasm`；`pnpm test` 继续运行宿主
行为/Broker 拒绝/包预算证据。后端对照使用：

```text
pnpm build:starlingmonkey
pnpm test:starlingmonkey
pnpm build:quickjs
pnpm test:quickjs
```

发布版 `componentize-qjs` 0.4.3 的 `test:quickjs` 预期在带参数 resource method 处失败。
若验证上游候选修复，可在构建时设置绝对路径
`FLOATILE_COMPONENTIZE_QJS_BIN=/path/to/componentize-qjs`；输出固定为
`clock-typescript-quickjs.wasm`，同一组 Rust host tests 通过
`FLOATILE_TYPESCRIPT_CLOCK_WASM` 选择组件，不复制行为语义。

`pnpm measure` 在 release 下串行输出单/10 实例启动、首 tick 与 RSS（CPU 需在外层用
`time -p`/平台 profiler 采样）。最小 receiver/参数复现位于
`repro/quickjs-resource-method/`。QuickJS 对照与资源数据记录在 ADR-0003；在修复尚未发布且
许可/三平台门未完成前结论仍为 no-go，因此公共 SDK、TSX View codegen、CLI TypeScript 模板
仍未开始。
