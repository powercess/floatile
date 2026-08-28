//! Floatile CLI 二进制入口。

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use floatile_cli::{
    CommandErrorReport, build, check, conformance, dev, inspect, install, instance, package,
    preview, project, run, test, trust,
};
use floatile_core::{InstanceConfig, InstanceDesiredState, InstanceId, PermissionChangeKind};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "用法: floatile <new|validate|check|inspect|build|install|trust|instance|dev|test|preview|run|schema|conformance> [参数]"
        );
        return ExitCode::from(2);
    }
    match args[1].as_str() {
        "new" => cmd_new(&args[2..]),
        "validate" => cmd_validate(&args[2..]),
        "check" => cmd_check(&args[2..]),
        "inspect" => cmd_inspect(&args[2..]),
        "build" => cmd_build(&args[2..]),
        "install" => cmd_install(&args[2..]),
        "trust" => cmd_trust(&args[2..]),
        "instance" => cmd_instance(&args[2..]),
        "dev" => cmd_dev(&args[2..]),
        "test" => cmd_test(&args[2..]),
        "preview" => cmd_preview(&args[2..]),
        "run" => cmd_run(&args[2..]),
        "schema" => cmd_schema(&args[2..]),
        "conformance" => cmd_conformance(&args[2..]),
        other => {
            eprintln!("未知命令: {other}");
            ExitCode::from(2)
        }
    }
}

fn cmd_trust(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let positionals = match author_positionals(args, &["--db"], &[], 3) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FTRUST_ARGUMENT", &detail, json, true),
    };
    let database = match opts_database(args) {
        Ok(database) => database,
        Err(detail) => return render_basic_error("FTRUST_ARGUMENT", &detail, json, true),
    };
    let result = match positionals.as_slice() {
        ["show", publisher] => trust::show(&database, publisher),
        ["add-key", publisher, public_key] => {
            trust::add_key(&database, publisher, public_key, unix_timestamp())
        }
        ["revoke-key", publisher, public_key] => {
            trust::revoke_key(&database, publisher, public_key, unix_timestamp())
        }
        ["revoke-publisher", publisher] => {
            trust::revoke_publisher(&database, publisher, unix_timestamp())
        }
        _ => {
            return render_basic_error(
                "FTRUST_ARGUMENT",
                "用法: trust <show|add-key|revoke-key|revoke-publisher> <publisher> [public-key-hex] [--db PATH]",
                json,
                true,
            );
        }
    };
    match result {
        Ok(view) => {
            if json {
                println!("{}", serialize_json(&view));
            } else {
                println!("trust: {} state={}", view.publisher_id, view.state);
                for key in view.keys {
                    println!("  key={} state={}", key.key_id, key.state);
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => render_basic_error(error.code(), &error.to_string(), json, false),
    }
}

fn cmd_conformance(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    if let Err(detail) = author_positionals(args, &[], &[], 0) {
        return render_basic_error("FCONF_ARGUMENT", &detail, json, true);
    }
    match conformance::lifecycle_report() {
        Ok(report) => {
            if json {
                println!("{}", serialize_json(&report));
            } else {
                println!(
                    "conformance: PASS suite={} engine={} vectors={}",
                    report.suite,
                    report.contract.engine_api_version,
                    report.contract.vectors.len()
                );
                for vector in report.contract.vectors {
                    println!(
                        "  {} callback={} guest-error={} host={}",
                        vector.id,
                        vector.callback,
                        vector.guest_error,
                        vector.expected_host_outcome
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => render_basic_error(error.code(), &error.to_string(), json, false),
    }
}

fn cmd_check(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let deny_warnings = args.iter().any(|argument| argument == "--deny-warnings");
    let mut project_dir = None;
    for argument in args {
        match argument.as_str() {
            "--json" | "--no-interactive" | "--deny-warnings" => {}
            value if value.starts_with('-') => {
                return render_check_error(
                    "FCHECK_ARGUMENT",
                    &format!("未知选项 {value}"),
                    check::CheckPhases::default(),
                    json,
                    true,
                );
            }
            value if project_dir.is_none() => project_dir = Some(PathBuf::from(value)),
            _ => {
                return render_check_error(
                    "FCHECK_ARGUMENT",
                    "check 只接受一个项目目录",
                    check::CheckPhases::default(),
                    json,
                    true,
                );
            }
        }
    }
    let project_dir = project_dir.unwrap_or_else(|| PathBuf::from("."));
    match check::check_project(&project_dir) {
        Ok(report) => {
            if deny_warnings && !report.warnings.is_empty() {
                return render_check_error(
                    "FCHECK_WARNINGS_DENIED",
                    "warning 已被 --deny-warnings 提升为失败",
                    report.phases,
                    json,
                    false,
                );
            }
            if json {
                match serde_json::to_string(&report) {
                    Ok(value) => println!("{value}"),
                    Err(error) => {
                        return render_check_error(
                            "FCHECK_SERIALIZE",
                            &error.to_string(),
                            report.phases,
                            true,
                            false,
                        );
                    }
                }
            } else {
                println!(
                    "check: PASS {}@{}",
                    report.inspection.package.id, report.inspection.package.version
                );
                println!("  metadata wasm ui manifest package: ok");
                println!("  warnings={}", report.warnings.len());
                for warning in &report.warnings {
                    println!("  warning code={} {}", warning.code, warning.message);
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => render_check_error(
            error.code(),
            error.public_detail().as_ref(),
            error.phases(),
            json,
            false,
        ),
    }
}

fn render_check_error(
    code: &str,
    detail: &str,
    phases: check::CheckPhases,
    json: bool,
    argument_error: bool,
) -> ExitCode {
    if json {
        let report = CommandErrorReport::new(code, detail, phases);
        eprintln!("{}", serialize_json(&report));
    } else {
        eprintln!("check: FAIL code={code} detail={detail}");
    }
    if argument_error {
        ExitCode::from(2)
    } else {
        ExitCode::FAILURE
    }
}

fn cmd_inspect(args: &[String]) -> ExitCode {
    let mut path = None;
    let json = args.iter().any(|arg| arg == "--json");
    for arg in args {
        match arg.as_str() {
            "--json" => {}
            "--no-interactive" | "--deny-warnings" => {}
            value if value.starts_with('-') => {
                return render_inspect_error(
                    "FINSPECT_ARGUMENT",
                    &format!("未知选项 {value}"),
                    json,
                );
            }
            value if path.is_none() => path = Some(PathBuf::from(value)),
            _ => {
                return render_inspect_error("FINSPECT_ARGUMENT", "inspect 只接受一个包路径", json);
            }
        }
    }
    let Some(path) = path else {
        return render_inspect_error(
            "FINSPECT_ARGUMENT",
            "用法: floatile inspect <pkg.floatile> [--json] [--no-interactive] [--deny-warnings]",
            json,
        );
    };
    match inspect::inspect_package(&path, &package::PackageLimits::default()) {
        Ok(report) => {
            if json {
                match serde_json::to_string(&report) {
                    Ok(value) => println!("{value}"),
                    Err(error) => {
                        return render_inspect_error(
                            "FINSPECT_SERIALIZE",
                            &error.to_string(),
                            true,
                        );
                    }
                }
            } else {
                println!(
                    "{} {} ({})",
                    report.package.id, report.package.version, report.package.name
                );
                println!(
                    "contract manifest={} engine={} ui={}",
                    report.package.manifest_version,
                    report.compatibility.engine_api_version,
                    report.compatibility.ui_api_version
                );
                println!(
                    "entries={} uncompressed={} archive={} digest={}",
                    report.budget.entry_count,
                    report.budget.uncompressed_bytes,
                    report.budget.archive_bytes,
                    report.digest
                );
                for permission in &report.permissions {
                    println!("permission {}", permission.capability);
                }
                for entry in &report.entries {
                    println!("entry {} {} {}", entry.bytes, entry.sha256, entry.path);
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => render_inspect_error(error.code(), &error.to_string(), json),
    }
}

fn render_inspect_error(code: &str, detail: &str, json: bool) -> ExitCode {
    if json {
        let report = CommandErrorReport::new(code, detail, serde_json::json!({}));
        eprintln!("{}", serialize_json(&report));
    } else {
        eprintln!("inspect 失败: code={code} detail={detail}");
    }
    if code == "FINSPECT_ARGUMENT" {
        ExitCode::from(2)
    } else {
        ExitCode::FAILURE
    }
}

fn serialize_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"schemaVersion":1,"status":"error","severity":"error","code":"FCLI_SERIALIZE","detail":"自动化结果序列化失败","phases":{},"warnings":[]}"#.to_owned()
    })
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
    let json = args.iter().any(|argument| argument == "--json");
    let positionals = match author_positionals(args, &[], &[], 3) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FNEW_ARGUMENT", &detail, json, true),
    };
    let dir = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let id = positionals
        .get(1)
        .map(|value| (*value).to_owned())
        .unwrap_or_else(|| "dev.example.widget".to_owned());
    let name = positionals
        .get(2)
        .map(|value| (*value).to_owned())
        .unwrap_or_else(|| "My Widget".to_owned());
    match project::generate_template(&dir, &id, &name) {
        Ok(()) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schemaVersion": 1,
                        "status": "ok",
                        "severity": "info",
                        "code": "ok",
                        "warnings": [],
                        "project": { "id": id, "name": name },
                    })
                );
            } else {
                println!("已生成项目模板于 {}", dir.display());
                println!(
                    "注意: floatile-sdk 尚未发布（许可 ADR 未通过），模板需在 SDK 可用后才能独立构建；workspace 内成员可直接构建。"
                );
            }
            ExitCode::SUCCESS
        }
        Err(_) => render_basic_error("FNEW_PROJECT", "无法生成项目模板", json, false),
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
    let json = args.iter().any(|a| a == "--json");
    let positionals =
        match author_positionals(args, &["--interval", "--duration-ms"], &["--once"], 1) {
            Ok(positionals) => positionals,
            Err(detail) => return render_basic_error("FDEV_ARGUMENT", &detail, json, true),
        };
    let dir = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let interval = match option_u64(args, "--interval", 500) {
        Ok(value) if value >= 50 => value,
        Ok(_) => {
            return render_basic_error("FDEV_ARGUMENT", "--interval 不得小于 50ms", json, true);
        }
        Err(detail) => return render_basic_error("FDEV_ARGUMENT", &detail, json, true),
    };
    if let Err(error) = dev::ensure_project(&dir) {
        return render_basic_error(error.code(), error.public_detail().as_ref(), json, false);
    }
    if args.iter().any(|argument| argument == "--once") {
        let duration_ms = match option_u64(args, "--duration-ms", 800) {
            Ok(value) if value > 0 => value,
            Ok(_) => return render_basic_error("FDEV_ARGUMENT", "运行时限必须大于 0", json, true),
            Err(detail) => return render_basic_error("FDEV_ARGUMENT", &detail, json, true),
        };
        return match preview::preview_project(&dir, Duration::from_millis(duration_ms)) {
            Ok(report) => {
                if json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "schemaVersion": 1,
                            "status": report.status,
                            "severity": report.severity,
                            "code": report.code,
                            "warnings": report.warnings,
                            "event": "preview_started",
                            "generation": 1,
                            "running": report.running,
                        })
                    );
                } else {
                    println!("[ok] preview generation 1 reached running");
                }
                if report.running {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(error) => render_basic_error(error.code(), error.public_detail(), json, false),
        };
    }
    let out = dir.join("out").join("plugin.floatile");
    dev::dev_loop(&dir, &out, interval, json);
}

fn cmd_build(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let positionals = match author_positionals(args, &[], &[], 2) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FBUILD_ARGUMENT", &detail, json, true),
    };
    let dir = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let out = positionals
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| dir.join("out").join("plugin.floatile"));
    match build::build_project(&dir, &out) {
        Ok(manifest) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schemaVersion": 1, "status": "ok", "severity": "info",
                        "code": "ok", "warnings": [],
                        "package": { "id": manifest.id.0, "version": manifest.version }
                    })
                );
            } else {
                println!("已构建 {} (id={})", out.display(), manifest.id.0);
            }
            ExitCode::SUCCESS
        }
        Err(error) => render_basic_error(error.code(), error.public_detail().as_ref(), json, false),
    }
}

fn cmd_test(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let positionals = match author_positionals(
        args,
        &["--timeout", "--event", "--payload", "--advance-ms"],
        &["--deny-all"],
        1,
    ) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FTEST_ARGUMENT", &detail, json, true),
    };
    let dir = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let timeout_ms: u64 = args
        .windows(2)
        .find(|w| w[0] == "--timeout")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(4000);
    let advance_ms = match option_u64(args, "--advance-ms", 0) {
        Ok(value) => value,
        Err(detail) => return render_basic_error("FTEST_ARGUMENT", &detail, json, true),
    };
    let event = option_string(args, "--event");
    let payload = option_string(args, "--payload").unwrap_or_else(|| "{}".to_owned());
    if serde_json::from_str::<serde_json::Value>(&payload).is_err() {
        return render_basic_error("FTEST_ARGUMENT", "--payload 必须是有效 JSON", json, true);
    }
    let scenario = test::TestScenario {
        ui_events: event.map(|name| vec![(name, payload)]).unwrap_or_default(),
        deny_all: args.iter().any(|argument| argument == "--deny-all"),
        advance_time: Duration::from_millis(advance_ms),
    };
    let out = dir.join("out").join("plugin.floatile");
    match test::test_project_with_scenario(&dir, &out, Duration::from_millis(timeout_ms), scenario)
    {
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
        Err(error) => render_basic_error(error.code(), error.public_detail(), json, false),
    }
}

fn cmd_install(args: &[String]) -> ExitCode {
    let json = args.iter().any(|a| a == "--json");
    let require_trusted = args.iter().any(|argument| argument == "--require-trusted");
    let accept_permissions = args
        .iter()
        .any(|argument| argument == "--accept-permissions");
    let positionals = match author_positionals(
        args,
        &["--store", "--db"],
        &["--require-trusted", "--accept-permissions"],
        1,
    ) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FINST_ARGUMENT", &detail, json, true),
    };
    let Some(path) = positionals.first().map(PathBuf::from) else {
        return render_basic_error("FINST_ARGUMENT", "install 需要包路径", json, true);
    };
    let store = match opts_store(args) {
        Ok(store) => store,
        Err(msg) => {
            return render_basic_error("FINST_ARGUMENT", &msg, json, true);
        }
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return render_basic_error("FINST_IO", "插件包读取失败", json, false),
    };
    let source = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin.floatile".to_owned());
    let result = if require_trusted {
        let database = match opts_database(args) {
            Ok(database) => database,
            Err(message) => return render_basic_error("FINST_ARGUMENT", &message, json, true),
        };
        if let Some(parent) = database.parent()
            && !parent.as_os_str().is_empty()
            && std::fs::create_dir_all(parent).is_err()
        {
            return render_basic_error("FINST_TRUST_STORE", "无法创建信任数据库目录", json, false);
        }
        let trust_store = match floatile_store::open(&database) {
            Ok(store) => store,
            Err(_) => {
                return render_basic_error("FINST_TRUST_STORE", "无法打开信任数据库", json, false);
            }
        };
        if let Err(error) = install::recover_trusted_installs(&store, &trust_store) {
            return render_basic_error(error.code(), error.public_detail().as_ref(), json, false);
        }
        install::install_trusted_package(
            &bytes,
            &store,
            &source,
            &package::PackageLimits::default(),
            &trust_store,
            accept_permissions,
        )
    } else {
        install::install_package(&bytes, &store, &source, &package::PackageLimits::default())
    };
    match result {
        Ok(installed) => {
            let upgrade = installed.upgrade.as_ref().map(upgrade_json);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "schemaVersion": 1, "status": "ok", "severity": "info",
                        "code": "ok", "warnings": [],
                        "installation": {
                            "id": installed.manifest.id.0,
                            "version": installed.meta.version,
                            "dir": installed.dir.display().to_string(),
                            "digest": installed.meta.digest,
                            "trust": if require_trusted { "trusted" } else { "unsigned" },
                            "upgrade": upgrade,
                        }
                    })
                );
            } else {
                println!(
                    "已安装 {} {} -> {} (digest {}, trust={})",
                    installed.manifest.id.0,
                    installed.meta.version,
                    installed.dir.display(),
                    &installed.meta.digest[..12],
                    if require_trusted {
                        "trusted"
                    } else {
                        "unsigned"
                    },
                );
                if let Some(plan) = &installed.upgrade {
                    println!(
                        "  upgrade {} -> {} permission_confirmation={}",
                        plan.current_version, plan.candidate_version, plan.requires_confirmation
                    );
                    for change in &plan.permissions {
                        println!("    {} {:?}", change.capability.name(), change.kind);
                    }
                }
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if let install::InstallError::PermissionConfirmationRequired(plan) = &error {
                let detail = error.public_detail();
                if json {
                    let report = CommandErrorReport::new(
                        error.code(),
                        detail.as_ref(),
                        serde_json::json!({ "upgrade": upgrade_json(plan) }),
                    );
                    eprintln!("{}", serialize_json(&report));
                } else {
                    eprintln!("命令失败: code={} detail={detail}", error.code());
                    for change in &plan.permissions {
                        eprintln!("  {} {:?}", change.capability.name(), change.kind);
                    }
                }
                ExitCode::FAILURE
            } else {
                render_basic_error(error.code(), error.public_detail().as_ref(), json, false)
            }
        }
    }
}

fn upgrade_json(plan: &floatile_core::UpgradePlan) -> serde_json::Value {
    serde_json::json!({
        "currentVersion": plan.current_version.to_string(),
        "candidateVersion": plan.candidate_version.to_string(),
        "requiresConfirmation": plan.requires_confirmation,
        "permissions": plan.permissions.iter().map(|change| serde_json::json!({
            "capability": change.capability,
            "change": match change.kind {
                PermissionChangeKind::Added => "added",
                PermissionChangeKind::Removed => "removed",
                PermissionChangeKind::Expanded => "expanded",
                PermissionChangeKind::Reduced => "reduced",
                PermissionChangeKind::Unchanged => "unchanged",
            }
        })).collect::<Vec<_>>()
    })
}

fn cmd_preview(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let positionals = match author_positionals(args, &["--duration-ms"], &[], 1) {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FPREVIEW_ARGUMENT", &detail, json, true),
    };
    let project = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let duration_ms = match option_u64(args, "--duration-ms", 5_000) {
        Ok(value) if value > 0 => value,
        Ok(_) => {
            return render_basic_error("FPREVIEW_ARGUMENT", "--duration-ms 必须大于 0", json, true);
        }
        Err(detail) => return render_basic_error("FPREVIEW_ARGUMENT", &detail, json, true),
    };
    match preview::preview_project(&project, Duration::from_millis(duration_ms)) {
        Ok(report) => {
            if json {
                println!("{}", serialize_json(&report));
            } else if report.running {
                println!("preview: PASS，真实宿主窗口已进入 running");
            } else {
                eprintln!("preview: FAIL code={}", report.code);
            }
            if report.running {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => render_basic_error(error.code(), error.public_detail(), json, false),
    }
}

fn cmd_run(args: &[String]) -> ExitCode {
    let json = args.iter().any(|argument| argument == "--json");
    let positionals = match author_positionals(args, &["--duration-ms", "--db", "--store"], &[], 1)
    {
        Ok(positionals) => positionals,
        Err(detail) => return render_basic_error("FRUN_ARGUMENT", &detail, json, true),
    };
    let project = positionals
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let duration_ms = match option_u64(args, "--duration-ms", 24 * 60 * 60 * 1_000) {
        Ok(value) if value > 0 => value,
        Ok(_) => return render_basic_error("FRUN_ARGUMENT", "运行时限必须大于 0", json, true),
        Err(detail) => return render_basic_error("FRUN_ARGUMENT", &detail, json, true),
    };
    let (default_database, default_store) = match run::default_run_paths() {
        Ok(paths) => paths,
        Err(error) => return render_basic_error(error.code(), error.public_detail(), json, false),
    };
    let database = option_path(args, "--db").unwrap_or(default_database);
    let store = option_path(args, "--store").unwrap_or(default_store);
    match run::run_project(
        &project,
        &database,
        &store,
        Duration::from_millis(duration_ms),
    ) {
        Ok(report) => {
            if json {
                println!("{}", serialize_json(&report));
            } else if report.running {
                println!(
                    "run: PASS instance={} {}@{}",
                    report.instance_id, report.plugin_id, report.version
                );
            } else {
                eprintln!("run: FAIL code={}", report.code);
            }
            if report.running {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => render_basic_error(error.code(), error.public_detail(), json, false),
    }
}

fn author_positionals<'a>(
    args: &'a [String],
    value_options: &[&str],
    boolean_options: &[&str],
    maximum: usize,
) -> Result<Vec<&'a str>, String> {
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        if matches!(argument, "--json" | "--no-interactive" | "--deny-warnings")
            || boolean_options.contains(&argument)
        {
            index += 1;
        } else if value_options.contains(&argument) {
            if args
                .get(index + 1)
                .is_none_or(|value| value.starts_with('-'))
            {
                return Err(format!("{argument} 缺少值"));
            }
            index += 2;
        } else if argument.starts_with('-') {
            return Err(format!("未知选项 {argument}"));
        } else {
            positionals.push(argument);
            index += 1;
        }
    }
    if positionals.len() > maximum {
        return Err(format!("位置参数最多允许 {maximum} 个"));
    }
    Ok(positionals)
}

fn option_u64(args: &[String], name: &str, default: u64) -> Result<u64, String> {
    let Some(index) = args.iter().position(|argument| argument == name) else {
        return Ok(default);
    };
    args.get(index + 1)
        .ok_or_else(|| format!("{name} 缺少值"))?
        .parse()
        .map_err(|_| format!("{name} 必须是整数"))
}

fn option_path(args: &[String], name: &str) -> Option<PathBuf> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
}

fn option_string(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|argument| argument == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn render_basic_error(code: &str, detail: &str, json: bool, argument_error: bool) -> ExitCode {
    if json {
        let report = CommandErrorReport::new(code, detail, serde_json::json!({}));
        eprintln!("{}", serialize_json(&report));
    } else {
        eprintln!("命令失败: code={code} detail={detail}");
    }
    if argument_error {
        ExitCode::from(2)
    } else {
        ExitCode::FAILURE
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

/// 解析宿主持久化数据库：优先 `--db PATH`，其次环境变量，最后平台数据目录。
fn opts_database(args: &[String]) -> Result<PathBuf, String> {
    if let Some(path) = option_path(args, "--db") {
        return Ok(path);
    }
    if let Some(path) = std::env::var_os("FLOATTILE_DB_PATH").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    floatile_platform::data_dir()
        .map(|path| path.join("layout.db"))
        .map_err(|error| error.to_string())
}
