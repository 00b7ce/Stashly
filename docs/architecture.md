# Architecture

## Goals

Stashly for BOOTH provides a human-readable local library, file actions, and an in-app BOOTH browsing flow without depending on BOOTH Library Manager internals.

## Trust boundaries

```text
Main native window
    |
    +-- Local React WebView `main` (app capabilities, persistent sidebar)
    |       |
    |       +-- typed Tauri commands
    |
    +-- Embedded BOOTH WebView `booth-browser` (no Tauri capabilities)
            |
            +-- allowlisted HTTPS navigation
            +-- ordinary BOOTH download handled by WebView2

Rust core -- SQLite index -- managed library root
    |
    +-- download worker (staging, SHA-256, atomic finalize)
```

The remote child WebView is positioned below the local browser toolbar and beside the persistent, collapsible local sidebar. Its native bounds are refreshed after sidebar state changes as well as native-window resizes because CSS layout alone cannot clip or reposition a child WebView. The toolbar continuously shows a non-editable URL so the active BOOTH or pixiv host and path remain visible even where library-only site chrome is hidden. The URL surface is not a text input: primary activation copies the displayed value and shows a short confirmation, while text selection, dragging, and the context menu are disabled. Rust removes fragments, credentials, and query parameters before emitting this display URL to `main`; only a numeric BOOTH library `page` parameter is retained. Back, forward, and reload requests originate in the trusted `main` WebView and are reduced to a closed Rust enum before the matching fixed script is evaluated in `booth-browser`. New-window requests are denied as separate windows; only allowlisted BOOTH or pixiv HTTPS targets are instead navigated in the existing embedded WebView. Exact product links on the account library are converted to same-WebView navigation before BOOTH can create a new tab. The trusted local UI selects the BOOTH or BOOTH-library sidebar destination from the sanitized loaded URL, while pixiv authentication pages retain the originating selection. Rust remembers the last successfully loaded BOOTH page and BOOTH library page separately in memory, so switching between their sidebar entries restores each browsing position without persisting it across application restarts. Explicit product links still take precedence over the remembered BOOTH page. The local application theme and accent color are persisted in the trusted WebView's local storage and are not propagated into the remote WebView: BOOTH keeps its official styling so site changes cannot break app-maintained presentation overrides. The remote child is never allowed to invoke application commands. Capabilities target the `main` WebView label instead of the containing window, so they are not inherited by `booth-browser`. Strict navigation and native-download callbacks are the only bridges from the remote page into the Rust core. Rust sends only an opaque random request ID and a closed download state to the remote page; filenames, BOOTH item IDs, local paths, and internal error messages are excluded. Rust also retains only active request IDs and their closed states in memory, then replays them after a remote page finishes loading so in-progress notifications survive navigation. A completed notification may request one narrowly defined `booth-shelf://open-product-folder?request_id=...` navigation. Rust accepts only a UUID currently present in a short-lived, one-time in-memory mapping, then looks up the product in the local database, canonicalizes the resulting path, and proves that it is inside the configured root before invoking Explorer. The remote page still receives no IPC or local capability.

## Download flow

1. On `accounts.booth.pm/library`, an origin-scoped initialization script hides the site chrome preceding the purchased/gift/free-download tabs, the footer following the library content, and every `その他のDL方法` control. The alternative-control hiding is independent of download admission. The script exposes no Tauri IPC or local capability.
2. When the user activates an ordinary `https://booth.pm/downloadables/...` target from the library's standard button or an exact `booth.pm`/`*.booth.pm` product page ending in `[/locale]/items/<numeric-id>`, the script validates the official host and exact path shape. The current product URL is parsed at activation time, so same-document navigation cannot reuse a previous product ID; that current ID takes precedence over library-only surrounding-card inference. The script sends only positive IDs plus an opaque UUID through a short-lived `booth-shelf://download-intent` navigation. A queryless standard URL uses its downloadable ID as the internal variation key; an explicit `variation_id` remains validated when present. Rust validates and arms the one-shot intent, then acknowledges it; only after that acknowledgement does the script navigate to the unchanged official download URL.
3. BOOTH performs the authenticated request in its own WebView. Rust's WebView2 download callback accepts only the matching one-shot intent and an exact BOOTH download endpoint or `s6.booth.pm` response, validates the suggested filename, and replaces the browser destination with an absolute UUID-named path inside the selected root's staging directory.
4. The local index is checked before the transfer. If the same item, variation, and filename still exists inside the active library root, the existing artifact is returned without another download.
5. Otherwise, up to two WebView downloads may be active. Additional requests and cleanup races are rejected before a destination is assigned.
6. Each completed staging file is checked for size limits and hashed. ZIP files are expanded in staging with traversal, link, entry-count, and expanded-size checks. A sole top-level directory is flattened only when its name matches the ZIP stem; the temporary ZIP is then removed before the extracted directory is atomically published.
7. Files and extracted directories are published to their exact destination. An existing unindexed destination causes a safe failure rather than a numbered `(2)` copy.
8. Public Open Graph metadata is fetched only for an exact HTTPS `booth.pm[/locale]/items/<numeric-id>` URL. SQLite keeps a 30-day successful-result cache, a 24-hour failed-attempt cooldown, and a global request timestamp that enforces a ten-second minimum interval across concurrent downloads and app restarts. A `429 Too Many Requests` response persists a global pause for the longer of `Retry-After` or 24 hours. Requests identify the application and public repository in their User-Agent, accept HTML only, follow at most three same-scope redirects, and stop reading after the HTML head or a 1 MiB limit. Cached metadata remains usable when a refresh fails; metadata failure never prevents the purchased file from being finalized. The active BOOTH page remains visible while a single updating status notification is shown in its lower-right corner. The remote notification uses fixed copy derived only from the closed state enum, so no local path or metadata is rendered there. A completed notification stays for six seconds, shows a decreasing expiry bar, and opens the validated saved folder through its one-time opaque request ID when clicked.

## Data cleanup flow

The Settings view can remove all artifacts indexed by Stashly for BOOTH. A themed in-app alert dialog names the configured library root immediately before execution, defaults focus to Cancel, and closes without action on Escape or a backdrop click. Cleanup and download admission are mutually exclusive, so a new download cannot begin during deletion and cleanup cannot begin while work is queued or active.

The cleanup command canonicalizes every existing artifact path and requires it to remain below the configured root. It removes only indexed artifact files/directories plus the app-owned staging directory, prunes empty parent directories, and clears the corresponding local index. The library root and unindexed sibling files are preserved.

Download intents, order IDs, cookies, and signed URLs are not logged or stored. The authenticated file body is transferred by the BOOTH WebView rather than re-requested by the Rust HTTP client.

The Settings view also provides a separately confirmed browser-data cleanup. If `booth-browser` exists, Rust asks that dedicated WebView profile to clear all browsing data. If it has not been created in the current process, Rust removes only the app-owned `booth-webview` profile directory. Downloaded artifacts and the Stashly for BOOTH SQLite library are outside this operation.

The app identifies Stashly for BOOTH as an unofficial BOOTH app in the persistent sidebar, including its collapsed state. Settings repeats the non-affiliation notice, obtains the displayed version from the packaged Tauri application metadata, and places the BOOTH/pixiv terms and privacy policy in a clearly separated official-information section. These links open in the system browser and are the only URLs allowed through the trusted WebView's opener capability. BOOTH support is deliberately not presented as the application's support contact.

## Storage model

The application owns a SQLite database under the operating system application-data directory. It records products, variations, downloadable files, and local artifacts. Remote credentials are owned by the WebView profile and never copied into the database. Canonical Windows paths may use the verbatim `\\?\` form internally, but the local UI removes that implementation prefix when displaying the configured root.

Local fixed drives are accepted as the normal storage mode. UNC paths, mapped network drives, recognized synchronization roots, removable or unknown drive types require an explicit warning acknowledgement. Rust canonicalizes and classifies the candidate, verifies create/write/sync/rename/delete behavior with a uniquely named probe, and stores a consent fingerprint bound to the canonical path and storage kind. The same classification, reachability, and fingerprint are checked again before each download. A legacy non-local root without matching consent fails closed until the user confirms it. Windows cannot identify every third-party synchronization tool or virtual filesystem, so the UI describes the classification as a warning boundary rather than a guarantee.

The default folder shape is:

```text
<library root>/
  <shop>/
    <product> [booth-<item id>]/
      <variation>/
        <original non-ZIP filename>
        <ZIP filename without extension>/
          <extracted contents>
```

Remote names are sanitized for Windows and the immutable BOOTH item ID disambiguates renamed or duplicate products.

## Compatibility strategy

The BOOTH library DOM and download-link shape are not public APIs. Parsing is origin-scoped and strict, and a mismatch fails closed instead of guessing identifiers; only the independent hiding of `その他のDL方法` remains active. The official `booth-library-manager://` handler and BOOTH Library Manager data remain outside the application boundary.
