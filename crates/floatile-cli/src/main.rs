//! Floatile CLI 二进制入口：`new` / `validate` / `build` 子命令。

use std::path::PathBuf;
use std::process::ExitCode;

use floatile_cli::{package, project};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: floatile <new|validate|build> [参数]");
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "new" => cmd_new(&args[2..]),
        "validate" => cmd_validate(&args[2..]),
        "build" => cmd_build(&args[2..]),
        other => {
            eprintln!("未知命令: {other}");
            ExitCode::from(2)
        }
    }
}

fn cmd_new(args: &[String]) -> ExitCode {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let id = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "dev.example.widget".to_owned());
    let name = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "My Widget".to_owned());
    match project::generate_template(&dir, &id, &name) {
        Ok(()) => {
            println!("已生成项目模板于 {}", dir.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("new 失败: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_validate(args: &[String]) -> ExitCode {
    let Some(path) = args.first().map(PathBuf::from) else {
        eprintln!("用法: floatile validate <pkg.floatile>");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读取失败: {e}");
            return ExitCode::FAILURE;
        }
    };
    match package::validate_package(&bytes, &package::PackageLimits::default()) {
        Ok(pkg) => {
            println!(
                "OK {} (id={}, entries={})",
                path.display(),
                pkg.manifest.id.0,
                pkg.entry_names.len()
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("校验失败: code={} detail={e}", e.code());
            ExitCode::FAILURE
        }
    }
}

fn cmd_build(args: &[String]) -> ExitCode {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    // 读取 floatile.toml → 生成 manifest。
    let toml_path = dir.join("floatile.toml");
    let toml_text = match std::fs::read_to_string(&toml_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("读取 {} 失败: {e}", toml_path.display());
            return ExitCode::FAILURE;
        }
    };
    let config = match project::parse_floatile_toml(&toml_text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("解析 floatile.toml 失败: {e}");
            return ExitCode::FAILURE;
        }
    };
    let manifest = match project::generate_manifest(&config) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("生成 manifest 失败: {e}");
            return ExitCode::FAILURE;
        }
    };

    // 读取已构建的 widget.ftui 与 plugin.wasm（由 SDK build 管线产出）。
    let ftui = match std::fs::read_to_string(dir.join("build/widget.ftui")) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("读取 build/widget.ftui 失败（先运行 SDK build）: {e}");
            return ExitCode::FAILURE;
        }
    };
    let wasm = match std::fs::read(dir.join("build/plugin.wasm")) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("读取 build/plugin.wasm 失败: {e}");
            return ExitCode::FAILURE;
        }
    };

    let out = args.get(1).map(PathBuf::from).unwrap_or_else(|| {
        dir.join("out")
            .join(format!("{}.floatile", config.plugin.id))
    });
    match package(&manifest, &ftui, &wasm, &out) {
        Ok(()) => {
            println!("已构建 {} (id={})", out.display(), manifest.id.0);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("打包失败: code={} detail={e}", e.code());
            ExitCode::FAILURE
        }
    }
}
