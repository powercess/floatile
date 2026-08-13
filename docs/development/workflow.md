# 开发与验证流程

> 状态：Accepted

## 1. 环境

```bash
rustup show
rustup target add wasm32-wasip2
cargo --version
wasm-tools --version
```

工具链由仓库固定。不要通过本地 `+stable` 绕过，也不要提交仅在未锁定 nightly 上可用的功能。

## 2. 日常循环

分支创建、提交时机、commit message、PR 与合并必须遵循仓库根目录的 `CONTRIBUTING.md`。

1. 核对 `git status`、`git branch --show-current` 和基线：任何 Git 修改操作前不假设共享工作区状态；
   普通任务显式从最新 `dev` 分出（`git checkout -b <name> dev`），hotfix 显式从最新 `main` 分出，并
   确保没有夹带其他协作者的修改或预期外提交。
2. 关联 `product/requirements.md` 的需求 ID 和 P0 验收项。
3. 读取对应事实源，确认 crate 归属、安全和平台影响。
4. 先写失败测试或明确手工复现，再实现最小完整切片。
5. 运行目标 crate 快速检查，再运行 workspace 门禁。
6. 更新文档与实测证据；在交付中列出未运行的平台/测试和原因。

快速检查示例：

```bash
cargo test -p floatile-core --locked
cargo clippy -p floatile-platform --all-targets --locked -- -D warnings
```

默认提交门禁：

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo check -p floatile-sdk --target wasm32-wasip2 --locked
cargo deny --locked check advisories bans sources
```

文本换行与编码门禁：`git ls-files --eol` 必须全部为 `i/lf`，且被追踪文件不得含 UTF-8 BOM
（CI 已执行此检查；本地可用同款命令自查）。CRLF/BOM 提交视为未通过门禁，先执行
`git add --renormalize .` 归一化基线再提交。

`cargo deny --locked check licenses` 在许可 ADR 完成前预期阻断；它是发布门，不得通过忽略 Slint 或
放宽所有 copyleft 许可证来绕过。

## 3. 变更类型的附加验证

| 变更 | 附加验证 |
|---|---|
| WIT/SDK/runtime | 生成物无 diff 或已提交；guest 构建；host/guest 契约与版本兼容测试；`wasm-tools validate` |
| manifest/package | schema 正反例；路径穿越、链接、重复项、zip-bomb 与大小边界测试 |
| Broker/能力 | allow/deny/scope/quota；审计脱敏；恶意插件；宿主存活 |
| store/migration | 空库、旧版本升级、事务失败、重复运行与数据保留 |
| platform/window | 对应 OS 构建；能力 probe；真实显示环境步骤；矩阵回填 |
| UI/性能 | release 构建；首帧、CPU、RSS、帧率采样；环境与原始值 |
| 依赖升级 | `Cargo.lock` diff；cargo-deny；三平台构建；关键 API/行为回归 |

## 4. CI 与证据

CI 负责三 OS 编译/lint/test、wasm guest 检查和依赖门禁；它不能替代透明度、点击穿透、DPI、
热插拔、帧率等真实桌面验收。手工证据写入平台矩阵或验收记录，包含日期、环境、commit、release
构建、步骤、结果和日志/截图位置。

`dev` 是持续集成目标，必须通过基础 workspace、WASI 和依赖门禁；`main` 的 release PR 还必须
满足许可、三平台 release 构建、相关真实平台/性能/安全验收与版本证据。CI 通过不授权跳过
`CONTRIBUTING.md` 规定的 PR、审查、合并或标签流程。

如果某目标环境不可用，结论写 `未验证`；不要复制设计表中的 ✅。允许 P0 因平台限制失败，但必须
记录可复现证据、降级行为和产品影响。

## 5. 变更说明清单

- 关联需求/风险/ADR：
- 范围与非范围：
- 安全、平台、兼容与许可影响：
- 自动化测试命令和结果：
- 手工验证环境和证据：
- 未验证项与后续条件：
