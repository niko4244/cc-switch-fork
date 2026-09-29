/**
 * Routing mode — the single control that replaces cc-switch's two header switches.
 *
 * WHY this exists: the header had two independent switches (ProxyToggle for
 * takeover, FailoverToggle for auto-failover) that both render emerald when on,
 * so they read as one ambiguous control. Worse, while failover is on the router
 * ignores the selected provider entirely and routes down the queue from P1, which
 * made the provider selection look broken. One explicit mode removes the
 * ambiguity. See docs/DESIGN-routing-mode.md for the full rationale.
 *
 * This module is intentionally free of React and i18n so the state machine can be
 * unit-tested directly.
 */

export type RoutingMode = "native" | "selected" | "failover";

/**
 * Apps that can be routed through cc-switch's local proxy.
 *
 * `claude-desktop` is deliberately absent: it has no takeover support in the
 * backend (no proxy_config row, not handled by the proxy router), so it cannot
 * offer a "native vs proxied" choice yet. It keeps its own route toggle until
 * the backend gains takeover support for it.
 */
export const PROXY_ROUTABLE_APPS = [
  "claude",
  "codex",
  "gemini",
  "grokbuild",
] as const;

export type ProxyRoutableApp = (typeof PROXY_ROUTABLE_APPS)[number];

export function isProxyRoutable(app: string): app is ProxyRoutableApp {
  return (PROXY_ROUTABLE_APPS as readonly string[]).includes(app);
}

/**
 * Collapse the two backend flags into the single mode the user selects.
 *
 * `failover` only exists while takeover is on, because the router needs the proxy
 * in front of the app to have anything to fail over between.
 */
export function deriveRoutingMode(
  takeover: boolean,
  failover: boolean,
): RoutingMode {
  if (!takeover) return "native";
  return failover ? "failover" : "selected";
}

export interface RoutingModePlan {
  /** Value for `takeover[app]`. */
  takeover: boolean;
  /** Value for `auto_failover_enabled`. */
  failover: boolean;
}

/**
 * The flags a mode change has to set.
 *
 * Switching to `native` clears failover first: leaving failover on with takeover
 * off is the state that produced "[FO-005] no provider configured" / HTTP 503,
 * because the router takes the failover branch, finds an empty candidate list and
 * never falls back to the selected provider.
 */
export function planRoutingModeChange(next: RoutingMode): RoutingModePlan {
  switch (next) {
    case "native":
      return { takeover: false, failover: false };
    case "selected":
      return { takeover: true, failover: false };
    case "failover":
      return { takeover: true, failover: true };
  }
}

/**
 * True when the mode change should be treated as leaving failover's queue in
 * charge of routing — used to warn that the provider selection is no longer
 * authoritative.
 */
export function modeIgnoresSelection(next: RoutingMode): boolean {
  return next === "failover";
}

/**
 * How long a recorded route still counts as confirmed.
 *
 * Past this the record is presented as history rather than as the current
 * route: nothing has confirmed it recently, so it must not pass as current.
 */
export const ROUTE_STALE_AFTER_SECONDS = 5 * 60;

/** Under this the age is not worth showing — the route is simply "just now". */
export const ROUTE_FRESH_WITHIN_SECONDS = 10;

/**
 * Compact age of a confirmation, e.g. `42s`, `5m`, `3h`, `2d`.
 *
 * Returns `null` while the route is fresh enough that an age would be noise.
 */
export function describeRouteAge(
  confirmedAtSeconds: number,
  nowSeconds: number,
): string | null {
  const age = Math.max(0, Math.floor(nowSeconds - confirmedAtSeconds));
  if (age < ROUTE_FRESH_WITHIN_SECONDS) return null;
  if (age < 60) return `${age}s`;
  if (age < 3600) return `${Math.floor(age / 60)}m`;
  if (age < 86400) return `${Math.floor(age / 3600)}h`;
  return `${Math.floor(age / 86400)}d`;
}

/**
 * Whether a recorded route has gone unconfirmed for long enough that it must be
 * shown as stale instead of as the route actually in use.
 */
export function isRouteStale(
  confirmedAtSeconds: number,
  nowSeconds: number,
): boolean {
  return nowSeconds - confirmedAtSeconds > ROUTE_STALE_AFTER_SECONDS;
}
