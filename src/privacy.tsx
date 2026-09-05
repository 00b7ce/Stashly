import { useEffect, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";
import { CheckCircle2, ShieldCheck, X } from "lucide-react";
import Markdown from "react-markdown";
import privacyDocument from "../PRIVACY.md?raw";

export const PRIVACY_VERSION = "2026-09-05";
export const PRIVACY_CONSENT_STORAGE_KEY = "stashly-privacy-consent-version";
export const PRIVACY_SUPPORT_URL = "https://github.com/00b7ce/Stashly/issues";

function hasCurrentConsent(): boolean {
  try {
    return window.localStorage.getItem(PRIVACY_CONSENT_STORAGE_KEY) === PRIVACY_VERSION;
  } catch {
    return false;
  }
}

function PrivacyDocument() {
  const [linkError, setLinkError] = useState<string | null>(null);

  function openDocumentLink(href: string | undefined) {
    if (href !== PRIVACY_SUPPORT_URL) return;
    setLinkError(null);
    void openUrl(href).catch((reason) => {
      const detail = reason instanceof Error ? reason.message : String(reason);
      setLinkError(`リンクを開けませんでした: ${detail}`);
    });
  }

  return <div className="privacy-document" tabIndex={0}>
    <Markdown
      skipHtml
      components={{
        h1: ({ children }) => <h3>{children}</h3>,
        h2: ({ children }) => <h4>{children}</h4>,
        h3: ({ children }) => <h5>{children}</h5>,
        a: ({ href, children }) => href === PRIVACY_SUPPORT_URL
          ? <a href={href} onClick={(event) => { event.preventDefault(); openDocumentLink(href); }}>{children}</a>
          : <span>{children}</span>,
      }}
    >
      {privacyDocument}
    </Markdown>
    {linkError && <p className="privacy-error" role="alert">{linkError}</p>}
  </div>;
}

export function PrivacyGate({ children }: { children: ReactNode }) {
  const [consented, setConsented] = useState(hasCurrentConsent);
  const [acknowledged, setAcknowledged] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function accept() {
    if (!acknowledged) return;
    try {
      window.localStorage.setItem(PRIVACY_CONSENT_STORAGE_KEY, PRIVACY_VERSION);
      if (!hasCurrentConsent()) throw new Error("保存内容を確認できませんでした");
      setConsented(true);
    } catch (reason) {
      const detail = reason instanceof Error ? reason.message : String(reason);
      setError(`同意内容を保存できませんでした: ${detail}`);
    }
  }

  async function decline() {
    setError(null);
    try {
      await getCurrentWindow().close();
    } catch {
      setError("アプリを終了できませんでした。ウィンドウの閉じるボタンから終了してください。");
    }
  }

  if (consented) return children;

  return <main className="privacy-gate">
    <section className="privacy-gate-card" aria-labelledby="privacy-gate-title">
      <header className="privacy-heading">
        <div className="privacy-heading-icon"><ShieldCheck size={28} /></div>
        <div>
          <p className="eyebrow">PRIVACY</p>
          <h1 id="privacy-gate-title">プライバシーポリシー</h1>
          <p>Stashlyを利用する前に、以下の内容を確認してください。</p>
        </div>
      </header>
      <PrivacyDocument />
      {error && <p className="privacy-error" role="alert">{error}</p>}
      <label className="privacy-acknowledgement">
        <input
          type="checkbox"
          checked={acknowledged}
          onChange={(event) => setAcknowledged(event.currentTarget.checked)}
        />
        <span>内容を読み、同意します</span>
      </label>
      <div className="privacy-actions">
        <button className="modal-cancel" type="button" onClick={() => void decline()}>同意しないで終了</button>
        <button className="privacy-accept" type="button" disabled={!acknowledged} onClick={accept}>
          <CheckCircle2 size={17} />同意して始める
        </button>
      </div>
    </section>
  </main>;
}

export function PrivacyPolicyDialog({ onClose }: { onClose: () => void }) {
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeRef.current?.focus();
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  return <div
    className="modal-backdrop privacy-modal-backdrop"
    onMouseDown={(event) => event.target === event.currentTarget && onClose()}
  >
    <section className="privacy-dialog" role="dialog" aria-modal="true" aria-labelledby="privacy-dialog-title">
      <header className="privacy-dialog-header">
        <div>
          <p className="eyebrow">STASHLY PRIVACY</p>
          <h2 id="privacy-dialog-title">プライバシーポリシー</h2>
        </div>
        <button ref={closeRef} className="privacy-close" type="button" onClick={onClose} aria-label="閉じる">
          <X size={20} />
        </button>
      </header>
      <PrivacyDocument />
      <footer className="privacy-dialog-footer">
        <span>文書バージョン: {PRIVACY_VERSION}</span>
        <button className="secondary-button" type="button" onClick={onClose}>閉じる</button>
      </footer>
    </section>
  </div>;
}
