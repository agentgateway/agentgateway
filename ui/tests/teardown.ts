import {
    copyFileSync,
    existsSync,
    mkdirSync,
    readdirSync,
    statSync,
} from 'fs';
import * as path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const RESULTS_DIR = path.join(__dirname, '..', 'test-results');

/**
 * Move each per-test `video.webm` up to a flat layout under `test-results/`,
 * named after the directory Playwright generated for that test (which already
 * encodes spec file + test title + project). Subdirectories that end up empty
 * after the move are removed; ones that still hold trace.zip / screenshots
 * are left intact.
 */
function flattenVideos(rootDir: string): void {
  if (!existsSync(rootDir)) return;
  const flatDir = path.join(rootDir, 'videos');
  for (const entry of readdirSync(rootDir)) {
    if (entry === 'videos') continue;
    const subPath = path.join(rootDir, entry);
    let s;
    try {
      s = statSync(subPath);
    } catch {
      continue;
    }
    if (!s.isDirectory()) continue;
    const videoSrc = path.join(subPath, 'video.webm');
    if (!existsSync(videoSrc)) continue;
    mkdirSync(flatDir, { recursive: true });
    const videoDest = path.join(flatDir, `${entry}.webm`);
    try {
      copyFileSync(videoSrc, videoDest);
    } catch (e) {
      console.warn(`Could not flatten video for ${entry}:`, e);
    }
  }
}

export default async function globalTeardown() {
  console.log('\n=== E2E UI Teardown: Stopping services ===\n');

  try {
    flattenVideos(RESULTS_DIR);
  } catch (e) {
    console.warn('Warning flattening video artifacts:', e);
  }

  console.log('\n=== E2E UI Teardown: Complete ===\n');
}
