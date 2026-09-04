(() => {
  "use strict";

  const BOOTH_LIBRARY_ORIGIN = "https://accounts.booth.pm";
  const NOTIFICATION_HOST_ID = "booth-shelf-download-notifications";
  const NOTIFICATION_STYLE_ID = "booth-shelf-download-notification-style";
  const NOTIFICATION_DURATION_MS = 6000;
  const DOWNLOAD_LABEL = "ダウンロード";
  const ALTERNATIVE_LABEL = "その他のDL方法";
  const HIDDEN_CLASS = "booth-shelf-hidden-download-option";
  const HIDDEN_CHROME_CLASS = "booth-shelf-hidden-library-chrome";
  const LIBRARY_MAIN_ATTRIBUTE = "data-booth-shelf-library-main";
  const LIBRARY_TAB_LABELS = ["購入した商品", "ギフト", "無料ダウンロード"];
  const INTENT_TIMEOUT_MS = 3000;

  if (window.__boothShelfDownloadBridgeInstalled) return;
  window.__boothShelfDownloadBridgeInstalled = true;

  const notificationTimers = new Map();
  const ensureNotificationHost = () => {
    if (!document.documentElement) return null;
    if (!document.getElementById(NOTIFICATION_STYLE_ID)) {
      const style = document.createElement("style");
      style.id = NOTIFICATION_STYLE_ID;
      style.textContent = `
        #${NOTIFICATION_HOST_ID} {
          position: fixed; right: 24px; bottom: 22px; z-index: 2147483647;
          width: min(460px, calc(100vw - 48px)); display: grid; gap: 10px;
          font-family: Inter, "Yu Gothic UI", system-ui, sans-serif;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification {
          position: relative; min-height: 78px; padding: 16px 14px 17px 16px;
          display: grid; grid-template-columns: auto minmax(0, 1fr) auto; gap: 13px;
          align-items: center; overflow: hidden; border: 1px solid #dbe1eb;
          border-radius: 14px; background: #fffffff2; box-shadow: 0 8px 28px #25304722;
          backdrop-filter: blur(12px); color: #687287;
          transition: transform 160ms ease, box-shadow 160ms ease;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed { color: #2d8a6a; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.failed { color: #b43d57; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed[data-request-id] { cursor: pointer; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed[data-request-id]:hover {
          transform: translateY(-2px); box-shadow: 0 12px 34px #25304730;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification-icon {
          width: 24px; height: 24px; display: grid; place-items: center;
          font-size: 20px; font-weight: 800;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification:not(.completed):not(.failed)
          .booth-shelf-notification-icon { animation: booth-shelf-spin 1s linear infinite; }
        #${NOTIFICATION_HOST_ID} strong, #${NOTIFICATION_HOST_ID} span {
          display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
        }
        #${NOTIFICATION_HOST_ID} strong { color: #2a3347; font-size: 13px; }
        #${NOTIFICATION_HOST_ID} span { margin-top: 5px; font-size: 11px; }
        #${NOTIFICATION_HOST_ID} button {
          width: 30px; height: 30px; display: grid; place-items: center; border: 0;
          border-radius: 7px; background: transparent; color: #8b95a8; cursor: pointer;
        }
        #${NOTIFICATION_HOST_ID} button:hover { background: #edf0f5; color: #394357; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification-expiry {
          position: absolute; right: 0; bottom: 0; left: 0; height: 4px; background: #e6ebf2;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed
          .booth-shelf-notification-expiry::after {
          content: ""; position: absolute; inset: 0; background: #43a985;
          transform-origin: left center;
          animation: booth-shelf-expiry ${NOTIFICATION_DURATION_MS}ms linear forwards;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification:not(.completed)
          .booth-shelf-notification-expiry { display: none; }
        @keyframes booth-shelf-spin { to { transform: rotate(360deg); } }
        @keyframes booth-shelf-expiry { from { transform: scaleX(1); } to { transform: scaleX(0); } }
      `;
      document.documentElement.appendChild(style);
    }
    let host = document.getElementById(NOTIFICATION_HOST_ID);
    if (!host) {
      host = document.createElement("aside");
      host.id = NOTIFICATION_HOST_ID;
      host.setAttribute("aria-live", "polite");
      document.documentElement.appendChild(host);
    }
    return host;
  };

  window.__boothShelfNotify = (status) => {
    if (!status || typeof status !== "object") return;
    const host = ensureNotificationHost();
    if (!host) return;
    const key = String(status.requestId || "general-download-error");
    let row = Array.from(host.children).find((element) => element.dataset.notificationKey === key);
    if (!row) {
      row = document.createElement("div");
      row.dataset.notificationKey = key;
      row.innerHTML = `
        <div class="booth-shelf-notification-icon"></div>
        <div><strong></strong><span></span></div>
        <button type="button" aria-label="通知を閉じる">×</button>
        <div class="booth-shelf-notification-expiry" aria-hidden="true"></div>
      `;
      const openCompletedFolder = () => {
        const requestId = row.dataset.requestId;
        if (!requestId || !row.classList.contains("completed")) return;
        window.location.href = `booth-shelf://open-product-folder?request_id=${encodeURIComponent(requestId)}`;
      };
      row.addEventListener("click", openCompletedFolder);
      row.addEventListener("keydown", (event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        openCompletedFolder();
      });
      row.querySelector("button").addEventListener("click", (event) => {
        event.stopPropagation();
        row.remove();
      });
      host.prepend(row);
    }
    row.className = `booth-shelf-notification ${String(status.state || "")}`;
    row.querySelector(".booth-shelf-notification-icon").textContent =
      status.state === "completed" ? "✓" : status.state === "failed" ? "!" : "◌";
    const copy = status.state === "completed"
      ? ["ダウンロード完了", "ダウンロードが完了しました。"]
      : status.state === "failed"
        ? ["ダウンロード失敗", "詳細はBooth Shelfを確認してください。"]
        : status.state === "downloading"
          ? ["ダウンロード中", "ダウンロードしています。"]
          : ["ダウンロード受付", "ダウンロードを受け付けました。"];
    row.querySelector("strong").textContent = copy[0];
    row.querySelector("span").textContent = copy[1];
    delete row.dataset.requestId;
    row.removeAttribute("role");
    row.removeAttribute("tabindex");
    if (status.state === "completed" && status.requestId) {
      row.dataset.requestId = String(status.requestId);
      row.setAttribute("role", "button");
      row.setAttribute("tabindex", "0");
      row.setAttribute("aria-label", "ダウンロード済み商品のフォルダを開く");
    }
    const existingTimer = notificationTimers.get(key);
    if (existingTimer !== undefined) window.clearTimeout(existingTimer);
    if (status.state === "completed") {
      const timer = window.setTimeout(() => {
        row.remove();
        notificationTimers.delete(key);
      }, NOTIFICATION_DURATION_MS);
      notificationTimers.set(key, timer);
    }
  };

  if (
    window.location.origin !== BOOTH_LIBRARY_ORIGIN ||
    !(window.location.pathname === "/library" || window.location.pathname.startsWith("/library/"))
  ) return;

  const normalize = (value) => (value || "").replace(/\s+/g, "").trim();
  const labelIs = (element, label) => normalize(element.textContent) === normalize(label);
  const actionElements = (root) => Array.from(
    root.querySelectorAll('button, a, [role="button"], [role="menuitem"]'),
  );
  const findLibraryTabs = () => Array.from(document.querySelectorAll("nav")).find((navigation) => {
    const text = normalize(navigation.textContent);
    return LIBRARY_TAB_LABELS.every((label) => text.includes(normalize(label)));
  });
  const hidePrecedingSiblings = (element) => {
    let sibling = element.previousElementSibling;
    while (sibling) {
      sibling.classList.add(HIDDEN_CHROME_CLASS);
      sibling = sibling.previousElementSibling;
    }
  };
  const simplifyLibraryChrome = () => {
    const tabs = findLibraryTabs();
    if (!tabs) return;
    for (const footer of document.querySelectorAll('footer, [role="contentinfo"]')) {
      footer.classList.add(HIDDEN_CHROME_CLASS);
    }
    let current = tabs;
    while (current && current !== document.body) {
      hidePrecedingSiblings(current);
      current = current.parentElement;
    }
    const main = tabs.closest("main");
    if (main) main.setAttribute(LIBRARY_MAIN_ATTRIBUTE, "true");
    tabs.style.setProperty("margin-top", "0", "important");
  };
  const findPair = (downloadButton) => {
    let container = downloadButton.parentElement;
    for (let depth = 0; container && depth < 6; depth += 1) {
      const actions = actionElements(container);
      const downloads = actions.filter((element) => labelIs(element, DOWNLOAD_LABEL));
      const alternatives = actions.filter((element) => labelIs(element, ALTERNATIVE_LABEL));
      if (downloads.length === 1 && alternatives.length === 1) {
        return { container, download: downloads[0], alternative: alternatives[0] };
      }
      container = container.parentElement;
    }
    return null;
  };
  const positiveInteger = (value) => {
    if (!/^[1-9]\d*$/.test(value || "")) return null;
    const number = Number(value);
    return Number.isSafeInteger(number) ? number : null;
  };
  const parseDownloadUrl = (value) => {
    let url;
    try { url = new URL(value, window.location.href); } catch { return null; }
    if (url.protocol !== "https:" || url.hostname !== "booth.pm") return null;
    const match = url.pathname.match(/^\/downloadables\/([1-9]\d*)\/?$/);
    const variationValues = url.searchParams.getAll("variation_id");
    if (!match || variationValues.length !== 1) return null;
    const downloadableId = positiveInteger(match[1]);
    const variationId = positiveInteger(variationValues[0]);
    if (!downloadableId || !variationId) return null;
    return { href: url.href, downloadableId, variationId };
  };
  const itemIdFromHref = (value) => {
    let url;
    try { url = new URL(value, window.location.href); } catch { return null; }
    if (url.protocol !== "https:" || url.hostname !== "booth.pm") return null;
    const match = url.pathname.match(/^\/(?:[a-z]{2}(?:-[a-z]{2})?\/)?items\/([1-9]\d*)\/?$/i);
    return match ? positiveInteger(match[1]) : null;
  };
  const findItemId = (element) => {
    let container = element;
    for (let depth = 0; container && depth < 12; depth += 1) {
      const ids = new Set(
        Array.from(container.querySelectorAll("a[href]"))
          .map((link) => itemIdFromHref(link.href))
          .filter(Boolean),
      );
      if (ids.size === 1) return ids.values().next().value;
      if (ids.size > 1) return null;
      container = container.parentElement;
    }
    return null;
  };
  const dropdownUrls = (element) => {
    const raw = element.getAttribute("data-dropdown-items");
    if (!raw || raw.length > 100000) return [];
    let items;
    try { items = JSON.parse(raw); } catch { return []; }
    if (!Array.isArray(items)) return [];
    return items.map((item) => item && typeof item.path === "string" ? item.path : null).filter(Boolean);
  };
  const contextByDownload = new Map();
  const downloadKey = ({ downloadableId, variationId }) => `${downloadableId}:${variationId}`;
  const enhance = () => {
    simplifyLibraryChrome();
    contextByDownload.clear();
    const validAlternatives = new Set();
    for (const button of actionElements(document)) {
      if (!labelIs(button, DOWNLOAD_LABEL)) continue;
      const pair = findPair(button);
      if (!pair || pair.download !== button) continue;
      const itemId = findItemId(pair.container);
      if (!itemId) continue;
      const candidates = [];
      if (button.matches("a[href]")) candidates.push(button.href);
      candidates.push(...dropdownUrls(button));
      const downloads = candidates.map(parseDownloadUrl).filter(Boolean);
      if (downloads.length === 0) continue;
      for (const download of downloads) contextByDownload.set(downloadKey(download), itemId);
      pair.alternative.classList.add(HIDDEN_CLASS);
      validAlternatives.add(pair.alternative);
    }
    for (const hidden of document.querySelectorAll(`.${HIDDEN_CLASS}`)) {
      if (!validAlternatives.has(hidden)) hidden.classList.remove(HIDDEN_CLASS);
    }
  };

  const pendingIntents = new Map();
  const rejectIntent = (requestId) => {
    const pending = pendingIntents.get(requestId);
    if (!pending) return;
    window.clearTimeout(pending.timer);
    pendingIntents.delete(requestId);
    window.__boothShelfNotify({ requestId, state: "failed" });
  };
  window.__boothShelfAcceptDownloadIntent = (requestId) => {
    const key = String(requestId);
    const pending = pendingIntents.get(key);
    if (!pending) return;
    window.clearTimeout(pending.timer);
    pendingIntents.delete(key);
    window.location.assign(pending.href);
  };
  window.__boothShelfRejectDownloadIntent = (requestId) => rejectIntent(String(requestId));
  const registerDownloadIntent = (download, itemId) => {
    const requestId = crypto.randomUUID();
    const timer = window.setTimeout(() => rejectIntent(requestId), INTENT_TIMEOUT_MS);
    pendingIntents.set(requestId, { href: download.href, timer });
    const query = new URLSearchParams({
      request_id: requestId,
      item_id: String(itemId),
      variation_id: String(download.variationId),
      downloadable_id: String(download.downloadableId),
    });
    window.location.assign(`booth-shelf://download-intent?${query}`);
  };

  const start = () => {
    if (!document.documentElement) {
      setTimeout(start, 0);
      return;
    }
    const style = document.createElement("style");
    style.textContent = `
      .${HIDDEN_CLASS}, .${HIDDEN_CHROME_CLASS} { display: none !important; }
      [${LIBRARY_MAIN_ATTRIBUTE}="true"] { margin-top: 0 !important; padding-top: 16px !important; }
    `;
    document.documentElement.appendChild(style);
    document.addEventListener("click", (event) => {
      const target = event.target;
      if (!(target instanceof Element)) return;
      const link = target.closest("a[href]");
      if (!link) return;
      const download = parseDownloadUrl(link.href);
      if (!download) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const itemId = contextByDownload.get(downloadKey(download)) || findItemId(link);
      if (!itemId) {
        window.__boothShelfNotify({ requestId: crypto.randomUUID(), state: "failed" });
        return;
      }
      registerDownloadIntent(download, itemId);
    }, true);
    let scheduled = false;
    const scheduleEnhance = () => {
      if (scheduled) return;
      scheduled = true;
      requestAnimationFrame(() => {
        scheduled = false;
        enhance();
      });
    };
    new MutationObserver(scheduleEnhance).observe(document.documentElement, {
      childList: true,
      subtree: true,
    });
    scheduleEnhance();
  };

  start();
})();
