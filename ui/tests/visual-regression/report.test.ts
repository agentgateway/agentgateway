import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { routes } from './pages.ts';
import { createReport, renderFrames } from './report.ts';

test('only stable screenshot differences complete a failed comparison', () => {
	const output = mkdtempSync(join(tmpdir(), 'visual-capture-failure-'));
	try {
		for (const side of ['reference', 'current']) {
			for (const theme of ['light', 'dark']) {
				mkdirSync(join(output, side, theme), { recursive: true });
				for (const [name] of routes)
					writeFileSync(join(output, side, theme, `${name}-full-page.png`), 'capture');
			}
		}
		writeFileSync(join(output, 'source.json'), '{}');
		for (const scenario of [
			{ attachments: [], message: 'Screenshot failed', complete: false },
			{
				attachments: ['expected', 'actual', 'diff', 'previous'],
				message: 'Failed to take two consecutive stable screenshots.',
				complete: false
			},
			{ attachments: ['expected', 'actual', 'diff'], message: 'Timeout: 5000ms', complete: false },
			{ attachments: ['expected', 'actual', 'diff'], message: 'Pixels differ.', complete: true }
		]) {
			const suites = ['light', 'dark'].map(projectName => ({
				specs: routes.map(([name, , title]) => ({
					title: `visual baseline: ${title}`,
					tests: [
						{
							projectName,
							results: [
								{
									status: name === 'cel' ? 'failed' : 'passed',
									attachments:
										name === 'cel'
											? scenario.attachments.map(kind => ({
													name: `cel-full-page-${kind}.png`,
													contentType: 'image/png'
												}))
											: [],
									errors: name === 'cel' ? [{ message: scenario.message }] : []
								}
							]
						}
					]
				}))
			}));
			writeFileSync(join(output, 'current-results.json'), JSON.stringify({ suites }));
			assert.equal(createReport(output), scenario.complete, scenario.message);
		}
	} finally {
		rmSync(output, { recursive: true, force: true });
	}
});

test('media includes the bottom when only the candidate overflows', async () => {
	const output = mkdtempSync(join(tmpdir(), 'visual-media-overflow-'));
	try {
		const data = {
			source: {
				reference: { ref: 'main', commit: '123456789abc' },
				candidate: { branch: 'test', commit: '123456789abc', dirty: true }
			},
			matched: 0,
			total: 1,
			rows: [
				{
					name: 'overview',
					title: 'Tall candidate',
					theme: 'light',
					route: '/',
					before: 'before.svg',
					after: 'after.svg',
					complete: true,
					status: 'failed',
					errors: []
				}
			]
		};
		for (const [name, height] of [
			['before', 200],
			['after', 2000]
		] as const)
			writeFileSync(
				join(output, `${name}.svg`),
				`<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="${height}"><rect width="1280" height="${height}" fill="white"/><text x="20" y="${height - 20}" font-size="40">${name} bottom</text></svg>`
			);
		writeFileSync(join(output, 'results.json'), JSON.stringify(data));
		writeFileSync(
			join(output, 'index.html'),
			readFileSync(join(import.meta.dirname, 'report.html'), 'utf8').replace(
				'__COMPARISON_DATA__',
				JSON.stringify(data)
			)
		);
		await renderFrames(output);
		assert.equal(readdirSync(join(output, 'frames')).length, 2);
		assert.equal(
			readFileSync(join(output, 'frames/0000.png')).equals(
				readFileSync(join(output, 'frames/0001.png'))
			),
			false
		);
	} finally {
		rmSync(output, { recursive: true, force: true });
	}
});

test('images without test results cannot complete a comparison', () => {
	const output = mkdtempSync(join(tmpdir(), 'visual-incomplete-'));
	try {
		for (const side of ['reference', 'current']) {
			for (const theme of ['light', 'dark']) {
				mkdirSync(join(output, side, theme), { recursive: true });
				for (const [name] of routes)
					writeFileSync(join(output, side, theme, `${name}-full-page.png`), 'capture');
			}
		}
		writeFileSync(join(output, 'current-results.json'), JSON.stringify({ suites: [] }));
		writeFileSync(join(output, 'source.json'), '{}');
		assert.equal(createReport(output), false);
	} finally {
		rmSync(output, { recursive: true, force: true });
	}
});

test('report keeps results from separate light and dark project suites', () => {
	const output = mkdtempSync(join(tmpdir(), 'visual-report-'));
	try {
		const suites = ['light', 'dark'].map(projectName => {
			for (const side of ['reference', 'current']) {
				mkdirSync(join(output, side, projectName), { recursive: true });
				writeFileSync(join(output, side, projectName, 'cel-full-page.png'), 'same capture');
			}
			return {
				specs: [
					{
						title: 'visual baseline: CEL Playground',
						tests: [{ projectName, results: [{ status: 'passed', attachments: [], errors: [] }] }]
					}
				]
			};
		});
		writeFileSync(join(output, 'current-results.json'), JSON.stringify({ suites }));
		writeFileSync(join(output, 'source.json'), '{}');
		assert.equal(createReport(output), false, 'uncaptured pages keep the report incomplete');
		const result = JSON.parse(readFileSync(join(output, 'results.json'), 'utf8'));
		assert.equal(result.matched, 2);
		assert.deepEqual(
			result.rows
				.filter((row: { name: string }) => row.name === 'cel')
				.map((row: { status: string }) => row.status),
			['matched', 'matched']
		);
	} finally {
		rmSync(output, { recursive: true, force: true });
	}
});
