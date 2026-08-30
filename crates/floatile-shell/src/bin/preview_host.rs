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
    if args
        .first()
        .is_some_and(|argument| argument == "--instance")
    {
        return run_persistent_instance(&args);
    }
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
        .map_err(report_runtime_error)
}

fn run_persistent_instance(
    args: &[std::ffi::OsString],
) -> Result<floatile_shell::preview::PreviewOutcome, (&'static str, &'static str)> {
    if args.len() != 5 {
        return Err(("FRUN_HOST_ARGUMENT", "持久实例宿主参数无效"));
    }
    let plugin_store = PathBuf::from(&args[1]);
    let database = PathBuf::from(&args[2]);
    let instance_id = args[3]
        .to_string_lossy()
        .parse::<u64>()
        .map(InstanceId)
        .map_err(|_| ("FRUN_HOST_ARGUMENT", "实例 ID 无效"))?;
    let duration_ms = args[4]
        .to_string_lossy()
        .parse::<u64>()
        .map_err(|_| ("FRUN_HOST_ARGUMENT", "运行时限无效"))?;
    let store =
        floatile_store::open(&database).map_err(|_| ("FRUN_STORE", "无法打开持久实例数据库"))?;
    let instance = store
        .instances()
        .get(instance_id)
        .map_err(|_| ("FRUN_STORE", "无法读取持久实例"))?
        .ok_or(("FRUN_INSTANCE_MISSING", "持久实例不存在"))?;
    let updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(instance.updated_at());
    store
        .instances()
        .advance_generation(instance_id, updated_at.max(instance.updated_at()))
        .map_err(|_| ("FRUN_GENERATION", "无法推进实例 generation"))?
        .ok_or(("FRUN_GENERATION", "实例 generation 未更新"))?;
    let instance = store
        .instances()
        .get(instance_id)
        .map_err(|_| ("FRUN_STORE", "无法重读持久实例"))?
        .ok_or(("FRUN_INSTANCE_MISSING", "持久实例不存在"))?;
    let runnable = floatile_shell::plugin_manager::load_runnable_instance(&plugin_store, instance)
        .map_err(|_| ("FRUN_LOAD", "实例安装或配置无法通过复验"))?
        .ok_or(("FRUN_INSTALLATION_MISSING", "实例固定的安装不存在"))?;
    floatile_shell::preview::run_preview(
        runnable.plugin,
        runnable.instance,
        Duration::from_millis(duration_ms),
        None,
    )
    .map_err(report_runtime_error)
}

fn report_runtime_error(
    error: floatile_shell::preview::PreviewError,
) -> (&'static str, &'static str) {
    let bounded: String = error.to_string().chars().take(2_048).collect();
    eprintln!("floatile preview diagnostic [{}]: {bounded}", error.code());
    (error.code(), error.public_detail())
}
