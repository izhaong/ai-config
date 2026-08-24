import { useTranslation } from "react-i18next";

import { useTheme } from "@/hooks/useTheme";
import { THEME_MODES, type ThemeMode } from "@/lib/theme";

interface ThemeSwitcherProps {
  inline?: boolean;
}

export function ThemeSwitcher({ inline = false }: ThemeSwitcherProps) {
  const { t } = useTranslation();
  const { mode, setMode } = useTheme();

  return (
    <label
      className={inline ? "theme-switch theme-switch-inline" : "theme-switch"}
      title={t("theme.switch")}
    >
      {!inline ? (
        <span className="theme-switch-label">{t("theme.switch")}</span>
      ) : null}
      <select
        value={mode}
        onChange={(e) => setMode(e.target.value as ThemeMode)}
        aria-label={t("theme.switch")}
      >
        {THEME_MODES.map((value) => (
          <option key={value} value={value}>
            {t(`theme.mode.${value}`)}
          </option>
        ))}
      </select>
    </label>
  );
}
