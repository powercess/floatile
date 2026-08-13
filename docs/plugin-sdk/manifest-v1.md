# manifest.json v1 草案

> 状态：Proposed
> 包扩展名 `.floatile`（本质是 zip，条目结构见 §5）。
> 版本：manifest v1，冻结于 P0 评审通过后；后续变更须 bump `manifestVersion`。

## 1. 示例

```json
{
  "manifestVersion": 1,
  "id": "dev.floatile.clock",
  "name": "World Clock",
  "description": "多时区时钟",
  "version": "0.1.0",
  "publisher": {
    "id": "dev.floatile",
    "name": "Floatile Labs",
    "url": "https://floatile.dev"
  },
  "engineApiVersion": "1.0.0",
  "type": "widget",
  "entrypoints": {
    "ui": "ui/widget.slint",
    "logic": "logic/plugin.wasm"
  },
  "sizes": {
    "default": { "width": 240, "height": 120 },
    "min": { "width": 160, "height": 80 },
    "max": { "width": 800, "height": 600 },
    "resizable": true
  },
  "permissions": [
    { "capability": "storage:read" },
    { "capability": "storage:write" },
    { "capability": "timer:schedule", "maxPerMinute": 60 },
    { "capability": "system:cpu" },
    { "capability": "system:memory" },
    { "capability": "theme:subscribe" },
    { "capability": "notification:show" }
  ],
  "platform": {
    "requirements": {
      "wayland": "layer-shell"
    }
  },
  "config": {
    "schema": "config.schema.json"
  },
  "storage": {
    "migrationVersion": 1
  },
  "signature": {
    "algorithm": "ed25519",
    "publicKeyId": "dev.floatile:2026-01",
    "digest": {
      "algorithm": "sha256",
      "files": {
        "manifest.json": "9a2c...",
        "ui/widget.slint": "7f1e...",
        "logic/plugin.wasm": "b04d..."
      }
    }
  }
}
```

## 2. 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `manifestVersion` | u32 | ✅ | 当前恒为 1；升级时宿主迁移。 |
| `id` | string | ✅ | 反向域名命名空间（`dev.floatile.clock`）；全包唯一、不可变。 |
| `name` / `description` | string | ✅/❌ | 显示名与描述。 |
| `version` | string | ✅ | 语义化版本 `major.minor.patch`（`semver` crate 校验）。 |
| `publisher` | object | ✅ | `id` + `name` + `url`；`id` 与签名公钥绑定。 |
| `engineApiVersion` | string | ✅ | 宿主 Plugin API 兼容版本，语义版本；宿主做 `major` 强制、`minor` 兼容判断。 |
| `type` | string | ✅ | P0/MVP 仅 `"widget"`。V1 扩展 `panel` / `background` / `command`。 |
| `entrypoints.ui` | string | ✅ | `.slint` 文件相对路径。 |
| `entrypoints.logic` | string | ✅ | WASM Component 文件相对路径。 |
| `sizes` | object | ✅ | `default/min/max/resizable`；逻辑像素。 |
| `permissions` | array | ✅ | 权限声明，可为空数组（零权限插件）。元素见 §3。 |
| `platform.requirements` | object | ❌ | 可选能力要求（如 `wayland: layer-shell`）。不满足时提示而非安装失败。 |
| `config.schema` | string | ❌ | `config.schema.json` 相对路径；缺省则无配置面板。 |
| `storage.migrationVersion` | u32 | ❌ | 插件数据迁移版本，宿主据此跑迁移。 |
| `signature` | object | V1+ | P0/MVP 可缺省（dev 模式）；V1 校验。 |

## 3. permissions 元素

```json
{
  "capability": "timer:schedule",
  "params": {
    "maxPerMinute": 60
  }
}
```

- `capability` 必须是注册表内已定义的能力（见 `docs/security/permission-model.md`），未知能力 → 校验失败。
- `params` 为该能力的配额/作用域，如 `network:https://api.example.com/*` 的域名、`file:read:user-selected` 的范围。
- 权限语义为**白名单 + 上限**：声明了才有，且宿主仍可收窄。

## 4. 校验规则（宿主 `PluginManager`）

1. JSON 解析 → schema 校验（`jsonschema`）。
2. `id` 命名空间合法性、`version` 语义版本。
3. `engineApiVersion` 与宿主声明的 `SUPPORTED_ENGINE_API` 匹配（major 必须相等）。
4. `permissions` 全部为已知能力且参数合法。
5. 引用的文件都存在（`ui/*`、`logic/*`、`config.schema.json`）。
6. `sizes`：`default` 在 `min`/`max` 区间内；`resizable=false` 时 `min == max == default`。
7. V1+：`signature` 存在且 digest 与文件一致，公钥受信任。

## 5. 包结构（.floatile zip）

```
manifest.json
ui/widget.slint
logic/plugin.wasm
assets/
config.schema.json
signature.json          # 外部签名文件（V1+；P0/MVP 可选）
```

- zip 用规范化打包（固定条目、无路径穿越、限制总大小与解压后大小比，防 zip-bomb）。
- 路径校验：所有条目相对路径，禁止 `..`、绝对路径、符号链接。

## 6. 与 WIT/API 的关系

- `engineApiVersion` 对应 `wit/` 中 world 的版本号（`floatile:widget@1.0.0`）。
- manifest 只描述「元数据 + 入口」，接口契约完全由 WIT 决定，两者独立演进。

## 7. 决策记录

- 扩展名 `.floatile` 与 zip 内容基于文件头嗅探（`PK\x03\x04`）而非扩展名，避免被改名绕过。
- `publisher.id` 而非 `publisher.name` 作为信任锚点，避免同名冒充。
