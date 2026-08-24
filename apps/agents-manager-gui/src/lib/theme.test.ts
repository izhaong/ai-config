import { describe, expect, it } from "vitest";

import {
  getStoredThemeMode,
  isThemeMode,
  resolveThemeModeForBoot,
  THEME_STORAGE_KEY,
} from "./theme";

describe("theme", () => {
  it("isThemeMode", () => {
    expect(isThemeMode("dark")).toBe(true);
    expect(isThemeMode("light")).toBe(true);
    expect(isThemeMode("system")).toBe(true);
    expect(isThemeMode("sepia")).toBe(false);
    expect(isThemeMode(null)).toBe(false);
  });

  it("getStoredThemeMode defaults to system", () => {
    localStorage.removeItem(THEME_STORAGE_KEY);
    expect(getStoredThemeMode()).toBe("system");
  });

  it("resolveThemeModeForBoot", () => {
    expect(resolveThemeModeForBoot("dark", false)).toBe("dark");
    expect(resolveThemeModeForBoot("light", true)).toBe("light");
    expect(resolveThemeModeForBoot("system", true)).toBe("dark");
    expect(resolveThemeModeForBoot("system", false)).toBe("light");
  });
});
