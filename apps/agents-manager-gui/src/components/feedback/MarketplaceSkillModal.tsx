import { useDebounceFn, useMemoizedFn } from "ahooks";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";
import {
  addSkillsBatch,
  fetchMarketplaceSkills,
  formatMarketplaceCount,
  skillRowKey,
  type MarketplaceSkill,
  type MarketplaceSort,
} from "../../api/marketplace";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "../../platformIcons";
import { ALL_PLATFORMS, type Platform } from "../../types";

interface MarketplaceSkillModalProps {
  busy?: boolean;
  activePlatform: Platform;
  activeProject: string;
  onCancel: () => void;
  onImported: (message: string) => void;
  onError: (message: string) => void;
  setBusy: (busy: boolean) => void;
}

const PAGE_SIZE = 50;

export function MarketplaceSkillModal({
  busy = false,
  activePlatform,
  activeProject,
  onCancel,
  onImported,
  onError,
  setBusy,
}: MarketplaceSkillModalProps) {
  const { t } = useTranslation();
  const [sort, setSort] = useState<MarketplaceSort>("installs");
  const [search, setSearch] = useState("");
  const [sourceFilter, setSourceFilter] = useState("");
  const [skills, setSkills] = useState<MarketplaceSkill[]>([]);
  const [total, setTotal] = useState(0);
  const [nextApiOffset, setNextApiOffset] = useState(0);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedSkillKeys, setSelectedSkillKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [selectedPlatforms, setSelectedPlatforms] = useState<Set<Platform>>(
    () => new Set([activePlatform]),
  );

  const buildQuery = useMemoizedFn(() => {
    const term = search.trim();
    const src = sourceFilter.trim();
    if (term) return term;
    if (src) return src;
    return undefined;
  });

  const loadPage = useCallback(
    async (apiOffset: number, append: boolean) => {
      setLoading(true);
      setLoadError(null);
      try {
        const result = await fetchMarketplaceSkills({
          sort,
          q: buildQuery(),
          sourceFilter: sourceFilter.trim() || undefined,
          offset: apiOffset,
          limit: PAGE_SIZE,
        });
        let rows = result.skills;
        const term = search.trim().toLowerCase();
        const src = sourceFilter.trim();
        if (term && src) {
          rows = rows.filter(
            (s) =>
              s.source === src &&
              (s.name.toLowerCase().includes(term) ||
                s.summary.toLowerCase().includes(term)),
          );
        }
        setSkills((prev) => (append ? [...prev, ...rows] : rows));
        setTotal(result.total);
        setNextApiOffset(apiOffset + PAGE_SIZE);
      } catch (e) {
        setLoadError(String(e));
        if (!append) setSkills([]);
      } finally {
        setLoading(false);
      }
    },
    [sort, search, sourceFilter, buildQuery],
  );

  const { run: reloadDebounced } = useDebounceFn(
    () => {
      void loadPage(0, false);
    },
    { wait: 350 },
  );

  useEffect(() => {
    void loadPage(0, false);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- sort 变更时整页重载
  }, [sort]);

  useEffect(() => {
    reloadDebounced();
  }, [search, sourceFilter, reloadDebounced]);

  const visibleKeys = useMemo(
    () => skills.map((s) => skillRowKey(s)),
    [skills],
  );
  const allVisibleSelected =
    visibleKeys.length > 0 &&
    visibleKeys.every((k) => selectedSkillKeys.has(k));

  const toggleSkill = useMemoizedFn((key: string) => {
    setSelectedSkillKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  });

  const toggleAllVisible = useMemoizedFn(() => {
    setSelectedSkillKeys((prev) => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        visibleKeys.forEach((k) => next.delete(k));
      } else {
        visibleKeys.forEach((k) => next.add(k));
      }
      return next;
    });
  });

  const togglePlatform = useMemoizedFn((plat: Platform) => {
    setSelectedPlatforms((prev) => {
      const next = new Set(prev);
      if (next.has(plat)) next.delete(plat);
      else next.add(plat);
      return next;
    });
  });

  const selectedSkills = useMemo(
    () => skills.filter((s) => selectedSkillKeys.has(skillRowKey(s))),
    [skills, selectedSkillKeys],
  );

  const canSubmit =
    !busy &&
    !loading &&
    selectedSkills.length > 0 &&
    selectedPlatforms.size > 0;

  const handleSubmit = useMemoizedFn(async () => {
    if (!canSubmit) return;
    setBusy(true);
    try {
      const outcome = await addSkillsBatch(
        activeProject,
        selectedSkills.map((s) => ({
          source: s.source,
          skillName: s.name,
        })),
        [...selectedPlatforms],
      );
      if (outcome.failed === 0) {
        onImported(
          t("marketplace.importSuccess", {
            ok: outcome.ok,
            skills: selectedSkills.length,
            platforms: selectedPlatforms.size,
          }),
        );
      } else if (outcome.ok > 0) {
        onImported(
          t("marketplace.importPartial", {
            ok: outcome.ok,
            failed: outcome.failed,
            detail: outcome.errors.slice(0, 3).join("; "),
          }),
        );
      } else {
        onError(
          t("marketplace.importFailed", {
            detail: outcome.errors.slice(0, 3).join("; "),
          }),
        );
        return;
      }
      setSelectedSkillKeys(new Set());
    } catch (e) {
      onError(t("marketplace.importFailed", { detail: String(e) }));
    } finally {
      setBusy(false);
    }
  });

  const hasMore = nextApiOffset < total && !loading;

  return (
    <AnimatedOverlay open className="confirm-backdrop" onClick={busy ? undefined : onCancel}>
      <AnimatedScaleDialog
        className="confirm-dialog marketplace-dialog"
        aria-labelledby="marketplace-skill-title"
      >
        <div className="marketplace-header">
          <div>
            <h4 id="marketplace-skill-title">{t("marketplace.title")}</h4>
            <p className="form-hint marketplace-subtitle">
              {t("marketplace.subtitle")}
            </p>
          </div>
          <a
            className="marketplace-ext-link"
            href="https://www.claudemarketplace.net/skills"
            target="_blank"
            rel="noreferrer"
          >
            {t("marketplace.openSite")}
          </a>
        </div>

        <div className="marketplace-toolbar">
          <label className="marketplace-field">
            <span>{t("marketplace.sortLabel")}</span>
            <select
              value={sort}
              disabled={busy || loading}
              onChange={(e) => setSort(e.target.value as MarketplaceSort)}
            >
              <option value="installs">{t("marketplace.sortInstalls")}</option>
              <option value="stars">{t("marketplace.sortStars")}</option>
              <option value="newest">{t("marketplace.sortNewest")}</option>
            </select>
          </label>
          <label className="marketplace-field marketplace-field-grow">
            <span>{t("marketplace.searchLabel")}</span>
            <input
              type="search"
              value={search}
              disabled={busy || loading}
              placeholder={t("marketplace.searchPlaceholder")}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
          <label className="marketplace-field marketplace-field-grow">
            <span>{t("marketplace.sourceLabel")}</span>
            <input
              type="text"
              value={sourceFilter}
              disabled={busy || loading}
              placeholder="anthropics/skills"
              onChange={(e) => setSourceFilter(e.target.value)}
            />
          </label>
        </div>

        <div className="marketplace-list-wrap">
          {loadError ? (
            <p className="marketplace-list-error">{loadError}</p>
          ) : null}
          <table className="marketplace-table">
            <thead>
              <tr>
                <th className="marketplace-col-check">
                  <input
                    type="checkbox"
                    checked={allVisibleSelected && visibleKeys.length > 0}
                    disabled={busy || loading || visibleKeys.length === 0}
                    onChange={toggleAllVisible}
                    aria-label={t("marketplace.selectAllPage")}
                  />
                </th>
                <th>{t("marketplace.colSkill")}</th>
                <th>{t("marketplace.colSource")}</th>
                <th>{t("marketplace.colStars")}</th>
                <th>{t("marketplace.colInstalls")}</th>
              </tr>
            </thead>
            <tbody>
              {skills.map((skill) => {
                const key = skillRowKey(skill);
                const checked = selectedSkillKeys.has(key);
                return (
                  <tr
                    key={key}
                    className={checked ? "marketplace-row-selected" : undefined}
                    onClick={() => !busy && toggleSkill(key)}
                  >
                    <td className="marketplace-col-check">
                      <input
                        type="checkbox"
                        checked={checked}
                        disabled={busy}
                        onChange={() => toggleSkill(key)}
                        onClick={(e) => e.stopPropagation()}
                      />
                    </td>
                    <td>
                      <div className="marketplace-skill-name">{skill.name}</div>
                      {skill.summary ? (
                        <div className="marketplace-skill-summary" title={skill.summary}>
                          {skill.summary}
                        </div>
                      ) : null}
                    </td>
                    <td>
                      <code className="marketplace-source">{skill.source}</code>
                    </td>
                    <td>{formatMarketplaceCount(skill.github_stars)}</td>
                    <td>{formatMarketplaceCount(skill.installs)}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          {loading ? (
            <p className="marketplace-list-status">{t("marketplace.loading")}</p>
          ) : null}
          {!loading && skills.length === 0 ? (
            <p className="marketplace-list-status">{t("marketplace.empty")}</p>
          ) : null}
        </div>

        <div className="marketplace-footer-meta">
          <span>
            {t("marketplace.listMeta", {
              shown: skills.length,
              total,
              selected: selectedSkills.length,
            })}
          </span>
          {hasMore ? (
            <button
              type="button"
              className="marketplace-load-more"
              disabled={busy || loading}
              onClick={() => void loadPage(nextApiOffset, true)}
            >
              {t("marketplace.loadMore")}
            </button>
          ) : null}
        </div>

        <div className="marketplace-platforms">
          <span className="marketplace-platforms-label">
            {t("marketplace.platformsLabel")}
          </span>
          <div className="marketplace-platform-toggles">
            {ALL_PLATFORMS.map((plat) => {
              const checked = selectedPlatforms.has(plat);
              return (
                <label
                  key={plat}
                  className={`marketplace-platform-chip${checked ? " selected" : ""}`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={busy}
                    onChange={() => togglePlatform(plat)}
                  />
                  <img src={PLATFORM_FAVICON[plat]} alt="" width={14} height={14} />
                  <span>{PLATFORM_NAME[plat]}</span>
                </label>
              );
            })}
          </div>
        </div>

        <div className="confirm-actions">
          <button type="button" disabled={busy} onClick={onCancel}>
            {t("confirm.cancel")}
          </button>
          <button
            type="button"
            className="primary"
            disabled={!canSubmit}
            onClick={() => void handleSubmit()}
          >
            {busy ? t("confirm.processing") : t("marketplace.submit")}
          </button>
        </div>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
