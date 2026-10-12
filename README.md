# StuCent Photo Editor

A **modified version of [PhotoCraft](https://github.com/storytold/photocraft)** (v0.7.0), the
open-source image editor written in Rust, adapted to run inside [StuCent](https://github.com/hafezparast/stucent_v5)
as the "photo editor" question type for students aged 14–18.

- Upstream: <https://github.com/storytold/photocraft> — this fork is based on tag **v0.7.0**. The
  upstream README is kept as [README.upstream.md](README.upstream.md).
- License: unchanged, **MIT OR Apache-2.0** ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)).
  [NOTICE](NOTICE) lists the modifications. Not endorsed by the PhotoCraft or ArtCraft teams.
- Releases: tags `v<upstream>-stucent.<n>` (first: `v0.7.0-stucent.1`), each with the web bundle
  `stucent-photo-editor-web-<tag>.zip` and `SHA256SUMS.txt`.

## What is different from PhotoCraft

Only the web app is built and shipped. Upstream code is untouched except for small, feature-gated hooks.

| Change | Where | Feature |
|---|---|---|
| Name "StuCent Photo Editor" in every UI language, neutral title-bar mark, no Discord / website / GitHub / ArtCraft links, About names the upstream version | `crates/ui-egui/src/brand.rs`, `links.rs`, `i18n/` | `photocraft-ui-egui/stucent-brand` (web: `stucent`) |
| Host bridge: a page that embeds the editor in an `<iframe>` drives it with `postMessage` | `apps/photocraft-web/src/host_bridge.rs`, `bridge_protocol.rs` | `host-bridge` |
| Menu items locked by the host show disabled and are refused | `Services::locked_command`, `menus::locked` | always compiled, used by the bridge |
| WebKit downloaded the wasm twice (it does not hand trunk's preload to the loader's fetch) | `scripts/build-stucent-web.sh` replaces the preload with one early `fetch()` given to `init()` | — |
| UI languages Malay (`ms`), Swahili (`sw`), Turkish (`tr`), Vietnamese (`vi`) | `crates/ui-egui/src/i18n/{ms,sw,tr,vi}.tsv` | — |

**Not yet supported:** Arabic, Persian and Tamil. The UI has no right-to-left layout (egui has no
bidi yet, emilk/egui#8576) and the web build ships no Arabic or Tamil UI fonts. Screen readers get
nothing on the web (eframe 0.36 drops AccessKit there).

## Host bridge (protocol `emu/1`)

The bridge is on only when the page URL names the host: `index.html?host=https://school.example`.
Messages from any other origin, or from a window other than the parent, are ignored; replies go to
that origin only. Optional parameters: `exam=1` (start locked) and `lang=<code>` (UI language;
English when there is no catalog for it).

Every message is `{bridge: "emu/1", type, …}`.

| Host → editor | Editor → host |
|---|---|
| `hello` (or `ping`) | `ready {protocol, app, version, upstream, exam, language, capabilities}` (also sent once at start) |
| `open {name, bytes, requestId?}` — a PSD or image, as `ArrayBuffer` or `Uint8Array` | `opened {requestId?, name, ok, warnings?, error?}` |
| `export {format: "psd" \| "png", requestId}` — `png` is the flattened image | `exported {requestId, format, ok, bytes?, warnings?, error?}` (`bytes` transferred) |
| `exam {on}` | `exam {on}` |
| `autosave {intervalMs}` — 0 turns it off; clamped to 5 s – 10 min | `autosave {intervalMs}`, then `snapshot {seq, format: "psd", ok, bytes?, error?, commandCount}` in each interval in which something changed |
| | `commands {ids, params}` — the edit log: every engine command since the last frame (params JSON, cut to 512 characters), plus `bridge.open {name, size}` for each document the host opened |

**Exam mode** locks everything that brings files in or takes the work out other than through the
bridge: Open, Open Recent, Place, Import, Automate, Scripts that read files, Revert, Recover, Save,
Save As, Save a Copy, Export, Quick Export, Package, Print, Generate, preset import/export. Locked
menu items show disabled; shortcuts and the command palette are refused too; the file picker and
downloads are blocked as a backstop. Dropped files and text pasted from outside are dropped. The
editor's own clipboard (copy a layer or selection, paste it) keeps working: on the web it never
reads the system clipboard for images.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk --locked          # 0.21.x
scripts/build-stucent-web.sh          # → dist/stucent/stucent-photo-editor-web-<tag>.zip
```

The script checks the tag against `VERSION` in `apps/photocraft-web/src/bridge_protocol.rs`, builds
with trunk (binaryen `version_133` by default, `TRUNK_TOOLS_WASM_OPT` to change), enforces the
24 MiB wasm gate, replaces the wasm preload with a single early fetch (one download in WebKit too), and writes `manifest.json` (file
sizes and SHA-256) into the zip. A cold build takes about 30 minutes (fat LTO).

CI: `.github/workflows/stucent-web.yml` runs the bridge and branding tests and the same script on
every `v*-stucent.*` tag, and attaches the zip and `SHA256SUMS.txt` to the release.

Tests: `cargo test -p photocraft-web --features stucent` (bridge rules) and
`cargo test -p photocraft-ui-egui --features stucent-brand --lib` (branding, lock, catalogs).

## Hosting notes

Serve the zip's files from one directory: `application/wasm` for `.wasm`, Brotli or gzip, the hashed
`.js`/`.wasm` as `Cache-Control: public, max-age=31536000, immutable`, `index.html` as `no-cache`.
No COOP/COEP needed. WebGPU needs HTTPS (or localhost); otherwise WebGL2 is used. Without either
the editor shows "A browser with WebGPU or WebGL2 is required". The iframe needs
`allow="clipboard-read; clipboard-write"` for copy and paste.
