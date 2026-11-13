import { Link, Outlet, useRouterState } from '@tanstack/react-router';
import { Breadcrumb, Button, Dropdown, Layout, Menu, type MenuProps } from 'antd';
import {
	BarChart3,
	Bolt,
	Bot,
	Boxes,
	Braces,
	Cable,
	ChevronDown,
	ChevronRight,
	Coins,
	FileCode2,
	GitFork,
	Globe,
	Home,
	KeyRound,
	LogOut,
	Menu as MenuIcon,
	MessageSquarePlus,
	Moon,
	Network,
	Play,
	Route,
	ScrollText,
	Server,
	Shield,
	ShieldCheck,
	SlidersHorizontal,
	Sun,
	UserRound
} from 'lucide-react';
import { useRef, useState } from 'react';

import { apiBase } from '@/api/base';
import type { RuntimeUser } from '@/api/runtimeApi';
import logoDark from '@/assets/agw-dark.svg';
import logoLight from '@/assets/agw-light.svg';
import { StatusBanner, Tooltip, useDismissiblePopover } from '@/components/Primitives';
import {
	useConfigDumpMode,
	useEffectiveGatewayConfig,
	useMcpConfigData,
	useRuntimeInfo,
	useTrafficConfigData
} from '@/hooks';
import { useTheme } from '@/theme';

type NavItemConfig = {
	to: string;
	label: string;
	icon: React.ComponentType<{ size?: number }>;
	placeholder?: boolean;
	groupStart?: boolean;
	exact?: boolean;
};

const projectLinks = [
	{
		label: 'GitHub',
		href: 'https://github.com/agentgateway/agentgateway',
		icon: GitFork
	},
	{
		label: 'Documentation',
		href: 'https://agentgateway.dev/docs/standalone/latest/',
		icon: Globe
	},
	{
		label: 'Feedback',
		href: 'https://github.com/agentgateway/agentgateway/issues/new?title=UI%20feedback%3A%20&body=Thanks%20for%20trying%20the%20agentgateway%20UI.%0A%0AWhat%20happened%3F%0A%0AWhat%20did%20you%20expect%20instead%3F%0A%0AAny%20screenshots%2C%20logs%2C%20or%20config%20that%20would%20help%3F',
		icon: MessageSquarePlus
	}
] as const;

export function Shell() {
	const router = useRouterState();
	const runtime = useRuntimeInfo();
	const mode = useConfigDumpMode();
	const dumpMode = mode.data?.mode === 'dump';
	const config = useEffectiveGatewayConfig({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const mcpData = useMcpConfigData({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const trafficData = useTrafficConfigData({
		enabled: Boolean(mode.data && mode.data.mode !== 'dump')
	});
	const { theme, toggleTheme } = useTheme();
	const hasLlm = dumpMode ? false : config.data ? Boolean(config.data.llm) : true;
	const hasMcp = dumpMode ? false : mcpData.data ? Boolean(mcpData.data.mcp) : true;
	const hasTraffic = dumpMode
		? true
		: trafficData.data
			? Boolean(trafficData.data.binds?.length) ||
				'gateways' in trafficData.data ||
				'routes' in trafficData.data ||
				'tcpRoutes' in trafficData.data
			: true;
	const hasBinds = dumpMode ? true : config.data ? Boolean(config.data.binds?.length) : false;
	const navGroups = navigationGroups({
		hasLlm,
		hasMcp,
		hasTraffic,
		hasBinds,
		dumpMode
	});
	const nav = navGroups.flatMap(group => group.items);
	const matchedNav = nav
		.filter(item => navItemActive(item, router.location.pathname))
		.sort((left, right) => right.to.length - left.to.length)[0];
	const currentNav = matchedNav ?? nav[0];
	const currentGroup = navGroups.find(group => group.items.includes(currentNav));
	const CurrentIcon = currentNav.icon;
	const selectedKeys = matchedNav && !matchedNav.placeholder ? [matchedNav.to] : [];
	const menuItems: MenuProps['items'] = navGroups.map(group => ({
		type: 'group',
		key: group.title,
		label: group.title,
		children: group.items.map(item => {
			const Icon = item.icon;
			return {
				key: item.to,
				icon: <Icon size={16} />,
				label: <Link to={item.to}>{item.label}</Link>,
				className: item.groupStart ? 'nav-group-start' : undefined
			};
		})
	}));

	return (
		<Layout className="app-shell" hasSider>
			<Layout.Sider className="sidebar" width={220}>
				<Link to="/" className="brand" aria-label="agentgateway home">
					<img className="brand-logo brand-logo-light" src={logoLight} alt="agentgateway" />
					<img className="brand-logo brand-logo-dark" src={logoDark} alt="agentgateway" />
					{runtime.data?.build.version && (
						<small title={runtime.data.build.gitRevision}>{runtime.data.build.version}</small>
					)}
				</Link>
				<nav className="nav-list" aria-label="Primary">
					<Menu mode="inline" inlineIndent={9} items={menuItems} selectedKeys={selectedKeys} />
				</nav>
				<div className="sidebar-links">
					{projectLinks.map(link => {
						const Icon = link.icon;
						return (
							<Tooltip content={link.label} key={link.href} side="top">
								<Button
									type="text"
									href={link.href}
									target="_blank"
									rel="noreferrer"
									aria-label={link.label}
									icon={<Icon size={17} />}
								/>
							</Tooltip>
						);
					})}
				</div>
			</Layout.Sider>
			<Layout className="main-area">
				<Layout.Header className="topbar">
					<div className="topbar-left">
						<div className="mobile-nav">
							<Dropdown
								trigger={['click']}
								menu={{ items: menuItems, selectedKeys }}
								classNames={{ root: 'mobile-nav-menu' }}
							>
								<Button className="mobile-nav-trigger" icon={<MenuIcon size={17} />}>
									<CurrentIcon size={16} />
									<span>{currentNav.label}</span>
								</Button>
							</Dropdown>
						</div>
						{matchedNav && currentGroup && (
							<Breadcrumb
								className="breadcrumb"
								aria-label="Breadcrumb"
								separator={<ChevronRight size={14} />}
								items={[{ title: currentGroup.title }, { title: matchedNav.label }]}
							/>
						)}
					</div>
					<div className="topbar-controls">
						{runtime.data?.user && <UserMenu user={runtime.data.user} />}
						<Tooltip content="Toggle theme">
							<Button
								aria-label="Toggle theme"
								onClick={toggleTheme}
								icon={theme === 'dark' ? <Sun size={18} /> : <Moon size={18} />}
							/>
						</Tooltip>
					</div>
				</Layout.Header>
				<Layout.Content className="content">
					{runtime.data?.configReload && !runtime.data.configReload.synchronized && (
						<StatusBanner state="warn" title="Configuration reload failed">
							The last configuration reload was rejected, so the gateway is still running the
							previous configuration. Fix the configuration and save it to retry.
						</StatusBanner>
					)}
					{runtime.data?.ui.configStoreMode === 'readOnly' && (
						<StatusBanner state="info" title="Read-only mode">
							The UI is configured as read-only. Editing is disabled.
						</StatusBanner>
					)}
					<Outlet />
				</Layout.Content>
			</Layout>
		</Layout>
	);
}

function UserMenu({ user }: { user: RuntimeUser }) {
	const [open, setOpen] = useState(false);
	const trigger = useRef<HTMLButtonElement>(null);
	const ref = useDismissiblePopover<HTMLDivElement>(open, () => {
		setOpen(false);
		trigger.current?.focus();
	});
	const label = user.name || user.email || user.subject || 'Signed in';
	const initials = user.name
		? user.name
				.split(/\s+/)
				.slice(0, 2)
				.map(part => Array.from(part)[0])
				.join('')
				.toLocaleUpperCase()
		: Array.from(user.email || user.subject || '')
				.slice(0, 1)
				.join('')
				.toLocaleUpperCase();

	return (
		<div className="user-menu" ref={ref}>
			<Button
				ref={trigger}
				type="text"
				className="user-menu-trigger"
				aria-label={`Account: ${label}`}
				aria-expanded={open}
				aria-controls="user-menu-panel"
				onClick={() => setOpen(!open)}
			>
				<span className="user-avatar" aria-hidden="true">
					{initials || <UserRound size={16} />}
				</span>
				<span className="user-menu-name">{label}</span>
				<ChevronDown size={14} aria-hidden="true" />
			</Button>
			{open && (
				<section id="user-menu-panel" className="user-menu-panel" aria-label="Your account">
					<div className="user-menu-identity">
						<span className="user-menu-caption">Signed in as</span>
						<strong>{label}</strong>
						{user.email && user.email !== label && <span>{user.email}</span>}
					</div>
					{user.canLogout && (
						<form action={`${apiBase}/api/auth/logout`} method="post">
							<Button type="text" block htmlType="submit" icon={<LogOut size={16} />}>
								Sign out
							</Button>
						</form>
					)}
				</section>
			)}
		</div>
	);
}

function navigationGroups(options: {
	hasBinds: boolean;
	hasLlm: boolean;
	hasMcp: boolean;
	hasTraffic: boolean;
	dumpMode: boolean;
}): ReadonlyArray<{ title: string; items: readonly NavItemConfig[] }> {
	const groups: Array<{ title: string; items: readonly NavItemConfig[] }> = [
		{
			title: 'Gateway',
			items: [{ to: '/', label: 'Home', icon: Home }]
		}
	];
	if (!options.dumpMode) {
		groups.push({
			title: 'LLM',
			items: options.hasLlm
				? [
						{ to: '/llm/models', label: 'Models', icon: Bot },
						{ to: '/llm/providers', label: 'Providers', icon: Boxes },

						{
							to: '/llm/policies',
							label: 'Policies',
							icon: Bolt,
							groupStart: true
						},
						{ to: '/llm/guardrails', label: 'Guardrails', icon: Shield },
						{ to: '/llm/keys', label: 'Virtual API Keys', icon: KeyRound },
						{ to: '/llm/costs', label: 'Costs', icon: Coins },

						{
							to: '/llm/analytics',
							label: 'Analytics',
							icon: BarChart3,
							groupStart: true
						},
						{ to: '/llm/logs', label: 'Logs', icon: ScrollText },

						{
							to: '/llm/client-setup',
							label: 'Client Setup',
							icon: Cable,
							groupStart: true
						},
						{ to: '/llm/playground', label: 'Chat Playground', icon: Play }
					]
				: [
						{
							to: '/llm/get-started',
							label: 'Get started',
							icon: Bot,
							placeholder: true
						}
					]
		});
		groups.push({
			title: 'MCP',
			items: options.hasMcp
				? [
						{ to: '/mcp/servers', label: 'Servers', icon: Server },
						{ to: '/mcp/policies', label: 'Policies', icon: ShieldCheck },
						{ to: '/mcp/playground', label: 'Tool Playground', icon: Play }
					]
				: [
						{
							to: '/mcp/get-started',
							label: 'Get started',
							icon: Server,
							placeholder: true
						}
					]
		});
	} else {
		groups.push({
			title: 'LLM',
			items: [
				{ to: '/llm/models', label: 'Models', icon: Bot },
				{ to: '/llm/client-setup', label: 'Client Setup', icon: Cable }
			]
		});
	}
	groups.push({
		title: 'Traffic',
		items: options.dumpMode
			? [
					{ to: '/traffic/listeners', label: 'Listeners', icon: Network },
					{ to: '/traffic/routes', label: 'Routes', icon: Route },
					{ to: '/traffic/policies', label: 'Policies', icon: ShieldCheck }
				]
			: options.hasTraffic
				? [
						{ to: '/traffic/gateways', label: 'Gateways', icon: Network },
						...(options.hasBinds
							? [
									{
										to: '/traffic/listeners',
										label: 'Listeners',
										icon: Network
									}
								]
							: []),
						{ to: '/traffic/routes', label: 'Routes', icon: Route }
					]
				: [
						{
							to: '/traffic/get-started',
							label: 'Get started',
							icon: Network,
							placeholder: true
						}
					]
	});
	groups.push({
		title: 'Tools',
		items: options.dumpMode
			? [{ to: '/cel', label: 'CEL Playground', icon: Braces }]
			: [
					{ to: '/cel', label: 'CEL Playground', icon: Braces },
					{
						to: '/raw-config',
						label: 'Raw Configuration',
						icon: FileCode2,
						exact: true
					},
					{
						to: '/settings',
						label: 'Settings',
						icon: SlidersHorizontal
					}
				]
	});
	return groups;
}

function navItemActive(item: { to: string; exact?: boolean }, path: string) {
	if (item.to === '/') return path === '/';
	if (item.exact) return path === item.to;
	return path === item.to || path.startsWith(`${item.to}/`);
}
