//! 内部真实预览宿主。作者只调用 `floatile preview`，CLI 通过稳定 JSON 驱动本进程。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use floatile_core::{
    InstallationRef, InstanceConfig, InstanceDesiredState, InstanceId, PluginInstance,
};

fn main() -> ExitCode {
    match run() {
        Ok(outcome) => {
            println!(
                "{}",
                serde_json::to_string(&outcome).unwrap_or_else(|_| {
                    r#"{"running":false,"code":"FPREVIEW_SERIALIZE","detail":"预览结果序列化失败"}"#
                        .to_owned()
                })
            );
            if outcome.running {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err((code, detail)) => {
            println!(
                "{}",
                serde_json::json!({ "running": false, "code": code, "detail": detail })
            );
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<floatile_shell::preview::PreviewOutcome, (&'static str, &'static str)> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err(("FPREVIEW_HOST_ARGUMENT", "预览宿主参数无效"));
    }
    let store = PathBuf::from(&args[0]);
    let plugin_id = args[1].to_string_lossy();
    let duration_ms = args[2]
        .to_string_lossy()
        .parse::<u64>()
        .map_err(|_| ("FPREVIEW_HOST_ARGUMENT", "预览时限无效"))?;
    let plugin = floatile_shell::plugin_manager::load_installed(&store, &plugin_id)
        .map_err(|_| ("FPREVIEW_LOAD", "预览安装无法通过完整性复验"))?
        .ok_or(("FPREVIEW_INSTALLATION_MISSING", "预览安装缺失"))?;
    let reference = InstallationRef::from_install_meta(&plugin.meta)
        .map_err(|_| ("FPREVIEW_INSTANCE", "无法建立预览安装身份"))?;
    let instance = PluginInstance::restore(
        InstanceId(u64::MAX - 1),
        reference,
        InstanceConfig::empty(),
        InstanceDesiredState::Running,
        0,
        0,
        0,
    )
    .map_err(|_| ("FPREVIEW_INSTANCE", "无法创建预览实例"))?;
    floatile_shell::preview::run_preview(plugin, instance, Duration::from_millis(duration_ms), None)
        .map_err(|error| (error.code(), error.public_detail()))
}
