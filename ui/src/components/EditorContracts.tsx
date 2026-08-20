import type { ReactNode } from 'react';

import { StatusBanner, YamlBlock } from '@/components/Primitives';
import { SchemaHelpPanel } from '@/components/SchemaHelpPanel';
import type { SchemaHelp } from '@/schemaHelp';

export function UnsupportedYamlFallback(props: {
	title: string;
	children: ReactNode;
	value: unknown;
	schema?: unknown;
	help?: SchemaHelp;
}) {
	return (
		<div className="policy-editor-stack">
			<StatusBanner state="warn" title={props.title}>
				{props.children}
			</StatusBanner>
			{props.schema && props.help ? (
				<SchemaHelpPanel schema={props.schema} help={props.help} />
			) : null}
			<YamlBlock value={props.value} />
		</div>
	);
}
