# Floatile

Floatile 是一个跨平台桌面浮动组件宿主：它负责透明置顶窗口、画布布局和持久化，并以
WASM Component Model + WIT 承载受权限控制的第三方 Widget 插件。

项目当前处于 **P0 技术可行性验证**。现有代码只完成 S1 的一部分：Rust workspace、
透明无边框时钟窗口和基础平台能力探测；插件运行时、Permission Broker、存储与完整多平台
验证仍待实现。P0 的成功标准是用证据暴露风险，而不是宣称所有平台能力一致。

## 快速开始

需要 `rustup`；仓库会通过 `rust-toolchain.toml` 选择固定工具链。

```bash
rustup show
cargo check --workspace --locked
cargo test --workspace --locked
cargo run -p floatile-shell
```

提交前执行：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

## 工程入口

- [项目需求基线](docs/product/requirements.md)
- [文档索引与治理](docs/README.md)
- [P0 技术设计](docs/architecture/p0-design.md)
- [Workspace 与 crate 边界](docs/architecture/workspace-and-crates.md)
- [技术栈与版本策略](docs/architecture/technology-stack.md)
- [代码规范](docs/development/coding-standards.md)
- [开发与验证流程](docs/development/workflow.md)
- [贡献指南](CONTRIBUTING.md)

## 许可状态

仓库当前为 `PROPRIETARY`，且 Slint 分发许可仍待法务与商业路线决策。在
[许可分析](docs/architecture/licensing.md)完成并形成 ADR 前，不得发布二进制、SDK 或
`.floatile` 包，也不得自行添加开源 `LICENSE`。
