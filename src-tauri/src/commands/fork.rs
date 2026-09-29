//! Commands exposing the fork identity and the updater pin.

use crate::fork::{self, UpdateMode, UpdatePolicy, UpstreamRelease};

/// Identity of the installed build, for the About panel and for diagnostics.
#[tauri::command]
pub async fn get_fork_info() -> Result<fork::ForkInfo, String> {
    Ok(fork::info(&fork::read_policy()))
}

#[tauri::command]
pub async fn get_fork_update_policy() -> Result<UpdatePolicy, String> {
    Ok(fork::read_policy())
}

/// Change how upstream artifacts may be applied.
///
/// Moving to `official` requires `acknowledge = true`: the caller has seen that
/// the official updater replaces this patched build. Moving back to `pinned`
/// needs no confirmation. The plugin registration happens at startup, so a
/// switch takes effect after a restart — which is intentional: the running
/// process should not acquire a replacement capability mid-session.
#[tauri::command]
pub async fn set_fork_update_policy(
    mode: UpdateMode,
    acknowledge: bool,
) -> Result<UpdatePolicy, String> {
    if mode == UpdateMode::Official && !acknowledge {
        return Err(format!(
            "{}: 切换到 official 更新通道需要显式确认",
            fork::REFUSAL_CODE_ACK_REQUIRED
        ));
    }

    let policy = UpdatePolicy {
        mode,
        acknowledged_at: (mode == UpdateMode::Official).then(fork::now_rfc3339),
        note: Some(match mode {
            UpdateMode::Official => {
                "operator allowed the official updater to replace this fork build".to_string()
            }
            UpdateMode::Pinned => "fork build pinned against upstream releases".to_string(),
        }),
    };

    let path = fork::write_policy(&policy).map_err(|e| e.to_string())?;
    log::info!("fork 更新策略已写入 {}: {:?}", path.display(), policy.mode);

    Ok(policy)
}

/// Ask upstream what its newest release is. Read-only: this never downloads or
/// installs anything, which is the whole point — the pin lives on the install
/// path, not on the ability to look.
#[tauri::command]
pub async fn check_upstream_changes() -> Result<Option<UpstreamRelease>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("初始化 HTTP 客户端失败: {e}"))?;

    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        fork::UPSTREAM_REPO
    );
    let response = client
        .get(&url)
        .header("User-Agent", "cc-switch-fork")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("查询上游版本失败: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("查询上游版本失败: HTTP {}", response.status()));
    }

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析上游版本失败: {e}"))?;

    Ok(fork::parse_upstream_release(&json))
}
