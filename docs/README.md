# Floatile 文档索引与治理

本文是仓库文档入口。文档描述目标与约束，代码和测试提供实现证据；二者冲突时不得静默选择一方，
应在同一变更中消除冲突。

## 事实源

| 领域 | 事实源 | 何时必须更新 |
|---|---|---|
| Git 分支、提交与 PR 协作 | `../CONTRIBUTING.md` | 分支命名、提交时机、message、审查或合并策略变化 |
| 产品范围与需求 | `product/requirements.md` | 目标、非目标、验收映射变化 |
| P0 验收 | `architecture/p0-acceptance.md` | 验收步骤、阈值或结果变化 |
| 总体架构与线程模型 | `architecture/p0-design.md` | 模块、数据流、线程或安全边界变化 |
| crate 依赖边界 | `architecture/workspace-and-crates.md` | crate 职责或依赖方向变化 |
| 技术选型与版本 | `architecture/technology-stack.md` | 引入/替换关键工具或版本策略变化 |
| 不可逆决策 | `architecture/decisions/` | 兼容性、安全、持久化、许可或平台承诺变化 |
| 风险与假设 | `architecture/risks.md` | 发现新风险或获得验证结论 |
| 权限与网络安全 | `security/permission-model.md`、`security/http-broker.md` | 能力、scope、配额、脱敏或网络策略变化 |
| 插件契约 | `plugin-sdk/manifest-v1.md`、`plugin-sdk/wit-api-v1.md` | manifest/WIT/包格式变化 |
| 平台事实 | `platform-matrix/platform-matrix.md` | 获得新的实测证据 |
| 工程规则 | `development/` | 本地流程、CI、代码或测试规范变化 |

## 文档状态

规范文档开头使用以下状态之一：

- `Proposed`：可讨论，不可作为兼容承诺。
- `Accepted`：当前实现应遵循；变更需同步测试，必要时新增 ADR。
- `Implemented`：已有实现与自动化测试，但不代表跨平台实测完成。
- `Validated`：在声明的目标环境按验收步骤获得证据。
- `Deprecated`：保留历史，不再用于新实现，并指向替代文档。

不得用未来目标的勾选框或 ✅ 表示已实测结果。目标值、设计预期和实测值必须分列；平台证据记录
`日期 | OS/版本 | 显示协议/合成器 | GPU/渲染后端 | 构建 | 步骤 | 结果 | 日志`。

## 修改规则

- 需求使用稳定 ID；测试和变更说明引用 ID，不按段落位置引用。
- 规范性语言使用“必须/不得/应/可以”；避免“尽量”“一般”等不可验收表达。
- 相对链接必须从当前文件可解析；重命名时全仓搜索反向引用。
- 架构图只表达一个事实源，正文描述异常与降级路径。
- 新增依赖要写入技术栈；新风险要写入风险清单；改变不可逆约束要新增 ADR。
- 验证结果只追加真实证据，不预填成功；过期数据标明日期和替代记录。
