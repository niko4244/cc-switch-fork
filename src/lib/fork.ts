import { invoke } from "@tauri-apps/api/core";

/**
 * Fork identity and update policy.
 *
 * This build is a local fork of upstream CC Switch. Upstream's Tauri updater is
 * pinned by default so an official release cannot silently replace the patched
 * binary; everything here is about *reporting* that state and letting the
 * operator deliberately opt out of it.
 *
 * Every call is defensive: a missing or older backend must degrade to "pinned"
 * rather than break the settings panel it renders in.
 */

export type UpdateMode = "pinned" | "official";

export interface ForkInfo {
  isFork: boolean;
  name: string;
  /** Version baked into the bundle (matches the base release). */
  bundleVersion: string;
  /** Full fork label, e.g. `3.19.1+fork.1.03cbb56`. */
  forkVersion: string;
  baseVersion: string;
  baseCommit: string;
  /** Commit this binary was built from. */
  commit: string;
  serial: number;
  upstreamRepo: string;
  upstreamUrl: string;
  updateMode: UpdateMode;
  acknowledgedAt: string | null;
  pinned: boolean;
  /** Observed, not intended: was the official updater plugin registered? */
  officialUpdaterRegistered: boolean;
  refusalCode: string | null;
}

export interface ForkUpdatePolicy {
  mode: UpdateMode;
  acknowledgedAt?: string | null;
  note?: string | null;
}

export interface UpstreamRelease {
  tag: string;
  version: string;
  publishedAt: string | null;
  htmlUrl: string | null;
  notes: string | null;
  /** Newer than the fork's *base* release. */
  isNewer: boolean;
}

/** Returned by the backend when it refuses to apply an upstream artifact. */
export const FORK_PINNED_REFUSAL = "FORK_UPDATER_PINNED";
/** Returned when opting into the official channel without acknowledging it. */
export const FORK_ACK_REQUIRED = "FORK_ACK_REQUIRED";

export async function getForkInfo(): Promise<ForkInfo | null> {
  try {
    return await invoke<ForkInfo>("get_fork_info");
  } catch (error) {
    console.warn("[fork] could not read fork identity", error);
    return null;
  }
}

export async function getForkUpdatePolicy(): Promise<ForkUpdatePolicy | null> {
  try {
    return await invoke<ForkUpdatePolicy>("get_fork_update_policy");
  } catch (error) {
    console.warn("[fork] could not read update policy", error);
    return null;
  }
}

/**
 * `acknowledge` records that the operator understands the official updater
 * replaces this fork build. Switching back to pinned needs no acknowledgement.
 */
export async function setForkUpdatePolicy(
  mode: UpdateMode,
  acknowledge: boolean,
): Promise<ForkUpdatePolicy> {
  return await invoke<ForkUpdatePolicy>("set_fork_update_policy", {
    mode,
    acknowledge,
  });
}

/** Read-only: asks GitHub for the newest upstream release. Never installs. */
export async function checkUpstreamChanges(): Promise<UpstreamRelease | null> {
  return await invoke<UpstreamRelease | null>("check_upstream_changes");
}

export function isPinnedRefusal(error: unknown): boolean {
  const message =
    typeof error === "string"
      ? error
      : error instanceof Error
        ? error.message
        : "";
  return message.includes(FORK_PINNED_REFUSAL);
}

/**
 * What the fork card should say. Kept pure so the state machine is testable
 * without a backend: the interesting case is "the operator opted in, but the
 * running process still has the updater unregistered" — the policy is not in
 * force until the next start, and saying otherwise would be a lie.
 */
export type ForkUpdateState =
  | "unknown"
  | "pinned"
  | "official-pending-restart"
  | "official-active";

export function describeForkUpdateState(
  info: ForkInfo | null | undefined,
): ForkUpdateState {
  if (!info) return "unknown";
  if (info.pinned || info.updateMode === "pinned") return "pinned";
  return info.officialUpdaterRegistered
    ? "official-active"
    : "official-pending-restart";
}

/** Human-facing build line: `3.19.1+fork.1.03cbb56 · 03cbb56`. */
export function describeForkBuild(info: ForkInfo | null | undefined): string {
  if (!info) return "";
  return `${info.forkVersion} · ${info.commit}`;
}
