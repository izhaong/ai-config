import { useTranslation } from "react-i18next";

import { setAppLocale, type AppLocale } from "../../i18n";

interface LanguageSwitcherProps {
  /** 设置面板内联样式（非顶栏） */
  inline?: boolean;
}

export function LanguageSwitcher({ inline = false }: LanguageSwitcherProps) {
  const { i18n, t } = useTranslation();
  const locale = (i18n.language === "en-US" ? "en-US" : "zh-CN") as AppLocale;

  return (
    <label
      className={inline ? "lang-switch lang-switch-inline" : "lang-switch"}
      title={t("language.switch")}
    >
      {!inline ? (
        <span className="lang-switch-label">{t("language.switch")}</span>
      ) : null}
      <select
        value={locale}
        onChange={(e) => void setAppLocale(e.target.value as AppLocale)}
        aria-label={t("language.switch")}
      >
        <option value="zh-CN">{t("language.zh")}</option>
        <option value="en-US">{t("language.en")}</option>
      </select>
    </label>
  );
}
