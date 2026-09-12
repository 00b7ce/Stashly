// @vitest-environment jsdom
// @vitest-environment-options {"url":"https://booth.pm/"}

import { beforeEach, describe, expect, it } from "vitest";
import bridgeFile from "../src-tauri/src/booth_download_bridge.js?raw";

const bridgeSource = bridgeFile
  .replace(
    'const BOOTH_LIBRARY_ORIGIN = "https://accounts.booth.pm";',
    "const BOOTH_LIBRARY_ORIGIN = window.location.origin;",
  )
  .replace(
    "window.location.assign(pending.href)",
    "window.__stashlyTestNavigation = pending.href",
  )
  .replace(
    "window.location.assign(product.href)",
    "window.__stashlyTestNavigation = product.href",
  )
  .replace(
    "window.location.assign(`stashly://download-intent?${query}`);",
    "window.__stashlyTestNavigation = `stashly://download-intent?${query}`;",
  )
  .replace(
    'document.addEventListener("click", (event) => {',
    'document.addEventListener("click", window.__stashlyTestClickHandler = (event) => {',
  )
  .replace(
    `      new MutationObserver(scheduleEnhance).observe(document.documentElement, {
        childList: true,
        subtree: true,
      });
`,
    "",
  );

function renderLibrary(includeProduct = true) {
  const productLink = includeProduct
    ? '<a href="https://sample-shop.booth.pm/items/12345"><span data-test="product-name">Sample product</span></a>'
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

function renderProduct() {
  window.history.replaceState({}, "", "/ja/items/12345");
  document.body.innerHTML = `
    <main>
      <h1>Sample free asset</h1>
      <a href="https://booth.pm/downloadables/9413860?variation_id=14512796" target="_blank">
        無料ダウンロード sample.zip
      </a>
    </main>
  `;
  window.eval(bridgeSource);
  return document.querySelector<HTMLAnchorElement>('a[href*="/downloadables/"]');
}

describe("BOOTH download bridge", () => {
  beforeEach(() => {
    const bridgeWindow = window as unknown as Record<string, unknown>;
    const previousClickHandler = bridgeWindow.__stashlyTestClickHandler as
      | EventListener
      | undefined;
    if (previousClickHandler) {
      document.removeEventListener("click", previousClickHandler, true);
    }
    window.history.replaceState({}, "", "/library");
    delete bridgeWindow.__stashlyDownloadBridgeInstalled;
    delete bridgeWindow.__stashlyTestClickHandler;
    window.requestAnimationFrame = (callback) => {
      callback(0);
      return 1;
    };
    Object.defineProperty(window.crypto, "randomUUID", {
      configurable: true,
      value: () => "00000000-0000-4000-8000-000000000001",
    });
    delete bridgeWindow.__stashlyTestNavigation;
  });

  it("hides the paired alternative only with complete official download context", () => {
    const alternative = renderLibrary();

    expect(alternative?.classList.contains("stashly-hidden-download-option")).toBe(true);
  });

  it("leaves the alternative visible when the item ID cannot be proven", () => {
    const alternative = renderLibrary(false);

    expect(alternative?.classList.contains("stashly-hidden-download-option")).toBe(false);
  });

  it("arms a button-based download and shows its notification before following the official URL", () => {
    renderLibrary();
    const button = document.querySelector<HTMLButtonElement>(
      '[data-label="ダウンロード"] button',
    );
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    button?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    const intent = new URL(String(bridgeWindow.__stashlyTestNavigation));
    expect(event.defaultPrevented).toBe(true);
    expect(intent.protocol).toBe("stashly:");
    expect(intent.hostname).toBe("download-intent");
    expect(intent.searchParams.get("variation_id")).toBe("789");
    expect(intent.searchParams.get("downloadable_id")).toBe("789");

    const accept = bridgeWindow.__stashlyAcceptDownloadIntent as (requestId: string) => void;
    accept(String(intent.searchParams.get("request_id")));

    expect(bridgeWindow.__stashlyTestNavigation).toBe(
      "https://booth.pm/downloadables/789",
    );
    const notificationHost = document.getElementById("stashly-download-notifications");
    expect(notificationHost?.parentElement).toBe(document.body);
    expect(notificationHost?.textContent).toContain("ダウンロード中");
    expect(notificationHost?.querySelector(".stashly-notification-icon")?.textContent).toBe("");
  });

  it("arms a free download from an exact BOOTH product page", () => {
    const link = renderProduct();
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    const intent = new URL(String(bridgeWindow.__stashlyTestNavigation));
    expect(event.defaultPrevented).toBe(true);
    expect(intent.protocol).toBe("stashly:");
    expect(intent.hostname).toBe("download-intent");
    expect(intent.searchParams.get("item_id")).toBe("12345");
    expect(intent.searchParams.get("variation_id")).toBe("14512796");
    expect(intent.searchParams.get("downloadable_id")).toBe("9413860");

    const accept = bridgeWindow.__stashlyAcceptDownloadIntent as (requestId: string) => void;
    accept(String(intent.searchParams.get("request_id")));

    expect(bridgeWindow.__stashlyTestNavigation).toBe(
      "https://booth.pm/downloadables/9413860?variation_id=14512796",
    );
  });

  it("uses the current product URL after same-document product navigation", () => {
    window.history.replaceState({}, "", "/ja/items/3087170");
    document.body.innerHTML = "<main>Previous product</main>";
    window.eval(bridgeSource);
    window.history.replaceState({}, "", "/ja/items/3813504");
    document.body.innerHTML = `
      <main>
        <a href="https://booth.pm/ja/items/3087170">Previously viewed product</a>
        <a href="https://booth.pm/downloadables/6359201?variation_id=6359201">
          無料ダウンロード HAOLAN_Ver1.6.zip
        </a>
      </main>
    `;
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    document.querySelector<HTMLAnchorElement>('a[href*="/downloadables/"]')?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    const intent = new URL(String(bridgeWindow.__stashlyTestNavigation));
    expect(event.defaultPrevented).toBe(true);
    expect(intent.searchParams.get("item_id")).toBe("3813504");
    expect(intent.searchParams.get("variation_id")).toBe("6359201");
    expect(intent.searchParams.get("downloadable_id")).toBe("6359201");
  });

  it("handles a product reached by same-document navigation from the BOOTH top page", () => {
    window.history.replaceState({}, "", "/ja");
    document.body.innerHTML = "<main>BOOTH top</main>";
    window.eval(bridgeSource);
    window.history.replaceState({}, "", "/ja/items/3813504");
    document.body.innerHTML = `
      <a href="https://booth.pm/downloadables/6359201?variation_id=6359201">
        無料ダウンロード HAOLAN_Ver1.6.zip
      </a>
    `;
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    document.querySelector("a")?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    const intent = new URL(String(bridgeWindow.__stashlyTestNavigation));
    expect(event.defaultPrevented).toBe(true);
    expect(intent.searchParams.get("item_id")).toBe("3813504");
  });

  it("opens a free-library product link in the existing browser", () => {
    renderLibrary();
    const link = document.querySelector<HTMLAnchorElement>('a[href*="/items/"]');
    link?.setAttribute("target", "_blank");
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.querySelector('[data-test="product-name"]')?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    expect(event.defaultPrevented).toBe(true);
    expect(bridgeWindow.__stashlyTestNavigation).toBe(
      "https://sample-shop.booth.pm/items/12345",
    );
  });

  it("opens a product-description BOOTH link in the existing browser", () => {
    window.history.replaceState({}, "", "/ja/items/8833509");
    document.body.innerHTML = `
      <main>
        <a
          href="https://booth.pm/ja/items/8779825"
          target="_blank"
          rel="nofollow noopener"
        >
          https://booth.pm/ja/items/8779825
        </a>
      </main>
    `;
    window.eval(bridgeSource);
    const link = document.querySelector<HTMLAnchorElement>('a[href*="/items/"]');
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    expect(event.defaultPrevented).toBe(true);
    expect(bridgeWindow.__stashlyTestNavigation).toBe(
      "https://booth.pm/ja/items/8779825",
    );
  });

  it("leaves a same-window product-description link to BOOTH handling", () => {
    window.history.replaceState({}, "", "/ja/items/8833509");
    document.body.innerHTML = `
      <main>
        <a href="https://booth.pm/ja/items/8779825">
          https://booth.pm/ja/items/8779825
        </a>
      </main>
    `;
    window.eval(bridgeSource);
    const link = document.querySelector<HTMLAnchorElement>('a[href*="/items/"]');
    let linkHandledNormally = false;
    link?.addEventListener("click", (event) => {
      linkHandledNormally = true;
      event.preventDefault();
    });
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    expect(linkHandledNormally).toBe(true);
    expect(bridgeWindow.__stashlyTestNavigation).toBeUndefined();
  });

  it("does not intercept a lookalike product host from the library", () => {
    renderLibrary();
    const link = document.querySelector<HTMLAnchorElement>('a[href*="/items/"]');
    link?.setAttribute("href", "https://sample-shop.booth.pm.example.test/items/12345");
    link?.setAttribute("target", "_blank");
    let linkHandledNormally = false;
    link?.addEventListener("click", (event) => {
      linkHandledNormally = true;
      event.preventDefault();
    });
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    expect(linkHandledNormally).toBe(true);
    expect(bridgeWindow.__stashlyTestNavigation).toBeUndefined();
  });

  it("does not arm a download outside an exact BOOTH product page", () => {
    window.history.replaceState({}, "", "/ja/search?q=free");
    document.body.innerHTML = `
      <a href="https://booth.pm/downloadables/9413860?variation_id=14512796">
        無料ダウンロード
      </a>
    `;
    window.eval(bridgeSource);
    let linkHandledNormally = false;
    const link = document.querySelector("a");
    link?.addEventListener("click", (event) => {
      linkHandledNormally = true;
      event.preventDefault();
    });
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });

    link?.dispatchEvent(event);

    const bridgeWindow = window as unknown as Record<string, unknown>;
    expect(linkHandledNormally).toBe(true);
    expect(bridgeWindow.__stashlyTestNavigation).toBeUndefined();
  });
});
