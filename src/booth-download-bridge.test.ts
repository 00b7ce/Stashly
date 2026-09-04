// @vitest-environment jsdom

import { beforeEach, describe, expect, it } from "vitest";
import bridgeFile from "../src-tauri/src/booth_download_bridge.js?raw";

const bridgeSource = bridgeFile
  .replace(
    'const BOOTH_LIBRARY_ORIGIN = "https://accounts.booth.pm";',
    "const BOOTH_LIBRARY_ORIGIN = window.location.origin;",
  )
  .replace(
    "window.location.assign(pending.href)",
    "window.__boothShelfTestNavigation = pending.href",
  )
  .replace(
    "window.location.assign(`booth-shelf://download-intent?${query}`);",
    "window.__boothShelfTestNavigation = `booth-shelf://download-intent?${query}`;",
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
    ? '<a href="https://sample-shop.booth.pm/items/12345">Sample product</a>'
    : "";
  document.body.innerHTML = `
    <nav>購入した商品 ギフト 無料ダウンロード</nav>
    <main><article>${productLink}<div class="actions">
      <div class="js-download-free-button" data-label="ダウンロード"
        data-href="/downloadables/789">
        <button role="button">ダウンロード</button>
      </div>
      <div class="js-download-free-button" data-label="その他のDL方法"
        data-dropdown-items='[{"path":"booth-library-manager://download/example"}]'
        data-test="other-downloads-control">
        <button role="button">その他のDL方法</button>
      </div>
    </div></article></main>
  `;
  window.eval(bridgeSource);
  return document.querySelector('[data-test="other-downloads-control"]');
}

describe("BOOTH download bridge", () => {
  beforeEach(() => {
    window.history.replaceState({}, "", "/library");
    delete (window as unknown as Record<string, unknown>).__boothShelfDownloadBridgeInstalled;
    window.requestAnimationFrame = (callback) => {
      callback(0);
      return 1;
    };
    Object.defineProperty(window.crypto, "randomUUID", {
      configurable: true,
      value: () => "00000000-0000-4000-8000-000000000001",
    });
    delete (window as unknown as Record<string, unknown>).__boothShelfTestNavigation;
  });

  it("hides the paired alternative only with complete official download context", () => {
    const alternative = renderLibrary();

    expect(alternative?.classList.contains("booth-shelf-hidden-download-option")).toBe(true);
  });

  it("leaves the alternative visible when the item ID cannot be proven", () => {
    const alternative = renderLibrary(false);

    expect(alternative?.classList.contains("booth-shelf-hidden-download-option")).toBe(false);
  });

  it("arms a button-based download and shows its notification before following the official URL", () => {
    renderLibrary();
    const button = document.querySelector<HTMLButtonElement>(
      '[data-label="ダウンロード"] button',
    );
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    button?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    const intent = new URL(String(bridgeWindow.__boothShelfTestNavigation));
    expect(event.defaultPrevented).toBe(true);
    expect(intent.protocol).toBe("booth-shelf:");
    expect(intent.hostname).toBe("download-intent");
    expect(intent.searchParams.get("variation_id")).toBe("789");
    expect(intent.searchParams.get("downloadable_id")).toBe("789");

    const accept = bridgeWindow.__boothShelfAcceptDownloadIntent as (requestId: string) => void;
    accept(String(intent.searchParams.get("request_id")));

    expect(bridgeWindow.__boothShelfTestNavigation).toBe(
      "https://booth.pm/downloadables/789",
    );
    const notificationHost = document.getElementById("booth-shelf-download-notifications");
    expect(notificationHost?.parentElement).toBe(document.body);
    expect(notificationHost?.textContent).toContain("ダウンロード中");
    expect(notificationHost?.querySelector(".booth-shelf-notification-icon")?.textContent).toBe("");
  });
});
