import { describe, it, expect } from "vitest";
import {
  deriveRoutingMode,
  planRoutingModeChange,
  isProxyRoutable,
  modeIgnoresSelection,
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
