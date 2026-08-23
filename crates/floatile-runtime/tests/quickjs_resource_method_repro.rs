//! componentize-qjs resource method 参数 lowering 的最小复现。
//!
//! 组件由 `spikes/typescript-runtime` 构建；测试刻意 ignored，不进入默认 Rust
//! 门禁。它把 Floatile Clock 缩减为无 import 的 constructor/ping/scalar/variant，
//! 用来区分 resource receiver 与 variant lowering。
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use wasmtime::component::{Component, Instance, Linker, ResourceAny, Val};
use wasmtime::{Config, Engine, Store};

const INTERFACE: &str = "floatile:quickjs-repro/contract@1.0.0";

fn component_path() -> PathBuf {
    std::env::var_os("FLOATILE_QUICKJS_REPRO_WASM")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("target/typescript-runtime-spike/quickjs-resource-method-repro.wasm")
        })
}

fn instantiate() -> (Store<()>, Instance) {
    let path = component_path();
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config).expect("创建 component engine");
    let component = Component::from_file(&engine, &path)
        .unwrap_or_else(|error| panic!("读取 QuickJS repro {} 失败: {error}", path.display()));
    let linker = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker
        .instantiate(&mut store, &component)
        .expect("实例化无 import 的 QuickJS repro");
    (store, instance)
}

fn construct(store: &mut Store<()>, instance: &Instance) -> ResourceAny {
    let interface = instance
        .get_export_index(&mut *store, None, INTERFACE)
        .expect("contract interface export");
    let constructor = instance
        .get_export_index(&mut *store, Some(&interface), "[constructor]instance")
        .expect("instance constructor export");
    let constructor = instance
        .get_func(&mut *store, constructor)
        .expect("instance constructor func");
    let mut results = [Val::Bool(false)];
    constructor
        .call(&mut *store, &[], &mut results)
        .expect("constructor 无参数，应成功");
    match results[0].clone() {
        Val::Resource(resource) => resource,
        value => panic!("constructor 应返回 resource，实际 {value:?}"),
    }
}

fn call_method(
    store: &mut Store<()>,
    instance: &Instance,
    name: &str,
    receiver: &ResourceAny,
    argument: Option<Val>,
) -> wasmtime::Result<()> {
    let interface = instance
        .get_export_index(&mut *store, None, INTERFACE)
        .expect("contract interface export");
    let method = instance
        .get_export_index(
            &mut *store,
            Some(&interface),
            &format!("[method]instance.{name}"),
        )
        .expect("resource method export");
    let method = instance
        .get_func(&mut *store, method)
        .expect("resource method func");
    let mut args = vec![Val::Resource(*receiver)];
    if let Some(argument) = argument {
        args.push(argument);
    }
    method.call(&mut *store, &args, &mut [])?;
    Ok(())
}

fn last_value(store: &mut Store<()>, instance: &Instance, receiver: &ResourceAny) -> String {
    let interface = instance
        .get_export_index(&mut *store, None, INTERFACE)
        .expect("contract interface export");
    let method = instance
        .get_export_index(&mut *store, Some(&interface), "[method]instance.get-last")
        .expect("get-last method export");
    let method = instance
        .get_func(&mut *store, method)
        .expect("get-last method func");
    let mut results = [Val::Bool(false)];
    method
        .call(&mut *store, &[Val::Resource(*receiver)], &mut results)
        .expect("get-last 无参数，应成功");
    match results[0].clone() {
        Val::String(value) => value,
        value => panic!("get-last 应返回 string，实际 {value:?}"),
    }
}

#[test]
#[ignore = "requires pnpm repro:quickjs"]
fn quickjs_resource_method_without_arguments_works() {
    let (mut store, instance) = instantiate();
    let receiver = construct(&mut store, &instance);
    call_method(&mut store, &instance, "ping", &receiver, None)
        .expect("无参数 resource method 是有效对照组");
}

#[test]
#[ignore = "requires pnpm repro:quickjs"]
fn quickjs_resource_method_arguments_trap_before_javascript() {
    for (name, argument) in [
        ("scalar", Val::U32(7)),
        (
            "handle",
            Val::Variant("tick".into(), Some(Box::new(Val::U32(1)))),
        ),
    ] {
        let (mut store, instance) = instantiate();
        let receiver = construct(&mut store, &instance);
        let error = call_method(&mut store, &instance, name, &receiver, Some(argument))
            .expect_err("componentize-qjs 0.4.3 带参数 resource method 应复现 trap");
        let detail = format!("{error:#}");
        assert!(
            detail.contains("unreachable") || detail.contains("method receiver"),
            "应在 adapter receiver lowering 内失败，实际: {detail}"
        );
    }
}

#[test]
#[ignore = "requires patched componentize-qjs CLI"]
fn quickjs_resource_method_arguments_reach_javascript_after_receiver_fix() {
    for (name, argument, expected) in [
        ("scalar", Val::U32(7), "scalar:7"),
        (
            "handle",
            Val::Variant("tick".into(), Some(Box::new(Val::U32(9)))),
            "tick:9",
        ),
    ] {
        let (mut store, instance) = instantiate();
        let receiver = construct(&mut store, &instance);
        call_method(&mut store, &instance, name, &receiver, Some(argument))
            .expect("修复后参数 resource method 应成功");
        assert_eq!(last_value(&mut store, &instance, &receiver), expected);
    }
}
