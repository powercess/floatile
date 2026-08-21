//! Floatile CLI 二进制入口：`new` / `validate` / `build` 子命令。

use std::path::PathBuf;
use std::process::ExitCode;

use floatile_cli::{build, dev, install, package, project};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: floatile <new|validate|build|install> [参数]");
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "new" => cmd_new(&args[2..]),
        "validate" => cmd_validate(&args[2..]),
        "build" => cmd_build(&args[2..]),
        "install" => cmd_install(&args[2..]),
        "dev" => cmd_dev(&args[2..]),
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
            println!(
                "注意: floatile-sdk 尚未发布（许可 ADR 未通过），模板需在 SDK 可用后才能独立构建；workspace 内成员可直接构建。"
            );
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

fn cmd_dev(args: &[String]) -> ExitCode {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let json = args.iter().any(|a| a == "--json");
    let interval: u64 = args
        .windows(2)
        .find(|w| w[0] == "--interval")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(500);
    if let Err(e) = dev::ensure_project(&dir) {
        eprintln!("dev 失败: {e}");
        return ExitCode::FAILURE;
    }
    let out = dir.join("out").join("plugin.floatile");
    dev::dev_loop(&dir, &out, interval, json);
}

fn cmd_build(args: &[String]) -> ExitCode {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let out = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("out").join("plugin.floatile"));
    match build::build_project(&dir, &out) {
        Ok(manifest) => {
            println!("已构建 {} (id={})", out.display(), manifest.id.0);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("build 失败: code={} detail={e}", e.code());
            ExitCode::FAILURE
        }
    }
}

fn cmd_install(args: &[String]) -> ExitCode {
    let Some(path) = args.first().map(PathBuf::from) else {
        eprintln!("用法: floatile install <pkg.floatile> [--store PATH] [--json]");
        return ExitCode::from(2);
    };
    let store = match opts_store(args) {
        Ok(store) => store,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    let json = args.iter().any(|a| a == "--json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("读取失败: {e}");
            return ExitCode::FAILURE;
        }
    };
    let source = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin.floatile".to_owned());
    match install::install_package(&bytes, &store, &source, &package::PackageLimits::default()) {
        Ok(installed) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "installed",
                        "id": installed.manifest.id.0,
                        "version": installed.meta.version,
                        "dir": installed.dir.display().to_string(),
                        "digest": installed.meta.digest,
                    })
                );
            } else {
                println!(
                    "已安装 {} {} -> {} (digest {})",
                    installed.manifest.id.0,
                    installed.meta.version,
                    installed.dir.display(),
                    &installed.meta.digest[..12],
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("install 失败: code={} detail={e}", e.code());
            ExitCode::FAILURE
        }
    }
}

/// 解析插件存储根目录：优先 `--store PATH`，其次 `$FLOATTILE_PLUGIN_DIR`。
fn opts_store(args: &[String]) -> Result<PathBuf, String> {
    if let Some(idx) = args.iter().position(|a| a == "--store") {
        return args
            .get(idx + 1)
            .map(PathBuf::from)
            .ok_or_else(|| "--store 缺少路径".to_owned());
    }
    if let Some(dir) = std::env::var_os("FLOATTILE_PLUGIN_DIR").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Err("未指定插件存储：请用 `--store PATH` 或设置环境变量 FLOATTILE_PLUGIN_DIR".to_owned())
}
