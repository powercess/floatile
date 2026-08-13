# 许可选项（与 Slint 兼容）与影响

> 状态：Proposed
> 决策：未决。本文只提出选项与影响，供评审与法务确认后再定。
> 本仓库暂不设 LICENSE；`cargo-deny` 的 licenses 检查刻意拒绝 Slint，正式决策前不放行发布。

## 1. 背景

- 宿主应用（floatile-shell 及各 crate）静态链接 Slint（Rust crate）。
- 插件包含两部分：
  - `ui/*.slint` —— Slint 源码文本，宿主运行时解释/编译，**以明文分发**；
  - `logic/plugin.wasm` —— WASM 二进制，与 Slint 无关（只通过 WIT 接口通信）。
- 许可证影响最大的不是宿主二进制，而是「插件 .slint 源 + 生态分发」的传播问题。

## 2. 选项

### 选项 A：Slint GPLv3（开源路线）
- Floatile 全仓以 GPLv3（或 GPLv3+）发布。
- **影响**：
  - 宿主与所有依赖需满足 GPL-3.0 兼容性（wasmtime = Apache-2.0，ok；但需逐项核对）。
  - 插件分发：`.slint` 以源码文本分发，且插件与宿主通过 WIT 紧耦合。**是否构成派生作品存在解释空间**：保守解读是插件开发者的 `.slint` 也须 GPL → 插件生态（闭源插件、付费市场）被摧毁；宽松解读认为 `.slint` 是「被解释的数据」，不必然受 GPL 传染。
  - **结论：需要法务确认；若选择 A，商业插件模式基本不可行。**
- 适合：完全开源、无闭源插件计划、可接受 GPL 传染不确定性。

### 选项 B：Slint 商业许可（付费路线）
- Floatile 向 Slint 购买商业授权 → 宿主可任意授权（Apache-2.0 / MIT / 闭源）。
- **影响**：
  - 有 License 成本（按营收阶梯，见 Slint 当前报价；需与 Slint 确认 2026 现行条款）。
  - 宿主与插件许可完全自主，`.slint` 明文分发不构成问题 → 插件市场、闭源插件、生态均可做。
- 适合：目标做商业插件生态。

### 选项 C：Slint Royalty-Free 许可（免费商用子集）
- Slint 提供 Royalty-Free License（免费，条件如团队规模/营收门槛、内置使用限制等，**具体条款需向 Slint 获取现行版本核对**）。
- **影响**：
  - 若满足条件可零成本商用，但**条款限制可能禁止把 Slint 作为「运行时解释插件 .slint」的分发模式或限制衍生工具链**——这正是 Floatile 的核心用法，必须先确认。
  - 插件生态收益受限程度取决于条款。
- 适合：预算有限但想保留商用；需先过条款核对。

## 3. 对插件生态的关键问题（需在选型前向 Slint 确认）

1. 宿主运行时**解释/编译第三方提供的 .slint 文件**，是否属于条款允许的「使用」范围（尤其选项 C）。
2. 插件作者**不直接链接 Slint crate**，仅提供 `.slint` 源 → 是否因此免受 GPL/条款约束。
3. 若选择 A，`.slint` 明文分发是否构成派生作品。
4. 商业/ Royalty-Free 授权的**分发形式**是否允许「应用内解释外部 .slint」。

## 4. 建议路径（非决策）

1. P0 阶段：无对外分发，许可不影响内部开发 → **暂不设 LICENSE**；`cargo-deny` 的
   advisories/bans/sources 作为持续门禁，licenses 保持发布阻断。
2. P0 评审时同步启动 Slint 许可咨询 + 法务评估（填第 3 节问题）。
3. 决策点设在 **MVP 发布前**（首次对外分发 .floatile 包与安装器时）。

## 5. 其余第三方许可提醒

- wasmtime / wasm-tools：Apache-2.0（ok）。
- wit-bindgen：Apache-2.0（ok）。
- slint 之外需重点核对的传染性许可 crate（若选 GPL 路线）：无已知 GPL 主依赖，但 `cargo-deny` 会逐项扫描。
- 插件 SDK 若随 .floatile 包分发模板工程，SDK 本身许可取决于宿主最终许可。
