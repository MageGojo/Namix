//! `nx new` 生成的 Vite + React 前端（TSX / JSX + 可选 Tailwind）。

use std::fs;
use std::path::Path;

use crate::template::FrontendLang;

pub fn scaffold(root: &Path, name: &str, lang: FrontendLang, tailwind: bool) -> Result<(), String> {
    let fe = root.join("frontend");
    fs::create_dir_all(fe.join("src")).map_err(|e| e.to_string())?;

    write(fe.join("package.json"), &package_json(name, lang, tailwind))?;
    write(fe.join(".gitignore"), "node_modules\ndist\n.DS_Store\n")?;

    match lang {
        FrontendLang::Tsx => scaffold_tsx(&fe, name, tailwind)?,
        FrontendLang::Jsx => scaffold_jsx(&fe, name, tailwind)?,
    }

    if tailwind {
        write(
            fe.join("src/index.css"),
            "/* Namix + Tailwind CSS v4 */\n@import \"tailwindcss\";\n",
        )?;
    } else {
        write(
            fe.join("src/index.css"),
            "/* Namix frontend */\n:root {\n  color-scheme: light;\n  font-family: ui-sans-serif, system-ui, sans-serif;\n}\nbody {\n  margin: 0;\n  min-height: 100vh;\n}\n#root {\n  min-height: 100vh;\n}\n",
        )?;
    }

    Ok(())
}

fn scaffold_tsx(fe: &Path, name: &str, tailwind: bool) -> Result<(), String> {
    write(fe.join("vite.config.ts"), &vite_config_ts(tailwind))?;
    write(
        fe.join("tsconfig.json"),
        r#"{
  "files": [],
  "references": [
    { "path": "./tsconfig.app.json" },
    { "path": "./tsconfig.node.json" }
  ]
}
"#,
    )?;
    write(
        fe.join("tsconfig.app.json"),
        r#"{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.app.tsbuildinfo",
    "target": "ES2022",
    "useDefineForClassFields": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "moduleDetection": "force",
    "noEmit": true,
    "jsx": "react-jsx",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "erasableSyntaxOnly": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true
  },
  "include": ["src"]
}
"#,
    )?;
    write(
        fe.join("tsconfig.node.json"),
        r#"{
  "compilerOptions": {
    "tsBuildInfoFile": "./node_modules/.tmp/tsconfig.node.tsbuildinfo",
    "target": "ES2023",
    "lib": ["ES2023"],
    "module": "ESNext",
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "verbatimModuleSyntax": true,
    "moduleDetection": "force",
    "noEmit": true,
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "erasableSyntaxOnly": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedSideEffectImports": true
  },
  "include": ["vite.config.ts"]
}
"#,
    )?;
    write(
        fe.join("index.html"),
        &format!(
            r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{name}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#
        ),
    )?;
    write(
        fe.join("src/vite-env.d.ts"),
        "/// <reference types=\"vite/client\" />\n",
    )?;
    write(fe.join("src/main.tsx"), MAIN_TSX)?;
    write(fe.join("src/App.tsx"), &app_tsx(name, tailwind))?;
    write(fe.join("src/routes.ts"), ROUTES_TS_STUB)?;
    Ok(())
}

fn scaffold_jsx(fe: &Path, name: &str, tailwind: bool) -> Result<(), String> {
    write(fe.join("vite.config.js"), &vite_config_js(tailwind))?;
    write(
        fe.join("index.html"),
        &format!(
            r#"<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>{name}</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.jsx"></script>
  </body>
</html>
"#
        ),
    )?;
    write(fe.join("src/main.jsx"), MAIN_JSX)?;
    write(fe.join("src/App.jsx"), &app_jsx(name, tailwind))?;
    write(fe.join("src/routes.js"), ROUTES_JS_STUB)?;
    Ok(())
}

fn package_json(name: &str, lang: FrontendLang, tailwind: bool) -> String {
    let deps = [
        r#"    "react": "^19.1.0""#.to_string(),
        r#"    "react-dom": "^19.1.0""#.to_string(),
    ];
    let mut dev = vec![
        r#"    "@vitejs/plugin-react": "^4.5.2""#.to_string(),
        r#"    "vite": "^6.3.5""#.to_string(),
    ];
    if matches!(lang, FrontendLang::Tsx) {
        dev.push(r#"    "@types/react": "^19.1.6""#.to_string());
        dev.push(r#"    "@types/react-dom": "^19.1.5""#.to_string());
        dev.push(r#"    "typescript": "~5.8.3""#.to_string());
    }
    if tailwind {
        dev.push(r#"    "@tailwindcss/vite": "^4.1.8""#.to_string());
        dev.push(r#"    "tailwindcss": "^4.1.8""#.to_string());
    }
    format!(
        r#"{{
  "name": "{name}-frontend",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {{
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview"
  }},
  "dependencies": {{
{deps}
  }},
  "devDependencies": {{
{dev}
  }}
}}
"#,
        deps = deps.join(",\n"),
        dev = dev.join(",\n"),
    )
}

fn vite_config_ts(tailwind: bool) -> String {
    if tailwind {
        r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/__namix': 'http://127.0.0.1:3000',
    },
  },
})
"#
        .into()
    } else {
        r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/__namix': 'http://127.0.0.1:3000',
    },
  },
})
"#
        .into()
    }
}

fn vite_config_js(tailwind: bool) -> String {
    if tailwind {
        r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/__namix': 'http://127.0.0.1:3000',
    },
  },
})
"#
        .into()
    } else {
        r#"import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': 'http://127.0.0.1:3000',
      '/__namix': 'http://127.0.0.1:3000',
    },
  },
})
"#
        .into()
    }
}

fn app_tsx(name: &str, tailwind: bool) -> String {
    if tailwind {
        format!(
            r#"import {{ route }} from './routes'

export default function App() {{
  return (
    <main className="min-h-screen bg-zinc-50 text-zinc-900">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-16">
        <p className="text-sm font-medium tracking-wide text-teal-700">Namix</p>
        <h1 className="text-4xl font-semibold tracking-tight">{name}</h1>
        <p className="max-w-xl text-lg text-zinc-600">
          Vite + React (TSX) + Tailwind。后端路由可用{{" "}}
          <code className="rounded bg-zinc-200 px-1.5 py-0.5 text-sm">route()</code>。
        </p>
        <p className="font-mono text-sm text-zinc-500">
          home → {{route("main.home")}}
        </p>
      </div>
    </main>
  )
}}
"#
        )
    } else {
        format!(
            r#"import {{ route }} from './routes'

export default function App() {{
  return (
    <main style={{{{ padding: '4rem 1.5rem', fontFamily: 'system-ui' }}}}>
      <p>Namix</p>
      <h1>{name}</h1>
      <p>Vite + React (TSX). home → {{route('main.home')}}</p>
    </main>
  )
}}
"#
        )
    }
}

fn app_jsx(name: &str, tailwind: bool) -> String {
    if tailwind {
        format!(
            r#"import {{ route }} from './routes'

export default function App() {{
  return (
    <main className="min-h-screen bg-zinc-50 text-zinc-900">
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-16">
        <p className="text-sm font-medium tracking-wide text-teal-700">Namix</p>
        <h1 className="text-4xl font-semibold tracking-tight">{name}</h1>
        <p className="max-w-xl text-lg text-zinc-600">
          Vite + React (JSX) + Tailwind。后端路由可用{{" "}}
          <code className="rounded bg-zinc-200 px-1.5 py-0.5 text-sm">route()</code>。
        </p>
        <p className="font-mono text-sm text-zinc-500">
          home → {{route("main.home")}}
        </p>
      </div>
    </main>
  )
}}
"#
        )
    } else {
        format!(
            r#"import {{ route }} from './routes'

export default function App() {{
  return (
    <main style={{{{ padding: '4rem 1.5rem', fontFamily: 'system-ui' }}}}>
      <p>Namix</p>
      <h1>{name}</h1>
      <p>Vite + React (JSX). home → {{route('main.home')}}</p>
    </main>
  )
}}
"#
        )
    }
}

const MAIN_TSX: &str = r#"import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
"#;

const MAIN_JSX: &str = r#"import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.jsx'

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
"#;

const ROUTES_TS_STUB: &str = r#"/* @generated by Namix — DO NOT EDIT
 * Boot / `nx export routes` 会覆盖本文件。
 * import { route } from './routes' → route('main.home')
 */

export const routes = {
  "main.home": { uri: "/", methods: ["GET"] as const },
} as const;

export type RouteName = keyof typeof routes;

export function route(
  name: RouteName,
  params?: Record<string, string | number>
): string {
  let uri: string = routes[name].uri;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      uri = uri.replace(`:${k}`, String(v));
    }
  }
  if (uri.split('/').some((s) => s.startsWith(':'))) {
    throw new Error(`route('${name}') missing params: ${uri}`);
  }
  return uri;
}
"#;

const ROUTES_JS_STUB: &str = r#"/* @generated by Namix — DO NOT EDIT
 * Boot / `nx export routes` 会覆盖本文件。
 * import { route } from './routes' → route('main.home')
 */

export const routes = {
  "main.home": { uri: "/", methods: ["GET"] },
};

export function route(name, params) {
  let uri = routes[name].uri;
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      uri = uri.replace(`:${k}`, String(v));
    }
  }
  if (uri.split('/').some((s) => s.startsWith(':'))) {
    throw new Error(`route('${name}') missing params: ${uri}`);
  }
  return uri;
}
"#;

fn write(path: impl AsRef<Path>, body: &str) -> Result<(), String> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(path, body).map_err(|e| e.to_string())
}
