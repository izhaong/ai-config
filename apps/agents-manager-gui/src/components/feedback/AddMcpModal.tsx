import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";

import {
  AnimatedOverlay,
  AnimatedScaleDialog,
} from "@/components/ui/animated";
import {
  MCP_SERVER_INPUT_PLACEHOLDER,
  parseMcpServerInput,
} from "../../utils/parseMcpServerInput";

interface AddMcpModalProps {
  busy?: boolean;
  onCancel: () => void;
  onSubmit: (name: string, configJson: string) => void;
}

export function AddMcpModal({
  busy = false,
  onCancel,
  onSubmit,
}: AddMcpModalProps) {
  const { t } = useTranslation();
  const [json, setJson] = useState("");
  const [error, setError] = useState<string | null>(null);

  const canSubmit = json.trim().length > 0 && !busy;

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;
    try {
      const { name, config } = parseMcpServerInput(json);
      setError(null);
      onSubmit(name, JSON.stringify(config, null, 2));
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <AnimatedOverlay open className="confirm-backdrop" onClick={busy ? undefined : onCancel}>
      <AnimatedScaleDialog
        className="confirm-dialog add-mcp-dialog"
        aria-labelledby="add-mcp-title"
      >
        <h4 id="add-mcp-title">{t("addMcp.title")}</h4>
        <p className="form-hint">{t("addMcp.hint")}</p>
        <form onSubmit={handleSubmit}>
          <div className="form-field">
            <label htmlFor="add-mcp-json">{t("addMcp.jsonLabel")}</label>
            <textarea
              id="add-mcp-json"
              className="skill-editor add-mcp-json"
              value={json}
              disabled={busy}
              autoFocus
              spellCheck={false}
              placeholder={MCP_SERVER_INPUT_PLACEHOLDER}
              onChange={(e) => {
                setJson(e.target.value);
                if (error) setError(null);
              }}
            />
            {error ? <p className="form-error">{error}</p> : null}
          </div>
          <div className="confirm-actions">
            <button type="button" disabled={busy} onClick={onCancel}>
              {t("confirm.cancel")}
            </button>
            <button type="submit" className="primary" disabled={!canSubmit}>
              {busy ? t("confirm.processing") : t("addMcp.submit")}
            </button>
          </div>
        </form>
      </AnimatedScaleDialog>
    </AnimatedOverlay>
  );
}
