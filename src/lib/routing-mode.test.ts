import { describe, it, expect } from "vitest";
import {
  describeRouteAge,
  deriveRoutingMode,
  isProxyRoutable,
  modeIgnoresSelection,
  planRoutingModeChange,
  ROUTE_FRESH_WITHIN_SECONDS,
  type RoutingMode,
} from "./routing-mode";

describe("deriveRoutingMode", () => {
  it("collapses both flags off into native", () => {
    expect(deriveRoutingMode(false, false)).toBe("native");
  });

  it("treats failover without takeover as native, not failover", () => {
    // The router's failover branch never falls back to the selected provider, so
    // "failover on, takeover off" is the state that produced FO-005 / HTTP 503.
    // Collapse it to native rather than showing a mode that cannot route anything.
    expect(deriveRoutingMode(false, true)).toBe("native");
  });

  it("reports selected when only takeover is on", () => {
    expect(deriveRoutingMode(true, false)).toBe("selected");
  });

  it("reports failover when both are on", () => {
    expect(deriveRoutingMode(true, true)).toBe("failover");
  });
});

describe("planRoutingModeChange", () => {
  it("clears failover when going native", () => {
    expect(planRoutingModeChange("native")).toEqual({
      takeover: false,
      failover: false,
    });
  });

  it("keeps takeover but clears failover when going selected", () => {
    expect(planRoutingModeChange("selected")).toEqual({
      takeover: true,
      failover: false,
    });
  });

  it("enables both when going failover", () => {
    expect(planRoutingModeChange("failover")).toEqual({
      takeover: true,
      failover: true,
    });
  });

  it("never leaves failover on with takeover off", () => {
    const modes: RoutingMode[] = ["native", "selected", "failover"];
    for (const mode of modes) {
      const plan = planRoutingModeChange(mode);
      if (!plan.takeover) {
        expect(plan.failover).toBe(false);
      }
    }
  });

  it("round-trips through derive so the UI settles on the requested mode", () => {
    const modes: RoutingMode[] = ["native", "selected", "failover"];
    for (const mode of modes) {
      const plan = planRoutingModeChange(mode);
      expect(deriveRoutingMode(plan.takeover, plan.failover)).toBe(mode);
    }
  });
});

describe("isProxyRoutable", () => {
  it("accepts the apps with backend takeover support", () => {
    expect(isProxyRoutable("claude")).toBe(true);
    expect(isProxyRoutable("codex")).toBe(true);
    expect(isProxyRoutable("gemini")).toBe(true);
    expect(isProxyRoutable("grokbuild")).toBe(true);
  });

  it("rejects claude-desktop, which has no takeover support yet", () => {
    expect(isProxyRoutable("claude-desktop")).toBe(false);
  });

  it("rejects the apps that render no proxy controls", () => {
    expect(isProxyRoutable("opencode")).toBe(false);
    expect(isProxyRoutable("openclaw")).toBe(false);
    expect(isProxyRoutable("hermes")).toBe(false);
  });
});

describe("modeIgnoresSelection", () => {
  it("is true only for failover", () => {
    expect(modeIgnoresSelection("failover")).toBe(true);
    expect(modeIgnoresSelection("selected")).toBe(false);
    expect(modeIgnoresSelection("native")).toBe(false);
  });
});

describe("describeRouteAge", () => {
  const now = 1_790_000_000;

  it("hides an age short enough to be noise", () => {
    expect(describeRouteAge(now, now)).toBeNull();
    expect(
      describeRouteAge(now - ROUTE_FRESH_WITHIN_SECONDS + 1, now),
    ).toBeNull();
  });

  it("scales the unit with the age", () => {
    expect(describeRouteAge(now - 42, now)).toBe("42s");
    expect(describeRouteAge(now - 5 * 60, now)).toBe("5m");
    expect(describeRouteAge(now - 3 * 3600, now)).toBe("3h");
    expect(describeRouteAge(now - 2 * 86400, now)).toBe("2d");
  });

  it("never reports a future timestamp as a negative age", () => {
    expect(describeRouteAge(now + 500, now)).toBeNull();
  });
});

describe("describeRouteAge", () => {
  const now = 1_790_000_000;

  it("still reports an age for a long-idle route", () => {
    // Age is information, not a verdict: the strip shows how long ago the route
    // was last used and leaves staleness to the configuration check.
    expect(describeRouteAge(now - 6 * 3600, now)).toBe("6h");
    expect(describeRouteAge(now - 3 * 86400, now)).toBe("3d");
  });
});
