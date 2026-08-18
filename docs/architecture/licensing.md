# 许可选项（与 Slint 兼容）与影响

> 状态：Proposed
> 决策：未决；本文不是法律意见
> 发布门：仓库暂不设 LICENSE，`cargo-deny check licenses` 刻意拒绝 Slint，ADR 通过前不得分发

## 1. ADR-0001 后的许可边界

- Floatile 宿主静态链接 Slint，并使用 Slint/winit 渲染内建 UI 与经过验证的 Floatile UI IR。
- P0/MVP 插件包包含 `widget.ftui + plugin.wasm`，不包含 `.slint` 源码，也不直接链接 Slint crate。
- Rust/TypeScript SDK 公开的是 Floatile 组件、State/Event/WIT 契约；Slint 不是插件作者 API。
- 这降低了“第三方插件源码是否受 Slint 许可传播”的不确定性，但不能自动解决宿主二进制、SDK、
  UI IR renderer 或插件生态的许可问题；仍需 Slint 与法务书面确认。

许可影响现在分成三件事：

1. Floatile 宿主如何获得并分发 Slint；
2. Floatile UI IR/SDK 是否可以在目标许可下公开、允许闭源/付费插件；
3. Slint 商业/Royalty-Free 条款是否允许宿主把 Slint 作为第三方插件 UI IR 的通用 renderer。

## 2. 候选路线

### A. Slint GPLv3 + Floatile GPLv3

- 宿主与仓库按 GPLv3 兼容方式分发；全部直接/传递依赖需核对兼容性。
- 插件不携带 Slint 源，也不链接 Slint，较旧 `.slint` 方案减少了直接传播问题；但插件通过 Floatile
  SDK/WIT/UI IR 与 GPL 宿主交互是否允许闭源/付费分发仍需要法务判断，项目不得自行宣称“插件
  一定不受 GPL 影响”。
- 适合完全开源路线；是否支持闭源插件市场是独立法律问题。

### B. Slint 商业许可

- 为宿主取得允许目标分发方式的商业授权，Floatile 仓库/SDK/插件可另行选择许可。
- 必须书面确认授权覆盖：桌面宿主、多个插件实例、把 `widget.ftui` 渲染为 Slint 组件、开发者预览/
  CI renderer、软件 renderer 与未来商业插件生态。
- 适合希望保留闭源宿主或商业插件生态的路线；成本与条款需获取 2026 现行报价/合同。

### C. Slint Royalty-Free 许可

- 仅当项目主体、营收/团队/产品形态与使用方式满足现行条款时可选。
- 核心问题不再是解释第三方 `.slint`，而是“将 Slint 用作外部 `widget.ftui` 的通用宿主 renderer”
  是否属于允许场景，以及 SDK/CLI/preview 的分发是否受限。
- 条款未经书面确认前不能把该路线当成默认免费商用方案。

### D. 更换宿主 UI 技术

- ADR-0001 使插件契约不含 Slint 名称，理论上可以保留 `widget.ftui`/SDK/WIT 并替换 renderer。
- 这降低迁移插件生态的成本，但窗口、组件、动画、性能与三平台能力都需重新验证；不能把“可替换”
  当成当前许可问题已解决。

## 3. 必须向 Slint/法务确认的问题

1. Floatile 宿主把第三方生成的、无 Slint 源码的 `widget.ftui` 映射为 Slint 组件，属于何种授权使用？
2. `floatile dev/preview` 在开发者机器或 CI 中启动同一 renderer，是否需要额外授权或分发条款？
3. 插件只链接 Floatile SDK/WIT、提交 `widget.ftui + wasm` 时，插件作者能否独立选择闭源/商业许可？
4. Floatile UI schema、组件名、SDK 代码和生成 IR 是否包含受 Slint 许可约束的派生材料？
5. GPL、商业和 Royalty-Free 各路线对插件商店、收费插件、企业内部分发和开源贡献的具体影响？
6. 允许的软件/硬件平台、收入/团队阈值、再分发、离线使用、版本升级和终止条款是什么？

答案必须保留可审查的书面证据，并形成独立许可 ADR；不得只引用市场页面摘要或口头理解。

## 4. P0 行动与发布门

1. P0 内部开发可以继续统一 UI/Wasmtime/Broker 可行性验证，但不对外发布二进制、SDK 或插件包。
2. UI IR 与 WIT 避免 Slint 专有名称/类型，保持 renderer 可替换；这是一项维护策略，不是规避许可。
3. 启动 Slint 许可咨询与法务评估，覆盖第 3 节全部问题。
4. MVP 首次公开 SDK、`.floatile` 包、安装器、商店或二进制前必须：
   - 接受许可 ADR；
   - 补仓库/SDK/示例/第三方 notices；
   - 让 `cargo deny --locked check licenses` 按决策通过；
   - 处理 risks.md R12 advisory/第三方资源边界。
5. 禁止通过宽泛 license allow、伪造 proprietary 结论、删除 Slint 依赖链或只发布插件而绕开门禁。

## 5. 其他依赖

- Wasmtime、wasm-tools、wit-bindgen 的具体版本许可在引入/升级时由 cargo-deny 与 notices 核对。
- TypeScript adapter/runtime 尚未选择；ADR 必须包含运行时、stdlib、bundler、npm 包与生成产物许可。
- UI schema/codegen、示例和 SDK 最终采用何种许可必须与插件生态路线一致。
- 每次关键依赖升级检查 license expression、feature、source、传递依赖和三平台产物，不继承旧结论。
