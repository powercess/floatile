//! 宿主侧 build 助手：调用插件的 `__floatile_ftui_json()` 输出 `widget.ftui` JSON。
//!
//! 由 `floatile build` 以 `--features build-host` 运行；wasm 编译不包含本二进制
//! （`required-features`）。

fn main() {
    print!("{}", floatile_clock_wasm::__floatile_ftui_json());
}
