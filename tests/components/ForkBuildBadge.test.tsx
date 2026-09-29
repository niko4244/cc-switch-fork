import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ForkBuildBadge } from "@/components/ForkBuildBadge";
import { setForkInfo } from "../msw/state";

/**
 * The badge exists so a stale or replaced binary is visible on the main window
 * instead of only behind Settings → About. These tests pin the two facts it has
 * to get right: *which* build is running, and whether upstream may replace it —
 * the latter read from the backend's observation, not from the shape of the
 * policy file.
 */
describe("ForkBuildBadge", () => {
  beforeEach(() => {
    setForkInfo({
      isFork: true,
      updateMode: "pinned",
      officialUpdaterRegistered: false,
    });
  });

  it("names the installed build rather than a bare version", async () => {
    render(<ForkBuildBadge />);

    const badge = await screen.findByTestId("fork-build-badge");
    expect(badge).toHaveTextContent("fork 03cbb56");
    expect(badge).toHaveAttribute("data-fork-commit", "03cbb56");
  });

  it("states the pinned policy and the full build in its tooltip", async () => {
    render(<ForkBuildBadge />);

    const badge = await screen.findByTestId("fork-build-badge");
    expect(badge).toHaveAttribute("data-fork-state", "pinned");
    expect(badge.getAttribute("title")).toContain("3.19.1+fork.1.03cbb56");
    expect(badge.getAttribute("title")).toContain("Pinned");
  });

  it("shows an opt-in that has not taken effect as pending, not as active", async () => {
    setForkInfo({ updateMode: "official", officialUpdaterRegistered: false });

    render(<ForkBuildBadge />);

    const badge = await screen.findByTestId("fork-build-badge");
    expect(badge).toHaveAttribute(
      "data-fork-state",
      "official-pending-restart",
    );
    expect(badge.getAttribute("title")).toContain("Opt-in pending restart");
  });

  it("shows the official channel once the running process registered the updater", async () => {
    setForkInfo({ updateMode: "official", officialUpdaterRegistered: true });

    render(<ForkBuildBadge />);

    const badge = await screen.findByTestId("fork-build-badge");
    expect(badge).toHaveAttribute("data-fork-state", "official-active");
    expect(badge.getAttribute("title")).toContain("Official channel");
  });

  it("renders nothing rather than inventing an identity it does not have", async () => {
    setForkInfo({ isFork: false });

    const { container } = render(<ForkBuildBadge />);

    await waitFor(() =>
      expect(screen.queryByTestId("fork-build-badge")).not.toBeInTheDocument(),
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("opens Settings when clicked", async () => {
    const onClick = vi.fn();
    render(<ForkBuildBadge onClick={onClick} />);

    fireEvent.click(await screen.findByTestId("fork-build-badge"));

    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
