import { useClickAway } from "ahooks";
import { Settings } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { LanguageSwitcher } from "./LanguageSwitcher";

export function SettingsMenu() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useClickAway(() => setOpen(false), rootRef);

  return (
    <div className="settings-menu" ref={rootRef}>
      <button
        type="button"
        className="settings-menu-trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        title={t("settings.title")}
        onClick={() => setOpen((v) => !v)}
      >
        <Settings size={14} aria-hidden />
        <span>{t("settings.title")}</span>
      </button>
      {open ? (
        <div className="settings-menu-panel" role="menu">
          <div className="settings-menu-section">
            <span className="settings-menu-label">{t("language.switch")}</span>
            <LanguageSwitcher inline />
          </div>
        </div>
      ) : null}
    </div>
  );
}
