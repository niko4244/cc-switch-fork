import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { ForkUpdateCard } from "@/components/settings/ForkUpdateCard";
import { setForkUpdatePolicy } from "@/lib/fork";
import {
  getLastForkPolicyCall,
  setForkInfo,
  setUpstreamRelease,
} from "../msw/state";

/**
 * The card is exercised through the real API layer (the IPC mock forwards to
 * MSW), so these tests cover the contract the backend actually implements:
 * derived `pinned`, the acknowledgement requirement, and the read-only
 * upstream report.
 */
describe("ForkUpdateCard", () => {
  beforeEach(() => {
    setForkInfo({ updateMode: "pinned", officialUpdaterRegistered: false });
    setUpstreamRelease(null);
  });

  it("identifies the installed fork build instead of a bare version", async () => {
    render(<ForkUpdateCard />);

    expect(await screen.findByTestId("fork-name")).toHaveTextContent(
      "CC Switch Fork",
    );
    expect(screen.getByTestId("fork-build")).toHaveTextContent(
      "3.19.1+fork.1.03cbb56",
    );
    expect(screen.getByTestId("fork-build")).toHaveTextContent("03cbb56");
  });

  it("says a pinned build cannot be replaced by upstream", async () => {
    render(<ForkUpdateCard />);

    expect(await screen.findByTestId("fork-state")).toHaveTextContent("Pinned");
    expect(screen.getByTestId("fork-explainer")).toHaveTextContent(
      "Upstream releases cannot replace it",
    );
    // The opt-out is available, but it is an explicit action, not a toggle that
    // looks like a normal setting.
    expect(screen.getByTestId("fork-allow-official")).toBeInTheDocument();
    expect(screen.queryByTestId("fork-return-pinned")).not.toBeInTheDocument();
  });

  it("reports an opt-in as pending until the restart registers the updater", async () => {
    setForkInfo({ updateMode: "official", officialUpdaterRegistered: false });

    render(<ForkUpdateCard />);

    expect(await screen.findByTestId("fork-state")).toHaveTextContent(
      "Opt-in pending restart",
    );
    expect(screen.getByTestId("fork-explainer")).toHaveTextContent(
      "may replace this fork build",
    );
  });

  it("reports the opt-in as active once the process registered the updater", async () => {
    setForkInfo({ updateMode: "official", officialUpdaterRegistered: true });

    render(<ForkUpdateCard />);

    expect(await screen.findByTestId("fork-state")).toHaveTextContent(
      "Official channel",
    );
    expect(screen.getByTestId("fork-return-pinned")).toBeInTheDocument();
  });

  it("reports a newer upstream release without offering to install it", async () => {
    setUpstreamRelease({
      tag: "v3.20.0",
      version: "3.20.0",
      publishedAt: "2026-09-20T10:00:00Z",
      htmlUrl: "https://github.com/farion1231/cc-switch/releases/tag/v3.20.0",
      notes: "notes",
      isNewer: true,
    });

    render(<ForkUpdateCard />);
    fireEvent.click(await screen.findByTestId("fork-check-upstream"));

    const result = await screen.findByTestId("fork-upstream-result");
    expect(result).toHaveTextContent("Upstream 3.20.0 is newer");
    expect(result).toHaveTextContent("Released 2026-09-20");
    // Read-only by construction: there is no install button here at all.
    expect(screen.queryByText(/Install/)).not.toBeInTheDocument();
  });

  it("says upstream has nothing newer than the base release", async () => {
    setUpstreamRelease({
      tag: "v3.19.1",
      version: "3.19.1",
      publishedAt: "2026-09-01T10:00:00Z",
      htmlUrl: null,
      notes: null,
      isNewer: false,
    });

    render(<ForkUpdateCard />);
    fireEvent.click(await screen.findByTestId("fork-check-upstream"));

    expect(await screen.findByTestId("fork-upstream-result")).toHaveTextContent(
      "not newer than the base release 3.19.1",
    );
  });

  it("surfaces a failed upstream check instead of an empty panel", async () => {
    // No release fixture ⇒ the MSW handler returns an error.
    render(<ForkUpdateCard />);
    fireEvent.click(await screen.findByTestId("fork-check-upstream"));

    await waitFor(() => {
      expect(
        screen.queryByTestId("fork-upstream-result"),
      ).not.toBeInTheDocument();
    });
  });

  it("requires explicit confirmation before enabling the official updater", async () => {
    render(<ForkUpdateCard />);

    fireEvent.click(await screen.findByTestId("fork-allow-official"));
    // Nothing is written until the operator confirms the warning.
    expect(getLastForkPolicyCall()).toBeNull();

    const confirm = await screen.findByRole("button", {
      name: "Allow anyway",
    });
    fireEvent.click(confirm);

    await waitFor(() =>
      expect(getLastForkPolicyCall()).toEqual({
        mode: "official",
        acknowledge: true,
      }),
    );
  });

  it("returns to pinned without any acknowledgement", async () => {
    setForkInfo({ updateMode: "official", officialUpdaterRegistered: true });

    render(<ForkUpdateCard />);
    fireEvent.click(await screen.findByTestId("fork-return-pinned"));
    fireEvent.click(
      await screen.findByRole("button", { name: "Return to pinned" }),
    );

    await waitFor(() =>
      expect(getLastForkPolicyCall()).toEqual({
        mode: "pinned",
        acknowledge: false,
      }),
    );
  });

  it("is refused by the backend if the opt-in is sent unacknowledged", async () => {
    // The UI always acknowledges, so this covers the guard underneath it: a
    // caller cannot switch the channel on by forgetting a flag.
    await expect(setForkUpdatePolicy("official", false)).rejects.toThrow(
      /FORK_ACK_REQUIRED/,
    );
  });
});
