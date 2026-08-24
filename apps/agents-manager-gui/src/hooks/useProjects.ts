import { useMemoizedFn, useRequest } from "ahooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  addProject,
  fetchDoctor,
  fetchProjectsList,
  removeProject,
} from "../api/tauriAssets";
import type { ConfirmRequest } from "./useConfirm";
import type { ToastKind } from "./useToast";
import type { ProjectItem } from "../types";
import { defaultProjectNameFromPath } from "../utils/defaultProjectNameFromPath";

interface UseProjectsOptions {
  showToast: (kind: ToastKind, text: string) => void;
  requestConfirm: (req: ConfirmRequest) => void;
  dismissConfirm: () => void;
  setBusy: (busy: boolean) => void;
  activeProject: string;
  onActiveProjectChange: (name: string) => void;
}

export function useProjects({
  showToast,
  requestConfirm,
  dismissConfirm,
  setBusy,
  activeProject,
  onActiveProjectChange,
}: UseProjectsOptions) {
  const { t } = useTranslation();
  const [registerProjectOpen, setRegisterProjectOpen] = useState(false);
  const [registerProjectBusy, setRegisterProjectBusy] = useState(false);

  const { data: doctor = null } = useRequest(fetchDoctor, {
    onError: (e) => showToast("err", t("toast.doctorFailed", { error: e })),
  });

  const { data: projects = [], mutate: mutateProjects } = useRequest(
    fetchProjectsList,
    {
      onError: (e) => showToast("err", String(e)),
    },
  );

  const submitRegisterProject = useMemoizedFn(
    async (name: string, rootPath: string) => {
      const trimmedPath = rootPath.trim();
      const resolvedName =
        name.trim() || defaultProjectNameFromPath(trimmedPath);
      if (!trimmedPath || !resolvedName) {
        showToast("err", t("toast.registerProjectInvalid"));
        return;
      }
      setRegisterProjectBusy(true);
      try {
        const project = await addProject(resolvedName, trimmedPath);
        mutateProjects((prev) => [...(prev ?? []), project]);
        setRegisterProjectOpen(false);
        showToast("ok", t("toast.projectRegistered", { name: project.name }));
      } catch (e) {
        showToast("err", t("toast.registerProjectFailed", { error: e }));
      } finally {
        setRegisterProjectBusy(false);
      }
    },
  );

  const runRemoveProject = useMemoizedFn(async (name: string) => {
    setBusy(true);
    try {
      await removeProject(name);
      mutateProjects((prev) => (prev ?? []).filter((q) => q.name !== name));
      if (activeProject === name) onActiveProjectChange("user-global");
      showToast("ok", t("toast.projectRemoved", { name }));
    } catch (e) {
      showToast("err", t("toast.removeProjectFailed", { error: e }));
    } finally {
      setBusy(false);
      dismissConfirm();
    }
  });

  const requestRemoveProject = useMemoizedFn((name: string) => {
    requestConfirm({
      title: t("confirm.removeProject"),
      message: t("confirm.removeProjectMessage", { name }),
      confirmLabel: t("confirm.removeProject"),
      onConfirm: () => runRemoveProject(name),
    });
  });

  const doctorIssueCount = doctor
    ? doctor.broken + doctor.wrong_source + doctor.wrong_type
    : 0;

  return {
    projects: projects as ProjectItem[],
    doctor,
    doctorIssueCount,
    registerProjectOpen,
    setRegisterProjectOpen,
    registerProjectBusy,
    submitRegisterProject,
    requestRemoveProject,
  };
}
