import { useTranslation } from "react-i18next";

import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "../../platformIcons";
import { ALL_PLATFORMS, ASSET_KINDS, type AssetKind, type Platform } from "../../types";
import type { PlatformKindPath } from "../../types";

interface AppSidebarProps {
  activeProject: string;
  onActiveProjectChange: (name: string) => void;
  projects: Array<{ id?: number; name: string; root_path: string }>;
  onRequestRemoveProject: (name: string) => void;
  onRegisterProject: () => void;
  activeKind: AssetKind;
  onActiveKindChange: (kind: AssetKind) => void;
  activePlatform: Platform;
  onPlatformBrowse: (platform: Platform) => void;
  platformPathMap: Map<Platform, PlatformKindPath>;
  issueMap: Map<string, string>;
}

export function AppSidebar({
  activeProject,
  onActiveProjectChange,
  projects,
  onRequestRemoveProject,
  onRegisterProject,
  activeKind,
  onActiveKindChange,
  activePlatform,
  onPlatformBrowse,
  platformPathMap,
  issueMap,
}: AppSidebarProps) {
  const { t } = useTranslation();

  return (
    <aside className="sidebar">
      <section className="sidebar-section sidebar-projects">
        <h2>{t("nav.projects")}</h2>
        <ul className="project-list">
          <li>
            <button
              type="button"
              className={activeProject === "user-global" ? "active" : ""}
              onClick={() => onActiveProjectChange("user-global")}
            >
              {t("nav.userGlobal")}
            </button>
          </li>
          {projects.map((p) => (
            <li key={p.id || p.name}>
              <button
                type="button"
                className={activeProject === p.name ? "active" : ""}
                onClick={() => onActiveProjectChange(p.name)}
                title={p.root_path}
              >
                <span className="project-name">{p.name}</span>
              </button>
              <span className="badge">.</span>
              <button
                type="button"
                className="remove"
                onClick={(e) => {
                  e.stopPropagation();
                  onRequestRemoveProject(p.name);
                }}
                title={t("nav.removeProject", { name: p.name })}
                aria-label={t("nav.removeProject", { name: p.name })}
              >
                ×
              </button>
            </li>
          ))}
          <li>
            <button
              type="button"
              className="add"
              onClick={onRegisterProject}
              title={t("nav.registerProjectTitle")}
            >
              {t("nav.registerProject")}
            </button>
          </li>
        </ul>
      </section>

      <section className="sidebar-section sidebar-assets">
        <h2>{t("nav.assets")}</h2>
        <ul>
          {ASSET_KINDS.map((k) => (
            <li key={k}>
              <button
                type="button"
                className={activeKind === k ? "active" : ""}
                onClick={() => onActiveKindChange(k)}
              >
                {assetKindLabel(t, k)}
                {issueMap.has(`cursor:${k}`) || issueMap.has(`codex:${k}`) ? (
                  <span className="badge">⚠</span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      </section>

      <section className="sidebar-section sidebar-platforms">
        <h2>{t("nav.platforms")}</h2>
        <ul className="platform-list">
          {ALL_PLATFORMS.map((p) => {
            const pathInfo = platformPathMap.get(p);
            const pathHint = pathInfo?.path
              ? pathInfo.supported
                ? pathInfo.path
                : t("platformView.unsupportedOnPlatform")
              : "";
            return (
              <li key={p}>
                <button
                  type="button"
                  className={`platform-item${activePlatform === p ? " active" : ""}${
                    pathInfo && !pathInfo.supported ? " unsupported" : ""
                  }`}
                  title={
                    pathHint
                      ? `${PLATFORM_NAME[p]} · ${pathHint}`
                      : PLATFORM_NAME[p]
                  }
                  onClick={() => onPlatformBrowse(p)}
                >
                  <img
                    src={PLATFORM_FAVICON[p]}
                    alt=""
                    className="platform-item-icon"
                    draggable={false}
                  />
                  <span className="platform-item-name">{PLATFORM_NAME[p]}</span>
                </button>
              </li>
            );
          })}
        </ul>
      </section>
    </aside>
  );
}
