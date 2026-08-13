# Floatile repository instructions

本文件作用于整个仓库。目标是让每次变更都保持可运行、可验证，并维护插件安全边界和平台差异隔离。

## 分支与提交协作（置顶规则）

以下规则优先于具体任务流程；完整事实源为 `CONTRIBUTING.md`。

- `main` 是正式版本分支，只接收通过发布门禁的 release PR 或紧急 hotfix；`dev` 是主要开发与集成
  分支。禁止直接在 `main` 或 `dev` 上开发、提交或 push。
- 普通任务从最新 `dev` 创建独立分支并通过 PR 合回 `dev`。Agent 分支使用 `agent/<topic>`；人工
  分支使用 `feat/`、`fix/`、`refactor/`、`test/`、`docs/`、`ci/` 或 `chore/` 前缀。
- 一个分支只承载一个需求、修复或治理目标。依赖升级单独成分支；不得夹带无关重构、格式化或
  其他协作者的修改。
- 开始、切换分支、提交和交付前都要检查 `git status` 与相关 diff。发现不属于当前任务的修改或
  并发写入时必须保留并避让，不得覆盖、回滚或带入提交。
- Agent 未经用户明确授权不得创建或切换分支、stage、commit、push、rebase、merge 或改写历史。
  获准提交时只能暂存本任务文件，不得使用宽泛暂存把已有修改一起提交。
- 只有独立可审查、可回退、workspace 可编译且相关测试通过的完整步骤才可提交。半迁移 API、失败
  测试、缺失联动文档/绑定/审计或必须写成 `WIP` 的状态应继续开发，不得进入共享历史。
- 每个 commit message 必须包含 subject、空行和有实际内容的 body，并按 `CONTRIBUTING.md` 记录
  `Refs:`、`Tests:`、`Unverified:`。commit message 的任何位置都不得包含 `Co-authored-by:`；匹配
  大小写不敏感。
- 禁止改写共享分支历史或直接 force-push。任务 PR 合入 `dev` 默认 squash merge；发布通过
  `dev → main` 的 release PR 使用 merge commit，合并后在 `main` 打版本标签；hotfix 从 `main`
  分出、合回 `main` 后必须通过 PR 同步到 `dev`。任务分支合并后删除，`dev` 永久保留。

## 开始任务

1. 先读 `CONTRIBUTING.md` 和 `docs/README.md`，再按任务类型读取其中标出的事实源。
2. 实现变更时使用 `.agents/skills/develop-floatile`；独立验证、审计或验收时使用
   `.agents/skills/verify-floatile`。
3. 检查工作区状态，保留用户已有变更；不要顺手改无关代码。
4. 明确该任务对应的需求、验收项和受影响 crate。没有对应需求时，先更新需求或 ADR。

## 不可破坏的边界

- 插件的宿主能力必须全部经过 `PermissionBroker`；默认无权限，拒绝也必须审计。
- `wit/` 是 host/guest 接口的唯一源。不得手写两套绑定或在 WIT 外暴露宿主句柄。
- OS API、窗口系统分支和平台 `unsafe` 只允许出现在 `floatile-platform`。
- `floatile-core` 保持纯领域模型与纯逻辑，不做 I/O、不依赖 runtime/UI/platform。
- `floatile-sdk` 面向 `wasm32-wasip2` guest，不得依赖任何宿主 crate。
- Slint 主线程不得阻塞 I/O、等待 Tokio 或同步执行不受信任 wasm。
- 不受信任输入包括 manifest、zip 路径、`.slint`、WASM、插件配置和 WIT 参数；校验后再使用。
- 许可 ADR 未通过前不得创建可对外分发的产物或放宽 license gate。

## 变更联动

- 改 WIT：同步 API 文档、`floatile-plugin-api`、`floatile-sdk`、runtime 适配、版本与契约测试。
- 改权限：同步能力注册表、manifest 校验、Broker、审计脱敏和恶意插件测试。
- 改平台能力：先改 platform trait/probe，再改上层降级行为、平台矩阵和平台测试证据。
- 改持久化：新增前向 migration；禁止修改已发布 migration；补升级与回滚失败测试。
- 改 crate 依赖方向、线程模型、安全边界或包格式：同步架构文档；不可逆决策新增 ADR。

## 代码与验证

- 遵循 `docs/development/coding-standards.md`；依赖统一声明在根 `Cargo.toml`。
- 生产代码不使用 `unwrap`/`expect`/`panic!` 处理可恢复错误；测试中的使用必须局部、清晰。
- `unsafe` 必须最小作用域并写 `// SAFETY:`，且只在 `floatile-platform` 中出现。
- 每次交付至少运行与变更相称的检查；默认完整门禁见 `docs/development/workflow.md`。
- 不把“能编译”等同于 P0 验收。平台、性能、安全结论必须附环境与日志/数据，未实测写 `未验证`。
- 不删除失败测试来通过门禁；修复原因或明确记录经批准的例外及到期条件。
