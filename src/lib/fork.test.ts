import { describe, expect, it } from "vitest";

import {
  FORK_ACK_REQUIRED,
  FORK_PINNED_REFUSAL,
  describeForkBuild,
  describeForkUpdateState,
  isPinnedRefusal,
  type ForkInfo,
} from "./fork";

function info(overrides: Partial<ForkInfo> = {}): ForkInfo {
  return {
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
    refusalCode: FORK_PINNED_REFUSAL,
    ...overrides,
  };
}

describe("describeForkUpdateState", () => {
  it("reports a pinned build as pinned", () => {
    expect(describeForkUpdateState(info())).toBe("pinned");
  });

  it("treats an unknown backend as unknown instead of guessing", () => {
    expect(describeForkUpdateState(null)).toBe("unknown");
    expect(describeForkUpdateState(undefined)).toBe("unknown");
  });

  it("distinguishes an opt-in that has not taken effect from one that has", () => {
    // The policy file says official, but the running process never registered
    // the plugin — claiming "active" here would overstate what is in force.
    expect(
      describeForkUpdateState(
        info({
          updateMode: "official",
          pinned: false,
          refusalCode: null,
          officialUpdaterRegistered: false,
        }),
      ),
    ).toBe("official-pending-restart");

    expect(
      describeForkUpdateState(
        info({
          updateMode: "official",
          pinned: false,
          refusalCode: null,
          officialUpdaterRegistered: true,
        }),
      ),
    ).toBe("official-active");
  });

  it("trusts pinned over a contradictory mode field", () => {
    // Defence in depth: the pin flag is computed by the backend from the policy,
    // so it wins if the two ever disagree.
    expect(
      describeForkUpdateState(info({ updateMode: "official", pinned: true })),
    ).toBe("pinned");
  });
});

describe("isPinnedRefusal", () => {
  it("recognises the structured refusal code in whatever shape it arrives", () => {
    expect(isPinnedRefusal(`${FORK_PINNED_REFUSAL}: pinned`)).toBe(true);
    expect(isPinnedRefusal(new Error(`${FORK_PINNED_REFUSAL}: pinned`))).toBe(
      true,
    );
  });

  it("does not mistake an unrelated failure for the pin", () => {
    expect(isPinnedRefusal("检查更新失败: network unreachable")).toBe(false);
    expect(isPinnedRefusal(`${FORK_ACK_REQUIRED}: 需要确认`)).toBe(false);
    expect(isPinnedRefusal(new Error("boom"))).toBe(false);
    expect(isPinnedRefusal(null)).toBe(false);
    expect(isPinnedRefusal(undefined)).toBe(false);
  });
});

describe("describeForkBuild", () => {
  it("names the fork label and the build commit", () => {
    expect(describeForkBuild(info())).toBe("3.19.1+fork.1.03cbb56 · 03cbb56");
  });

  it("says nothing when the identity is unknown", () => {
    expect(describeForkBuild(null)).toBe("");
  });
});
