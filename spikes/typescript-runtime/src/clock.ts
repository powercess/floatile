import { now } from "floatile:widget/host-clock@1.2.0";
import { log } from "floatile:widget/host-log@1.2.0";
import { schedule } from "floatile:widget/host-timer@1.2.0";
import { updateState } from "floatile:widget/host-ui@1.2.0";
import type {
  WidgetEvent,
  WidgetInit,
} from "floatile:widget/widget-contract@1.2.0";

class WidgetInstance {
  private readonly initialState: unknown;
  private readonly mode: string | undefined;

  constructor(init: WidgetInit) {
    // constructor 只建立内存状态，不调用任何 host capability。
    this.initialState = JSON.parse(init.initialStateJson) as unknown;
    const config = JSON.parse(init.configJson) as { mode?: unknown };
    this.mode = typeof config.mode === "string" ? config.mode : undefined;
  }

  start(): void {
    if (this.mode === "loop") {
      // 安全向量：验证同一 JS runtime 仍受 Wasmtime fuel/epoch 预算约束。
      while (true) {
        // 保持可观察循环，不能被构建期删除。
        this.initialState;
      }
    }
    try {
      log("info", "clock-typescript started");
    } catch {
      // 与 Rust 参考实现一致：日志/计时器拒绝降级，不终止实例。
    }
    try {
      schedule(1000n);
    } catch {
      // Permission Broker 的 deny 仍由宿主审计。
    }
  }

  handleEvent(event: WidgetEvent): void {
    if (event.tag === "ui" && event.val.name === "start") {
      try {
        updateState(JSON.stringify({ running: true }));
      } catch {
        // State 权威校验在宿主；拒绝不回写本地镜像。
      }
      return;
    }

    if (event.tag === "timer") {
      const seconds = now().unixMillis / 1000n;
      const second = Number(seconds % 60n);
      const minute = Number((seconds / 60n) % 60n);
      const hour = Number((seconds / 3600n) % 24n);
      const pad = (value: number): string => String(value).padStart(2, "0");
      const time = `${pad(hour)}:${pad(minute)}:${pad(second)}`;

      try {
        updateState(JSON.stringify({ time }));
      } catch {
        // 保持与 Rust clock 的 best-effort State Patch 语义一致。
      }
      try {
        log("debug", `tick ${time}`);
      } catch {
        // 日志不影响业务状态。
      }
      try {
        schedule(1000n);
      } catch {
        // 一次性 timer 重新调度失败时停止 tick，但实例存活。
      }
    }
  }

  stop(): void {
    try {
      log("info", "clock-typescript stopped");
    } catch {
      // stop 是有预算的尽力清理通知。
    }
  }
}

// jco 从同一 WIT world 生成/检查导出形状。
export const widgetContract = { WidgetInstance };
