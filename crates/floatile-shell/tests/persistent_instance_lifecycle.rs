//! PP-M1 动态持久实例 Xvfb 证据。
//!
//! 同一 Installation 的两个实例先独立进入 running；随后停止第二实例、临时移走安装
//! 内容使其单独进入 failed，第一实例必须保持 running；恢复内容后通过 supervisor
//! 手动 retry，第二实例重新进入 running。测试中的 SQLite/文件操作由协调 worker
//! 执行，Slint timer 只读取 observed 快照和发送有界命令。

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use floatile_core::install::{InstallMeta, content_digest, file_digest, hex_encode};
use floatile_core::{InstanceConfig, InstanceDesiredState};
use floatile_shell::instance_control::InstanceControlSurface;
use floatile_shell::instance_supervisor::{DynamicInstanceSupervisor, ObservedInstanceState};
use slint::{ComponentHandle, Timer, TimerMode};

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn clock_wasm() -> Option<Vec<u8>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("target/wasm32-wasip2/debug/floatile_clock_wasm.wasm");
    std::fs::read(path).ok()
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "floatile-persistent-lifecycle-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn write_clock_install(root: &Path, wasm: Vec<u8>) -> PathBuf {
    let dir = root.join("dev.floatile.clock").join("1.0.0");
    std::fs::create_dir_all(dir.join("ui")).unwrap();
    std::fs::create_dir_all(dir.join("logic")).unwrap();
    let manifest = serde_json::json!({
        "manifestVersion": 1,
        "id": "dev.floatile.clock",
        "name": "World Clock",
        "version": "1.0.0",
        "publisher": { "id": "dev.floatile", "name": "Floatile" },
        "engineApiVersion": "1.0.0",
        "uiApiVersion": "1.0.0",
        "type": "widget",
        "entrypoints": { "ui": "ui/widget.ftui", "logic": "logic/plugin.wasm" },
        "sizes": { "default": { "width": 240, "height": 120 }, "min": { "width": 160, "height": 80 }, "max": { "width": 800, "height": 600 }, "resizable": true },
        "permissions": [{ "capability": "timer:schedule", "params": { "maxPerMinute": 60, "maxActive": 4 } }]
    })
    .to_string()
    .into_bytes();
    let mut files = BTreeMap::from([
        ("logic/plugin.wasm".to_owned(), wasm),
        ("manifest.json".to_owned(), manifest),
        (
            "ui/widget.ftui".to_owned(),
            floatile_clock_wasm::__floatile_ftui_json().into_bytes(),
        ),
    ]);
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let meta = InstallMeta {
        manifest_version: 1,
        id: "dev.floatile.clock".to_owned(),
        version: "1.0.0".to_owned(),
        engine_api_version: "1.0.0".to_owned(),
        ui_api_version: "1.0.0".to_owned(),
        installed_at: 1,
        source: "xvfb-evidence".to_owned(),
        trust: floatile_core::install::InstallationTrust::Unsigned,
        files: files
            .iter()
            .map(|(name, bytes)| (name.clone(), hex_encode(&file_digest(bytes))))
            .collect(),
        digest: hex_encode(&content_digest(&files)),
    };
    std::fs::write(dir.join("install.json"), serde_json::to_vec(&meta).unwrap()).unwrap();
    files.clear();
    dir
}

#[derive(Debug)]
enum CoordinatorCommand {
    StopSecond,
    HideInstallationAndRunSecond,
    RestoreInstallation,
    Finish,
}

#[test]
fn two_persistent_windows_isolate_failure_and_manual_retry_recovers() {
    if !has_display() {
        eprintln!("SKIP: persistent lifecycle evidence needs DISPLAY/WAYLAND");
        return;
    }
    let Some(wasm) = clock_wasm() else {
        eprintln!("SKIP: build clock-wasm for wasm32-wasip2 before Xvfb evidence");
        return;
    };

    let root = temp_root();
    let plugin_store = root.join("plugins");
    let install_dir = write_clock_install(&plugin_store, wasm);
    let hidden_install = root.join("hidden-installation");
    let database = root.join("layout.db");
    let store = floatile_store::open(&database).unwrap();
    let installation =
        floatile_store::installation::load_exact(&plugin_store, "dev.floatile.clock", "1.0.0")
            .unwrap()
            .unwrap();
    let reference = installation.reference().unwrap();
    let first = store
        .instances()
        .create(
            &reference,
            &InstanceConfig::empty(),
            InstanceDesiredState::Running,
            1,
        )
        .unwrap();
    let second = store
        .instances()
        .create(
            &reference,
            &InstanceConfig::empty(),
            InstanceDesiredState::Running,
            2,
        )
        .unwrap();
    let first_id = first.id();
    let second_id = second.id();
    drop(store);

    let supervisor = DynamicInstanceSupervisor::start(
        database.clone(),
        plugin_store.clone(),
        floatile_platform::capability::probe(),
        None,
    )
    .unwrap();
    let handle = supervisor.handle();
    let control_surface =
        InstanceControlSurface::start(database.clone(), plugin_store, handle.clone()).unwrap();
    control_surface.weak().upgrade().unwrap().show().unwrap();
    let (command_tx, command_rx) = mpsc::sync_channel(2);
    let (ack_tx, ack_rx) = mpsc::sync_channel(2);
    let coordinator = std::thread::Builder::new()
        .name("floatile-lifecycle-evidence".to_owned())
        .spawn(move || {
            let store = floatile_store::open(&database).unwrap();
            let mut hidden = false;
            while let Ok(command) = command_rx.recv() {
                match command {
                    CoordinatorCommand::StopSecond => {
                        assert!(
                            store
                                .instances()
                                .set_desired_state(
                                    second_id,
                                    InstanceDesiredState::Stopped,
                                    unix_now(),
                                )
                                .unwrap()
                        );
                    }
                    CoordinatorCommand::HideInstallationAndRunSecond => {
                        std::fs::rename(&install_dir, &hidden_install).unwrap();
                        hidden = true;
                        assert!(
                            store
                                .instances()
                                .set_desired_state(
                                    second_id,
                                    InstanceDesiredState::Running,
                                    unix_now() + 1,
                                )
                                .unwrap()
                        );
                    }
                    CoordinatorCommand::RestoreInstallation => {
                        std::fs::rename(&hidden_install, &install_dir).unwrap();
                        hidden = false;
                    }
                    CoordinatorCommand::Finish => {
                        if hidden {
                            std::fs::rename(&hidden_install, &install_dir).unwrap();
                        }
                        break;
                    }
                }
                ack_tx.send(()).unwrap();
            }
        })
        .unwrap();

    let stage = Rc::new(Cell::new(0_u8));
    let awaiting = Rc::new(Cell::new(false));
    let passed = Rc::new(Cell::new(false));
    let final_status = Rc::new(RefCell::new(Vec::new()));
    let started_at = Instant::now();
    let timer = Timer::default();
    let timer_stage = Rc::clone(&stage);
    let timer_awaiting = Rc::clone(&awaiting);
    let timer_passed = Rc::clone(&passed);
    let timer_status = Rc::clone(&final_status);
    let timer_handle = handle.clone();
    let timer_command_tx = command_tx.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(100), move || {
        if timer_awaiting.get() && ack_rx.try_recv().is_ok() {
            timer_awaiting.set(false);
            timer_stage.set(timer_stage.get() + 1);
        }
        let observed = timer_handle.observed_snapshot();
        *timer_status.borrow_mut() = observed.clone();
        let state = |id| {
            observed
                .iter()
                .find(|status| status.instance_id == id)
                .map(|status| status.state)
        };
        if !timer_awaiting.get() {
            match timer_stage.get() {
                0 if state(first_id) == Some(ObservedInstanceState::Running)
                    && state(second_id) == Some(ObservedInstanceState::Running) =>
                {
                    timer_command_tx
                        .try_send(CoordinatorCommand::StopSecond)
                        .unwrap();
                    timer_awaiting.set(true);
                }
                1 if state(first_id) == Some(ObservedInstanceState::Running)
                    && state(second_id) == Some(ObservedInstanceState::Stopped) =>
                {
                    timer_command_tx
                        .try_send(CoordinatorCommand::HideInstallationAndRunSecond)
                        .unwrap();
                    timer_awaiting.set(true);
                }
                2 if state(first_id) == Some(ObservedInstanceState::Running)
                    && state(second_id) == Some(ObservedInstanceState::Failed) =>
                {
                    timer_command_tx
                        .try_send(CoordinatorCommand::RestoreInstallation)
                        .unwrap();
                    timer_awaiting.set(true);
                }
                3 => {
                    timer_handle.retry(second_id).unwrap();
                    timer_stage.set(4);
                }
                4 if state(first_id) == Some(ObservedInstanceState::Running)
                    && state(second_id) == Some(ObservedInstanceState::Running) =>
                {
                    timer_passed.set(true);
                    slint::quit_event_loop().unwrap();
                }
                _ => {}
            }
        }
        if started_at.elapsed() > Duration::from_secs(30) {
            slint::quit_event_loop().unwrap();
        }
    });

    slint::run_event_loop_until_quit().unwrap();
    timer.stop();
    let _ = command_tx.try_send(CoordinatorCommand::Finish);
    coordinator.join().unwrap();
    assert!(
        passed.get(),
        "dynamic lifecycle did not recover; stage={} awaiting={} observed={:?}",
        stage.get(),
        awaiting.get(),
        final_status.borrow(),
    );
    drop(control_surface);
    drop(supervisor);
}
