# ADR 0001: Isolate BOOTH in a capability-free WebView

## Status

Accepted

## Decision

BOOTH runs in a dedicated persistent Tauri WebView whose window label is excluded from every capability. The trusted local React window is the only window allowed to invoke custom commands, dialogs, or opener functionality.

The remote WebView may navigate only to HTTPS origins under `booth.pm` and `pixiv.net`. A request to open an allowlisted target in a new window is redirected into that same isolated WebView; no additional WebView is created. A standard BOOTH download from the account library or an exact BOOTH product page is armed by one strict, short-lived `stashly://download-intent` navigation and transferred by the WebView into app-owned staging; the remote page receives no command capability or local path. A completed download notification may navigate once to the internal folder action with a short-lived opaque UUID; no item ID or local path is exposed to the page. The global `booth-library-manager://` protocol registration is not read or changed.

## Consequences

- BOOTH authentication cookies remain in a dedicated local profile, are not copied into SQLite, and can be cleared separately from downloaded files and library metadata.
- Remote content cannot invoke filesystem or application commands.
- Popup windows remain denied. Allowlisted BOOTH and pixiv links that request a new window reuse the existing isolated WebView, while all other popup targets remain blocked.
- The custom scheme and Open Graph metadata are private integration surfaces and must fail closed when their observed format changes.
