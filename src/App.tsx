import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  ArrowLeft,
  ArrowRight,
  Box,
  CheckCircle2,
  Cookie,
  Copy,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Grid2X2,
  Grid3X3,
  Info,
  LayoutGrid,
  Library,
  List,
  LoaderCircle,
  Monitor,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Palette,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  ShoppingBag,
  Sun,
  Trash2,
  TriangleAlert,
  X,
} from "lucide-react";
import {
  filterProducts,
  formatDate,
  mergeDownloadStatus,
  type DownloadStatus,
  type LibrarySnapshot,
  type LibraryStorageKind,
  type LibraryStorageSummary,
  type Product,
} from "./library";
import { PrivacyPolicyDialog } from "./privacy";

const EMPTY_LIBRARY: LibrarySnapshot = {
  products: [],
  libraryRoot: null,
  libraryStorage: null,
};
const BOOTH_TOP_URL = "https://booth.pm/ja";
const BOOTH_LIBRARY_URL = "https://accounts.booth.pm/library";
const BROWSER_LOCATION_EVENT = "booth-browser-location";
export const OFFICIAL_TERMS_URL = "https://booth.pm/terms";
export const OFFICIAL_PRIVACY_URL = "https://booth.pm/privacy";
export type OfficialInformationUrl =
  | typeof OFFICIAL_TERMS_URL
  | typeof OFFICIAL_PRIVACY_URL;
type ActiveView = "library" | "booth" | "booth-library" | "settings";
export type LibraryViewMode = "large" | "medium" | "small" | "list";
export type BrowserNavigationAction = "back" | "forward" | "reload";
export type ThemeMode = "system" | "light" | "dark";

const LIBRARY_VIEW_STORAGE_KEY = "stashly-library-view";
const SIDEBAR_COLLAPSED_STORAGE_KEY = "stashly-sidebar-collapsed";
const THEME_MODE_STORAGE_KEY = "stashly-theme-mode";
const ACCENT_COLOR_STORAGE_KEY = "stashly-accent-color";

export function browserViewForUrl(value: string): Extract<ActiveView, "booth" | "booth-library"> | null {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return null;
  }
  if (url.protocol !== "https:") return null;
  if (
    url.hostname === "accounts.booth.pm" &&
    (url.pathname === "/library" || url.pathname.startsWith("/library/"))
  ) {
    return "booth-library";
  }
  if (url.hostname === "booth.pm" || url.hostname.endsWith(".booth.pm")) {
    return "booth";
  }
  return null;
}
const DEFAULT_ACCENT_COLOR = "#e76b8c";

function initialLibraryView(): LibraryViewMode {
  if (typeof window === "undefined") return "medium";
  try {
    const saved = window.localStorage.getItem(LIBRARY_VIEW_STORAGE_KEY);
    if (saved === "large" || saved === "medium" || saved === "small" || saved === "list") {
      return saved;
    }
  } catch {
    // A blocked localStorage should not prevent the library from rendering.
  }
  return "medium";
}

function initialSidebarCollapsed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(SIDEBAR_COLLAPSED_STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

function initialThemeMode(): ThemeMode {
  if (typeof window === "undefined") return "system";
  try {
    const saved = window.localStorage.getItem(THEME_MODE_STORAGE_KEY);
    if (saved === "system" || saved === "light" || saved === "dark") return saved;
  } catch {
    // Fall back to the system theme when persistence is unavailable.
  }
  return "system";
}

export function normalizeAccentColor(value: string): string | null {
  return /^#[0-9a-f]{6}$/i.test(value) ? value.toLowerCase() : null;
}

function initialAccentColor(): string {
  if (typeof window === "undefined") return DEFAULT_ACCENT_COLOR;
  try {
    return normalizeAccentColor(window.localStorage.getItem(ACCENT_COLOR_STORAGE_KEY) ?? "")
      ?? DEFAULT_ACCENT_COLOR;
  } catch {
    return DEFAULT_ACCENT_COLOR;
  }
}

function accentContrastColor(color: string): "#ffffff" | "#20283a" {
  const channels = [color.slice(1, 3), color.slice(3, 5), color.slice(5, 7)]
    .map((channel) => Number.parseInt(channel, 16) / 255)
    .map((channel) => channel <= 0.04045
      ? channel / 12.92
      : ((channel + 0.055) / 1.055) ** 2.4);
  const [red = 0, green = 0, blue = 0] = channels;
  const luminance = 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  return luminance > 0.34 ? "#20283a" : "#ffffff";
}

function systemPrefersDark(): boolean {
  return typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: dark)").matches === true;
}

type BrowserBounds = {
  x: number;
  y: number;
  width: number;
  height: number;
};

type DeleteLibraryResult = {
  removedArtifacts: number;
  removedPaths: number;
};

type MetadataFeedback = {
  kind: "success" | "error";
  message: string;
};

export type LibraryRootCandidate = {
  path: string;
  kind: Exclude<LibraryStorageKind, "local">;
  reasons: string[];
};

type SetLibraryRootResult =
  | { status: "saved"; library: LibrarySnapshot }
  | { status: "confirmation_required"; candidate: LibraryRootCandidate };

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function App() {
  const [library, setLibrary] = useState<LibrarySnapshot>(EMPTY_LIBRARY);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activity, setActivity] = useState<DownloadStatus[]>([]);
  const [activeView, setActiveView] = useState<ActiveView>("library");
  const [libraryView, setLibraryView] = useState<LibraryViewMode>(initialLibraryView);
  const [sidebarCollapsed, setSidebarCollapsed] = useState(initialSidebarCollapsed);
  const [themeMode, setThemeMode] = useState<ThemeMode>(initialThemeMode);
  const [accentColor, setAccentColor] = useState(initialAccentColor);
  const [systemDark, setSystemDark] = useState(systemPrefersDark);
  const [deleting, setDeleting] = useState(false);
  const [deleteConfirmationOpen, setDeleteConfirmationOpen] = useState(false);
  const [cleanupMessage, setCleanupMessage] = useState<string | null>(null);
  const [clearingBrowserData, setClearingBrowserData] = useState(false);
  const [browserDataConfirmationOpen, setBrowserDataConfirmationOpen] = useState(false);
  const [browserDataMessage, setBrowserDataMessage] = useState<string | null>(null);
  const [privacyPolicyOpen, setPrivacyPolicyOpen] = useState(false);
  const [pendingLibraryRoot, setPendingLibraryRoot] = useState<LibraryRootCandidate | null>(null);
  const [savingLibraryRoot, setSavingLibraryRoot] = useState(false);
  const [browserUrl, setBrowserUrl] = useState(BOOTH_TOP_URL);
  const [appVersion, setAppVersion] = useState("取得中…");
  const [metadataFeedback, setMetadataFeedback] = useState<MetadataFeedback | null>(null);
  const contentRef = useRef<HTMLElement>(null);
  const browserViewportRef = useRef<HTMLDivElement>(null);
  const notificationTimers = useRef(new Map<string, number>());
  const metadataFeedbackTimer = useRef<number | null>(null);
  const browserViewActive =
    activeView === "booth" || activeView === "booth-library";
  const effectiveTheme = themeMode === "system"
    ? (systemDark ? "dark" : "light")
    : themeMode;

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const updateSystemTheme = () => setSystemDark(media.matches);
    updateSystemTheme();
    media.addEventListener("change", updateSystemTheme);
    return () => media.removeEventListener("change", updateSystemTheme);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = effectiveTheme;
    document.documentElement.style.colorScheme = effectiveTheme;
  }, [effectiveTheme]);

  useEffect(() => {
    document.documentElement.style.setProperty("--accent", accentColor);
    document.documentElement.style.setProperty("--accent-contrast", accentContrastColor(accentColor));
  }, [accentColor]);

  useEffect(() => {
    void getVersion().then(setAppVersion).catch(() => setAppVersion("取得できませんでした"));
    const unlisten = listen<string>(BROWSER_LOCATION_EVENT, ({ payload }) => {
      setBrowserUrl(payload);
      const view = browserViewForUrl(payload);
      if (view) setActiveView(view);
    });
    return () => {
      void unlisten.then((dispose) => dispose());
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      setLibrary(await invoke<LibrarySnapshot>("get_library"));
      setError(null);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const unlisten = listen<DownloadStatus>("download-status", ({ payload }) => {
      const notificationKey = payload.requestId || "general-download-error";
      const existingTimer = notificationTimers.current.get(notificationKey);
      if (existingTimer !== undefined) window.clearTimeout(existingTimer);
      setActivity((current) => mergeDownloadStatus(current, payload));
      if (payload.state === "completed") {
        void refresh();
        const timer = window.setTimeout(() => {
          setActivity((current) =>
            current.filter(
              (entry) =>
                (entry.requestId || "general-download-error") !== notificationKey,
            ),
          );
          notificationTimers.current.delete(notificationKey);
        }, 6_000);
        notificationTimers.current.set(notificationKey, timer);
      }
    });
    return () => {
      void unlisten.then((dispose) => dispose());
      for (const timer of notificationTimers.current.values()) {
        window.clearTimeout(timer);
      }
      notificationTimers.current.clear();
    };
  }, [refresh]);

  const visibleProducts = useMemo(
    () => filterProducts(library.products, query),
    [library.products, query],
  );

  const dismissActivity = useCallback((requestId: string) => {
    const notificationKey = requestId || "general-download-error";
    const timer = notificationTimers.current.get(notificationKey);
    if (timer !== undefined) window.clearTimeout(timer);
    notificationTimers.current.delete(notificationKey);
    setActivity((current) =>
      current.filter(
        (entry) =>
          (entry.requestId || "general-download-error") !== notificationKey,
      ),
    );
  }, []);

  const openActivityFolder = useCallback((itemId: number) => {
    void invoke("open_product_folder", { itemId }).catch((reason) =>
      setError(errorText(reason)),
    );
  }, []);

  const showMetadataFeedback = useCallback((feedback: MetadataFeedback) => {
    if (metadataFeedbackTimer.current !== null) {
      window.clearTimeout(metadataFeedbackTimer.current);
    }
    setMetadataFeedback(feedback);
    metadataFeedbackTimer.current = window.setTimeout(() => {
      setMetadataFeedback(null);
      metadataFeedbackTimer.current = null;
    }, 5_000);
  }, []);

  useEffect(() => () => {
    if (metadataFeedbackTimer.current !== null) {
      window.clearTimeout(metadataFeedbackTimer.current);
    }
  }, []);

  const refreshProductMetadata = useCallback(async (itemId: number) => {
    try {
      const snapshot = await invoke<LibrarySnapshot>("refresh_product_metadata", { itemId });
      setLibrary(snapshot);
      setError(null);
      const updated = snapshot.products.find((product) => product.itemId === itemId);
      showMetadataFeedback({
        kind: "success",
        message: `${updated?.name ?? `商品 #${itemId}`}の商品情報を更新しました。`,
      });
    } catch (reason) {
      const message = errorText(reason);
      setError(message);
      showMetadataFeedback({ kind: "error", message });
    }
  }, [showMetadataFeedback]);

  const changeLibraryView = useCallback((view: LibraryViewMode) => {
    setLibraryView(view);
    try {
      window.localStorage.setItem(LIBRARY_VIEW_STORAGE_KEY, view);
    } catch {
      // Keep the in-memory selection when persistence is unavailable.
    }
  }, []);

  const changeThemeMode = useCallback((mode: ThemeMode) => {
    setThemeMode(mode);
    try {
      window.localStorage.setItem(THEME_MODE_STORAGE_KEY, mode);
    } catch {
      // Keep the in-memory selection when persistence is unavailable.
    }
  }, []);

  const changeAccentColor = useCallback((value: string) => {
    const normalized = normalizeAccentColor(value);
    if (!normalized) return;
    setAccentColor(normalized);
    try {
      window.localStorage.setItem(ACCENT_COLOR_STORAGE_KEY, normalized);
    } catch {
      // Keep the in-memory selection when persistence is unavailable.
    }
  }, []);

  const toggleSidebar = useCallback(() => {
    setSidebarCollapsed((collapsed) => {
      const next = !collapsed;
      try {
        window.localStorage.setItem(SIDEBAR_COLLAPSED_STORAGE_KEY, String(next));
      } catch {
        // Keep the in-memory selection when persistence is unavailable.
      }
      return next;
    });
  }, []);

  const navigateBooth = useCallback((action: BrowserNavigationAction) => {
    void invoke("navigate_booth_browser", { action }).catch((reason) =>
      setError(errorText(reason)),
    );
  }, []);

  const openOfficialInformation = useCallback((url: OfficialInformationUrl) => {
    void openUrl(url).catch((reason) => setError(errorText(reason)));
  }, []);

  const browserBounds = useCallback((): BrowserBounds | null => {
    const viewport = browserViewportRef.current;
    if (!viewport) return null;
    const rect = viewport.getBoundingClientRect();
    return {
      x: rect.left,
      y: rect.top,
      width: Math.max(1, rect.width),
      height: Math.max(1, rect.height),
    };
  }, []);

  useEffect(() => {
    if (!browserViewActive) {
      void invoke("hide_booth_browser").catch((reason) =>
        setError(errorText(reason)),
      );
      return;
    }

    let frame = 0;
    const resizeBrowser = () => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(() => {
        const bounds = browserBounds();
        if (!bounds) return;
        void invoke("resize_booth_browser", { bounds }).catch((reason) =>
          setError(errorText(reason)),
        );
      });
    };
    resizeBrowser();
    window.addEventListener("resize", resizeBrowser);
    return () => {
      window.removeEventListener("resize", resizeBrowser);
      window.cancelAnimationFrame(frame);
    };
  }, [browserBounds, browserViewActive, sidebarCollapsed]);

  async function saveLibraryRoot(root: string, allowNonLocal: boolean): Promise<boolean> {
    setSavingLibraryRoot(true);
    try {
      const result = await invoke<SetLibraryRootResult>("set_library_root", {
        root,
        allowNonLocal,
      });
      if (result.status === "confirmation_required") {
        setPendingLibraryRoot(result.candidate);
        setError(null);
        return false;
      }
      setLibrary(result.library);
      setPendingLibraryRoot(null);
      setCleanupMessage(null);
      setError(null);
      return true;
    } catch (reason) {
      setError(errorText(reason));
      return false;
    } finally {
      setSavingLibraryRoot(false);
    }
  }

  async function chooseLibraryRoot(): Promise<boolean> {
    const selected = await open({ directory: true, multiple: false });
    if (!selected) return false;
    return saveLibraryRoot(selected, false);
  }

  async function openBooth(view: "booth" | "booth-library", initialUrl: string) {
    setBrowserUrl(initialUrl);
    setActiveView(view);
    await new Promise<void>((resolve) =>
      window.requestAnimationFrame(() => resolve()),
    );
    const bounds = browserBounds();
    if (!bounds) {
      setActiveView("library");
      setError("BOOTH表示領域を取得できませんでした。");
      return;
    }
    try {
      await invoke("open_booth_browser", { initialUrl, bounds });
      setError(null);
    } catch (reason) {
      setActiveView("library");
      setError(errorText(reason));
    }
  }

  function requestDeleteAllDownloads() {
    const root = library.libraryRoot;
    if (!root || deleting) return;
    setDeleteConfirmationOpen(true);
  }

  async function confirmDeleteAllDownloads() {
    if (!library.libraryRoot || deleting) return;
    setDeleteConfirmationOpen(false);
    setDeleting(true);
    setCleanupMessage(null);
    try {
      const result = await invoke<DeleteLibraryResult>(
        "delete_downloaded_files",
      );
      await refresh();
      setCleanupMessage(
        `${result.removedArtifacts}件の登録データを削除しました（削除したパス: ${result.removedPaths}件）。`,
      );
      setError(null);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setDeleting(false);
    }
  }

  function requestClearBrowserData() {
    if (clearingBrowserData) return;
    setBrowserDataConfirmationOpen(true);
  }

  async function confirmClearBrowserData() {
    if (clearingBrowserData) return;
    setBrowserDataConfirmationOpen(false);
    setClearingBrowserData(true);
    setBrowserDataMessage(null);
    try {
      await invoke("clear_booth_browser_data");
      setBrowserDataMessage("BOOTHブラウザーの個人データを削除しました。次回表示時は再ログインが必要です。");
      setError(null);
    } catch (reason) {
      setError(errorText(reason));
    } finally {
      setClearingBrowserData(false);
    }
  }

  return (
    <div className={`app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><Box size={22} strokeWidth={2.2} /></div>
          <div className="brand-copy"><strong>Stashly</strong><small>Local asset library</small></div>
        </div>

        <nav className="nav-list" aria-label="メインナビゲーション">
          <button
            className={`nav-item ${activeView === "library" ? "active" : ""}`}
            onClick={() => setActiveView("library")}
            aria-current={activeView === "library" ? "page" : undefined}
          >
            <Library size={18} /><span className="nav-label">ローカルライブラリ</span>
          </button>
          <button
            className={`nav-item ${activeView === "booth" ? "active" : ""}`}
            onClick={() => void openBooth("booth", BOOTH_TOP_URL)}
            aria-current={activeView === "booth" ? "page" : undefined}
          >
            <ShoppingBag size={18} /><span className="nav-label">BOOTHを開く</span>
          </button>
          <button
            className={`nav-item ${activeView === "booth-library" ? "active" : ""}`}
            onClick={() => void openBooth("booth-library", BOOTH_LIBRARY_URL)}
            aria-current={activeView === "booth-library" ? "page" : undefined}
          >
            <Download size={18} /><span className="nav-label">BOOTHライブラリ</span>
          </button>
        </nav>

        <div className="sidebar-spacer" />
        <button
          className={`nav-item sidebar-settings ${activeView === "settings" ? "active" : ""}`}
          onClick={() => setActiveView("settings")}
          aria-current={activeView === "settings" ? "page" : undefined}
        >
          <Settings2 size={18} /><span className="nav-label">設定</span>
        </button>
        <button
          className="sidebar-toggle"
          type="button"
          onClick={toggleSidebar}
          aria-label={sidebarCollapsed ? "サイドバーを展開" : "サイドバーを折りたたむ"}
          title={sidebarCollapsed ? "サイドバーを展開" : "サイドバーを折りたたむ"}
        >
          {sidebarCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
          <span className="nav-label">折りたたむ</span>
        </button>
        <p className="unofficial"><span className="unofficial-full">BOOTH非公式アプリ</span><span className="unofficial-short">非公式</span></p>
      </aside>

      <main
        ref={contentRef}
        className={`content ${browserViewActive ? "browser-content" : ""} ${activeView === "library" ? "library-content" : ""}`}
      >
        {browserViewActive && (
          <>
            <BrowserToolbar
              currentUrl={browserUrl}
              onNavigate={navigateBooth}
              onCopyError={(message) => setError(message)}
            />
            <div ref={browserViewportRef} className="browser-viewport" aria-hidden="true" />
          </>
        )}

        {!browserViewActive && activeView === "settings" && (
          <header className="topbar">
            <div>
              <p className="eyebrow">PREFERENCES</p>
              <h1>設定</h1>
            </div>
          </header>
        )}

        {!browserViewActive && error && <div className="error-banner"><TriangleAlert size={18} />{error}</div>}

        {activeView === "settings" ? (
          <SettingsView
            appVersion={appVersion}
            libraryRoot={library.libraryRoot}
            libraryStorage={library.libraryStorage}
            themeMode={themeMode}
            accentColor={accentColor}
            deleting={deleting}
            cleanupMessage={cleanupMessage}
            clearingBrowserData={clearingBrowserData}
            browserDataMessage={browserDataMessage}
            onChooseRoot={() => void chooseLibraryRoot()}
            onThemeChange={changeThemeMode}
            onAccentColorChange={changeAccentColor}
            onDelete={requestDeleteAllDownloads}
            onClearBrowserData={requestClearBrowserData}
            onOpenPrivacyPolicy={() => setPrivacyPolicyOpen(true)}
            onOpenOfficialInformation={openOfficialInformation}
          />
        ) : activeView === "library" ? (
          <>
            <section className="library-toolbar">
              <div className="library-heading">
                <h1>ローカルライブラリ</h1>
                <span className="item-count">{visibleProducts.length} items</span>
              </div>
              <label className="search-box">
                <Search size={18} />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="商品名、ショップ名、商品IDで検索"
                  aria-label="ライブラリを検索"
                />
              </label>
              <LibraryViewPicker value={libraryView} onChange={changeLibraryView} />
              <button className="icon-button" onClick={() => void refresh()} aria-label="再読み込み">
                <RefreshCw size={18} className={loading ? "spin" : ""} />
              </button>
            </section>

            {!library.libraryRoot ? (
              <EmptyState
                icon={<FolderOpen size={34} />}
                title="まず保存先を選択します"
                detail="ダウンロードは選択したフォルダへ、商品名で整理して保存されます。"
                action="保存先を選択"
                onAction={() => void chooseLibraryRoot()}
              />
            ) : visibleProducts.length === 0 ? (
              <EmptyState
                icon={<ShoppingBag size={34} />}
                title={query ? "一致する商品がありません" : "ライブラリはまだ空です"}
                detail={query ? "別のキーワードを試してください。" : "BOOTHライブラリを開き、商品の「ダウンロード」を押してください。"}
                action={query ? "検索をクリア" : "BOOTHライブラリを開く"}
                onAction={() => query ? setQuery("") : void openBooth("booth-library", BOOTH_LIBRARY_URL)}
              />
            ) : (
              <section className={`product-grid view-${libraryView}`} aria-label="ダウンロード商品">
                {visibleProducts.map((product) => (
                  <ProductCard
                    key={product.itemId}
                    product={product}
                    onError={setError}
                    onOpenProduct={(url) => void openBooth("booth", url)}
                    onRefreshMetadata={refreshProductMetadata}
                  />
                ))}
              </section>
            )}
          </>
        ) : null}

        {activity.length > 0 && (
          <ActivityPanel
            activity={activity}
            onDismiss={dismissActivity}
            onOpenFolder={openActivityFolder}
          />
        )}

        {metadataFeedback && (
          <div
            className={`metadata-feedback ${metadataFeedback.kind}`}
            role={metadataFeedback.kind === "error" ? "alert" : "status"}
          >
            {metadataFeedback.kind === "success"
              ? <CheckCircle2 size={18} />
              : <TriangleAlert size={18} />}
            <span>{metadataFeedback.message}</span>
          </div>
        )}

        {deleteConfirmationOpen && library.libraryRoot && (
          <DeleteConfirmationDialog
            libraryRoot={library.libraryRoot}
            onCancel={() => setDeleteConfirmationOpen(false)}
            onConfirm={() => void confirmDeleteAllDownloads()}
          />
        )}

        {browserDataConfirmationOpen && (
          <BrowserDataConfirmationDialog
            onCancel={() => setBrowserDataConfirmationOpen(false)}
            onConfirm={() => void confirmClearBrowserData()}
          />
        )}

        {pendingLibraryRoot && (
          <LibraryRootConfirmationDialog
            candidate={pendingLibraryRoot}
            saving={savingLibraryRoot}
            onCancel={() => setPendingLibraryRoot(null)}
            onConfirm={() => void saveLibraryRoot(pendingLibraryRoot.path, true)}
          />
        )}

        {privacyPolicyOpen && (
          <PrivacyPolicyDialog onClose={() => setPrivacyPolicyOpen(false)} />
        )}
      </main>
    </div>
  );
}

export function BrowserToolbar({ currentUrl, onNavigate, onCopyError }: {
  currentUrl: string;
  onNavigate: (action: BrowserNavigationAction) => void;
  onCopyError?: (message: string) => void;
}) {
  const [copied, setCopied] = useState(false);
  const copiedTimer = useRef<number | null>(null);

  useEffect(() => () => {
    if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
  }, []);

  async function copyCurrentUrl() {
    try {
      if (!navigator.clipboard) throw new Error("クリップボードを利用できません");
      await navigator.clipboard.writeText(currentUrl);
      setCopied(true);
      if (copiedTimer.current !== null) window.clearTimeout(copiedTimer.current);
      copiedTimer.current = window.setTimeout(() => setCopied(false), 2_000);
    } catch (reason) {
      onCopyError?.(`URLをコピーできませんでした: ${errorText(reason)}`);
    }
  }

  return <header className="browser-toolbar" aria-label="BOOTHブラウザ操作">
    <button type="button" onClick={() => onNavigate("back")} aria-label="戻る" title="戻る">
      <ArrowLeft size={19} />
    </button>
    <button type="button" onClick={() => onNavigate("forward")} aria-label="進む" title="進む">
      <ArrowRight size={19} />
    </button>
    <button type="button" onClick={() => onNavigate("reload")} aria-label="ページを更新" title="ページを更新">
      <RefreshCw size={18} />
    </button>
    <button
      className="browser-location"
      type="button"
      onClick={() => void copyCurrentUrl()}
      onMouseDown={(event) => {
        if (event.button !== 0) event.preventDefault();
      }}
      onContextMenu={(event) => event.preventDefault()}
      onDragStart={(event) => event.preventDefault()}
      aria-label={`現在のBOOTH・pixiv公式ページURL: ${currentUrl}。クリックしてコピー`}
      title="クリックしてURLをコピー"
    >
      <span>{currentUrl}</span>
      <Copy size={16} aria-hidden="true" />
    </button>
    {copied && <div className="browser-copy-toast" role="status">
      <span aria-hidden="true">✓</span>URLをコピーしました
    </div>}
  </header>;
}

export function LibraryViewPicker({ value, onChange }: {
  value: LibraryViewMode;
  onChange: (view: LibraryViewMode) => void;
}) {
  const options: Array<{
    value: LibraryViewMode;
    label: string;
    icon: React.ReactNode;
  }> = [
    { value: "large", label: "大きいサムネイル", icon: <Grid2X2 size={17} /> },
    { value: "medium", label: "標準サムネイル", icon: <LayoutGrid size={17} /> },
    { value: "small", label: "小さいサムネイル", icon: <Grid3X3 size={17} /> },
    { value: "list", label: "リスト表示", icon: <List size={18} /> },
  ];

  return <div className="view-picker" role="group" aria-label="ライブラリの表示方法">
    {options.map((option) => (
      <button
        key={option.value}
        type="button"
        className={value === option.value ? "active" : ""}
        aria-label={option.label}
        aria-pressed={value === option.value}
        title={option.label}
        onClick={() => onChange(option.value)}
      >
        {option.icon}
      </button>
    ))}
  </div>;
}

export function ThemePicker({ value, onChange }: {
  value: ThemeMode;
  onChange: (mode: ThemeMode) => void;
}) {
  const options: Array<{
    value: ThemeMode;
    label: string;
    description: string;
    icon: React.ReactNode;
  }> = [
    { value: "system", label: "システム設定", description: "Windowsの設定に従います", icon: <Monitor size={19} /> },
    { value: "light", label: "ライト", description: "明るい配色を使用します", icon: <Sun size={19} /> },
    { value: "dark", label: "ダーク", description: "暗い配色を使用します", icon: <Moon size={19} /> },
  ];

  return <div className="theme-picker" role="radiogroup" aria-label="カラーテーマ">
    {options.map((option) => (
      <button
        key={option.value}
        type="button"
        className={value === option.value ? "active" : ""}
        role="radio"
        aria-checked={value === option.value}
        onClick={() => onChange(option.value)}
      >
        {option.icon}
        <span><strong>{option.label}</strong><small>{option.description}</small></span>
      </button>
    ))}
  </div>;
}

const ACCENT_PRESETS = [
  { color: "#e76b8c", label: "ローズ" },
  { color: "#e05a47", label: "コーラル" },
  { color: "#d08a24", label: "アンバー" },
  { color: "#2c9a73", label: "グリーン" },
  { color: "#2f86d7", label: "ブルー" },
  { color: "#8767d6", label: "バイオレット" },
] as const;

export function AccentColorPicker({ value, onChange }: {
  value: string;
  onChange: (color: string) => void;
}) {
  return <div className="accent-picker">
    <div className="accent-presets" role="radiogroup" aria-label="アクセントカラーのプリセット">
      {ACCENT_PRESETS.map((preset) => (
        <button
          key={preset.color}
          type="button"
          className={value === preset.color ? "active" : ""}
          role="radio"
          aria-checked={value === preset.color}
          aria-label={preset.label}
          title={preset.label}
          style={{ backgroundColor: preset.color }}
          onClick={() => onChange(preset.color)}
        />
      ))}
    </div>
    <label className="custom-color-picker">
      <span className="custom-color-preview" style={{ backgroundColor: value }} aria-hidden="true" />
      <span><strong>カスタムカラー</strong><small>カラーパレットから自由に選択</small></span>
      <code>{value.toUpperCase()}</code>
      <input
        type="color"
        value={value}
        aria-label="カスタムアクセントカラー"
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </label>
  </div>;
}

export function DeleteConfirmationDialog({ libraryRoot, onCancel, onConfirm }: {
  libraryRoot: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onCancel]);

  return <div
    className="modal-backdrop"
    onMouseDown={(event) => event.target === event.currentTarget && onCancel()}
  >
    <section className="confirmation-dialog" role="alertdialog" aria-modal="true" aria-labelledby="delete-dialog-title" aria-describedby="delete-dialog-description">
      <div className="confirmation-icon"><TriangleAlert size={25} /></div>
      <div className="confirmation-copy">
        <p className="eyebrow">DESTRUCTIVE ACTION</p>
        <h2 id="delete-dialog-title">ダウンロード済みファイルを削除しますか？</h2>
        <p id="delete-dialog-description">
          Stashlyが記録しているファイルと展開フォルダをすべて削除します。この操作は元に戻せません。管理対象外のファイルは削除しません。
        </p>
        <div className="confirmation-path"><span>保存先</span><code>{libraryRoot}</code></div>
      </div>
      <div className="confirmation-actions">
        <button ref={cancelRef} className="modal-cancel" type="button" onClick={onCancel}>キャンセル</button>
        <button className="modal-delete" type="button" onClick={onConfirm}><Trash2 size={17} />すべて削除</button>
      </div>
    </section>
  </div>;
}

export function BrowserDataConfirmationDialog({ onCancel, onConfirm }: {
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onCancel]);

  return <div
    className="modal-backdrop"
    onMouseDown={(event) => event.target === event.currentTarget && onCancel()}
  >
    <section className="confirmation-dialog" role="alertdialog" aria-modal="true" aria-labelledby="browser-data-dialog-title" aria-describedby="browser-data-dialog-description">
      <div className="confirmation-icon"><TriangleAlert size={25} /></div>
      <div className="confirmation-copy">
        <p className="eyebrow">PRIVACY DATA</p>
        <h2 id="browser-data-dialog-title">BOOTHブラウザーの個人データを削除しますか？</h2>
        <p id="browser-data-dialog-description">
          専用WebViewに保存されたCookie、キャッシュ、閲覧履歴、ローカルストレージなどを削除します。BOOTHとpixivからログアウトします。ダウンロード済みファイルとStashlyのライブラリ登録は削除しません。
        </p>
      </div>
      <div className="confirmation-actions">
        <button ref={cancelRef} className="modal-cancel" type="button" onClick={onCancel}>キャンセル</button>
        <button className="modal-delete" type="button" onClick={onConfirm}><Trash2 size={17} />個人データを削除</button>
      </div>
    </section>
  </div>;
}

const STORAGE_KIND_LABELS: Record<LibraryStorageKind, string> = {
  local: "ローカル保存先",
  network: "ネットワーク保存先",
  sync: "同期フォルダー",
  unknown: "種類を確認できない保存先",
};

export function LibraryRootConfirmationDialog({ candidate, saving, onCancel, onConfirm }: {
  candidate: LibraryRootCandidate;
  saving: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [acknowledged, setAcknowledged] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !saving) onCancel();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onCancel, saving]);

  return <div
    className="modal-backdrop"
    onMouseDown={(event) => event.target === event.currentTarget && !saving && onCancel()}
  >
    <section className="confirmation-dialog storage-confirmation" role="alertdialog" aria-modal="true" aria-labelledby="storage-dialog-title" aria-describedby="storage-dialog-description">
      <div className="confirmation-icon storage-warning-icon"><TriangleAlert size={25} /></div>
      <div className="confirmation-copy">
        <p className="eyebrow">EXTERNAL STORAGE</p>
        <h2 id="storage-dialog-title">{STORAGE_KIND_LABELS[candidate.kind]}を使用しますか？</h2>
        <p id="storage-dialog-description">
          この保存先はローカル固定ドライブとして確認できませんでした。次の点を理解した場合だけ使用してください。
        </p>
        <ul className="storage-warning-list">
          <li>購入者本人以外がアクセスできないことを確認してください。</li>
          <li>切断、遅延、同期競合によってダウンロードや展開が失敗する場合があります。</li>
          <li>同期サービスはバックアップの代わりにはなりません。</li>
        </ul>
        <div className="confirmation-path"><span>保存先</span><code>{candidate.path}</code></div>
        <label className="storage-acknowledgement">
          <input
            type="checkbox"
            checked={acknowledged}
            disabled={saving}
            onChange={(event) => setAcknowledged(event.currentTarget.checked)}
          />
          <span>リスクを理解し、この保存先を自分の責任で使用します。</span>
        </label>
      </div>
      <div className="confirmation-actions">
        <button ref={cancelRef} className="modal-cancel" type="button" onClick={onCancel} disabled={saving}>キャンセル</button>
        <button className="modal-confirm" type="button" onClick={onConfirm} disabled={!acknowledged || saving}>
          {saving ? <LoaderCircle size={17} className="spin" /> : <CheckCircle2 size={17} />}
          {saving ? "確認しています…" : "理解して使用"}
        </button>
      </div>
    </section>
  </div>;
}

export function SettingsView({ appVersion, libraryRoot, libraryStorage, themeMode, accentColor, deleting, cleanupMessage, clearingBrowserData, browserDataMessage, onChooseRoot, onThemeChange, onAccentColorChange, onDelete, onClearBrowserData, onOpenPrivacyPolicy, onOpenOfficialInformation }: {
  appVersion: string;
  libraryRoot: string | null;
  libraryStorage: LibraryStorageSummary | null;
  themeMode: ThemeMode;
  accentColor: string;
  deleting: boolean;
  cleanupMessage: string | null;
  clearingBrowserData: boolean;
  browserDataMessage: string | null;
  onChooseRoot: () => void;
  onThemeChange: (mode: ThemeMode) => void;
  onAccentColorChange: (color: string) => void;
  onDelete: () => void;
  onClearBrowserData: () => void;
  onOpenPrivacyPolicy: () => void;
  onOpenOfficialInformation: (url: OfficialInformationUrl) => void;
}) {
  return <section className="settings-layout" aria-label="設定">
    <article className="settings-card">
      <div className="settings-icon"><Moon size={22} /></div>
      <div className="settings-body">
        <h2>外観</h2>
        <p>Stashlyの表示テーマを選択します。BOOTHサイトには適用されません。</p>
        <ThemePicker value={themeMode} onChange={onThemeChange} />
        <div className="appearance-divider" />
        <div className="appearance-subheading"><Palette size={17} /><h3>アクセントカラー</h3></div>
        <p>ボタンや選択状態に使用する色を選択します。</p>
        <AccentColorPicker value={accentColor} onChange={onAccentColorChange} />
      </div>
    </article>

    <article className="settings-card">
      <div className="settings-icon"><Folder size={22} /></div>
      <div className="settings-body">
        <h2>保存先</h2>
        <p>BOOTHから取得したファイルを整理して保存するフォルダです。</p>
        <div className={`path-display ${libraryRoot ? "" : "unset"}`} title={libraryRoot ?? undefined}>
          {libraryRoot ?? "保存先が設定されていません"}
        </div>
        {libraryStorage && <div className={`storage-status ${libraryStorage.kind === "local" ? "local" : "external"}`}>
          {libraryStorage.kind === "local"
            ? "ローカル固定ドライブ"
            : `${STORAGE_KIND_LABELS[libraryStorage.kind]} · ${libraryStorage.nonLocalConfirmed ? "確認済み" : "再確認が必要"}`}
        </div>}
        <button className="secondary-button" onClick={onChooseRoot}>
          <FolderOpen size={17} />{libraryRoot ? "保存先を変更" : "保存先を選択"}
        </button>
      </div>
    </article>

    <article className="settings-card">
      <div className="settings-icon"><ShieldCheck size={22} /></div>
      <div className="settings-body">
        <h2>Stashlyのプライバシー</h2>
        <p>初回起動時に同意したStashlyのプライバシーポリシーを確認できます。</p>
        <button className="secondary-button" type="button" onClick={onOpenPrivacyPolicy}>
          <ShieldCheck size={17} />プライバシーポリシーを表示
        </button>
      </div>
    </article>

    <article className="settings-card about-card">
      <div className="settings-icon"><Info size={22} /></div>
      <div className="settings-body">
        <h2>このアプリについて</h2>
        <p>Stashlyはピクシブ株式会社、BOOTH、pixivとは提携・承認・協賛関係のない非公式アプリです。</p>
        <dl className="version-information">
          <dt>バージョン</dt>
          <dd>{appVersion}</dd>
        </dl>
        <div className="official-information">
          <h3>BOOTH・pixiv公式情報（外部サイト）</h3>
          <p>以下はBOOTH・pixivの公式文書です。Stashlyのサポート窓口ではありません。</p>
          <nav aria-label="BOOTH・pixiv公式情報">
            <a href={OFFICIAL_TERMS_URL} onClick={(event) => { event.preventDefault(); onOpenOfficialInformation(OFFICIAL_TERMS_URL); }}>
              <span><strong>サービス利用規約</strong><small>booth.pm</small></span><ExternalLink size={16} />
            </a>
            <a href={OFFICIAL_PRIVACY_URL} onClick={(event) => { event.preventDefault(); onOpenOfficialInformation(OFFICIAL_PRIVACY_URL); }}>
              <span><strong>プライバシーポリシー</strong><small>booth.pm</small></span><ExternalLink size={16} />
            </a>
          </nav>
        </div>
      </div>
    </article>

    <article className="settings-card danger-card">
      <div className="settings-icon danger-icon"><Cookie size={22} /></div>
      <div className="settings-body">
        <h2>BOOTHブラウザーの個人データ</h2>
        <p>専用WebViewに保存されたCookie、キャッシュ、閲覧履歴、ローカルストレージなどを削除します。ダウンロード済みファイルとライブラリ登録には影響しません。</p>
        {browserDataMessage && <div className="success-banner"><CheckCircle2 size={17} />{browserDataMessage}</div>}
        <button className="danger-button" onClick={onClearBrowserData} disabled={clearingBrowserData}>
          {clearingBrowserData ? <LoaderCircle size={17} className="spin" /> : <Trash2 size={17} />}
          {clearingBrowserData ? "削除しています…" : "BOOTHブラウザーの個人データを削除"}
        </button>
      </div>
    </article>

    <article className="settings-card danger-card">
      <div className="settings-icon danger-icon"><Trash2 size={22} /></div>
      <div className="settings-body">
        <h2>データの削除</h2>
        <p>StashlyがDBに記録したダウンロード済みファイル・展開フォルダとライブラリ登録を削除します。保存先そのものや、管理対象外のファイルは残します。</p>
        {cleanupMessage && <div className="success-banner"><CheckCircle2 size={17} />{cleanupMessage}</div>}
        <button className="danger-button" onClick={onDelete} disabled={!libraryRoot || deleting}>
          {deleting ? <LoaderCircle size={17} className="spin" /> : <Trash2 size={17} />}
          {deleting ? "削除しています…" : "ダウンロード済みファイルをすべて削除"}
        </button>
      </div>
    </article>
  </section>;
}

function EmptyState({ icon, title, detail, action, onAction }: {
  icon: React.ReactNode; title: string; detail: string; action: string; onAction: () => void;
}) {
  return <section className="empty-state">
    <div className="empty-icon">{icon}</div><h2>{title}</h2><p>{detail}</p>
    <button className="secondary-button" onClick={onAction}>{action}</button>
  </section>;
}

type ContextMenuPosition = { x: number; y: number };

export function ProductCard({ product, onError, onOpenProduct, onRefreshMetadata }: {
  product: Product;
  onError: (error: string) => void;
  onOpenProduct: (url: string) => void;
  onRefreshMetadata: (itemId: number) => Promise<void>;
}) {
  const [contextMenu, setContextMenu] = useState<ContextMenuPosition | null>(null);
  const [refreshingMetadata, setRefreshingMetadata] = useState(false);
  const cardRef = useRef<HTMLElement>(null);
  const contextMenuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!contextMenu) return;
    const menu = contextMenuRef.current;
    menu?.querySelector<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')?.focus();

    const closeOnPointerDown = (event: PointerEvent) => {
      if (!contextMenuRef.current?.contains(event.target as Node)) setContextMenu(null);
    };
    const closeOnKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setContextMenu(null);
        cardRef.current?.focus();
        return;
      }
      if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
      const items = Array.from(
        contextMenuRef.current?.querySelectorAll<HTMLButtonElement>(
          '[role="menuitem"]:not(:disabled)',
        ) ?? [],
      );
      if (items.length === 0) return;
      event.preventDefault();
      const current = items.indexOf(document.activeElement as HTMLButtonElement);
      const offset = event.key === "ArrowDown" ? 1 : -1;
      items[(current + offset + items.length) % items.length]?.focus();
    };
    const close = () => setContextMenu(null);
    window.addEventListener("pointerdown", closeOnPointerDown);
    window.addEventListener("keydown", closeOnKeyDown);
    window.addEventListener("blur", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("pointerdown", closeOnPointerDown);
      window.removeEventListener("keydown", closeOnKeyDown);
      window.removeEventListener("blur", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [contextMenu]);

  const openFolder = () => invoke("open_product_folder", { itemId: product.itemId }).catch((reason) => onError(errorText(reason)));
  const refreshMetadata = async () => {
    if (refreshingMetadata) return;
    setRefreshingMetadata(true);
    try {
      await onRefreshMetadata(product.itemId);
    } finally {
      setRefreshingMetadata(false);
    }
  };
  const showContextMenu = (requestedX: number, requestedY: number) => {
    const margin = 8;
    const menuWidth = 230;
    const menuHeight = 92;
    setContextMenu({
      x: Math.max(margin, Math.min(requestedX, window.innerWidth - menuWidth - margin)),
      y: Math.max(margin, Math.min(requestedY, window.innerHeight - menuHeight - margin)),
    });
  };
  const openContextMenu = (event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    showContextMenu(event.clientX || bounds.left + 16, event.clientY || bounds.top + 16);
  };
  const openContextMenuFromKeyboard = (event: React.KeyboardEvent<HTMLElement>) => {
    if (event.key !== "ContextMenu" && !(event.shiftKey && event.key === "F10")) return;
    event.preventDefault();
    const bounds = event.currentTarget.getBoundingClientRect();
    showContextMenu(bounds.left + 16, bounds.top + 16);
  };
  const contextMenuPortal = contextMenu && typeof document !== "undefined"
    ? createPortal(
      <div
        ref={contextMenuRef}
        className="product-context-menu"
        role="menu"
        aria-label={`${product.name} の操作`}
        style={{ left: contextMenu.x, top: contextMenu.y }}
        onContextMenu={(event) => {
          event.preventDefault();
          event.stopPropagation();
        }}
      >
        <button
          type="button"
          role="menuitem"
          disabled={!product.localPath}
          onClick={() => {
            setContextMenu(null);
            cardRef.current?.focus();
            void openFolder();
          }}
        >
          <FolderOpen size={16} />フォルダを開く
        </button>
        <button
          type="button"
          role="menuitem"
          disabled={refreshingMetadata}
          onClick={() => {
            setContextMenu(null);
            cardRef.current?.focus();
            void refreshMetadata();
          }}
        >
          <RefreshCw size={16} className={refreshingMetadata ? "spin" : ""} />
          {refreshingMetadata ? "商品情報を再取得中…" : "商品情報を再取得"}
        </button>
      </div>,
      document.body,
    )
    : null;

  return <article
    ref={cardRef}
    className="product-card"
    tabIndex={0}
    onContextMenu={openContextMenu}
    onKeyDown={openContextMenuFromKeyboard}
  >
    <div className="thumbnail" title="右クリックでメニューを開く">
      {product.thumbnailUrl ? <img src={product.thumbnailUrl} alt="" /> : <Box size={34} />}
      <span className="file-count"><Download size={13} />{product.artifactCount}</span>
    </div>
    <div className="product-body">
      <p className="shop-name">{product.shopName}</p>
      <h2>{product.name}</h2>
      <p className="product-meta">#{product.itemId} · {formatDate(product.lastDownloadedAt)}</p>
    </div>
    <div className="card-actions">
      <button onClick={() => void openFolder()} disabled={!product.localPath}><FolderOpen size={16} />フォルダを開く</button>
      <button onClick={() => onOpenProduct(product.productUrl)}><ExternalLink size={16} />BOOTH</button>
    </div>
    {contextMenuPortal}
  </article>;
}

export function ActivityPanel({ activity, onDismiss, onOpenFolder }: {
  activity: DownloadStatus[];
  onDismiss: (requestId: string) => void;
  onOpenFolder: (itemId: number) => void;
}) {
  return <aside className="activity-panel" aria-live="polite">
    {activity.map((entry, index) => {
      const canOpen = entry.state === "completed" && entry.itemId !== null;
      return <div className={`activity-row ${entry.state}`} key={entry.requestId || `general-${index}`}>
        <button
          className="activity-open"
          onClick={() => canOpen && onOpenFolder(entry.itemId!)}
          disabled={!canOpen}
          aria-label={canOpen ? `${entry.filename ?? "BOOTH download"} の保存フォルダを開く` : undefined}
        >
          {entry.state === "completed" ? <CheckCircle2 size={22} /> : entry.state === "failed" ? <TriangleAlert size={22} /> : <LoaderCircle size={22} className="spin" />}
          <span className="activity-copy"><strong>{entry.filename ?? "BOOTH download"}</strong><span>{entry.message}</span></span>
        </button>
        <button className="activity-dismiss" onClick={() => onDismiss(entry.requestId)} aria-label="通知を閉じる"><X size={17} /></button>
        {entry.state === "completed" && <span className="activity-expiry" aria-hidden="true" />}
      </div>;
    })}
  </aside>;
}
