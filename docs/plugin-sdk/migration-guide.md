# Floatile SDK 迁移指南

> 状态：Implemented（Rust SDK 0.1 项目模型；TypeScript adapter 尚未实现）
> 范围：PP-M7、FR-PLUGIN-01、F11、NFR-MAINT-01

本文记录作者可执行的 SDK、UI API、Engine API 与 manifest 迁移。迁移不得改变安装授权、绕过
Permission Broker，或直接编辑生成的 `manifest.json`、`widget.ftui` 和 WIT bindings。

## 自动迁移契约

```text
floatile migrate <project> --json --no-interactive
floatile migrate <project> --write --json --no-interactive
```

默认命令是只读 dry-run，输出 `schemaVersion=1`、目标 SDK 版本及有序 change 列表。只有显式
`--write` 才修改作者文件；重复执行必须返回空 change。CI 应先运行 dry-run，再由作者审查并决定是否
写入。`--deny-warnings` 可与其他作者命令一致使用。

迁移输入视为不可信：`floatile.toml` 必须是小于 256 KiB 的普通 UTF-8 文件并通过当前 parser；缺失、
符号链接、超限或无效 TOML 使用稳定 `FMIGRATE_*` code 拒绝。写入采用同目录 staging 和 backup，
替换失败时恢复原文件，不留下半写 TOML。

## 0.1 隐式 Rust SDK → 显式 SDK

早期项目没有 SDK 段，CLI 兼容解释为 `rust@0.1.0`。迁移后为：

```toml
[sdk]
language = "rust"
version = "0.1.0"
```

该字段生成 manifest 的诊断性 `build.sdk/sdkVersion`，不能改变授权或 runtime 语义。显式 language
是后续 TypeScript 模板复用同一项目 schema 的前置条件，不代表 TypeScript runtime 已通过 ADR 门。

## 版本轴处理

- SDK major：必须使用对应迁移规则和 conformance suite；未知 major 拒绝自动写入。
- Engine API：WIT 是唯一事实源；不得用源码改写绕过 host/guest major 不兼容。
- UI API：组件和 schema 由 `floatile-ui-schema` 单源生成；major 迁移必须提供确定性源码建议。
- manifest：作者迁移 `floatile.toml`，生成物在 `build/check` 时重新生成，禁止原地修补包。
- 权限：迁移只能保留或收窄声明。新增/扩大权限必须由作者编辑并在安装升级时重新确认。

当前只有上述 0.1 项目模型迁移。未来每条规则必须同时增加 dry-run、write、幂等、失败恢复及 CLI JSON
测试，并在本文件记录来源版本、目标版本和不可自动处理项。
