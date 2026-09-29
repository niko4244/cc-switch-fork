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
    lastErrorCode: null,
    ...overrides,
  };
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
    expect(
      screen.getByText("http://127.0.0.1:4000/v1"),
    ).toBeInTheDocument();
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

  it("states that native traffic bypasses the proxy", () => {
    renderStrip("native");

    expect(screen.getByText("using its own upstream")).toBeInTheDocument();
    expect(screen.queryByText("predicted")).not.toBeInTheDocument();
  });
});
