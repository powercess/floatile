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
const SCHEMA_VERSION: u32 = 1;

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
        self.conn.execute(
            "INSERT INTO layout (instance_id, plugin_id, monitor_key, x, y, w, h, z, mode, version, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(instance_id) DO UPDATE SET
                 plugin_id = excluded.plugin_id,
                 monitor_key = excluded.monitor_key,
                 x = excluded.x,
                 y = excluded.y,
                 w = excluded.w,
                 h = excluded.h,
                 z = excluded.z,
                 mode = excluded.mode,
                 version = excluded.version,
                 updated_at = excluded.updated_at",
            rusqlite::params![
                layout.instance_id.0 as i64,
                layout.plugin_id.0,
                layout.monitor_key,
                layout.rect.position.x as f64,
                layout.rect.position.y as f64,
                layout.rect.size.width as f64,
                layout.rect.size.height as f64,
                layout.z as i64,
                mode,
                layout.version as i64,
                layout.updated_at as i64,
            ],
        )?;
        Ok(())
    }

    /// 按实例 ID 读取布局记录。
    pub fn get(&self, instance_id: u64) -> Result<Option<WidgetLayout>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, plugin_id, monitor_key, x, y, w, h, z, mode, version, updated_at
             FROM layout WHERE instance_id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![instance_id as i64])?;
        let row = rows.next()?;
        let Some(row) = row else { return Ok(None) };
        let mode = row.get::<_, String>(8)?;
        let mode = match mode.as_str() {
            "edit" => floatile_core::WidgetMode::Edit,
            "show" => floatile_core::WidgetMode::Show,
            other => return Err(StoreError::Corrupt(format!("未知 mode: {other}"))),
        };
        Ok(Some(WidgetLayout {
            instance_id: floatile_core::InstanceId(row.get::<_, i64>(0)? as u64),
            plugin_id: floatile_core::PluginId(row.get(1)?),
            monitor_key: row.get(2)?,
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
            z: row.get::<_, i64>(7)? as u32,
            mode,
            version: row.get::<_, i64>(9)? as u32,
            updated_at: row.get::<_, i64>(10)? as u64,
        }))
    }

    /// 列出全部布局记录（按 z 升序）。
    pub fn list(&self) -> Result<Vec<WidgetLayout>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT instance_id, plugin_id, monitor_key, x, y, w, h, z, mode, version, updated_at
             FROM layout ORDER BY z ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let mode = row.get::<_, String>(8)?;
            let mode = match mode.as_str() {
                "edit" => floatile_core::WidgetMode::Edit,
                "show" => floatile_core::WidgetMode::Show,
                _ => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        8,
                        rusqlite::types::Type::Text,
                        Box::new(StoreError::Corrupt(format!("未知 mode: {mode}"))),
                    ));
                }
            };
            Ok(WidgetLayout {
                instance_id: floatile_core::InstanceId(row.get::<_, i64>(0)? as u64),
                plugin_id: floatile_core::PluginId(row.get(1)?),
                monitor_key: row.get(2)?,
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
                z: row.get::<_, i64>(7)? as u32,
                mode,
                version: row.get::<_, i64>(9)? as u32,
                updated_at: row.get::<_, i64>(10)? as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 删除布局记录。
    pub fn delete(&self, instance_id: u64) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM layout WHERE instance_id = ?1",
            rusqlite::params![instance_id as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use floatile_core::{
        InstanceId, LogicalPosition, LogicalRect, LogicalSize, PluginId, WidgetMode,
    };

    fn sample(id: u64) -> WidgetLayout {
        WidgetLayout {
            instance_id: InstanceId(id),
            plugin_id: PluginId("dev.floatile.clock".into()),
            monitor_key: Some("edid-abc123".into()),
            rect: LogicalRect {
                position: LogicalPosition { x: 120.0, y: 80.0 },
                size: LogicalSize {
                    width: 260.0,
                    height: 120.0,
                },
            },
            z: 10,
            mode: WidgetMode::Edit,
            version: 1,
            updated_at: 1_700_000_000,
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
    fn save_get_roundtrip() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        let got = store.layout().get(1).unwrap().unwrap();
        assert_eq!(got, sample(1));
    }

    #[test]
    fn save_upserts_same_instance() {
        let store = open(":memory:").unwrap();
        store.layout().save(&sample(1)).unwrap();
        let mut updated = sample(1);
        updated.mode = WidgetMode::Show;
        store.layout().save(&updated).unwrap();
        let got = store.layout().get(1).unwrap().unwrap();
        assert_eq!(got.mode, WidgetMode::Show);
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
    fn newer_schema_version_rejected() {
        let mut store = open(":memory:").unwrap();
        store.conn.execute("PRAGMA user_version = 99", []).unwrap();
        // migrate() 只在 open 时执行；直接验证已有连接不允许更高版本。
        let result = store.migrate();
        assert!(matches!(result, Err(StoreError::Migration(_))));
    }
}
