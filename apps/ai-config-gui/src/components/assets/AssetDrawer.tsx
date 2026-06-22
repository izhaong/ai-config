import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedSlidePanel,
} from "@/components/ui/animated";
import type { AssetDetail, AssetKind } from "../../types";
import { assetKindLabel } from "../../i18n/labels";
import { sanitizeListDescription } from "../../utils/sanitizeListDescription";

interface AssetDrawerProps {
  open: boolean;
  kind: AssetKind;
  name: string;
  detail: AssetDetail | null;
  loading: boolean;
  editing: boolean;
  draft: string;
  /** 仅平台预览、无 ai-config 源时隐藏编辑/删除 */
  readOnly?: boolean;
  onClose: () => void;
  onDraftChange: (value: string) => void;
  onEdit: () => void;
  onCancelEdit: () => void;
  onSave: () => void;
  onDelete: () => void;
}

export function AssetDrawer({
  open,
  kind,
  name,
  detail,
  loading,
  editing,
  draft,
  readOnly = false,
  onClose,
  onDraftChange,
  onEdit,
  onCancelEdit,
  onSave,
  onDelete,
}: AssetDrawerProps) {
  const { t } = useTranslation();
  const label = assetKindLabel(t, kind);

  const titleId = "asset-drawer-title";

  return (
    <AnimatedOverlay open={open} className="drawer-backdrop" onClick={onClose}>
      <AnimatedSlidePanel
        className="skill-drawer"
        aria-labelledby={titleId}
      >
        <header className="skill-drawer-header">
          <div>
            <h4 id={titleId}>{detail?.name ?? name}</h4>
            {detail?.description ? (
              <p className="skill-drawer-desc">
                {sanitizeListDescription(detail.description)}
              </p>
            ) : null}
          </div>
          <button type="button" className="icon-btn" onClick={onClose}>
            ✕
          </button>
        </header>

        <div className="skill-drawer-body">
          {loading && !detail ? (
            <div className="empty">{t("drawer.loading")}</div>
          ) : editing ? (
            <textarea
              className="skill-editor"
              value={draft}
              onChange={(e) => onDraftChange(e.target.value)}
              spellCheck={false}
            />
          ) : (
            <pre className="skill-preview">{detail?.content ?? ""}</pre>
          )}
        </div>

        <footer className="skill-drawer-footer">
          {detail?.source_path ? (
            <span className="skill-path" title={detail.source_path}>
              {detail.source_path}
            </span>
          ) : null}
          <div className="spacer" />
          {editing ? (
            <>
              <button disabled={loading} onClick={onCancelEdit}>
                {t("drawer.cancel")}
              </button>
              <button className="primary" disabled={loading} onClick={onSave}>
                {t("drawer.save")}
              </button>
            </>
          ) : readOnly ? (
            <span className="skill-path">{t("drawer.readOnlyHint")}</span>
          ) : (
            <>
              <button className="danger" disabled={loading} onClick={onDelete}>
                {t("drawer.delete", { label })}
              </button>
              <button disabled={loading || !detail} onClick={onEdit}>
                {t("drawer.edit")}
              </button>
            </>
          )}
        </footer>
      </AnimatedSlidePanel>
    </AnimatedOverlay>
  );
}
