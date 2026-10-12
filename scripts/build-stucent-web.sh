#!/usr/bin/env bash
# Build the StuCent Photo Editor web bundle (the browser app with the `emu/1` host bridge and the
# StuCent branding) and zip it for StuCent to host:
#
#   dist/stucent/stucent-photo-editor-web-<tag>.zip   index.html, the hashed .js and _bg.wasm, manifest.json
#   dist/stucent/SHA256SUMS.txt
#
# Usage: scripts/build-stucent-web.sh [<tag>] [--skip-build]
#   <tag>  defaults to the VERSION in apps/photocraft-web/src/bridge_protocol.rs and must match it
#          (the app reports it in the `ready` handshake).
#
# Needs: the wasm32-unknown-unknown target, trunk 0.21 (cargo install trunk --locked), the
# wasm-bindgen CLI matching Cargo.lock (trunk fetches it), python3, zip, and sha256sum or shasum.
# wasm-opt: trunk downloads binaryen; pick the version with TRUNK_TOOLS_WASM_OPT (default
# version_133 here, newer than upstream's default: it gives a smaller wasm).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/apps/photocraft-web"
SITE="$ROOT/dist/web"
OUT="$ROOT/dist/stucent"
MAX_WASM_BYTES="${PHOTOCRAFT_WASM_MAX_BYTES:-25165824}" # the project's 24 MiB gate (packaging/web/package.sh)

CODE_VERSION="$(sed -n 's/^pub const VERSION: &str = "\(.*\)";$/\1/p' "$APP/src/bridge_protocol.rs")"
TAG="$CODE_VERSION"
SKIP_BUILD=0
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    -*) echo "error: unknown option $arg" >&2; exit 2 ;;
    *) TAG="$arg" ;;
  esac
done
if [ "$TAG" != "$CODE_VERSION" ]; then
  echo "error: tag $TAG does not match VERSION $CODE_VERSION in apps/photocraft-web/src/bridge_protocol.rs" >&2
  exit 1
fi

if [ "$SKIP_BUILD" = 0 ]; then
  command -v trunk >/dev/null || { echo "error: trunk not found (cargo install trunk --locked)" >&2; exit 1; }
  export TRUNK_TOOLS_WASM_OPT="${TRUNK_TOOLS_WASM_OPT:-version_133}"
  (cd "$APP" && trunk build --release)
fi
[ -f "$SITE/index.html" ] || { echo "error: $SITE/index.html missing; run without --skip-build" >&2; exit 1; }
grep -q 'host-bridge' "$APP/index.html" || { echo "error: index.html does not build the host-bridge feature" >&2; exit 1; }
if grep -Eq '(src|href)="/[^/]' "$SITE/index.html"; then
  echo "error: $SITE/index.html has root-absolute URLs; it would break when served from a sub-path" >&2
  exit 1
fi
# One wasm download in every browser. WebKit does not hand trunk's `<link rel="preload" as="fetch">`
# to the loader's own fetch() (with or without an integrity value), so it downloaded the 25 MB file
# twice. Instead: drop the preload, start the one fetch() first thing in <head>, and give that
# Response promise to wasm-bindgen's init(), which accepts one.
python3 - "$SITE/index.html" <<'PY'
import re, sys
path = sys.argv[1]
html = open(path, encoding="utf-8").read()
if "__stucentWasm" in html:
    sys.exit(0)  # already rewritten (a --skip-build re-run)
m = re.search(r"init\(\{ module_or_path: '(\./[^']+_bg\.wasm)' \}\)", html)
if not m:
    sys.exit("error: trunk's loader call init({ module_or_path: '…_bg.wasm' }) not found in index.html")
wasm = m.group(1)
html = html.replace(m.group(0), "init({ module_or_path: window.__stucentWasm || '%s' })" % wasm)
html, n = re.subn(r'<link rel="preload" href="%s"[^>]*>' % re.escape(wasm), "", html)
if n != 1:
    sys.exit("error: expected one wasm preload link, found %d" % n)
early = "<script>window.__stucentWasm = fetch('%s');</script>" % wasm
html = html.replace('<meta charset="utf-8">', '<meta charset="utf-8">\n  ' + early, 1)
if early not in html:
    sys.exit("error: could not place the early wasm fetch")
open(path, "w", encoding="utf-8").write(html)
PY
if grep -q 'rel="preload"[^>]*\.wasm' "$SITE/index.html"; then
  echo "error: index.html still preloads the wasm (WebKit would download it twice)" >&2
  exit 1
fi

for wasm in "$SITE"/*.wasm; do
  size=$(wc -c <"$wasm" | tr -d ' ')
  echo "$(basename "$wasm"): $size bytes (limit $MAX_WASM_BYTES)"
  if [ "$size" -gt "$MAX_WASM_BYTES" ]; then
    echo "error: $(basename "$wasm") is over the 24 MiB gate" >&2
    exit 1
  fi
done

NAME="stucent-photo-editor-web-$TAG"
WORK="$OUT/$NAME"
rm -rf "$WORK" "$OUT/$NAME.zip"
mkdir -p "$WORK"
cp "$SITE"/index.html "$SITE"/*.js "$SITE"/*.wasm "$WORK/"
UPSTREAM_VERSION="$(awk '/^\[/ { p = ($0 == "[workspace.package]"); next } p && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }' "$ROOT/Cargo.toml")"
UPSTREAM_COMMIT="$(git -C "$ROOT" rev-parse "v$UPSTREAM_VERSION^{commit}" 2>/dev/null || echo unknown)"
python3 - "$WORK" "$TAG" "$UPSTREAM_VERSION" "$UPSTREAM_COMMIT" "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)" <<'PY'
import hashlib, json, os, sys
work, tag, upstream, upstream_commit, commit = sys.argv[1:6]
files = []
for name in sorted(os.listdir(work)):
    data = open(os.path.join(work, name), "rb").read()
    files.append({"path": name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
manifest = {
    "app": "stucent-photo-editor",
    "version": tag,
    "protocol": "emu/1",
    "upstream": f"photocraft {upstream}",
    "upstreamCommit": upstream_commit,
    "commit": commit,
    "entry": "index.html",
    "files": files,
}
with open(os.path.join(work, "manifest.json"), "w") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
PY
(cd "$WORK" && zip -qr9X "../$NAME.zip" .)
if command -v sha256sum >/dev/null; then
  (cd "$OUT" && sha256sum "$NAME.zip" >SHA256SUMS.txt)
else
  (cd "$OUT" && shasum -a 256 "$NAME.zip" >SHA256SUMS.txt)
fi
echo "wrote $OUT/$NAME.zip"
cat "$OUT/SHA256SUMS.txt"
