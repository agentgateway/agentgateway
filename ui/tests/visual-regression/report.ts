import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import type { JSONReport, JSONReportSuite, JSONReportTestResult } from '@playwright/test/reporter';

import { routes, testTitle } from './pages.ts';

// A failed capture counts only as a stable screenshot difference, never as a timeout or retry.
function classify(name: string, result: JSONReportTestResult | undefined) {
	if (result?.status === 'passed') return 'matched';
	const attachments = result?.attachments.map(item => item.name) ?? [];
	const stableDifference =
		result?.status === 'failed' &&
		['expected', 'actual', 'diff'].every(kind =>
			attachments.includes(`${name}-full-page-${kind}.png`)
		) &&
		!attachments.includes(`${name}-full-page-previous.png`) &&
		result.errors.length === 1 &&
		!/^Timeout:/m.test(result.errors[0].message ?? '');
	return stableDifference ? 'failed' : 'missing';
}

// Shapes of the JSON files that the host, container and report pass between processes.
export type ComparisonSource = {
	renderer: string;
	reference: { ref: string; commit: string };
	candidate: { commit: string; branch: string; dirty: boolean; sha256: string };
	media: { video: boolean; gif: boolean };
	runtime?: { node: string; architecture: string; playwright: string };
};

export type ComparisonRow = {
	name: string;
	route: string;
	title: string;
	theme: string;
	before: string;
	after: string;
	diff?: string;
	complete: boolean;
	status: 'matched' | 'failed' | 'missing';
	exactBytes: boolean;
	errors: string[];
};

export type ComparisonResults = {
	source: ComparisonSource;
	rows: ComparisonRow[];
	matched: number;
	total: number;
};

export function writeIndex(output: string, data: unknown) {
	const template = readFileSync(join(import.meta.dirname, 'report.html'), 'utf8');
	const json = JSON.stringify(data).replaceAll('<', '\\u003c');
	// A replacer function keeps `$` sequences in branch names and errors literal.
	writeFileSync(
		join(output, 'index.html'),
		template.replace('__COMPARISON_DATA__', () => json)
	);
}

export function createReport(output: string) {
	const report: JSONReport = JSON.parse(readFileSync(join(output, 'current-results.json'), 'utf8'));
	const specs = (suites: JSONReportSuite[]): JSONReportSuite['specs'] =>
		suites.flatMap(suite => [...suite.specs, ...specs(suite.suites ?? [])]);
	const cases = specs(report.suites);
	const rows = routes.flatMap(([name, route, title]) =>
		['light', 'dark'].map((theme): ComparisonRow => {
			const test = cases
				.filter(spec => spec.title === testTitle(title))
				.flatMap(spec => spec.tests)
				.find(item => item.projectName === theme);
			const result = test?.results.at(-1);
			const before = `reference/${theme}/${name}-full-page.png`;
			const after = `current/${theme}/${name}-full-page.png`;
			const outcome = classify(name, result);
			const complete =
				outcome !== 'missing' &&
				existsSync(join(output, before)) &&
				existsSync(join(output, after));
			const attachment = result?.attachments.find(item => item.path?.endsWith('-diff.png'));
			let diff: string | undefined;
			if (attachment?.path && resolve(attachment.path).startsWith(`${resolve(output)}/`)) {
				diff = `diff/${theme}/${name}.png`;
				mkdirSync(join(output, 'diff', theme), { recursive: true });
				copyFileSync(attachment.path, join(output, diff));
			}
			return {
				name,
				route,
				title,
				theme,
				before,
				after,
				diff,
				complete,
				status: complete ? outcome : 'missing',
				exactBytes:
					complete && readFileSync(join(output, before)).equals(readFileSync(join(output, after))),
				errors: result?.errors.map(error => error.message) ?? ['No test result']
			};
		})
	);
	const source: ComparisonSource = JSON.parse(readFileSync(join(output, 'source.json'), 'utf8'));
	const data: ComparisonResults = {
		source,
		rows,
		matched: rows.filter(row => row.status === 'matched').length,
		total: rows.length
	};
	writeFileSync(join(output, 'results.json'), `${JSON.stringify(data, null, 2)}\n`);
	writeIndex(output, data);
	return !report.errors?.length && rows.every(row => row.complete);
}

export async function renderFrames(output: string) {
	const { rows }: ComparisonResults = JSON.parse(
		readFileSync(join(output, 'results.json'), 'utf8')
	);
	const frames = join(output, 'frames');
	mkdirSync(frames, { recursive: true });
	const browser = await chromium.launch();
	const durations: string[] = [];
	let index = 0;
	try {
		const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
		await page.goto(pathToFileURL(join(output, 'index.html')).href);
		for (const row of rows) {
			await page.selectOption('#page', row.name);
			await page.selectOption('#theme', row.theme);
			await page.waitForFunction(() =>
				[...document.images].every(image => image.complete && image.naturalWidth > 0)
			);
			const positions = await page
				.locator('.viewport')
				.evaluateAll(elements =>
					elements.some(element => element.scrollHeight > element.clientHeight + 10) ? [0, 1] : [0]
				);
			for (const position of positions) {
				await page.locator('.viewport').evaluateAll((elements, fraction) => {
					for (const element of elements)
						element.scrollTop = (element.scrollHeight - element.clientHeight) * fraction;
				}, position);
				const name = `${String(index++).padStart(4, '0')}.png`;
				await page.screenshot({ path: join(frames, name) });
				durations.push(`file 'frames/${name}'\nduration 2\n`);
			}
		}
		writeFileSync(
			join(output, 'frames.txt'),
			`${durations.join('')}file 'frames/${String(index - 1).padStart(4, '0')}.png'\n`
		);
	} finally {
		await browser.close();
	}
}
