import { loader } from '@monaco-editor/react';
import * as localMonaco from 'monaco-editor/editor/editor.api.js';
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker';
import YamlWorker from 'monaco-yaml/yaml.worker.js?worker';
import 'monaco-editor/editor/contrib/hover/browser/hoverContribution.js';
import 'monaco-editor/editor/contrib/suggest/browser/suggestController.js';
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
