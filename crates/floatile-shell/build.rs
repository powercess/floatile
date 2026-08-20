//! 构建期把参考时钟的 `widget.ftui` 经 `floatile-renderer` 生成为宿主控制的
//! Slint 源码(`clock_plugin.slnt`)并输出运行时元数据(`plugin_meta.json`)。
//!
//! 单一事实源仍是作者的 `Widget::view + State::schema/initial`
//! (`floatile_sdk::build::build_ftui`);`renderer` 负责 IR → Slint 的安全映射,
//! 本脚本只做"生成 + 元数据输出",不手写第二份 UI。guest SDK 代码只进 build
//! 脚本,不进入宿主运行时二进制。
//!
//! renderer 生成的 `.slnt` 是宿主控制的"插件内容区组件"(非 Window,遵循
//! renderer 中立原则);由集成测试用 `slint-build` 编译,证明生成输出可被宿主
//! 编译为合法 Slint 组件(arch 路径二的可编译证据)。`plugin_meta.json` 输出
//! binding/event 槽位与 canonical initial State,`main.rs` 运行时据此把权威
//! State 投影到宿主属性、把输入事件转发回 runtime。
//!
//! 构建脚本是受信任的构建期代码:失败即整体构建失败,`expect` 是标准失败路径,
//! 不进入生产代码,故豁免 clippy 的 unwrap/expect 提示。

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::env;
use std::path::PathBuf;

fn main() {
    let ftui_json = floatile_clock_wasm::__floatile_ftui_json();
    let doc: floatile_ui_schema::UiDocument =
        serde_json::from_str(&ftui_json).expect("clock widget.ftui 应为合法 JSON");
    // renderer 内部独立复验预算/结构;失败即构建失败,防止过期/恶意 IR 静默进入宿主。
    let rendered = floatile_renderer::render_component(&doc).expect("clock UI 渲染失败");

    // 参考时钟源变化时需要重新生成,防止嵌入过期 UI IR。
    println!("cargo:rerun-if-changed=../../plugins/clock-wasm/src/lib.rs");
    println!("cargo:rerun-if-changed=../../plugins/clock-wasm/Cargo.toml");
    // renderer 内部逻辑变化也需重新生成。
    println!("cargo:rerun-if-changed=../../crates/floatile-renderer/src/render.rs");

    let out = env::var_os("OUT_DIR").expect("OUT_DIR 应存在");
    let out_dir = PathBuf::from(out);

    let plugin_path = out_dir.join("clock_plugin.slnt");
    std::fs::write(&plugin_path, &rendered.source).expect("写入 clock_plugin.slnt 失败");

    // 用 slint-build 编译生成的插件内容组件:这是 renderer spike 的"可编译"证据,
    // 也是未来 shell renderer 实例化生成组件的前置。生成物是非 Window 内容组件
    // (renderer 中立),slint 1.17 会给出 deprecation 提示;编译结果必须成功。
    #[allow(clippy::let_unit_value, unused_must_use)]
    {
        slint_build::compile_with_config(
            plugin_path,
            slint_build::CompilerConfiguration::new().with_include_paths(vec![out_dir.clone()]),
        )
        .expect("slint-build 编译 clock_plugin.slnt 失败");
    }

    // 输出运行时元数据:binding/event 槽位 + canonical initial State(renderer 已复验)。
    let meta = serde_json::json!({
        "bindings": rendered.bindings,
        "events": rendered.events,
        "initial_state": doc.state.initial,
    });
    std::fs::write(
        out_dir.join("plugin_meta.json"),
        serde_json::to_string(&meta).unwrap(),
    )
    .expect("写入 plugin_meta.json 失败");
}
