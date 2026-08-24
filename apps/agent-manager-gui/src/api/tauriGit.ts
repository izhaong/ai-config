import { invoke } from "@tauri-apps/api/core";

import type {
  GitBootstrapResponse,
  GitRepoStatus,
  GitSyncConfig,
  GitSyncOutcome,
} from "../types";

export function fetchGitBootstrap(): Promise<GitBootstrapResponse> {
  return invoke<GitBootstrapResponse>("cmd_git_bootstrap");
}

export function fetchGitStatus(): Promise<GitRepoStatus> {
  return invoke<GitRepoStatus>("cmd_git_status");
}

export function fetchGitConfig(): Promise<GitSyncConfig> {
  return invoke<GitSyncConfig>("cmd_git_config_get");
}

export function saveGitConfig(
  remoteUrl: string | null,
  branch?: string,
): Promise<GitSyncConfig> {
  return invoke<GitSyncConfig>("cmd_git_config_set", {
    remoteUrl: remoteUrl?.trim() || null,
    branch: branch?.trim() || null,
  });
}

export function runGitSync(): Promise<GitSyncOutcome> {
  return invoke<GitSyncOutcome>("cmd_git_sync");
}

export function runGitPull(): Promise<string> {
  return invoke<string>("cmd_git_pull");
}

export function runGitPush(): Promise<string> {
  return invoke<string>("cmd_git_push");
}
