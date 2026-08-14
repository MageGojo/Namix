#!/usr/bin/env bash
# Local self-check: `nx new` in /tmp + a fake production dist that GETs every
# JS/CSS/WASM URL from real HTML. Does not touch dist/<semver> or any server.
#
#   ops/smoke-nx.sh
#   KEEP_WORKDIR=1 ops/smoke-nx.sh          # leave $WORKDIR on failure/success
#   SKIP_NEW=1 ops/smoke-nx.sh              # only the example-app release layout
#   SKIP_RELEASE=1 ops/smoke-nx.sh          # only nx new + cargo/npm
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKDIR="${WORKDIR:-$(mktemp -d /tmp/namix-smoke-XXXXXX)}"
KEEP_WORKDIR="${KEEP_WORKDIR:-0}"
SKIP_NEW="${SKIP_NEW:-0}"
SKIP_RELEASE="${SKIP_RELEASE:-0}"
APP_PID=""

log() { printf '→ %s\n' "$*"; }
fail() { printf '✗ %s\n' "$*" >&2; exit 1; }

stop_app() {
  if [[ -n "${APP_PID}" ]] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null || true
    wait "$APP_PID" 2>/dev/null || true
  fi
  APP_PID=""
}

cleanup() {
  stop_app
  if [[ "$KEEP_WORKDIR" != "1" ]]; then
    rm -rf "$WORKDIR"
  else
    printf 'kept workdir %s\n' "$WORKDIR" >&2
  fi
}
trap cleanup EXIT

nx() {
  cargo run -q --manifest-path "$ROOT/Cargo.toml" -p nx -- "$@"
}

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

wait_http() {
  local url="$1"
  local i
  for i in $(seq 1 80); do
    if curl -fsS -o /dev/null "$url" 2>/dev/null; then
      return 0
    fi
    sleep 0.25
  done
  return 1
}

extract_urls() {
  python3 -c '
import re, sys
text = sys.stdin.read()
urls = []
urls += re.findall(r"""(?:src|href)\s*=\s*["\x27]([^"\x27]+)["\x27]""", text, re.I)
urls += re.findall(r"""["\x27](/?(?:[A-Za-z0-9._-]+/)?build/[^"\x27\s)]+)["\x27]""", text)
urls += re.findall(r"""["\x27](assets/[^"\x27\s)]+\.(?:js|css|wasm|woff2?))["\x27]""", text)
for u in urls:
    print(u)
'
}

# Collect /build and prefixed asset URLs from HTML, then from downloaded JS.
audit_origin() {
  local origin="$1"
  shift
  local require_prefix="${REQUIRE_PREFIX:-}"
  local page html path tmp n
  local seen_file extra_file
  seen_file="$(mktemp "$WORKDIR/seen.XXXXXX")"
  extra_file="$(mktemp "$WORKDIR/extra.XXXXXX")"
  : >"$seen_file"
  : >"$extra_file"

  enqueue_file() {
    local dest="$1"
    local item="$2"
    [[ -n "$item" ]] || return 0
    case "$item" in
      http://*|https://*|//*|data:*) return 0 ;;
    esac
    case "$item" in
      *'*'*|*':'*) return 0 ;;
    esac
    case "$item" in
      /*) ;;
      *) item="/$item" ;;
    esac
    case "$item" in
      */build/*|/build/*|*/assets/*) ;;
      *) return 0 ;;
    esac
    case "$item" in
      *.js|*.css|*.wasm|*.woff|*.woff2|*.svg|*.png|*.jpg|*.jpeg|*.webp|*.ico) ;;
      *) return 0 ;;
    esac
    if grep -Fxq "$item" "$seen_file" 2>/dev/null; then
      return 0
    fi
    printf '%s\n' "$item" >>"$seen_file"
    printf '%s\n' "$item" >>"$dest"
  }

  get_ok() {
    local item="$1"
    local out="$2"
    local code
    code="$(curl -sS -o "$out" -w '%{http_code}' "${origin}${item}" || true)"
    if [[ "$code" != "200" ]]; then
      fail "GET ${origin}${item} → HTTP ${code:-curl-fail}"
    fi
    if [[ ! -s "$out" ]]; then
      fail "GET ${origin}${item} → empty body"
    fi
  }

  fetch_list() {
    local list="$1"
    local more="$2"
    : >"$more"
    while IFS= read -r path || [[ -n "$path" ]]; do
      [[ -n "$path" ]] || continue
      tmp="$(mktemp "$WORKDIR/body.XXXXXX")"
      get_ok "$path" "$tmp"
      case "$path" in
        *.js)
          while IFS= read -r u; do
            case "$u" in
              assets/*) enqueue_file "$more" "/build/$u" ;;
              *) enqueue_file "$more" "$u" ;;
            esac
          done < <(extract_urls <"$tmp")
          ;;
      esac
      rm -f "$tmp"
    done <"$list"
  }

  local html_list
  html_list="$(mktemp "$WORKDIR/html.XXXXXX")"
  : >"$html_list"
  for page in "$@"; do
    log "audit ${origin}${page}"
    html="$(curl -fsS "${origin}${page}")" || fail "GET ${origin}${page}"
    if printf '%s' "$html" | grep -q 'namix view: cd app && npm run build'; then
      fail "${page} rendered without Vite manifest"
    fi
    if [[ -n "$require_prefix" ]] && ! printf '%s' "$html" | grep -q "$require_prefix"; then
      fail "${page} HTML missing asset prefix ${require_prefix}"
    fi
    while IFS= read -r u; do
      enqueue_file "$html_list" "$u"
    done < <(printf '%s' "$html" | extract_urls)
  done

  fetch_list "$html_list" "$extra_file"
  if [[ -s "$extra_file" ]]; then
    local extra2
    extra2="$(mktemp "$WORKDIR/extra2.XXXXXX")"
    fetch_list "$extra_file" "$extra2"
    rm -f "$extra2"
  fi

  n="$(wc -l <"$seen_file" | tr -d ' ')"
  if [[ "$n" -eq 0 ]]; then
    printf '%s\n' "$html" | head -c 2500 >&2
    printf '\n' >&2
    fail "no asset URLs found at ${origin}"
  fi
  log "ok ${n} urls from ${origin}"
  rm -f "$html_list" "$seen_file" "$extra_file"
}

# Obfuscated JS may hide wasm URLs; always GET hashed files from disk.
audit_disk_assets() {
  local origin="$1"
  local build_dir="$2"
  local prefix="${3:-}"
  local rel path code
  [[ -d "$build_dir" ]] || fail "missing build dir $build_dir"
  while IFS= read -r path; do
    rel="${path#"$build_dir"/}"
    case "$rel" in
      ssr/*|.vite/*|manifest.json) continue ;;
    esac
    case "$rel" in
      *.js|*.css|*.wasm|*.woff|*.woff2) ;;
      *) continue ;;
    esac
    for item in "/build/${rel}" ${prefix:+"${prefix}/build/${rel}"}; do
      code="$(curl -sS -o /dev/null -w '%{http_code}' "${origin}${item}" || true)"
      if [[ "$code" != "200" ]]; then
        fail "GET ${origin}${item} → HTTP ${code:-curl-fail}"
      fi
    done
  done < <(find "$build_dir" -type f)
}

smoke_new() {
  local app="$WORKDIR/scaffold"
  log "nx new → $app"
  # stdin is not a TTY here; --single and --no-https skip the wizard.
  nx new smoke --single --no-https --tsx --no-git --path "$app"
  (
    cd "$app"
    log "nx doctor"
    nx doctor
    log "cargo check -p app"
    cargo check -p app
    log "npm typecheck"
    (cd app && npm install --no-fund --no-audit && npm run typecheck)
    log "nx make page Notes"
    nx make page Notes
    cargo check -p app
  )
  log "nx new track ok"
}

write_smoke_toml() {
  local src="$1"
  local dst="$2"
  python3 - "$src" "$dst" <<'PY'
import sys
from pathlib import Path
src, dst = Path(sys.argv[1]), Path(sys.argv[2])
text = src.read_text()
text = text.replace("https = true", "https = false")
text = text.replace("http3 = true", "http3 = false")
dst.write_text(text)
PY
}

sync_shared_assets() {
  local home="$1"
  local dist
  dist="$(dirname "$home")"
  local src="$home/public/build/assets"
  local shared="$dist/data/public/build/assets"
  [[ -d "$src" ]] || fail "missing $src"
  mkdir -p "$shared"
  cp -R "$src/." "$shared/"
}

start_app() {
  local home="$1"
  local port="$2"
  local logf="$3"
  shift 3
  mkdir -p "$home"
  # Spawn from the repo root so CWD ≠ NAMIX_HOME until init_workdir; assets
  # must still resolve via NAMIX_HOME (empty relative public/build is the 404).
  (
    cd "$ROOT"
    env NAMIX_HOME="$home" NAMIX_VITE_DEV=0 NAMIX_RELEASE_VERSION=0.0.0-smoke "$@" \
      "$home/app" -p "$port"
  ) >"$logf" 2>&1 &
  APP_PID="$!"
  if ! wait_http "http://127.0.0.1:${port}/__namix/health"; then
    cat "$logf" >&2 || true
    fail "app did not become ready on :${port}"
  fi
}

entry_file() {
  python3 - "$1" <<'PY'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
for cand in [root / ".vite/manifest.json", root / "manifest.json"]:
    if cand.is_file():
        data = json.loads(cand.read_text())
        entry = data.get("src/views/_entry.tsx")
        if not entry:
            entry = next((v for v in data.values() if isinstance(v, dict) and v.get("isEntry")), None)
        if not entry:
            raise SystemExit("no vite entry")
        print(entry["file"].split("/build/")[-1].lstrip("/"))
        break
else:
    raise SystemExit("no manifest")
PY
}

smoke_release() {
  local home="$WORKDIR/dist/0.0.0-smoke"
  local dist="$WORKDIR/dist"
  local cfg="$WORKDIR/namix.smoke.toml"
  mkdir -p "$home/public" "$dist/data/storage"

  log "cargo build -p app"
  cargo build --manifest-path "$ROOT/Cargo.toml" -p app

  if [[ ! -f "$ROOT/app/public/build/.vite/manifest.json" && ! -f "$ROOT/app/public/build/manifest.json" ]]; then
    log "frontend build (client)"
    (cd "$ROOT/app" && npm run build:client)
  fi
  [[ -f "$ROOT/app/public/build/.vite/manifest.json" || -f "$ROOT/app/public/build/manifest.json" ]] \
    || fail "app/public/build missing Vite manifest after frontend build"

  local bin=""
  if [[ -f "$ROOT/target/debug/app" ]]; then
    bin="$ROOT/target/debug/app"
  elif [[ -f "$ROOT/target/debug/app.exe" ]]; then
    bin="$ROOT/target/debug/app.exe"
  else
    fail "missing target/debug/app"
  fi
  cp "$bin" "$home/app"
  chmod +x "$home/app"
  cp -R "$ROOT/app/public/build" "$home/public/build"
  write_smoke_toml "$ROOT/app/namix.toml" "$cfg"
  cp "$cfg" "$home/namix.toml"
  if [[ -d "$ROOT/app/lang" ]]; then
    cp -R "$ROOT/app/lang" "$home/lang"
  fi
  ln -sfn ../data/storage "$home/storage"
  sync_shared_assets "$home"

  local port sample
  port="$(free_port)"
  log "start example app :${port} (no prefix)"
  start_app "$home" "$port" "$WORKDIR/app.noprefix.log" NAMIX_CONFIG="$cfg"
  REQUIRE_PREFIX="" audit_origin "http://127.0.0.1:${port}" / /login
  audit_disk_assets "http://127.0.0.1:${port}" "$home/public/build"
  stop_app

  port="$(free_port)"
  log "start example app :${port} NAMIX_ASSET_PREFIX=/lr"
  start_app "$home" "$port" "$WORKDIR/app.lr.log" NAMIX_CONFIG="$cfg" NAMIX_ASSET_PREFIX=/lr
  REQUIRE_PREFIX="/lr/build" audit_origin "http://127.0.0.1:${port}" / /login
  sample="$(entry_file "$home/public/build")"
  curl -fsS -o /dev/null "http://127.0.0.1:${port}/build/${sample}" \
    || fail "root /build/${sample} 404 while prefix=/lr"
  curl -fsS -o /dev/null "http://127.0.0.1:${port}/lr/build/${sample}" \
    || fail "/lr/build/${sample} 404"
  audit_disk_assets "http://127.0.0.1:${port}" "$home/public/build" "/lr"
  stop_app
  log "release layout track ok"
}

log "workdir $WORKDIR"
cd "$ROOT"
if [[ "$SKIP_NEW" != "1" ]]; then
  smoke_new
fi
if [[ "$SKIP_RELEASE" != "1" ]]; then
  smoke_release
fi
log "smoke-nx ok"
