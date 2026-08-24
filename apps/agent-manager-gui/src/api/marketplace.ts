import { invoke } from "@tauri-apps/api/core";

import type { Platform } from "../types";

export type MarketplaceSort = "installs" | "stars" | "newest";

export interface MarketplaceSkill {
  slug: string;
  name: string;
  source: string;
  summary: string;
  github_stars: number;
  installs: number;
  github_language?: string | null;
  categories: string[];
}

export interface MarketplaceListResult {
  skills: MarketplaceSkill[];
  total: number;
  offset: number;
  limit: number;
}

export interface SkillAddBatchItem {
  source: string;
  skillName?: string;
}

export interface SkillAddBatchOutcome {
  ok: number;
  failed: number;
  errors: string[];
}

export function fetchMarketplaceSkills(params: {
  sort: MarketplaceSort;
  q?: string;
  sourceFilter?: string;
  offset?: number;
  limit?: number;
}): Promise<MarketplaceListResult> {
  return invoke<MarketplaceListResult>("cmd_marketplace_list_skills", {
    sort: params.sort,
    q: params.q ?? null,
    sourceFilter: params.sourceFilter ?? null,
    offset: params.offset ?? 0,
    limit: params.limit ?? 50,
  });
}

export function addSkillsBatch(
  project: string,
  skills: SkillAddBatchItem[],
  targetPlatforms: Platform[],
): Promise<SkillAddBatchOutcome> {
  return invoke<SkillAddBatchOutcome>("cmd_skill_add_batch", {
    project,
    skills: skills.map((s) => ({
      source: s.source,
      skill_name: s.skillName ?? null,
    })),
    targetPlatforms,
  });
}

export function skillRowKey(skill: MarketplaceSkill): string {
  return `${skill.source}/${skill.name}`;
}

export function formatMarketplaceCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}
