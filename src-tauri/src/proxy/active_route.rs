//! 最近一次实际生效的路由决策（`get_active_route` 的数据源）。
//!
//! WHY: 路由器过去只把“本次请求目标供应商”写进日志（`select_providers`），前端因此无法回答
//! 一个最基本的问题——**到底哪个上游会真正应答？** 故障转移开启时选中项会被队列覆盖，
//! 而界面上没有任何地方说明这一点（见 docs/DESIGN-routing-mode.md §5.2 / §6.3）。
//!
//! 这里把决策**在做出时**记录到注册表，`get_active_route` 命令再读出来。
//!
//! 决策同时落盘（`<配置目录>/active-route.json`），启动时回灌：否则刚打开应用时界面
//! 只能退回到「预测」，而用户最想知道的恰恰是「上次到底走了哪条路」。落盘只在**决策变化**
//! 时发生（而不是每个请求），且写入由应用启动时的 [`enable_persistence`] 显式开启，
//! 因此单元测试跑路由逻辑不会碰真实配置目录。
//!
//! 仍然没有任何记录时返回 `None`，前端据此退回到「预测」展示。

use crate::app_config::AppType;
use crate::database::Database;
use crate::provider::Provider;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// `mode` 取值，与前端 `RoutingMode` 联合类型一致。
///
/// 路由器**不会**记录 `native`（只有被接管的请求才经过它），但当前输入可能是 native：
/// 退出接管后历史记录就不再成立。
pub const MODE_NATIVE: &str = "native";
pub const MODE_SELECTED: &str = "selected";
pub const MODE_FAILOVER: &str = "failover";

/// 最近一次路由决策的快照（返回给前端）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRoute {
    pub app_type: String,
    pub mode: String,
    pub effective_provider_id: Option<String>,
    pub effective_provider_name: Option<String>,
    pub upstream_base_url: Option<String>,
    /// 故障转移选中了非当前选中的供应商：界面上必须明确提示「选择已被忽略」。
    pub selection_ignored: bool,
    /// 生效供应商发生变化的 Unix 秒级时间戳。
    pub last_switch_at: Option<i64>,
    /// 路由器**最近一次**为该应用做出决定的时间（与是否切换供应商无关）。
    ///
    /// 仅用于显示「多久之前」。它**不是**失效判据：应用空闲时时间照样流逝，
    /// 一条完全有效的路由会被冤成陈迹。失效判据是 [`ActiveRoute::inputs_changed`]。
    ///
    /// 也不能用 `last_switch_at` 代替它：供应商不变时后者不会更新。
    pub last_confirmed_at: Option<i64>,
    /// 产生这条记录的输入是否已经变了。**这才是「还算不算当前」的判据**。
    pub inputs_changed: bool,
    /// FO-004 / FO-005：最近一次路由失败的结构化错误码。
    pub last_error_code: Option<String>,
}

const PERSISTED_FILE_NAME: &str = "active-route.json";

/// 缓存格式版本。v3 新增 `inputs`（旧文件直接作废重建，它只是缓存）。
const PERSISTED_VERSION: u32 = 3;

/// 「路由仍然有效」的心跳间隔。
///
/// 每个请求都精确记录确认时间的话，热路径上每次请求都要落盘；而确认时间只用来回答
/// 「这条记录还算不算当前」，秒级精度没有意义。因此最多每 60 秒刷一次，代价是重启后
/// 看到的时间最多偏小 60 秒 —— 对一个「是否过期」的判断完全够用。
const CONFIRMATION_REFRESH_SECS: i64 = 60;

/// 持久化格式的版本；不匹配时整份文件作废重建（它只是缓存，不是事实源）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedRoutes {
    version: u32,
    routes: HashMap<String, RecordedRoute>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordedRoute {
    mode: String,
    effective_provider_id: Option<String>,
    effective_provider_name: Option<String>,
    upstream_base_url: Option<String>,
    selection_ignored: bool,
    last_switch_at: Option<i64>,
    #[serde(default)]
    last_confirmed_at: Option<i64>,
    /// 产生这次决策的输入；落盘后下次启动可以判断它是否仍然成立。
    #[serde(default)]
    inputs: DecisionInputs,
    /// 有意不落盘：错误码描述的是**本进程**的失败，跨重启复活成红色警告只会误导。
    #[serde(skip_serializing, default)]
    last_error_code: Option<String>,
}

/// 决定「谁会应答」的输入集合。
///
/// 任何一项变化都说明旧记录不再代表当前会走的路。用输入而不是时间判断失效，
/// 是因为时间会因应用空闲而白白流逝：没有请求就没有新记录，但路由依然有效。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionInputs {
    /// 接管后的实际模式（native / selected / failover）。
    pub mode: String,
    /// 当前选中的供应商。
    pub selected_provider_id: Option<String>,
    /// 故障转移队列顺序。
    pub failover_queue: Vec<String>,
    /// `failoverPrefersSelected`：决定选中项是否排在队列前面。
    pub prefers_selected: bool,
    /// 生效供应商**当前**的上游地址：供应商配置被改过时会与记录不一致。
    pub effective_base_url: Option<String>,
}

fn registry() -> &'static RwLock<HashMap<String, RecordedRoute>> {
    static ROUTES: OnceLock<RwLock<HashMap<String, RecordedRoute>>> = OnceLock::new();
    ROUTES.get_or_init(|| RwLock::new(HashMap::new()))
}

/// 是否把决策落盘。默认关闭，由应用启动时开启：路由逻辑的单元测试
/// （例如 `provider_router` 的用例）会直接调用 `record_selection`，不能因此写入
/// 真实 `~/.cc-switch`。
static PERSISTENCE_ENABLED: AtomicBool = AtomicBool::new(false);

/// 上次成功落盘的 Unix 秒级时间戳，用于心跳节流（`CONFIRMATION_REFRESH_SECS`）。
static LAST_PERSIST_AT: AtomicI64 = AtomicI64::new(i64::MIN);

/// 允许读写 `<配置目录>/active-route.json`。幂等。
pub fn enable_persistence() {
    PERSISTENCE_ENABLED.store(true, Ordering::Relaxed);
}

fn persistence_enabled() -> bool {
    PERSISTENCE_ENABLED.load(Ordering::Relaxed)
}

fn last_persist_at() -> Option<i64> {
    match LAST_PERSIST_AT.load(Ordering::Relaxed) {
        i64::MIN => None,
        value => Some(value),
    }
}

/// 两个记录是否代表**不同的决策**。
///
/// 比较时忽略 `last_confirmed_at`：它每个请求都会变，如果算进差异，热路径上每个请求
/// 都会写盘 —— 而这正是「只在变化时落盘」想避免的。
fn decision_differs(previous: &RecordedRoute, next: &RecordedRoute) -> bool {
    let mut comparable = next.clone();
    comparable.last_confirmed_at = previous.last_confirmed_at;
    comparable != *previous
}

/// 是否该落盘：决策变了，或者确认时间已经超过刷新窗口。
///
/// 抽成纯函数是因为「不改不变的东西」和「心跳到期」这两种触发条件的区别很容易写错，
/// 而它们决定了热路径上是否会每个请求都写一次磁盘。
fn should_persist(decision_changed: bool, last_persist_at: Option<i64>, now: i64) -> bool {
    if decision_changed {
        return true;
    }

    last_persist_at.map_or(true, |at| now.saturating_sub(at) >= CONFIRMATION_REFRESH_SECS)
}

fn persisted_file_path() -> PathBuf {
    crate::config::get_app_config_dir().join(PERSISTED_FILE_NAME)
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|delta| delta.as_secs() as i64)
        .unwrap_or_default()
}

/// 记录一次成功的路由决策。
///
/// `last_switch_at` 只在**生效供应商真的变了**时更新（包括首次记录），所以前端可以把它
/// 当作「上次切换」而不是「上次请求」。
pub fn record_selection(
    app_type: &str,
    mode: &str,
    provider: &Provider,
    selection_ignored: bool,
    upstream_base_url: Option<String>,
    inputs: DecisionInputs,
) {
    let now = now_unix_secs();

    let (decision_changed, last_persist) = {
        let Ok(mut routes) = registry().write() else {
            log::warn!("[{app_type}] active-route 注册表写入锁中毒，跳过本次路由记录");
            return;
        };

        let previous = routes.remove(app_type).unwrap_or_default();
        let switched = previous.effective_provider_id.as_deref() != Some(provider.id.as_str());
        let last_switch_at = if switched {
            Some(now)
        } else {
            previous.last_switch_at
        };

        let recorded = RecordedRoute {
            mode: mode.to_string(),
            effective_provider_id: Some(provider.id.clone()),
            effective_provider_name: Some(provider.name.clone()),
            upstream_base_url,
            selection_ignored,
            last_switch_at,
            last_confirmed_at: Some(now),
            inputs,
            // 一次成功的选择清掉上一次的结构化错误。
            last_error_code: None,
        };

        let decision_changed = decision_differs(&previous, &recorded);

        routes.insert(app_type.to_string(), recorded);
        (decision_changed, last_persist_at())
    };

    // `select_providers` 是每个请求的热路径：只有决策真的变了，或心跳到期时才写盘。
    if should_persist(decision_changed, last_persist, now) {
        write_through();
    }
}

/// 记录一次路由失败（FO-004 / FO-005）。保留上一次的生效供应商，便于界面解释失败前
/// 的最后一个目标。
///
/// 错误码不落盘（见 [`RecordedRoute::last_error_code`]），因此这里不需要写文件。
pub fn record_error(app_type: &str, code: &str) {
    let Ok(mut routes) = registry().write() else {
        log::warn!("[{app_type}] active-route 注册表写入锁中毒，跳过本次错误记录");
        return;
    };

    let entry = routes.entry(app_type.to_string()).or_default();
    entry.last_error_code = Some(code.to_string());
}

/// 读取指定应用最近一次路由决策；从未路由过时返回 `None`。
///
/// `current_inputs` 是**当前**配置下的输入，用来判断记录是否已经失效：
/// 传 `None` 表示不做校验（调用方拿不到当前配置），此时 `inputs_changed` 为 false。
pub fn snapshot(app_type: &str, current_inputs: Option<&DecisionInputs>) -> Option<ActiveRoute> {
    let routes = registry().read().ok()?;
    let recorded = routes.get(app_type)?;

    Some(ActiveRoute {
        app_type: app_type.to_string(),
        mode: recorded.mode.clone(),
        effective_provider_id: recorded.effective_provider_id.clone(),
        effective_provider_name: recorded.effective_provider_name.clone(),
        upstream_base_url: recorded.upstream_base_url.clone(),
        selection_ignored: recorded.selection_ignored,
        last_switch_at: recorded.last_switch_at,
        last_confirmed_at: recorded.last_confirmed_at,
        inputs_changed: current_inputs.is_some_and(|current| *current != recorded.inputs),
        last_error_code: recorded.last_error_code.clone(),
    })
}

/// 读出记录并用**当前**配置校验它。存在记录时返回带 `inputs_changed` 的视图。
///
/// 这是给 `get_active_route` 用的入口：调用方不需要知道校验细节（记录里的生效
/// 供应商要先去查一下它的当前上游地址）。
pub async fn verified_snapshot(db: &Database, app_type: &str) -> Option<ActiveRoute> {
    let recorded_provider = {
        let routes = registry().read().ok()?;
        routes.get(app_type)?.effective_provider_id.clone()
    };

    let inputs = current_inputs(db, app_type, recorded_provider.as_deref()).await;
    snapshot(app_type, inputs.as_ref())
}

/// 读出**当前**配置下的决策输入，用于校验一条历史记录。
///
/// `effective_provider_id` 是记录里的生效供应商（可能已经不在了）：只有它才能回答
/// 「同一个供应商现在的上游地址还是那个吗」。
pub async fn current_inputs(
    db: &Database,
    app_type: &str,
    effective_provider_id: Option<&str>,
) -> Option<DecisionInputs> {
    let config = db.get_proxy_config_for_app(app_type).await.ok()?;
    let mode = if !config.enabled {
        MODE_NATIVE
    } else if config.auto_failover_enabled {
        MODE_FAILOVER
    } else {
        MODE_SELECTED
    };

    let failover_queue = db
        .get_failover_queue(app_type)
        .map(|queue| queue.into_iter().map(|item| item.provider_id).collect())
        .unwrap_or_default();

    let effective_base_url = effective_provider_id
        .and_then(|id| db.get_provider_by_id(id, app_type).ok().flatten())
        .zip(AppType::from_str(app_type).ok())
        .and_then(|(provider, app)| {
            crate::proxy::providers::get_adapter(&app)
                .extract_base_url(&provider)
                .ok()
        });

    Some(DecisionInputs {
        mode: mode.to_string(),
        selected_provider_id: crate::settings::get_effective_current_provider(
            db,
            &AppType::from_str(app_type).ok()?,
        )
        .ok()
        .flatten()
        .or_else(|| db.get_current_provider(app_type).ok().flatten()),
        failover_queue,
        prefers_selected: crate::settings::failover_prefers_selected(),
        effective_base_url,
    })
}

/// 忘记指定应用的路由记录（例如退出接管时）。
pub fn clear(app_type: &str) {
    let removed = match registry().write() {
        Ok(mut routes) => routes.remove(app_type).is_some(),
        Err(_) => false,
    };

    if removed {
        write_through();
    }
}

/// 回灌上次运行留下的决策，返回读入的条数。
///
/// 内存里已有的记录优先（它更新），所以重复调用是安全的。
pub fn load_persisted() -> usize {
    if !persistence_enabled() {
        return 0;
    }

    load_from_path(&persisted_file_path())
}

fn load_from_path(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }

    let payload: PersistedRoutes = match crate::config::read_json_file(path) {
        Ok(payload) => payload,
        Err(e) => {
            log::warn!("读取 active-route 缓存失败，已忽略: {e}");
            return 0;
        }
    };

    if payload.version != PERSISTED_VERSION {
        log::info!(
            "active-route 缓存版本 {} 与当前 {PERSISTED_VERSION} 不符，已忽略",
            payload.version
        );
        return 0;
    }

    let Ok(mut routes) = registry().write() else {
        return 0;
    };

    let mut loaded = 0;
    for (app_type, recorded) in payload.routes {
        if routes.contains_key(&app_type) {
            continue;
        }
        routes.insert(app_type, recorded);
        loaded += 1;
    }

    if loaded > 0 {
        // 刚读回来的心跳时间也要认：否则启动后第一个请求会立刻又写一次。
        let newest = routes.values().filter_map(|route| route.last_confirmed_at).max();
        if let Some(newest) = newest {
            LAST_PERSIST_AT.store(newest, Ordering::Relaxed);
        }
        log::info!("✓ 恢复 {loaded} 条上次运行的路由记录");
    }
    loaded
}

fn write_through() {
    if !persistence_enabled() {
        return;
    }

    let routes = match registry().read() {
        Ok(routes) => routes.clone(),
        Err(_) => return,
    };

    LAST_PERSIST_AT.store(now_unix_secs(), Ordering::Relaxed);
    write_or_remove(&persisted_file_path(), &routes);
}

/// 没有记录时删掉缓存，而不是留一份空文件让下次启动误以为有内容。
fn write_or_remove(path: &Path, routes: &HashMap<String, RecordedRoute>) {
    if routes.is_empty() {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(path) {
                log::warn!("清理 active-route 缓存失败: {e}");
            }
        }
        return;
    }

    write_to_path(path, routes);
}

fn write_to_path(path: &Path, routes: &HashMap<String, RecordedRoute>) {
    let payload = PersistedRoutes {
        version: PERSISTED_VERSION,
        routes: routes.clone(),
    };

    match serde_json::to_vec_pretty(&payload) {
        Ok(bytes) => {
            if let Err(e) = crate::config::atomic_write(path, &bytes) {
                // 缓存写失败不影响路由：下一次决策变化会再试一次。
                log::warn!("写入 active-route 缓存失败: {e}");
            }
        }
        Err(e) => log::warn!("序列化 active-route 缓存失败: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(id: &str, name: &str) -> Provider {
        Provider::with_id(id.to_string(), name.to_string(), json!({}), None)
    }

    fn inputs(mode: &str) -> DecisionInputs {
        DecisionInputs {
            mode: mode.to_string(),
            ..Default::default()
        }
    }

    /// 记录一次决策，输入与 `mode` 一致（大多数用例只关心决策本身）。
    fn record(
        app_type: &str,
        mode: &str,
        provider: &Provider,
        selection_ignored: bool,
        upstream_base_url: Option<&str>,
    ) {
        record_selection(
            app_type,
            mode,
            provider,
            selection_ignored,
            upstream_base_url.map(str::to_string),
            inputs(mode),
        );
    }

    /// 不做输入校验的读取（校验本身由专门的用例覆盖）。
    fn view(app_type: &str) -> ActiveRoute {
        snapshot(app_type, None).expect("a recorded route")
    }

    #[test]
    fn unknown_app_has_no_active_route() {
        assert!(snapshot("active-route-test-unknown", None).is_none());
    }

    #[test]
    fn records_selection_and_clears_previous_error() {
        let app = "active-route-test-selection";
        let provider = provider("p1", "Provider One");

        record_error(app, crate::proxy::log_codes::fo::NO_PROVIDERS);
        assert_eq!(
            view(app).last_error_code.as_deref(),
            Some("FO-005")
        );

        record(
            app,
            MODE_SELECTED,
            &provider,
            false,
            Some("https://relay.example.com/v1"),
        );

        let route = view(app);
        assert_eq!(route.app_type, app);
        assert_eq!(route.mode, MODE_SELECTED);
        assert_eq!(route.effective_provider_id.as_deref(), Some("p1"));
        assert_eq!(route.effective_provider_name.as_deref(), Some("Provider One"));
        assert_eq!(
            route.upstream_base_url.as_deref(),
            Some("https://relay.example.com/v1")
        );
        assert!(!route.selection_ignored);
        assert!(route.last_switch_at.is_some());
        assert_eq!(route.last_error_code, None);

        clear(app);
        assert!(snapshot(app, None).is_none());
    }

    #[test]
    fn last_switch_at_only_moves_when_the_effective_provider_changes() {
        let app = "active-route-test-switch-time";
        let first = provider("p1", "First");
        let second = provider("p2", "Second");

        record(app, MODE_FAILOVER, &first, true, None);
        let initial = view(app).last_switch_at;

        // Same provider again: the timestamp must not move.
        record(app, MODE_FAILOVER, &first, true, None);
        assert_eq!(view(app).last_switch_at, initial);

        // Different provider: the timestamp is refreshed.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record(app, MODE_FAILOVER, &second, true, None);
        let after = view(app).last_switch_at;
        assert!(after > initial);

        clear(app);
    }

    #[test]
    fn selection_ignored_is_reported_for_failover_winners() {
        let app = "active-route-test-ignored";
        let winner = provider("queue-p1", "Queue P1");

        record(app, MODE_FAILOVER, &winner, true, None);
        let route = view(app);
        assert_eq!(route.mode, MODE_FAILOVER);
        assert!(route.selection_ignored);

        clear(app);
    }

    /// 用户真正关心的场景：应用开着但一直空闲，记录很旧，配置却一点没变。
    /// 这时路由依然成立，不能因为「很久没有请求」就当成陈迹。
    #[test]
    fn an_idle_apps_old_record_still_counts_as_current() {
        let app = "active-route-test-idle";
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);

        let mut entry = recorded("queue-p1");
        entry.last_confirmed_at = Some(1_790_000_000 - 6 * 3600);
        write_sample(&path, app, entry);
        assert_eq!(load_from_path(&path), 1);

        let route = snapshot(app, Some(&inputs(MODE_FAILOVER))).expect("a record");
        assert!(
            !route.inputs_changed,
            "an idle app must not turn a still-valid route into history"
        );

        clear(app);
    }

    /// 失效的判据是输入变了，而不是时间过去了。
    #[test]
    fn changed_inputs_mark_the_record_as_no_longer_current() {
        let app = "active-route-test-inputs";
        record(app, MODE_FAILOVER, &provider("queue-p1", "Queue P1"), true, None);

        let matching = inputs(MODE_FAILOVER);
        assert!(!snapshot(app, Some(&matching)).unwrap().inputs_changed);

        let mut selected_changed = matching.clone();
        selected_changed.selected_provider_id = Some("another-provider".to_string());
        assert!(snapshot(app, Some(&selected_changed)).unwrap().inputs_changed);

        let mut queue_changed = matching.clone();
        queue_changed.failover_queue = vec!["p2".to_string()];
        assert!(snapshot(app, Some(&queue_changed)).unwrap().inputs_changed);

        // Takeover released: nothing is proxied any more, so the record is history.
        assert!(snapshot(app, Some(&inputs(MODE_NATIVE)))
            .unwrap()
            .inputs_changed);

        let mut url_changed = matching.clone();
        url_changed.effective_base_url = Some("https://moved.example.com/v1".to_string());
        assert!(snapshot(app, Some(&url_changed)).unwrap().inputs_changed);

        let mut preferring = matching;
        preferring.prefers_selected = !preferring.prefers_selected;
        assert!(snapshot(app, Some(&preferring)).unwrap().inputs_changed);

        clear(app);
    }

    /// 拿不到当前配置时不做校验（不能无端把每条记录都标成历史）。
    #[test]
    fn without_current_inputs_a_record_is_returned_unverified() {
        let app = "active-route-test-unverified";
        record(app, MODE_SELECTED, &provider("p1", "Provider One"), false, None);

        assert!(!snapshot(app, None).unwrap().inputs_changed);

        clear(app);
    }

    #[test]
    fn error_code_is_recorded_without_dropping_the_last_provider() {
        let app = "active-route-test-error";
        let provider = provider("p1", "Provider One");

        record(app, MODE_SELECTED, &provider, false, None);
        record_error(app, crate::proxy::log_codes::fo::ALL_CIRCUIT_OPEN);

        let route = view(app);
        assert_eq!(route.effective_provider_id.as_deref(), Some("p1"));
        assert_eq!(route.last_error_code.as_deref(), Some("FO-004"));

        clear(app);
    }

    fn recorded(id: &str) -> RecordedRoute {
        RecordedRoute {
            mode: MODE_FAILOVER.to_string(),
            effective_provider_id: Some(id.to_string()),
            effective_provider_name: Some(format!("Provider {id}")),
            upstream_base_url: Some("https://relay.example.com/v1".to_string()),
            selection_ignored: true,
            last_switch_at: Some(1_790_000_000),
            last_confirmed_at: Some(1_790_000_010),
            inputs: inputs(MODE_FAILOVER),
            last_error_code: None,
        }
    }

    /// 落盘时机：热路径上绝不能每个请求都写盘，但心跳必须能到期。
    #[test]
    fn persisting_happens_on_change_or_on_the_heartbeat() {
        let now = 1_790_000_000;

        assert!(should_persist(true, None, now), "a change always persists");
        assert!(should_persist(false, None, now), "nothing written yet");

        assert!(
            !should_persist(false, Some(now), now),
            "the same decision again must not touch the disk"
        );
        assert!(!should_persist(
            false,
            Some(now - CONFIRMATION_REFRESH_SECS + 1),
            now
        ));
        assert!(
            should_persist(false, Some(now - CONFIRMATION_REFRESH_SECS), now),
            "the heartbeat must eventually refresh the confirmation time"
        );

        // A clock that jumped backwards waits instead of refreshing: the reported
        // age may then look older than reality, never fresher, and the heartbeat
        // resumes as soon as the clock catches up.
        assert!(!should_persist(false, Some(now + 5), now));
    }

    /// 确认时间必须跟着**每一个**请求走，而不是只在切换时更新。
    #[test]
    fn the_confirmation_time_tracks_every_request() {
        let app = "active-route-test-confirmed-at";
        let provider = provider("p1", "Provider One");

        record(app, MODE_SELECTED, &provider, false, None);
        let first = view(app);
        assert_eq!(first.last_confirmed_at, first.last_switch_at);

        // Same provider: the switch time must stay put while the confirmation
        // time advances — this is why `lastConfirmedAt` cannot reuse
        // `lastSwitchAt`.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record(app, MODE_SELECTED, &provider, false, None);
        let second = view(app);
        assert_eq!(second.last_switch_at, first.last_switch_at);
        assert!(second.last_confirmed_at > first.last_confirmed_at);

        clear(app);
    }

    /// 确认时间每次请求都会变，所以它不能算作「决策变了」——否则热路径上每个请求写一次盘。
    #[test]
    fn a_new_confirmation_time_is_not_a_new_decision() {
        let before = recorded("p1");

        let mut same_decision = before.clone();
        same_decision.last_confirmed_at = before.last_confirmed_at.map(|at| at + 3600);
        assert!(!decision_differs(&before, &same_decision));

        let mut switched = same_decision.clone();
        switched.effective_provider_id = Some("p2".to_string());
        switched.last_switch_at = Some(1_790_099_999);
        assert!(decision_differs(&before, &switched));

        let mut flipped_mode = same_decision.clone();
        flipped_mode.mode = MODE_SELECTED.to_string();
        assert!(decision_differs(&before, &flipped_mode));

        let mut now_ignored = same_decision;
        now_ignored.selection_ignored = !before.selection_ignored;
        assert!(
            decision_differs(&before, &now_ignored),
            "the selection-ignored badge is part of the decision"
        );
    }

    fn write_sample(path: &Path, app_type: &str, entry: RecordedRoute) {
        let mut routes = HashMap::new();
        routes.insert(app_type.to_string(), entry);
        write_to_path(path, &routes);
    }

    #[test]
    fn persisted_routes_survive_a_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);
        let expected = recorded("litellm");

        write_sample(&path, "active-route-test-roundtrip", expected.clone());

        let raw = std::fs::read_to_string(&path).expect("read cache");
        let payload: PersistedRoutes = serde_json::from_str(&raw).expect("parse cache");
        assert_eq!(payload.version, PERSISTED_VERSION);
        assert_eq!(
            payload.routes.get("active-route-test-roundtrip"),
            Some(&expected)
        );
        // 缓存是给前端读的，字段名必须和 `ActiveRoute` 一样是 camelCase。
        assert!(raw.contains("\"effectiveProviderId\""));
        assert!(raw.contains("\"selectionIgnored\""));
    }

    /// 启动后立刻能说出真实路由，而不是退回「预测」——正是这个功能存在的理由。
    #[test]
    fn a_persisted_route_is_visible_immediately_after_startup() {
        let app = "active-route-test-restart";
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);
        write_sample(&path, app, recorded("queue-p1"));

        assert_eq!(load_from_path(&path), 1);

        let route = view(app);
        assert_eq!(route.effective_provider_id.as_deref(), Some("queue-p1"));
        assert_eq!(route.mode, MODE_FAILOVER);
        assert!(route.selection_ignored);
        assert_eq!(route.upstream_base_url.as_deref(), Some("https://relay.example.com/v1"));

        clear(app);
    }

    #[test]
    fn loading_does_not_overwrite_a_fresher_in_memory_decision() {
        let app = "active-route-test-fresh-wins";
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);
        write_sample(&path, app, recorded("stale-from-disk"));

        record(app, MODE_SELECTED, &provider("live", "Live"), false, None);

        assert_eq!(load_from_path(&path), 0);
        assert_eq!(
            view(app).effective_provider_id.as_deref(),
            Some("live")
        );

        clear(app);
    }

    #[test]
    fn a_stale_version_or_corrupt_cache_is_ignored() {
        let dir = tempfile::tempdir().expect("tempdir");

        let stale = dir.path().join("stale.json");
        std::fs::write(
            &stale,
            r#"{"version":999,"routes":{"active-route-test-stale":{"mode":"failover"}}}"#,
        )
        .expect("write stale cache");
        assert_eq!(load_from_path(&stale), 0);
        assert!(snapshot("active-route-test-stale", None).is_none());

        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, b"not json at all").expect("write corrupt cache");
        assert_eq!(load_from_path(&corrupt), 0);
    }

    #[test]
    fn an_empty_registry_removes_the_cache_instead_of_leaving_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);

        write_or_remove(&path, &HashMap::new());
        assert!(!path.exists(), "nothing to cache: no file to leave behind");

        let mut routes = HashMap::new();
        routes.insert("codex".to_string(), recorded("p1"));
        write_or_remove(&path, &routes);
        assert!(path.exists());

        // Leaving takeover drops the last record, which must delete the cache
        // rather than leave a stale route for the next launch to restore.
        write_or_remove(&path, &HashMap::new());
        assert!(!path.exists());
    }

    #[test]
    fn the_cache_lives_under_the_app_config_dir() {
        let path = persisted_file_path();

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(PERSISTED_FILE_NAME)
        );
        assert_eq!(path.parent(), Some(crate::config::get_app_config_dir().as_path()));
    }

    /// 错误码描述本进程的失败，跨重启复活成红色警告只会误导。
    #[test]
    fn an_error_code_is_never_resurrected_from_disk() {
        let app = "active-route-test-no-error-cache";
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PERSISTED_FILE_NAME);

        let mut entry = recorded("queue-p1");
        entry.last_error_code = Some("FO-005".to_string());
        write_sample(&path, app, entry);

        let raw = std::fs::read_to_string(&path).expect("read cache");
        assert!(!raw.contains("FO-005"), "the error code must not be written");

        assert_eq!(load_from_path(&path), 1);
        assert_eq!(view(app).last_error_code, None);

        clear(app);
    }

    /// 开关闭合下的真实路径（写入 + 回灌）。
    ///
    /// `#[ignore]`：它会打开持久化开关并写盘，必须在隔离的 home 下单独跑，
    /// 不能和并行测试共享进程状态。
    ///
    /// ```text
    /// cargo test --lib persistence_round_trips_through -- --ignored --nocapture
    /// ```
    #[test]
    #[serial_test::serial]
    #[ignore = "enables persistence for this process; run alone"]
    fn persistence_round_trips_through_the_real_config_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", dir.path());
        enable_persistence();

        let path = persisted_file_path();
        assert_eq!(path, dir.path().join(".cc-switch").join(PERSISTED_FILE_NAME));

        // A decision change writes itself out on the way past.
        record(
            "codex",
            MODE_FAILOVER,
            &provider("queue-p1", "Queue P1"),
            true,
            Some("http://127.0.0.1:4000/v1"),
        );
        assert!(path.exists(), "a decision change must be persisted");

        // Repeating the same decision must not rewrite the cache.
        let first_write = std::fs::metadata(&path).expect("stat cache").modified().expect("mtime");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record(
            "codex",
            MODE_FAILOVER,
            &provider("queue-p1", "Queue P1"),
            true,
            Some("http://127.0.0.1:4000/v1"),
        );
        assert_eq!(
            std::fs::metadata(&path).expect("stat cache").modified().expect("mtime"),
            first_write,
            "an unchanged decision must not touch the disk"
        );

        // Leaving takeover forgets the route, cache included.
        clear("codex");
        assert!(!path.exists(), "clear() must not leave a stale cache");

        // Next launch: the gated entry point restores what the file holds.
        let app = "active-route-test-gated-restart";
        let mut routes = HashMap::new();
        routes.insert(app.to_string(), recorded("litellm"));
        write_to_path(&path, &routes);

        assert_eq!(load_persisted(), 1);
        let route = view(app);
        assert_eq!(route.effective_provider_id.as_deref(), Some("litellm"));
        assert_eq!(route.mode, MODE_FAILOVER);

        clear(app);
        match previous {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
    }
}
