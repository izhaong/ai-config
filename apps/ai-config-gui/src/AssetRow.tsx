import type { AssetEntry, Platform } from "./types";
import { isPlatformActive, platformUiLabel } from "./types";
import { PLATFORM_FAVICON, PLATFORM_NAME } from "./platformIcons";

interface AssetRowProps {
  entry: AssetEntry;
  loading: boolean;
  issueReasonFor?: (plat: Platform) => string | undefined;
  selected?: boolean;
  checked?: boolean;
  onCheckedChange?: (checked: boolean) => void;
  onOpen?: () => void;
  onPlatformToggle: (plat: Platform) => void;
}

export function AssetRow({
  entry,
  loading,
  issueReasonFor,
  selected,
  checked = false,
  onCheckedChange,
  onOpen,
  onPlatformToggle,
}: AssetRowProps) {
  return (
    <div className={`asset-row${selected ? " selected" : ""}${checked ? " checked" : ""}`}>
      {onCheckedChange ? (
        <label className="asset-row-check" onClick={(e) => e.stopPropagation()}>
          <input
            type="checkbox"
            checked={checked}
            onChange={(e) => onCheckedChange(e.target.checked)}
          />
        </label>
      ) : null}
      <div
        className={`asset-row-main${onOpen ? " clickable" : ""}`}
        onClick={onOpen}
        onKeyDown={
          onOpen
            ? (e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  onOpen();
                }
              }
            : undefined
        }
        role={onOpen ? "button" : undefined}
        tabIndex={onOpen ? 0 : undefined}
      >
        <div className="name">{entry.name}</div>
        <div className="desc">{entry.description || "(无描述)"}</div>
      </div>

      <div className="platform-actions">
        {(["cursor", "codex", "claude", "hermes"] as Platform[]).map((p) => {
          const state = entry.states[p];
          const active = isPlatformActive(state);
          const issueReason = issueReasonFor?.(p);
          const platUnsupported = !!issueReason;
          const disabled = loading || platUnsupported;
          const title = platUnsupported
            ? `${PLATFORM_NAME[p]}: ${issueReason}`
            : `${PLATFORM_NAME[p]} · ${platformUiLabel(active)} · ${active ? "点击移除" : "点击下发"}`;

          return (
            <button
              key={p}
              type="button"
              className={`plat-btn ${active ? "active" : "inactive"}${platUnsupported ? " unsupported" : ""}`}
              disabled={disabled}
              title={title}
              onClick={() => onPlatformToggle(p)}
            >
              <img src={PLATFORM_FAVICON[p]} alt={PLATFORM_NAME[p]} draggable={false} />
            </button>
          );
        })}
      </div>
    </div>
  );
}
