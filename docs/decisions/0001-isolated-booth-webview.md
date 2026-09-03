# ADR 0001: Isolate BOOTH in a capability-free WebView

## Status

Accepted

## Decision

BOOTH runs in a dedicated persistent Tauri WebView whose window label is excluded from every capability. The trusted local React window is the only window allowed to invoke custom commands, dialogs, or opener functionality.

The remote WebView may navigate only to HTTPS origins under `booth.pm` and `pixiv.net`. Top-level `booth-library-manager://` navigation is validated and cancelled before Windows can dispatch it to the globally registered official client. A completed download notification may navigate once to the internal folder action with a short-lived opaque UUID; no item ID or local path is exposed to the page. The global protocol registration is not changed.

## Consequences

- BOOTH authentication cookies remain in a dedicated local profile, are not copied into SQLite, and can be cleared separately from downloaded files and library metadata.
- Remote content cannot invoke filesystem or application commands.
- Popup windows are denied in the initial implementation. A BOOTH authentication change that requires popups will need a narrowly reviewed policy update.
- The custom scheme and Open Graph metadata are private integration surfaces and must fail closed when their observed format changes.
