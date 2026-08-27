# manifest.json v1 与 `.floatile` 包格式

> 状态：Proposed（核心模型已实现；CLI 包校验与 zip 级安全测试后标记 Implemented）
> 包扩展名：`.floatile`（zip container，必须按文件头与安全规则识别）
> 关联：ADR-0001、FR-PACK-01、FR-PLUGIN-01、F11、F12

`floatile-core` 已实现 manifest 纯模型与校验（字段/版本轴/semver/sizes/entrypoints/permissions）、
capability 参数解析（未知字段拒绝）与包路径规范化规则；`floatile-cli` 已实现 `.floatile` zip 包校验
（预算、路径穿越/碰撞/symlink/zip-bomb、manifest/UI IR/WASM world 校验与正反例 corpus）与
build 打包+自校验。原子安装已实现（`floatile-cli install`：staging/逐文件 fsync/digest/原子
rename 到 `<插件存储>/<id>/<version>/`，写 `install.json`，同版本重复安装拒绝、失败零残留）；
`floatile inspect` 在同一完整校验后输出 manifest、版本轴、权限、预算、规范文件 SHA-256 与聚合摘要，
并提供版本化 JSON 成功/失败契约；
`floatile-core::install` 为 InstallMeta 与内容 digest 的单一事实源，`floatile-shell::plugin_manager`
按 digest 复核后加载已安装包。config.schema 结构/边界校验已落地（声明时引用文件必须是合法、有界、
根为 object 的 JSON Schema，且 `$ref`/`$dynamicRef`/`$recursiveRef` 只能指向当前文档 fragment）。
CLI 创建/配置与 shell 恢复实例时共用求值器；外部引用在安装期和求值前都拒绝，不会为
不受信 schema 触发宿主网络/文件 I/O。独立 manifest JSON Schema 产物已由单一源 serde 模型和
`CAPABILITY_REGISTRY` 生成（permission 的稳定名称与参数 schema 不再手写平行列表）
（`floatile-core::manifest_json_schema` + `floatile schema <out>` 输出 JSON Schema；用 `jsonschema`
自检与 serde 序列化无 drift）。签名仍待后续切片。

manifest 是安装与运行时的显式事实，不是开发者主要编辑界面。Rust/TypeScript 项目使用最小
`floatile.toml`，CLI 结合代码生成的 UI/State/Event schema 和 capability 候选产生 manifest；作者
仍必须显式确认权限、标识、版本、尺寸和配置。

## 1. P0 示例

```json
{
  "manifestVersion": 1,
  "id": "dev.floatile.clock",
  "name": "World Clock",
  "description": "A multi-time-zone clock",
  "version": "0.1.0",
  "publisher": {
    "id": "dev.floatile",
    "name": "Floatile Labs"
  },
  "engineApiVersion": "1.2.0",
  "uiApiVersion": "1.5.0",
  "type": "widget",
  "entrypoints": {
    "ui": "ui/widget.ftui",
    "logic": "logic/plugin.wasm"
  },
  "sizes": {
    "default": { "width": 240, "height": 120 },
    "min": { "width": 160, "height": 80 },
    "max": { "width": 800, "height": 600 },
    "resizable": true
  },
  "permissions": [
    {
      "capability": "timer:schedule",
      "params": { "maxPerMinute": 60, "maxActive": 2 }
    }
  ],
  "config": {
    "schema": "config.schema.json"
  },
  "storage": {
    "migrationVersion": 1
  },
  "build": {
    "sdk": "rust",
    "sdkVersion": "0.1.0"
  }
}
```

`build` 只用于诊断与复现，不能改变授权或运行语义。宿主不能根据 `sdk` 字段给 Rust/TypeScript
不同权限。

## 2. 字段

| 字段 | 必填 | 规则 |
|---|---:|---|
| `manifestVersion` | 是 | v1 恒为 `1`；未知值拒绝 |
| `id` | 是 | 反向域名；稳定、全局唯一、不可随版本改变 |
| `name` / `description` | 是/否 | UTF-8、长度限制；不参与信任 |
| `version` | 是 | 严格 semver |
| `publisher.id/name` | 是 | P0 为元数据；V1 签名后由 id 关联信任锚 |
| `engineApiVersion` | 是 | WIT world 兼容版本 |
| `uiApiVersion` | 是 | `widget.ftui` schema 与组件语义版本 |
| `type` | 是 | P0/MVP 仅 `widget` |
| `entrypoints.ui` | 是 | 规范相对路径，指向 `widget.ftui`，不得是 `.slint` |
| `entrypoints.logic` | 是 | 规范相对路径，指向 WASM Component |
| `sizes` | 是 | 有限正逻辑像素；default 在 min/max 内 |
| `permissions` | 是 | 可以为空；未知 capability 或非法 params 拒绝 |
| `httpTemplates` | 否 | 固定 HTTPS GET 模板；origin/响应/超时预算不得超过 `network:https` |
| `config.schema` | 否 | 包内规范路径；缺省表示无用户配置；只允许当前 schema 文档内 fragment 引用 |
| `storage.migrationVersion` | 否 | 非负整数；只描述插件私有 KV 迁移 |
| `build` | 否 | 诊断元数据，不参与信任/授权 |
| `signature` | P0 否 | 进入分发前另行 ADR 与 schema |

manifest 不重复存放 State/Event schema；它们属于 UI IR 并与组件树同 digest。宿主加载 UI IR 后
必须验证内嵌 schema 与 State bindings/event declarations 一致。

## 3. 开发者项目配置

CLI 可以使用以下最小输入；准确 TOML schema 在 CLI 实现时单独版本化：

```toml
[plugin]
id = "dev.floatile.clock"
name = "World Clock"
version = "0.1.0"

[widget]
default_size = [240, 120]
min_size = [160, 80]
max_size = [800, 600]

[permissions.timer]
max_per_minute = 60
max_active = 2
```

不可从代码安全推导的字段必须显式声明。CLI 检出的 capability 只是候选：代码使用但未声明为
error；声明但未使用为 warning；CLI 不自动扩大权限。

## 4. 权限对象

PP-M5 的 `network:https` 使用精确 HTTPS origin 白名单，并绑定速率、最大响应和超时预算。
`httpTemplates` 固定 URL、credential header（仅 `authorization`/`x-api-key`）、允许状态码及 query
参数名。guest 只能提交模板 ID、获授权的 Connection ID 和已声明 query；secret 不得进入 manifest、
config、State、WIT 参数、SQLite、日志、审计或错误。
模板还可声明 `cacheTtlMs`、`staleIfErrorMs`、`maxRetries`（最多 2）和
`retryBaseDelayMs`；缓存键包含 Connection credential generation，轮换后不会复用旧响应。只有瞬时
transport failure 会重试，权限、模板、响应校验和凭证错误不会重试。

```json
{
  "capability": "storage:write",
  "params": {
    "keys": ["settings.*"],
    "maxBytes": 65536
  }
}
```

- capability 必须来自版本化 registry；字符串前缀相似不能视为兼容。
- params 必须通过该 capability 的专属 schema；未知字段默认拒绝。
- manifest 是安装授权上限；用户 grant 和环境能力只能收窄。
- `host-ui`、`host-log`、`host-clock` 是固定实例 scope 的固有能力，不写入 permissions；它们仍经过
  Broker 的身份、schema、配额与审计路径。

## 5. 包结构

```text
manifest.json
ui/widget.ftui
logic/plugin.wasm
assets/
config.schema.json       # 可选
signature.json           # P0 不使用，未来分发版本可选/必需
```

P0 不允许 `.slint`、原生库、脚本 entrypoint、符号链接或额外可执行文件。即使文件未被 manifest
引用，非法类型/路径/大小也必须在安装前拒绝。

## 6. 安全校验顺序

安装器必须在独立临时目录/流式读取边界内完成，不能先完整解压再验证：

1. 嗅探 zip 文件头与 central directory；扩展名不是信任依据。
2. 限制压缩包大小、条目数、单条目大小、总解压大小与压缩比。
3. 规范化每个 UTF-8 路径；拒绝绝对路径、`..`、`.` 混淆、反斜杠变体、NUL、重复规范路径、
   大小写碰撞、symlink/hardlink/device/特殊文件。
4. 只读取有上限的 manifest；做 JSON schema、未知字段策略、字符串长度和 semver 校验。
5. 校验 engine/UI API major、插件 id/version、sizes、capabilities 与 params。
6. 要求全部 entrypoint/config/asset 引用存在且只引用普通文件；拒绝未允许的可执行条目。
7. 解析并验证 `widget.ftui`：版本、组件 registry、State/Event schema、binding、If/ForEach、Canvas 与
   asset budget。
8. `wasm-tools validate` + Component world/import/export 检查；拒绝未声明 world、ambient WASI、
   native import 与未知 host function。
9. 计算所有允许文件 digest；未来签名在完全相同的规范文件集合上验证。
10. 所有检查通过后原子移动到插件存储；失败清理临时文件并留下脱敏审计。

## 7. UI IR 校验

`widget.ftui` v1 至少包含：

```text
uiApiVersion
root component tree
initial State + JSON schema
event names + payload schemas
State bindings
asset references
limited If/ForEach/animation declarations
```

限制：无脚本、无网络/文件路径求值、无递归组件、无运行时 import、无 Slint/DOM 节点句柄。所有
长度、深度、节点数、binding 数、If/ForEach 展开量、Canvas 指令和 asset 引用有硬上限。

## 8. 版本与升级

- manifest、engine API、UI API、SDK、插件自身版本是独立轴。
- engine/UI major 不兼容直接拒绝；minor 降级必须明确指出缺少的 capability/component。
- 新版本新增或扩大权限时必须重新确认；只降权可以无提示更新但记录审计。
- storage migration 在新版本首次激活前以事务执行；失败保留旧版本与原数据，不运行半升级插件。
- 包格式、安全限制或签名规范的不可逆变化必须新增 ADR。

## 9. CLI 正反例测试

P0 必须覆盖：合法最小包、未知字段/版本/capability、缺失 entrypoint、非 component WASM、world 不
匹配、非法 UI component/binding/state、绝对/穿越/重复/大小写碰撞路径、symlink、zip bomb、超条目/
大小/压缩比、损坏 central directory、未引用可执行条目、config schema 失败与安装原子回滚。

## 10. 非目标

P0 不定义公开签名信任、商店发布、增量更新、网络获取、第三方 `.slint`、原生代码或多插件依赖。
这些能力不能用可选字段偷渡进 v1。
