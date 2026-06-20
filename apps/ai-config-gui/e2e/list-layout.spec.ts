/**
 * UI 布局对齐冒烟（需本地 Vite：pnpm run dev 或 tauri:dev）
 *
 * 运行：npm run test:e2e
 */
import { expect, test } from "@playwright/test";

const PLATFORM_BTN = `<button type="button" class="plat-btn inactive"><img width="16" height="16" alt="" /></button>`;
const DELETE_BTN = `<button type="button" class="plat-btn row-delete-btn" aria-label="delete"></button>`;
const ROW_ACTIONS = `<div class="row-sync-actions"><div class="platform-actions">${PLATFORM_BTN.repeat(5)}</div>${DELETE_BTN}</div>`;

test.describe("列表顶栏与行对齐", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
    await page.waitForSelector(".list-header .list-col-actions", {
      timeout: 15_000,
    });

    await page.evaluate((actionsHtml) => {
      if (document.querySelector(".asset-row")) {
        return;
      }
      const pane = document.querySelector(".content-pane");
      if (!pane) {
        return;
      }
      pane.querySelector(".empty")?.remove();
      const list = document.createElement("div");
      list.className = "asset-list";
      const row = document.createElement("div");
      row.className = "asset-row list-row-shell";
      row.innerHTML = `
        <label class="list-col-check"><input type="checkbox" /></label>
        <div class="list-col-main asset-row-main">
          <div class="name">e2e-layout-fixture</div>
          <div class="desc">alignment probe</div>
        </div>
        <div class="list-col-actions">${actionsHtml}</div>`;
      list.appendChild(row);
      pane.appendChild(list);
    }, ROW_ACTIONS);
  });

  test("批量操作列与首行操作列水平对齐", async ({ page }) => {
    const header = page.locator(".list-header .list-col-actions").first();
    const row = page.locator(".asset-row .list-col-actions").first();

    const headerBox = await header.boundingBox();
    const rowBox = await row.boundingBox();
    expect(headerBox).not.toBeNull();
    expect(rowBox).not.toBeNull();

    const dx = Math.abs(headerBox!.x - rowBox!.x);
    const dw = Math.abs(headerBox!.width - rowBox!.width);
    expect(dx, `actions col x delta=${dx}px`).toBeLessThanOrEqual(1);
    expect(dw, `actions col width delta=${dw}px`).toBeLessThanOrEqual(1);
  });

  test("复选框列与首行复选框列水平对齐", async ({ page }) => {
    const headerCheck = page.locator(".list-header .list-col-check").first();
    const rowCheck = page.locator(".asset-row .list-col-check").first();

    const a = await headerCheck.boundingBox();
    const b = await rowCheck.boundingBox();
    expect(a).not.toBeNull();
    expect(b).not.toBeNull();

    const dx = Math.abs(a!.x - b!.x);
    expect(dx, `checkbox col x delta=${dx}px`).toBeLessThanOrEqual(1);
  });

  test("顶栏截图归档（人工复核）", async ({ page }) => {
    await page.screenshot({
      path: "e2e/output/list-layout.png",
      fullPage: true,
    });
  });
});
