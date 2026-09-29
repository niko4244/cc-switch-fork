import { invoke } from "@tauri-apps/api/core";
import type { RoutingMode } from "@/lib/routing-mode";

/** Last routing decision the proxy actually made for an app (spec §6.3). */
export interface ActiveRoute {
  appType: string;
  mode: Exclude<RoutingMode, "native"> | RoutingMode;
  effectiveProviderId: string | null;
  effectiveProviderName: string | null;
  upstreamBaseUrl: string | null;
  /** Failover routed to something other than the selected provider. */
  selectionIgnored: boolean;
  /** Unix seconds of the last change of the effective provider. */
  lastSwitchAt: number | null;
  /**
   * Unix seconds of the last time the router decided anything for this app.
   *
   * This is the freshness signal, and it is *not* `lastSwitchAt`: that only
   * moves when the provider changes, so a route serving steadily for days still
   * has an ancient switch time.
   */
  lastConfirmedAt: number | null;
  /** FO-004 / FO-005 when the last routing attempt failed. */
  lastErrorCode: string | null;
}

export const routingApi = {
  /**
   * Resolve the route that will actually answer for `appType`.
   *
   * `null` means the proxy has not routed a request for this app in this
   * process yet — callers must fall back to a *predicted* route rather than
   * presenting a guess as fact.
   */
  async getActiveRoute(appType: string): Promise<ActiveRoute | null> {
    return invoke("get_active_route", { appType });
  },
};
