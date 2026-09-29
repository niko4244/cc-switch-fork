import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, ShieldCheck } from "lucide-react";

import {
  describeForkBuild,
  describeForkUpdateState,
  getForkInfo,
  type ForkInfo,
} from "@/lib/fork";
import { cn } from "@/lib/utils";

interface ForkBuildBadgeProps {
  className?: string;
  onClick?: () => void;
}

/**
 * Which fork build is running, and whether upstream may replace it.
 *
 * Settings → About states all of this already, but that is the wrong place for
 * it: the thing this guards against is a *stale or replaced binary*, and nobody
 * opens About to discover that. Mounting it in the main-window header means the
 * build actually being executed is checkable without navigating anywhere.
 *
 * Read-only by construction — it reports identity, never changes policy — and an
 * older backend that cannot answer `get_fork_info` yields no badge at all rather
 * than invented fork details.
 */
export function ForkBuildBadge({
  className = "",
  onClick,
}: ForkBuildBadgeProps) {
  const { t } = useTranslation();
  const [info, setInfo] = useState<ForkInfo | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getForkInfo().then((result) => {
      if (!cancelled) setInfo(result);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // No identity, or a backend that does not claim to be this fork: say nothing.
  if (!info || !info.isFork) return null;

  const state = describeForkUpdateState(info);
  const isPinned = state === "pinned";
  // build.rs already shortens the hash; slicing here would drop a `-dirty`
  // suffix, hiding exactly the uncommitted build this badge exists to expose.
  const commit = info.commit || info.bundleVersion;
  const stateLabel =
    state === "official-pending-restart"
      ? t("fork.stateOfficialPending", {
          defaultValue: "Opt-in pending restart",
        })
      : isPinned
        ? t("fork.statePinned", { defaultValue: "Pinned" })
        : t("fork.stateOfficial", { defaultValue: "Official channel" });

  const label = t("fork.badgeLabel", {
    defaultValue: "fork {{commit}}",
    commit,
  });
  const title = t("fork.badgeTitle", {
    defaultValue: "{{build}} — official updater: {{state}}",
    build: describeForkBuild(info),
    state: stateLabel,
  });

  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-label={title}
      data-testid="fork-build-badge"
      data-fork-state={state}
      data-fork-commit={info.commit}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] font-medium leading-4 transition-colors",
        isPinned
          ? "border-border text-muted-foreground hover:bg-muted/60"
          : "border-amber-500/40 text-amber-600 hover:bg-amber-500/10 dark:text-amber-400",
        className,
      )}
    >
      {isPinned ? (
        <ShieldCheck className="h-3 w-3" aria-hidden="true" />
      ) : (
        <AlertTriangle className="h-3 w-3" aria-hidden="true" />
      )}
      <span className="tabular-nums">{label}</span>
    </button>
  );
}
