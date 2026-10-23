import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import type { JSONReport, JSONReportSuite } from '@playwright/test/reporter';

import { routes } from './pages.ts';

export function createReport(output: string) {
	const report: JSONReport = JSON.parse(readFileSync(join(output, 'current-results.json'), 'utf8'));
	const specs = (suites: JSONReportSuite[]): JSONReportSuite['specs'] =>
		suites.flatMap(suite => [...suite.specs, ...specs(suite.suites ?? [])]);
	const cases = specs(report.suites);
	const rows = routes.flatMap(([name, route, title]) =>
		['light', 'dark'].map(theme => {
			const test = cases
				.filter(spec => spec.title === `visual baseline: ${title}`)
				.flatMap(spec => spec.tests)
				.find(item => item.projectName === theme);
			const result = test?.results.at(-1);
			const before = `reference/${theme}/${name}-full-page.png`;
			const after = `current/${theme}/${name}-full-page.png`;
			const attachments = result?.attachments.map(item => item.name) ?? [];
			const mismatch =
				result?.status === 'failed' &&
				['expected', 'actual', 'diff'].every(kind =>
					attachments.includes(`${name}-full-page-${kind}.png`)
				) &&
				!attachments.includes(`${name}-full-page-previous.png`) &&
				result.errors.length === 1 &&
				!/^Timeout:/m.test(result.errors[0].message ?? '');
			const complete =
				(result?.status === 'passed' || mismatch) &&
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
				status: !complete ? 'missing' : result?.status === 'passed' ? 'matched' : 'failed',
				exactBytes:
					complete && readFileSync(join(output, before)).equals(readFileSync(join(output, after))),
				errors: result?.errors.map(error => error.message) ?? ['No test result']
			};
		})
	);
	const source = JSON.parse(readFileSync(join(output, 'source.json'), 'utf8'));
	const data = {
		source,
		rows,
		matched: rows.filter(row => row.status === 'matched').length,
		total: rows.length
	};
	writeFileSync(join(output, 'results.json'), `${JSON.stringify(data, null, 2)}\n`);
	const template = readFileSync(join(import.meta.dirname, 'report.html'), 'utf8');
	writeFileSync(
		join(output, 'index.html'),
		template.replace('__COMPARISON_DATA__', JSON.stringify(data).replaceAll('<', '\\u003c'))
	);
	return !report.errors?.length && rows.every(row => row.complete);
}

export async function renderFrames(output: string) {
	const { rows } = JSON.parse(readFileSync(join(output, 'results.json'), 'utf8'));
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
