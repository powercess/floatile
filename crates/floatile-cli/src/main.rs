//! Floatile CLI 二进制入口。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_cli::{build, dev, install, instance, package, project, test};
use floatile_core::{InstanceConfig, InstanceDesiredState, InstanceId};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("用法: floatile <new|validate|build|install|instance|dev|test|schema> [参数]");
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "new" => cmd_new(&args[2..]),
        "validate" => cmd_validate(&args[2..]),
        "build" => cmd_build(&args[2..]),
        "install" => cmd_install(&args[2..]),
        "instance" => cmd_instance(&args[2..]),
        "dev" => cmd_dev(&args[2..]),
        "test" => cmd_test(&args[2..]),
        "schema" => cmd_schema(&args[2..]),
        other => {
            eprintln!("未知命令: {other}");
            ExitCode::from(2)
        }
    }
}

fn cmd_instance(args: &[String]) -> ExitCode {
    let Some(action) = args.first() else {
        print_instance_usage();
        return ExitCode::from(2);
    };
    let options = match InstanceCliOptions::parse(&args[1..]) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("instance 参数错误: {message}");
            return ExitCode::from(2);
        }
    };
    let database = match options.database() {
        Ok(path) => path,
        Err(message) => {
            return render_instance_result(
                Err(instance::InstanceCommandError::InvalidArguments(message)),
                options.json,
            );
        }
    };
    let result = match action.as_str() {
        "create" => cmd_instance_create(&database, &options),
        "list" if options.positionals.is_empty() => {
            instance::list_instances(&database).map(InstanceOutput::Many)
        }
        "get" => instance_id(&options)
            .and_then(|id| instance::get_instance(&database, id).map(InstanceOutput::One)),
        "configure" => instance_id(&options).and_then(|id| {
            let config = options.config()?;
            let store = options.plugin_store()?;
            instance::configure_instance(&database, &store, id, config, unix_timestamp())
                .map(InstanceOutput::One)
        }),
        "start" => instance_id(&options).and_then(|id| {
            instance::set_instance_desired_state(
                &database,
                id,
                InstanceDesiredState::Running,
                unix_timestamp(),
            )
            .map(InstanceOutput::One)
        }),
        "stop" => instance_id(&options).and_then(|id| {
            instance::set_instance_desired_state(
                &database,
                id,
                InstanceDesiredState::Stopped,
                unix_timestamp(),
            )
            .map(InstanceOutput::One)
        }),
        "delete" => instance_id(&options)
            .and_then(|id| instance::delete_instance(&database, id).map(InstanceOutput::Deleted)),
        _ => {
            print_instance_usage();
            return ExitCode::from(2);
        }
    };
    render_instance_result(result, options.json)
}

fn cmd_instance_create(
    database: &std::path::Path,
    options: &InstanceCliOptions,
) -> Result<InstanceOutput, instance::InstanceCommandError> {
    let Some(plugin_id) = options.positionals.first() else {
        return Err(instance::InstanceCommandError::InvalidArguments(
            "create 缺少 plugin-id".to_owned(),
        ));
    };
    if options.positionals.len() != 1 {
        return Err(instance::InstanceCommandError::InvalidArguments(
            "create 只接受一个 plugin-id".to_owned(),
        ));
    }
    let version = options.version.as_deref().ok_or_else(|| {
        instance::InstanceCommandError::InvalidArguments("create 需要 --version".to_owned())
    })?;
    let store = options.plugin_store()?;
    let config = options.config_or_empty()?;
    let desired = if options.start {
        InstanceDesiredState::Running
    } else {
        InstanceDesiredState::Stopped
    };
    instance::create_instance(
        database,
        &store,
        plugin_id,
        version,
        config,
        desired,
        unix_timestamp(),
    )
    .map(InstanceOutput::One)
}

fn instance_id(options: &InstanceCliOptions) -> Result<InstanceId, instance::InstanceCommandError> {
    if options.positionals.len() != 1 {
        return Err(instance::InstanceCommandError::InvalidArguments(
            "命令需要且只接受一个 instance-id".to_owned(),
        ));
    }
    options.positionals[0]
        .parse::<u64>()
        .map(InstanceId)
        .map_err(|_| {
            instance::InstanceCommandError::InvalidArguments("instance-id 必须是整数".to_owned())
        })
}

enum InstanceOutput {
    One(instance::InstanceView),
    Many(Vec<instance::InstanceView>),
    Deleted(instance::InstanceView),
}

fn render_instance_result(
    result: Result<InstanceOutput, instance::InstanceCommandError>,
    json: bool,
) -> ExitCode {
    match result {
        Ok(output) => {
            if json {
                let value = match &output {
                    InstanceOutput::One(view) => {
                        serde_json::json!({"schemaVersion": 1, "status": "ok", "instance": view})
                    }
                    InstanceOutput::Many(views) => {
                        serde_json::json!({"schemaVersion": 1, "status": "ok", "instances": views})
                    }
                    InstanceOutput::Deleted(view) => {
                        serde_json::json!({"schemaVersion": 1, "status": "deleted", "instance": view})
                    }
                };
                println!("{value}");
            } else {
                match output {
                    InstanceOutput::One(view) => println!(
                        "instance {} {}@{} desired={} generation={}",
                        view.instance_id,
                        view.plugin_id,
                        view.version,
                        view.desired_state.as_str(),
                        view.generation
                    ),
                    InstanceOutput::Many(views) => {
                        for view in views {
                            println!(
                                "{}\t{}@{}\t{}\tgeneration={}",
                                view.instance_id,
                                view.plugin_id,
                                view.version,
                                view.desired_state.as_str(),
                                view.generation
                            );
                        }
                    }
                    InstanceOutput::Deleted(view) => {
                        println!("deleted instance {}", view.instance_id);
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::json!({"schemaVersion": 1, "status": "error", "code": error.code(), "detail": error.to_string()})
                );
            } else {
                eprintln!("instance 失败: code={} detail={error}", error.code());
            }
            if matches!(error, instance::InstanceCommandError::InvalidArguments(_)) {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

#[derive(Default)]
struct InstanceCliOptions {
    positionals: Vec<String>,
    version: Option<String>,
    config: Option<String>,
    config_file: Option<PathBuf>,
    database: Option<PathBuf>,
    store: Option<PathBuf>,
    json: bool,
    start: bool,
}

impl InstanceCliOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut index = 0;
        while index < args.len() {
            match args[index].as_str() {
                "--json" => parsed.json = true,
                "--no-interactive" => {}
                "--start" => parsed.start = true,
                "--version" | "--config" | "--config-file" | "--db" | "--store" => {
                    let name = args[index].as_str();
                    index += 1;
                    let value = args.get(index).ok_or_else(|| format!("{name} 缺少值"))?;
                    if name == "--version" {
                        parsed.version = Some(value.clone());
                    } else if name == "--config" {
                        parsed.config = Some(value.clone());
                    } else if name == "--config-file" {
                        parsed.config_file = Some(PathBuf::from(value));
                    } else if name == "--db" {
                        parsed.database = Some(PathBuf::from(value));
                    } else {
                        parsed.store = Some(PathBuf::from(value));
                    }
                }
                value if value.starts_with('-') => return Err(format!("未知选项 {value}")),
                value => parsed.positionals.push(value.to_owned()),
            }
            index += 1;
        }
        if parsed.config.is_some() && parsed.config_file.is_some() {
            return Err("--config 与 --config-file 不能同时使用".to_owned());
        }
        Ok(parsed)
    }

    fn database(&self) -> Result<PathBuf, String> {
        if let Some(path) = &self.database {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os("FLOATTILE_DB_PATH").filter(|value| !value.is_empty())
        {
            return Ok(PathBuf::from(path));
        }
        floatile_platform::data_dir()
            .map(|path| path.join("layout.db"))
            .map_err(|error| error.to_string())
    }

    fn plugin_store(&self) -> Result<PathBuf, instance::InstanceCommandError> {
        if let Some(path) = &self.store {
            return Ok(path.clone());
        }
        if let Some(path) =
            std::env::var_os("FLOATTILE_PLUGIN_DIR").filter(|value| !value.is_empty())
        {
            return Ok(PathBuf::from(path));
        }
        floatile_platform::data_dir()
            .map(|path| path.join("plugins"))
            .map_err(|error| instance::InstanceCommandError::InvalidArguments(error.to_string()))
    }

    fn config_or_empty(&self) -> Result<InstanceConfig, instance::InstanceCommandError> {
        if self.config.is_none() && self.config_file.is_none() {
            return Ok(InstanceConfig::empty());
        }
        self.config()
    }

    fn config(&self) -> Result<InstanceConfig, instance::InstanceCommandError> {
        let bytes = if let Some(value) = &self.config {
            value.as_bytes().to_vec()
        } else if let Some(path) = &self.config_file {
            std::fs::read(path).map_err(|error| {
                instance::InstanceCommandError::InvalidArguments(format!(
                    "读取配置 {} 失败: {error}",
                    path.display()
                ))
            })?
        } else {
            return Err(instance::InstanceCommandError::InvalidArguments(
                "configure 需要 --config 或 --config-file".to_owned(),
            ));
        };
        let value = serde_json::from_slice(&bytes).map_err(|error| {
            instance::InstanceCommandError::InvalidArguments(format!("配置不是有效 JSON: {error}"))
        })?;
        InstanceConfig::new(value).map_err(Into::into)
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn print_instance_usage() {
    eprintln!(
        "用法:\n  floatile instance create <plugin-id> --version <semver> [--config <JSON>|--config-file <path>] [--start] [--db PATH] [--store PATH] [--json]\n  floatile instance <list|get|start|stop|delete> [instance-id] [--db PATH] [--json]\n  floatile instance configure <instance-id> (--config <JSON>|--config-file <path>) [--db PATH] [--store PATH] [--json]"
    );
}

/// `floatile schema <manifest.schema.json>`：由单一源生成并输出 manifest.json 的
/// 独立 JSON Schema 产物，供外部工具/编辑器校验 manifest，避免手写平行 schema。
fn cmd_schema(args: &[String]) -> ExitCode {
    let Some(path) = args.first().map(PathBuf::from) else {
        eprintln!("用法: floatile schema <manifest.schema.json>");
        return ExitCode::from(2);
    };
    let schema = floatile_core::manifest_json_schema();
    let text = serde_json::to_string_pretty(&schema).unwrap_or_else(|_| "{}".to_owned());
    match std::fs::write(&path, text) {
        Ok(()) => {
            println!("已写出 manifest JSON Schema 到 {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("写入失败: {e}");
            ExitCode::FAILURE
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

fn cmd_test(args: &[String]) -> ExitCode {
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let json = args.iter().any(|a| a == "--json");
    let timeout_ms: u64 = args
        .windows(2)
        .find(|w| w[0] == "--timeout")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(4000);
    let out = dir.join("out").join("plugin.floatile");
    match test::test_project(&dir, &out, Duration::from_millis(timeout_ms)) {
        Ok(status) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&status).unwrap_or_else(|_| r#"{"ok":false}"#.to_owned())
                );
            } else if status.ok {
                println!("test: PASS");
                println!(
                    "  build={} load={} start={} state_updates={} shutdown={}",
                    status.phases.build,
                    status.phases.load,
                    status.phases.start,
                    status.phases.state_updates,
                    status.phases.shutdown
                );
            } else {
                println!("test: FAIL (code={})", status.code);
                println!(
                    "  build={} load={} start={} state_updates={} shutdown={}",
                    status.phases.build,
                    status.phases.load,
                    status.phases.start,
                    status.phases.state_updates,
                    status.phases.shutdown
                );
                eprintln!("detail: {}", status.detail);
            }
            if status.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("test 失败: code={} detail={e}", e.code());
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
