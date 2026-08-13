# 贡献指南

> 本文件是 Floatile 分支、提交与 PR 协作规则的事实源。无论人工还是 Agent 修改仓库都必须遵循。

## 分支协作规则（必读）

Floatile 当前处于 P0，变更应优先形成小而完整的垂直切片。

- `main` 是正式版本分支，只接收通过发布门禁的 release PR 或紧急 hotfix；`dev` 是主要开发与集成
  分支，必须始终保持可编译且基础 CI 通过。
- 禁止直接在 `main` 或 `dev` 上开发、提交或 push。普通任务分支必须从最新 `dev` 创建，并通过
  PR 合回 `dev`。
- Agent 分支使用 `codex/<topic>`；人工分支使用 `feat/<topic>`、`fix/<topic>`、
  `refactor/<topic>`、`test/<topic>`、`docs/<topic>`、`ci/<topic>` 或 `chore/<topic>`；紧急修复使用
  `hotfix/<topic>` 并从 `main` 创建。
- 一个分支只解决一个需求、缺陷或治理目标。依赖升级必须单独成分支，并保留 `Cargo.lock`。
- 创建或切换分支前必须检查工作区；不得让未提交修改意外跟随分支，也不得覆盖其他协作者的修改。
- 私有、尚未共享的普通任务分支可以基于最新 `dev` rebase；hotfix 则基于最新 `main`。共享分支
  不得改写历史；需要同步基线时使用 merge，或先获得所有协作者明确同意再 rebase。
- 禁止直接 force-push。确需修复个人分支历史时，只能在明确确认无人基于该历史开发后使用
  `--force-with-lease`；Agent 还必须取得用户明确授权。
- 所有合并必须通过 PR。普通任务 PR 合入 `dev` 默认使用 squash merge；合并后删除任务分支，
  `dev` 永久保留。

Agent 未经用户明确授权不得创建或切换分支、stage、commit、push、rebase、merge 或改写历史。
获准操作 Git 时，必须在操作前后检查 `git status` 和 diff，只能处理当前任务文件；不得使用宽泛
暂存把工作区已有修改一起带入提交。

## 集成、发布与 Hotfix

正常开发路径为：

```text
feat/* | fix/* | codex/*
              │ squash PR
              ▼
             dev
              │ release PR + merge commit
              ▼
             main ── tag: vX.Y.Z
```

- `dev` 接收日常功能、修复、测试、文档和依赖更新。实验性工作仍必须放在独立任务分支，不能直接
  破坏 `dev`。
- 准备发布时，先在 `dev` 完成版本、变更记录和发布证据，并通过完整发布门禁；随后创建
  `dev → main` 的 release PR。release PR 不得夹带尚未在 `dev` 审查过的新功能。
- release PR 使用 merge commit，以保留 `dev` 中已经 squash 整理过的功能提交。该 merge commit
  同样必须使用本文件规定的 subject、body、`Refs:`、`Tests:` 和 `Unverified:`，且不得包含
  `Co-authored-by:`。
- release PR 合并后在 `main` 对该提交创建 `vX.Y.Z` 标签。许可 ADR 与发布门未通过时，不得因为
  分支名或标签存在就创建或分发对外产物。
- 紧急修复从最新 `main` 创建 `hotfix/<topic>`，通过 PR 合回 `main` 并按需发布新标签；随后必须
  通过 PR 将 `main` 的修复同步回 `dev`，不得只在两个分支各自手写一遍修复。
- `main` 和 `dev` 都必须配置分支保护：禁止直接 push，要求目标门禁通过并至少完成一次审查。

## 什么时候可以提交

一个 commit 不必完成整个需求，但必须形成独立可审查、可回退的完整步骤，并同时满足：

1. 只有一个明确目的，能够用一句 subject 准确描述。
2. workspace 可以编译，不存在依赖后续 commit 才能恢复的半迁移 API。
3. 受影响 crate 的相关测试通过；格式化通过，且没有临时调试代码或无追踪 TODO。
4. 错误与降级路径已包含在本步骤内，或其边界没有被本步骤改变。
5. WIT、权限、平台、持久化等由 `AGENTS.md` 要求的联动内容已经同步完成。
6. 只包含当前任务的文件和修改，不夹带无关重构、格式化、依赖升级或他人工作。
7. commit body 能如实记录动机、验证结果和未验证项。

形成 commit 前至少运行：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test -p <affected-crate> --locked
```

变更类型要求更高时，按 [开发与验证流程](docs/development/workflow.md)执行附加检查。完整 workspace
门禁必须在 push 或创建/更新 PR 前执行。

出现以下任一情况时必须继续开发，不得提交到共享历史：

- 编译或相关测试失败；
- 需要用 `WIP`、`temporary` 或 `checkpoint` 才能解释当前状态；
- 测试已加入但实现未完成，或 bug 修复尚未包含回归测试；
- WIT host/guest、权限检查/审计、migration/升级测试等联动只完成一部分；
- 当前代码依赖下一次 commit 才能正常构建或运行；
- 尚不能填写真实的 `Tests:` 或 `Unverified:`；
- diff 中仍有无关修改或其他协作者的工作。

为保护本地实验可以使用临时分支；临时 commit 在进入共享历史前必须整理成符合上述条件的提交。

## Commit message 规范

所有 commit message 必须使用以下结构；body 不得省略：

```text
<type>(<scope>): <summary>

<说明为什么需要这项变更、采用什么边界或取舍；不能只重复 subject。>

Refs: <requirement/acceptance/issue/ADR>
Tests: <实际执行的命令和结果>
Unverified: <未验证项或 none>
```

- `type` 只能使用 `feat`、`fix`、`refactor`、`test`、`docs`、`ci`、`build`、`chore`、
  `perf` 或 `revert`。
- `scope` 使用 crate 或稳定领域名，例如 `platform`、`runtime`、`store`、`wit`、`permissions`。
- subject 后必须有一个空行；body 必须至少有一个非空说明段落。
- `Refs:`、`Tests:`、`Unverified:` 必须存在。无需求 ID 或未运行测试时必须写明原因，不得虚构记录。
- commit message 的任何位置都不得包含 `Co-authored-by:`，匹配大小写不敏感。一个 commit 只使用
  Git 的 author/committer 字段表达作者身份，不添加共同作者 trailer。

示例：

```text
feat(platform): expose window degradation reasons

Return explicit capability reasons so the shell can distinguish an
unsupported compositor from an unavailable display environment.

Refs: FR-PROBE-01, F13
Tests: cargo test -p floatile-platform --locked
Unverified: Windows and macOS runtime behavior
```

## 开发与 PR 流程

提交变更前：

1. 从 [项目需求基线](docs/product/requirements.md)确认目标与非目标。
2. 从 [文档索引](docs/README.md)定位该领域的事实源。
3. 按 [开发流程](docs/development/workflow.md)实现、测试并记录证据。
4. 按 [代码规范](docs/development/coding-standards.md)自查 crate 边界、安全与错误处理。

变更说明至少包含：问题与范围、实现取舍、风险、执行过的命令、平台/环境、未验证项。涉及
UI 或平台行为时附截图或日志；涉及性能时给出 release 构建、采样方法和原始数值；涉及安全边界时
必须包含拒绝路径与宿主存活断言。

PR 必须保持单一目标，关联需求/验收项，并确保每个保留在共享历史中的 commit 都满足本文件规范。
普通任务 PR 的目标分支必须是 `dev`；只有 release 或 hotfix PR 可以直接以 `main` 为目标。
依赖升级应说明直接/传递依赖变化，并执行 `cargo deny` 门禁。
许可证仍未决，不接受以“临时”为理由绕过许可约束或创建发布产物。
