// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  PRIVACY_CONSENT_STORAGE_KEY,
  PRIVACY_SUPPORT_URL,
  PRIVACY_VERSION,
  PrivacyGate,
  PrivacyPolicyDialog,
} from "./privacy";

const { closeWindow, openExternalUrl } = vi.hoisted(() => ({
  closeWindow: vi.fn().mockResolvedValue(undefined),
  openExternalUrl: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ close: closeWindow }),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: openExternalUrl,
}));

beforeEach(() => {
  window.localStorage.clear();
  closeWindow.mockClear();
  openExternalUrl.mockClear();
});

afterEach(cleanup);

describe("PrivacyGate", () => {
  it("does not mount the application before explicit consent", () => {
    render(<PrivacyGate><p>application mounted</p></PrivacyGate>);

    expect(screen.queryByText("application mounted")).toBeNull();
    expect(screen.getByRole("heading", { name: "プライバシーポリシー" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Stashly プライバシーポリシー", level: 3 })).toBeTruthy();
    expect((screen.getByRole("button", { name: "同意して始める" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("opens the allowlisted support link outside the app", async () => {
    render(<PrivacyGate><p>application mounted</p></PrivacyGate>);
    fireEvent.click(screen.getByRole("link", { name: "Stashly GitHub Issues" }));
    await waitFor(() => expect(openExternalUrl).toHaveBeenCalledWith(PRIVACY_SUPPORT_URL));
  });

  it("stores the current document version and mounts the application", () => {
    render(<PrivacyGate><p>application mounted</p></PrivacyGate>);

    fireEvent.click(screen.getByRole("checkbox", { name: "内容を読み、同意します" }));
    fireEvent.click(screen.getByRole("button", { name: "同意して始める" }));

    expect(window.localStorage.getItem(PRIVACY_CONSENT_STORAGE_KEY)).toBe(PRIVACY_VERSION);
    expect(screen.getByText("application mounted")).toBeTruthy();
  });

  it("skips the gate only for the current document version", () => {
    window.localStorage.setItem(PRIVACY_CONSENT_STORAGE_KEY, PRIVACY_VERSION);
    const { unmount } = render(<PrivacyGate><p>current consent</p></PrivacyGate>);
    expect(screen.getByText("current consent")).toBeTruthy();
    unmount();

    window.localStorage.setItem(PRIVACY_CONSENT_STORAGE_KEY, "2026-09-04");
    render(<PrivacyGate><p>stale consent</p></PrivacyGate>);
    expect(screen.queryByText("stale consent")).toBeNull();
    expect((screen.getByRole("button", { name: "同意して始める" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("closes the application when consent is declined", () => {
    render(<PrivacyGate><p>application mounted</p></PrivacyGate>);
    fireEvent.click(screen.getByRole("button", { name: "同意しないで終了" }));
    expect(closeWindow).toHaveBeenCalledOnce();
  });
});

describe("PrivacyPolicyDialog", () => {
  it("shows the same document and can be closed", () => {
    const onClose = vi.fn();
    render(<PrivacyPolicyDialog onClose={onClose} />);

    expect(screen.getByText(/Stashly プライバシーポリシー/)).toBeTruthy();
    fireEvent.click(screen.getAllByRole("button", { name: "閉じる" })[0]!);
    expect(onClose).toHaveBeenCalledOnce();
  });
});
