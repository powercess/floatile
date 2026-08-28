//! Floatile SQLite 持久化、migration 与事务边界。
//!
//! `Store` 负责打开数据库并执行前向 migration（只追加，禁止修改已发布版本）；
//! `LayoutStore` 提供布局记录的持久化接口。所有写入都走事务，失败回滚。

use std::path::Path;

use floatile_core::{
    Connection as HostConnection, ConnectionGrant, ConnectionHealth, ConnectionId, CredentialRef,
    InstallationDigest, InstallationRef, InstanceConfig, InstanceDesiredState, InstanceId,
    PluginId, PluginInstance, WidgetLayout,
};
use rusqlite::{Connection, OptionalExtension};

pub mod installation;
pub mod trust;

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
const SCHEMA_VERSION: u32 = 8;

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
        if current < 3 {
            self.migration_v3()?;
        }
        if current < 4 {
            self.migration_v4()?;
        }
        if current < 5 {
            self.migration_v5()?;
        }
        if current < 6 {
            self.migration_v6()?;
        }
        if current < 7 {
            self.migration_v7()?;
        }
        if current < 8 {
            self.migration_v8()?;
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

    fn migration_v3(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS audit_log (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                unix_ts    INTEGER NOT NULL,
                plugin     TEXT NOT NULL,
                instance   INTEGER NOT NULL,
                capability TEXT NOT NULL,
                decision   TEXT NOT NULL CHECK (decision IN ('allow','deny')),
                reason     TEXT,
                detail     TEXT NOT NULL
            );
            PRAGMA user_version = 3;",
        )
        .map_err(|e| StoreError::Migration(format!("v3 建表失败: {e}")))?;
        tx.commit()
            .map_err(|e| StoreError::Migration(format!("v3 提交失败: {e}")))
    }

    fn migration_v4(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS plugin_instances (
                instance_id          INTEGER PRIMARY KEY,
                plugin_id            TEXT NOT NULL,
                installation_version TEXT NOT NULL,
                installation_digest  BLOB NOT NULL
                    CHECK (length(installation_digest) = 32),
                config_json          TEXT NOT NULL,
                desired_state        TEXT NOT NULL
                    CHECK (desired_state IN ('stopped','running')),
                generation           INTEGER NOT NULL DEFAULT 0
                    CHECK (generation >= 0),
                created_at           INTEGER NOT NULL CHECK (created_at >= 0),
                updated_at           INTEGER NOT NULL
                    CHECK (updated_at >= created_at)
            );
            CREATE INDEX IF NOT EXISTS plugin_instances_plugin
                ON plugin_instances(plugin_id);
            CREATE TABLE IF NOT EXISTS plugin_instance_id_allocator (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                next_id   INTEGER NOT NULL CHECK (next_id >= 2)
            );
            INSERT OR IGNORE INTO plugin_instance_id_allocator(singleton, next_id)
                VALUES (1, 2);
            PRAGMA user_version = 4;",
        )
        .map_err(|error| StoreError::Migration(format!("v4 建立插件实例表失败: {error}")))?;
        tx.commit()
            .map_err(|error| StoreError::Migration(format!("v4 提交失败: {error}")))
    }

    fn migration_v5(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS connections (
                connection_id        INTEGER PRIMARY KEY,
                provider             TEXT NOT NULL,
                account_identity     TEXT NOT NULL,
                credential_ref       TEXT NOT NULL UNIQUE,
                health               TEXT NOT NULL CHECK (
                    health IN ('unknown','healthy','degraded','unavailable','revoked')
                ),
                credential_generation INTEGER NOT NULL DEFAULT 0
                    CHECK (credential_generation >= 0),
                created_at           INTEGER NOT NULL CHECK (created_at >= 0),
                updated_at           INTEGER NOT NULL CHECK (updated_at >= created_at)
            );
            CREATE INDEX IF NOT EXISTS connections_provider
                ON connections(provider);
            CREATE TABLE IF NOT EXISTS connection_id_allocator (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                next_id   INTEGER NOT NULL CHECK (next_id >= 1)
            );
            INSERT OR IGNORE INTO connection_id_allocator(singleton, next_id)
                VALUES (1, 1);
            CREATE TABLE IF NOT EXISTS instance_connection_grants (
                instance_id   INTEGER NOT NULL,
                connection_id INTEGER NOT NULL,
                granted_at    INTEGER NOT NULL CHECK (granted_at >= 0),
                PRIMARY KEY (instance_id, connection_id),
                FOREIGN KEY (instance_id) REFERENCES plugin_instances(instance_id),
                FOREIGN KEY (connection_id) REFERENCES connections(connection_id)
            );
            CREATE INDEX IF NOT EXISTS instance_connection_grants_connection
                ON instance_connection_grants(connection_id);
            PRAGMA user_version = 5;",
        )
        .map_err(|error| StoreError::Migration(format!("v5 建立 Connection 表失败: {error}")))?;
        tx.commit()
            .map_err(|error| StoreError::Migration(format!("v5 提交失败: {error}")))
    }

    fn migration_v6(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS publisher_trust (
                publisher_id TEXT PRIMARY KEY,
                state        TEXT NOT NULL CHECK (state IN ('active','revoked')),
                updated_at   INTEGER NOT NULL CHECK (updated_at >= 0)
            );
            CREATE TABLE IF NOT EXISTS publisher_keys (
                publisher_id TEXT NOT NULL,
                key_id       TEXT NOT NULL,
                public_key   BLOB NOT NULL CHECK (length(public_key) = 32),
                state        TEXT NOT NULL CHECK (state IN ('active','revoked')),
                updated_at   INTEGER NOT NULL CHECK (updated_at >= 0),
                PRIMARY KEY (publisher_id, key_id),
                FOREIGN KEY (publisher_id) REFERENCES publisher_trust(publisher_id)
            );
            CREATE TABLE IF NOT EXISTS accepted_packages (
                publisher_id TEXT NOT NULL,
                plugin_id    TEXT NOT NULL,
                version      TEXT NOT NULL,
                digest       BLOB NOT NULL CHECK (length(digest) = 32),
                accepted_at  INTEGER NOT NULL CHECK (accepted_at >= 0),
                PRIMARY KEY (publisher_id, plugin_id),
                FOREIGN KEY (publisher_id) REFERENCES publisher_trust(publisher_id)
            );
            PRAGMA user_version = 6;",
        )
        .map_err(|error| {
            StoreError::Migration(format!(
                "v6 建立 publisher trust 与 anti-rollback 表失败: {error}"
            ))
        })?;
        tx.commit()
            .map_err(|error| StoreError::Migration(format!("v6 提交失败: {error}")))
    }

    fn migration_v7(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_installations (
                transaction_id TEXT PRIMARY KEY,
                publisher_id   TEXT NOT NULL,
                plugin_id      TEXT NOT NULL,
                version        TEXT NOT NULL,
                signed_digest  BLOB NOT NULL CHECK (length(signed_digest) = 32),
                install_digest BLOB NOT NULL CHECK (length(install_digest) = 32),
                staging_name   TEXT NOT NULL,
                final_relative TEXT NOT NULL,
                created_at     INTEGER NOT NULL CHECK (created_at >= 0),
                FOREIGN KEY (publisher_id) REFERENCES publisher_trust(publisher_id)
            );
            PRAGMA user_version = 7;",
        )
        .map_err(|error| StoreError::Migration(format!("v7 建立可恢复安装意图表失败: {error}")))?;
        tx.commit()
            .map_err(|error| StoreError::Migration(format!("v7 提交失败: {error}")))
    }

    fn migration_v8(&mut self) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS installation_rollbacks (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                instance_id   INTEGER NOT NULL,
                plugin_id     TEXT NOT NULL,
                from_version  TEXT NOT NULL,
                from_digest   BLOB NOT NULL CHECK (length(from_digest) = 32),
                target_version TEXT NOT NULL,
                target_digest BLOB NOT NULL CHECK (length(target_digest) = 32),
                reason        TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 512),
                rolled_back_at INTEGER NOT NULL CHECK (rolled_back_at >= 0)
            );
            CREATE INDEX IF NOT EXISTS installation_rollbacks_instance
                ON installation_rollbacks(instance_id, id);
            PRAGMA user_version = 8;",
        )
        .map_err(|error| StoreError::Migration(format!("v8 建立显式回滚审计表失败: {error}")))?;
        tx.commit()
            .map_err(|error| StoreError::Migration(format!("v8 提交失败: {error}")))
    }

    /// 布局存储接口。
    pub fn layout(&self) -> LayoutStore<'_> {
        LayoutStore { conn: &self.conn }
    }

    /// 脱敏能力审计存储接口。
    ///
    /// 审计记录按插入顺序写入（`list` 依 `id` 升序返回），供一致性断言/宿主查看。
    /// 值字段（detail）由调用方负责脱敏——本层只持久化已脱敏的结构化记录，语义见
    /// `floatile-services::audit`（长度/哈希，不落 secret 或完整值）。
    pub fn audit(&self) -> AuditStore<'_> {
        AuditStore { conn: &self.conn }
    }
    /// 持久化插件实例接口。
    pub fn instances(&self) -> InstanceStore<'_> {
        InstanceStore { conn: &self.conn }
    }

    /// 宿主外部数据连接及其实例授权接口。只持久化不透明 CredentialRef，不保存 secret。
    pub fn connections(&self) -> ConnectionStore<'_> {
        ConnectionStore { conn: &self.conn }
    }

    /// 宿主持有的 publisher trust、签名 key 与 anti-rollback 状态接口。
    pub fn trust(&self) -> trust::PublisherTrustStore<'_> {
        trust::PublisherTrustStore::new(&self.conn)
    }
}

pub struct ConnectionStore<'a> {
    conn: &'a Connection,
}

impl<'a> ConnectionStore<'a> {
    pub fn create(
        &self,
        provider: &str,
        account_identity: &str,
        credential: &CredentialRef,
        created_at: u64,
    ) -> Result<HostConnection, StoreError> {
        let created_at_sql = sqlite_i64(created_at, "created_at")?;
        let tx = self.conn.unchecked_transaction()?;
        let id: i64 = tx.query_row(
            "UPDATE connection_id_allocator SET next_id = next_id + 1
             WHERE singleton = 1 RETURNING next_id - 1",
            [],
            |row| row.get(0),
        )?;
        let model = HostConnection::restore(
            ConnectionId(read_positive_id(id, "connection_id")?),
            provider,
            account_identity,
            credential.clone(),
            ConnectionHealth::Unknown,
            0,
            created_at,
            created_at,
        )
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        tx.execute(
            "INSERT INTO connections (
                connection_id, provider, account_identity, credential_ref, health,
                credential_generation, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'unknown', 0, ?5, ?5)",
            rusqlite::params![
                id,
                provider,
                account_identity,
                credential.as_str(),
                created_at_sql
            ],
        )?;
        tx.commit()?;
        Ok(model)
    }

    pub fn get(&self, id: ConnectionId) -> Result<Option<HostConnection>, StoreError> {
        let id = sqlite_i64(id.0, "connection_id")?;
        let mut statement = self.conn.prepare(
            "SELECT connection_id, provider, account_identity, credential_ref, health,
                    credential_generation, created_at, updated_at
             FROM connections WHERE connection_id = ?1",
        )?;
        let mut rows = statement.query(rusqlite::params![id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        row_to_connection(row).map(Some)
    }

    pub fn list(&self) -> Result<Vec<HostConnection>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT connection_id, provider, account_identity, credential_ref, health,
                    credential_generation, created_at, updated_at
             FROM connections ORDER BY connection_id",
        )?;
        let mut rows = statement.query([])?;
        let mut result = Vec::new();
        while let Some(row) = rows.next()? {
            result.push(row_to_connection(row)?);
        }
        Ok(result)
    }

    pub fn grant(
        &self,
        instance_id: InstanceId,
        connection_id: ConnectionId,
        granted_at: u64,
    ) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO instance_connection_grants
                (instance_id, connection_id, granted_at)
             SELECT ?1, ?2, ?3
             WHERE EXISTS (SELECT 1 FROM plugin_instances WHERE instance_id = ?1)
               AND EXISTS (SELECT 1 FROM connections WHERE connection_id = ?2)",
            rusqlite::params![
                sqlite_i64(instance_id.0, "instance_id")?,
                sqlite_i64(connection_id.0, "connection_id")?,
                sqlite_i64(granted_at, "granted_at")?,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn revoke(
        &self,
        instance_id: InstanceId,
        connection_id: ConnectionId,
    ) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "DELETE FROM instance_connection_grants
             WHERE instance_id = ?1 AND connection_id = ?2",
            rusqlite::params![
                sqlite_i64(instance_id.0, "instance_id")?,
                sqlite_i64(connection_id.0, "connection_id")?,
            ],
        )?;
        Ok(changed == 1)
    }

    pub fn grants_for_instance(
        &self,
        instance_id: InstanceId,
    ) -> Result<Vec<ConnectionGrant>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT instance_id, connection_id, granted_at
             FROM instance_connection_grants WHERE instance_id = ?1 ORDER BY connection_id",
        )?;
        let mut rows =
            statement.query(rusqlite::params![sqlite_i64(instance_id.0, "instance_id")?])?;
        let mut grants = Vec::new();
        while let Some(row) = rows.next()? {
            grants.push(ConnectionGrant {
                instance_id: InstanceId(read_u64(row, 0, "instance_id")?),
                connection_id: ConnectionId(read_u64(row, 1, "connection_id")?),
                granted_at: read_u64(row, 2, "granted_at")?,
            });
        }
        Ok(grants)
    }

    pub fn rotate_credential(
        &self,
        id: ConnectionId,
        credential: &CredentialRef,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "UPDATE connections
             SET credential_ref = ?1, credential_generation = credential_generation + 1,
                 health = 'unknown', updated_at = ?2
             WHERE connection_id = ?3 AND updated_at <= ?2
               AND credential_generation < 9223372036854775807",
            rusqlite::params![
                credential.as_str(),
                sqlite_i64(updated_at, "updated_at")?,
                sqlite_i64(id.0, "connection_id")?,
            ],
        )?;
        Ok(changed == 1)
    }

    /// Persist the latest host-observed health without allowing stale probes to overwrite newer
    /// state. Secret or provider error text is intentionally not accepted by this API.
    pub fn set_health(
        &self,
        id: ConnectionId,
        health: ConnectionHealth,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "UPDATE connections SET health = ?1, updated_at = ?2
             WHERE connection_id = ?3 AND updated_at <= ?2",
            rusqlite::params![
                health.as_str(),
                sqlite_i64(updated_at, "updated_at")?,
                sqlite_i64(id.0, "connection_id")?,
            ],
        )?;
        Ok(changed == 1)
    }

    /// 只有无实例引用时才能删除 Connection，避免破坏共享连接。
    pub fn delete_unreferenced(&self, id: ConnectionId) -> Result<bool, StoreError> {
        let changed = self.conn.execute(
            "DELETE FROM connections WHERE connection_id = ?1
             AND NOT EXISTS (
                SELECT 1 FROM instance_connection_grants WHERE connection_id = ?1
             )",
            rusqlite::params![sqlite_i64(id.0, "connection_id")?],
        )?;
        Ok(changed == 1)
    }
}

/// 持久化插件实例 CRUD。
pub struct InstanceStore<'a> {
    conn: &'a Connection,
}

impl<'a> InstanceStore<'a> {
    /// 创建实例并分配永不复用的宿主全局 ID。
    ///
    /// ID 1 已由当前内建参考时钟占用，持久化第三方实例从 2 开始。
    pub fn create(
        &self,
        installation: &InstallationRef,
        config: &InstanceConfig,
        desired_state: InstanceDesiredState,
        created_at: u64,
    ) -> Result<PluginInstance, StoreError> {
        installation
            .validate()
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        let created_at_sql = sqlite_i64(created_at, "created_at")?;
        let config_json = serde_json::to_string(config)?;
        let tx = self.conn.unchecked_transaction()?;
        let instance_id_sql: i64 = tx.query_row(
            "UPDATE plugin_instance_id_allocator
             SET next_id = next_id + 1
             WHERE singleton = 1
             RETURNING next_id - 1",
            [],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT INTO plugin_instances (
                instance_id, plugin_id, installation_version, installation_digest,
                config_json, desired_state, generation, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)",
            rusqlite::params![
                instance_id_sql,
                installation.plugin().0,
                installation.version(),
                installation.digest().as_bytes().as_slice(),
                config_json,
                desired_state.as_str(),
                created_at_sql,
            ],
        )?;
        tx.commit()?;

        let instance_id = u64::try_from(instance_id_sql).map_err(|_| {
            StoreError::Corrupt(format!("instance_id 不得为负数: {instance_id_sql}"))
        })?;
        PluginInstance::restore(
            InstanceId(instance_id),
            installation.clone(),
            config.clone(),
            desired_state,
            0,
            created_at,
            created_at,
        )
        .map_err(|error| StoreError::Corrupt(error.to_string()))
    }

    /// 按全局实例 ID 读取。
    pub fn get(&self, instance_id: InstanceId) -> Result<Option<PluginInstance>, StoreError> {
        let instance_id = sqlite_i64(instance_id.0, "instance_id")?;
        let mut statement = self.conn.prepare(
            "SELECT
                instance_id, plugin_id, installation_version, installation_digest,
                config_json, desired_state, generation, created_at, updated_at
             FROM plugin_instances
             WHERE instance_id = ?1",
        )?;
        let mut rows = statement.query(rusqlite::params![instance_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        row_to_instance(row).map(Some)
    }

    /// 按实例 ID 稳定枚举全部实例。
    pub fn list(&self) -> Result<Vec<PluginInstance>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT
                instance_id, plugin_id, installation_version, installation_digest,
                config_json, desired_state, generation, created_at, updated_at
             FROM plugin_instances
             ORDER BY instance_id ASC",
        )?;
        let mut rows = statement.query([])?;
        let mut instances = Vec::new();
        while let Some(row) = rows.next()? {
            instances.push(row_to_instance(row)?);
        }
        Ok(instances)
    }

    /// 原子替换 canonical config；过期时间戳不会覆盖较新的记录。
    pub fn update_config(
        &self,
        instance_id: InstanceId,
        config: &InstanceConfig,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        let instance_id = sqlite_i64(instance_id.0, "instance_id")?;
        let updated_at = sqlite_i64(updated_at, "updated_at")?;
        let config_json = serde_json::to_string(config)?;
        let changed = self.conn.execute(
            "UPDATE plugin_instances
             SET config_json = ?1, updated_at = ?2
             WHERE instance_id = ?3 AND updated_at <= ?2",
            rusqlite::params![config_json, updated_at, instance_id],
        )?;
        Ok(changed == 1)
    }

    /// 更新宿主重启后应恢复的运行意图；不把 observed runtime 状态持久化。
    pub fn set_desired_state(
        &self,
        instance_id: InstanceId,
        desired_state: InstanceDesiredState,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        let instance_id = sqlite_i64(instance_id.0, "instance_id")?;
        let updated_at = sqlite_i64(updated_at, "updated_at")?;
        let changed = self.conn.execute(
            "UPDATE plugin_instances
             SET desired_state = ?1, updated_at = ?2
             WHERE instance_id = ?3 AND updated_at <= ?2",
            rusqlite::params![desired_state.as_str(), updated_at, instance_id],
        )?;
        Ok(changed == 1)
    }

    /// Atomically rebinds a stopped instance to a verified historical installation and audits it.
    pub fn rollback_installation(
        &self,
        instance_id: InstanceId,
        current: &InstallationRef,
        target: &InstallationRef,
        reason: &str,
        updated_at: u64,
    ) -> Result<bool, StoreError> {
        if reason.is_empty() || reason.len() > 512 {
            return Err(StoreError::Corrupt(
                "rollback reason 必须为 1..=512 字节".to_owned(),
            ));
        }
        current
            .validate()
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        target
            .validate()
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        let instance_id_sql = sqlite_i64(instance_id.0, "instance_id")?;
        let updated_at_sql = sqlite_i64(updated_at, "updated_at")?;
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE plugin_instances
             SET installation_version = ?1, installation_digest = ?2, updated_at = ?3
             WHERE instance_id = ?4
               AND plugin_id = ?5
               AND installation_version = ?6
               AND installation_digest = ?7
               AND desired_state = 'stopped'
               AND updated_at <= ?3",
            rusqlite::params![
                target.version(),
                target.digest().as_bytes().as_slice(),
                updated_at_sql,
                instance_id_sql,
                current.plugin().0,
                current.version(),
                current.digest().as_bytes().as_slice(),
            ],
        )?;
        if changed == 1 {
            tx.execute(
                "INSERT INTO installation_rollbacks (
                    instance_id, plugin_id, from_version, from_digest,
                    target_version, target_digest, reason, rolled_back_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    instance_id_sql,
                    current.plugin().0,
                    current.version(),
                    current.digest().as_bytes().as_slice(),
                    target.version(),
                    target.digest().as_bytes().as_slice(),
                    reason,
                    updated_at_sql,
                ],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }

    /// 启动或重启前推进 generation；迟到异步结果必须匹配该值才能投递。
    pub fn advance_generation(
        &self,
        instance_id: InstanceId,
        updated_at: u64,
    ) -> Result<Option<u64>, StoreError> {
        let instance_id = sqlite_i64(instance_id.0, "instance_id")?;
        let updated_at = sqlite_i64(updated_at, "updated_at")?;
        let generation: Option<i64> = self
            .conn
            .query_row(
                "UPDATE plugin_instances
                 SET generation = generation + 1, updated_at = ?1
                 WHERE instance_id = ?2
                   AND updated_at <= ?1
                   AND generation < 9223372036854775807
                 RETURNING generation",
                rusqlite::params![updated_at, instance_id],
                |row| row.get(0),
            )
            .optional()?;
        generation
            .map(|value| {
                u64::try_from(value)
                    .map_err(|_| StoreError::Corrupt(format!("generation 不得为负数: {value}")))
            })
            .transpose()
    }

    /// 删除实例及其实例所有的布局；Installation 和历史审计保留。
    pub fn delete(&self, instance_id: InstanceId) -> Result<bool, StoreError> {
        let instance_id = sqlite_i64(instance_id.0, "instance_id")?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM instance_connection_grants WHERE instance_id = ?1",
            rusqlite::params![instance_id],
        )?;
        let changed = tx.execute(
            "DELETE FROM plugin_instances WHERE instance_id = ?1",
            rusqlite::params![instance_id],
        )?;
        if changed == 1 {
            tx.execute(
                "DELETE FROM layout WHERE instance_id = ?1",
                rusqlite::params![instance_id],
            )?;
        }
        tx.commit()?;
        Ok(changed == 1)
    }
}

/// 一条已脱敏的能力审计记录（`audit_log` 表行）。
///
/// `detail` 必须已脱敏（长度/哈希摘要，不落 secret 或完整 State/Storage 值）。
/// `decision` 为 `allow` 或 `deny`；`reason` 仅拒绝时存在。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    pub plugin: String,
    pub instance: u64,
    pub capability: String,
    pub decision: String,
    pub reason: Option<String>,
    pub detail: String,
    pub unix_ts: u64,
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

/// 脱敏审计记录存取（audit_log 表追加写 + 顺序读）。
pub struct AuditStore<'a> {
    conn: &'a Connection,
}

impl<'a> AuditStore<'a> {
    /// 追加一条已脱敏审计记录。
    pub fn record(&self, record: &AuditRecord) -> Result<(), StoreError> {
        if record.decision != "allow" && record.decision != "deny" {
            return Err(StoreError::Corrupt(format!(
                "audit decision 必须为 allow/deny，实际 {}",
                record.decision
            )));
        }
        let instance = sqlite_i64(record.instance, "instance")?;
        let ts = sqlite_i64(record.unix_ts, "unix_ts")?;
        self.conn.execute(
            "INSERT INTO audit_log
                (unix_ts, plugin, instance, capability, decision, reason, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                ts,
                record.plugin,
                instance,
                record.capability,
                record.decision,
                record.reason,
                record.detail,
            ],
        )?;
        Ok(())
    }

    /// 按插入顺序列出全部审计记录（id 升序）。
    pub fn list(&self) -> Result<Vec<AuditRecord>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT
                unix_ts, plugin, instance, capability, decision, reason, detail
             FROM audit_log ORDER BY id ASC",
        )?;
        let mut rows = stmt.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(AuditRecord {
                unix_ts: read_u64(row, 0, "unix_ts")?,
                plugin: row.get(1)?,
                instance: read_u64(row, 2, "instance")?,
                capability: row.get(3)?,
                decision: row.get(4)?,
                reason: row.get(5)?,
                detail: row.get(6)?,
            });
        }
        Ok(records)
    }
}

fn row_to_instance(row: &rusqlite::Row<'_>) -> Result<PluginInstance, StoreError> {
    let instance_id = InstanceId(read_u64(row, 0, "instance_id")?);
    let plugin = PluginId(row.get(1)?);
    let version = row.get::<_, String>(2)?;
    let digest = row.get::<_, Vec<u8>>(3)?;
    let digest: [u8; 32] = digest.try_into().map_err(|bytes: Vec<u8>| {
        StoreError::Corrupt(format!(
            "installation_digest 必须为 32 字节，实际为 {}",
            bytes.len()
        ))
    })?;
    let installation =
        InstallationRef::new(plugin, version, InstallationDigest::from_bytes(digest))
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    let config_json = row.get::<_, String>(4)?;
    let config: InstanceConfig = serde_json::from_str(&config_json)
        .map_err(|error| StoreError::Corrupt(format!("实例配置无效: {error}")))?;
    let desired_state = InstanceDesiredState::parse(&row.get::<_, String>(5)?)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    let generation = read_u64(row, 6, "generation")?;
    let created_at = read_u64(row, 7, "created_at")?;
    let updated_at = read_u64(row, 8, "updated_at")?;
    PluginInstance::restore(
        instance_id,
        installation,
        config,
        desired_state,
        generation,
        created_at,
        updated_at,
    )
    .map_err(|error| StoreError::Corrupt(error.to_string()))
}

fn row_to_connection(row: &rusqlite::Row<'_>) -> Result<HostConnection, StoreError> {
    let id = ConnectionId(read_u64(row, 0, "connection_id")?);
    let provider = row.get::<_, String>(1)?;
    let account_identity = row.get::<_, String>(2)?;
    let credential = CredentialRef::new(row.get::<_, String>(3)?)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    let health = ConnectionHealth::parse(&row.get::<_, String>(4)?)
        .map_err(|error| StoreError::Corrupt(error.to_string()))?;
    HostConnection::restore(
        id,
        provider,
        account_identity,
        credential,
        health,
        read_u64(row, 5, "credential_generation")?,
        read_u64(row, 6, "created_at")?,
        read_u64(row, 7, "updated_at")?,
    )
    .map_err(|error| StoreError::Corrupt(error.to_string()))
}

fn read_positive_id(value: i64, field: &str) -> Result<u64, StoreError> {
    let value =
        u64::try_from(value).map_err(|_| StoreError::Corrupt(format!("{field} 不得为负数")))?;
    if value == 0 {
        return Err(StoreError::Corrupt(format!("{field} 不得为 0")));
    }
    Ok(value)
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
        instance_id: InstanceId(read_u64(row, 0, "instance_id")?),
        plugin_id: PluginId(row.get(1)?),
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
        // migrate() 继续跑到最新 schema(v3 追加 audit_log 表,不影响 layout)。
        assert_eq!(version, SCHEMA_VERSION);
    }

    fn v2_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store.migration_v2().unwrap();
        store
    }

    fn sample_audit(plugin: &str, instance: u64, capability: &str, decision: &str) -> AuditRecord {
        AuditRecord {
            plugin: plugin.into(),
            instance,
            capability: capability.into(),
            decision: decision.into(),
            reason: if decision == "deny" {
                Some("deny-by-default".into())
            } else {
                None
            },
            detail: "message len=5".into(),
            unix_ts: 1_700_000_000,
        }
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
    fn migration_v3_adds_audit_log_table() {
        let store = open(":memory:").unwrap();
        let mut stmt = store.conn.prepare("PRAGMA table_info(audit_log)").unwrap();
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "id",
            "unix_ts",
            "plugin",
            "instance",
            "capability",
            "decision",
            "reason",
            "detail",
        ] {
            assert!(
                columns.iter().any(|c| c == expected),
                "缺列 {expected}: {columns:?}"
            );
        }
    }

    #[test]
    fn migration_v3_forward_from_v2_is_append_only() {
        let mut store = v2_store();
        // 先写入一条 v2 布局,证明 v3 不破坏 layout。
        store
            .conn
            .execute(
                "INSERT INTO layout (
                    instance_id, plugin_id, monitor_key, x, y, w, h,
                    physical_w, physical_h, scale_factor, lost_monitor,
                    z, mode, version, updated_at
                 ) VALUES (7, 'dev.floatile.clock', 'edid', 1, 2, 260, 120,
                    260, 120, 1.0, 0, 1, 'edit', 1, 1700000000)",
                [],
            )
            .unwrap();
        store.migrate().unwrap();
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(store.layout().get(7).unwrap().is_some());
        // audit_log 表可写。
        store
            .audit()
            .record(&sample_audit("dev.floatile.clock", 7, "log:write", "allow"))
            .unwrap();
        assert_eq!(store.audit().list().unwrap().len(), 1);
    }

    #[test]
    fn audit_record_and_list_preserve_order_and_fields() {
        let store = open(":memory:").unwrap();
        store
            .audit()
            .record(&sample_audit("dev.floatile.clock", 1, "log:write", "allow"))
            .unwrap();
        store
            .audit()
            .record(&sample_audit(
                "dev.floatile.clock",
                1,
                "metrics:memory",
                "deny",
            ))
            .unwrap();
        let list = store.audit().list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].capability, "log:write");
        assert_eq!(list[0].decision, "allow");
        assert_eq!(list[1].capability, "metrics:memory");
        assert_eq!(list[1].decision, "deny");
        assert_eq!(list[1].reason.as_deref(), Some("deny-by-default"));
    }

    #[test]
    fn audit_rejects_invalid_decision() {
        let store = open(":memory:").unwrap();
        let mut bad = sample_audit("p", 1, "log:write", "maybe");
        bad.decision = "maybe".into();
        assert!(matches!(
            store.audit().record(&bad),
            Err(StoreError::Corrupt(_))
        ));
    }
    fn installation() -> InstallationRef {
        InstallationRef::new(
            PluginId("dev.floatile.clock".into()),
            "1.2.3",
            InstallationDigest::from_bytes([0x2a; 32]),
        )
        .unwrap()
    }

    fn historical_installation() -> InstallationRef {
        InstallationRef::new(
            PluginId("dev.floatile.clock".into()),
            "1.0.0",
            InstallationDigest::from_bytes([0x19; 32]),
        )
        .unwrap()
    }

    fn config(label: &str) -> InstanceConfig {
        InstanceConfig::new(serde_json::json!({ "label": label })).unwrap()
    }

    fn v3_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store.migration_v2().unwrap();
        store.migration_v3().unwrap();
        store
    }

    #[test]
    fn migration_v4_preserves_existing_data_and_allocates_after_builtin() {
        let mut store = v3_store();
        store.layout().save(&sample(1)).unwrap();
        store
            .audit()
            .record(&sample_audit(
                "dev.floatile.clock",
                1,
                "timer:schedule",
                "allow",
            ))
            .unwrap();

        store.migrate().unwrap();

        assert!(store.layout().get(1).unwrap().is_some());
        assert_eq!(store.audit().list().unwrap().len(), 1);
        let instance = store
            .instances()
            .create(
                &installation(),
                &config("first"),
                InstanceDesiredState::Stopped,
                1_700_000_100,
            )
            .unwrap();
        assert_eq!(instance.id(), InstanceId(2));
    }

    #[test]
    fn rollback_rebinds_stopped_instance_and_appends_audit_atomically() {
        let store = open(":memory:").unwrap();
        let instance = store
            .instances()
            .create(
                &installation(),
                &config("rollback"),
                InstanceDesiredState::Stopped,
                10,
            )
            .unwrap();
        assert!(
            store
                .instances()
                .rollback_installation(
                    instance.id(),
                    instance.installation(),
                    &historical_installation(),
                    "regression in 1.2.3",
                    11,
                )
                .unwrap()
        );
        let restored = store.instances().get(instance.id()).unwrap().unwrap();
        assert_eq!(restored.installation(), &historical_installation());
        let count: u32 = store
            .conn
            .query_row("SELECT COUNT(*) FROM installation_rollbacks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);

        store
            .instances()
            .set_desired_state(instance.id(), InstanceDesiredState::Running, 12)
            .unwrap();
        assert!(
            !store
                .instances()
                .rollback_installation(
                    instance.id(),
                    &historical_installation(),
                    &installation(),
                    "must stop first",
                    13,
                )
                .unwrap()
        );
        let count: u32 = store
            .conn
            .query_row("SELECT COUNT(*) FROM installation_rollbacks", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn migration_v4_failure_rolls_back_allocator_and_version() {
        let mut store = v3_store();
        store
            .conn
            .execute_batch("CREATE TABLE plugin_instances (broken INTEGER);")
            .unwrap();

        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 3);
        let allocator_exists: u32 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'plugin_instance_id_allocator'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(allocator_exists, 0);
    }

    #[test]
    fn same_installation_instances_have_independent_state() {
        let store = open(":memory:").unwrap();
        let installation = installation();
        let first = store
            .instances()
            .create(
                &installation,
                &config("first"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        let second = store
            .instances()
            .create(
                &installation,
                &config("second"),
                InstanceDesiredState::Running,
                100,
            )
            .unwrap();
        assert_eq!(first.id(), InstanceId(2));
        assert_eq!(second.id(), InstanceId(3));

        assert!(
            store
                .instances()
                .update_config(first.id(), &config("updated"), 110)
                .unwrap()
        );
        assert!(
            store
                .instances()
                .set_desired_state(first.id(), InstanceDesiredState::Running, 111)
                .unwrap()
        );
        assert_eq!(
            store
                .instances()
                .advance_generation(first.id(), 112)
                .unwrap(),
            Some(1)
        );
        assert!(
            !store
                .instances()
                .update_config(first.id(), &config("stale"), 109)
                .unwrap()
        );

        let first = store.instances().get(first.id()).unwrap().unwrap();
        let second = store.instances().get(second.id()).unwrap().unwrap();
        assert_eq!(
            first.config().to_value(),
            serde_json::json!({"label": "updated"})
        );
        assert_eq!(first.desired_state(), InstanceDesiredState::Running);
        assert_eq!(first.generation(), 1);
        assert_eq!(
            second.config().to_value(),
            serde_json::json!({"label": "second"})
        );
        assert_eq!(second.desired_state(), InstanceDesiredState::Running);
        assert_eq!(second.generation(), 0);
        assert_eq!(store.instances().list().unwrap().len(), 2);
    }

    #[test]
    fn deleting_instance_removes_owned_layout_but_not_peer() {
        let store = open(":memory:").unwrap();
        let first = store
            .instances()
            .create(
                &installation(),
                &config("first"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        let second = store
            .instances()
            .create(
                &installation(),
                &config("second"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        store.layout().save(&sample(first.id().0)).unwrap();

        assert!(store.instances().delete(first.id()).unwrap());
        assert!(store.instances().get(first.id()).unwrap().is_none());
        assert!(store.layout().get(first.id().0).unwrap().is_none());
        assert!(store.instances().get(second.id()).unwrap().is_some());
        assert!(!store.instances().delete(first.id()).unwrap());
    }

    #[test]
    fn instances_persist_across_reopen_without_reusing_ids() {
        let path = TempDb::new();
        let first_id;
        {
            let store = open(&path.0).unwrap();
            let first = store
                .instances()
                .create(
                    &installation(),
                    &config("persistent"),
                    InstanceDesiredState::Running,
                    100,
                )
                .unwrap();
            first_id = first.id();
            assert!(store.instances().delete(first_id).unwrap());
        }
        {
            let store = open(&path.0).unwrap();
            let second = store
                .instances()
                .create(
                    &installation(),
                    &config("replacement"),
                    InstanceDesiredState::Stopped,
                    200,
                )
                .unwrap();
            assert!(second.id().0 > first_id.0);
            assert_eq!(
                second.config().to_value(),
                serde_json::json!({"label": "replacement"})
            );
        }
    }

    #[test]
    fn corrupt_instance_row_is_rejected() {
        let store = open(":memory:").unwrap();
        store
            .conn
            .execute(
                "INSERT INTO plugin_instances (
                    instance_id, plugin_id, installation_version, installation_digest,
                    config_json, desired_state, generation, created_at, updated_at
                 ) VALUES (2, 'dev.floatile.clock', '1.0.0', ?1, '[]', 'stopped', 0, 1, 1)",
                rusqlite::params![[0u8; 32].as_slice()],
            )
            .unwrap();
        assert!(matches!(
            store.instances().get(InstanceId(2)),
            Err(StoreError::Corrupt(_))
        ));
    }

    fn credential(name: &str) -> CredentialRef {
        CredentialRef::new(format!("cred://openai/{name}")).unwrap()
    }

    fn v4_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store.migration_v2().unwrap();
        store.migration_v3().unwrap();
        store.migration_v4().unwrap();
        store
    }

    #[test]
    fn migration_v5_failure_rolls_back_connection_allocator_and_version() {
        let mut store = v4_store();
        store
            .conn
            .execute_batch("CREATE TABLE connections (broken INTEGER);")
            .unwrap();

        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 4);
        let allocator_exists: u32 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'connection_id_allocator'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(allocator_exists, 0);
    }

    fn v5_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store.migration_v2().unwrap();
        store.migration_v3().unwrap();
        store.migration_v4().unwrap();
        store.migration_v5().unwrap();
        store
    }

    #[test]
    fn migration_v6_failure_rolls_back_all_trust_tables_and_version() {
        let mut store = v5_store();
        store
            .conn
            .execute_batch("CREATE INDEX publisher_trust ON layout(instance_id);")
            .unwrap();

        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 5);
        for table in ["publisher_trust", "accepted_packages"] {
            let exists: u32 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 0, "{table} leaked from failed migration");
        }
    }

    fn v6_store() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        let mut store = Store { conn };
        store.migration_v1().unwrap();
        store.migration_v2().unwrap();
        store.migration_v3().unwrap();
        store.migration_v4().unwrap();
        store.migration_v5().unwrap();
        store.migration_v6().unwrap();
        store
    }

    #[test]
    fn migration_v7_failure_rolls_back_pending_table_and_version() {
        let mut store = v6_store();
        store
            .conn
            .execute_batch("CREATE INDEX pending_installations ON layout(instance_id);")
            .unwrap();
        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 6);
        let exists: u32 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'pending_installations'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[test]
    fn migration_v8_failure_rolls_back_audit_table_and_version() {
        let mut store = v6_store();
        store.migration_v7().unwrap();
        store
            .conn
            .execute_batch("CREATE INDEX installation_rollbacks ON layout(instance_id);")
            .unwrap();
        assert!(matches!(store.migrate(), Err(StoreError::Migration(_))));
        let version: u32 = store
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
        let exists: u32 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'installation_rollbacks'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0);
    }

    #[test]
    fn connection_grants_are_instance_scoped_and_shared_connection_survives_deletion() {
        let store = open(":memory:").unwrap();
        let first = store
            .instances()
            .create(
                &installation(),
                &config("first"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        let second = store
            .instances()
            .create(
                &installation(),
                &config("second"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        let connection = store
            .connections()
            .create("openai", "account@example.com", &credential("primary"), 100)
            .unwrap();

        assert!(
            store
                .connections()
                .grant(first.id(), connection.id(), 101)
                .unwrap()
        );
        assert!(
            store
                .connections()
                .grant(second.id(), connection.id(), 102)
                .unwrap()
        );
        assert!(
            !store
                .connections()
                .grant(second.id(), connection.id(), 103)
                .unwrap()
        );
        assert!(
            !store
                .connections()
                .delete_unreferenced(connection.id())
                .unwrap()
        );

        assert!(store.instances().delete(first.id()).unwrap());
        assert!(
            store
                .connections()
                .grants_for_instance(first.id())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .connections()
                .grants_for_instance(second.id())
                .unwrap()
                .len(),
            1
        );
        assert!(store.connections().get(connection.id()).unwrap().is_some());
        assert!(
            store
                .connections()
                .revoke(second.id(), connection.id())
                .unwrap()
        );
        assert!(
            store
                .connections()
                .delete_unreferenced(connection.id())
                .unwrap()
        );
    }

    #[test]
    fn credential_rotation_only_persists_reference_and_advances_generation() {
        let store = open(":memory:").unwrap();
        let connection = store
            .connections()
            .create("openai", "account", &credential("old"), 100)
            .unwrap();
        assert!(
            store
                .connections()
                .rotate_credential(connection.id(), &credential("new"), 110)
                .unwrap()
        );
        assert!(
            !store
                .connections()
                .rotate_credential(connection.id(), &credential("stale"), 109)
                .unwrap()
        );
        let restored = store.connections().get(connection.id()).unwrap().unwrap();
        assert_eq!(restored.credential().as_str(), "cred://openai/new");
        assert_eq!(restored.credential_generation(), 1);
        assert_eq!(restored.health(), ConnectionHealth::Unknown);

        let columns: Vec<String> = store
            .conn
            .prepare("PRAGMA table_info(connections)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column.contains("secret")));
    }

    #[test]
    fn connection_health_rejects_stale_probe_updates() {
        let store = open(":memory:").unwrap();
        let connection = store
            .connections()
            .create("openai", "account", &credential("health"), 100)
            .unwrap();
        assert!(
            store
                .connections()
                .set_health(connection.id(), ConnectionHealth::Healthy, 120)
                .unwrap()
        );
        assert!(
            !store
                .connections()
                .set_health(connection.id(), ConnectionHealth::Unavailable, 119)
                .unwrap()
        );
        let restored = store.connections().get(connection.id()).unwrap().unwrap();
        assert_eq!(restored.health(), ConnectionHealth::Healthy);
        assert_eq!(restored.updated_at(), 120);
    }

    #[test]
    fn connection_grant_rejects_unknown_instance_or_connection() {
        let store = open(":memory:").unwrap();
        let instance = store
            .instances()
            .create(
                &installation(),
                &config("known"),
                InstanceDesiredState::Stopped,
                100,
            )
            .unwrap();
        let connection = store
            .connections()
            .create("openai", "account", &credential("known"), 100)
            .unwrap();
        assert!(
            !store
                .connections()
                .grant(InstanceId(999), connection.id(), 100)
                .unwrap()
        );
        assert!(
            !store
                .connections()
                .grant(instance.id(), ConnectionId(999), 100)
                .unwrap()
        );
    }

    #[test]
    fn audit_persists_across_reopen() {
        let path = TempDb::new();
        {
            let store = open(&path.0).unwrap();
            store
                .audit()
                .record(&sample_audit("a", 1, "log:write", "allow"))
                .unwrap();
        }
        {
            let store = open(&path.0).unwrap();
            let list = store.audit().list().unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].plugin, "a");
            assert_eq!(list[0].detail, "message len=5");
        }
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
