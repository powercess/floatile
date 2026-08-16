//! Floatile SQLite 持久化、migration 与事务边界。
//!
//! `Store` 负责打开数据库并执行前向 migration（只追加，禁止修改已发布版本）；
//! `LayoutStore` 提供布局记录的持久化接口。所有写入都走事务，失败回滚。

use std::path::Path;

use floatile_core::WidgetLayout;
use rusqlite::Connection;

/// 持久化错误。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("打开数据库失败: {0}")]
    Open(#[from] rusqlite::Error),
    #[error("迁移失败: {0}")]
    Migration(String),
    #[error("序列化失败: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("数据损坏或类型不匹配: {0}")]
    Corrupt(String),
}

/// 当前 schema 版本（与 migration 列表一一对应）。
const SCHEMA_VERSION: u32 = 2;

/// 打开数据库并迁移到最新版本。
///
/// `path` 为数据库文件路径；`:memory:` 可用于测试。
pub fn open(path: impl AsRef<Path>) -> Result<Store, StoreError> {
    let conn = Connection::open(path)?;
    let mut store = Store { conn };
    store.migrate()?;
    Ok(store)
}

/// SQLite 存储句柄。
pub struct Store {
    conn: Connection,
}

impl Store {
    /// 迁移到最新 schema。基于 `PRAGMA user_version` 只追加前向迁移。
    fn migrate(&mut self) -> Result<(), StoreError> {
        let current: u32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            return Err(StoreError::Migration(format!(
                "数据库版本 {current} 高于当前支持版本 {SCHEMA_VERSION}，请升级宿主"
            )));
        }
        if current < 1 {
            self.migration_v1()?;
        }
        if current < 2 {
            self.migration_v2()?;
        }
        Ok(())
    }

    fn migration_v1(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS layout (
                instance_id   INTEGER PRIMARY KEY,
                plugin_id     TEXT NOT NULL,
                monitor_key   TEXT,
                x             REAL NOT NULL,
                y             REAL NOT NULL,
                w             REAL NOT NULL,
                h             REAL NOT NULL,
                z             INTEGER NOT NULL,
                mode          TEXT NOT NULL,
                version       INTEGER NOT NULL,
                updated_at    INTEGER NOT NULL
            );
            PRAGMA user_version = 1;",
        )
        .map_err(|e| StoreError::Migration(format!("v1 建表失败: {e}")))?;
        tx.commit()
            .map_err(|e| StoreError::Migration(format!("v1 提交失败: {e}")))
    }

    fn migration_v2(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "ALTER TABLE layout
                ADD COLUMN scale_factor REAL NOT NULL DEFAULT 1.0;
            ALTER TABLE layout
                ADD COLUMN physical_w INTEGER NOT NULL DEFAULT 1;
            ALTER TABLE layout
                ADD COLUMN physical_h INTEGER NOT NULL DEFAULT 1;
            ALTER TABLE layout
                ADD COLUMN lost_monitor INTEGER NOT NULL DEFAULT 0
                    CHECK (lost_monitor IN (0, 1));
            UPDATE layout SET
                physical_w = MAX(1, CAST(ROUND(w) AS INTEGER)),
                physical_h = MAX(1, CAST(ROUND(h) AS INTEGER));
            PRAGMA user_version = 2;",
        )
        .map_err(|e| StoreError::Migration(format!("v2 增加 DPI/恢复状态失败: {e}")))?;
        tx.commit()
            .map_err(|e| StoreError::Migration(format!("v2 提交失败: {e}")))
    }

    /// 布局存储接口。
    pub fn layout(&self) -> LayoutStore<'_> {
        LayoutStore { conn: &self.conn }
    }
}

/// 布局记录存取（layout 表 CRUD）。
pub struct LayoutStore<'a> {
    conn: &'a Connection,
}

impl<'a> LayoutStore<'a> {
    /// 保存或更新一条布局记录（按 instance_id upsert）。
    pub fn save(&self, layout: &WidgetLayout) -> Result<(), StoreError> {
        layout
            .validate()
            .map_err(|e| StoreError::Corrupt(e.to_string()))?;
        let mode = match layout.mode {
            floatile_core::WidgetMode::Edit => "edit",
            floatile_core::WidgetMode::Show => "show",
        };
        let instance_id = sqlite_i64(layout.instance_id.0, "instance_id")?;
        let updated_at = sqlite_i64(layout.updated_at, "updated_at")?;
        self.conn.execute(
            "INSERT INTO layout (
                instance_id, plugin_id, monitor_key, x, y, w, h,
                physical_w, physical_h, scale_factor, lost_monitor,
                z, mode, version, updated_at
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(instance_id) DO UPDATE SET
                 plugin_id = excluded.plugin_id,
                 monitor_key = excluded.monitor_key,
                 x = excluded.x,
                 y = excluded.y,
                 w = excluded.w,
                 h = excluded.h,
                 physical_w = excluded.physical_w,
                 physical_h = excluded.physical_h,
                 scale_factor = excluded.scale_factor,
                 lost_monitor = excluded.lost_monitor,
                 z = excluded.z,
                 mode = excluded.mode,
                 version = excluded.version,
                 updated_at = excluded.updated_at",
            rusqlite::params![
                instance_id,
                layout.plugin_id.0,
                layout
                    .monitor_key
                    .as_ref()
                    .map(floatile_core::MonitorKey::as_str),
                f64::from(layout.rect.position.x),
                f64::from(layout.rect.position.y),
                f64::from(layout.rect.size.width),
                f64::from(layout.rect.size.height),
                i64::from(layout.physical_size.width),
                i64::from(layout.physical_size.height),
                layout.scale_factor.get(),
                layout.lost_monitor,
                i64::from(layout.z),
                mode,
                i64::from(layout.version),
                updated_at,
            ],
        )?;
        Ok(())
    }

    /// 按实例 ID 读取布局记录。
    pub fn get(&self, instance_id: u64) -> Result<Option<WidgetLayout>, StoreError> {
        let instance_id = sqlite_i64(instance_id, "instance_id")?;
        let mut stmt = self.conn.prepare(
            "SELECT
                instance_id, plugin_id, monitor_key, x, y, w, h,
                physical_w, physical_h, scale_factor, lost_monitor,
                z, mode, version, updated_at
             FROM layout WHERE instance_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![instance_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(row_to_layout(row)?))
    }

    /// 列出全部布局记录（按 z 升序）。
    pub fn list(&self) -> Result<Vec<WidgetLayout>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT
                instance_id, plugin_id, monitor_key, x, y, w, h,
                physical_w, physical_h, scale_factor, lost_monitor,
                z, mode, version, updated_at
             FROM layout ORDER BY z ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut layouts = Vec::new();
        while let Some(row) = rows.next()? {
            layouts.push(row_to_layout(row)?);
        }
        Ok(layouts)
    }

    /// 删除布局记录。
    pub fn delete(&self, instance_id: u64) -> Result<(), StoreError> {
        let instance_id = sqlite_i64(instance_id, "instance_id")?;
        self.conn.execute(
            "DELETE FROM layout WHERE instance_id = ?1",
            rusqlite::params![instance_id],
        )?;
        Ok(())
    }
}

fn sqlite_i64(value: u64, field: &'static str) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Corrupt(format!("{field} 超出 SQLite INTEGER 范围: {value}")))
}

fn row_to_layout(row: &rusqlite::Row<'_>) -> Result<WidgetLayout, StoreError> {
    let mode = row.get::<_, String>(12)?;
    let mode = match mode.as_str() {
        "edit" => floatile_core::WidgetMode::Edit,
        "show" => floatile_core::WidgetMode::Show,
        _ => return Err(StoreError::Corrupt(format!("未知 mode: {mode}"))),
    };
    let scale_factor = floatile_core::ScaleFactor::new(row.get(9)?)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    let lost_monitor = match row.get::<_, i64>(10)? {
        0 => false,
        1 => true,
        value => {
            return Err(StoreError::Corrupt(format!(
                "lost_monitor 必须为 0 或 1，实际为 {value}"
            )));
        }
    };
    let layout = WidgetLayout {
        instance_id: floatile_core::InstanceId(read_u64(row, 0, "instance_id")?),
        plugin_id: floatile_core::PluginId(row.get(1)?),
        monitor_key: row
            .get::<_, Option<String>>(2)?
            .map(floatile_core::MonitorKey),
        rect: floatile_core::LogicalRect {
            position: floatile_core::LogicalPosition {
                x: row.get::<_, f64>(3)? as f32,
                y: row.get::<_, f64>(4)? as f32,
            },
            size: floatile_core::LogicalSize {
                width: row.get::<_, f64>(5)? as f32,
                height: row.get::<_, f64>(6)? as f32,
            },
        },
        physical_size: floatile_core::PhysicalSize {
            width: read_u32(row, 7, "physical_w")?,
            height: read_u32(row, 8, "physical_h")?,
        },
        scale_factor,
        lost_monitor,
        z: read_u32(row, 11, "z")?,
        mode,
        version: read_u32(row, 13, "version")?,
        updated_at: read_u64(row, 14, "updated_at")?,
    };
    layout
        .validate()
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    Ok(layout)
}

fn read_u32(row: &rusqlite::Row<'_>, index: usize, field: &'static str) -> Result<u32, StoreError> {
    let value = row.get::<_, i64>(index)?;
    u32::try_from(value).map_err(|_| StoreError::Corrupt(format!("{field} 超出 u32 范围: {value}")))
}

fn read_u64(row: &rusqlite::Row<'_>, index: usize, field: &'static str) -> Result<u64, StoreError> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("{field} 不得为负数: {value}")))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use floatile_core::{
        InstanceId, LogicalPosition, LogicalRect, LogicalSize, MonitorKey, PhysicalSize, PluginId,
        ScaleFactor, WidgetMode,
    };

    fn sample(id: u64) -> WidgetLayout {
        WidgetLayout {
            instance_id: InstanceId(id),
            plugin_id: PluginId("dev.floatile.clock".into()),
            monitor_key: Some(MonitorKey("edid-abc123".into())),
            rect: LogicalRect {
                position: LogicalPosition { x: 120.0, y: 80.0 },
                size: LogicalSize {
                    width: 260.0,
                    height: 120.0,
                },
            },
            physical_size: PhysicalSize {
                width: 325,
                height: 150,
            },
            scale_factor: ScaleFactor::new(1.25).unwrap(),
            lost_monitor: false,
            z: 10,
            mode: WidgetMode::Edit,
            version: 1,
            updated_at: 1_700_000_000,
        }
    }

    fn v1_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store
    }

    struct TempDb(std::path::PathBuf);

    impl TempDb {
        fn new() -> Self {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Self(std::env::temp_dir().join(format!(
                "floatile-store-test-{}-{id}.sqlite",
                std::process::id()
            )))
        }
    }

    impl Drop for TempDb {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
            let _ = std::fs::remove_file(self.0.with_extension("sqlite-shm"));
            let _ = std::fs::remove_file(self.0.with_extension("sqlite-wal"));
        }
    }

    #[test]
    fn migration_creates_layout_table_and_user_version() {
        let store = open(":memory:").unwrap();
        let ver: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(ver, SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent_at_current_schema() {
        let mut store = open(":memory:").unwrap();
        store.migrate().unwrap();
        store.migrate().unwrap();
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn migration_v2_preserves_v1_layout_and_derives_physical_size() {
        let mut store = v1_store();
        store
            .conn
            .execute(
                "INSERT INTO layout (
                    instance_id, plugin_id, monitor_key, x, y, w, h, z, mode, version, updated_at
                 ) VALUES (1, 'dev.floatile.clock', 'edid-v1', 12, 34, 260, 120, 10, 'edit', 1, 1700000000)",
                [],
            )
            .unwrap();

        store.migrate().unwrap();

        let migrated = store.layout().get(1).unwrap().unwrap();
        assert_eq!(migrated.monitor_key, Some(MonitorKey("edid-v1".into())));
        assert_eq!(migrated.rect.position, LogicalPosition { x: 12.0, y: 34.0 });
        assert_eq!(
            migrated.physical_size,
            PhysicalSize {
                width: 260,
                height: 120
            }
        );
        assert_eq!(migrated.scale_factor, ScaleFactor::new(1.0).unwrap());
        assert!(!migrated.lost_monitor);
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn migration_v2_failure_rolls_back_added_columns_and_version() {
        let mut store = v1_store();
        store
            .conn
            .execute(
                "ALTER TABLE layout ADD COLUMN physical_w INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .unwrap();

        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));

        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
        let mut stmt = store.conn.prepare("PRAGMA table_info(layout)").unwrap();
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "physical_w"));
        assert!(!columns.iter().any(|column| column == "scale_factor"));
    }

    #[test]
    fn save_get_roundtrip() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        let got = store.layout().get(1).unwrap().unwrap();
        assert_eq!(got, sample(1));
    }

    #[test]
    fn file_database_persists_layout_across_reopen() {
        let path = TempDb::new();
        {
            let store = open(&path.0).unwrap();
            store.layout().save(&sample(42)).unwrap();
        }
        {
            let store = open(&path.0).unwrap();
            assert_eq!(store.layout().get(42).unwrap(), Some(sample(42)));
        }
    }

    #[test]
    fn save_upserts_same_instance() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        let mut updated = sample(1);
        updated.mode = WidgetMode::Show;
        updated.lost_monitor = true;
        updated.scale_factor = ScaleFactor::new(2.0).unwrap();
        updated.physical_size = PhysicalSize {
            width: 520,
            height: 240,
        };
        store.layout().save(&updated).unwrap();
        let got = store.layout().get(1).unwrap().unwrap();
        assert_eq!(got, updated);
        assert_eq!(store.layout().list().unwrap().len(), 1);
    }

    #[test]
    fn list_orders_by_z() {
        let store = open(":memory:").unwrap();
        let mut a = sample(1);
        a.z = 20;
        let mut b = sample(2);
        b.z = 5;
        store.layout().save(&a).unwrap();
        store.layout().save(&b).unwrap();
        let list = store.layout().list().unwrap();
        assert_eq!(list[0].instance_id, InstanceId(2));
        assert_eq!(list[1].instance_id, InstanceId(1));
    }

    #[test]
    fn delete_removes_record() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        store.layout().delete(1).unwrap();
        assert!(store.layout().get(1).unwrap().is_none());
    }

    #[test]
    fn invalid_layout_rejected_on_save() {
        let store = open(":memory:").unwrap();
        let mut bad = sample(1);
        bad.rect.size.width = 0.0;
        assert!(matches!(
            store.layout().save(&bad),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn corrupt_mode_fails_load() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        store
            .conn
            .execute(
                "UPDATE layout SET mode = 'garbage' WHERE instance_id = 1",
                [],
            )
            .unwrap();
        assert!(matches!(store.layout().get(1), Err(StoreError::Corrupt(_))));
    }

    #[test]
    fn corrupt_dpi_data_fails_load() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        store
            .conn
            .execute(
                "UPDATE layout SET scale_factor = 0 WHERE instance_id = 1",
                [],
            )
            .unwrap();
        assert!(matches!(store.layout().get(1), Err(StoreError::Corrupt(_))));
    }

    #[test]
    fn newer_schema_version_rejected() {
        let mut store = open(":memory:").unwrap();
        store.conn.execute("PRAGMA user_version = 99", []).unwrap();
        // migrate() 只在 open 时执行；直接验证已有连接不允许更高版本。
        let result = store.migrate();
        assert!(matches!(result, Err(StoreError::Migration(_))));
    }
}
