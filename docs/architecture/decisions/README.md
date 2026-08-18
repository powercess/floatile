# Architecture Decision Records

> 状态：Accepted

ADR 记录会长期约束兼容性或迁移成本的决策，例如 UI/runtime/ABI、权限边界、数据库 schema 策略、
Wayland 产品承诺、许可与分发方式。普通实现细节不需要 ADR。

当前决策：

| ADR | 状态 | 结论 |
|---|---|---|
| [ADR-0001](0001-unified-plugin-ui.md) | Accepted | 插件使用统一 Floatile UI IR；Slint 仅为宿主实现，P0/MVP 不接受第三方 `.slint` |

文件名使用 `NNNN-short-title.md`，编号递增。Accepted ADR 不改写结论；需要改变时新增 ADR，并在
旧文件标记 `Superseded by ADR-NNNN`。

模板：

```markdown
# ADR-NNNN: 标题

> 状态：Proposed | Accepted | Rejected | Superseded
> 日期：YYYY-MM-DD
> 决策者：

## 背景与需求

关联需求、约束、风险和已有事实。

## 候选方案

列出实际可行候选及安全、平台、性能、许可、维护影响。

## 决策

给出选择、适用边界和明确不做的内容。

## 后果

说明收益、代价、兼容性、迁移/回退和后续验证。

## 证据

记录原型、benchmark、测试或权威资料链接。
```
