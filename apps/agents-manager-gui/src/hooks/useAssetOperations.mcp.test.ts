import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PlatformAssetEntry } from "../types";
import { useAssetOperations } from "./useAssetOperations";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));

const mocks = vi.hoisted(() => ({
  saveAsset: vi.fn(() => Promise.resolve("mcp `agents-manager` 已保存")),
  deployAssetFromPlatform: vi.fn(() =>
    Promise.resolve("mcp `demo`: cursor → codex OK"),
  ),
  retractAsset: vi.fn(() => Promise.resolve("mcp `demo` ← codex 已移除")),
  importAsset: vi.fn(() => Promise.resolve("mcp `demo` 已导入")),
  deployAsset: vi.fn(() => Promise.resolve("mcp `demo` → codex OK")),
  detectSyncConflict: vi.fn(() => Promise.resolve(null)),
  applySyncChoice: vi.fn(() => Promise.resolve("synced OK")),
  fetchProjectionPlan: vi.fn(() =>
    Promise.resolve({
      schema_version: 1,
      plan_digest: "reviewed-digest",
      actions: [],
      blocking_reason: null,
    }),
  ),
  fetchImportToSourcePlan: vi.fn(() =>
    Promise.resolve({
      schema_version: 1,
      plan_digest: "import-digest",
      actions: [],
      blocking_reason: null,
    }),
  ),
}));

vi.mock("../api/tauriAssets", () => ({
  addSkillFromRemote: vi.fn(),
  applySyncChoice: mocks.applySyncChoice,
  deployAsset: mocks.deployAsset,
  deployAssetFromPlatform: mocks.deployAssetFromPlatform,
  fetchImportToSourcePlan: mocks.fetchImportToSourcePlan,
  fetchProjectionPlan: mocks.fetchProjectionPlan,
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
      agentsmanager: "unlinked",
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
      activePlatform: "agentsmanager",
      browsingSource: true,
    });

    act(() => {
      result.current.openAddMcp();
    });
    expect(result.current.addMcpOpen).toBe(true);

    const configJson = JSON.stringify(
      { command: "agents-manager", args: ["serve"] },
      null,
      2,
    );
    await act(async () => {
      await result.current.submitAddMcp("agents-manager", configJson);
    });

    expect(mocks.saveAsset).toHaveBeenCalledWith(
      "mcp",
      "agents-manager",
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

  it("从 Cursor 视图不能直接复制 MCP 到 Codex", async () => {
    const { result, showToast } = renderOps();
    const entry = mcpEntry({ codex: "missing" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "codex");
    });

    await vi.waitFor(() => {
      expect(mocks.deployAssetFromPlatform).not.toHaveBeenCalled();
      expect(mocks.deployAsset).not.toHaveBeenCalled();
      expect(showToast).not.toHaveBeenCalled();
    });
  });

  it("源视图部署先请求 scope-level source-first review，而不是平台互拷", async () => {
    const { result } = renderOps({ browsingSource: true, activePlatform: "agentsmanager" });
    const entry = mcpEntry({ agentsmanager: "linked", codex: "missing" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "codex");
    });

    await vi.waitFor(() => {
      expect(mocks.fetchProjectionPlan).toHaveBeenCalledWith("user-global", false);
      expect(mocks.deployAssetFromPlatform).not.toHaveBeenCalled();
      expect(mocks.deployAsset).not.toHaveBeenCalled();
    });
  });

  it("从 Cursor 视图先请求 source-first 导入审阅，而不是直接写入", async () => {
    const { result } = renderOps();
    const entry = mcpEntry({ agentsmanager: "unlinked" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "agentsmanager");
    });

    await vi.waitFor(() => {
      expect(mocks.fetchImportToSourcePlan).toHaveBeenCalledWith(
        "mcp",
        "demo",
        "user-global",
        "cursor",
      );
      expect(mocks.importAsset).not.toHaveBeenCalled();
    });
  });

  it("从 Codex 视图为 MCP 请求 Codex source-first 导入审阅", async () => {
    const { result } = renderOps({ activePlatform: "codex" });
    const entry = mcpEntry({ cursor: "unlinked", codex: "synced", agentsmanager: "unlinked" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "agentsmanager");
    });

    await vi.waitFor(() => {
      expect(mocks.fetchImportToSourcePlan).toHaveBeenCalledWith(
        "mcp",
        "demo",
        "user-global",
        "codex",
      );
      expect(mocks.importAsset).not.toHaveBeenCalled();
    });
  });

  it("不会收回其它平台上已 synced 的 MCP", async () => {
    const { result } = renderOps();
    const entry = mcpEntry({ claude: "synced" });

    await act(async () => {
      result.current.handlePlatformToggle(entry, "claude");
    });

    expect(mocks.retractAsset).not.toHaveBeenCalled();
  });
});
