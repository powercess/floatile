# Floatile UI IR v1

> 状态：Proposed（renderer spike、schema 实现和正反例通过后冻结）
> 文件：`ui/widget.ftui`
> 版本：`uiApiVersion = 1.0.0`
> 关联：ADR-0001、FR-PLUGIN-01、F11

`floatile-ui-schema` 已实现 IR 类型、组件 registry v1、State/Event JSON Schema 校验、JSONPath 绑定
解析、结构/预算校验与 JSON Merge Patch（`merge_patch` + State/update 预算常量），host+wasm 双编译；
renderer spike（IR→Slint）与正反例向量冻结仍待后续切片。

UI IR 是 Rust/TypeScript View 在构建期产生的、宿主可验证的静态 UI 文档。它不是作者手写格式、
不是 WIT、不是虚拟 DOM、不是 Slint 源码，也不能调用宿主能力。

## 1. v1 原则

1. **静态结构**：组件类型、binding、event 和资源引用在构建期确定；运行期只更新 State。
2. **无脚本**：IR 不含 JS/Rust/Slint、函数体、动态 import、selector 或任意表达式求值。
3. **schema 驱动**：完整 State 与 event payload 都有 schema；host 在 plugin 回调前后验证。
4. **有限执行**：If/ForEach/animation/Canvas 都有显式数量与深度预算，不具有图灵完备能力。
5. **renderer 中立**：公开节点和属性不使用 Slint 类型/名称；宿主可以替换 renderer。
6. **双 SDK 同源**：Rust/TypeScript 生成相同语义文档和 contract vectors。

## 2. 文档形状

P0 可以使用 canonical JSON 编码，后续可换二进制编码，但 v1 语义和错误必须保持。示意：

```json
{
  "uiApiVersion": "1.0.0",
  "state": {
    "initial": {
      "time": "--:--:--",
      "running": false,
      "zones": []
    },
    "schema": {
      "type": "object",
      "additionalProperties": false,
      "required": ["time", "running", "zones"],
      "properties": {
        "time": { "type": "string", "maxLength": 32 },
        "running": { "type": "boolean" },
        "zones": {
          "type": "array",
          "maxItems": 16,
          "items": { "type": "string", "maxLength": 64 }
        }
      }
    }
  },
  "events": {
    "toggle": {
      "payload": { "type": "object", "additionalProperties": false }
    }
  },
  "root": {
    "type": "Column",
    "props": {
      "padding": 16,
      "gap": 8
    },
    "children": [
      {
        "type": "Text",
        "props": {
          "text": { "bind": "$.time" },
          "style": "title"
        }
      },
      {
        "type": "Button",
        "props": { "label": "Start / Stop" },
        "events": {
          "activate": { "emit": "toggle", "payload": {} }
        }
      }
    ]
  }
}
```

真实 schema 必须拒绝未知顶层/节点/属性字段；示例不是放宽策略。

## 3. State

- 根必须是 JSON object；字段路径使用规范 JSONPath 子集：`$.name`、`$.object.name`，不支持脚本、
  filter、递归 descent 或动态 key。
- schema 必须设置 `additionalProperties: false`；所有字符串、数组、对象都有显式上限。
- 初始 State 必须通过 schema，不能含 secret、NaN/Infinity、句柄或二进制大对象。
- 图片/大数据通过受控 asset/model 引用表达，不塞进 State base64。
- State 默认不持久化；Config 和 Storage 遵循插件系统架构。

运行期 `ctx.state.update` 采用 JSON Merge Patch：host 在副本上应用 patch，验证完整新 State 后原子
替换。删除 required 字段、未知字段、类型错误、超大小/深度/频率全部拒绝，旧 State 不变。

host runtime 保存权威 State。constructor 把 canonical initial State 传给 guest；SDK 的 typed mirror
只在 host 接受 patch 后提交本地候选。host 拒绝时 UI State 与 mirror 都保持旧值，测试必须覆盖该
事务性。Config/mode/theme 不直接写 State，而是发送独立 event，由插件决定是否产生 patch。

## 4. Binding

v1 prop value 只有三种来源：

```text
literal                JSON scalar / approved structured value
state binding          { "bind": "$.path" }
local item binding     { "item": "field" }，只在 ForEach template 内
```

SDK 在构建期检查 binding 目标类型与组件 prop 类型。host 仍必须重新验证不可信 IR，不能信任 SDK
标记。v1 不提供通用算术/字符串/函数表达式；格式化、派生值和业务条件由插件计算进 State，避免在
host 建第二个逻辑运行时。

## 5. 条件与列表

### If

```json
{
  "type": "If",
  "when": { "bind": "$.running" },
  "then": { "type": "Text", "props": { "text": "Running" } },
  "else": { "type": "Text", "props": { "text": "Stopped" } }
}
```

`when` v1 只绑定 boolean State，不执行比较表达式。

### ForEach

```json
{
  "type": "ForEach",
  "items": { "bind": "$.zones" },
  "key": "value",
  "template": {
    "type": "Text",
    "props": { "text": { "item": "value" } }
  }
}
```

items schema 必须有 `maxItems`；template 不能递归引用自身。key 必须稳定、唯一、可验证，缺失/重复时
拒绝更新而不是重用错误节点。

## 6. Event

- 只有 component registry 声明的输入事件可绑定，例如 Button `activate`。
- `emit` 必须出现在顶层 events，payload 必须是 literal 与当前 item 的有限结构，不读取任意 State
  路径或宿主数据。
- host 在进入 WASM 前验证 event name/payload schema，并从 runtime context 绑定 instance id；IR/
  guest 不能提供 instance id。
- 高频 pointer/move/scroll event 默认聚合/采样；P0 组件不暴露原始平台事件或按键扫描码。

## 7. Component registry v1

P0 candidate：

```text
Layout:       Row Column Stack Grid Scroll
Content:      Text Icon Image
Interaction:  Button Toggle
Data:         Progress Gauge List
Control:      If ForEach
Extension:    Canvas Path（通过 renderer spike 后再启用）
```

每个组件条目包含稳定名称、introduced minor、props schema、allowed children/slots、events、accessibility
要求和 renderer test vector。新增 optional prop/component 可以 bump minor；删除/改语义必须 bump major。

组件 registry 是单一机器源。README、Rust/TypeScript API 或 Slint adapter 中的手写列表不是事实源。

## 8. Style、theme 与 animation

- v1 使用命名 theme token 与有限结构化 style prop；不接受 CSS、selector、任意 shader 或 renderer
  专有字符串。
- 自定义颜色可以是有限 RGBA literal；系统/主题值使用 token，由 host 根据 theme snapshot 解析。
- animation 只允许 registry 定义的 property、duration、delay、easing 与 enter/exit/value transition；
  duration/并发动画数有上限，后台/suspend 时降频或停止。
- font v1 只使用宿主字体 token；SVG、自定义字体在 R12 退出和专门校验前禁止。

## 9. Assets

- IR 只引用 manifest/package 中规范化的 asset id，不直接包含文件路径、URL 或 data URI。
- installer 验证 MIME/file signature、压缩/解码后尺寸、像素数、帧数、总 bytes 与引用完整性。
- runtime/renderer 不根据扩展名选择 decoder，不允许 asset 触发网络/文件系统读取。
- asset cache 按 plugin version/digest 共享只读数据，不共享可变 instance State。

## 10. P0 初始预算候选

这些值是实现起点，不是已验证结果；evil/clock/10-instance 数据后才能冻结：

| 项 | candidate hard limit |
|---|---:|
| IR 文件 | 256 KiB |
| 节点 | 256 |
| 树深 | 32 |
| binding | 512 |
| event declaration | 128 |
| State | 64 KiB |
| 单 State Patch | 16 KiB |
| State 深度 | 16 |
| UI updates | 30/s/instance |
| ForEach items | 256/instance total |
| asset refs | 64 |
| 单解码位图 | 4096×4096，仍受总像素/bytes 上限 |

不能只把 limit 设大来通过参考插件。超限在分配/渲染前返回稳定错误并记录审计。

## 11. 校验阶段

1. 限字节读取，检查 encoding/version/unknown fields。
2. 验证 State schema 与 initial State 的大小、深度和完整性。
3. 逐节点校验 registry component、prop、child/slot、event 与 accessibility required fields。
4. 静态检查 binding/item 类型、If boolean、ForEach array/max/key/template 与无递归。
5. 校验 event declarations/payload、theme/style/animation 和 assets。
6. 计算 worst-case expanded nodes/bindings/Canvas/asset budget，超限拒绝。
7. renderer dry-build；失败返回节点 path 与稳定 code，不泄漏宿主内部/Slint 实现。

CLI 与 runtime 使用同一验证库和 contract vectors；CLI 通过不能让 runtime 跳过复验。

## 12. Renderer spike

> 状态：路径已选定（2026-08）。本仓库以 host-only `floatile-renderer` crate 实现
> 路径二变体：从已验证 IR 结构化生成为宿主控制的 Slint 源码文本，构建期由
> `slint-build` 编译。参考时钟的生成物可经 `slint-build` 编译为合法 Slint 组件
> （`floatile-shell/build.rs` 承担该可编译证据）。运行时任意 IR 树的即时渲染已按
> ADR-0002 采用 `slint-interpreter` 运行时编译 renderer 生成的源码（`floatile-shell::runtime_ui`：
> 自窗口实例化 + binding 槽位 State 投影 + 事件回投），已落地并测试。

P0 比较：

- 预编译通用 Slint 组件 + host 数据模型；
- 宿主从已验证 IR 生成受控 Slint 定义并缓存编译结果。

后一方案不得拼接插件源文本，所有 token/value 经结构化 encoder；两方案都不能把 Slint 类型/错误变成
公开 UI API。比较嵌套布局/If/ForEach/动画、首帧、缓存、patch、销毁、错误定位和恶意 IR。结果写入
风险 R2，并在冻结 IR 前形成实现记录；失败不能用 raw `.slint` 绕过。

### 12.1 本仓库采取的实现（`floatile-renderer`）

- 输入必须是已通过 `floatile_ui_schema::validate_document` 的 `UiDocument`；renderer
  在生成前再次复验预算/结构（validate 与 renderer 双层防线，任一拒绝都是安全结果）。
- 输出是纯文本：所有字符串字面量经结构化转义，组件名/属性名/回调名由 renderer 生成，
  插件不能定义标识符，杜绝把 IR 原始文本拼进 Slint 语法位置。
- 组件映射（v1 子集）：Column→`VerticalLayout`、Row→`HorizontalLayout`、Stack/Grid→布局容器；
  Text/Button/Toggle/Progress/Gauge→宿主基础元素；If→`if` 结构、ForEach→`for` 循环（模板内 item
  绑定进入独立命名空间）。公共样式 prop（padding/gap/color/opacity 等）映射为受限 Slint 属性。
  Canvas/Path 等未映射组件稳定拒绝（`RNDR_UNSUPPORTED_COMPONENT`）。
- 输出 `component ClockPluginUI`（非 Window 内容组件，遵循 renderer 中立）+ binding/event 槽位：
  binding 槽位（State 路径→生成属性名）驱动运行时把权威 State 按路径写入宿主属性；event 槽位
  （声明事件→生成回调名）供未来 shell renderer 把输入事件转发回 runtime。
- 恶意 IR（超节点/深度/绑定、未知组件、病态字面量）在 renderer 层拒绝并以固定码
  （`RNDR_*`）返回，不泄漏宿主内部。

## 13. Contract tests

- canonical valid clock/system-monitor/countdown fixtures；
- host/CLI/Rust/TS 对相同文档得到相同 pass/fail/code/path；
- unknown version/component/prop/event、类型错、未知 State、递归/过深/超节点、ForEach 超限/重复 key；
- patch 正常/删除 required/未知字段/超大/过深/洪泛/原子回滚；
- asset path/MIME/dimension/bomb；
- renderer create/update/destroy、cache isolation、trap 后 UI 状态和宿主存活。

## 14. 版本演进

- `uiApiVersion` 独立于 WIT/manifest/SDK/plugin version。
- 同 major minor 只能增加 host 支持的 optional component/prop/event；宿主缺少时在安装阶段明确拒绝或
  按 manifest 声明的 fallback 降级，不能运行后静默消失。
- major 迁移由 `floatile migrate` dry-run 生成源码级修改建议；不能只改生成 IR。
- 编码变化可以在 manifest/文件头协商，但不得改变同一版本的语义、错误和预算解释。
