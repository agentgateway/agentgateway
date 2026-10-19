import { PolicyCatalogPage } from '@/pages/Policies';
import type { PolicyKey } from '@/policies/types';

const observabilityKeys: PolicyKey[] = ['tracing', 'accessLog'];

export function ObservabilitySettingsPage() {
	return (
		<PolicyCatalogPage
			title="Observability"
			description="Configure tracing and access logs for all traffic."
			schemaRoot="LocalFrontendPolicies"
			resourceKind="frontend.policy"
			sections={[{ title: 'Frontend policies', keys: observabilityKeys }]}
			policyKeys={observabilityKeys}
			yamlDescription="Read-only view of effective frontend policies."
			policies={config =>
				config.data?.frontendPolicies as Record<string, unknown> | null | undefined
			}
			onSavePolicy={(next, key, value) => {
				next.frontendPolicies ??= {};
				(next.frontendPolicies as Record<string, unknown>)[key] = value;
			}}
			onDisablePolicy={(next, key) => {
				if (next.frontendPolicies) {
					delete (next.frontendPolicies as Record<string, unknown>)[key];
				}
			}}
		/>
	);
}
