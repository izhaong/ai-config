import { expect, test } from "@playwright/test";

test("skill details preview scrolls vertically with the mouse wheel", async ({
  page,
}) => {
  await page.goto("/");

  await page.evaluate(() => {
    document.body.innerHTML = `
      <div class="content-pane-scroll is-drawer-open" style="height: 420px">
        <div class="content-pane-body">
          <div class="asset-list" style="height: 900px"></div>
          <div class="drawer-backdrop">
            <aside class="skill-drawer">
              <header class="skill-drawer-header"><h4>long-skill</h4></header>
              <div class="skill-drawer-body">
                <pre class="skill-preview">${Array.from(
                  { length: 40 },
                  (_, index) => `line ${index + 1}`,
                ).join("\n")}</pre>
              </div>
              <footer class="skill-drawer-footer">footer</footer>
            </aside>
          </div>
        </div>
      </div>`;
  });

  const preview = page.locator(".skill-preview");
  await expect(preview).toBeVisible();
  await expect
    .poll(() =>
      preview.evaluate((element) => element.scrollHeight > element.clientHeight),
    )
    .toBe(true);

  await preview.hover();
  await page.mouse.wheel(0, 500);

  await expect
    .poll(() => preview.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
});
