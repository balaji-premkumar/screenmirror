/** @type {import('tailwindcss').Config} */

// Colours are CSS variables holding bare RGB channels (see index.css), so one
// class works in both themes and still takes an opacity: `bg-signal/10`.
const token = (name) => `rgb(var(--${name}) / <alpha-value>)`;

export default {
	content: [
		"./src/mainview/**/*.{html,js,ts,jsx,tsx}",
		"./src/features/**/*.{html,js,ts,jsx,tsx}",
		"./src/components/**/*.{html,js,ts,jsx,tsx}"
	],
	theme: {
		extend: {
			colors: {
				paper: token("paper"),
				panel: token("panel"),
				line: token("line"),
				ink: token("ink"),
				muted: token("muted"),
				signal: token("signal"),
				"signal-ink": token("signal-ink"),
				good: token("good"),
				warn: token("warn"),
				bad: token("bad"),
			},
			fontFamily: {
				sans: ['"Schibsted Grotesk Variable"', "system-ui", "sans-serif"],
			},
		},
	},
	plugins: [],
};
