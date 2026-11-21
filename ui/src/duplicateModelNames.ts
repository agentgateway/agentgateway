const duplicateModelNamesPrefix = 'Duplicate model names: ';

export const duplicateModelNamesDocsUrl =
	'https://agentgateway.dev/docs/standalone/latest/documentation/llm/about/#combine-patterns';

export function duplicateModelNamesWarning(names: string[]): string {
	return `${duplicateModelNamesPrefix}${Array.from(new Set(names)).join(', ')}. If entries with the same name match a request, the first one listed is used. Set a unique id on each entry to clear this warning.`;
}

export function isDuplicateModelNamesWarning(warning: string): boolean {
	return warning.startsWith(duplicateModelNamesPrefix);
}
