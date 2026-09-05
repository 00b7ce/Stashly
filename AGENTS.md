# Stashly project instructions

## Purpose

Stashly is an unofficial Windows desktop library manager for files downloaded from BOOTH. It uses a dedicated, unprivileged BOOTH WebView and stores library metadata in its own SQLite database. It must not modify BOOTH Library Manager files, its SQLite database, or the global `booth-library-manager://` protocol registration.

## Architecture

- `src/`: React and TypeScript local management UI.
- `src-tauri/src/`: Rust application core, WebView policy, native WebView downloads, SQLite storage, and post-processing.
- `PRIVACY.md`: bundled privacy policy shown by the first-run consent gate and Settings dialog.
- `docs/architecture.md`: trust boundaries and data flow.
- `docs/development.md`: setup and verification commands.

The local `main` WebView may invoke explicitly exposed Tauri commands. The remote `booth-browser` WebView must never receive local filesystem, shell, opener, dialog, or custom-command capabilities.

The main React application must not mount before the user has accepted the exact current `PRIVACY.md` document version. A policy version change must require consent again, and Settings must render the same bundled Markdown document rather than a duplicate copy.

## Development commands

- `npm.cmd install`: install JavaScript dependencies on Windows.
- `npm.cmd run dev`: run the Vite frontend.
- `npm.cmd run build`: TypeScript check and production frontend build.
- `npm.cmd run test`: run frontend unit tests.
- `npm.cmd run tauri -- dev`: run the desktop application.
- `npm.cmd run tauri -- build --bundles nsis`: build the Windows installer.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check`: verify Rust formatting.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`: run Rust linting.
- `cargo test --manifest-path src-tauri/Cargo.toml`: run Rust tests.

## Safety constraints

- Treat all remote page content, download intents, and filenames as untrusted.
- Never log or persist signed download URLs, session cookies, CSRF tokens, order IDs, or raw download-intent payloads.
- Permit remote navigation only to reviewed BOOTH and pixiv authentication hosts.
- Downloads must enter a staging directory, be hashed, and be atomically moved into a path proven to be inside the configured library root.
- Sanitize every remote filename and reject traversal, device names, alternate data streams, and absolute paths.
- ZIP extraction must occur only in the application staging directory, reject traversal and links, enforce entry and expanded-size limits, and publish the extracted directory atomically. Never execute extracted files.
- Deletion is out of scope until a Recycle Bin workflow with explicit confirmation exists.
- Accept native downloads only after a validated, short-lived intent from the BOOTH library; fail closed if the DOM or ordinary download-link shape changes.

## Git and verification

- Preserve unrelated user changes and do not commit unless explicitly requested.
- Do not commit generated build output, databases, browser profiles, downloaded products, or secrets.
- Update architecture/development documentation when behavior or commands change.
- Before completion, run frontend tests/build plus Rust fmt, clippy, and tests. For browser work, verify that the official protocol handler remains unchanged.
