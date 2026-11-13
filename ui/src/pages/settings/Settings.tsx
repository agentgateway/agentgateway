import { Link, Outlet, useRouterState } from '@tanstack/react-router';
import { Menu } from 'antd';

export const settingsTabs = [
	{ to: '/settings/ui', label: 'UI' },
	{ to: '/settings/observability', label: 'Observability' }
] as const;

export function SettingsLayout() {
	const pathname = useRouterState({ select: state => state.location.pathname });
	const active = settingsTabs.find(tab => pathname.endsWith(tab.to));
	return (
		<div className="settings-layout">
			<nav className="settings-nav nav-list" aria-label="Settings">
				<Menu
					mode="inline"
					inlineIndent={9}
					selectedKeys={active ? [active.to] : []}
					items={[
						{
							type: 'group',
							key: 'settings',
							label: 'Settings',
							children: settingsTabs.map(tab => ({
								key: tab.to,
								label: <Link to={tab.to}>{tab.label}</Link>
							}))
						}
					]}
				/>
			</nav>
			<div className="settings-content">
				<Outlet />
			</div>
		</div>
	);
}
