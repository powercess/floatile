# Floatile Rust/TypeScript SDK 与开发者体验

> 状态：Accepted（体验原则）；API 名称在 P0 参考插件通过后冻结
> 范围：标准 Widget 插件、CLI、测试工具与 AI Agent 接口

本文定义插件作者和 AI Agent 应看到的产品表面。内部 runtime、WIT 和 Slint 细节不得泄漏成使用
前置知识。完整安全与数据流见 `plugin-system-architecture.md`。

## 1. 五分钟成功标准

一个首次接触 Floatile、但会 Rust 或 TypeScript 的开发者必须能够只执行：

```text
floatile new clock
cd clock
floatile dev
```

在五分钟内看到可交互预览。默认路径不得要求：手写 manifest/WIT、安装 Wasmtime/wasm-tools、理解
Component Model、配置 Slint、创建数据库或处理宿主线程。

CLI 默认只询问语言；其余采用可解释默认值。所有命令支持 `--no-interactive` 和结构化输出。

## 2. 一个模型，两种语言

两套 SDK 都表达：

```text
Widget<State, Event>
  view(state) -> View
  start(ctx)
  event(event, ctx)
```

### 2.1 TypeScript 目标表面

```tsx
import { Column, Gauge, Text, defineWidget } from "@floatile/sdk";

export default defineWidget({
  state: {
    time: "--:--:--",
    cpu: 0,
  },

  view: (state) => (
    <Column padding={16} gap={8}>
      <Text style="title">{state.time}</Text>
      <Gauge value={state.cpu} />
    </Column>
  ),

  async start(ctx) {
    ctx.timer.every("1s", "refresh");
  },

  async event(event, ctx) {
    if (event.name === "refresh") {
      ctx.state.update({ time: ctx.clock.localTime() });
    }
  },
});
```

JSX 是构建期 UI 描述，不是 React、DOM 或运行时虚拟 DOM。禁止公开 `document`、CSS selector、
浏览器 API 或 React lifecycle，避免借用熟悉语法却制造错误心智模型。

### 2.2 Rust 目标表面

```rust
use floatile_sdk::prelude::*;

#[derive(State)]
struct ClockState {
    time: String,
    cpu: f64,
}

#[widget]
impl Clock {
    fn view(state: &ClockState) -> View {
        column![
            text(&state.time).style("title"),
            gauge(state.cpu),
        ]
        .padding(16)
        .gap(8)
    }

    fn start(ctx: &mut Context<Self>) -> WidgetResult {
        ctx.timer().every("1s", Event::Refresh)
    }

    fn event(event: Event, ctx: &mut Context<Self>) -> WidgetResult {
        if matches!(event, Event::Refresh) {
            let time = ctx.clock().local_time();
            ctx.state().update(|state| state.time = time)?;
        }
        Ok(())
    }
}
```

公开 API 可以随原型调整，但以下语义必须一致：组件名、State 字段、event 名、Context capability、
错误码、默认预算和生命周期顺序。

Rust SDK 的 `Widget::start` 与 `Widget::event` 返回 `WidgetResult`。`WidgetError` 会原样穿过 WIT
`widget-error` 交给宿主，宿主将其分类为 guest 业务拒绝；SDK 不得吞掉错误，也不得把它伪装成 trap、
fuel、超时或内存错误。
插件项目必须优先使用 `floatile_sdk::prelude::*`；crate 根的生成 WIT 模块主要供 adapter 与一致性测试
使用，不作为普通作者需要理解的入口。prelude 的移除或不兼容改名按 SDK major 变更处理。

## 3. 项目模板

### TypeScript

```text
clock/
  floatile.toml          # 作者维护的最小项目配置
  package.json
  src/
    widget.tsx           # State、View、Event
  assets/
  tests/
    widget.test.ts
```

### Rust

```text
clock/
  floatile.toml
  Cargo.toml
  src/
    lib.rs
  assets/
  tests/
    widget.rs
```

构建目录中的 `manifest.json`、`widget.ftui`、WIT bindings 和 adapter 文件是生成物，不要求作者维护，
也不得把生成实现复制进教程。`floatile.toml` 只保留不可从代码推导的产品元数据、尺寸、配置 schema
引用与明确权限上限。

## 4. 标准组件 v1

P0 首批组件限定为：

| 分类 | 组件/能力 |
|---|---|
| 布局 | `Row`、`Column`、`Stack`、`Grid`、`Scroll` |
| 内容 | `Text`、`Icon`、受限 `Image` |
| 交互 | `Button`、`Toggle` |
| 数据 | `Badge`、`Progress`、`Gauge`、`List` |
| 控制 | `If`、`ForEach`、State binding、event |
| 样式 | spacing、size、color token、border、radius、opacity |
| 动画 | 有限的 enter/exit/value transition |
| 扩展 | 受预算约束的 `Canvas`/`Path`（可在 P0 后半加入） |

组件属性由机器可读 schema 定义，Rust/TypeScript 类型和文档从同一源生成。P0 不加入 TextInput、
UI API 1.1 提供 `page_state` 组合 loading/error/empty/content，并为 `Badge`、`Progress` 提供 Rust
builder；boolean/number State 通过类型化 binding slot 投影，不退化为字符串。UI API 1.2 提供
`grid`、`list`、`list_bind`，动态列表只接受具有显式项数预算的字符串数组；UI API 1.3 增加
`sparkline_bind`，用有界数值数组和必填可访问标签表达监控趋势；UI API 1.4 增加 `responsive`，按
宿主窗口逻辑宽度在纵向与横向布局间切换；UI API 1.5 增加 `with_color_token`，插件只能选择
宿主命名 palette；UI API 1.6 要求 `Toggle`、`Progress`、`Gauge` 提供无障碍标签，并提供
`progress_bind_labeled` builder。富文本、WebView、地图、
视频、自定义字体和任意 SVG；出现真实需求后单独评审。

## 5. Context API

```text
ctx.state       固有：当前实例 State Patch
ctx.log         固有：限速、脱敏日志
ctx.timer       权限：timer:schedule
ctx.storage     权限：storage:read/write
ctx.metrics     权限：system:cpu/memory
ctx.theme       权限：theme:subscribe
ctx.clock       纯时间格式化/宿主时区快照；不暴露系统句柄
```

SDK 包装必须保留具体错误；不得把拒绝转换成空值或静默成功。可选能力使用显式 `is_available()` 或
错误分支，示例必须演示降级。

## 6. 权限生成与确认

CLI 可以从已使用的 Context API 生成候选权限：

```text
Detected capabilities:
  timer:schedule
  system:cpu
```

规则：

1. 候选只能帮助生成，不能成为运行时授权事实源。
2. 作者必须把权限与 scope/配额明确写入项目配置；CI 使用无交互模式验证。
3. 代码使用未声明能力时 build/check 失败；声明但未使用时给 warning，避免权限膨胀。
4. 插件升级新增或扩大权限时，宿主必须重新确认；降权不需要确认但需要记录。

## 7. CLI 契约

| 命令 | 最小职责 |
|---|---|
| `floatile new` | Rust/TypeScript 模板、固定工具链、示例测试 |
| `floatile dev` | watch、增量构建、预览、日志、State/event/权限面板 |
| `floatile check` | schema、类型、WIT、权限、预算、包路径和兼容检查 |
| `floatile test` | 无桌面逻辑测试；mock timer/storage/metrics；恶意输入测试辅助 |
| `floatile preview` | 固定 size/DPI/theme/locale 渲染与截图 |
| `floatile build` | 可复现地产生 `.floatile`，默认不签名 |
| `floatile inspect` | 显示 manifest、版本轴、权限、预算、entry digest |
| `floatile install [--require-trusted] [--accept-permissions]` | 开发模式允许显式 unsigned 安装；分发模式强制 publisher trust、签名、撤销、anti-rollback 与升级权限确认 |
| `floatile trust add-key/revoke-key/revoke-publisher/show` | 管理宿主持有的 Ed25519 public key 信任；不得接收或显示 private key |
| `floatile migrate` | SDK/UI/manifest 兼容迁移；默认先 dry-run |
| `floatile conformance` | 校验并输出语言无关的 SDK contract vectors |
| `floatile instance create/list/get/configure/start/stop/delete` | 按精确安装版本管理持久实例与 desired state |
| `floatile instance rollback` | 将 stopped 实例显式重绑到当前信任仍有效的历史 Installation，记录原因且不降低 anti-rollback 水位 |

所有命令必须支持：

```text
--json            NDJSON 或单一 JSON 结果；schema 有版本
--no-interactive  CI/Agent 不等待输入
--deny-warnings   CI 把 warning 提升为失败
```

退出码与诊断 code 稳定；日志文本不是自动化接口。

当前已落地：`new/validate/check/dev/test/preview/build/install/trust/run/inspect/instance`；`test` 用
`floatile-runtime::harness` 对已构建包跑无头生命周期冒烟。`instance` 子命令提供单一
JSON 结果、稳定 `FINSTANCE_*`/`FCAT_*`/`FCONFIG_*` code，支持 `--db`、`--store`、
`--config`/`--config-file`与 `--no-interactive`。`inspect` 在输出前复用完整包安全校验，提供 manifest、
版本轴、权限、归档/解压预算、规范文件 SHA-256 与聚合内容摘要；支持版本化 `--json`、
`--no-interactive`、`--deny-warnings` 和稳定失败 code，JSON 失败诊断不回显输入包路径。
`check` 在自动清理的临时目录中复用正式 build + package validation + inspect 链，返回
`metadata/wasm/ui/manifest/package` 五阶段、warning 数组和 inspection 结果；支持相同的三个自动化选项，
JSON 失败使用稳定 `FBUILD_*`/`FPAK_*`/`FCHECK_*` code 和不含宿主路径/cargo stderr 的有界描述。
`install --require-trusted` 从 `--db`/`FLOATTILE_DB_PATH`/平台数据目录读取宿主 trust，强制 detached
DSSE/Ed25519 验证、publisher/key 撤销和 anti-rollback；默认 `install` 保留 PP-M4 本地开发所需的
`unsigned` 结果并明确输出 trust 状态。受信安装使用 SQLite pending intent 协调 staging/rename/最高版本
水位，下一次受信安装会先复核并恢复中断事务。`trust` 命令只接收 32 字节 public key hex，输出 key id
和状态，不存储或回显 private key。受信升级会比较当前最高安装的 manifest：新增 capability 或扩大
scope/配额返回 `FINST_PERMISSION_CONFIRMATION` 且零落盘；调用方必须显式传
`--accept-permissions` 才能继续。纯移除/收窄无需该参数，成功输出包含逐 capability 的 upgrade diff。
`instance rollback <id> --version <historical> --reason <text>` 仅接受 stopped 实例；目标安装先复核
install digest 与当前 publisher/key trust，且 storage migration 必须相等、旧权限不得重新扩大。实例
重绑定与原因审计原子提交，最高已接受版本/摘要保持不变。
`check` 按 Component Model 实际保留的 Floatile interface/function imports 与 Capability Registry 比对：
使用声明能力但 manifest 未声明时以 `FCHECK_CAPABILITY_MISSING` 失败；已声明但未导入时产生
`FCHECK_CAPABILITY_UNUSED` warning，`--deny-warnings` 可将其提升为失败。固有能力无需写入 manifest；
该静态诊断不做控制流可达性证明，也不替代 Permission Broker 的运行时强制。
`new/build/test/install/check/inspect` 已共用版本化自动化契约：固定
`schemaVersion/status/severity/code/detail/phases/warnings` 基础字段，命令只可向 `detail` 写入有界
脱敏描述；参数错误使用 exit 2，行为失败使用 exit 1，自动化 flags 不会被误解析为项目或包路径。
`preview` 已通过 shell 所属的专用 preview-host 运行正式 renderer、Slint、Wasmtime 与 Broker；CLI 只负责
构建、校验和原子临时安装，不链接宿主 capability 实现。`dev` 在文件签名变化后先准备新预览，只有新
宿主进程成功派生才替换上一代，构建失败时保留旧预览；每次替换递增 generation 并输出版本化事件。
`run` 构建并原子安装项目、创建固定到精确 Installation 的 desired-running 持久实例，再由 shell 宿主
从 SQLite 重读实例、推进 generation、复验 digest/Config 后启动真实窗口。相同 id/version 仅在内容
digest 一致时复用安装，不同内容必须提升版本；重复 `run` 创建彼此隔离的新实例。
`test` 支持注入一个有界 UI event/payload、短时推进和 `--deny-all` Broker 场景；结果报告 event 数、
State 更新数和 deny 审计数。当前 `advance_time` 使用真实 Tokio 短时延，不宣称虚拟时间；timeout、取消
和 operation vectors 继续由 runtime 契约测试覆盖，不在 CLI 复制服务实现。
Rust SDK 包内包含由根 `wit/floatile-widget.wit` 机械同步的发行快照，仓库测试要求二者逐字节一致；
干净目录测试从 `floatile-sdk`、`floatile-sdk-macros` 与 `floatile-ui-schema` 的独立 Cargo 包快照解析
模板依赖，不使用仓库内部 path。许可 ADR 通过前这些包只用于仓库内可发布性验证，不授权上传 registry。
`migrate` 仍未实现，属于后续 SDK/API 稳定化工作，不在 PP-M4 Rust 作者闭环范围内。

## 8. 诊断格式

```json
{
  "schemaVersion": 1,
  "severity": "error",
  "code": "FTUI_STATE_TYPE_MISMATCH",
  "message": "state.cpu expects number, got string",
  "file": "src/widget.tsx",
  "line": 18,
  "column": 26,
  "path": "state.cpu",
  "suggestion": "pass a number or change the State schema"
}
```

诊断不得泄露本机绝对敏感路径、secret、storage value 或未脱敏 WIT 参数。Rust 与 TypeScript 的同一
错误使用同一 `code`。

## 9. 测试模型

SDK test harness 使用虚拟时间和内存服务：

```text
WidgetHarness::new/plugin()
  .with_config(...)
  .grant(...)
  .start()
  .emit(...)
  .advance_time(...)
  .assert_state(...)
  .assert_audit(...)
```

TypeScript 提供同名语义。测试默认不启动窗口、不访问真实 SQLite/网络/文件/系统指标。UI golden
测试由 `floatile preview` 在固定 renderer、字体、DPI 和 theme 下输出 screenshot + 可访问 UI tree；
平台窗口行为仍需真实平台验收，不能用快照替代。

跨语言 lifecycle 与错误分类使用仓库根 `conformance/` 的版本化 JSON 向量，格式与覆盖状态见
[`conformance-kit.md`](conformance-kit.md)。Rust/TypeScript 测试不得分别维护同名用例的预期结果。

实现状态（P0）：Rust 侧 `WidgetHarness` 已在 `floatile-runtime::harness` 落地——
`grant/start/emit_ui/wait_for_state(谓词断言)/advance_time/audit/assert_audit`，所有宿主能力仍走生产
deny-by-default Broker；`floatile test` 用它对已构建 `.floatile` 跑无头生命周期冒烟（build→提取→
load/start/State 更新/shutdown + 宿主存活）并输出稳定 JSON。`advance_time` 当前按真实短时延驱动
（guest 计时器经 Broker 落到 tokio 定时器）；P0 用有界真实时间，确定性虚拟时钟留作后续。

## 10. AI Agent 一等支持

每个模板包含简短 `AGENTS.md`，只说明项目文件、允许命令和不可编辑生成物。SDK 发布：

- 组件、属性、事件、capability、错误、CLI 输出的版本化 JSON schema；
- 每个能力一个最小、可编译示例；不提供大而全 demo 作为唯一资料；
- `floatile check --json --no-interactive` 的确定性输出；
- `floatile preview --screenshot <path> --ui-tree <path>`；
- 虚拟时间、固定随机种子和 mock capability；
- 文档代码块 CI 编译，防止 Agent 学到过期 API。

Agent 推荐闭环：

```text
read request → inspect schema/example → edit source
→ floatile check --json → floatile test --json
→ floatile preview --screenshot → floatile build
```

Agent 不应直接修改 WIT、生成 manifest、`widget.ftui` 或 bindings；只有 SDK/引擎开发任务才能改变
这些事实源。

## 11. TypeScript 工具链门

公共 TypeScript SDK 实现前必须用独立 ADR 选择 adapter/runtime，并验证：

- 普通 TypeScript 语义与 async/error 行为；
- 无 Node、DOM、网络、文件等未授权隐式能力；
- 单实例与 10 实例冷启动、RSS、CPU、包大小；
- trap/timeout/内存限制与实例隔离；
- Windows/macOS/Linux 构建；
- 与 Rust SDK 相同的契约测试向量。

不得为了轻量而公开不兼容的“TypeScript 子集”，也不得为了完整 JS 体验绕开 Wasmtime/Broker。

## 12. 完成定义

SDK 只有同时满足以下条件才可标记 Implemented：

1. Rust 与 TypeScript 参考时钟通过同一行为测试；若 TypeScript 尚未实现，文档必须明确标为计划中。
2. 作者无需编辑 WIT、manifest 生成物或 UI IR。
3. `new → dev → test → preview → build` 在记录的平台可复现。
4. permission deny、quota、timeout、trap、invalid patch 有稳定诊断与宿主存活断言。
5. 文档示例、schema、SDK 类型、WIT 和 host adapter 有自动一致性检查。
