# agent-manager GUI

Tauri 2 + React 19 + Vite 桌面壳。

## 国际化 (i18n)

- 框架：[i18next](https://www.i18next.com/) + [react-i18next](https://react.i18next.com/)
- 语言包：`src/i18n/locales/zh-CN.json`、`src/i18n/locales/en-US.json`
- 默认：浏览器语言为 `zh*` 时用中文，否则英文；选择会写入 `localStorage`（`agent-manager.locale`）
- 顶栏右侧下拉可切换 **中文 / English**

新增文案时同步更新两个 JSON 文件中的同名 key。

## 开发

```bash
npm install
npm run tauri:dev
```

## 构建

```bash
npm run build
cargo build -p agent-manager-gui
```
