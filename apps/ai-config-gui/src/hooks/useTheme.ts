import { useEffect, useSyncExternalStore } from "react";

import {
  getStoredThemeMode,
  initTheme,
  setThemeMode,
  subscribeThemeSystemChange,
  type ThemeMode,
} from "@/lib/theme";

function subscribeThemeStore(onStoreChange: () => void) {
  const onThemeChange = () => onStoreChange();
  const onSystemChange = () => {
    if (getStoredThemeMode() === "system") {
      initTheme();
      onStoreChange();
    }
  };

  window.addEventListener("ai-config:theme", onThemeChange);
  const unsubscribeSystem = subscribeThemeSystemChange(onSystemChange);

  return () => {
    window.removeEventListener("ai-config:theme", onThemeChange);
    unsubscribeSystem();
  };
}

function getThemeSnapshot(): ThemeMode {
  return getStoredThemeMode();
}

export function useTheme() {
  const mode = useSyncExternalStore(
    subscribeThemeStore,
    getThemeSnapshot,
    () => "dark" as ThemeMode,
  );

  useEffect(() => {
    initTheme();
    return subscribeThemeSystemChange(() => {
      if (getStoredThemeMode() === "system") {
        initTheme();
      }
    });
  }, []);

  return {
    mode,
    setMode: setThemeMode,
  };
}
