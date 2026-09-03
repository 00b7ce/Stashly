# Development

## Requirements

- Windows 10 or later
- Microsoft Edge WebView2 Runtime
- Rust MSVC toolchain
- Node.js and npm
- Visual Studio Build Tools with the Desktop development with C++ workload

## Setup

```powershell
npm.cmd install
npm.cmd run tauri -- dev
```

The first launch creates only application-owned state. Choose a new library root; do not select a BOOTH Library Manager directory for destructive testing.

## Verification

```powershell
npm.cmd run test
npm.cmd run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

For a manual deeplink test, use a free item already present in the user's account. Stop before any purchase or account change. Confirm that:

- the navigation is intercepted inside the dedicated WebView;
- BOOTH opens inside the main native window beside the persistent sidebar, without creating another top-level window;
- the BOOTH toolbar stays above the embedded page and its back, forward, and reload buttons affect only the embedded browser;
- collapsing the sidebar leaves recognizable menu icons and expands the embedded page, while expanding it restores the labels;
- Settings remains directly above the sidebar collapse control in both expanded and collapsed layouts;
- system, light, and dark theme choices update the local UI immediately and persist after restart;
- preset and custom accent colors update local controls immediately, retain readable foreground contrast, and persist after restart;
- BOOTH retains its official site styling regardless of the selected local application theme;
- switching between local library, BOOTH top, BOOTH library, and Settings preserves the expected page and focus;
- the local library heading, search field, view selector, and refresh action share one row; all three grid sizes and the horizontal list layout remain usable at the minimum window width;
- the `booth-browser` child WebView cannot invoke local commands, dialogs, opener APIs, or filesystem functionality;
- the normal BOOTH library download button starts the Booth Shelf flow and its paired alternative-download control is hidden;
- a DOM mismatch restores the original alternative-download control instead of starting an unmanaged browser download;
- no raw deeplink or signed URL appears in logs or SQLite;
- the file lands under the selected test root;
- the completed notification contains no filename, product ID, local path, or internal error text; its expiry bar decreases from right to left over six seconds, and clicking it still opens the downloaded product folder through an opaque one-time request ID;
- a ZIP lands as an extracted directory and the original ZIP is absent;
- traversal entries, links, more than 20,000 entries, and more than 16 GiB of expanded data are rejected without a partial final directory;
- the global `booth-library-manager://` handler still points to the official application;
- the existing BOOTH Library Manager database and download root are unchanged.
- Settings shows the configured root once, and changing it updates subsequent downloads;
- the delete action opens a themed in-app confirmation, initially focuses Cancel, closes on Escape, and does not delete until the destructive button is explicitly chosen;
- Settings displays a normal drive or UNC path without the Windows verbatim `\\?\` prefix;
- cancelling the cleanup confirmation removes nothing;
- confirmed cleanup removes indexed artifacts and empties the library view, while an unrelated sentinel file directly under the test root remains;
- cleanup is rejected while a download is queued or active.
- the BOOTH browser-data action opens a themed in-app confirmation, defaults focus to Cancel, and closes on Escape or a backdrop click;
- cancelling browser-data cleanup preserves the BOOTH login, while confirming it removes the dedicated profile's cookies, cache, history, and local storage and requires BOOTH/pixiv login again;
- browser-data cleanup leaves downloaded artifacts, the Booth Shelf SQLite library, and local UI preferences unchanged.

## Packaging

```powershell
npm.cmd run tauri -- build --bundles nsis
```

Do not claim the installer is signed unless its Authenticode signature has been verified.
