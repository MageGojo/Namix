import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import JavaScriptObfuscator from 'javascript-obfuscator'

/** 默认开混淆；`NAMIX_OBFUSCATE=0` 关闭 */
const obfuscateOn = process.env.NAMIX_OBFUSCATE !== '0'
/** 默认最小体积轻混淆；`NAMIX_MIN_SIZE=0` 可开更重混淆 */
const minSize = process.env.NAMIX_MIN_SIZE !== '0'

/**
 * 生产客户端 JS 混淆（SSR bundle 不混淆，避免弄坏 Node worker）。
 * WASM 二进制不动；仅处理 emit 出的 .js chunk。
 */
function obfuscateClientJs(): Plugin {
  return {
    name: 'namix-obfuscate-client-js',
    apply: 'build',
    enforce: 'post',
    renderChunk(code, chunk) {
      if (!chunk.fileName.endsWith('.js')) return null

      const result = JavaScriptObfuscator.obfuscate(code, {
        compact: true,
        simplify: true,
        renameGlobals: false,
        identifierNamesGenerator: 'hexadecimal',
        stringArray: true,
        stringArrayRotate: true,
        stringArrayShuffle: true,
        stringArrayThreshold: minSize ? 0.5 : 0.75,
        stringArrayEncoding: minSize ? ['none'] : ['base64'],
        splitStrings: !minSize,
        splitStringsChunkLength: 6,
        // control-flow 体积代价大，最小体积模式关闭
        controlFlowFlattening: !minSize,
        controlFlowFlatteningThreshold: 0.35,
        deadCodeInjection: false,
        selfDefending: false,
        debugProtection: false,
        disableConsoleOutput: false,
        reservedStrings: ['/build/', 'namix_seal', 'nx_call'],
      })
      return { code: result.getObfuscatedCode(), map: null }
    },
  }
}

export default defineConfig(({ command, isSsrBuild }) => ({
  plugins: [
    react(),
    tailwindcss(),
    !isSsrBuild && obfuscateOn && obfuscateClientJs(),
  ],
  // 开发 HMR 挂在 /；生产静态挂在 Namix `/build/*`
  base: command === 'serve' ? '/' : '/build/',
  publicDir: false,
  assetsInclude: ['**/*.wasm'],
  build: {
    outDir: 'public/build',
    emptyOutDir: true,
    manifest: true,
    minify: 'esbuild',
    cssMinify: true,
    sourcemap: false,
    reportCompressedSize: false,
    target: 'es2020',
    rollupOptions: {
      input: 'src/views/_entry.tsx',
      output: {
        // 减少无意义的大 vendor 拆分开销；seal 仍按动态 import 独立
        manualChunks: undefined,
      },
    },
  },
  esbuild: {
    legalComments: 'none',
    minifyIdentifiers: true,
    minifySyntax: true,
    minifyWhitespace: true,
  },
  ssr: {
    noExternal: true,
  },
  server: {
    // 与 nx dev --vite-port / NAMIX_VITE_ORIGIN 对齐（默认 5173）
    origin: process.env.NAMIX_VITE_ORIGIN || 'http://127.0.0.1:5173',
    cors: true,
    strictPort: true,
  },
}))
