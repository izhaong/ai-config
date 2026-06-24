import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PlatformAssetEntry } from "../types";
import { useAssetOperations } from "./useAssetOperations";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

const mocks = vi.hoisted(() => ({
  saveAsset: vi.fn(() => Promise.resolve("mcp `ai-config` 已保存")),
  deployAssetFromPlatform: vi.fn(() =>
    Promise.resolve("mcp `demo`: cursor → codex OK"),
  ),
  retractAsset: vi.fn(() => Promise.resolve("mcp `demo` ← codex 已移除")),
  importAsset: vi.fn(() => Promise.resolve("mcp `demo` 已导入")),
  deployAsset: vi.fn(() => Promise.resolve("mcp `demo` → codex OK")),
  detectSyncConflict: vi.fn(() => Promise.resolve(null)),
  applySyncChoice: vi.fn(() => Promise.resolve("synced OK")),
}));

vi.mock("../api/tauriAssets", () => ({
  addSkillFromRemote: vi.fn(),
  applySyncChoice: mocks.applySyncChoice,
  deployAsset: mocks.deployAsset,
  deployAssetFromPlatform: mocks.deployAssetFromPlatform,
  detectSyncConflict: mocks.detectSyncConflict,
  importAsset: mocks.importAsset,
  retractAsset: mocks.retractAsset,
  revealPath: vi.fn(),
  saveAsset: mocks.saveAsset,
}));

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

function mcpEntry(
  states: Partial<PlatformAssetEntry["states"]> = {},
): PlatformAssetEntry {
  return {
    name: "demo",
    kind: "mcp",
    description: "stdio",
    platform_path: "/home/.cursor/mcp.json",
    states: {
      aiconfig: "unlinked",
      cursor: "synced",
      codex: "unlinked",
      claude: "unlinked",
      hermes: "unlinked",
      ...states,
    },
  };
}

function makeBrowser(overrides: Record<string, unknown> = {}) {
  return {
    activeProject: "user-global",
    activePlatform: "cursor",
    activeKind: "mcp" as const,
    browsingSource: false,
    selectedEntries: [],
    issueMap: new Map(),
    browsePath: "/home/.cursor/mcp.json",
    refreshView: vi.fn(() => Promise.resolve()),
    clearSelection: vi.fn(),
    ...overrides,
  };
}

function makeDrawer() {
  return { runDeleteEntries: vi.fn() };
}

function renderOps(browserOverrides: Record<string, unknown> = {}) {
  const showToast = vi.fn();
  const setBusy = vi.fn();
  const hook = renderHook(() =>
    useAssetOperations({
      browser: makeBrowser(browserOverrides) as never,
      drawer: makeDrawer() as never,
      projects: [],
      showToast,
      requestConfirm: vi.fn(),
      dismissConfirm: vi.fn(),
      setBusy,
    }),
  );
  return { ...hook, showToast, setBusy };
}

describe("useAssetOperations MCP add", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("submitAddMcp saves via saveAsset", async () => {
    const { result } = renderOps({
      activePlatform: "aiconfig",
      browsingSource: true,
    });

    act(() => {
      result.current.openAddMcp();
    });
    expect(result.current.addMcpOpen).toBe(true);

    const configJson = JSON.stringify(
      { command: "ai-config", args: ["serve"] },
      null,
      2,
    );
    await act(async () => {
      await result.current.submitAddMcp("ai-config", configJson);
    });

    expect(mocks.saveAsset).toHaveBeenCalledWith(
      "mcp",
      "ai-config",
      configJson,
      "user-global",
    );
    expect(result.current.addMcpOpen).toBe(false);
  });

  it("openAddMcp ignores non-mcp source context", () => {
    const { result } = renderOps({
      activeKind: "skill",
      browsingSource: false,
    });

    act(() => {
      result.current.openAddMcp();
    });
    expect(result.current.addMcpOpen).toBe(false);
  });
});

describe("useAssetOperations MCP platform toggle", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("点击当前浏览平台 icon 不触发 API", async () => {
    const { result } = renderOps();
    const entry = mcpEntry();

    act(() => {
      result.current.handlePlatformToggle(entry, "cursor");
    });

    await vi.waitFor(() => {
      expect(mocks.deployAssetFromPlatform).not.toHaveBeenCalled();
      expect(mocks.retractAsset).not.toHaveBeenCalled();
      expect(mocks.importAsset).not.toHaveBeenCalled();
    });
  });

  it("从 Cursor 视图拷贝 MCP 到 Codex", async () => {
    const { result, showToast } = renderOps();
    const entry = mcpEntry({ codex: "missing" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "codex");
    });

    await vi.waitFor(() => {
      expect(mocks.deployAssetFromPlatform).toHaveBeenCalledWith(
        "mcp",
        "demo",
        "user-global",
        "cursor",
        "codex",
      );
      expect(showToast).toHaveBeenCalled();
    });
  });

  it("从 Cursor 视图导入 MCP 到 ai-config", async () => {
    const { result } = renderOps();
    const entry = mcpEntry({ aiconfig: "unlinked" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "aiconfig");
    });

    await vi.waitFor(() => {
      expect(mocks.importAsset).toHaveBeenCalledWith(
        "mcp",
        "demo",
        "user-global",
        "cursor",
      );
    });
  });

  it("收回其它平台上已 synced 的 MCP", async () => {
    const { result } = renderOps();
    const entry = mcpEntry({ claude: "synced" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "claude");
    });

    await vi.waitFor(() => {
      expect(mocks.retractAsset).toHaveBeenCalledWith(
        "mcp",
        "demo",
        "user-global",
        "claude",
      );
    });
  });
});
