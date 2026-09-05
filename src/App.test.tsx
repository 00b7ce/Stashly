// @vitest-environment jsdom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AccentColorPicker,
  ActivityPanel,
  App,
  BrowserToolbar,
  BrowserDataConfirmationDialog,
  browserViewForUrl,
  DeleteConfirmationDialog,
  LibraryRootConfirmationDialog,
  LibraryViewPicker,
  OFFICIAL_PRIVACY_URL,
  OFFICIAL_TERMS_URL,
  ProductCard,
  SettingsView,
  ThemePicker,
  normalizeAccentColor,
} from "./App";
import type { DownloadStatus, Product } from "./library";

afterEach(cleanup);

const product: Product = {
  itemId: 12345,
  name: "Sample product",
  shopName: "Sample shop",
  productUrl: "https://booth.pm/ja/items/12345",
  thumbnailUrl: null,
  localPath: "F:\\BOOTH\\Sample product [booth-12345]",
  latestArtifactPath:
    "F:\\BOOTH\\Sample product [booth-12345]\\variation-1\\sample.zip",
  artifactCount: 2,
  lastDownloadedAt: "2026-09-03T00:00:00Z",
};

describe("ProductCard", () => {
  it("offers one folder action instead of opening a single artifact", () => {
    const markup = renderToStaticMarkup(
      <ProductCard
        product={product}
        onError={() => undefined}
        onOpenProduct={() => undefined}
        onRefreshMetadata={async () => undefined}
      />,
    );

    expect(markup.match(/<button/g)).toHaveLength(2);
    expect(markup).toContain("フォルダを開く");
    expect(markup).toContain("BOOTH");
  });

  it("opens a context menu with folder and per-product metadata actions", async () => {
    const onRefreshMetadata = vi.fn().mockResolvedValue(undefined);
    render(
      <ProductCard
        product={product}
        onError={() => undefined}
        onOpenProduct={() => undefined}
        onRefreshMetadata={onRefreshMetadata}
      />,
    );

    fireEvent.contextMenu(screen.getByRole("article"), { clientX: 120, clientY: 80 });
    const menu = screen.getByRole("menu", { name: "Sample product の操作" });
    expect(menu.textContent).toContain("フォルダを開く");
    fireEvent.click(screen.getByRole("menuitem", { name: "商品情報を再取得" }));

    await waitFor(() => expect(onRefreshMetadata).toHaveBeenCalledWith(12345));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("opens from the keyboard and returns focus with Escape", () => {
    render(
      <ProductCard
        product={product}
        onError={() => undefined}
        onOpenProduct={() => undefined}
        onRefreshMetadata={async () => undefined}
      />,
    );

    const card = screen.getByRole("article");
    card.focus();
    fireEvent.keyDown(card, { key: "F10", shiftKey: true });
    expect(screen.getByRole("menu")).toBeTruthy();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(card);
  });
});

describe("App navigation", () => {
  it("places both BOOTH destinations in the persistent sidebar", () => {
    const markup = renderToStaticMarkup(<App />);

    expect(markup.match(/BOOTHを開く/g)).toHaveLength(1);
    expect(markup.match(/BOOTHライブラリ/g)).toHaveLength(1);
    expect(markup).toContain("ローカルライブラリ");
    expect(markup).toContain("設定");
    expect(markup).toContain("sidebar-settings");
    expect(markup.indexOf("sidebar-settings")).toBeLessThan(markup.indexOf("sidebar-toggle"));
    expect(markup).toContain("サイドバーを折りたたむ");
    expect(markup).toContain("Stashly");
    expect(markup).not.toContain("for BOOTH");
    expect(markup).toContain("BOOTH非公式アプリ");
    expect(markup).toContain("unofficial-short");
  });

  it("selects the BOOTH destination for product and dashboard locations", () => {
    expect(browserViewForUrl("https://booth.pm/ja/items/3813504")).toBe("booth");
    expect(browserViewForUrl("https://accounts.booth.pm/dashboard")).toBe("booth");
    expect(browserViewForUrl("https://accounts.booth.pm/library/free_downloads?page=1"))
      .toBe("booth-library");
    expect(browserViewForUrl("https://accounts.pixiv.net/login")).toBeNull();
    expect(browserViewForUrl("https://booth.pm.example.test/items/3813504")).toBeNull();
  });

  it("keeps the library heading, search, view controls, and refresh action together", () => {
    const markup = renderToStaticMarkup(<App />);

    expect(markup).toContain("library-toolbar");
    expect(markup).toContain("ローカルライブラリ");
    expect(markup).toContain("ライブラリを検索");
    expect(markup).toContain("再読み込み");
    expect(markup).toContain("ライブラリの表示方法");
  });
});

describe("BrowserToolbar", () => {
  it("offers navigation and a non-editable click-to-copy address", () => {
    const markup = renderToStaticMarkup(
      <BrowserToolbar
        currentUrl="https://accounts.booth.pm/library?page=2"
        onNavigate={() => undefined}
      />,
    );

    expect(markup.match(/<button/g)).toHaveLength(4);
    expect(markup).toContain('aria-label="戻る"');
    expect(markup).toContain('aria-label="進む"');
    expect(markup).toContain('aria-label="ページを更新"');
    expect(markup).toContain("クリックしてコピー");
    expect(markup).not.toContain("<input");
    expect(markup).not.toContain("contenteditable");
    expect(markup).toContain("https://accounts.booth.pm/library?page=2");
  });

  it("copies on activation, shows confirmation, and suppresses the context menu", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    render(
      <BrowserToolbar
        currentUrl="https://accounts.booth.pm/library?page=2"
        onNavigate={() => undefined}
      />,
    );
    const address = screen.getByRole("button", { name: /現在のBOOTH・pixiv公式ページURL/ });
    const contextMenu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    const middleMouseDown = new MouseEvent("mousedown", {
      bubbles: true,
      button: 1,
      cancelable: true,
    });
    const dragStart = new Event("dragstart", { bubbles: true, cancelable: true });

    address.dispatchEvent(contextMenu);
    expect(contextMenu.defaultPrevented).toBe(true);
    address.dispatchEvent(middleMouseDown);
    expect(middleMouseDown.defaultPrevented).toBe(true);
    address.dispatchEvent(dragStart);
    expect(dragStart.defaultPrevented).toBe(true);
    fireEvent.click(address);

    await waitFor(() => expect(writeText).toHaveBeenCalledWith(
      "https://accounts.booth.pm/library?page=2",
    ));
    expect(screen.getByRole("status").textContent).toContain("✓URLをコピーしました");
  });
});

describe("SettingsView", () => {
  it("identifies the app as unofficial and separates official policy links", () => {
    const markup = renderToStaticMarkup(
      <SettingsView
        appVersion="1.0.1"
        libraryRoot={null}
        libraryStorage={null}
        themeMode="system"
        accentColor="#e76b8c"
        deleting={false}
        cleanupMessage={null}
        clearingBrowserData={false}
        browserDataMessage={null}
        onChooseRoot={() => undefined}
        onThemeChange={() => undefined}
        onAccentColorChange={() => undefined}
        onDelete={() => undefined}
        onClearBrowserData={() => undefined}
        onOpenPrivacyPolicy={() => undefined}
        onOpenOfficialInformation={() => undefined}
      />,
    );

    expect(markup).toContain("バージョン");
    expect(markup).toContain("1.0.1");
    expect(markup).toContain("非公式アプリ");
    expect(markup).toContain("Stashlyのサポート窓口ではありません");
    expect(markup).toContain("Stashlyのプライバシー");
    expect(markup).toContain("プライバシーポリシーを表示");
    expect(markup).toContain(`href="${OFFICIAL_TERMS_URL}"`);
    expect(markup).toContain(`href="${OFFICIAL_PRIVACY_URL}"`);
    expect(markup).not.toContain("BOOTH公式サポート");
  });
});

describe("LibraryRootConfirmationDialog", () => {
  it("requires an explicit acknowledgement for a non-local destination", () => {
    const onConfirm = vi.fn();
    render(
      <LibraryRootConfirmationDialog
        candidate={{
          path: "\\\\nas\\private\\BOOTH",
          kind: "network",
          reasons: ["network_path"],
        }}
        saving={false}
        onCancel={() => undefined}
        onConfirm={onConfirm}
      />,
    );

    const confirm = screen.getByRole("button", { name: "理解して使用" });
    expect((confirm as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText("\\\\nas\\private\\BOOTH")).toBeTruthy();
    fireEvent.click(screen.getByRole("checkbox"));
    expect((confirm as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});

describe("LibraryViewPicker", () => {
  it("offers three thumbnail sizes and a list view", () => {
    const markup = renderToStaticMarkup(
      <LibraryViewPicker value="medium" onChange={() => undefined} />,
    );

    expect(markup.match(/<button/g)).toHaveLength(4);
    expect(markup).toContain("大きいサムネイル");
    expect(markup).toContain("標準サムネイル");
    expect(markup).toContain("小さいサムネイル");
    expect(markup).toContain("リスト表示");
    expect(markup).toContain('class="active" aria-label="標準サムネイル" aria-pressed="true"');
  });
});

describe("ThemePicker", () => {
  it("offers system, light, and dark themes", () => {
    const markup = renderToStaticMarkup(
      <ThemePicker value="system" onChange={() => undefined} />,
    );

    expect(markup.match(/role="radio"/g)).toHaveLength(3);
    expect(markup).toContain("システム設定");
    expect(markup).toContain("ライト");
    expect(markup).toContain("ダーク");
    expect(markup).toContain('role="radio" aria-checked="true"');
  });
});

describe("AccentColorPicker", () => {
  it("offers presets and a custom color input", () => {
    const markup = renderToStaticMarkup(
      <AccentColorPicker value="#2f86d7" onChange={() => undefined} />,
    );

    expect(markup.match(/role="radio"/g)).toHaveLength(6);
    expect(markup).toContain('aria-label="ブルー"');
    expect(markup).toContain('aria-label="カスタムアクセントカラー"');
    expect(markup).toContain('type="color"');
    expect(markup).toContain("#2F86D7");
  });

  it("accepts only six-digit hexadecimal colors", () => {
    expect(normalizeAccentColor("#A1B2C3")).toBe("#a1b2c3");
    expect(normalizeAccentColor("#abc")).toBeNull();
    expect(normalizeAccentColor("red")).toBeNull();
  });
});

describe("DeleteConfirmationDialog", () => {
  it("shows an in-app destructive confirmation with the exact root", () => {
    const markup = renderToStaticMarkup(
      <DeleteConfirmationDialog
        libraryRoot={"F:\\BOOTH_files"}
        onCancel={() => undefined}
        onConfirm={() => undefined}
      />,
    );

    expect(markup).toContain('role="alertdialog"');
    expect(markup).toContain("F:\\BOOTH_files");
    expect(markup).toContain("キャンセル");
    expect(markup).toContain("すべて削除");
  });
});

describe("BrowserDataConfirmationDialog", () => {
  it("explains the isolated data scope before clearing it", () => {
    const markup = renderToStaticMarkup(
      <BrowserDataConfirmationDialog
        onCancel={() => undefined}
        onConfirm={() => undefined}
      />,
    );

    expect(markup).toContain('role="alertdialog"');
    expect(markup).toContain("Cookie、キャッシュ、閲覧履歴、ローカルストレージ");
    expect(markup).toContain("ダウンロード済みファイル");
    expect(markup).toContain("個人データを削除");
  });
});

describe("ActivityPanel", () => {
  it("makes completed downloads openable and shows an expiry bar", () => {
    const completed: DownloadStatus = {
      requestId: "request-1",
      itemId: 12345,
      filename: "sample.zip",
      state: "completed",
      message: "Saved",
    };
    const markup = renderToStaticMarkup(
      <ActivityPanel
        activity={[completed]}
        onDismiss={() => undefined}
        onOpenFolder={() => undefined}
      />,
    );

    expect(markup).toContain("activity-open");
    expect(markup).toContain("activity-expiry");
    expect(markup).toContain("sample.zip");
    expect(markup).not.toContain("activity-open\" disabled");
  });
});
