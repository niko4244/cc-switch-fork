/**
 * RouteStatusStrip — answers the question the header never answered: *which
 * upstream will actually answer this app's requests?*
 *
 * WHY this exists: with failover on, the router ignores the selected provider
 * and routes down the queue from P1, and nothing on screen said so — the
 * provider radio looked broken. One line closes that gap (see
 * docs/DESIGN-routing-mode.md §5.2).
 *
 * Data source is the `get_active_route` command (§6.3), which reports the route
 * the proxy *actually* used. Before the proxy has routed anything for this app
 * the command returns null, and the strip falls back to the route the current
 * mode implies — labelled `predicted` so a guess is never shown as a fact.
 */

import { ArrowRight, AlertTriangle } from "lucide-react";
import { useTranslation } from "react-i18next";

import { useProvidersQuery } from "@/lib/query/queries";
import { useActiveRoute } from "@/lib/query/routing";
import type { RoutingMode } from "@/lib/routing-mode";
import { cn } from "@/lib/utils";
import type { AppId } from "@/lib/api/types";

interface RouteStatusStripProps {
  activeApp: AppId;
  mode: RoutingMode;
  className?: string;
}

/** FO-004 / FO-005 → the copy that explains them. */
const ERROR_MESSAGES: Record<string, string> = {
  "FO-004": "Every candidate provider is circuit-broken",
  "FO-005": "No provider to route to — pick one, or add one to the queue",
};

export function RouteStatusStrip({
  activeApp,
  mode,
  className,
}: RouteStatusStripProps) {
  const { t } = useTranslation();
  const { data: actual } = useActiveRoute(activeApp, {
    enabled: mode !== "native",
  });
  const { data: providersData } = useProvidersQuery(activeApp);

  if (mode === "native") {
    return (
      <div
        className={cn(
          "flex items-center gap-1 px-1 text-[11px] text-muted-foreground",
          className,
        )}
        aria-live="polite"
      >
        <span>{t("routing.native", { defaultValue: "Native" })}</span>
        <span>·</span>
        <span>
          {t("routing.status.nativeHint", {
            defaultValue: "using its own upstream",
          })}
        </span>
      </div>
    );
  }

  const predictedProviderId = providersData?.currentProviderId ?? "";
  const predictedProvider = providersData?.providers[predictedProviderId];

  const providerName =
    actual?.effectiveProviderName ?? predictedProvider?.name ?? null;
  const upstreamUrl = actual?.upstreamBaseUrl ?? null;
  const selectionIgnored = actual?.selectionIgnored ?? false;
  const errorMessage = actual?.lastErrorCode
    ? ERROR_MESSAGES[actual.lastErrorCode]
    : undefined;
  // No recorded decision yet: what is shown is derived from the mode, not from
  // an actual request, so say so.
  const isPredicted = !actual;

  return (
    <div
      className={cn(
        "flex items-center gap-1 px-1 text-[11px] min-w-0",
        className,
      )}
      aria-live="polite"
    >
      <span
        className={cn(
          "shrink-0 font-medium",
          mode === "failover"
            ? "text-amber-700 dark:text-amber-400"
            : "text-emerald-700 dark:text-emerald-400",
        )}
      >
        {t(`routing.${mode}`, {
          defaultValue: mode === "failover" ? "Failover" : "Selected",
        })}
      </span>
      <span className="shrink-0 text-muted-foreground">·</span>
      <span className="truncate text-foreground/80">
        {providerName ??
          t("routing.status.noProvider", {
            defaultValue: "no provider selected",
          })}
      </span>
      {upstreamUrl && (
        <>
          <ArrowRight className="h-3 w-3 shrink-0 text-muted-foreground" />
          <span className="truncate font-mono text-muted-foreground">
            {upstreamUrl}
          </span>
        </>
      )}
      {isPredicted && (
        <span className="shrink-0 rounded bg-muted px-1 text-muted-foreground">
          {t("routing.status.predicted", { defaultValue: "predicted" })}
        </span>
      )}
      {selectionIgnored && (
        <span
          className="flex shrink-0 items-center gap-0.5 rounded bg-amber-500/15 px-1 text-amber-700 dark:text-amber-400"
          title={t("routing.status.selectionIgnoredHint", {
            defaultValue:
              "Failover is routing this app down the queue, so the provider you selected is not in use.",
          })}
        >
          <AlertTriangle className="h-3 w-3" />
          {t("routing.status.selectionIgnored", {
            defaultValue: "selection ignored",
          })}
        </span>
      )}
      {errorMessage && (
        <span className="shrink-0 rounded bg-red-500/15 px-1 text-red-700 dark:text-red-400">
          {errorMessage} ({actual?.lastErrorCode})
        </span>
      )}
    </div>
  );
}
