import '@/monacoWorkers';
import Editor from '@monaco-editor/react';
import { dump, load } from 'js-yaml';
import { ExternalLink, Play } from 'lucide-react';
import type * as Monaco from 'monaco-editor';
import { useCallback, useEffect, useRef, useState } from 'react';

import { evaluateCel } from '@/api/celApi';
import { celEditorOptions, celLanguage, configureCelMonaco } from '@/celMonaco';
import { FieldGroup, PageHeader, Panel, StatusBanner, YamlBlock } from '@/components/Primitives';
import { configureConfigYamlMonaco } from '@/configMonaco';
import sampleContext from '@/pages/celSampleContext.json';
import { pendingCelExpression } from '@/policies/AuthorizationPolicyEditor';

export function CelPage() {
	const [expression, setExpression] = useState(
		() => pendingCelExpression() ?? 'request.path.startsWith("/v1/") && metadata.tier == "prod"'
	);
	const [context, setContext] = useState(dump(sampleContext));
	const [result, setResult] = useState<unknown>(null);
	const [hasResult, setHasResult] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);
	const runRef = useRef<() => void>(() => {});

	const run = useCallback(async () => {
		if (loading) return;
		setLoading(true);
		setError(null);
		try {
			const data = context.trim() ? load(context) : {};
			const response = await evaluateCel(expression, data);
			if (response.error) setError(response.error);
			setResult(response.result);
			setHasResult(true);
		} catch (err) {
			setError(err instanceof Error ? err.message : 'Evaluation failed');
		} finally {
			setLoading(false);
		}
	}, [context, expression, loading]);

	useEffect(() => {
		runRef.current = () => {
			void run();
		};
	}, [run]);

	function registerRunShortcut(editor: Monaco.editor.IStandaloneCodeEditor, monaco: typeof Monaco) {
		editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter, () => runRef.current());
	}

	return (
		<div className="page-stack">
			<PageHeader
				title="CEL Playground"
				description="Evaluate policy expressions against a sample or custom request."
				actions={
					<>
						<a
							className="button"
							href="https://agentgateway.dev/docs/standalone/latest/reference/cel/"
							rel="noreferrer"
							target="_blank"
						>
							<ExternalLink size={16} /> CEL reference
						</a>
						<button className="button primary" type="button" disabled={loading} onClick={run}>
							<Play size={16} />
							Evaluate
						</button>
					</>
				}
			/>
			{error ? (
				<StatusBanner state="bad" title="CEL error">
					{error}
				</StatusBanner>
			) : null}
			<section className="two-column wide-left">
				<Panel>
					<FieldGroup label="Expression">
						<div className="editor-wrap short">
							<Editor
								beforeMount={configureCelMonaco}
								language={celLanguage}
								theme={document.documentElement.dataset.theme === 'dark' ? 'vs-dark' : 'light'}
								value={expression}
								onChange={value => setExpression(value ?? '')}
								onMount={registerRunShortcut}
								options={{
									...celEditorOptions,
									lineNumbers: 'off'
								}}
							/>
						</div>
					</FieldGroup>
					<FieldGroup label="Request context YAML">
						<div className="editor-wrap">
							<Editor
								beforeMount={configureConfigYamlMonaco}
								language="yaml"
								path="cel-request-context.yaml"
								theme={document.documentElement.dataset.theme === 'dark' ? 'vs-dark' : 'light'}
								value={context}
								onChange={value => setContext(value ?? '')}
								onMount={registerRunShortcut}
								options={{
									minimap: { enabled: false },
									fontSize: 13,
									renderLineHighlight: 'none',
									scrollBeyondLastLine: false,
									copyWithSyntaxHighlighting: false
								}}
							/>
						</div>
					</FieldGroup>
				</Panel>
				<Panel>
					<div className="section-heading">
						<h3>Result</h3>
						<p>YAML value returned by CEL evaluation.</p>
					</div>
					{hasResult ? <YamlBlock value={result ?? null} /> : null}
				</Panel>
			</section>
		</div>
	);
}

declare global {
	interface Window {
		monaco?: typeof import('monaco-editor');
	}
}
