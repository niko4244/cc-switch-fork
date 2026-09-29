import { getVersion } from "@tauri-apps/api/app";

import { getForkInfo, type ForkInfo } from "./fork";

export type UpdateChannel = "stable" | "beta";

export interface UpdateInfo {
  currentVersion: string;
  availableVersion: string;
  notes?: string;
  pubDate?: string;
}

export interface CheckOptions {
  timeout?: number;
  channel?: UpdateChannel;
}

/**
 * `pinned` is not "up to date" — it means this is a fork build and the official
 * updater is deliberately not in play. Collapsing the two would let the UI imply
 * an upstream release can be applied when the pin forbids it.
 */
export type UpdateCheckResult =
  | { status: "up-to-date" }
  | { status: "available"; info: UpdateInfo }
  | { status: "pinned"; fork: ForkInfo };

export async function getCurrentVersion(): Promise<string> {
  try {
    return await getVersion();
  } catch {
    return "";
  }
}

export async function checkForUpdate(
  opts: CheckOptions = {},
): Promise<UpdateCheckResult> {
  // Consult the pin first: while it is active the backend never registers the
  // updater plugin, so asking it anything is either an error or — if a future
  // build registers it by mistake — a path to replacing a build that must not
  // be replaced. No network call is made in this branch either.
  const fork = await getForkInfo();
  if (fork?.pinned) {
    return { status: "pinned", fork };
  }

  // 动态引入，避免在未安装插件时导致打包期问题
  const { check } = await import("@tauri-apps/plugin-updater");

  const currentVersion = await getCurrentVersion();
  const update = await check({ timeout: opts.timeout ?? 30000 } as any);

  if (!update) {
    return { status: "up-to-date" };
  }

  const info: UpdateInfo = {
    currentVersion,
    availableVersion: (update as any).version ?? "",
    notes: (update as any).notes,
    pubDate: (update as any).date,
  };

  return { status: "available", info };
}
