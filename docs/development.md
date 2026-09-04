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

Debug builds may set the task-specific `STASHLY_DEV_DATA_DIR` environment variable to an absolute temporary directory. Only the app SQLite database is redirected; use it for isolated test data and remove the temporary directory after capture. Release builds ignore this variable.

## Verification

```powershell
npm.cmd run test
npm.cmd run build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

For a manual download test, use a free item already present in the user's account. Stop before any purchase or account change. Confirm that:

- the ordinary BOOTH download stays inside the dedicated WebView download flow and no copy appears in the system Downloads folder;
- BOOTH opens inside the main native window beside the persistent sidebar, without creating another top-level window;
- expanding and collapsing the sidebar immediately repositions and resizes the embedded BOOTH page without overlap;
- links that BOOTH marks to open in a new tab, including followed-shop updates and free-library item pages, open in the existing embedded WebView; a non-BOOTH/pixiv popup remains blocked;
- a product opened from the free-download library selects the BOOTH sidebar destination, while returning to BOOTH library restores the previous free-download page;
- the BOOTH toolbar stays above the embedded page, its back, forward, and reload buttons affect only the embedded browser, and its non-editable URL follows navigation;
- primary-clicking the displayed URL copies the visible sanitized value and shows the confirmation popup; right-clicking opens no context menu, and the address cannot be edited, selected, or dragged;
- authentication parameters, fragments, and non-page library query parameters are absent from the displayed and copied URL;
- collapsing the sidebar leaves recognizable menu icons and expands the embedded page, while expanding it restores the labels;
- both expanded and collapsed sidebars keep an explicit `BOOTH非公式アプリ`/`非公式` marker visible beside the neutral Stashly branding;
- Settings remains directly above the sidebar collapse control in both expanded and collapsed layouts;
- system, light, and dark theme choices update the local UI immediately and persist after restart;
- preset and custom accent colors update local controls immediately, retain readable foreground contrast, and persist after restart;
- BOOTH retains its official site styling regardless of the selected local application theme;
- switching between local library, BOOTH, BOOTH library, and Settings preserves the expected page and focus, including each BOOTH destination's last loaded page during the current application session;
- the local library heading, search field, view selector, and refresh action share one row; all three grid sizes and the horizontal list layout remain usable at the minimum window width;
- the `booth-browser` child WebView cannot invoke local commands, dialogs, opener APIs, or filesystem functionality;
- the normal BOOTH library download button starts the Stashly for BOOTH flow and its paired alternative-download control is hidden;
- an exact BOOTH product page's free-download link starts the same Stashly for BOOTH flow;
- navigating between product pages without a full document reload binds the download to the currently displayed product ID and folder, never a previously viewed product;
- a DOM mismatch keeps the alternative-download control hidden but refuses to arm the normal download instead of guessing product or download IDs;
- no download intent or signed URL appears in logs or SQLite;
- the file lands under the selected test root;
- the completed notification contains no filename, product ID, local path, or internal error text; its expiry bar decreases from right to left over six seconds, and clicking it still opens the downloaded product folder through an opaque one-time request ID;
- navigating, going back or forward, and reloading while a download is active restores one updating notification for each in-progress request;
- a ZIP lands as an extracted directory and the original ZIP is absent;
- traversal entries, links, more than 20,000 entries, and more than 16 GiB of expanded data are rejected without a partial final directory;
- the global `booth-library-manager://` handler still points to the official application;
- the existing BOOTH Library Manager database and download root are unchanged.
- a local fixed-drive root is accepted without an external-storage warning;
- UNC, mapped-network, recognized sync, removable, and unknown roots show the exact candidate path and require the acknowledgement checkbox before they can be saved;
- cancelling the external-storage confirmation preserves the previous root, while confirming it records consent only for that canonical root and storage kind;
- the candidate probe verifies create, write, flush, rename, and delete support without leaving a test file behind;
- a disconnected or changed non-local root fails closed before a download, and legacy non-local roots require confirmation after upgrade;
- changing to a different root is rejected while downloads/cleanup are active or while indexed artifacts would be orphaned;
- a second download of the same product within 30 days reuses SQLite Open Graph metadata without another product-page request;
- failed metadata requests are not retried for 24 hours, requests for different products remain at least ten seconds apart, and a simulated `429` persists a global pause across database reopen;
- metadata requests use the documented Stashly for BOOTH User-Agent and reject redirects outside exact public BOOTH product URLs, non-HTML responses, oversized HTML heads, and non-BOOTH image hosts;
- metadata refresh failure does not prevent the downloaded artifact from being finalized;
- Settings shows the packaged application version and an explicit nonofficial notice; the terms and privacy links open the expected official pages in the system browser under a separate official-information heading, with no BOOTH support link presented as the app's support contact;
- the delete action opens a themed in-app confirmation, initially focuses Cancel, closes on Escape, and does not delete until the destructive button is explicitly chosen;
- Settings displays a normal drive or UNC path without the Windows verbatim `\\?\` prefix;
- cancelling the cleanup confirmation removes nothing;
- confirmed cleanup removes indexed artifacts and empties the library view, while an unrelated sentinel file directly under the test root remains;
- cleanup is rejected while a download is active.
- the BOOTH browser-data action opens a themed in-app confirmation, defaults focus to Cancel, and closes on Escape or a backdrop click;
- cancelling browser-data cleanup preserves the BOOTH login, while confirming it removes the dedicated profile's cookies, cache, history, and local storage and requires BOOTH/pixiv login again;
- browser-data cleanup leaves downloaded artifacts, the Stashly for BOOTH SQLite library, and local UI preferences unchanged.

## Packaging

```powershell
npm.cmd run tauri -- build --bundles nsis
```

The tag-triggered release workflow adds a GitHub-linked list of commits since the previous reachable `v*` tag to the draft Release body, together with a compare link. If no previous release tag exists, it lists commits from the beginning of the repository instead.

Do not claim the installer is signed unless its Authenticode signature has been verified.
