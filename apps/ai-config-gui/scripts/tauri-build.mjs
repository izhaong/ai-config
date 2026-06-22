#!/usr/bin/env node
/**
 * Local Tauri release build with updater signing.
 * Reads ~/.tauri/ai-config.key unless TAURI_SIGNING_PRIVATE_KEY is already set.
 */
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const tauriBin = join(__dirname, "..", "node_modules", ".bin", "tauri");
const defaultKeyPath = join(homedir(), ".tauri", "ai-config.key");

function resolvePrivateKey() {
  if (process.env.TAURI_SIGNING_PRIVATE_KEY?.trim()) {
    return process.env.TAURI_SIGNING_PRIVATE_KEY.trim();
  }

  const keyPath =
    process.env.TAURI_SIGNING_PRIVATE_KEY_PATH?.trim() || defaultKeyPath;

  if (!existsSync(keyPath)) {
    console.error(`[tauri:build] 未找到签名私钥: ${keyPath}`);
    console.error(
      "[tauri:build] 生成命令: cd apps/ai-config-gui && CI=true npm run tauri signer generate -- -w ~/.tauri/ai-config.key -p \"\" -f",
    );
    console.error(
      "[tauri:build] 或设置环境变量 TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PATH",
    );
    process.exit(1);
  }

  return readFileSync(keyPath, "utf8").trim();
}

const result = spawnSync(tauriBin, ["build"], {
  stdio: "inherit",
  env: {
    ...process.env,
    TAURI_SIGNING_PRIVATE_KEY: resolvePrivateKey(),
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD:
      process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? "",
  },
});

process.exit(result.status ?? 1);
