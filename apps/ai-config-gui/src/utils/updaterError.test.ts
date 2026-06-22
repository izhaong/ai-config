import { describe, expect, it } from "vitest";

import { formatUpdaterCheckError } from "./updaterError";

const t = (key: string, opts?: { error?: string }) => {
  if (key === "updater.toast.manifestUnavailable") {
    return "manifest unavailable";
  }
  return `check failed: ${opts?.error ?? ""}`;
};

describe("formatUpdaterCheckError", () => {
  it("maps missing release JSON to manifest message", () => {
    expect(
      formatUpdaterCheckError(
        new Error("Could not fetch a valid release JSON from the remote"),
        t,
      ),
    ).toBe("manifest unavailable");
  });

  it("keeps generic errors", () => {
    expect(formatUpdaterCheckError(new Error("timeout"), t)).toBe(
      "check failed: Error: timeout",
    );
  });
});
