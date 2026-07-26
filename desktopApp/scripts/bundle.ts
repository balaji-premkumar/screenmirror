#!/usr/bin/env bun
/**
 * Copies the freshly built native libraries into `bin/`, where the packaged
 * app looks for them.
 *
 * This replaces `bundle.sh`. The shell version only ran on a POSIX shell, so
 * the Windows half of the build matrix had no way to bundle anything — it
 * built `mirror_backend.dll` and then left it in `target/release`. Bun is
 * already required to build this app, so a Bun script runs everywhere the
 * build does.
 */

import { copyFile, mkdir } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = dirname(fileURLToPath(import.meta.url));
const desktopRoot = resolve(scriptDir, '..');
const repoRoot = resolve(desktopRoot, '..');
const binDir = join(desktopRoot, 'bin');

/** Cargo puts workspace output in the workspace root's `target/`, not the crate's. */
const cargoRelease = join(repoRoot, 'target', 'release');

/** What the Rust backend is called on each platform. */
const BACKEND_NAME: Record<string, string> = {
  linux: 'libmirror_backend.so',
  darwin: 'libmirror_backend.dylib',
  win32: 'mirror_backend.dll',
};

/** What the OBS plugin is called on each platform. */
const PLUGIN_NAME: Record<string, string> = {
  linux: 'mirror-source.so',
  darwin: 'mirror-source.so',
  win32: 'mirror-source.dll',
};

interface Artifact {
  label: string;
  /** Candidate paths, tried in order. */
  from: string[];
  to: string;
  /** Whether the build should fail if none of the candidates exist. */
  required: boolean;
}

function artifacts(): Artifact[] {
  const platform = process.platform;
  const backend = BACKEND_NAME[platform];
  const plugin = PLUGIN_NAME[platform];

  if (!backend) {
    throw new Error(`No native backend is built for platform "${platform}".`);
  }

  const pluginBuild = join(desktopRoot, 'obs_plugin', 'build');

  return [
    {
      label: 'Rust backend',
      from: [join(cargoRelease, backend)],
      to: join(binDir, backend),
      required: true,
    },
    {
      label: 'OBS plugin',
      // Single-config generators put it directly in build/; MSVC adds a
      // per-configuration subdirectory.
      from: [
        join(pluginBuild, plugin),
        join(pluginBuild, 'Release', plugin),
        join(pluginBuild, 'RelWithDebInfo', plugin),
      ],
      to: join(binDir, plugin),
      // The OBS plugin is optional: the app runs fine without OBS installed,
      // and requiring libobs headers to build the desktop app at all would be
      // a needless barrier.
      required: false,
    },
  ];
}

async function main() {
  await mkdir(binDir, { recursive: true });

  let missingRequired = false;

  for (const artifact of artifacts()) {
    const source = artifact.from.find((candidate) => existsSync(candidate));

    if (!source) {
      if (artifact.required) {
        console.error(`✗ ${artifact.label} not found. Looked in:`);
        artifact.from.forEach((candidate) => console.error(`    ${candidate}`));
        missingRequired = true;
      } else {
        console.warn(`- ${artifact.label} not built; skipping.`);
      }
      continue;
    }

    await copyFile(source, artifact.to);
    console.log(`✓ ${artifact.label} → ${artifact.to}`);
  }

  if (missingRequired) {
    console.error('\nRun `bun run build:rust` first.');
    process.exit(1);
  }
}

await main();
