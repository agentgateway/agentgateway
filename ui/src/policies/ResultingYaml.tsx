import { useTranslation } from 'react-i18next';

import { YamlBlock } from '@/components/Primitives';

export function ResultingYaml(props: { value: unknown; label?: string }) {
	const { t } = useTranslation();
	return (
		<details className="schema-details">
			<summary>{props.label ?? t('copy.resultingYaml')}</summary>
			<YamlBlock value={props.value} />
		</details>
	);
}
