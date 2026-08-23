# componentize-qjs resource method 参数最小复现

该 fixture 隔离复现 `componentize-qjs` 0.4.3 的导出 resource method receiver 错位：
无参数 method 可调用，但 scalar 或 variant 参数都会在进入 JavaScript 前 trap。
它没有 host import，因此与 Floatile Clock 业务逻辑和 Permission Broker 无关。

```text
pnpm repro:quickjs
```

上游 `crates/runtime/src/interpreter.rs` 在 method 分发时用 `pop_value` 读取
receiver，但 canonical 参数栈顺序是 receiver 在首位、普通参数随后。最小候选修复是从
首位移出 receiver，再按原顺序 drain 普通参数。验证候选 CLI 时：

```text
FLOATILE_COMPONENTIZE_QJS_BIN=/absolute/path/to/componentize-qjs \
  pnpm repro:quickjs
```

修复版必须让 `scalar(7)` 与 `handle(tick(9))` 的实际值到达 JavaScript；fixture
分别通过 `get-last()` 断言 `scalar:7` 与 `tick:9`，避免只证明“没有 trap”。

上游基线：[`componentize-qjs` v0.4.3](https://github.com/andreiltd/componentize-qjs/releases/tag/v0.4.3)；
候选修复与回归测试：[`andreiltd/componentize-qjs#76`](https://github.com/andreiltd/componentize-qjs/pull/76)。
