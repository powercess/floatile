//! Floatile CLI：插件包校验、构建与开发工具。
//!
//! P0 先落地安全核心 `.floatile` 包校验（`package`），供后续 `validate/build`
//! 命令与 PluginManager 复用；不链接宿主 capability 实现来"顺便执行"插件。

pub mod build;
pub mod check;
pub mod conformance;
pub mod dev;
pub mod inspect;
pub mod install;
pub mod instance;
pub mod output;
pub mod package;
pub mod preview;
pub mod project;
pub mod run;
pub mod test;
pub mod trust;

pub use build::{BuildError, build_project, package};
pub use check::{CheckError, CheckPhases, CheckReport, CheckWarning, check_project};
pub use conformance::{ConformanceError, ConformanceReport, lifecycle_report};
pub use dev::{BuildStatus, build_once, dev_loop, ensure_project};
pub use inspect::{InspectError, InspectReport, inspect_package, inspect_package_bytes};
pub use install::{
    InstallError, InstalledPackage, RecoveryReport, install_dir, install_package,
    install_trusted_package, recover_trusted_installs,
};
pub use instance::{
    InstanceCommandError, InstanceView, configure_instance, create_instance, delete_instance,
    get_instance, list_instances, set_instance_desired_state,
};
pub use output::{CommandErrorReport, CommandWarning, OUTPUT_SCHEMA_VERSION};
pub use package::{PackageError, PackageLimits, ValidatedPackage, validate_package};
pub use preview::{PreviewError, PreviewReport, PreviewSession, preview_project};
pub use project::{
    ProjectConfig, ProjectError, generate_manifest, generate_template, parse_floatile_toml,
};
pub use run::{RunError, RunReport, default_run_paths, run_project};
pub use test::{
    TestError, TestPhases, TestScenario, TestStatus, test_project, test_project_with_scenario,
};
