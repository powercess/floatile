import hostClock from "floatile:widget/host-clock@1.0.0";
import hostLog from "floatile:widget/host-log@1.0.0";
import hostTimer from "floatile:widget/host-timer@1.0.0";
import hostUi from "floatile:widget/host-ui@1.0.0";

class WidgetInstance {
  constructor(init) {
    // constructor 只建立内存状态，不调用任何 host capability。
    this.initialState = JSON.parse(init.initialStateJson);
    const config = JSON.parse(init.configJson);
    this.mode = typeof config.mode === "string" ? config.mode : undefined;
  }

  start() {
    if (this.mode === "loop") {
      // 安全向量：验证同一 JS runtime 仍受 Wasmtime fuel/epoch 预算约束。
      while (true) {
        this.initialState;
      }
    }
    try {
      hostLog.log("info", "clock-typescript started");
    } catch {
      // 与 Rust 参考实现一致：日志/计时器拒绝降级，不终止实例。
    }
    try {
      // componentize-qjs 0.4.x 将 WIT u64 映射为安全整数范围内的 number。
      hostTimer.schedule(1000);
    } catch {
      // Permission Broker 的 deny 仍由宿主审计。
    }
  }

  handleEvent(event) {
    if (event.tag === "ui" && event.val.name === "start") {
      try {
        hostUi.updateState(JSON.stringify({ running: true }));
      } catch {
        // State 权威校验在宿主；拒绝不回写本地镜像。
      }
      return;
    }

    if (event.tag === "timer") {
      const seconds = Math.floor(hostClock.now().unixMillis / 1000);
      const second = seconds % 60;
      const minute = Math.floor(seconds / 60) % 60;
      const hour = Math.floor(seconds / 3600) % 24;
      const pad = (value) => String(value).padStart(2, "0");
      const time = `${pad(hour)}:${pad(minute)}:${pad(second)}`;

      try {
        hostUi.updateState(JSON.stringify({ time }));
      } catch {
        // 保持与 Rust clock 的 best-effort State Patch 语义一致。
      }
      try {
        hostLog.log("debug", `tick ${time}`);
      } catch {
        // 日志不影响业务状态。
      }
      try {
        hostTimer.schedule(1000);
      } catch {
        // 一次性 timer 重新调度失败时停止 tick，但实例存活。
      }
    }
  }

  stop() {
    try {
      hostLog.log("info", "clock-typescript stopped");
    } catch {
      // stop 是有预算的尽力清理通知。
    }
  }
}

export const widgetContract = { WidgetInstance };
