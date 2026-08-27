//! Floatile 宿主侧 WIT 绑定（`floatile-plugin-api`）。
//!
//! 与 `floatile-sdk` 的 guest 绑定同源（`wit/floatile-widget.wit`）；CI 校验
//! 二者必须来自同一 commit 的 WIT（WIT 单一事实源，见
//! `docs/plugin-sdk/wit-api-v1.md` §6）。
//!
//! `bindgen!` 生成：
//! - `FloatileWidget`：组件加载/实例化入口
//! - `HostLog` / `HostStorage` / `HostTimer` / `HostOperation` / `HostMetrics` / `HostTheme`：
//!   宿主能力 trait，由 `floatile-runtime` 经 PermissionBroker 实现

wasmtime::component::bindgen!({
    world: "floatile-widget",
    path: "../../wit/floatile-widget.wit",
    // 契约要求宿主能力异步执行（wasmtime async_support），见 wit-api-v1.md §1.5。
    imports: { default: async },
    exports: { default: async },
});

/// 宿主要求的引擎 API 版本；与 `floatile-sdk::ENGINE_API_VERSION` 一致，
/// 二者都来自 `wit/floatile-widget.wit` 的 package 版本。宿主加载插件时校验
/// 插件的 `engineApiVersion`：major 不匹配拒绝加载，minor 兼容时按能力降级。
pub const ENGINE_API_VERSION: &str = "1.2.0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_api_version_matches_wit_package() {
        // 该值必须与 wit/floatile-widget.wit 的 package 版本同步；
        // 修改 WIT 时本测试与 SDK 常量一起更新（CI 校验两者一致）。
        assert_eq!(ENGINE_API_VERSION, "1.2.0");
    }
}
