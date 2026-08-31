import { requestJson } from '@/api/base';
import type { AdminConfigDump } from '@/types';

export function getConfigDump() {
	return requestJson<AdminConfigDump>('/config_dump');
}
