# ADR 0001: Isolate BOOTH in a capability-free WebView

## Status

Accepted

## Decision

BOOTH runs in a dedicated persistent Tauri WebView whose window label is excluded from every capability. The trusted local React window is the only window allowed to invoke custom commands, dialogs, or opener functionality.

The remote WebView may navigate only to HTTPS origins under `booth.pm` and `pixiv.net`. Top-level `booth-library-manager://` navigation is validated and cancelled before Windows can dispatch it to the globally registered official client. The global protocol registration is not changed.

## Consequences

- BOOTH authentication cookies remain in a dedicated local profile and are not copied into SQLite.
- Remote content cannot invoke filesystem or application commands.
- Popup windows are denied in the initial implementation. A BOOTH authentication change that requires popups will need a narrowly reviewed policy update.
- The custom scheme and Open Graph metadata are private integration surfaces and must fail closed when their observed format changes.
