import { Plus } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { useSidebarLayout } from "../../hooks/useSidebarLayout";
import { assetKindLabel } from "../../i18n/labels";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "../../platformIcons";
import { ALL_PLATFORMS, ASSET_KINDS, type AssetKind, type Platform } from "../../types";
import type { PlatformKindPath } from "../../types";
import { SidebarResizer } from "./SidebarResizer";

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
  const { layout, shellRef, startResize, projectsH, assetsW } = useSidebarLayout();

  return (
    <>
      <aside
        ref={shellRef}
        className="sidebar"
        style={{ width: layout.sidebarW, flex: `0 0 ${layout.sidebarW}px` }}
      >
        <section
          className="sidebar-section sidebar-projects"
          style={projectsH !== undefined ? { height: projectsH, flexShrink: 0 } : undefined}
        >
          <div className="sidebar-section-header">
            <h2>{t("nav.projects")}</h2>
            <Button
              type="button"
              variant="outline"
              size="icon-xs"
              className="sidebar-header-extra"
              onClick={onRegisterProject}
              title={t("nav.registerProjectTitle")}
              aria-label={t("nav.registerProjectTitle")}
            >
              <Plus size={14} aria-hidden />
            </Button>
          </div>
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
          </ul>
        </section>

        <SidebarResizer
          orientation="row"
          title={t("nav.resizeProjects")}
          onPointerDown={(e) => startResize("projects", e)}
        />

        <div className="sidebar-bottom-row">
          <section
            className="sidebar-section sidebar-assets"
            style={assetsW !== undefined ? { width: assetsW, flexShrink: 0 } : undefined}
          >
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

          <SidebarResizer
            orientation="col"
            title={t("nav.resizeAssetsPlatforms")}
            onPointerDown={(e) => startResize("assets", e)}
          />

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
        </div>
      </aside>

      <SidebarResizer
        orientation="main"
        title={t("nav.resizeSidebar")}
        onPointerDown={(e) => startResize("sidebar", e)}
      />
    </>
  );
}
