#!/usr/bin/env bun
import { copyFile, mkdir, rm, readdir } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const isRelease = process.argv.includes('--release');

const staticDir = join(__dirname, 'static');
const srcDir = join(__dirname, 'src');
const distDir = join(__dirname, 'dist');

console.log(`Building frontend (${isRelease ? 'release' : 'debug'} mode)...`);

// Clean dist directory
try {
  await rm(distDir, { recursive: true, force: true });
} catch (err) {
  // Directory might not exist
}
await mkdir(distDir, { recursive: true });

// Bundle the app.js with Bun
const buildResult = await Bun.build({
  entrypoints: [join(srcDir, 'app.js')],
  outdir: distDir,
  target: 'browser',
  format: 'esm',
  minify: isRelease,
  sourcemap: isRelease ? 'none' : 'inline',
  naming: {
    entry: 'app.js',
    chunk: '[name]-[hash].js',
    asset: '[name]-[hash].[ext]'
  }
});

if (!buildResult.success) {
  console.error('Build failed:');
  for (const log of buildResult.logs) {
    console.error(log);
  }
  process.exit(1);
}

console.log(`Bundled ${buildResult.outputs.length} file(s)`);

// Copy index.html from static
await copyFile(
  join(staticDir, 'index.html'),
  join(distDir, 'index.html')
);

// Copy styles.css from static (or we could generate it from Pico)
await copyFile(
  join(staticDir, 'styles.css'),
  join(distDir, 'styles.css')
);

// Copy Pico CSS
const picoSource = join(__dirname, 'node_modules/@picocss/pico/css/pico.min.css');
await copyFile(picoSource, join(distDir, 'pico.min.css'));

console.log('Frontend build complete!');
