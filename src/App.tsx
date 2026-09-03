import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft,
  ArrowRight,
  Box,
  CheckCircle2,
  Download,
  ExternalLink,
  Folder,
  FolderOpen,
  Grid2X2,
  Grid3X3,
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
  type Product,
} from "./library";

const EMPTY_LIBRARY: LibrarySnapshot = { products: [], libraryRoot: null };
const BOOTH_TOP_URL = "https://booth.pm/ja";
const BOOTH_LIBRARY_URL = "https://accounts.booth.pm/library";
type ActiveView = "library" | "booth" | "booth-library" | "settings";
export type LibraryViewMode = "large" | "medium" | "small" | "list";
export type BrowserNavigationAction = "back" | "forward" | "reload";
export type ThemeMode = "system" | "light" | "dark";

const LIBRARY_VIEW_STORAGE_KEY = "booth-shelf-library-view";
const SIDEBAR_COLLAPSED_STORAGE_KEY = "booth-shelf-sidebar-collapsed";
const THEME_MODE_STORAGE_KEY = "booth-shelf-theme-mode";
const ACCENT_COLOR_STORAGE_KEY = "booth-shelf-accent-color";
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
  const contentRef = useRef<HTMLElement>(null);
  const browserViewportRef = useRef<HTMLDivElement>(null);
  const notificationTimers = useRef(new Map<string, number>());
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
  }, [sidebarCollapsed]);

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
    window.addEventListener("resize", resizeBrowser);
    return () => {
      window.removeEventListener("resize", resizeBrowser);
      window.cancelAnimationFrame(frame);
    };
  }, [browserBounds, browserViewActive]);

  async function chooseLibraryRoot(): Promise<boolean> {
    const selected = await open({ directory: true, multiple: false });
    if (!selected) return false;
    try {
      setLibrary(
        await invoke<LibrarySnapshot>("set_library_root", { root: selected }),
      );
      setCleanupMessage(null);
      setError(null);
      return true;
    } catch (reason) {
      setError(errorText(reason));
      return false;
    }
  }

  async function openBooth(view: "booth" | "booth-library", initialUrl: string) {
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

  return (
    <div className={`app-shell ${sidebarCollapsed ? "sidebar-collapsed" : ""}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><Box size={22} strokeWidth={2.2} /></div>
          <div className="brand-copy"><strong>Booth Shelf</strong><span>Local library</span></div>
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
        <p className="unofficial">非公式クライアント · データは端末内のみ</p>
      </aside>

      <main
        ref={contentRef}
        className={`content ${browserViewActive ? "browser-content" : ""} ${activeView === "library" ? "library-content" : ""}`}
      >
        {browserViewActive && (
          <>
            <BrowserToolbar onNavigate={navigateBooth} />
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
            libraryRoot={library.libraryRoot}
            themeMode={themeMode}
            accentColor={accentColor}
            deleting={deleting}
            cleanupMessage={cleanupMessage}
            onChooseRoot={() => void chooseLibraryRoot()}
            onThemeChange={changeThemeMode}
            onAccentColorChange={changeAccentColor}
            onDelete={requestDeleteAllDownloads}
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

        {deleteConfirmationOpen && library.libraryRoot && (
          <DeleteConfirmationDialog
            libraryRoot={library.libraryRoot}
            onCancel={() => setDeleteConfirmationOpen(false)}
            onConfirm={() => void confirmDeleteAllDownloads()}
          />
        )}
      </main>
    </div>
  );
}

export function BrowserToolbar({ onNavigate }: {
  onNavigate: (action: BrowserNavigationAction) => void;
}) {
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
          Booth Shelfが記録しているファイルと展開フォルダをすべて削除します。この操作は元に戻せません。管理対象外のファイルは削除しません。
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

function SettingsView({ libraryRoot, themeMode, accentColor, deleting, cleanupMessage, onChooseRoot, onThemeChange, onAccentColorChange, onDelete }: {
  libraryRoot: string | null;
  themeMode: ThemeMode;
  accentColor: string;
  deleting: boolean;
  cleanupMessage: string | null;
  onChooseRoot: () => void;
  onThemeChange: (mode: ThemeMode) => void;
  onAccentColorChange: (color: string) => void;
  onDelete: () => void;
}) {
  return <section className="settings-layout" aria-label="設定">
    <article className="settings-card">
      <div className="settings-icon"><Moon size={22} /></div>
      <div className="settings-body">
        <h2>外観</h2>
        <p>Booth Shelfの表示テーマを選択します。BOOTHサイトには適用されません。</p>
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
        <button className="secondary-button" onClick={onChooseRoot}>
          <FolderOpen size={17} />{libraryRoot ? "保存先を変更" : "保存先を選択"}
        </button>
      </div>
    </article>

    <article className="settings-card danger-card">
      <div className="settings-icon danger-icon"><Trash2 size={22} /></div>
      <div className="settings-body">
        <h2>データの削除</h2>
        <p>Booth ShelfがDBに記録したダウンロード済みファイル・展開フォルダとライブラリ登録を削除します。保存先そのものや、管理対象外のファイルは残します。</p>
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

export function ProductCard({ product, onError, onOpenProduct }: {
  product: Product;
  onError: (error: string) => void;
  onOpenProduct: (url: string) => void;
}) {
  const openFolder = () => invoke("open_product_folder", { itemId: product.itemId }).catch((reason) => onError(errorText(reason)));
  return <article className="product-card" onContextMenu={(event) => { if (product.localPath) { event.preventDefault(); void openFolder(); } }}>
    <div className="thumbnail" title={product.localPath ? "右クリックで保存フォルダを開く" : undefined}>
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
