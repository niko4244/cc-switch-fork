import { render, screen } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { RouteStatusStrip } from "@/components/proxy/RouteStatusStrip";
import type { ActiveRoute } from "@/lib/api/routing";
import { createTestQueryClient } from "../utils/testQueryClient";

// The component must not disagree with the router, so both data sources are
// stubbed at the API layer and the real query hooks run.
const getActiveRoute = vi.fn<() => Promise<ActiveRoute | null>>();
const getAll = vi.fn();
const getCurrent = vi.fn();

vi.mock("@/lib/api/routing", () => ({
  routingApi: {
    getActiveRoute: (...args: unknown[]) => getActiveRoute(...(args as [])),
  },
}));

vi.mock("@/lib/api/providers", () => ({
  providersApi: {
    getAll: (...args: unknown[]) => getAll(...args),
    getCurrent: (...args: unknown[]) => getCurrent(...args),
  },
}));

const SELECTED = {
  id: "brainz",
  name: "Brainz Chain",
  settingsConfig: {},
};
const QUEUE_P1 = { id: "litellm", name: "LiteLLM Gateway", settingsConfig: {} };

function renderStrip(mode: "native" | "selected" | "failover") {
  return render(
    <QueryClientProvider client={createTestQueryClient()}>
      <RouteStatusStrip activeApp="codex" mode={mode} />
    </QueryClientProvider>,
  );
}

function route(overrides: Partial<ActiveRoute> = {}): ActiveRoute {
  return {
    appType: "codex",
    mode: "failover",
    effectiveProviderId: QUEUE_P1.id,
    effectiveProviderName: QUEUE_P1.name,
    upstreamBaseUrl: "http://127.0.0.1:4000/v1",
    selectionIgnored: false,
    lastSwitchAt: 1_700_000_000,
    // Confirmed "now", with the configuration still matching, unless a test
    // says otherwise.
    lastConfirmedAt: Math.floor(Date.now() / 1000),
    inputsChanged: false,
    lastErrorCode: null,
    ...overrides,
  };
}

function agoSeconds(seconds: number): number {
  return Math.floor(Date.now() / 1000) - seconds;
}

describe("RouteStatusStrip", () => {
  beforeEach(() => {
    getAll.mockResolvedValue({
      [SELECTED.id]: SELECTED,
      [QUEUE_P1.id]: QUEUE_P1,
    });
    getCurrent.mockResolvedValue(SELECTED.id);
  });

  it("shows the queue winner and flags that the selection was ignored", async () => {
    getActiveRoute.mockResolvedValue(route({ selectionIgnored: true }));

    renderStrip("failover");

    expect(await screen.findByText("LiteLLM Gateway")).toBeInTheDocument();
    expect(screen.getByText("http://127.0.0.1:4000/v1")).toBeInTheDocument();
    // This single label is the whole point: failover, not the selection, routed
    // the request.
    expect(screen.getByText("selection ignored")).toBeInTheDocument();
    expect(screen.queryByText("predicted")).not.toBeInTheDocument();
  });

  it("does not claim the selection was ignored when failover kept it", async () => {
    getActiveRoute.mockResolvedValue(
      route({
        effectiveProviderId: SELECTED.id,
        effectiveProviderName: SELECTED.name,
      }),
    );

    renderStrip("failover");

    expect(await screen.findByText("Brainz Chain")).toBeInTheDocument();
    expect(screen.queryByText("selection ignored")).not.toBeInTheDocument();
  });

  it("labels a composed route as predicted until the proxy routes something", async () => {
    getActiveRoute.mockResolvedValue(null);

    renderStrip("selected");

    expect(await screen.findByText("Brainz Chain")).toBeInTheDocument();
    expect(screen.getByText("predicted")).toBeInTheDocument();
    expect(screen.queryByText("selection ignored")).not.toBeInTheDocument();
  });

  it("surfaces the structured failover error code", async () => {
    getActiveRoute.mockResolvedValue(
      route({
        effectiveProviderId: null,
        effectiveProviderName: null,
        upstreamBaseUrl: null,
        lastErrorCode: "FO-005",
      }),
    );

    renderStrip("failover");

    expect(
      await screen.findByText(/No provider to route to/),
    ).toBeInTheDocument();
    expect(screen.getByText(/\(FO-005\)/)).toBeInTheDocument();
  });

  it("shows how long ago the route was confirmed", async () => {
    // 4m: old enough to have a unit, young enough to still count as current.
    getActiveRoute.mockResolvedValue(
      route({ lastConfirmedAt: agoSeconds(4 * 60 + 5) }),
    );

    renderStrip("failover");

    expect(await screen.findByText("confirmed 4m ago")).toBeInTheDocument();
    expect(screen.queryByText(/last seen/)).not.toBeInTheDocument();
  });

  it("says the route was confirmed just now while it is fresh", async () => {
    getActiveRoute.mockResolvedValue(route({ lastConfirmedAt: agoSeconds(1) }));

    renderStrip("failover");

    expect(await screen.findByText("confirmed just now")).toBeInTheDocument();
  });

  it("keeps a long-idle route current when the config has not moved", async () => {
    // The case that matters: the app has been open for hours without a single
    // request, so the record is old — but nothing about the routing changed, so
    // it is still exactly the route in use. Age must not libel it.
    getActiveRoute.mockResolvedValue(
      route({ lastConfirmedAt: agoSeconds(6 * 3600), inputsChanged: false }),
    );

    renderStrip("failover");

    expect(await screen.findByText("confirmed 6h ago")).toBeInTheDocument();
    expect(screen.queryByText(/config changed/)).not.toBeInTheDocument();
    expect(screen.queryByText(/last used/)).not.toBeInTheDocument();
  });

  it("demotes a route whose configuration changed to history", async () => {
    getActiveRoute.mockResolvedValue(
      route({ lastConfirmedAt: agoSeconds(6 * 3600), inputsChanged: true }),
    );

    renderStrip("failover");

    expect(await screen.findByText("config changed since")).toBeInTheDocument();
    expect(screen.getByText("last used 6h ago")).toBeInTheDocument();
    // "confirmed" next to "config changed since" would contradict itself.
    expect(screen.queryByText(/confirmed/)).not.toBeInTheDocument();
  });

  it("phrases a just-superseded route as used just now", async () => {
    getActiveRoute.mockResolvedValue(
      route({ lastConfirmedAt: agoSeconds(1), inputsChanged: true }),
    );

    renderStrip("failover");

    expect(await screen.findByText("config changed since")).toBeInTheDocument();
    expect(screen.getByText("last used just now")).toBeInTheDocument();
  });

  it("treats an undated record as unconfirmed rather than trusting it", async () => {
    getActiveRoute.mockResolvedValue(route({ lastConfirmedAt: null }));

    renderStrip("failover");

    expect(await screen.findByText("predicted")).toBeInTheDocument();
    expect(screen.queryByText("LiteLLM Gateway")).not.toBeInTheDocument();
    expect(screen.queryByText(/confirmed|last seen/)).not.toBeInTheDocument();
  });

  it("ignores a persisted route recorded under a different mode", async () => {
    // The record survives restarts, so it can be older than the current intent:
    // a failover record must not be shown as the actual route for `selected`.
    getActiveRoute.mockResolvedValue(route({ selectionIgnored: true }));

    renderStrip("selected");

    expect(await screen.findByText("Brainz Chain")).toBeInTheDocument();
    expect(screen.getByText("predicted")).toBeInTheDocument();
    expect(screen.queryByText("selection ignored")).not.toBeInTheDocument();
    expect(screen.queryByText("LiteLLM Gateway")).not.toBeInTheDocument();
  });

  it("states that native traffic bypasses the proxy", () => {
    renderStrip("native");

    expect(screen.getByText("using its own upstream")).toBeInTheDocument();
    expect(screen.queryByText("predicted")).not.toBeInTheDocument();
  });
});
