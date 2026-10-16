import react from "@vitejs/plugin-react";
import { resolve } from "path";
import { defineConfig } from "vite";

const projectRoot = __dirname;
const repoRoot = resolve(projectRoot, "..");

export default defineConfig({
	plugins: [react()],
	root: "src/mainview",
	build: {
		outDir: "../../dist",
		emptyOutDir: true,
	},
	server: {
		port: 5173,
		strictPort: true,
		fs: {
			// The event catalog lives in packages/mirror-i18n and is shared with
			// the Rust backend. Vite's dev server refuses to read outside its
			// root by default, so the repository root is allowed explicitly.
			allow: [repoRoot],
		},
	},
	resolve: {
		alias: {
			"@": resolve(projectRoot, "src"),
			// The same file the mirror-i18n crate embeds with include_str!.
			// Aliased rather than copied, so the two cannot disagree.
			"@catalog": resolve(repoRoot, "packages/mirror-i18n/catalog"),
		},
	},
});
