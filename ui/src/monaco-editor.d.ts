interface Window {
	__rawConfigEditor?: import('monaco-editor').editor.IStandaloneCodeEditor;
	__rawConfigMonaco?: typeof import('monaco-editor');
}

declare module 'monaco-editor/esm/vs/editor/editor.main' {
	export * from 'monaco-editor';
}
