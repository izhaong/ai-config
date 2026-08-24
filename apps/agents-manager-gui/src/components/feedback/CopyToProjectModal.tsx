import { useMemo, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";
import type { ProjectItem } from "../../types";

function targetAssetRoot(
  toProject: string,
  projects: ProjectItem[],
  globalAssetRoot: string,
): string {
  if (toProject === "user-global") {
    return globalAssetRoot;
  }
  const project = projects.find((p) => p.name === toProject);
  return project ? `${project.root_path}/.agents-manager` : "";
}

interface CopyToProjectModalProps {
  busy?: boolean;
  activeProject: string;
  projects: ProjectItem[];
  globalAssetRoot: string;
  selectedCount: number;
  selectedNames: string[];
  selectedKinds: string[];
  onCancel: () => void;
  onSubmit: (toProject: string) => void;
}

export function CopyToProjectModal({
  busy = false,
  activeProject,
  projects,
  globalAssetRoot,
  selectedCount,
  selectedNames,
  selectedKinds,
  onCancel,
  onSubmit,
}: CopyToProjectModalProps) {
  const { t } = useTranslation();

  const targets = useMemo(() => {
    const list: { id: string; label: string }[] = [];
    if (activeProject !== "user-global") {
      list.push({ id: "user-global", label: t("nav.userGlobal") });
    }
    for (const p of projects) {
      if (p.name !== activeProject) {
        list.push({ id: p.name, label: p.name });
      }
    }
    return list;
  }, [activeProject, projects, t]);

  const [toProject, setToProject] = useState(() => targets[0]?.id ?? "");

  const destAssetRoot = useMemo(
    () => targetAssetRoot(toProject, projects, globalAssetRoot),
    [toProject, projects, globalAssetRoot],
  );

  const hasHooks = selectedKinds.includes("hook");

  const preview = useMemo(() => {
    const shown = selectedNames.slice(0, 5);
    const rest = selectedNames.length - shown.length;
    const names = shown.join(", ");
    if (rest > 0) {
      return t("copyToProject.previewMore", { names, rest });
    }
    return t("copyToProject.preview", { names });
  }, [selectedNames, t]);

  const canSubmit = toProject.length > 0 && targets.length > 0 && !busy;

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    onSubmit(toProject);
  };

  return (
    <AnimatedOverlay
      open
      className="confirm-backdrop"
      onClick={busy ? undefined : onCancel}
    >
      <AnimatedScaleDialog
        className="confirm-dialog copy-to-project-dialog"
        aria-labelledby="copy-to-project-title"
      >
        <h4 id="copy-to-project-title">{t("copyToProject.title")}</h4>
        <p className="copy-to-project-summary">
          {t("copyToProject.summary", { count: selectedCount })}
        </p>
        <p className="copy-to-project-preview" title={selectedNames.join(", ")}>
          {preview}
        </p>

        {targets.length === 0 ? (
          <p className="copy-to-project-empty">{t("copyToProject.noTargets")}</p>
        ) : (
          <form onSubmit={handleSubmit}>
            <div className="form-field">
              <label htmlFor="copy-to-project-target">
                {t("copyToProject.targetLabel")}
              </label>
              <select
                id="copy-to-project-target"
                className="copy-to-project-select"
                value={toProject}
                disabled={busy}
                onChange={(e) => setToProject(e.target.value)}
              >
                {targets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {target.label}
                  </option>
                ))}
              </select>
            </div>
            {destAssetRoot ? (
              <p className="copy-to-project-dest" title={destAssetRoot}>
                {t("copyToProject.destPath", { path: destAssetRoot })}
              </p>
            ) : null}
            {hasHooks ? (
              <p className="copy-to-project-hint">
                {t("copyToProject.hintHook", { path: destAssetRoot || "…" })}
              </p>
            ) : null}
            <p className="copy-to-project-hint">{t("copyToProject.hint")}</p>
            <div className="confirm-actions">
              <Button
                type="button"
                variant="ghost"
                disabled={busy}
                onClick={onCancel}
              >
                {t("confirm.cancel")}
              </Button>
              <Button type="submit" disabled={!canSubmit}>
                {busy ? t("confirm.processing") : t("copyToProject.submit")}
              </Button>
            </div>
          </form>
        )}

        {targets.length === 0 ? (
          <div className="confirm-actions">
            <Button type="button" variant="ghost" onClick={onCancel}>
              {t("confirm.cancel")}
            </Button>
          </div>
        ) : null}
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
