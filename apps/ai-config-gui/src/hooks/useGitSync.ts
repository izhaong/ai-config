import { useMemoizedFn, useMount, useRequest } from "ahooks";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import {
  fetchGitBootstrap,
  fetchGitStatus,
  runGitPull,
  runGitPush,
  runGitSync,
  saveGitConfig,
} from "../api/tauriGit";
import type { GitRepoStatus, GitSyncConfig } from "../types";

import type { ToastKind } from "./useToast";

interface UseGitSyncOptions {
  showToast: (kind: ToastKind, text: string) => void;
}

export function useGitSync({ showToast }: UseGitSyncOptions) {
  const { t } = useTranslation();
  const [config, setConfig] = useState<GitSyncConfig>({ branch: "main" });
  const [status, setStatus] = useState<GitRepoStatus | null>(null);
  const [assetRoot, setAssetRoot] = useState("");
  const [busy, setBusy] = useState(false);
  const [remoteDraft, setRemoteDraft] = useState("");
  const [branchDraft, setBranchDraft] = useState("main");

  const refreshStatus = useMemoizedFn(async () => {
    const s = await fetchGitStatus();
    setStatus(s);
    return s;
  });

  const { loading: bootLoading, run: bootstrap } = useRequest(
    fetchGitBootstrap,
    {
      manual: true,
      onSuccess: (data) => {
        setConfig(data.config);
        setStatus(data.status);
        setAssetRoot(data.asset_root);
        setRemoteDraft(data.config.remote_url ?? "");
        setBranchDraft(data.config.branch || "main");

        if (!data.outcome.git_available) {
          showToast("err", t("git.toast.gitUnavailable"));
          return;
        }
        if (data.outcome.just_initialized) {
          if (data.config.remote_url) {
            showToast("ok", t("git.toast.initializedWithRemote"));
          } else {
            showToast("ok", t("git.toast.initializedLocal"));
          }
        }
      },
      onError: (err) => {
        showToast(
          "err",
          t("git.toast.bootstrapFailed", { error: String(err) }),
        );
      },
    },
  );

  useMount(() => {
    bootstrap();
  });

  const saveConfig = useMemoizedFn(async () => {
    setBusy(true);
    try {
      const next = await saveGitConfig(
        remoteDraft.trim() || null,
        branchDraft.trim() || "main",
      );
      setConfig(next);
      setRemoteDraft(next.remote_url ?? "");
      setBranchDraft(next.branch);
      await refreshStatus();
      showToast(
        "ok",
        next.remote_url
          ? t("git.toast.configSavedRemote")
          : t("git.toast.configSavedLocal"),
      );
    } catch (err) {
      showToast("err", t("git.toast.configSaveFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  });

  const sync = useMemoizedFn(async () => {
    setBusy(true);
    try {
      const outcome = await runGitSync();
      await refreshStatus();
      showToast("ok", outcome.message);
    } catch (err) {
      showToast("err", t("git.toast.syncFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  });

  const pull = useMemoizedFn(async () => {
    setBusy(true);
    try {
      const msg = await runGitPull();
      await refreshStatus();
      showToast("ok", msg);
    } catch (err) {
      showToast("err", t("git.toast.pullFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  });

  const push = useMemoizedFn(async () => {
    setBusy(true);
    try {
      const msg = await runGitPush();
      await refreshStatus();
      showToast("ok", msg);
    } catch (err) {
      showToast("err", t("git.toast.pushFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  });

  const statusLabel = (() => {
    if (bootLoading || !status) return t("git.status.loading");
    if (!status.git_available) return t("git.status.unavailable");
    if (!status.is_repo) return t("git.status.notRepo");
    if (!status.has_remote && !config.remote_url)
      return t("git.status.localOnly");
    if (status.dirty)
      return t("git.status.dirty", { count: status.dirty_count });
    if (status.behind > 0)
      return t("git.status.behind", { count: status.behind });
    if (status.ahead > 0) return t("git.status.ahead", { count: status.ahead });
    return t("git.status.clean");
  })();

  return {
    config,
    status,
    assetRoot,
    busy: busy || bootLoading,
    remoteDraft,
    branchDraft,
    setRemoteDraft,
    setBranchDraft,
    saveConfig,
    sync,
    pull,
    push,
    refreshStatus,
    statusLabel,
    hasRemote: Boolean(
      (config.remote_url && config.remote_url.trim()) || status?.has_remote,
    ),
  };
}
