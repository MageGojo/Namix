//! `nx new`：在 **app/** 内脚手架全栈视图工具链（Vite + React + Tailwind）。
//! 不再生成根目录独立 `frontend/` SPA —— Namix 不是纯后端框架。

use std::fs;
use std::path::Path;

use crate::template::FrontendLang;

pub fn scaffold(root: &Path, name: &str, lang: FrontendLang, tailwind: bool) -> Result<(), String> {
    let app = root.join("app");
    fs::create_dir_all(app.join("src/views/pages")).map_err(|e| e.to_string())?;
    fs::create_dir_all(app.join("src/views/components")).map_err(|e| e.to_string())?;
    fs::create_dir_all(app.join("src/views/lib")).map_err(|e| e.to_string())?;
    fs::create_dir_all(app.join("src/views/generated")).map_err(|e| e.to_string())?;
    fs::create_dir_all(app.join("src/views/generated/seal")).map_err(|e| e.to_string())?;

    write(app.join("package.json"), PACKAGE_JSON)?;
    write(app.join("tsconfig.json"), TSCONFIG)?;
    write(
        app.join("vite.config.ts"),
        if tailwind {
            VITE_CONFIG_TW
        } else {
            VITE_CONFIG_PLAIN
        },
    )?;

    // 只复制与业务无关的前端运行时。示例应用里的 routes、聊天类型、导航等
    // 都属于示例业务，复制到新项目会制造幽灵路由和缺失的生成类型。
    let fw_views = PathBufFw::views();
    if fw_views.is_dir() {
        for name in ["_entry.tsx", "_ssr.tsx", "types.ts", "vite-env.d.ts"] {
            let src = fw_views.join(name);
            if src.is_file() {
                fs::copy(&src, app.join("src/views").join(name))
                    .map_err(|error| format!("copy {}: {error}", src.display()))?;
            }
        }
        for (dir, files) in [
            (
                "lib",
                &[
                    "actionError.ts",
                    "csrf.tsx",
                    "page.tsx",
                    "progress.ts",
                    "router.ts",
                    "theme.ts",
                    "useForm.ts",
                    "i18n.ts",
                ][..],
            ),
            ("components", &["head.tsx", "link.tsx"][..]),
        ] {
            for file in files {
                let src = fw_views.join(dir).join(file);
                if src.is_file() {
                    fs::copy(&src, app.join("src/views").join(dir).join(file))
                        .map_err(|error| format!("copy {}: {error}", src.display()))?;
                }
            }
        }
    }

    // Keep the scaffold self-contained when the framework checkout predates
    // this runtime helper; current checkouts copy the same business-neutral
    // module through the whitelist above.
    let csrf_runtime = app.join("src/views/lib/csrf.tsx");
    if !csrf_runtime.is_file() {
        write(&csrf_runtime, CSRF_TSX)?;
    }
    let theme_runtime = app.join("src/views/lib/theme.ts");
    if !theme_runtime.is_file() {
        write(&theme_runtime, THEME_TS)?;
    }
    let i18n_runtime = app.join("src/views/lib/i18n.ts");
    if !i18n_runtime.is_file() {
        write(&i18n_runtime, I18N_TS)?;
    }
    write(app.join("lang/zh-CN.json"), LANG_ZH_CN)?;
    write(app.join("lang/en.json"), LANG_EN)?;

    write(app.join("src/views/namix.ts"), NAMIX_TS)?;
    write(app.join("src/views/routes.ts"), ROUTES_TS)?;
    write(
        app.join("src/views/generated/registry.ts"),
        INITIAL_REGISTRY_TS,
    )?;
    // `cargo check` generates callRust.ts before the WASM package exists. Keep
    // plain `npm run typecheck` useful in a brand-new project; wasm-bindgen
    // replaces this declaration with its generated one during the first build.
    write(
        app.join("src/views/generated/seal/namix_seal.d.ts"),
        INITIAL_SEAL_D_TS,
    )?;
    write(
        app.join("scripts/build-seal-wasm.sh"),
        &seal_build_script(&PathBufFw::root()),
    )?;

    write(
        app.join("src/views/app.css"),
        if tailwind { APP_CSS_TW } else { APP_CSS_PLAIN },
    )?;

    // 页面语言遵守 --tsx/--jsx；框架内部运行时仍以 TypeScript 维护。
    let pages = app.join("src/views/pages");
    let (home, stale_home, props_type) = match lang {
        FrontendLang::Tsx => (
            pages.join("home.tsx"),
            pages.join("home.jsx"),
            ": { title?: string; message?: string }",
        ),
        FrontendLang::Jsx => (pages.join("home.jsx"), pages.join("home.tsx"), ""),
    };
    if stale_home.is_file() {
        fs::remove_file(&stale_home)
            .map_err(|error| format!("remove stale {}: {error}", stale_home.display()))?;
    }
    write(
        &home,
        &format!(
            r#"import {{ Head }} from '../namix'

export default function Home({{ title, message }}{props_type} = {{}}) {{
  return (
    <main className="mx-auto max-w-3xl px-6 py-14">
      <Head title={{title ? `${{title}} · {name}` : '{name}'}} />
      <p className="text-sm font-medium tracking-wide text-teal-700">Namix</p>
      <h1 className="mt-2 text-4xl font-semibold tracking-tight">{{title ?? '{name}'}}</h1>
      <p className="mt-3 text-zinc-600">{{message ?? '全栈 views：控制器 req.view 渲染本页。'}}</p>
    </main>
  )
}}
"#
        ),
    )?;

    write(
        app.join("src/views/.namix-feature"),
        "feature = \"pages\"\n# managed by namix-build — do not remove\n",
    )?;

    Ok(())
}

struct PathBufFw;
impl PathBufFw {
    fn root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
    }

    fn views() -> std::path::PathBuf {
        Self::root().join("app/src/views")
    }
}

fn seal_build_script(framework_root: &Path) -> String {
    let framework_root = framework_root
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
APP_DIR="$ROOT/app"
FRAMEWORK_ROOT="{framework_root}"
TARGET_DIR="${{CARGO_TARGET_DIR:-$ROOT/target/namix-seal}}"
OUT="$APP_DIR/src/views/generated/seal"

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
WB_VER="$(cargo tree --manifest-path "$FRAMEWORK_ROOT/crates/namix-seal/Cargo.toml" -i wasm-bindgen --depth 0 2>/dev/null | sed -n 's/.*wasm-bindgen v\([0-9.]*\).*/\1/p' | head -1)"
WB_VER="${{WB_VER:-0.2.126}}"
if ! wasm-bindgen -V 2>/dev/null | grep -q "$WB_VER"; then
  cargo install wasm-bindgen-cli --version "$WB_VER" --force --locked
fi

NAMIX_APP_DIR="$APP_DIR" CARGO_TARGET_DIR="$TARGET_DIR" \
  cargo build --manifest-path "$FRAMEWORK_ROOT/crates/namix-seal/Cargo.toml" \
  --target wasm32-unknown-unknown --release
mkdir -p "$OUT"
wasm-bindgen "$TARGET_DIR/wasm32-unknown-unknown/release/namix_seal.wasm" \
  --out-dir "$OUT" --target web
test -f "$OUT/namix_seal.js"
echo "✓ wasm → $OUT"
"#
    )
}

fn write(path: impl AsRef<Path>, body: &str) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| e.to_string())
}

const PACKAGE_JSON: &str = r#"{
  "name": "namix-app-views",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "typecheck": "tsc --noEmit",
    "build:wasm": "bash scripts/build-seal-wasm.sh",
    "build": "npm run build:wasm && vite build",
    "build:client": "npm run build:wasm && vite build",
    "build:ssr": "vite build --ssr src/views/_ssr.tsx --outDir public/build/ssr --emptyOutDir",
    "preview": "vite preview"
  },
  "dependencies": {
    "nprogress": "^0.2.0",
    "react": "^19.1.0",
    "react-dom": "^19.1.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.1.8",
    "@types/node": "^26.1.2",
    "@types/nprogress": "^0.2.3",
    "@types/react": "^19.1.6",
    "@types/react-dom": "^19.1.5",
    "@vitejs/plugin-react": "^4.5.2",
    "javascript-obfuscator": "^5.5.0",
    "tailwindcss": "^4.1.8",
    "typescript": "~5.8.3",
    "vite": "^6.3.5"
  }
}
"#;

const NAMIX_TS: &str = r#"/** Namix frontend facade (business-neutral).
 *
 * import { Link, Head, useForm, route, AppRoute } from '../namix'
 * route.login()  ≡  route(AppRoute.Login)
 */
export { Link, type LinkProps } from './components/link'
export { Head } from './components/head'
export { applyTheme, setTheme, theme, toggleTheme, type Theme } from './lib/theme'
export { router, type VisitOptions } from './lib/router'
export { useForm, type SubmitOpts } from './lib/useForm'
export { csrfToken, CsrfField, type CsrfFieldProps } from './lib/csrf'
export { t } from './lib/i18n'
export {
  ActionException,
  parseActionFailure,
  translateErrors,
  type FieldErrors,
} from './lib/actionError'
export { usePage, PageProvider } from './lib/page'
export { progress, configureProgress } from './lib/progress'
export { route, AppRoute, routes, type RouteName } from './routes'
export type { NamixPage, PageProps } from './types'
"#;

const I18N_TS: &str = r#"import zh from '../../../lang/zh-CN.json'
import en from '../../../lang/en.json'

type Json = string | number | boolean | null | Json[] | { [key: string]: Json }

const catalogs: Record<string, Record<string, string>> = {
  'zh-CN': flatten('', zh as Json, {}),
  zh: flatten('', zh as Json, {}),
  en: flatten('', en as Json, {}),
}

function flatten(prefix: string, value: Json, out: Record<string, string>): Record<string, string> {
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    for (const [key, child] of Object.entries(value)) {
      const next = prefix ? `${prefix}.${key}` : key
      flatten(next, child, out)
    }
    return out
  }
  if (prefix && (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean')) {
    out[prefix] = String(value)
  }
  return out
}

export function locale(): string {
  if (typeof document === 'undefined') return 'zh-CN'
  return document.documentElement.lang || 'zh-CN'
}

function catalog(): Record<string, string> {
  const current = locale()
  return catalogs[current] ?? catalogs['zh-CN'] ?? {}
}

function interpolate(template: string, params?: Record<string, string>): string {
  if (!params) return template
  let out = template
  for (const [key, value] of Object.entries(params)) {
    out = out.replaceAll(`:${key}`, value)
  }
  return out
}

/** Look up `auth.failed` / `username.taken`; falls back to `validation.{rule}`. */
export function t(key: string, params?: Record<string, string>): string {
  const messages = catalog()
  const specific = messages[key]
  if (specific) return interpolate(specific, params)
  const dot = key.lastIndexOf('.')
  if (dot > 0) {
    const attribute = key.slice(0, dot)
    const rule = key.slice(dot + 1)
    const fallback = messages[`validation.${rule}`]
    if (fallback) {
      const attr = messages[`attributes.${attribute}`] ?? attribute
      return interpolate(fallback, { attribute: attr, ...params })
    }
  }
  return interpolate(key, params)
}
"#;

const LANG_ZH_CN: &str = r#"{
  "auth": {
    "failed": "用户名或密码不正确"
  },
  "validation": {
    "failed": "校验未通过",
    "payload": "请求内容无效",
    "required": "请填写 :attribute",
    "email": ":attribute 不是有效邮箱",
    "min": ":attribute 太短",
    "max": ":attribute 太长",
    "between": ":attribute 长度不符合要求",
    "numeric": ":attribute 必须是数字",
    "integer": ":attribute 必须是整数",
    "digits": ":attribute 位数不正确",
    "alpha_num": ":attribute 只能含字母和数字",
    "url": ":attribute 必须是网址",
    "local_path": ":attribute 必须是站内路径",
    "boolean": ":attribute 必须是布尔值",
    "accepted": "请同意 :attribute",
    "declined": "请拒绝 :attribute",
    "eq": ":attribute 值不正确",
    "invalid": ":attribute 不合法",
    "starts_with": ":attribute 前缀不正确",
    "ends_with": ":attribute 后缀不正确",
    "confirmed": "两次 :attribute 不一致",
    "same": ":attribute 必须与另一字段相同",
    "regex": ":attribute 格式不正确",
    "file": "请上传 :attribute",
    "image": ":attribute 必须是图片",
    "mimes": ":attribute 文件类型不允许",
    "max_bytes": ":attribute 文件过大",
    "taken": ":attribute 已被占用",
    "exists": ":attribute 不存在",
    "presence": "暂时无法校验 :attribute"
  }
}
"#;

const LANG_EN: &str = r#"{
  "auth": {
    "failed": "Invalid username or password"
  },
  "validation": {
    "failed": "Validation failed",
    "payload": "The request payload is invalid",
    "required": ":attribute is required",
    "email": ":attribute must be a valid email",
    "min": ":attribute is too short",
    "max": ":attribute is too long",
    "between": ":attribute length is invalid",
    "numeric": ":attribute must be numeric",
    "integer": ":attribute must be an integer",
    "digits": ":attribute has the wrong number of digits",
    "alpha_num": ":attribute must be alphanumeric",
    "url": ":attribute must be a URL",
    "local_path": ":attribute must be a local path",
    "boolean": ":attribute must be boolean",
    "accepted": ":attribute must be accepted",
    "declined": ":attribute must be declined",
    "eq": ":attribute is incorrect",
    "invalid": ":attribute is invalid",
    "starts_with": ":attribute prefix is invalid",
    "ends_with": ":attribute suffix is invalid",
    "confirmed": ":attribute confirmation does not match",
    "same": ":attribute must match the other field",
    "regex": ":attribute format is invalid",
    "file": ":attribute must be a file",
    "image": ":attribute must be an image",
    "mimes": ":attribute file type is not allowed",
    "max_bytes": ":attribute is too large",
    "taken": ":attribute has already been taken",
    "exists": ":attribute does not exist",
    "presence": ":attribute cannot be checked right now"
  }
}
"#;

const CSRF_TSX: &str = r#"import { useEffect, useState } from 'react'

const CSRF_COOKIE = 'namix_csrf'

/** Read Namix's readable double-submit CSRF cookie. */
export function csrfToken(cookieSource?: string): string {
  const source =
    cookieSource ?? (typeof document === 'undefined' ? '' : document.cookie)
  for (const part of source.split(';')) {
    const separator = part.indexOf('=')
    if (separator < 0 || part.slice(0, separator).trim() !== CSRF_COOKIE) continue
    const value = part.slice(separator + 1).trim()
    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }
  return ''
}

export type CsrfFieldProps = { token?: string }

/** Hidden field for classic browser POST forms protected by Namix CSRF. */
export function CsrfField({ token }: CsrfFieldProps) {
  // Both SSR and the client's first hydration render use the same value. The
  // readable browser cookie is applied immediately after mount.
  const [value, setValue] = useState(token ?? '')
  useEffect(() => {
    setValue(token ?? csrfToken())
  }, [token])
  return <input type="hidden" name="_csrf" value={value} />
}
"#;

const THEME_TS: &str = r#"export type Theme = 'dark' | 'light' | 'system'

const COOKIE = 'namix_theme'

function readCookie(source: string): string {
  for (const part of source.split(';')) {
    const separator = part.indexOf('=')
    if (separator < 0 || part.slice(0, separator).trim() !== COOKIE) continue
    const value = part.slice(separator + 1).trim()
    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }
  return ''
}

export function theme(cookieSource?: string): Theme {
  const source = cookieSource ?? (typeof document === 'undefined' ? '' : document.cookie)
  const value = readCookie(source)
  if (value === 'dark' || value === 'light' || value === 'system') return value
  return 'system'
}

export function applyTheme(next: Theme = theme()) {
  if (typeof document === 'undefined') return
  const dark =
    next === 'dark' ||
    (next !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches)
  const resolved = dark ? 'dark' : 'light'
  document.documentElement.setAttribute('data-theme', resolved)
  document.documentElement.style.colorScheme = resolved
}

export function setTheme(next: Theme) {
  if (typeof document === 'undefined') return
  const secure = location.protocol === 'https:' ? '; Secure' : ''
  document.cookie = `${COOKIE}=${encodeURIComponent(next)}; Path=/; Max-Age=31536000; SameSite=Lax${secure}`
  applyTheme(next)
}

export function toggleTheme() {
  setTheme(document.documentElement.getAttribute('data-theme') === 'dark' ? 'light' : 'dark')
}
"#;

const ROUTES_TS: &str = r#"/* @generated bootstrap; Namix codegen replaces this file. */
export const routes = {
  home: { uri: '/', methods: ['GET'] as const },
} as const

export type RouteName = keyof typeof routes

/** 与 Rust `AppRoute::Home` 对齐；`route(AppRoute.Home)` ≡ `route.home()` */
export const AppRoute = {
  Home: 'home',
} as const
export type AppRoute = (typeof AppRoute)[keyof typeof AppRoute]

function fill(uri: string, params?: Record<string, string | number>): string {
  let out = uri
  for (const [key, value] of Object.entries(params ?? {})) {
    out = out.replace(`:${key}`, encodeURIComponent(String(value)))
  }
  if (out.split('/').some((segment) => segment.startsWith(':'))) {
    throw new Error(`missing route params: ${out}`)
  }
  return out
}

export function resolveRoute(
  name: RouteName,
  params?: Record<string, string | number>,
): string {
  return fill(routes[name].uri, params)
}

export const route = {
  home: Object.assign(() => fill(routes.home.uri), {
    routeName: 'home' as const,
    uri: '/' as const,
  }),
} as const

type RouteFn = {
  (name: RouteName, params?: Record<string, string | number>): string
} & typeof route

export default Object.assign(
  ((name: RouteName, params?: Record<string, string | number>) =>
    resolveRoute(name, params)) as RouteFn,
  route,
)
"#;

const INITIAL_REGISTRY_TS: &str = r#"/* @generated bootstrap; namix-build replaces this file. */
import type { ComponentType } from 'react'
import Home from '../pages/home'

export const pages: Record<string, ComponentType<Record<string, unknown>>> = {
  home: Home as unknown as ComponentType<Record<string, unknown>>,
}
"#;

const INITIAL_SEAL_D_TS: &str = r#"/* Bootstrap declaration; wasm-bindgen replaces this file. */
export default function init(input?: unknown): Promise<unknown>
export function nx_call(token: string, bodyJson: string): Promise<string>
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "types": ["node"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowJs": true,
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "resolveJsonModule": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true
  },
  "include": ["src/views"]
}
"#;

const VITE_CONFIG_TW: &str = r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

/** 与运行时 `NAMIX_ASSET_PREFIX` / `NAMIX_ASSET_BASE` 对齐，避免子路径挂载后 JS/wasm 404 白屏 */
function productionAssetBase() {
  const fromBase = (process.env.NAMIX_ASSET_BASE || '').trim().replace(/\/$/, '')
  if (fromBase) return (fromBase.startsWith('/') ? fromBase : `/${fromBase}`) + '/'
  const prefix = (process.env.NAMIX_ASSET_PREFIX || '').trim().replace(/\/$/, '')
  if (prefix) {
    const p = prefix.startsWith('/') ? prefix : `/${prefix}`
    return `${p}/build/`
  }
  return '/build/'
}

export default defineConfig(({ command }) => ({
  plugins: [react(), tailwindcss()],
  base: command === 'serve' ? '/' : productionAssetBase(),
  publicDir: false,
  build: {
    outDir: 'public/build',
    emptyOutDir: true,
    manifest: true,
    minify: 'esbuild',
    rollupOptions: { input: 'src/views/_entry.tsx' },
  },
  server: { origin: process.env.NAMIX_VITE_ORIGIN ?? 'http://127.0.0.1:5173' },
}))
"#;

const VITE_CONFIG_PLAIN: &str = r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

/** 与运行时 `NAMIX_ASSET_PREFIX` / `NAMIX_ASSET_BASE` 对齐，避免子路径挂载后 JS/wasm 404 白屏 */
function productionAssetBase() {
  const fromBase = (process.env.NAMIX_ASSET_BASE || '').trim().replace(/\/$/, '')
  if (fromBase) return (fromBase.startsWith('/') ? fromBase : `/${fromBase}`) + '/'
  const prefix = (process.env.NAMIX_ASSET_PREFIX || '').trim().replace(/\/$/, '')
  if (prefix) {
    const p = prefix.startsWith('/') ? prefix : `/${prefix}`
    return `${p}/build/`
  }
  return '/build/'
}

export default defineConfig(({ command }) => ({
  plugins: [react()],
  base: command === 'serve' ? '/' : productionAssetBase(),
  publicDir: false,
  build: {
    outDir: 'public/build',
    emptyOutDir: true,
    manifest: true,
    minify: 'esbuild',
    rollupOptions: { input: 'src/views/_entry.tsx' },
  },
  server: { origin: process.env.NAMIX_VITE_ORIGIN ?? 'http://127.0.0.1:5173' },
}))
"#;

const APP_CSS_TW: &str = r#"@import "tailwindcss";
@custom-variant dark (&:where(.dark, .dark *), &:where([data-theme=dark], [data-theme=dark] *));

html { color-scheme: light; background: #fafafa; color: #18181b; }
html[data-theme="dark"] { color-scheme: dark; background: #09090b; color: #fafafa; }
html[data-theme="dark"] body { background: inherit; color: inherit; }

#nprogress { pointer-events: none; }
#nprogress .bar {
  background: var(--namix-progress, #0f766e);
  position: fixed; z-index: 1031; top: 0; left: 0; width: 100%; height: 2px;
}
"#;

const APP_CSS_PLAIN: &str = r#"body { margin: 0; font-family: system-ui, sans-serif; }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_case(language: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "namix-frontend-{language}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("app")).expect("create app directory");
        root
    }

    #[test]
    fn scaffold_honors_page_language_and_seeds_seal_types() {
        for (language, expected, absent) in [
            (FrontendLang::Tsx, "home.tsx", "home.jsx"),
            (FrontendLang::Jsx, "home.jsx", "home.tsx"),
        ] {
            let root = temp_case(language.label());
            scaffold(&root, "demo", language, true).expect("scaffold frontend");

            let pages = root.join("app/src/views/pages");
            assert!(pages.join(expected).is_file());
            assert!(!pages.join(absent).exists());
            assert!(
                root.join("app/src/views/generated/seal/namix_seal.d.ts")
                    .is_file()
            );
            let csrf = fs::read_to_string(root.join("app/src/views/lib/csrf.tsx"))
                .expect("read csrf runtime");
            assert!(csrf.contains("name=\"_csrf\""));
            let facade = fs::read_to_string(root.join("app/src/views/namix.ts"))
                .expect("read frontend facade");
            assert!(facade.contains("csrfToken, CsrfField"));
            assert!(facade.contains("export { t }"));
            assert!(root.join("app/src/views/lib/i18n.ts").is_file());
            assert!(root.join("app/lang/zh-CN.json").is_file());
            assert!(root.join("app/lang/en.json").is_file());
            let tsconfig = fs::read_to_string(root.join("app/tsconfig.json")).expect("tsconfig");
            assert!(tsconfig.contains("resolveJsonModule"));
            let _ = fs::remove_dir_all(root);
        }
    }
}
