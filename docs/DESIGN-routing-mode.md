# cc-switch: Routing Mode redesign

Design spec for a local fork of **cc-switch v3.19.1** (commit `2852962`).
Scope: Codex (ChatGPT desktop + CLI) and Claude Desktop first, then Claude Code / Gemini / Grok Build.
Status: spec only — implementation follows approval.

---

## 1. Why this exists

Four independent defects made the cc-switch header ambiguous, and each one silently
mis-routed traffic rather than failing loudly. All are present in v3.19.1.

| # | Defect | Where | Observed effect |
|---|---|---|---|
| D1 | With failover on, the router **ignores the selected provider** and routes down the queue from P1 | `src-tauri/src/proxy/provider_router.rs` | Panel selection had no effect; every request went to queue P1 |
| D2 | Two header switches, both rendered `text-emerald-500 animate-pulse` when on | `src/components/proxy/ProxyToggle.tsx`, `FailoverToggle.tsx` | Looked like one ambiguous on/off control |
| D3 | Failover switch is hidden by default, then appears unannounced | `settings.rs:519` `enable_failover_toggle: false`; gated at `App.tsx:1264` | A second mystery switch appeared after an Advanced-settings change |
| D4 | "Restore native" exists only as the *off* state of takeover, restoring a possibly stale snapshot | `ProxyToggle.tsx`; `commands/proxy.rs::stop_proxy_with_restore` | Turning the switch off overwrote config with an old snapshot |

Two compounding gaps:

- **Routing errors never reach the UI.** `FO-004` / `FO-005` are defined in
  `proxy/log_codes.rs:45-46`, raised in `provider_router.rs:100-104`, and mapped to a
  hardcoded Chinese string `未配置供应商` in `proxy/error_mapper.rs:80`. The frontend has
  no path to them.
- **Selected ≠ serving.** `ProviderList.tsx:53,75,407` renders `currentProviderId`, which is
  the *selection*. Under failover this is not what answers the request.

And one latent correctness bug that bit the ChatGPT login specifically:

```rust
// src-tauri/src/proxy/providers/codex.rs:225
pub fn is_codex_official_provider(provider: &Provider) -> bool {
    provider.id == crate::database::CODEX_OFFICIAL_PROVIDER_ID
        && provider.category.as_deref() == Some("official")
}
```

Official status is keyed on a **hardcoded id** as well as the category. An imported or renamed
official row is therefore not "official" to the rest of the app, and the proxy-takeover guard
(`official_provider_supports_proxy_takeover`) will not treat it as such. A real ChatGPT login
sat in a row cc-switch did not recognise, which is exactly how the login-preservation logic ends
up writing over `~/.codex/auth.json`.

---

## 2. Design principles

1. **One control per app.** Routing intent is a single decision, not three switches.
2. **State is visible without hovering.** No behaviour may be explained only in a `title=`
   tooltip. Both existing toggles fail this.
3. **Never fail silently.** A routing error the user cannot see is indistinguishable from a
   working setup. Every error path must surface in the UI.
4. **The app owns its own login.** Switching to a third-party provider must never destroy the
   ChatGPT / Claude sign-in without an explicit, informed confirmation.
5. **Show the real target.** Display the upstream that will actually answer, not the one
   selected.
6. **Reversible.** Every state change is undoable, and the restore path never depends on a
   snapshot whose age we don't disclose.

---

## 3. The Routing Mode model

One control, three exclusive modes. This is a **frontend state machine over existing backend
flags** — it introduces no new backend concept.

| Mode | `takeover` | `auto_failover_enabled` | Meaning |
|---|---|---|---|
| `native` | off | off | App uses its own sign-in. cc-switch does not touch its config. |
| `selected` | on | off | Proxy in front; traffic goes to the provider chosen in the list. |
| `failover` | on | on | Proxy in front; traffic follows the queue P1 → P2 → … |

**Transition rules**

- `native` → clear failover, then disable takeover **and restore** (see §5.3).
- `selected` → enable takeover; if failover was on, turn it off and tell the user the queue
  stopped being authoritative.
- `failover` → requires takeover. If takeover is off, enable it first (the current
  `FailoverToggle` just disables the switch, which is a dead end).
- Any mode change to `native` or `failover` that would rewrite a sign-in file must run the
  login-safety confirmation (§5.4) first.

**Why `selected` is the default when takeover is first enabled:** it matches the user's mental
model ("I picked a provider, use it") and is the only mode with no hidden queue.

---

## 4. Per-app surfaces

| App | Today | Target |
|---|---|---|
| **Codex** (ChatGPT desktop + CLI) | `ProxyToggle` + `FailoverToggle` + provider radio | Full three-mode control, status strip, login-safety chip, restore dialog |
| **Claude Desktop** | `ClaudeDesktopRouteToggle` only (`App.tsx:1257-1261`); no takeover switch, no failover switch, so **"native" is not currently expressible** | Same three-mode control. Largest single win for this app |
| Claude Code | Same as Codex | Same treatment, second pass |
| Gemini / Grok Build | Same as Codex | Same treatment, mechanical |
| OpenCode / OpenClaw / Hermes | Toggles not rendered (`App.tsx:1251-1253`) | Out of scope |

Claude Desktop is called out because it is currently the *least* controllable: it can be
routed but cannot be left alone, and its official row is subject to the same id-based
official-detection bug (`claude-desktop-official`, `providers_seed.rs:14`).

---

## 5. UI specification

### 5.1 `RoutingModeControl` (new)

Replaces `ProxyToggle` + `FailoverToggle` in the header. Mounted where they are today
(`App.tsx:1248-1268`).

```
┌─ Codex (ChatGPT) ─────────────────────────────────────────────┐
│  Routing   [ Native ] [ Selected provider ] [ Failover ]      │
│  Live      Brainz Chain → http://127.0.0.1:3200/v1            │
│  Sign-in   ✓ ChatGPT login preserved                          │
└────────────────────────────────────────────────────────────────┘
```

- Segmented control, not a switch. Selected segment carries the state colour.
- **Native** is first and visually neutral — it is the safe default, not an "off".
- Disabled segments explain themselves inline rather than via tooltip.
- Props: `activeApp: AppId`, `disabled?: boolean`, `onModeChange(mode)`.
- Reads existing hooks: `useProxyStatus()` (takeover status, server address) and
  `useAutoFailoverEnabled` / `useSetAutoFailoverEnabled` from `@/lib/query/failover`.
- a11y: `role="radiogroup"`, arrow-key navigation, `aria-describedby` pointing at the status line.

### 5.2 `RouteStatusStrip` (new)

Always visible under the control. Answers: *which upstream will actually answer?*

```
selected : Brainz Chain → http://127.0.0.1:3200/v1
failover : queue P1 LiteLLM Gateway → http://127.0.0.1:4000/v1   (selection ignored)
```

The `failover` variant renders a warning tone and the words **"selection ignored"** whenever
queue P1 ≠ the selected provider. This single line would have made the original bug obvious in
under a second.

Data source: new `get_active_route` command (§6.3). Until that exists, fall back to composing
`currentProviderId` + failover state, and label it as *predicted* rather than *actual*.

### 5.3 `RestoreNativeDialog` (new, replaces D4)

Entering `native` from a proxy mode opens a dialog that:

1. Lists exactly which files will be written, per app:

   | App | Files |
   |---|---|
   | Codex | `~/.codex/config.toml`, `~/.codex/auth.json` |
   | Claude Code | `~/.claude/settings.json`, `~/.claude.json` |
   | Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` |

2. States the **provenance** of the restore: *"restoring the config saved by takeover on
   2026-09-28 13:19"*, or, when no usable snapshot exists, *"no takeover snapshot found —
   cc-switch will write a clean native config"*.
   **Snapshot age must be surfaced.** Restoring a months-old snapshot is how a broken config
   survives a "fix".
3. Offers two actions:
   - **Restore snapshot** — existing `stop_proxy_with_restore` (`commands/proxy.rs:40`).
   - **Write clean native config** — default when the snapshot is missing or older than the
     provider's last change; does not depend on the snapshot at all.
4. Always writes a timestamped backup into `~/.cc-switch/backups/restore-<ts>/` first.

### 5.4 `LoginSafetyChip` (new, Codex / Claude Desktop)

```
✓ ChatGPT login preserved        (auth.json untouched)
⚠ This switch will overwrite auth.json
```

Derived from three facts, all already available:

- `preserveCodexOfficialAuthOnSwitch` (`settings.json`)
- whether the current row satisfies "official" — **after §6.2 fixes this, by category**
- whether the target app's auth file is written by the current path
  (`write_codex_live_for_provider` writes `auth.json` only when
  `official && has_login_material`, or when not-official and not preserving)

If a switch *would* overwrite a sign-in file, require an explicit confirmation that names the
file. Never silently.

### 5.5 `RoutingErrorBanner` (new)

Surfaces `FO-004` / `FO-005` in the UI with actionable text:

| Code | Current message | Proposed |
|---|---|---|
| `FO-005` | `未配置供应商` (503) | "No provider selected and the failover queue is empty. Pick a provider, or add one to the queue." |
| `FO-004` | all circuits open | "All {n} providers are circuit-broken. Re-enable one in the provider list, or wait for the cooldown." |

Backend must emit a structured code the frontend can switch on (`handlers.rs:1885` already
produces `cc_switch_no_providers_configured` — reuse that shape).

### 5.6 Visual tokens

| State | Colour | Never |
|---|---|---|
| Native | neutral / muted | — |
| Selected (takeover on) | emerald | shared with any other state |
| Failover | **amber** | emerald (this is the D2 fix) |
| Error / selection ignored | red | — |

### 5.7 i18n

New keys under `src/i18n/locales/`, all five locales. Existing keys to retire once the new
control lands: `proxy.takeover.tooltip.*`, `failover.tooltip.*`,
`settings.advanced.proxy.enableFailoverToggle*`. Keep `enableFailoverToggle` as a *gate* for
displaying the control at all, or drop the gate — it currently exists only to hide the problem
(D3); the recommended fix is to always show the control and let the mode express the choice.

---

## 6. Backend specification (Slice 2)

### 6.1 Router honours the selection under failover — `proxy/provider_router.rs`

Today the failover branch builds candidates from the queue only. Change it so the **selected
provider is always candidate 0**, and the queue supplies the fallbacks:

```rust
// pseudocode
let mut candidates = Vec::new();
if let Some(cur) = current_provider(app) { candidates.push(cur); }   // NEW
if failover_enabled {
    for p in queue_in_priority_order() {
        if !candidates.contains(p) { candidates.push(p); }
    }
}
```

Guards: keep the existing empty-candidate and all-circuits-open errors, and log which candidate
won. Behaviour change for anyone relying on "failover jumps to P1" — hence the settings escape
hatch `failoverPrefersSelected: bool` (default `true`), documented in the changelog.

### 6.2 Official detection by category — `proxy/providers/codex.rs`, `database/dao/providers*.rs`

```rust
// was: id == CODEX_OFFICIAL_PROVIDER_ID && category == "official"
pub fn is_codex_official_provider(provider: &Provider) -> bool {
    provider.category.as_deref() == Some("official")
}
```

Keep the id check only where a *specific* seed row is required (e.g. takeover support). Add an
idempotent **duplicate-official merge** run at startup and after import/deeplink add: if an app
has more than one `category = "official"` row, keep the one with the most recent
`last_refresh` in its auth, merge its `settings_config` into the canonical seeded row, and
archive the other. This is the automated version of the manual `default` / `codex-official`
surgery, and it should have prevented the bug rather than requiring it.

### 6.3 `get_active_route` (new command)

No route/telemetry command exists today (verified: no `last_route` / `active_route` /
`current_target` in `src-tauri`). Add to `commands/proxy.rs`:

```rust
#[derive(Serialize)]
pub struct ActiveRoute {
    pub app_type: String,
    pub mode: String,            // native | selected | failover
    pub effective_provider_id: Option<String>,
    pub effective_provider_name: Option<String>,
    pub upstream_base_url: Option<String>,
    pub selection_ignored: bool, // true when failover picked something other than the selection
    pub last_switch_at: Option<i64>,
    pub last_error_code: Option<String>,   // FO-004 / FO-005
}
```

The router already logs `[Codex] >>> 请求目标: <url>`; store that decision in app state when it
is made instead of only logging it.

### 6.4 Structured routing errors — `proxy/error.rs`, `error_mapper.rs`, `handlers.rs`, `log_codes.rs`

Attach `code` + `remedy` to `ProxyError::NoProvidersConfigured` and the all-circuits-open
variant, return them through `handlers.rs`, and stop relying on the hardcoded Chinese string in
`error_mapper.rs:80` for anything user-facing.

### 6.5 Codex live-config block rename — `services/provider/live.rs`, `codex_config.rs`

`apply_codex_proxy_toml_config_for_provider` patches only `base_url`, `wire_api` and `model` in
the live `~/.codex/config.toml`, so the block key keeps whatever it was:

```toml
model_provider = "litellm"   # left over from a previous provider
model = "brainz-chain"
base_url = "http://127.0.0.1:15721/v1"
```

Rewrite the `[model_providers.<key>]` block to match the selected provider and update
`model_provider` to match. A live file that names one provider and routes to another is a
recurring time sink.

---

## 7. New settings

| Key | Default | Meaning |
|---|---|---|
| `routingMode.<app>` | `"selected"` when takeover already on, else `"native"` | Persisted intent; the control is stateless otherwise |
| `failoverPrefersSelected` | `true` | §6.1 behaviour switch |
| `restorePrefersCleanNative` | `true` | §5.3 default action |
| `confirmSignInOverwrite` | `true` | §5.4 confirmation gate |

Migrating existing installs: derive the initial mode from
`takeover[app] && auto_failover_enabled[app]` → `failover`, `takeover[app]` → `selected`, else
`native`. Idempotent, no data loss.

---

## 8. DB repair (idempotent, ships with the fork)

Run once on upgrade. Mirrors what was done by hand on this machine:

```sql
-- exactly one current provider per app
UPDATE providers SET is_current = 0 WHERE is_current = 1;

-- merge duplicate official rows into the seeded one, then drop them
DELETE FROM providers
 WHERE app_type = 'codex' AND id = 'default'
   AND category = 'official'
   AND EXISTS (SELECT 1 FROM providers o
                WHERE o.app_type = 'codex' AND o.id = 'codex-official'
                  AND json_array_length(json_extract(o.settings_config, '$.auth.tokens')) >= 0);
```

The real merge must compare `auth.tokens` freshness (`last_refresh`), not assume the survivor:
on this machine the row that looked like debris (`default`, login from 2026-07-24) was the
**stale** one, and the canonical row had the fresher login. Never delete the fresher login.

Also enforce: `in_failover_queue` is meaningless when failover is off, so on migration clear it
for the app whose failover we are not enabling.

---

## 9. Test plan

**Unit (Rust)**
- `select_providers` with failover on: asserts candidate 0 is the selected provider (§6.1).
- `is_codex_official_provider` with a renamed official row: asserts `true` by category (§6.2).
- Duplicate-official merge keeps the row with the newer `last_refresh`.
- TOML rewrite: selected provider name appears in `model_provider` and the block key (§6.5).

**Unit (frontend)**
- `RoutingModeControl` renders three modes, honours disabled reasons.
- Native → restores; Failover with empty queue → shows inline error, no dead-end switch.
- `RouteStatusStrip` shows "selection ignored" when P1 ≠ selection.
- `RestoreNativeDialog` shows snapshot age and picks the clean-native default when stale.

**Integration**
- Codex: `selected` → request to `:15721` reaches the configured upstream; `native` → live
  config restored, `auth.json` untouched.

**Manual E2E (the check that actually caught this)**

```bash
curl -s -X POST http://127.0.0.1:15721/v1/responses \
  -H "Authorization: Bearer PROXY_MANAGED" -H "Content-Type: application/json" \
  -d '{"model":"brainz-chain","input":"reply with exactly: CCSWITCH-OK"}'
grep "请求目标" ~/.cc-switch/logs/cc-switch.log | tail -1
```

Pass = HTTP 200 **and** the log line naming the intended upstream. Assert the upstream, not just
the status code — a 200 from the wrong provider is the failure we shipped for a week.

---

## 10. Build and rollout

- `pnpm` is **missing** on this machine and the repo is a pnpm workspace → `corepack enable pnpm`
  first. `cargo 1.95`, `rustc 1.95`, `node 22.18`, `npm 10.9` are present.
- C: has ~15 GB free, which is not enough. Build with:
  - `CARGO_TARGET_DIR=I:\cc-build\target` (I: has ~40 GB) or `H:\cc-build\target` (~96 GB)
  - pnpm store on `H:` (e.g. `pnpm config set store-dir H:\pnpm-store`)
- Back up before first run of the new build: `~/.cc-switch/cc-switch.db`, `settings.json`,
  `~/.codex/config.toml`, `~/.codex/auth.json`. A Tauri build replaces the installed binary.
- Rollback = reinstall the previous `cc-switch.exe` and restore those four files.

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| §6.1 changes routing for existing failover users | Settings escape hatch, default documented in changelog |
| "Write clean native config" produces a config the app rejects | Prefer snapshot restore when fresh; treat clean-native as opt-in until proven |
| Official-by-category widens what counts as official | Restrict to `category = "official"` only; keep id checks for takeover support |
| Deleting the header switches breaks muscle memory / docs | Keep `enableFailoverToggle` as an alias for one release; note in changelog |
| Larger diff → harder upstream merge | Land §5 and §6 as separate commits; §5 is frontend-only |

---

## 12. Commit sequence

1. `feat(ui): add RoutingModeControl, replace ProxyToggle + FailoverToggle` (frontend only)
2. `feat(ui): add RouteStatusStrip with predicted routing` (no backend dependency)
3. `feat(ui): add RestoreNativeDialog with snapshot provenance + backup`
4. `feat(ui): add LoginSafetyChip and RoutingErrorBanner`
5. `feat(proxy): prefer selected provider under failover` (§6.1, behind the setting)
6. `fix(providers): treat category=official as official; merge duplicates` (§6.2)
7. `feat(proxy): add get_active_route and store routing decisions` (§6.3, §6.4)
8. `fix(codex): rename live model_providers block on switch` (§6.5)
9. `chore(db): idempotent repair migration` (§8)
