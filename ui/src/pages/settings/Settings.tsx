import { Link, Outlet } from '@tanstack/react-router';

export const settingsTabs = [
	{ to: '/settings/ui', label: 'UI' },
	{ to: '/settings/observability', label: 'Observability' }
] as const;

export function SettingsLayout() {
	return (
		<div className="settings-layout">
			<nav className="settings-nav" aria-label="Settings">
				<div className="nav-section">Settings</div>
				{settingsTabs.map(tab => (
					<Link key={tab.to} to={tab.to} className="nav-item" activeProps={{ className: 'active' }}>
						{tab.label}
					</Link>
				))}
			</nav>
			<div className="settings-content">
				<Outlet />
			</div>
		</div>
	);
}
