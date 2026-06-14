import type { AssetDetail, AssetKind } from "./types";
import { ASSET_LABEL } from "./types";

interface AssetDrawerProps {
  kind: AssetKind;
  name: string;
  detail: AssetDetail | null;
  loading: boolean;
  editing: boolean;
  draft: string;
  onClose: () => void;
  onDraftChange: (value: string) => void;
  onEdit: () => void;
  onCancelEdit: () => void;
  onSave: () => void;
  onDelete: () => void;
}

export function AssetDrawer({
  kind,
  name,
  detail,
  loading,
  editing,
  draft,
  onClose,
  onDraftChange,
  onEdit,
  onCancelEdit,
  onSave,
  onDelete,
}: AssetDrawerProps) {
  const label = ASSET_LABEL[kind];

  return (
    <div className="drawer-backdrop" onClick={onClose}>
      <aside className="skill-drawer" onClick={(e) => e.stopPropagation()}>
        <header className="skill-drawer-header">
          <div>
            <h4>{detail?.name ?? name}</h4>
            {detail?.description ? (
              <p className="skill-drawer-desc">{detail.description}</p>
            ) : null}
          </div>
          <button type="button" className="icon-btn" onClick={onClose}>
            ✕
          </button>
        </header>

        <div className="skill-drawer-body">
          {loading && !detail ? (
            <div className="empty">加载中…</div>
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
                取消
              </button>
              <button className="primary" disabled={loading} onClick={onSave}>
                保存
              </button>
            </>
          ) : (
            <>
              <button className="danger" disabled={loading} onClick={onDelete}>
                删除{label}
              </button>
              <button disabled={loading || !detail} onClick={onEdit}>
                编辑
              </button>
            </>
          )}
        </footer>
      </aside>
    </div>
  );
}
