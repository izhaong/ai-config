# ai-config 应用图标

四色方块 + 同步弧线，对应 Cursor / Codex / Claude / Hermes 四平台分发。

## 重新生成

在 `apps/ai-config-gui` 目录：

```bash
# 源图须为正方形 PNG（建议 1024×1024）
npx tauri icon icons/app-icon-1024.png
```

会更新本目录下 `32x32.png`、`128x128.png`、`icon.png`、`icon.icns`、`icon.ico` 等打包资源。

Web 内嵌 favicon：`public/favicon.png`（由 `32x32.png` 同步，改图标后记得复制）。
