#!/usr/bin/env node
/**
 * Build Tauri static updater manifest (latest.json).
 *
 * Example (macOS arm64, local build artifacts):
 *   node scripts/generate-latest-json.mjs \
 *     --version 0.3.1 \
 *     --tag v0.3.1 \
 *     --platform darwin-aarch64 \
 *     --asset ai-config.app.tar.gz \
 *     --sig ../../target/release/bundle/macos/ai-config.app.tar.gz.sig \
 *     --out latest.json
 *
 * Merge another platform into an existing file:
 *   node scripts/generate-latest-json.mjs ... --merge latest.json
 */
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    tag: { type: "string" },
    repo: { type: "string", default: "izhaong/ai-config" },
    platform: { type: "string" },
    asset: { type: "string" },
    sig: { type: "string" },
    url: { type: "string" },
    notes: { type: "string", default: "" },
    "pub-date": { type: "string" },
    out: { type: "string", default: "latest.json" },
    merge: { type: "string" },
  },
});

function required(name, value) {
  if (!value?.trim()) {
    console.error(`[generate-latest-json] missing --${name}`);
    process.exit(1);
  }
  return value.trim();
}

const version = required("version", values.version);
const tag = required("tag", values.tag);
const platform = required("platform", values.platform);
const asset = required("asset", values.asset);
const sigPath = required("sig", values.sig);
const repo = values.repo ?? "izhaong/ai-config";
const outPath = values.out ?? "latest.json";

if (!existsSync(sigPath)) {
  console.error(`[generate-latest-json] signature not found: ${sigPath}`);
  process.exit(1);
}

const signature = readFileSync(sigPath, "utf8").trim();
const url =
  values.url?.trim() ||
  `https://github.com/${repo}/releases/download/${tag}/${asset}`;

const manifest = values.merge && existsSync(values.merge)
  ? JSON.parse(readFileSync(values.merge, "utf8"))
  : {
      version,
      notes: values.notes ?? "",
      pub_date: values["pub-date"] ?? new Date().toISOString(),
      platforms: {},
    };

manifest.version = version;
if (values.notes) {
  manifest.notes = values.notes;
}
if (values["pub-date"]) {
  manifest.pub_date = values["pub-date"];
}
manifest.platforms[platform] = { signature, url };

writeFileSync(outPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`[generate-latest-json] wrote ${outPath}`);
console.log(`  ${platform} -> ${url}`);
