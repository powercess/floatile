//! Floatile 不受信任插件的 WASM、实例 actor 与 UI State 运行时。
//!
//! ADR-0001 规定第三方插件提供统一 UI IR 而不是 Slint 源码。P0 S5 实现前保持为空，避免无资源
//! 限制、无 Broker 或可直接操作宿主 UI 的临时执行路径进入宿主。
