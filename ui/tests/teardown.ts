import {
    existsSync,
    readdirSync,
    renameSync,
    rmSync,
    statSync,
    unlinkSync
} from 'fs';
import * as path from 'path';

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
  for (const entry of readdirSync(rootDir)) {
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
    const videoDest = path.join(rootDir, `${entry}.webm`);
    if (existsSync(videoDest)) {
      try {
        unlinkSync(videoDest);
      } catch {
        // overwrite race; the rename below will fail loudly if so
      }
    }
    try {
      renameSync(videoSrc, videoDest);
    } catch (e) {
      console.warn(`Could not flatten video for ${entry}:`, e);
      continue;
    }
    try {
      if (readdirSync(subPath).length === 0) rmSync(subPath, { recursive: false });
    } catch {
      // Leave the dir if it still has trace/screenshots etc.
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
