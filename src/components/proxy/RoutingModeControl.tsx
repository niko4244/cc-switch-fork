/**
 * RoutingModeControl — replaces ProxyToggle + FailoverToggle with one explicit choice.
 *
 * WHY: the header used to show two independent switches that both rendered emerald
 * when on, so they read as a single ambiguous on/off control. And because the
 * failover branch of the router ignores the selected provider entirely, turning
 * the second switch on silently made the provider selection stop mattering — with
 * nothing on screen saying so. One segmented control states the intent directly.
 *
 * See docs/DESIGN-routing-mode.md for the full rationale.
 */

import { useCallback, useRef } from "react";
import { Loader2, Route, Zap, Shuffle, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";

import { useProxyStatus } from "@/hooks/useProxyStatus";
import {
  useAutoFailoverEnabled,
  useSetAutoFailoverEnabled,
} from "@/lib/query/failover";
import {
  deriveRoutingMode,
  planRoutingModeChange,
  type RoutingMode,
} from "@/lib/routing-mode";
import { cn } from "@/lib/utils";
import type { AppId } from "@/lib/api/types";

interface ModeOption {
  mode: RoutingMode;
  icon: LucideIcon;
  /** Applied to the active segment. Failover is amber, never emerald: emerald is
   *  already "proxied via your selection", and sharing the colour was defect D2. */
  activeClass: string;
}

const MODE_OPTIONS: ModeOption[] = [
  {
    mode: "native",
    icon: Route,
    activeClass:
      "bg-background text-foreground shadow-sm dark:bg-muted dark:text-foreground",
  },
  {
    mode: "selected",
    icon: Zap,
    activeClass:
      "bg-emerald-500/15 text-emerald-700 dark:text-emerald-400 shadow-sm",
  },
  {
    mode: "failover",
    icon: Shuffle,
    activeClass: "bg-amber-500/15 text-amber-700 dark:text-amber-400 shadow-sm",
  },
];

interface RoutingModeControlProps {
  className?: string;
  activeApp: AppId;
  /** Display name used in the descriptions and aria labels. */
  appLabel?: string;
}

export function RoutingModeControl({
  className,
  activeApp,
  appLabel,
}: RoutingModeControlProps) {
  const { t } = useTranslation();
  const { takeoverStatus, setTakeoverForApp, isPending } = useProxyStatus();
  const { data: failoverEnabled = false, isLoading } =
    useAutoFailoverEnabled(activeApp);
  const setFailover = useSetAutoFailoverEnabled();
  const groupRef = useRef<HTMLDivElement>(null);

  const takeover = takeoverStatus?.[activeApp] ?? false;
  const mode = deriveRoutingMode(takeover, failoverEnabled);
  const busy = isPending || isLoading || setFailover.isPending;

  const applyMode = useCallback(
    async (next: RoutingMode) => {
      if (next === mode || busy) return;
      const plan = planRoutingModeChange(next);
      try {
        // Failover is cleared BEFORE takeover is dropped. Leaving failover on
        // while takeover goes off is the state that made the router return
        // "[FO-005] no provider configured" / HTTP 503, because the failover
        // branch builds its candidate list from the queue only.
        if (plan.failover !== failoverEnabled) {
          await setFailover.mutateAsync({
            appType: activeApp,
            enabled: plan.failover,
          });
        }
        if (plan.takeover !== takeover) {
          await setTakeoverForApp({
            appType: activeApp,
            enabled: plan.takeover,
          });
        }
      } catch (error) {
        console.error("[RoutingModeControl] mode change failed:", error);
        toast.error(
          t("routing.changeFailed", {
            app: appLabel ?? activeApp,
            defaultValue: "Could not change {{app}} routing",
          }),
        );
      }
    },
    [
      activeApp,
      appLabel,
      busy,
      failoverEnabled,
      mode,
      setFailover,
      setTakeoverForApp,
      t,
      takeover,
    ],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
      event.preventDefault();
      const index = MODE_OPTIONS.findIndex((o) => o.mode === mode);
      const delta = event.key === "ArrowRight" ? 1 : -1;
      const next =
        MODE_OPTIONS[
          (index + delta + MODE_OPTIONS.length) % MODE_OPTIONS.length
        ];
      void applyMode(next.mode);
      const buttons = groupRef.current?.querySelectorAll("button");
      buttons?.[
        (index + delta + MODE_OPTIONS.length) % MODE_OPTIONS.length
      ]?.focus();
    },
    [applyMode, mode],
  );

  return (
    <div
      className={cn(
        "flex items-center gap-1 px-1 h-8 rounded-lg bg-muted/50",
        className,
      )}
    >
      <span className="px-1 text-[11px] text-muted-foreground select-none">
        {t("routing.label", { defaultValue: "Routing" })}
      </span>
      <div
        ref={groupRef}
        role="radiogroup"
        aria-label={t("routing.groupLabel", {
          app: appLabel ?? activeApp,
          defaultValue: "{{app}} routing mode",
        })}
        className="flex items-center gap-0.5"
        onKeyDown={onKeyDown}
      >
        {MODE_OPTIONS.map(({ mode: optionMode, icon: Icon, activeClass }) => {
          const active = optionMode === mode;
          return (
            <button
              key={optionMode}
              type="button"
              role="radio"
              aria-checked={active}
              disabled={busy}
              title={t(`routing.description.${optionMode}`, {
                app: appLabel ?? activeApp,
                defaultValue: "",
              })}
              onClick={() => void applyMode(optionMode)}
              className={cn(
                "flex items-center gap-1 h-6 px-2 rounded-md text-[11px] transition-colors",
                "text-muted-foreground hover:text-foreground disabled:opacity-50",
                active && activeClass,
              )}
            >
              {busy && active ? (
                <Loader2 className="h-3 w-3 animate-spin" />
              ) : (
                <Icon className="h-3 w-3" />
              )}
              <span>{t(`routing.${optionMode}`)}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
