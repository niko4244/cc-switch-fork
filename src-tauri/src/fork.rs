//! Fork identity and the updater pin.
//!
//! This binary is a local fork of upstream CC Switch. Upstream ships a Tauri
//! updater whose endpoints are baked into `tauri.conf.json`, and on Windows
//! `update.install()` launches the signed NSIS installer and exits the process —
//! so an upstream release can replace this build and silently drop every local
//! patch (routing mode, the Codex live-config repair, the stale-login guard).
//!
//! The pin is deliberately fail-safe. The default mode is [`UpdateMode::Pinned`],
//! and so is the behaviour when the policy file is missing, unreadable or
//! malformed: an operator who never expressed an opinion gets the safe answer.
//! Opting into the official channel is an explicit, recorded decision, and even
//! then the updater is only registered on the next start — see the plugin
//! registration in `lib.rs`.
//!
//! Only the *updater* is pinned. Nothing here disables network checks: the app
//! can still ask upstream what the latest release is, which is what the
//! read-only "check upstream" action reports.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::AppError;

/// Human-facing fork name, shown next to the version in the UI.
pub const FORK_NAME: &str = "CC Switch Fork";

/// The upstream release this fork is based on. `tauri.conf.json` deliberately
/// keeps this same value for the bundle version so installer and registry
/// identity stay stable; the fork identity is carried separately (see
/// [`fork_version_label`]).
pub const FORK_BASE_VERSION: &str = "3.19.1";

/// Upstream commit this fork branched from.
pub const FORK_BASE_COMMIT: &str = "2852962";

/// Bumped for each fork release built on top of the same base.
pub const FORK_SERIAL: u32 = 1;

/// Upstream repository, used by the read-only release check.
pub const UPSTREAM_REPO: &str = "farion1231/cc-switch";

/// Short commit of the fork build itself, injected by `build.rs`.
pub const FORK_COMMIT: &str = env!("CC_SWITCH_FORK_COMMIT");

/// Policy file, next to the database in the app config dir.
pub const POLICY_FILE: &str = "fork-update-policy.json";

/// Stable code returned when the pin refuses an upstream install, so the
/// frontend can localize the message instead of matching prose.
pub const REFUSAL_CODE_PINNED: &str = "FORK_UPDATER_PINNED";

/// Stable code returned when a switch to the official channel is not confirmed.
pub const REFUSAL_CODE_ACK_REQUIRED: &str = "FORK_ACK_REQUIRED";

/// Whether the official updater plugin was registered in this process. Reported
/// to the UI so "pinned" is an observed fact, not a promise.
static UPDATER_REGISTERED: AtomicBool = AtomicBool::new(false);

/// How upstream artifacts may be applied to this install.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateMode {
    /// Upstream artifacts must never replace this build. The default.
    #[default]
    Pinned,
    /// The operator explicitly allowed the official updater channel.
    Official,
}

/// Persisted update policy (`fork-update-policy.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePolicy {
    #[serde(default)]
    pub mode: UpdateMode,
    /// When the operator acknowledged the fork-replacement risk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acknowledged_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self {
            mode: UpdateMode::Pinned,
            acknowledged_at: None,
            note: None,
        }
    }
}

/// What the pin says about applying an upstream artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateDecision {
    RefusePinned,
    AllowOfficial,
}

/// The pin's decision point, kept pure so the refusal is unit-testable without
/// a Tauri app handle or the operator's real policy file.
pub fn update_decision(policy: &UpdatePolicy) -> UpdateDecision {
    match policy.mode {
        UpdateMode::Pinned => UpdateDecision::RefusePinned,
        UpdateMode::Official => UpdateDecision::AllowOfficial,
    }
}

/// The fork's own version label: base release + fork serial + build commit,
/// e.g. `3.19.1+fork.1.03cbb56`. Build metadata, so semver ordering is preserved
/// and upstream comparisons stay meaningful.
pub fn fork_version_label() -> String {
    format!("{}+fork.{}.{}", FORK_BASE_VERSION, FORK_SERIAL, FORK_COMMIT)
}

/// Bundled version string (what `getVersion()` in the frontend returns).
pub fn bundle_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn upstream_url() -> String {
    format!("https://github.com/{UPSTREAM_REPO}")
}

pub fn policy_path() -> PathBuf {
    crate::config::get_app_config_dir().join(POLICY_FILE)
}

/// Read a policy from an explicit path. Missing or malformed input yields the
/// safe default instead of an error: a broken file must never unlock installs.
pub fn read_policy_from(path: &Path) -> UpdatePolicy {
    let Ok(text) = std::fs::read_to_string(path) else {
        return UpdatePolicy::default();
    };
    match serde_json::from_str::<UpdatePolicy>(&text) {
        Ok(policy) => policy,
        Err(e) => {
            log::warn!(
                "fork 更新策略文件无法解析，按 pinned 处理: {}: {e}",
                path.display()
            );
            UpdatePolicy::default()
        }
    }
}

pub fn read_policy() -> UpdatePolicy {
    read_policy_from(&policy_path())
}

/// Persist the policy atomically (write + rename) so the startup read can never
/// observe a half-written file.
pub fn write_policy(policy: &UpdatePolicy) -> Result<PathBuf, AppError> {
    let path = policy_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| AppError::IoContext {
            context: format!("创建 fork 更新策略目录失败: {}", dir.display()),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(policy)
        .map_err(|source| AppError::JsonSerialize { source })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|source| AppError::IoContext {
        context: format!("写入 fork 更新策略失败: {}", tmp.display()),
        source,
    })?;
    std::fs::rename(&tmp, &path).map_err(|source| AppError::IoContext {
        context: format!("替换 fork 更新策略失败: {}", path.display()),
        source,
    })?;
    Ok(path)
}

/// The refusal message for an install attempt, or `None` when it is allowed.
/// Pure, so tests can assert the pin without touching the operator's files.
pub fn refusal_message(policy: &UpdatePolicy) -> Option<String> {
    match update_decision(policy) {
        UpdateDecision::AllowOfficial => None,
        UpdateDecision::RefusePinned => Some(format!(
            "{REFUSAL_CODE_PINNED}: 本构建是本地 fork（{}），更新策略为 pinned，\
             官方更新器不会替换该构建。如需接管官方更新通道，请在 设置 → 关于 中显式开启。",
            fork_version_label()
        )),
    }
}

/// Same as [`refusal_message`], reading the persisted policy.
pub fn refuse_official_install() -> Option<String> {
    refusal_message(&read_policy())
}

pub fn mark_updater_registered() {
    UPDATER_REGISTERED.store(true, Ordering::SeqCst);
}

pub fn updater_registered() -> bool {
    UPDATER_REGISTERED.load(Ordering::SeqCst)
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Everything the UI needs to identify the installed build. `pinned` and
/// `official_updater_registered` are observations, not intentions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkInfo {
    pub is_fork: bool,
    pub name: String,
    pub bundle_version: String,
    pub fork_version: String,
    pub base_version: String,
    pub base_commit: String,
    pub commit: String,
    pub serial: u32,
    pub upstream_repo: String,
    pub upstream_url: String,
    pub update_mode: UpdateMode,
    pub acknowledged_at: Option<String>,
    pub pinned: bool,
    pub official_updater_registered: bool,
    pub refusal_code: Option<String>,
}

pub fn info(policy: &UpdatePolicy) -> ForkInfo {
    let pinned = update_decision(policy) == UpdateDecision::RefusePinned;
    ForkInfo {
        is_fork: true,
        name: FORK_NAME.to_string(),
        bundle_version: bundle_version().to_string(),
        fork_version: fork_version_label(),
        base_version: FORK_BASE_VERSION.to_string(),
        base_commit: FORK_BASE_COMMIT.to_string(),
        commit: FORK_COMMIT.to_string(),
        serial: FORK_SERIAL,
        upstream_repo: UPSTREAM_REPO.to_string(),
        upstream_url: upstream_url(),
        update_mode: policy.mode,
        acknowledged_at: policy.acknowledged_at.clone(),
        pinned,
        official_updater_registered: updater_registered(),
        refusal_code: pinned.then(|| REFUSAL_CODE_PINNED.to_string()),
    }
}

/// Read-only report of the newest upstream release. Never installs anything.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamRelease {
    pub tag: String,
    pub version: String,
    pub published_at: Option<String>,
    pub html_url: Option<String>,
    pub notes: Option<String>,
    /// Compared against the fork's *base* version, since the fork is built on it.
    pub is_newer: bool,
}

pub fn parse_upstream_release(json: &serde_json::Value) -> Option<UpstreamRelease> {
    let tag = json.get("tag_name")?.as_str()?.trim().to_string();
    if tag.is_empty() {
        return None;
    }
    let version = tag.strip_prefix('v').unwrap_or(&tag).to_string();
    if version.is_empty() {
        return None;
    }
    Some(UpstreamRelease {
        is_newer: is_newer_version(&version, FORK_BASE_VERSION),
        tag,
        version,
        published_at: json
            .get("published_at")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        html_url: json
            .get("html_url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        notes: json
            .get("body")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    })
}

/// Numeric triple comparison. Anything unparsable counts as *not newer*, so a
/// malformed or exotic tag can never be presented as an urgent upgrade.
pub fn is_newer_version(candidate: &str, current: &str) -> bool {
    match (
        parse_version_triple(candidate),
        parse_version_triple(current),
    ) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn parse_version_triple(version: &str) -> Option<(u64, u64, u64)> {
    let trimmed = version.trim();
    let core = trimmed.trim_start_matches('v');
    // Drop pre-release/build metadata: 3.20.0-rc.1 is still newer than 3.19.1.
    let core = core.split(['-', '+']).next().unwrap_or(core);
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "cc-switch-fork-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn default_policy_is_pinned() {
        let policy = UpdatePolicy::default();
        assert_eq!(policy.mode, UpdateMode::Pinned);
        assert_eq!(update_decision(&policy), UpdateDecision::RefusePinned);
    }

    #[test]
    fn a_missing_policy_file_is_pinned() {
        let dir = TempDir::new("missing");
        let policy = read_policy_from(&dir.file("nope.json"));
        assert_eq!(policy, UpdatePolicy::default());
        assert!(refuse_official_install_message(&policy));
    }

    #[test]
    fn a_corrupt_policy_file_cannot_unlock_installs() {
        let dir = TempDir::new("corrupt");
        let path = dir.file("fork-update-policy.json");
        // Every shape an operator or a half-finished write could leave behind.
        for junk in [
            "",
            "{",
            "not json at all",
            "\"official\"",
            "{\"mode\":\"yolo\"}",
            "{\"mode\":null}",
            "[]",
        ] {
            std::fs::write(&path, junk).expect("write junk policy");
            let policy = read_policy_from(&path);
            assert_eq!(
                policy.mode,
                UpdateMode::Pinned,
                "junk {junk:?} must not unlock the official updater"
            );
            assert!(refuse_official_install_message(&policy));
        }
    }

    #[test]
    fn an_explicit_official_policy_round_trips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let path = dir.file("fork-update-policy.json");
        let policy = UpdatePolicy {
            mode: UpdateMode::Official,
            acknowledged_at: Some("2026-09-29T00:00:00+00:00".to_string()),
            note: Some("operator opted in".to_string()),
        };

        std::fs::write(&path, serde_json::to_string_pretty(&policy).unwrap()).unwrap();
        let read = read_policy_from(&path);

        assert_eq!(read, policy);
        assert_eq!(update_decision(&read), UpdateDecision::AllowOfficial);
        assert!(refusal_message(&read).is_none());
    }

    #[test]
    fn a_policy_file_missing_the_mode_field_is_pinned() {
        let dir = TempDir::new("no-mode");
        let path = dir.file("fork-update-policy.json");
        std::fs::write(&path, "{\"note\":\"looks official-ish\"}").unwrap();
        assert_eq!(read_policy_from(&path).mode, UpdateMode::Pinned);
    }

    #[test]
    fn the_refusal_names_the_pin_so_the_ui_can_localize_it() {
        let message = refusal_message(&UpdatePolicy::default()).expect("pinned refuses");
        assert!(message.starts_with(REFUSAL_CODE_PINNED));
        assert!(message.contains(&fork_version_label()));
    }

    #[test]
    fn fork_version_label_identifies_base_serial_and_commit() {
        let label = fork_version_label();
        assert!(label.starts_with(FORK_BASE_VERSION));
        assert!(label.contains(&format!("+fork.{FORK_SERIAL}.")));
        assert!(label.ends_with(FORK_COMMIT));
    }

    #[test]
    fn the_bundle_version_matches_the_stable_base_release() {
        // The bundle version is what the installer and the registry key use, so
        // it must not silently drift into a fork-specific string.
        assert_eq!(bundle_version(), FORK_BASE_VERSION);
    }

    #[test]
    fn fork_info_reports_the_pin_as_an_observed_fact() {
        let pinned = info(&UpdatePolicy::default());
        assert!(pinned.is_fork);
        assert!(pinned.pinned);
        assert_eq!(pinned.refusal_code.as_deref(), Some(REFUSAL_CODE_PINNED));
        // The plugin is not registered in a unit test process.
        assert!(!pinned.official_updater_registered);

        let official = info(&UpdatePolicy {
            mode: UpdateMode::Official,
            ..Default::default()
        });
        assert!(!official.pinned);
        assert!(official.refusal_code.is_none());
    }

    #[test]
    fn upstream_release_parsing_reads_the_github_shape() {
        let json = serde_json::json!({
            "tag_name": "v3.20.0",
            "published_at": "2026-09-20T10:00:00Z",
            "html_url": "https://github.com/farion1231/cc-switch/releases/tag/v3.20.0",
            "body": "notes"
        });
        let release = parse_upstream_release(&json).expect("parses");
        assert_eq!(release.version, "3.20.0");
        assert_eq!(release.tag, "v3.20.0");
        assert!(release.is_newer);
        assert_eq!(release.notes.as_deref(), Some("notes"));
    }

    #[test]
    fn upstream_release_at_or_below_the_base_is_not_newer() {
        for tag in ["v3.19.1", "3.19.1", "v3.19.0", "v3.18.9"] {
            let json = serde_json::json!({ "tag_name": tag });
            let release = parse_upstream_release(&json).expect("parses");
            assert!(!release.is_newer, "{tag} must not read as an upgrade");
        }
    }

    #[test]
    fn a_tag_we_cannot_parse_is_never_an_upgrade() {
        for tag in ["", "nightly", "release-notes", "vNext", "v"] {
            let json = serde_json::json!({ "tag_name": tag });
            assert!(
                parse_upstream_release(&json).map(|r| r.is_newer) != Some(true),
                "{tag:?} must not be presented as an upgrade"
            );
        }
        assert!(!is_newer_version("nightly", "3.19.1"));
        assert!(!is_newer_version("3.19.1", "3.19.1"));
    }

    #[test]
    fn version_comparison_ignores_prerelease_and_prefix_noise() {
        assert!(is_newer_version("3.20.0-rc.1", "3.19.1"));
        assert!(is_newer_version("v4", "3.19.1"));
        assert!(is_newer_version("3.19.2", "3.19.1"));
        assert!(!is_newer_version("3.9.9", "3.19.1"));
    }

    /// Small shim so the assertions above read as intent rather than Option
    /// juggling: "would this policy refuse an install?".
    fn refuse_official_install_message(policy: &UpdatePolicy) -> bool {
        refusal_message(policy).is_some()
    }
}
