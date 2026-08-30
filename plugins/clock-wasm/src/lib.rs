//! 参考时钟 WASM 插件的 Component 导出外壳。
//!
//! 作者模型、行为和 UI 单一事实源位于 `floatile-clock-model`；本 crate 只负责
//! 生成 WASM `cdylib`，避免宿主 build/dev 图在 Windows 上争用同名 DLL。

#[cfg(target_arch = "wasm32")]
use floatile_sdk::Widget;

#[cfg(target_arch = "wasm32")]
floatile_sdk::impl_export_widget!(floatile_clock_model::Clock);

#[cfg(not(target_arch = "wasm32"))]
pub fn __floatile_ftui_json() -> String {
    floatile_clock_model::ftui_json()
}
