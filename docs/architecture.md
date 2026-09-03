# Architecture

## Goals

Booth Shelf provides a human-readable local library, file actions, and an in-app BOOTH browsing flow without depending on BOOTH Library Manager internals.

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
            +-- BLM deeplink intercepted in Rust before OS dispatch

Rust core -- SQLite index -- managed library root
    |
    +-- download worker (staging, SHA-256, atomic finalize)
```

The remote child WebView is positioned below the local browser toolbar and beside the persistent, collapsible local sidebar. Back, forward, and reload requests originate in the trusted `main` WebView and are reduced to a closed Rust enum before the matching fixed script is evaluated in `booth-browser`. The local application theme and accent color are persisted in the trusted WebView's local storage and are not propagated into the remote WebView: BOOTH keeps its official styling so site changes cannot break app-maintained presentation overrides. The remote child is never allowed to invoke application commands. Capabilities target the `main` WebView label instead of the containing window, so they are not inherited by `booth-browser`. Navigation callbacks are the only bridge from the remote page into the Rust core. Rust sends only an opaque random request ID and a closed download state to the remote page; filenames, BOOTH item IDs, local paths, and internal error messages are excluded. A completed notification may request one narrowly defined `booth-shelf://open-product-folder?request_id=...` navigation. Rust accepts only a UUID currently present in a short-lived, one-time in-memory mapping, then looks up the product in the local database, canonicalizes the resulting path, and proves that it is inside the configured root before invoking Explorer. The remote page still receives no IPC or local capability.

## Download flow

1. On `accounts.booth.pm/library`, an origin-scoped initialization script hides the site chrome preceding the purchased/gift/free-download tabs and the footer following the library content, pairs each standard download button with its existing BLM action, hides the redundant alternative-download control, and routes the standard click to that action. It exposes no Tauri IPC or local capability. If pairing fails, the original alternative control is restored with an error instead of falling back to an unmanaged browser download.
2. BOOTH returns a `booth-library-manager://` navigation.
3. Rust validates the scheme, host, identifiers, file count, and download hosts.
4. The known BLM introduction fallback is cancelled only during the short period immediately following a captured deeplink.
5. The payload is reduced to non-secret metadata; signed URLs remain memory-only.
6. The local index is checked before network access. If the same item, variation, and filename still exists inside the active library root, the existing artifact is returned without another download.
7. Otherwise, up to two files begin immediately and download into a staging directory inside the selected root. Additional requests are rejected with a retry prompt instead of retaining an expiring signed URL.
8. Each file is checked for size limits and hashed. ZIP files are expanded in staging with traversal, link, entry-count, and expanded-size checks. A sole top-level directory is flattened only when its name matches the ZIP stem; the temporary ZIP is then removed before the extracted directory is atomically published.
9. Files and extracted directories are published to their exact destination. An existing unindexed destination causes a safe failure rather than a numbered `(2)` copy.
10. Public Open Graph metadata is fetched once when available, then SQLite is updated after the file is finalized. The active BOOTH page remains visible while a single updating status notification is shown in its lower-right corner. The remote notification uses fixed copy derived only from the closed state enum, so no local path or metadata is rendered there. A completed notification stays for six seconds, shows a decreasing expiry bar, and opens the validated saved folder through its one-time opaque request ID when clicked.

## Data cleanup flow

The Settings view can remove all artifacts indexed by Booth Shelf. A themed in-app alert dialog names the configured library root immediately before execution, defaults focus to Cancel, and closes without action on Escape or a backdrop click. Cleanup and download admission are mutually exclusive, so a new download cannot begin during deletion and cleanup cannot begin while work is queued or active.

The cleanup command canonicalizes every existing artifact path and requires it to remain below the configured root. It removes only indexed artifact files/directories plus the app-owned staging directory, prunes empty parent directories, and clears the corresponding local index. The library root and unindexed sibling files are preserved.

Raw deeplinks, order IDs, cookies, and signed URLs are not logged or stored.

The Settings view also provides a separately confirmed browser-data cleanup. If `booth-browser` exists, Rust asks that dedicated WebView profile to clear all browsing data. If it has not been created in the current process, Rust removes only the app-owned `booth-webview` profile directory. Downloaded artifacts and the Booth Shelf SQLite library are outside this operation.

## Storage model

The application owns a SQLite database under the operating system application-data directory. It records products, variations, downloadable files, and local artifacts. Remote credentials are owned by the WebView profile and never copied into the database. Canonical Windows paths may use the verbatim `\\?\` form internally, but the local UI removes that implementation prefix when displaying the configured root.

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

The BLM deeplink and BOOTH page metadata are private interfaces. Parsing is isolated, fixtures cover known payload shapes, and a mismatch fails closed with an actionable error instead of launching the official handler or guessing fields. Normal browser downloads are not intercepted in the initial release.
