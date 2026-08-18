//! `dev`：watch 项目变更并自动 rebuild/validate。
//!
//! P0 最小实现：轮询项目目录 mtime 签名（floatile.toml + src/），变更触发
//! `build_project`，输出人类可读或 `--json` 结构化诊断。真实预览依赖
//! renderer spike，未接入。

use std::path::Path;
use std::time::{Duration, SystemTime};

use crate::build::{BuildError, build_project};

/// 单次构建状态（结构化诊断，`--json` 输出）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildStatus {
    pub ok: bool,
    pub code: String,
    pub detail: String,
    pub out: String,
}

impl BuildStatus {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"ok":false,"code":"internal","detail":"json"}"#.to_owned())
    }
}

/// 执行一次构建并返回结构化状态（不 panic）。
pub fn build_once(project_dir: &Path, out: &Path) -> BuildStatus {
    match build_project(project_dir, out) {
        Ok(manifest) => BuildStatus {
            ok: true,
            code: "ok".into(),
            detail: format!("id={}", manifest.id.0),
            out: out.display().to_string(),
        },
        Err(e) => BuildStatus {
            ok: false,
            code: e.code().to_owned(),
            detail: e.to_string(),
            out: out.display().to_string(),
        },
    }
}

/// 项目文件变更签名：(最新 mtime, 文件数)。读取失败返回 `Ok(None)`。
fn watch_signature(project_dir: &Path) -> std::io::Result<Option<(SystemTime, usize)>> {
    let mut latest: Option<SystemTime> = None;
    let mut count = 0usize;
    let mut walk = |path: &Path| -> std::io::Result<()> {
        let mtime = std::fs::metadata(path)?.modified()?;
        latest = Some(match latest {
            Some(prev) if prev >= mtime => prev,
            _ => mtime,
        });
        count += 1;
        Ok(())
    };
    walk(&project_dir.join("floatile.toml"))?;
    walk(&project_dir.join("Cargo.toml"))?;
    let src = project_dir.join("src");
    if src.is_dir() {
        for entry in std::fs::read_dir(&src)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "rs") {
                walk(&entry.path())?;
            }
        }
    }
    Ok(latest.map(|m| (m, count)))
}

/// 轮询循环：初始构建一次，随后在签名变化时重建。
pub fn dev_loop(project_dir: &Path, out: &Path, interval_ms: u64, json: bool) -> ! {
    let mut last_sig = None;
    loop {
        match watch_signature(project_dir) {
            Ok(Some(sig)) => {
                if last_sig != Some(sig) {
                    let status = build_once(project_dir, out);
                    if json {
                        println!("{}", status.to_json());
                    } else if status.ok {
                        println!("[ok] {}", status.out);
                    } else {
                        println!("[fail] code={} detail={}", status.code, status.detail);
                    }
                    last_sig = Some(sig);
                }
            }
            Ok(_) => {}
            Err(e) => eprintln!("watch 错误（继续轮询）: {e}"),
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
}

/// 校验项目目录结构是否具备可构建前提（供 `dev`/`build` 共享）。
pub fn ensure_project(project_dir: &Path) -> Result<(), BuildError> {
    for required in ["Cargo.toml", "floatile.toml"] {
        if !project_dir.join(required).exists() {
            return Err(BuildError::CargoMetadata(format!(
                "{} 缺失",
                project_dir.join(required).display()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn build_once_reports_ok_for_clock_wasm() {
        let project_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("plugins/clock-wasm");
        let out =
            std::env::temp_dir().join(format!("floatile-dev-test-{}.floatile", std::process::id()));
        let status = build_once(&project_dir, &out);
        assert!(status.ok, "clock-wasm 应构建成功: {}", status.detail);
        assert_eq!(status.code, "ok");
        // JSON 输出可解析且含稳定字段。
        let parsed: serde_json::Value = serde_json::from_str(&status.to_json()).unwrap();
        assert_eq!(parsed["ok"], serde_json::json!(true));
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn build_once_reports_failure_for_missing_dir() {
        let status = build_once(
            &PathBuf::from("/nonexistent-floatile-project"),
            &PathBuf::from("/tmp/x.floatile"),
        );
        assert!(!status.ok);
        assert_eq!(status.code, "FBUILD_CARGO_METADATA");
    }

    #[test]
    fn watch_signature_changes_with_file_mtime() {
        let dir = std::env::temp_dir().join(format!("floatile-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("floatile.toml"), "[plugin]").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "fn main() {}").unwrap();
        let sig1 = watch_signature(&dir).unwrap().expect("应有签名");
        // 等待 mtime 粒度变化后修改。
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(dir.join("src/lib.rs"), "fn main() { /* changed */ }").unwrap();
        let sig2 = watch_signature(&dir).unwrap().expect("应有签名");
        assert_ne!(sig1, sig2, "修改文件后签名应变化");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ensure_project_checks_prerequisites() {
        let dir = std::env::temp_dir().join(format!("floatile-prereq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "").unwrap();
        assert!(ensure_project(&dir).is_err(), "缺 floatile.toml 应报错");
        std::fs::write(dir.join("floatile.toml"), "").unwrap();
        assert!(ensure_project(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
