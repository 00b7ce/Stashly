import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import {
  AccentColorPicker,
  ActivityPanel,
  App,
  BrowserToolbar,
  DeleteConfirmationDialog,
  LibraryViewPicker,
  ProductCard,
  ThemePicker,
  normalizeAccentColor,
} from "./App";
import type { DownloadStatus, Product } from "./library";

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
      />,
    );

    expect(markup.match(/<button/g)).toHaveLength(2);
    expect(markup).toContain("フォルダを開く");
    expect(markup).toContain("BOOTH");
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
  it("offers back, forward, and reload actions", () => {
    const markup = renderToStaticMarkup(
      <BrowserToolbar onNavigate={() => undefined} />,
    );

    expect(markup.match(/<button/g)).toHaveLength(3);
    expect(markup).toContain('aria-label="戻る"');
    expect(markup).toContain('aria-label="進む"');
    expect(markup).toContain('aria-label="ページを更新"');
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
