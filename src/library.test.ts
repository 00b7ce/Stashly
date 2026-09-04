import { describe, expect, it } from "vitest";
import {
  filterProducts,
  mergeDownloadStatus,
  type DownloadStatus,
  type Product,
} from "./library";

const products: Product[] = [
  {
    itemId: 12345,
    name: "星空アクセサリー",
    shopName: "Moon Shop",
    productUrl: "https://booth.pm/ja/items/12345",
    thumbnailUrl: null,
    localPath: null,
    latestArtifactPath: null,
    artifactCount: 0,
    lastDownloadedAt: null,
  },
];

describe("filterProducts", () => {
  it("matches product name, shop, and BOOTH item ID", () => {
    expect(filterProducts(products, "星空")).toHaveLength(1);
    expect(filterProducts(products, "moon")).toHaveLength(1);
    expect(filterProducts(products, "12345")).toHaveLength(1);
    expect(filterProducts(products, "missing")).toHaveLength(0);
  });
});

describe("mergeDownloadStatus", () => {
  it("replaces earlier states from the same download", () => {
    const downloading: DownloadStatus = {
      requestId: "request-1",
      itemId: 12345,
      filename: "package.zip",
      state: "downloading",
      message: "Downloading",
    };
    const completed: DownloadStatus = {
      ...downloading,
      state: "completed",
      message: "Completed",
    };

    expect(mergeDownloadStatus([downloading], completed)).toEqual([completed]);
  });
});
