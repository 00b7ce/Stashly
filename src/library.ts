export type Product = {
  itemId: number;
  name: string;
  shopName: string;
  productUrl: string;
  thumbnailUrl: string | null;
  localPath: string | null;
  latestArtifactPath: string | null;
  artifactCount: number;
  lastDownloadedAt: string | null;
};

export type LibrarySnapshot = {
  products: Product[];
  libraryRoot: string | null;
};

export type DownloadState = "downloading" | "completed" | "failed";

export type DownloadStatus = {
  requestId: string;
  itemId: number | null;
  filename: string | null;
  state: DownloadState;
  message: string;
};

export function mergeDownloadStatus(
  current: DownloadStatus[],
  next: DownloadStatus,
  limit = 5,
): DownloadStatus[] {
  const key = next.requestId || "general-download-error";
  return [
    next,
    ...current.filter(
      (entry) => (entry.requestId || "general-download-error") !== key,
    ),
  ].slice(0, limit);
}

export function filterProducts(products: Product[], query: string): Product[] {
  const normalized = query.trim().toLocaleLowerCase("ja");
  if (!normalized) return products;
  return products.filter((product) =>
    [product.name, product.shopName, String(product.itemId)].some((value) =>
      value.toLocaleLowerCase("ja").includes(normalized),
    ),
  );
}

export function formatDate(value: string | null): string {
  if (!value) return "未ダウンロード";
  return new Intl.DateTimeFormat("ja-JP", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value));
}
