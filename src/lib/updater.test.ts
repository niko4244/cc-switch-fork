import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ForkInfo } from "./fork";

const getForkInfoMock = vi.fn<() => Promise<ForkInfo | null>>();
const pluginCheck = vi.fn();

vi.mock("./fork", () => ({
  getForkInfo: () => getForkInfoMock(),
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: async () => "3.19.1",
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: (...args: unknown[]) => pluginCheck(...args),
}));

import { checkForUpdate } from "./updater";

const pinnedFork: ForkInfo = {
  isFork: true,
  name: "CC Switch Fork",
  bundleVersion: "3.19.1",
  forkVersion: "3.19.1+fork.1.03cbb56",
  baseVersion: "3.19.1",
  baseCommit: "2852962",
  commit: "03cbb56",
  serial: 1,
  upstreamRepo: "farion1231/cc-switch",
  upstreamUrl: "https://github.com/farion1231/cc-switch",
  updateMode: "pinned",
  acknowledgedAt: null,
  pinned: true,
  officialUpdaterRegistered: false,
  refusalCode: "FORK_UPDATER_PINNED",
};

describe("checkForUpdate", () => {
  beforeEach(() => {
    pluginCheck.mockReset();
    getForkInfoMock.mockReset();
  });

  it("reports the pin instead of an update, without touching the updater", async () => {
    getForkInfoMock.mockResolvedValue(pinnedFork);

    const result = await checkForUpdate({ timeout: 10 });

    expect(result).toEqual({ status: "pinned", fork: pinnedFork });
    // The whole point: while pinned, the official updater must not even be
    // asked, because a "yes" here is a path to replacing the fork build.
    expect(pluginCheck).not.toHaveBeenCalled();
  });

  it("uses the official updater once the operator opted in", async () => {
    getForkInfoMock.mockResolvedValue({
      ...pinnedFork,
      updateMode: "official",
      pinned: false,
      refusalCode: null,
      officialUpdaterRegistered: true,
    });
    pluginCheck.mockResolvedValue(null);

    const result = await checkForUpdate({ timeout: 10 });

    expect(result).toEqual({ status: "up-to-date" });
    expect(pluginCheck).toHaveBeenCalledTimes(1);
  });

  it("still reports an offer when the official channel is opted in", async () => {
    getForkInfoMock.mockResolvedValue({
      ...pinnedFork,
      updateMode: "official",
      pinned: false,
      refusalCode: null,
      officialUpdaterRegistered: true,
    });
    pluginCheck.mockResolvedValue({ version: "3.20.0", notes: "notes" });

    const result = await checkForUpdate({ timeout: 10 });

    expect(result).toEqual({
      status: "available",
      info: {
        currentVersion: "3.19.1",
        availableVersion: "3.20.0",
        notes: "notes",
        pubDate: undefined,
      },
    });
  });

  it("falls back to the official path when the backend cannot report a fork", async () => {
    // An older backend (or a read failure) must not silently disable updates.
    getForkInfoMock.mockResolvedValue(null);
    pluginCheck.mockResolvedValue(null);

    const result = await checkForUpdate({ timeout: 10 });

    expect(result).toEqual({ status: "up-to-date" });
    expect(pluginCheck).toHaveBeenCalledTimes(1);
  });
});
