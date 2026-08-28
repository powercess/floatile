import hostClock from "floatile:widget/host-clock@1.2.0";
import hostLog from "floatile:widget/host-log@1.2.0";
import hostTimer from "floatile:widget/host-timer@1.2.0";
import hostUi from "floatile:widget/host-ui@1.2.0";

import {
  createWidgetContract,
  defineWidget,
  WidgetError,
} from "../../../sdk/typescript/dist/index.js";

function conformanceError(config, callback) {
  const mode = config?.mode;
  if (mode === `conformance-${callback}-invalid-input`) {
    return WidgetError.invalidInput(`conformance ${callback} invalid input`);
  }
  if (mode === `conformance-${callback}-rejected`) {
    return WidgetError.rejected(`conformance ${callback} rejection`);
  }
  if (mode === `conformance-${callback}-internal`) {
    return WidgetError.internal();
  }
  return undefined;
}

const clock = defineWidget({
  state: { running: false, time: "" },
  view: () => ({ type: "Text" }),

  start(context) {
    const error = conformanceError(context.config, "start");
    if (error) throw error;
    const mode = context.config?.mode;
    if (mode === "loop") {
      while (true) {
        context.config;
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
  },

  event(event, context) {
    const error = conformanceError(context.config, "event");
    if (error) throw error;
    if (event.tag === "ui" && event.val.name === "start") {
      try {
        context.state.update({ running: true });
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
        context.state.update({ time });
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
  },

  stop() {
    try {
      hostLog.log("info", "clock-typescript stopped");
    } catch {
      // stop 是有预算的尽力清理通知。
    }
  },
});

export const widgetContract = createWidgetContract(
  clock,
  { updateState: (patchJson) => hostUi.updateState(patchJson) },
  { decodeEvent: (event) => event },
);
