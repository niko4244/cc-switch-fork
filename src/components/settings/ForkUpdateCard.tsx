import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  AlertTriangle,
  ExternalLink,
  GitBranch,
  Loader2,
  RefreshCw,
  ShieldCheck,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { settingsApi } from "@/lib/api";
import {
  checkUpstreamChanges,
  describeForkUpdateState,
  getForkInfo,
  setForkUpdatePolicy,
  type ForkInfo,
  type UpdateMode,
  type UpstreamRelease,
} from "@/lib/fork";
import { extractErrorMessage } from "@/utils/errorUtils";

/**
 * Fork identity and the upstream-update pin.
 *
 * Two separate statements, deliberately: *which build is installed* and *whether
 * upstream may replace it*. The second is an observation from the backend
 * (`officialUpdaterRegistered`), so a policy that has not taken effect yet shows
 * as pending rather than as already active.
 */
export function ForkUpdateCard() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<ForkInfo | null>(null);
  const [upstream, setUpstream] = useState<UpstreamRelease | null>(null);
  const [isCheckingUpstream, setIsCheckingUpstream] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [pendingMode, setPendingMode] = useState<UpdateMode | null>(null);

  useEffect(() => {
    let cancelled = false;
    void getForkInfo().then((result) => {
      if (!cancelled) setInfo(result);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const handleCheckUpstream = useCallback(async () => {
    setIsCheckingUpstream(true);
    try {
      setUpstream(await checkUpstreamChanges());
    } catch (error) {
      toast.error(
        t("fork.checkFailed", { defaultValue: "Could not reach GitHub" }),
        {
          description: extractErrorMessage(error) || undefined,
          closeButton: true,
        },
      );
    } finally {
      setIsCheckingUpstream(false);
    }
  }, [t]);

  const applyMode = useCallback(
    async (mode: UpdateMode) => {
      setIsSaving(true);
      try {
        await setForkUpdatePolicy(mode, mode === "official");
        setInfo(await getForkInfo());
        toast.success(
          mode === "official"
            ? t("fork.savedOfficial", {
                defaultValue: "Official update channel enabled",
              })
            : t("fork.savedPinned", { defaultValue: "Pinned to this fork" }),
          {
            description: t("fork.restartToApply", {
              defaultValue: "Takes effect after a restart.",
            }),
            closeButton: true,
          },
        );
      } catch (error) {
        toast.error(
          t("fork.saveFailed", {
            defaultValue: "Could not change the update policy",
          }),
          {
            description: extractErrorMessage(error) || undefined,
            closeButton: true,
          },
        );
      } finally {
        setIsSaving(false);
        setPendingMode(null);
      }
    },
    [t],
  );

  // No identity, no card: an older backend must not be papered over with
  // invented fork details.
  if (!info) return null;

  const state = describeForkUpdateState(info);
  const isPinned = state === "pinned";

  return (
    <div
      className="space-y-3 rounded-xl border border-border bg-gradient-to-br from-card/80 to-card/40 p-4 shadow-sm"
      data-testid="fork-update-card"
    >
      <div className="flex flex-wrap items-center gap-2">
        <GitBranch className="h-4 w-4 text-muted-foreground" />
        <h4 className="text-sm font-medium">
          {t("fork.title", { defaultValue: "Fork build" })}
        </h4>
        <Badge variant="secondary" data-testid="fork-name">
          {info.name}
        </Badge>
        {isPinned ? (
          <Badge variant="outline" className="gap-1" data-testid="fork-state">
            <ShieldCheck className="h-3 w-3" />
            {t("fork.statePinned", { defaultValue: "Pinned" })}
          </Badge>
        ) : (
          <Badge
            variant="outline"
            className="gap-1 border-amber-500/40 text-amber-600 dark:text-amber-400"
            data-testid="fork-state"
          >
            <AlertTriangle className="h-3 w-3" />
            {state === "official-pending-restart"
              ? t("fork.stateOfficialPending", {
                  defaultValue: "Opt-in pending restart",
                })
              : t("fork.stateOfficial", { defaultValue: "Official channel" })}
          </Badge>
        )}
      </div>

      <p className="text-xs text-muted-foreground" data-testid="fork-build">
        {t("fork.buildLine", {
          defaultValue:
            "{{version}} · built from {{commit}} · base {{base}} ({{baseCommit}})",
          version: info.forkVersion,
          commit: info.commit,
          base: info.baseVersion,
          baseCommit: info.baseCommit,
        })}
      </p>

      <p className="text-xs text-muted-foreground" data-testid="fork-explainer">
        {isPinned
          ? t("fork.pinnedExplainer", {
              defaultValue:
                "This is a local fork build. Upstream releases cannot replace it, and the official updater stays unregistered.",
            })
          : t("fork.officialExplainer", {
              defaultValue:
                "The official updater may replace this fork build on the next update, discarding every local patch.",
            })}
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8 gap-1.5 text-xs"
          onClick={handleCheckUpstream}
          disabled={isCheckingUpstream}
          data-testid="fork-check-upstream"
        >
          <RefreshCw
            className={`h-3.5 w-3.5 ${isCheckingUpstream ? "animate-spin" : ""}`}
          />
          {isCheckingUpstream
            ? t("fork.checking", { defaultValue: "Checking…" })
            : t("fork.checkUpstream", {
                defaultValue: "Check upstream for changes",
              })}
        </Button>

        {isPinned ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 gap-1.5 text-xs text-muted-foreground"
            onClick={() => setPendingMode("official")}
            disabled={isSaving}
            data-testid="fork-allow-official"
          >
            <AlertTriangle className="h-3.5 w-3.5" />
            {t("fork.allowOfficial", {
              defaultValue: "Allow the official updater",
            })}
          </Button>
        ) : (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="h-8 gap-1.5 text-xs"
            onClick={() => setPendingMode("pinned")}
            disabled={isSaving}
            data-testid="fork-return-pinned"
          >
            <ShieldCheck className="h-3.5 w-3.5" />
            {t("fork.returnPinned", { defaultValue: "Return to pinned" })}
          </Button>
        )}
      </div>

      {upstream && (
        <div
          className="space-y-1 rounded-lg border border-border/60 bg-background/60 px-3 py-2 text-xs"
          data-testid="fork-upstream-result"
        >
          <p>
            {upstream.isNewer
              ? t("fork.upstreamNewer", {
                  defaultValue:
                    "Upstream {{version}} is newer than this fork's base release.",
                  version: upstream.version,
                })
              : t("fork.upstreamCurrent", {
                  defaultValue:
                    "Upstream {{version}} is not newer than the base release {{base}}.",
                  base: info.baseVersion,
                  version: upstream.version,
                })}
          </p>
          {upstream.publishedAt && (
            <p className="text-muted-foreground">
              {t("fork.upstreamPublished", {
                defaultValue: "Released {{date}}",
                date: upstream.publishedAt.slice(0, 10),
              })}
            </p>
          )}
          {upstream.htmlUrl && (
            <Button
              type="button"
              variant="link"
              size="sm"
              className="h-6 gap-1 px-0 text-xs"
              onClick={() =>
                settingsApi.openExternal(upstream.htmlUrl as string)
              }
            >
              <ExternalLink className="h-3 w-3" />
              {t("fork.openRelease", { defaultValue: "Open release notes" })}
            </Button>
          )}
        </div>
      )}

      <ConfirmDialog
        isOpen={pendingMode !== null}
        variant={pendingMode === "official" ? "destructive" : "info"}
        title={
          pendingMode === "official"
            ? t("fork.optInTitle", {
                defaultValue: "Let the official updater replace this build?",
              })
            : t("fork.revertTitle", {
                defaultValue: "Return to the pinned policy?",
              })
        }
        message={
          pendingMode === "official"
            ? t("fork.optInMessage", {
                defaultValue:
                  "An upstream release would overwrite this patched build and every local fix it carries. This takes effect after the next restart.",
              })
            : t("fork.revertMessage", {
                defaultValue:
                  "The official updater will not be registered on the next start. This build stays exactly as it is.",
              })
        }
        confirmText={
          pendingMode === "official"
            ? t("fork.optInConfirm", { defaultValue: "Allow anyway" })
            : t("fork.revertConfirm", { defaultValue: "Return to pinned" })
        }
        onConfirm={() => {
          if (pendingMode) void applyMode(pendingMode);
          else setPendingMode(null);
        }}
        onCancel={() => setPendingMode(null)}
      />

      {isSaving && (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <Loader2 className="h-3 w-3 animate-spin" />
          {t("fork.saving", { defaultValue: "Saving…" })}
        </p>
      )}
    </div>
  );
}
