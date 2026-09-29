//! 供应商状态修复（幂等）
//!
//! 见 `docs/DESIGN-routing-mode.md` §6.2（按 `category == "official"` 认定官方供应商 +
//! 重复官方行合并）与 §8（一次性 DB 修复）。
//!
//! 背景：官方供应商曾经既可能以「种子 id」（`codex-official`）存在，也可能以
//! 历史手工行（`default`）存在，两行 `category` 都是 `"official"`。旧代码用
//! **id** 判断官方身份，于是切回官方登录时可能写到过期的那一行上。
//!
//! 本模块按 **登录新鲜度**（`settings_config.auth.last_refresh`）决定谁持有登录态：
//! 种子行保留 id（接管等按 id 查找的路径依赖它），但把最新登录态合并进种子行；
//! 其余重复行归档到 `~/.cc-switch/backups/provider-official-merge-v1/<时间戳>/`
//! 后再删除，因此"较新的登录"永不被丢弃。
//!
//! 幂等性：
//! - 没有重复行时不写归档、不改库；
//! - 合并只会在**种子行存在**时发生 —— 官方种子行永不被删除；
//! - `is_current` 去重、队列清理都收敛到固定状态，重复执行结果一致。

use crate::database::dao::providers_seed::OFFICIAL_SEEDS;
use crate::database::{lock_conn, Database};
use crate::error::AppError;
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::PathBuf;

/// 归档目录名：`~/.cc-switch/backups/<这个名字>/<时间戳>/providers.json`
pub(crate) const OFFICIAL_MERGE_ARCHIVE_DIR: &str = "provider-official-merge-v1";

/// 修复范围。破坏性操作只在迁移时执行一次，日常启动只做收敛性合并。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RepairScope {
    /// 合并同一 app 下重复的 `category = "official"` 行（§6.2）
    pub merge_duplicates: bool,
    /// 为未启用故障转移的 app 清空 `in_failover_queue`（§8，仅迁移时执行）
    pub clear_stale_failover_queue: bool,
}

impl RepairScope {
    /// 升级迁移：合并 + 清理失效队列（一次性）
    pub(crate) const MIGRATION: Self = Self {
        merge_duplicates: true,
        clear_stale_failover_queue: true,
    };

    /// 启动 / 导入后的常驻修复：只合并不做破坏性清理，可反复执行
    pub(crate) const STARTUP: Self = Self {
        merge_duplicates: true,
        clear_stale_failover_queue: false,
    };
}

/// 一次被合并掉的 app 记录（仅用于日志与测试断言）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergedOfficialApp {
    pub app_type: String,
    /// 保留下来的行 id（存在官方种子时即种子 id）
    pub survivor_id: String,
    /// 被归档并删除的重复行 id
    pub removed_ids: Vec<String>,
    /// 被采纳进幸存行的登录时间（原始字符串）
    pub adopted_login_at: Option<String>,
}

/// 修复结果汇总。
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderRepairReport {
    pub merged_apps: Vec<MergedOfficialApp>,
    /// 归档文件路径（仅当确有行被合并时写入）
    pub archived_path: Option<String>,
    /// 被清掉的「多余 is_current 标记」数量
    pub cleared_current_flags: usize,
    /// 被清空故障转移队列的 app
    pub cleared_failover_queue_apps: Vec<String>,
}

impl ProviderRepairReport {
    pub(crate) fn is_noop(&self) -> bool {
        self.merged_apps.is_empty()
            && self.cleared_current_flags == 0
            && self.cleared_failover_queue_apps.is_empty()
    }
}

/// 一行官方供应商（含归档所需的原始快照）。
#[derive(Debug, Clone)]
struct OfficialRow {
    id: String,
    settings_config: Value,
    created_at: Option<i64>,
    is_current: bool,
    in_failover_queue: bool,
    /// 完整行快照，删除前归档
    raw: Value,
}

impl OfficialRow {
    /// 登录新鲜度键：`auth.last_refresh` 解析出的毫秒时间戳。
    fn login_millis(&self) -> Option<i64> {
        self.settings_config
            .get("auth")
            .and_then(|auth| auth.get("last_refresh"))
            .and_then(parse_login_timestamp)
    }

    fn login_at(&self) -> Option<String> {
        self.settings_config
            .get("auth")
            .and_then(|auth| auth.get("last_refresh"))
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// 排序键：先按登录时间，再按创建时间，最后用 id 兜底以保证确定性。
    fn freshness_key(&self) -> (i64, i64, std::cmp::Reverse<String>) {
        (
            self.login_millis().unwrap_or(i64::MIN),
            self.created_at.unwrap_or(i64::MIN),
            std::cmp::Reverse(self.id.clone()),
        )
    }
}

impl Database {
    /// 修复供应商状态（幂等）。见模块头注释与 `RepairScope`。
    pub(crate) fn repair_provider_state(
        &self,
        scope: RepairScope,
    ) -> Result<ProviderRepairReport, AppError> {
        let conn = lock_conn!(self.conn);
        repair_provider_state_on_conn(&conn, scope)
    }
}

/// 在指定连接上执行修复（供 schema 迁移与测试复用）。
pub(crate) fn repair_provider_state_on_conn(
    conn: &Connection,
    scope: RepairScope,
) -> Result<ProviderRepairReport, AppError> {
    let mut report = ProviderRepairReport::default();

    // 防御：迁移链会跑在历史/精简库上（测试里也有只含部分表的快照），
    // 结构不完整时静默跳过，绝不因为修复本身让升级失败。
    if !Database::table_exists(conn, "providers")?
        || !Database::has_column(conn, "providers", "category")?
        || !Database::has_column(conn, "providers", "is_current")?
    {
        log::debug!("○ providers 表结构不完整，跳过供应商状态修复");
        return Ok(report);
    }

    if scope.merge_duplicates {
        let mut archived_rows: Vec<Value> = Vec::new();
        for app_type in app_types_with_official_rows(conn)? {
            let rows = load_official_rows(conn, &app_type)?;
            if rows.len() < 2 {
                continue;
            }
            match merge_official_rows(conn, &app_type, &rows, &mut archived_rows)? {
                Some(merged) => {
                    log::info!(
                        "✓ 合并重复官方供应商 [{app_type}]：保留 {}，归档 {:?}",
                        merged.survivor_id,
                        merged.removed_ids
                    );
                    report.merged_apps.push(merged);
                }
                None => {
                    log::warn!(
                        "○ [{app_type}] 有 {} 个官方供应商但没有内置种子行，跳过合并（人工确认后再处理）",
                        rows.len()
                    );
                }
            }
        }

        if !archived_rows.is_empty() {
            report.archived_path = Some(archive_removed_rows(&archived_rows)?);
        }
    }

    report.cleared_current_flags = enforce_single_current_per_app(conn)?;

    if scope.clear_stale_failover_queue && Database::table_exists(conn, "proxy_config")? {
        report.cleared_failover_queue_apps = clear_failover_queues_without_failover(conn)?;
    }

    Ok(report)
}

fn app_types_with_official_rows(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn
        .prepare("SELECT DISTINCT app_type FROM providers WHERE category = 'official' ORDER BY app_type")
        .map_err(|e| AppError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;
    Ok(rows)
}

fn load_official_rows(conn: &Connection, app_type: &str) -> Result<Vec<OfficialRow>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, settings_config, website_url, created_at, sort_index, notes, icon,
                    icon_color, meta, is_current, in_failover_queue
             FROM providers
             WHERE app_type = ?1 AND category = 'official'
             ORDER BY id ASC",
        )
        .map_err(|e| AppError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![app_type], |row| {
            let id: String = row.get(0)?;
            let name: String = row.get(1)?;
            let settings_config_raw: String = row.get(2)?;
            let website_url: Option<String> = row.get(3)?;
            let created_at: Option<i64> = row.get(4)?;
            let sort_index: Option<i64> = row.get(5)?;
            let notes: Option<String> = row.get(6)?;
            let icon: Option<String> = row.get(7)?;
            let icon_color: Option<String> = row.get(8)?;
            let meta_raw: String = row.get(9)?;
            let is_current: bool = row.get(10)?;
            let in_failover_queue: bool = row.get(11)?;

            let settings_config: Value =
                serde_json::from_str(&settings_config_raw).unwrap_or(Value::Null);
            let meta: Value = serde_json::from_str(&meta_raw).unwrap_or(Value::Null);

            Ok(OfficialRow {
                raw: serde_json::json!({
                    "id": id.clone(),
                    "appType": app_type,
                    "name": name.clone(),
                    "settingsConfig": settings_config.clone(),
                    "websiteUrl": website_url.clone(),
                    "createdAt": created_at,
                    "sortIndex": sort_index,
                    "notes": notes.clone(),
                    "icon": icon.clone(),
                    "iconColor": icon_color.clone(),
                    "meta": meta.clone(),
                    "isCurrent": is_current,
                    "inFailoverQueue": in_failover_queue,
                    "category": "official",
                }),
                id,
                settings_config,
                created_at,
                is_current,
                in_failover_queue,
            })
        })
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;

    Ok(rows)
}

/// 合并一个 app 下重复的官方行。
///
/// 返回 `None` 表示没有可用的内置种子行 —— 此时不做任何删除，交由人工处理。
fn merge_official_rows(
    conn: &Connection,
    app_type: &str,
    rows: &[OfficialRow],
    archived_rows: &mut Vec<Value>,
) -> Result<Option<MergedOfficialApp>, AppError> {
    let Some(canonical_id) = canonical_seed_id(app_type) else {
        return Ok(None);
    };
    let Some(survivor) = rows.iter().find(|row| row.id == canonical_id) else {
        return Ok(None);
    };

    // 登录最新的一行持有真实登录态（可能是非种子行）
    let freshest = rows
        .iter()
        .max_by(|a, b| a.freshness_key().cmp(&b.freshness_key()))
        .expect("rows is non-empty");

    let merged_settings = if freshest.id == survivor.id {
        survivor.settings_config.clone()
    } else {
        merge_settings_config(&freshest.settings_config, &survivor.settings_config)
    };

    let adopt_current = rows
        .iter()
        .any(|row| row.id != survivor.id && row.is_current);
    let adopt_queue = rows
        .iter()
        .any(|row| row.id != survivor.id && row.in_failover_queue);
    let is_current = survivor.is_current || adopt_current;
    let in_failover_queue = survivor.in_failover_queue || adopt_queue;

    let mut removed_ids = Vec::new();
    for row in rows.iter().filter(|row| row.id != survivor.id) {
        archived_rows.push(row.raw.clone());
        conn.execute(
            "DELETE FROM providers WHERE id = ?1 AND app_type = ?2",
            params![row.id, app_type],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        removed_ids.push(row.id.clone());
    }

    conn.execute(
        "UPDATE providers SET settings_config = ?1, is_current = ?2, in_failover_queue = ?3
         WHERE id = ?4 AND app_type = ?5",
        params![
            serde_json::to_string(&merged_settings).map_err(|e| AppError::Database(format!(
                "Failed to serialize merged settings_config: {e}"
            )))?,
            is_current,
            in_failover_queue,
            survivor.id,
            app_type,
        ],
    )
    .map_err(|e| AppError::Database(e.to_string()))?;

    // 重复行被删除后，其端点与健康记录一并清理（外键 CASCADE 只在
    // `PRAGMA foreign_keys=ON` 时生效，这里显式兜底）；只保留幸存行的数据
    for table in ["provider_endpoints", "provider_health"] {
        if !Database::table_exists(conn, table)? {
            continue;
        }
        conn.execute(
            &format!(
                "DELETE FROM {table} WHERE app_type = ?1 AND provider_id NOT IN (
                     SELECT id FROM providers WHERE app_type = ?1
                 )"
            ),
            params![app_type],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
    }

    Ok(Some(MergedOfficialApp {
        app_type: app_type.to_string(),
        survivor_id: survivor.id.clone(),
        removed_ids,
        adopted_login_at: if freshest.id == survivor.id {
            None
        } else {
            freshest.login_at()
        },
    }))
}

/// 把 `fresh` 的登录态合并进 `canonical`，返回合并结果。
///
/// 规则：
/// - `auth` 是登录态的唯一载体，`fresh` 非空时以 `fresh` 为准（§8：绝不丢较新的登录）；
/// - 其余键只在 `canonical` 缺失或为空时从 `fresh` 补齐，避免覆盖种子行的身份字段。
fn merge_settings_config(fresh: &Value, canonical: &Value) -> Value {
    let (Some(fresh_obj), Some(canonical_obj)) = (fresh.as_object(), canonical.as_object()) else {
        return fresh.clone();
    };

    let mut merged: Map<String, Value> = canonical_obj.clone();

    if let Some(fresh_auth) = fresh_obj.get("auth") {
        let canonical_auth_empty = merged.get("auth").map(is_empty_json).unwrap_or(true);
        if canonical_auth_empty || !is_empty_json(fresh_auth) {
            merged.insert("auth".to_string(), fresh_auth.clone());
        }
    }

    for (key, value) in fresh_obj {
        if key == "auth" || is_empty_json(value) {
            continue;
        }
        match merged.get(key) {
            None => {
                merged.insert(key.clone(), value.clone());
            }
            Some(existing) if is_empty_json(existing) => {
                merged.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }

    Value::Object(merged)
}

fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn canonical_seed_id(app_type: &str) -> Option<&'static str> {
    OFFICIAL_SEEDS
        .iter()
        .find(|seed| seed.app_type.as_str() == app_type)
        .map(|seed| seed.id)
}

/// 解析 `auth.last_refresh`。兼容 RFC3339、无时区的朴素时间、以及纯数字时间戳。
fn parse_login_timestamp(value: &Value) -> Option<i64> {
    if let Some(millis) = value.as_i64() {
        return Some(if millis.abs() < 100_000_000_000 {
            millis.saturating_mul(1000)
        } else {
            millis
        });
    }

    let text = value.as_str()?.trim();
    if text.is_empty() {
        return None;
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc).timestamp_millis());
    }
    if let Ok(parsed) = DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f%z") {
        return Some(parsed.with_timezone(&Utc).timestamp_millis());
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Some(naive.and_utc().timestamp_millis());
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Some(
            date.and_hms_opt(0, 0, 0)?
                .and_utc()
                .timestamp_millis(),
        );
    }

    None
}

/// 每个 app 只保留一个 `is_current`（重复标记会导致"切回官方"落到不确定的行上）。
fn enforce_single_current_per_app(conn: &Connection) -> Result<usize, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT app_type FROM providers GROUP BY app_type HAVING SUM(is_current) > 1",
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
    let apps = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;

    let mut cleared = 0usize;
    for app_type in apps {
        // 保留排序最靠前的一行（与 providers 展示顺序一致），其余清除
        let keep_id: Option<String> = conn
            .query_row(
                "SELECT id FROM providers WHERE app_type = ?1 AND is_current = 1
                 ORDER BY COALESCE(sort_index, 999999), created_at ASC, id ASC LIMIT 1",
                params![app_type],
                |row| row.get(0),
            )
            .ok();

        let Some(keep_id) = keep_id else { continue };

        cleared += conn
            .execute(
                "UPDATE providers SET is_current = 0
                 WHERE app_type = ?1 AND is_current = 1 AND id <> ?2",
                params![app_type, keep_id],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;

        log::info!("✓ [{app_type}] 重复的当前供应商标记已清理，保留 {keep_id}");
    }

    Ok(cleared)
}

/// 故障转移关闭时 `in_failover_queue` 没有意义，清理并返回受影响的 app。
fn clear_failover_queues_without_failover(conn: &Connection) -> Result<Vec<String>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT p.app_type
             FROM providers p
             JOIN proxy_config c ON c.app_type = p.app_type
             WHERE p.in_failover_queue = 1 AND c.auto_failover_enabled = 0
             ORDER BY p.app_type",
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
    let apps = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| AppError::Database(e.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::Database(e.to_string()))?;

    for app_type in &apps {
        conn.execute(
            "UPDATE providers SET in_failover_queue = 0 WHERE app_type = ?1",
            params![app_type],
        )
        .map_err(|e| AppError::Database(e.to_string()))?;
        log::info!("✓ [{app_type}] 故障转移未启用，已清空故障转移队列");
    }

    Ok(apps)
}

/// 把被合并掉的行归档为 JSON，返回归档文件路径。
fn archive_removed_rows(rows: &[Value]) -> Result<String, AppError> {
    let dir = archive_removed_rows_dir();
    std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;

    let path = dir.join("providers.json");
    let payload = serde_json::json!({
        "reason": "duplicate-official-merge",
        "schema": OFFICIAL_MERGE_ARCHIVE_DIR,
        "archivedAt": Utc::now().to_rfc3339(),
        "providers": rows,
    });
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|e| AppError::Database(format!("Failed to serialize repair archive: {e}")))?;
    crate::config::atomic_write(&path, &bytes)?;

    let path_string = path.to_string_lossy().to_string();
    log::info!("✓ 重复官方供应商已归档到 {path_string}");
    Ok(path_string)
}

fn archive_removed_rows_dir() -> PathBuf {
    crate::config::get_app_config_dir()
        .join("backups")
        .join(OFFICIAL_MERGE_ARCHIVE_DIR)
        .join(Utc::now().format("%Y%m%d_%H%M%S").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use serial_test::serial;
    use std::env;
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("CC_SWITCH_TEST_HOME").ok();

            env::set_var("HOME", dir.path());
            env::set_var("USERPROFILE", dir.path());
            env::set_var("CC_SWITCH_TEST_HOME", dir.path());
            crate::settings::reload_settings().expect("reload settings");

            Self {
                dir,
                original_home,
                original_userprofile,
                original_test_home,
            }
        }

        fn archive_dir(&self) -> PathBuf {
            self.dir
                .path()
                .join(".cc-switch")
                .join("backups")
                .join(OFFICIAL_MERGE_ARCHIVE_DIR)
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }
            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }
            match &self.original_test_home {
                Some(value) => env::set_var("CC_SWITCH_TEST_HOME", value),
                None => env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    fn official_provider(id: &str, login_at: Option<&str>) -> crate::provider::Provider {
        let auth = match login_at {
            Some(login_at) => serde_json::json!({
                "auth": {
                    "last_refresh": login_at,
                    "tokens": { "access_token": format!("token-{id}") }
                },
                "config": format!("config-{id}")
            }),
            None => serde_json::json!({ "auth": {}, "config": "" }),
        };

        let mut provider = crate::provider::Provider::with_id(
            id.to_string(),
            format!("Official {id}"),
            auth,
            None,
        );
        provider.category = Some("official".to_string());
        provider
    }

    fn login_at(db: &Database, app_type: &str, id: &str) -> Option<String> {
        db.get_provider_by_id(id, app_type)
            .unwrap()
            .and_then(|p| {
                p.settings_config
                    .get("auth")
                    .and_then(|auth| auth.get("last_refresh"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    #[test]
    #[serial]
    fn merges_stale_seed_with_fresher_manual_row() {
        let home = TempHome::new();
        let db = Database::memory().unwrap();

        db.save_provider("codex", &official_provider("codex-official", None))
            .unwrap();
        db.save_provider(
            "codex",
            &official_provider("default", Some("2026-07-24T10:00:00Z")),
        )
        .unwrap();

        let report = db
            .repair_provider_state(RepairScope::STARTUP)
            .expect("repair");

        assert_eq!(report.merged_apps.len(), 1);
        let merged = &report.merged_apps[0];
        assert_eq!(merged.app_type, "codex");
        // 种子行保留 id，非种子行被归档
        assert_eq!(merged.survivor_id, "codex-official");
        assert_eq!(merged.removed_ids, vec!["default".to_string()]);
        assert_eq!(
            merged.adopted_login_at.as_deref(),
            Some("2026-07-24T10:00:00Z")
        );

        // 较新的登录态被搬进种子行
        assert_eq!(
            login_at(&db, "codex", "codex-official").as_deref(),
            Some("2026-07-24T10:00:00Z")
        );
        assert!(db.get_provider_by_id("default", "codex").unwrap().is_none());

        // 归档留有痕迹，且落在本次测试的临时 home 下
        let archived = report.archived_path.expect("archive path");
        assert!(std::path::Path::new(&archived).exists());
        assert!(
            archived.starts_with(&home.archive_dir().to_string_lossy().to_string()),
            "归档必须写在 {OFFICIAL_MERGE_ARCHIVE_DIR} 下: {archived}"
        );
    }

    #[test]
    #[serial]
    fn never_takes_the_login_from_a_stale_row() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        db.save_provider(
            "codex",
            &official_provider("codex-official", Some("2026-08-01T00:00:00Z")),
        )
        .unwrap();
        db.save_provider(
            "codex",
            &official_provider("default", Some("2026-07-24T10:00:00Z")),
        )
        .unwrap();

        let report = db
            .repair_provider_state(RepairScope::STARTUP)
            .expect("repair");

        assert_eq!(report.merged_apps.len(), 1);
        assert_eq!(report.merged_apps[0].adopted_login_at, None);
        assert_eq!(
            login_at(&db, "codex", "codex-official").as_deref(),
            Some("2026-08-01T00:00:00Z"),
            "较新的登录必须保留"
        );
    }

    #[test]
    #[serial]
    fn skips_merge_without_a_seed_row() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        // 两行都不是内置种子：不删任何东西
        db.save_provider("codex", &official_provider("a", Some("2026-01-01T00:00:00Z")))
            .unwrap();
        db.save_provider("codex", &official_provider("b", Some("2026-02-01T00:00:00Z")))
            .unwrap();

        let report = db
            .repair_provider_state(RepairScope::STARTUP)
            .expect("repair");

        assert!(report.merged_apps.is_empty());
        assert!(report.archived_path.is_none());
        assert!(db.get_provider_by_id("a", "codex").unwrap().is_some());
        assert!(db.get_provider_by_id("b", "codex").unwrap().is_some());
    }

    #[test]
    #[serial]
    fn repair_is_idempotent() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        db.save_provider("codex", &official_provider("codex-official", None))
            .unwrap();
        db.save_provider(
            "codex",
            &official_provider("default", Some("2026-07-24T10:00:00Z")),
        )
        .unwrap();

        let first = db.repair_provider_state(RepairScope::STARTUP).unwrap();
        assert_eq!(first.merged_apps.len(), 1);

        let second = db.repair_provider_state(RepairScope::STARTUP).unwrap();
        assert!(second.is_noop(), "第二次运行必须是无操作: {second:?}");
        assert!(db.get_provider_by_id("codex-official", "codex").unwrap().is_some());
        assert!(db.get_provider_by_id("default", "codex").unwrap().is_none());
    }

    #[test]
    #[serial]
    fn keeps_a_single_current_provider_per_app() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        let mut first = crate::provider::Provider::with_id(
            "first".to_string(),
            "First".to_string(),
            serde_json::json!({}),
            None,
        );
        first.sort_index = Some(0);
        let mut second = crate::provider::Provider::with_id(
            "second".to_string(),
            "Second".to_string(),
            serde_json::json!({}),
            None,
        );
        second.sort_index = Some(1);

        db.save_provider("claude", &first).unwrap();
        db.save_provider("claude", &second).unwrap();
        db.set_current_provider("claude", "second").unwrap();
        // 人为制造重复的 is_current（旧代码路径可能留下）
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE providers SET is_current = 1 WHERE app_type = 'claude'",
                [],
            )
            .unwrap();
        }

        let report = db.repair_provider_state(RepairScope::STARTUP).unwrap();

        assert_eq!(report.cleared_current_flags, 1);
        assert_eq!(
            db.get_current_provider("claude").unwrap().as_deref(),
            Some("first")
        );
    }

    #[test]
    #[serial]
    fn v16_to_v17_migration_repairs_a_legacy_database() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        // 复刻这台机器升级前的脏状态：种子行与历史行并存，两行都是 official，
        // 且较新的登录在被当成“碎片”的那一行上。
        db.save_provider("codex", &official_provider("codex-official", None))
            .unwrap();
        db.save_provider(
            "codex",
            &official_provider("default", Some("2026-07-24T10:00:00Z")),
        )
        .unwrap();
        db.set_current_provider("codex", "default").unwrap();
        db.add_to_failover_queue("codex", "codex-official").unwrap();

        {
            let conn = db.conn.lock().unwrap();
            Database::set_user_version(&conn, 16).expect("set user_version=16");
            Database::apply_schema_migrations_on_conn(&conn).expect("apply migrations");
            assert_eq!(
                Database::get_user_version(&conn).unwrap(),
                crate::database::SCHEMA_VERSION
            );
        }

        // 只剩种子行，且换上了较新的登录态
        assert!(db.get_provider_by_id("codex-official", "codex").unwrap().is_some());
        assert!(db.get_provider_by_id("default", "codex").unwrap().is_none());
        assert_eq!(
            login_at(&db, "codex", "codex-official").as_deref(),
            Some("2026-07-24T10:00:00Z")
        );

        // 迁移前选中的是被删掉的 default：当前标记必须落到幸存行上
        assert_eq!(
            db.get_current_provider("codex").unwrap().as_deref(),
            Some("codex-official")
        );

        // codex 未启用故障转移，队列在迁移中被清空（§8）
        assert!(!db.is_in_failover_queue("codex", "codex-official").unwrap());
    }

    #[tokio::test]
    #[serial]
    async fn migration_scope_clears_queue_only_when_failover_is_off() {
        let _home = TempHome::new();
        let db = Database::memory().unwrap();

        let provider_a = crate::provider::Provider::with_id(
            "a".to_string(),
            "A".to_string(),
            serde_json::json!({}),
            None,
        );
        db.save_provider("claude", &provider_a).unwrap();
        db.add_to_failover_queue("claude", "a").unwrap();

        // 启动范围不做清理（队列可以在故障转移关闭期间保留）
        let startup = db.repair_provider_state(RepairScope::STARTUP).unwrap();
        assert!(startup.cleared_failover_queue_apps.is_empty());
        assert!(db.is_in_failover_queue("claude", "a").unwrap());

        // 迁移范围清理故障转移关闭的 app（§8）
        let migration = db.repair_provider_state(RepairScope::MIGRATION).unwrap();
        assert_eq!(
            migration.cleared_failover_queue_apps,
            vec!["claude".to_string()]
        );
        assert!(!db.is_in_failover_queue("claude", "a").unwrap());

        // 已启用故障转移的 app 队列必须保留
        let mut config = db.get_proxy_config_for_app("codex").await.unwrap();
        config.auto_failover_enabled = true;
        db.update_proxy_config_for_app(config).await.unwrap();
        db.save_provider("codex", &provider_a).unwrap();
        db.add_to_failover_queue("codex", "a").unwrap();

        let second = db.repair_provider_state(RepairScope::MIGRATION).unwrap();
        assert!(second.cleared_failover_queue_apps.is_empty());
        assert!(db.is_in_failover_queue("codex", "a").unwrap());
    }

    #[test]
    fn parses_common_login_timestamp_shapes() {
        use serde_json::json;
        assert!(parse_login_timestamp(&json!("2026-07-24T10:00:00Z")).is_some());
        assert!(parse_login_timestamp(&json!("2026-07-24T10:00:00.500+08:00")).is_some());
        assert!(parse_login_timestamp(&json!("2026-07-24T10:00:00")).is_some());
        assert!(parse_login_timestamp(&json!("2026-07-24")).is_some());
        assert!(parse_login_timestamp(&json!(1753351200)).is_some());
        assert!(parse_login_timestamp(&json!("not a date")).is_none());
        assert!(parse_login_timestamp(&json!("")).is_none());

        // 毫秒时间戳不应被当成秒再次放大
        assert_eq!(
            parse_login_timestamp(&json!(1_753_351_200_000i64)),
            Some(1_753_351_200_000)
        );
    }

    #[test]
    fn merge_prefers_fresh_auth_and_keeps_seed_identity() {
        let canonical = serde_json::json!({ "auth": {}, "config": "" });
        let fresh = serde_json::json!({
            "auth": { "last_refresh": "2026-07-24T10:00:00Z", "tokens": { "access_token": "t" } },
            "config": "model_provider = \"openai\""
        });

        let merged = merge_settings_config(&fresh, &canonical);

        assert_eq!(
            merged.pointer("/auth/last_refresh").and_then(Value::as_str),
            Some("2026-07-24T10:00:00Z")
        );
        assert_eq!(
            merged.get("config").and_then(Value::as_str),
            Some("model_provider = \"openai\"")
        );

        // 种子行已有的非空值不被覆盖
        let canonical = serde_json::json!({ "auth": {}, "config": "seed-config" });
        let merged = merge_settings_config(&fresh, &canonical);
        assert_eq!(
            merged.get("config").and_then(Value::as_str),
            Some("seed-config")
        );
    }

    /// 把一份**真实**的 `cc-switch.db` 副本从 v16 升到 v17，走完整启动路径。
    ///
    /// 与前面的单测不同，这里的输入不是构造出来的内存库，而是用户线上库的副本被
    /// 还原成"修复前"的形状（每个 app 重复官方行 + 过期的故障转移队列），迁移由
    /// `Database::init`（= 应用启动时的同一条链）执行，因此能证明线上数据会被正确
    /// 修复，而不是只证明测试夹具自洽。
    ///
    /// 默认 `#[ignore]`：需要外部夹具。
    ///
    /// ```text
    /// set CC_SWITCH_REPAIR_FIXTURE_DIR=C:\tmp\cc-migrate-test\fixture-source
    /// set CC_SWITCH_REPAIR_OUTPUT_DIR=C:\tmp\cc-migrate-test\migrated
    /// cargo test --lib migration_upgrades_a_real_database_copy -- --ignored --nocapture
    /// ```
    #[test]
    #[serial]
    #[ignore = "needs CC_SWITCH_REPAIR_FIXTURE_DIR and CC_SWITCH_REPAIR_OUTPUT_DIR"]
    fn migration_upgrades_a_real_database_copy() {
        let fixture = PathBuf::from(
            env::var("CC_SWITCH_REPAIR_FIXTURE_DIR")
                .expect("CC_SWITCH_REPAIR_FIXTURE_DIR must point at a config dir"),
        );
        let output = PathBuf::from(
            env::var("CC_SWITCH_REPAIR_OUTPUT_DIR")
                .expect("CC_SWITCH_REPAIR_OUTPUT_DIR must point at a writable dir"),
        );
        let fixture_db = fixture.join(".cc-switch").join("cc-switch.db");
        assert!(fixture_db.exists(), "fixture db missing: {}", fixture_db.display());

        // 每次都从夹具重新开始，保证可重复运行。
        if output.exists() {
            std::fs::remove_dir_all(&output).expect("clear output dir");
        }
        copy_tree(&fixture, &output);

        let db_path = output.join(".cc-switch").join("cc-switch.db");
        let before_version = raw_query_i64(&db_path, "PRAGMA user_version");
        assert_eq!(before_version, 16, "fixture must start at v16");

        let original_home = env::var("HOME").ok();
        let original_userprofile = env::var("USERPROFILE").ok();
        let original_test_home = env::var("CC_SWITCH_TEST_HOME").ok();
        env::set_var("HOME", &output);
        env::set_var("USERPROFILE", &output);
        env::set_var("CC_SWITCH_TEST_HOME", &output);
        crate::settings::reload_settings().expect("reload settings");

        // 生产启动路径：建表 + 迁移链（v16 -> v17 就在其中）。
        drop(Database::init().expect("migrate the real database copy"));

        let conn = Connection::open(&db_path).expect("reopen migrated db");
        let report = |label: &str| {
            println!("\n=== {label} ===");
            for (app, count) in query_pairs(
                &conn,
                "SELECT app_type, COUNT(*) FROM providers WHERE category = 'official' \
                 GROUP BY app_type ORDER BY app_type",
            ) {
                println!("  official rows {app:<16} {count}");
            }
            for (app, queued) in query_pairs(
                &conn,
                "SELECT app_type, COUNT(*) FROM providers WHERE in_failover_queue = 1 \
                 GROUP BY app_type ORDER BY app_type",
            ) {
                println!("  queued rows   {app:<16} {queued}");
            }
        };

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("user_version");
        assert_eq!(version, 17, "migration must advance the schema to v17");
        report("AFTER");

        // codex：重复官方行合并，种子行保留 id 并接过更新的登录态。
        let codex_official = official_ids(&conn, "codex");
        assert_eq!(codex_official, vec!["codex-official".to_string()]);
        let codex_auth = official_settings(&conn, "codex", "codex-official");
        assert_eq!(
            codex_auth
                .pointer("/auth/tokens/access_token")
                .and_then(Value::as_str),
            Some("FIXTURE-CODEX-FRESH-LOGIN"),
            "the freshest login must move onto the surviving seed row"
        );
        assert_eq!(
            codex_auth
                .pointer("/auth/last_refresh")
                .and_then(Value::as_str),
            Some("2026-09-28T18:00:00.000000000Z")
        );
        assert_eq!(
            conn.query_row(
                "SELECT is_current FROM providers WHERE app_type='codex' AND id='codex-official'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("codex-official is_current"),
            1,
            "the removed row held is_current, so the survivor must adopt it"
        );

        // codex 故障转移是关的：整个队列（含第三方行）必须被清空。
        assert_eq!(
            queued_count(&conn, "codex"),
            0,
            "auto_failover_enabled = 0 must clear the whole codex queue"
        );

        // gemini：同样合并，但故障转移开着，队列必须原样保留。
        assert_eq!(official_ids(&conn, "gemini"), vec!["gemini-official".to_string()]);
        let gemini_auth = official_settings(&conn, "gemini", "gemini-official");
        assert_eq!(
            gemini_auth.pointer("/auth/apiKey").and_then(Value::as_str),
            Some("FIXTURE-GEMINI-FRESH-LOGIN")
        );
        assert_eq!(queued_count(&conn, "gemini"), 1, "failover-on queue must survive");

        // claude 本来就没有重复官方行：一行都不能少，队列原样保留。
        assert_eq!(official_ids(&conn, "claude"), vec!["claude-official".to_string()]);
        assert_eq!(queued_count(&conn, "claude"), 12);
        assert_eq!(official_ids(&conn, "claude-desktop"), vec!["claude-desktop-official".to_string()]);
        assert_eq!(official_ids(&conn, "grokbuild"), vec!["grokbuild-official".to_string()]);

        // 被删掉的行必须先落盘归档，绝不静默丢弃登录态。
        let archive = find_archive(&output);
        let archived = std::fs::read_to_string(&archive).expect("read archive");
        let archived: Value = serde_json::from_str(&archived).expect("parse archive");
        let archived_ids: Vec<String> = archived
            .get("providers")
            .and_then(Value::as_array)
            .expect("archive providers")
            .iter()
            .filter_map(|row| row.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        println!("\narchive: {}", archive.display());
        println!("archived ids: {archived_ids:?}");
        assert!(archived_ids.contains(&"codex-official-imported".to_string()));
        assert!(archived_ids.contains(&"gemini-official-imported".to_string()));

        // 幂等：再跑一次启动路径不得再改动任何东西。
        let before = snapshot_counts(&conn);
        drop(conn);
        drop(Database::init().expect("second startup must succeed"));
        let conn = Connection::open(&db_path).expect("reopen migrated db");
        assert_eq!(snapshot_counts(&conn), before, "repair must be idempotent");
        drop(conn);

        match original_home {
            Some(value) => env::set_var("HOME", value),
            None => env::remove_var("HOME"),
        }
        match original_userprofile {
            Some(value) => env::set_var("USERPROFILE", value),
            None => env::remove_var("USERPROFILE"),
        }
        match original_test_home {
            Some(value) => env::set_var("CC_SWITCH_TEST_HOME", value),
            None => env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }

    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).expect("create dir");
        for entry in std::fs::read_dir(from).expect("read dir") {
            let entry = entry.expect("dir entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("file type").is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).expect("copy file");
            }
        }
    }

    fn raw_query_i64(path: &std::path::Path, sql: &str) -> i64 {
        Connection::open(path)
            .expect("open db")
            .query_row(sql, [], |row| row.get(0))
            .expect("query")
    }

    fn query_pairs(conn: &Connection, sql: &str) -> Vec<(String, i64)> {
        let mut stmt = conn.prepare(sql).expect("prepare");
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect");
        rows
    }

    fn official_ids(conn: &Connection, app_type: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT id FROM providers WHERE app_type = ?1 AND category = 'official' ORDER BY id",
            )
            .expect("prepare");
        stmt.query_map(params![app_type], |row| row.get(0))
            .expect("query")
            .collect::<Result<Vec<String>, _>>()
            .expect("collect")
    }

    fn official_settings(conn: &Connection, app_type: &str, id: &str) -> Value {
        let raw: String = conn
            .query_row(
                "SELECT settings_config FROM providers WHERE app_type = ?1 AND id = ?2",
                params![app_type, id],
                |row| row.get(0),
            )
            .expect("settings_config");
        serde_json::from_str(&raw).expect("parse settings_config")
    }

    fn queued_count(conn: &Connection, app_type: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM providers WHERE app_type = ?1 AND in_failover_queue = 1",
            params![app_type],
            |row| row.get(0),
        )
        .expect("queued count")
    }

    /// 迁移后库内容的稳定指纹，用于幂等性断言。
    fn snapshot_counts(conn: &Connection) -> Vec<(String, i64)> {
        query_pairs(
            conn,
            "SELECT app_type || ':' || COALESCE(category, '') || ':' || is_current || ':' \
             || in_failover_queue, COUNT(*) FROM providers GROUP BY 1 ORDER BY 1",
        )
    }

    fn find_archive(root: &std::path::Path) -> PathBuf {
        let dir = root
            .join(".cc-switch")
            .join("backups")
            .join(OFFICIAL_MERGE_ARCHIVE_DIR);
        let mut files: Vec<PathBuf> = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("read archive dir") {
            let entry = entry.expect("archive entry");
            files.push(entry.path().join("providers.json"));
        }
        files.sort();
        files.pop().expect("an archive must exist")
    }
}
