//! 最近一次实际生效的路由决策（`get_active_route` 的数据源）。
//!
//! WHY: 路由器过去只把“本次请求目标供应商”写进日志（`select_providers`），前端因此无法回答
//! 一个最基本的问题——**到底哪个上游会真正应答？** 故障转移开启时选中项会被队列覆盖，
//! 而界面上没有任何地方说明这一点（见 docs/DESIGN-routing-mode.md §5.2 / §6.3）。
//!
//! 这里把决策**在做出时**记录到进程内注册表，`get_active_route` 命令再读出来。
//! 只保留内存状态：它是“最近一次观测”，随进程重启失效是正确语义（后端没有任何持久化路由
//! 需求的场景）。没有任何请求被路由过时返回 `None`，前端据此退回到「预测」展示。

use crate::provider::Provider;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// `mode` 取值，与前端 `RoutingMode` 联合类型一致。
///
/// 没有 `native`：只有被代理接管的请求才会经过路由器，因此「原生」不会被记录，
/// `snapshot` 返回 `None` 即代表没有发生过代理路由。
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
    /// FO-004 / FO-005：最近一次路由失败的结构化错误码。
    pub last_error_code: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RecordedRoute {
    mode: String,
    effective_provider_id: Option<String>,
    effective_provider_name: Option<String>,
    upstream_base_url: Option<String>,
    selection_ignored: bool,
    last_switch_at: Option<i64>,
    last_error_code: Option<String>,
}

fn registry() -> &'static RwLock<HashMap<String, RecordedRoute>> {
    static ROUTES: OnceLock<RwLock<HashMap<String, RecordedRoute>>> = OnceLock::new();
    ROUTES.get_or_init(|| RwLock::new(HashMap::new()))
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
) {
    let Ok(mut routes) = registry().write() else {
        log::warn!("[{app_type}] active-route 注册表写入锁中毒，跳过本次路由记录");
        return;
    };

    let previous = routes.remove(app_type).unwrap_or_default();
    let switched = previous.effective_provider_id.as_deref() != Some(provider.id.as_str());
    let last_switch_at = if switched {
        Some(now_unix_secs())
    } else {
        previous.last_switch_at
    };

    routes.insert(
        app_type.to_string(),
        RecordedRoute {
            mode: mode.to_string(),
            effective_provider_id: Some(provider.id.clone()),
            effective_provider_name: Some(provider.name.clone()),
            upstream_base_url,
            selection_ignored,
            last_switch_at,
            // 一次成功的选择清掉上一次的结构化错误。
            last_error_code: None,
        },
    );
}

/// 记录一次路由失败（FO-004 / FO-005）。保留上一次的生效供应商，便于界面解释失败前
/// 的最后一个目标。
pub fn record_error(app_type: &str, code: &str) {
    let Ok(mut routes) = registry().write() else {
        log::warn!("[{app_type}] active-route 注册表写入锁中毒，跳过本次错误记录");
        return;
    };

    let entry = routes.entry(app_type.to_string()).or_default();
    entry.last_error_code = Some(code.to_string());
}

/// 读取指定应用最近一次路由决策；从未路由过时返回 `None`。
pub fn snapshot(app_type: &str) -> Option<ActiveRoute> {
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
        last_error_code: recorded.last_error_code.clone(),
    })
}

/// 忘记指定应用的路由记录（例如退出接管时）。
pub fn clear(app_type: &str) {
    if let Ok(mut routes) = registry().write() {
        routes.remove(app_type);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider(id: &str, name: &str) -> Provider {
        Provider::with_id(id.to_string(), name.to_string(), json!({}), None)
    }

    #[test]
    fn unknown_app_has_no_active_route() {
        assert!(snapshot("active-route-test-unknown").is_none());
    }

    #[test]
    fn records_selection_and_clears_previous_error() {
        let app = "active-route-test-selection";
        let provider = provider("p1", "Provider One");

        record_error(app, crate::proxy::log_codes::fo::NO_PROVIDERS);
        assert_eq!(
            snapshot(app).unwrap().last_error_code.as_deref(),
            Some("FO-005")
        );

        record_selection(
            app,
            MODE_SELECTED,
            &provider,
            false,
            Some("https://relay.example.com/v1".to_string()),
        );

        let route = snapshot(app).unwrap();
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
        assert!(snapshot(app).is_none());
    }

    #[test]
    fn last_switch_at_only_moves_when_the_effective_provider_changes() {
        let app = "active-route-test-switch-time";
        let first = provider("p1", "First");
        let second = provider("p2", "Second");

        record_selection(app, MODE_FAILOVER, &first, true, None);
        let initial = snapshot(app).unwrap().last_switch_at;

        // Same provider again: the timestamp must not move.
        record_selection(app, MODE_FAILOVER, &first, true, None);
        assert_eq!(snapshot(app).unwrap().last_switch_at, initial);

        // Different provider: the timestamp is refreshed.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        record_selection(app, MODE_FAILOVER, &second, true, None);
        let after = snapshot(app).unwrap().last_switch_at;
        assert!(after > initial);

        clear(app);
    }

    #[test]
    fn selection_ignored_is_reported_for_failover_winners() {
        let app = "active-route-test-ignored";
        let winner = provider("queue-p1", "Queue P1");

        record_selection(app, MODE_FAILOVER, &winner, true, None);
        let route = snapshot(app).unwrap();
        assert_eq!(route.mode, MODE_FAILOVER);
        assert!(route.selection_ignored);

        clear(app);
    }

    #[test]
    fn error_code_is_recorded_without_dropping_the_last_provider() {
        let app = "active-route-test-error";
        let provider = provider("p1", "Provider One");

        record_selection(app, MODE_SELECTED, &provider, false, None);
        record_error(app, crate::proxy::log_codes::fo::ALL_CIRCUIT_OPEN);

        let route = snapshot(app).unwrap();
        assert_eq!(route.effective_provider_id.as_deref(), Some("p1"));
        assert_eq!(route.last_error_code.as_deref(), Some("FO-004"));

        clear(app);
    }
}
