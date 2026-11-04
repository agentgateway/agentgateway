import { fileURLToPath, URL } from 'node:url';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig(({ command, mode }) => ({
	// Prod builds use relative asset paths and let the server's <base> decide where the UI lives.
	base: mode === 'e2e' ? '/' : command === 'build' ? './' : '/ui/',
	plugins: [
		react(),
		{
			name: 'ui-trailing-slash',
			configureServer(server) {
				server.middlewares.use((req, res, next) => {
					if (req.url !== '/ui') return next();
					res.writeHead(302, { Location: '/ui/' }).end();
				});
			}
		}
	],
	resolve: {
		alias: {
			'@': fileURLToPath(new URL('./src', import.meta.url)),
			'monaco-editor/esm/vs/editor/editor.worker.js': 'monaco-editor/editor/editor.worker.js',
			'path-browserify': fileURLToPath(new URL('./src/pathBrowserifyEsm.ts', import.meta.url))
		}
	},
	build: {
		emptyOutDir: true,
		reportCompressedSize: false,
		sourcemap: false,
		chunkSizeWarningLimit: 5000
	},
	server: {
		host: '0.0.0.0',
		port: 19000
	},
	preview: {
		host: mode === 'e2e' ? '127.0.0.1' : '0.0.0.0',
		port: 19100
	}
}));
