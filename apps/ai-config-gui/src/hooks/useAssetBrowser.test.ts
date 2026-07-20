import { cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useAssetBrowser } from "./useAssetBrowser";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

vi.mock("../api/tauriAssets", () => ({
  fetchPlatformList: vi.fn(() =>
    Promise.resolve({ entries: [], platform: "cursor" }),
  ),
  fetchPlatformKindPaths: vi.fn(() => Promise.resolve([])),
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function browserOptions(onContextReset: () => void) {
  return {
    showToast: vi.fn(),
    doctor: null,
    activeProject: "user-global",
    setActiveProject: vi.fn(),
    onContextReset,
  };
}

describe("useAssetBrowser context reset", () => {
  afterEach(() => {
    cleanup();
  });

  it("opens on the ai-config source platform by default", () => {
    const { result } = renderHook(() =>
      useAssetBrowser(browserOptions(vi.fn())),
    );

    expect(result.current.activePlatform).toBe("aiconfig");
  });

  it("does not reset drawer when onContextReset callback identity changes", async () => {
    const reset = vi.fn();
    const { rerender } = renderHook(
      ({ onReset }) => useAssetBrowser(browserOptions(onReset)),
      { initialProps: { onReset: reset } },
    );

    await waitFor(() => {
      expect(reset).toHaveBeenCalledTimes(1);
    });
    reset.mockClear();

    rerender({ onReset: vi.fn() });

    expect(reset).not.toHaveBeenCalled();
  });
});
