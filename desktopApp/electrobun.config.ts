import type { ElectrobunConfig } from "electrobun";

export default {
	app: {
		name: "mirror-receiver-enterprise",
		identifier: "stream.mirror.enterprise",
		version: "1.0.0",
	},
	build: {
		copy: {
			"dist/index.html": "views/mainview/index.html",
			"dist/assets": "views/mainview/assets",
			"bin": "bin",
		},
		watchIgnore: ["dist/**"],
		mac: {
			bundleCEF: false,
		},
		linux: {
			bundleCEF: false, // Fallback to system WebKit for stability on this machine
		},
		win: {
			bundleCEF: false,
		},
	},
} satisfies ElectrobunConfig;
