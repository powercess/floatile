//! 构建期生成参考时钟的 `widget.ftui`，嵌入宿主二进制。
//!
//! `widget.ftui` 的唯一事实源是作者的 `Widget::view + State::schema/initial`
//! （`floatile_sdk::build::build_ftui`）；这里在编译期把 SDK 生成结果固化到
//! `OUT_DIR/clock_ftui.json`，`main.rs` 用 `include_str!` 嵌入，避免手写第二份
//! JSON 造成 drift。guest SDK 代码只进 build 脚本，不进入宿主运行时二进制。
//!
//! 构建脚本是受信任的构建期代码：失败即整体构建失败，`expect` 是标准失败路径，
//! 不进入生产代码，故豁免 clippy 的 unwrap/expect 提示。

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::env;
use std::path::PathBuf;

fn main() {
    let json = floatile_clock_wasm::__floatile_ftui_json();

    // 参考时钟源变化时需要重新生成，防止嵌入过期 UI IR。
    println!("cargo:rerun-if-changed=../../plugins/clock-wasm/src/lib.rs");
    println!("cargo:rerun-if-changed=../../plugins/clock-wasm/Cargo.toml");

    let out = env::var_os("OUT_DIR").expect("OUT_DIR 应存在");
    let path = PathBuf::from(out).join("clock_ftui.json");
    std::fs::write(&path, json).expect("写入 clock_ftui.json 失败");
}
