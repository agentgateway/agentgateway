import { loader } from '@monaco-editor/react';
import * as localMonaco from 'monaco-editor/editor/editor.api.js';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import YamlWorker from 'monaco-yaml/yaml.worker.js?worker';
import 'monaco-editor/editor/contrib/hover/browser/hoverContribution.js';
import 'monaco-editor/editor/contrib/suggest/browser/suggestController.js';
import 'monaco-editor/editor/contrib/wordOperations/browser/wordOperations.js';
import 'monaco-editor/languages/definitions/yaml/register.js';

let workersConfigured = false;

export function configureConfigMonacoWorkers() {
	if (workersConfigured) return;
	workersConfigured = true;

	globalThis.MonacoEnvironment = {
		getWorker(_moduleId: string, label: string) {
			if (label === 'yaml') {
				return new YamlWorker();
			}
			return new EditorWorker();
		}
	};
}

loader.config({ monaco: localMonaco });
configureConfigMonacoWorkers();

// Editor themes on the app's surface colours (styles.css --surface / --surface-2 / --line).
localMonaco.editor.defineTheme('agw-light', {
	base: 'vs',
	inherit: true,
	rules: [],
	colors: {
		'editor.background': '#ffffff',
		'editorGutter.background': '#ffffff',
		'editor.lineHighlightBackground': '#f3f4f6',
		'editor.lineHighlightBorder': '#00000000',
		'editorWidget.background': '#ffffff',
		'editorWidget.border': '#e5e7eb'
	}
});
localMonaco.editor.defineTheme('agw-dark', {
	base: 'vs-dark',
	inherit: true,
	rules: [],
	colors: {
		'editor.background': '#10141f',
		'editorGutter.background': '#10141f',
		'editor.lineHighlightBackground': '#1c2029',
		'editor.lineHighlightBorder': '#00000000',
		'editorWidget.background': '#1c2029',
		'editorWidget.border': '#1f2937'
	}
});

export const monacoTheme = (mode: 'light' | 'dark') => (mode === 'dark' ? 'agw-dark' : 'agw-light');
