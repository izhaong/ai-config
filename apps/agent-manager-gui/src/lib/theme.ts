export const THEME_STORAGE_KEY = "agent-manager.theme";

export const THEME_MODES = ["dark", "light", "system"] as const;
export type ThemeMode = (typeof THEME_MODES)[number];

export type ResolvedTheme = "dark" | "light";

export function isThemeMode(value: string | null | undefined): value is ThemeMode {
  return value === "dark" || value === "light" || value === "system";
}

export function getStoredThemeMode(): ThemeMode {
  try {
    const stored = localStorage.getItem(THEME_STORAGE_KEY);
    if (isThemeMode(stored)) {
      return stored;
    }
  } catch {
    /* private mode / SSR */
  }
  return "system";
}

export function resolveThemeMode(mode: ThemeMode): ResolvedTheme {
  if (mode === "dark") return "dark";
  if (mode === "light") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

export function applyResolvedTheme(resolved: ResolvedTheme): void {
  const root = document.documentElement;
  root.dataset.theme = resolved;
  root.style.colorScheme = resolved;
  root.classList.toggle("dark", resolved === "dark");
}

export function applyThemeMode(mode: ThemeMode): void {
  applyResolvedTheme(resolveThemeMode(mode));
}

export function setThemeMode(mode: ThemeMode): void {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, mode);
  } catch {
    /* ignore */
  }
  applyThemeMode(mode);
  window.dispatchEvent(new CustomEvent("agent-manager:theme", { detail: mode }));
}

export function initTheme(): ThemeMode {
  const mode = getStoredThemeMode();
  applyThemeMode(mode);
  return mode;
}

export function subscribeThemeSystemChange(onChange: () => void): () => void {
  const mq = window.matchMedia("(prefers-color-scheme: dark)");
  mq.addEventListener("change", onChange);
  return () => mq.removeEventListener("change", onChange);
}

/** 内联脚本用：无 localStorage 时回退跟随系统 */
export function resolveThemeModeForBoot(
  mode: ThemeMode,
  prefersDark: boolean,
): ResolvedTheme {
  if (mode === "dark") return "dark";
  if (mode === "light") return "light";
  return prefersDark ? "dark" : "light";
}
