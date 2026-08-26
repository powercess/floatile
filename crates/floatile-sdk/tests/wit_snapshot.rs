//! SDK 包内 WIT 发行快照必须与仓库根事实源逐字节一致。

#![allow(clippy::expect_used)]

use std::path::PathBuf;

#[test]
fn packaged_wit_snapshot_matches_workspace_source() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let canonical = crate_root.join("../../wit/floatile-widget.wit");
    if !canonical.exists() {
        // 从发布包运行时根事实源不在包内；包内容已由仓库内测试和发布门生成。
        return;
    }
    let snapshot = crate_root.join("wit/floatile-widget.wit");
    let canonical_bytes = std::fs::read(canonical).expect("read canonical WIT");
    let snapshot_bytes = std::fs::read(snapshot).expect("read packaged WIT snapshot");
    assert_eq!(
        snapshot_bytes, canonical_bytes,
        "run scripts/sync-sdk-wit.sh"
    );
}
