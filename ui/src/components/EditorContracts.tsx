import { createContext, Fragment, useContext, useLayoutEffect, useState } from 'react';

import { PinRawContext, StatusBanner } from '@/components/Primitives';
import { withoutSchemaDefaults } from '@/configValidation';
import { mergeEdits } from '@/mergeEdits';

// Rendered by a form that cannot represent its value: the drawer switches to
// raw mode, with the form disabled and `supports` explaining why.
export function UnsupportedInForm(props: { supports: string }) {
	const pin = useContext(PinRawContext);
	useLayoutEffect(() => {
		pin?.(`The form editor only supports ${props.supports}, so this is edited as raw YAML.`);
	}, [pin, props.supports]);
	return null;
}

// Receives an editor's current value, so a drawer can read it without
// submitting the form (and so without the editor's save validation).
export const EditorValueContext = createContext<((value: unknown) => void) | null>(null);

// The config schema path of the value an editor edits (see useMergedEdits).
export const EditorSchemaContext = createContext<string[] | null>(null);

export function useReportEditorValue(value: unknown) {
	useContext(EditorValueContext)?.(value);
}

// Structured editors save through this: `edited` is the value rebuilt from form
// state, and the first one seen is the editor's baseline. Returns the edit
// applied onto `original` (see mergeEdits) and any paths it would replace, so
// only fields the user changed are written. A new value has no original: its
// starting form output minus schema defaults (`schemaPath`, or
// EditorSchemaContext) stands in, so defaults nobody touched are left out; the
// same goes for fields and list entries the form adds later.
// Pass `edited` before any cleanup that drops modeled fields (e.g. implied
// defaults) and apply that cleanup to the result. `reset` makes a value loaded
// into the form (e.g. from raw mode) the new original, or with no value starts
// a new one from the form's next output.
export function useMergedEdits<T>(original: unknown, edited: T, schemaPath?: string[]) {
	const contextPath = useContext(EditorSchemaContext);
	const path = schemaPath ?? contextPath;
	const start = (value: unknown, first: T) =>
		value ?? (path ? withoutSchemaDefaults(first, path) : undefined);
	const [base, setBase] = useState<{ original: unknown; baseline: { value: T } | null }>(() => ({
		original: start(original, edited),
		baseline: { value: edited }
	}));
	useLayoutEffect(() => {
		if (!base.baseline)
			setBase({ original: start(base.original, edited), baseline: { value: edited } });
	});
	const reset = (value: unknown) => setBase({ original: value, baseline: null });
	const baseline = base.baseline ? base.baseline.value : edited;
	const merged =
		base.original === null || base.original === undefined
			? { value: edited, lost: [] as string[] }
			: mergeEdits(base.original, baseline, edited, (value, at) =>
					path ? withoutSchemaDefaults(value, path, at) : value
				);
	useReportEditorValue(merged.value);
	return { value: merged.value as T, lost: merged.lost, reset };
}

export function LostFieldsWarning(props: { lost: string[] }) {
	if (!props.lost.length) return null;
	const paths = props.lost.filter(path => path !== '.');
	return (
		<StatusBanner state="warn" title="This form cannot show all of this configuration">
			Saving will overwrite{' '}
			{paths.length === props.lost.length
				? paths.map((path, index) => (
						<Fragment key={path}>
							{index ? ', ' : null}
							<code>{path}</code>
						</Fragment>
					))
				: 'parts of it'}
			. Edit it as raw YAML (the code icon) to keep them.
		</StatusBanner>
	);
}
