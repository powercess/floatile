//! Floatile CLI：插件包校验、构建与开发工具。
//!
//! P0 先落地安全核心 `.floatile` 包校验（`package`），供后续 `validate/build`
//! 命令与 PluginManager 复用；不链接宿主 capability 实现来"顺便执行"插件。

pub mod build;
pub mod dev;
pub mod install;
pub mod instance;
pub mod package;
pub mod project;
pub mod test;

pub use build::{BuildError, build_project, package};
pub use dev::{BuildStatus, build_once, dev_loop, ensure_project};
pub use install::{InstallError, InstalledPackage, install_dir, install_package};
pub use instance::{
    InstanceCommandError, InstanceView, configure_instance, create_instance, delete_instance,
    get_instance, list_instances, set_instance_desired_state,
};
pub use package::{PackageError, PackageLimits, ValidatedPackage, validate_package};
pub use project::{
    ProjectConfig, ProjectError, generate_manifest, generate_template, parse_floatile_toml,
};
pub use test::{TestError, TestPhases, TestStatus, test_project};
