import type { TFunction } from "i18next";

const MANIFEST_UNAVAILABLE_PATTERNS = [
  "valid release JSON",
  "404",
  "Not Found",
  "failed to fetch",
  "network",
] as const;

export function formatUpdaterCheckError(err: unknown, t: TFunction): string {
  const message = String(err);
  const manifestMissing = MANIFEST_UNAVAILABLE_PATTERNS.some((pattern) =>
    message.toLowerCase().includes(pattern.toLowerCase()),
  );
  if (manifestMissing) {
    return t("updater.toast.manifestUnavailable");
  }
  return t("updater.toast.checkFailed", { error: message });
}
