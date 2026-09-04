// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import bridgeFile from "../src-tauri/src/booth_download_bridge.js?raw";

const bridgeSource = bridgeFile
  .replace(
    'const BOOTH_LIBRARY_ORIGIN = "https://accounts.booth.pm";',
    "const BOOTH_LIBRARY_ORIGIN = window.location.origin;",
  )
  .replace(
    `    new MutationObserver(scheduleEnhance).observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
`,
    "",
  );

function renderLibrary(includeProduct = true) {
  const productLink = includeProduct
    ? '<a href="https://booth.pm/ja/items/12345">Sample product</a>'
    : "";
  document.body.innerHTML = `
    <nav>購入した商品 ギフト 無料ダウンロード</nav>
    <main><article>${productLink}<div class="actions">
      <button role="button" data-dropdown-items='[{"path":"https://booth.pm/downloadables/789?variation_id=456"}]'>ダウンロード</button>
      <button role="button" data-test="other-downloads-button">その他のDL方法</button>
    </div></article></main>
  `;
  window.eval(bridgeSource);
  return document.querySelector('[data-test="other-downloads-button"]');
}

describe("BOOTH download bridge", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/library");
    delete (window as unknown as Record<string, unknown>).__boothShelfDownloadBridgeInstalled;
    window.requestAnimationFrame = (callback) => {
      callback(0);
      return 1;
    };
  });

  it("hides the paired alternative only with complete official download context", () => {
    const alternative = renderLibrary();

    expect(alternative?.classList.contains("booth-shelf-hidden-download-option")).toBe(true);
  });

  it("leaves the alternative visible when the item ID cannot be proven", () => {
    const alternative = renderLibrary(false);

    expect(alternative?.classList.contains("booth-shelf-hidden-download-option")).toBe(false);
  });
});
