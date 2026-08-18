//! 面向 `wasm32-wasip2` guest 的 Floatile Rust 插件 SDK。
//!
//! P0 S5 引入统一 State/View/Event/Context、UI schema 与 WIT guest 绑定前保持为空，不依赖任何
//! 宿主 crate，也不公开 Slint 或 raw host bindings。
