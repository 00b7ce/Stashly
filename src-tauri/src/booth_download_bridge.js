(() => {
  "use strict";

  const BOOTH_LIBRARY_ORIGIN = "https://accounts.booth.pm";
  const NOTIFICATION_HOST_ID = "booth-shelf-download-notifications";
  const NOTIFICATION_STYLE_ID = "booth-shelf-download-notification-style";
  const NOTIFICATION_DURATION_MS = 6000;
  const DOWNLOAD_LABEL = "ダウンロード";
  const ALTERNATIVE_LABEL = "その他のDL方法";
  const BLM_LABEL = "BOOTH Library ManagerでDL";
  const HIDDEN_CLASS = "booth-shelf-hidden-download-option";
  const HIDDEN_CHROME_CLASS = "booth-shelf-hidden-library-chrome";
  const LIBRARY_MAIN_ATTRIBUTE = "data-booth-shelf-library-main";
  const ENHANCED_ATTRIBUTE = "data-booth-shelf-download";
  const LIBRARY_TAB_LABELS = ["購入した商品", "ギフト", "無料ダウンロード"];

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
          align-items: center; overflow: hidden;
          border: 1px solid #dbe1eb; border-radius: 14px;
          background: #fffffff2; box-shadow: 0 8px 28px #25304722;
          backdrop-filter: blur(12px); color: #687287;
          transition: transform 160ms ease, box-shadow 160ms ease;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed { color: #2d8a6a; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.failed { color: #b43d57; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed[data-request-id] {
          cursor: pointer;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification.completed[data-request-id]:hover {
          transform: translateY(-2px); box-shadow: 0 12px 34px #25304730;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification-icon {
          width: 24px; height: 24px; display: grid; place-items: center;
          font-size: 20px; font-weight: 800;
        }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification:not(.completed):not(.failed)
          .booth-shelf-notification-icon { animation: booth-shelf-spin 1s linear infinite; }
        #${NOTIFICATION_HOST_ID} strong,
        #${NOTIFICATION_HOST_ID} span {
          display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
        }
        #${NOTIFICATION_HOST_ID} strong { color: #2a3347; font-size: 13px; }
        #${NOTIFICATION_HOST_ID} span { margin-top: 5px; font-size: 11px; }
        #${NOTIFICATION_HOST_ID} button {
          width: 30px; height: 30px; display: grid;
          place-items: center; border: 0; border-radius: 7px;
          background: transparent; color: #8b95a8; cursor: pointer;
        }
        #${NOTIFICATION_HOST_ID} button:hover { background: #edf0f5; color: #394357; }
        #${NOTIFICATION_HOST_ID} .booth-shelf-notification-expiry {
          position: absolute; right: 0; bottom: 0; left: 0; height: 4px;
          background: #e6ebf2;
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
    let row = Array.from(host.children).find(
      (element) => element.dataset.notificationKey === key,
    );
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
    !(
      window.location.pathname === "/library" ||
      window.location.pathname.startsWith("/library/")
    )
  ) {
    return;
  }

  const normalize = (value) => (value || "").replace(/\s+/g, "").trim();
  const labelIs = (element, label) =>
    normalize(element.textContent) === normalize(label);
  const actionElements = (root) =>
    Array.from(
      root.querySelectorAll('button, a, [role="button"], [role="menuitem"]'),
    );

  const findLibraryTabs = () =>
    Array.from(document.querySelectorAll("nav")).find((navigation) => {
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

  const hideLibraryFooter = () => {
    for (const footer of document.querySelectorAll(
      'footer, [role="contentinfo"]',
    )) {
      footer.classList.add(HIDDEN_CHROME_CLASS);
    }
  };

  const simplifyLibraryChrome = () => {
    const tabs = findLibraryTabs();
    if (!tabs) return;

    hideLibraryFooter();

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
      const downloads = actions.filter((element) =>
        labelIs(element, DOWNLOAD_LABEL),
      );
      const alternatives = actions.filter((element) =>
        labelIs(element, ALTERNATIVE_LABEL),
      );
      if (downloads.length === 1 && alternatives.length === 1) {
        return { container, alternative: alternatives[0] };
      }
      container = container.parentElement;
    }
    return null;
  };

  const isVisible = (element) =>
    element.getClientRects().length > 0 &&
    window.getComputedStyle(element).visibility !== "hidden";

  const findVisibleTextElement = (root, label) => {
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
    let textNode = walker.nextNode();
    while (textNode) {
      if (normalize(textNode.nodeValue) === normalize(label)) {
        const element = textNode.parentElement;
        if (element && isVisible(element)) return element;
      }
      textNode = walker.nextNode();
    }
    return null;
  };

  const findBlmAction = (container) => {
    const localLink = Array.from(container.querySelectorAll("a[href]")).find(
      (element) =>
        (element.getAttribute("href") || "").startsWith(
          "booth-library-manager://",
        ),
    );
    if (localLink) return localLink;

    const visibleLink = Array.from(document.querySelectorAll("a[href]")).find(
      (element) =>
        isVisible(element) &&
        (element.getAttribute("href") || "").startsWith(
          "booth-library-manager://",
        ),
    );
    if (visibleLink) return visibleLink;

    return (
      actionElements(container).find(
        (element) => isVisible(element) && labelIs(element, BLM_LABEL),
      ) ||
      findVisibleTextElement(container, BLM_LABEL) ||
      findVisibleTextElement(document, BLM_LABEL)
    );
  };

  const waitForBlmAction = async (container) => {
    const deadline = performance.now() + 1200;
    do {
      const action = findBlmAction(container);
      if (action) return action;
      await new Promise((resolve) => setTimeout(resolve, 25));
    } while (performance.now() < deadline);
    return null;
  };

  const activateBlmAction = (action) => {
    const link = action.matches("a[href]")
      ? action
      : action.closest("a[href]") || action.querySelector("a[href]");
    const href = link?.getAttribute("href") || "";
    if (href.startsWith("booth-library-manager://")) {
      window.location.assign(href);
      return;
    }
    action.click();
  };

  const restoreAlternative = (alternative, shouldHide) => {
    alternative.style.removeProperty("position");
    alternative.style.removeProperty("left");
    alternative.style.removeProperty("top");
    alternative.style.removeProperty("visibility");
    if (shouldHide) alternative.classList.add(HIDDEN_CLASS);
  };

  const dismissAlternativeMenu = (action) => {
    setTimeout(() => {
      let overlay = action.parentElement;
      for (let depth = 0; overlay && depth < 8; depth += 1) {
        const position = window.getComputedStyle(overlay).position;
        if (
          (position === "absolute" || position === "fixed") &&
          normalize(overlay.textContent).includes(normalize(BLM_LABEL))
        ) {
          overlay.classList.add(HIDDEN_CLASS);
          break;
        }
        overlay = overlay.parentElement;
      }

      const outside = document.body || document.documentElement;
      const mouseOptions = { bubbles: true, cancelable: true, view: window };
      outside.dispatchEvent(new PointerEvent("pointerdown", mouseOptions));
      outside.dispatchEvent(new MouseEvent("mousedown", mouseOptions));
      outside.dispatchEvent(new PointerEvent("pointerup", mouseOptions));
      outside.dispatchEvent(new MouseEvent("mouseup", mouseOptions));
      outside.dispatchEvent(new MouseEvent("click", mouseOptions));
    }, 0);
  };

  const bridgeDownload = async (downloadButton) => {
    if (downloadButton.dataset.boothShelfBusy === "true") return;
    const pair = findPair(downloadButton);
    if (!pair) return;

    downloadButton.dataset.boothShelfBusy = "true";
    const existingAction = findBlmAction(pair.container);
    if (existingAction) {
      const menuWasVisible = isVisible(existingAction);
      activateBlmAction(existingAction);
      if (menuWasVisible) dismissAlternativeMenu(existingAction);
      return;
    }

    pair.alternative.classList.remove(HIDDEN_CLASS);
    pair.alternative.style.setProperty("position", "fixed", "important");
    pair.alternative.style.setProperty("left", "-10000px", "important");
    pair.alternative.style.setProperty("top", "0", "important");
    pair.alternative.style.setProperty("visibility", "hidden", "important");
    pair.alternative.click();

    const action = await waitForBlmAction(pair.container);
    if (action) {
      restoreAlternative(pair.alternative, true);
      activateBlmAction(action);
      dismissAlternativeMenu(action);
      return;
    }

    restoreAlternative(pair.alternative, false);
    downloadButton.dataset.boothShelfBusy = "false";
    window.alert(
      "Booth Shelf用のダウンロードリンクを取得できませんでした。表示された「その他のDL方法」から再試行してください。",
    );
  };

  const enhance = () => {
    simplifyLibraryChrome();
    for (const button of actionElements(document)) {
      if (!labelIs(button, DOWNLOAD_LABEL) || button.hasAttribute(ENHANCED_ATTRIBUTE)) {
        continue;
      }
      const pair = findPair(button);
      if (!pair) continue;
      button.setAttribute(ENHANCED_ATTRIBUTE, "true");
      button.setAttribute("title", "Booth Shelfでダウンロード");
      pair.alternative.classList.add(HIDDEN_CLASS);
    }
  };

  const start = () => {
    if (!document.documentElement) {
      setTimeout(start, 0);
      return;
    }

    const style = document.createElement("style");
    style.textContent = `
      .${HIDDEN_CLASS}, .${HIDDEN_CHROME_CLASS} { display: none !important; }
      [${LIBRARY_MAIN_ATTRIBUTE}="true"] {
        margin-top: 0 !important;
        padding-top: 16px !important;
      }
    `;
    document.documentElement.appendChild(style);

    document.addEventListener(
      "click",
      (event) => {
        const target = event.target;
        if (!(target instanceof Element)) return;
        const button = target.closest(`[${ENHANCED_ATTRIBUTE}="true"]`);
        if (!button) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        bridgeDownload(button);
      },
      true,
    );

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
