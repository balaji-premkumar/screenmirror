#!/usr/bin/env bun
/**
 * Builds everything and stages it under `releases/v<version>/`.
 *
 * This replaces `package_release.sh`, which could not run on Windows and had
 * gone stale in a way that would have failed silently: it copied the backend
 * from `desktopApp/mirror_backend/target/release/`, but the crate is a
 * workspace member now, so Cargo writes to the workspace root's `target/`
 * instead. The `cp` would have failed and taken the release with it.
 *
 * Usage:
 *   bun run tools/package_release.ts             # build and stage
 *   bun run tools/package_release.ts --skip-build  # stage what is already built
 *
 * The version comes from `desktopApp/package.json` so there is one place to
 * change it.
 */

import { existsSync } from 'node:fs';
import { copyFile, mkdir, readFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const desktopRoot = join(repoRoot, 'desktopApp');
const mobileRoot = join(repoRoot, 'mobileApp');

const skipBuild = process.argv.includes('--skip-build');

type Platform = 'linux' | 'darwin' | 'win32';

const BACKEND: Record<Platform, string> = {
  linux: 'libmirror_backend.so',
  darwin: 'libmirror_backend.dylib',
  win32: 'mirror_backend.dll',
};

const PLUGIN: Record<Platform, string> = {
  linux: 'mirror-source.so',
  darwin: 'mirror-source.so',
  win32: 'mirror-source.dll',
};

function platform(): Platform {
  const p = process.platform;
  if (p === 'linux' || p === 'darwin' || p === 'win32') return p;
  throw new Error(`Releases are not built on "${p}".`);
}

async function version(): Promise<string> {
  const pkg = JSON.parse(await readFile(join(desktopRoot, 'package.json'), 'utf8'));
  return pkg.version as string;
}

async function run(command: string[], cwd: string) {
  console.log(`\n$ ${command.join(' ')}   (in ${cwd})`);
  const proc = Bun.spawn(command, { cwd, stdout: 'inherit', stderr: 'inherit' });
  const code = await proc.exited;
  if (code !== 0) {
    throw new Error(`"${command.join(' ')}" exited with ${code}`);
  }
}

async function stage(label: string, from: string[], to: string, required: boolean) {
  const source = from.find((candidate) => existsSync(candidate));
  if (!source) {
    if (required) throw new Error(`${label} not found. Looked in:\n  ${from.join('\n  ')}`);
    console.warn(`- ${label} not built; skipping.`);
    return;
  }
  await copyFile(source, to);
  console.log(`✓ ${label} → ${to}`);
}

async function main() {
  const os = platform();
  const releaseDir = join(repoRoot, 'releases', `v${await version()}`);

  for (const sub of ['desktop', 'mobile', 'obs-plugin']) {
    await mkdir(join(releaseDir, sub), { recursive: true });
  }

  if (!skipBuild) {
    await run(['bun', 'install'], desktopRoot);
    await run(['bun', 'run', 'build:all'], desktopRoot);
    await run(['flutter', 'build', 'apk', '--release'], mobileRoot);
  }

  await stage(
    'Rust backend',
    [join(repoRoot, 'target', 'release', BACKEND[os])],
    join(releaseDir, 'desktop', BACKEND[os]),
    true,
  );

  await stage(
    'Android APK',
    [join(mobileRoot, 'build', 'app', 'outputs', 'flutter-apk', 'app-release.apk')],
    join(releaseDir, 'mobile', 'mirror-companion.apk'),
    true,
  );

  const pluginBuild = join(desktopRoot, 'obs_plugin', 'build');
  const pluginCandidates = [
    join(pluginBuild, PLUGIN[os]),
    join(pluginBuild, 'Release', PLUGIN[os]),
    join(pluginBuild, 'RelWithDebInfo', PLUGIN[os]),
  ];

  await stage(
    'OBS plugin',
    pluginCandidates,
    join(releaseDir, 'obs-plugin', PLUGIN[os]),
    false,
  );

  // The desktop app installs the plugin out of its own bin/, so stage a copy
  // there as well. Without it the "Install OBS plugin" button has nothing to
  // install and falls back to compiling from source on the user's machine.
  const bundled = pluginCandidates.find((candidate) => existsSync(candidate));
  if (bundled) {
    await mkdir(join(desktopRoot, 'bin'), { recursive: true });
    await copyFile(bundled, join(desktopRoot, 'bin', PLUGIN[os]));
  }

  console.log(`\nRelease staged in ${releaseDir}`);
}

await main();
