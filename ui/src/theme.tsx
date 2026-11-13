import { App as AntApp, theme as antdTheme, ConfigProvider, type ThemeConfig } from 'antd';
import type { ReactNode } from 'react';
import { createContext, useContext, useEffect, useState } from 'react';

export type ThemeMode = 'light' | 'dark';

// Mirrors the :root and [data-theme='dark'] vars in styles.css. `primary` is the fill (white text on
// it), `primaryText` the ink; dark mode needs both because no one purple passes 4.5:1 for each.
const palettes = {
	light: {
		primary: '#8023c3',
		primaryText: '#8023c3',
		bg: '#f8f9fb',
		surface: '#ffffff',
		surface2: '#f3f4f6',
		text: '#151927',
		muted: '#667085',
		line: '#e5e7eb',
		lineStrong: '#7d8594',
		accentSoft: '#f4e8ff',
		ok: '#027a48',
		okBg: '#ecfdf3',
		warn: '#b7791f',
		warnText: '#92400e',
		warnBg: '#fff7e6',
		bad: '#b42318',
		badBg: '#fff0ee',
		info: '#175cd3',
		infoBg: '#eff8ff',
		// surface-2 55% over surface, as the old table header.
		tableHeaderBg: '#f8f9fa',
		buttonBorder: '#d1d5db',
		tagBg: 'rgba(21, 25, 39, 0.07)',
		tagText: 'rgba(21, 25, 39, 0.85)'
	},
	dark: {
		primary: '#9333ea',
		primaryText: '#c084fc',
		bg: '#030712',
		surface: '#10141f',
		surface2: '#1c2029',
		text: '#f2f5f8',
		muted: '#9ca3af',
		line: '#1f2937',
		lineStrong: '#6b7280',
		accentSoft: '#251333',
		ok: '#63d799',
		okBg: '#14251b',
		warn: '#f2bd5c',
		warnText: '#f2bd5c',
		warnBg: '#2b2315',
		bad: '#ff8a7a',
		badBg: '#2c1918',
		info: '#84caff',
		infoBg: '#102235',
		tableHeaderBg: '#171b25',
		buttonBorder: '#374151',
		tagBg: 'rgba(242, 245, 248, 0.07)',
		tagText: 'rgba(242, 245, 248, 0.85)'
	}
} as const;

function themeConfig(mode: ThemeMode): ThemeConfig {
	const c = palettes[mode];
	const control = { colorBorder: c.lineStrong, colorTextPlaceholder: c.muted };
	return {
		algorithm: mode === 'dark' ? antdTheme.darkAlgorithm : antdTheme.defaultAlgorithm,
		token: {
			colorPrimary: c.primary,
			colorBgBase: c.bg,
			colorTextBase: c.text,
			colorTextSecondary: c.muted,
			colorTextTertiary: c.muted,
			colorTextDescription: c.muted,
			colorBorder: c.line,
			colorBorderSecondary: c.line,
			// antd derives this from colorBgBase, which tints it blue in dark mode.
			colorBorderDisabled: c.line,
			colorBgLayout: c.bg,
			colorBgContainer: c.surface,
			colorBgElevated: c.surface,
			colorSuccess: c.ok,
			colorSuccessBg: c.okBg,
			colorSuccessText: c.ok,
			colorWarning: c.warn,
			colorWarningBg: c.warnBg,
			colorWarningText: c.warnText,
			colorError: c.bad,
			colorErrorBg: c.badBg,
			colorErrorText: c.bad,
			colorInfo: c.info,
			colorInfoBg: c.infoBg,
			colorInfoText: c.info,
			colorLink: c.primaryText,
			colorLinkHover: c.primaryText,
			fontFamily: 'var(--font-sans)',
			controlHeight: 36,
			borderRadius: 6
		},
		components: {
			Alert: {
				defaultPadding: '12px 14px',
				withDescriptionPadding: '12px 14px',
				withDescriptionIconSize: 18
			},
			Breadcrumb: {
				itemColor: c.muted,
				separatorColor: c.muted,
				lastItemColor: c.text,
				separatorMargin: 6
			},
			Button: {
				fontWeight: 500,
				paddingInline: 12,
				contentFontSizeSM: 12,
				paddingInlineSM: 8,
				defaultBorderColor: c.buttonBorder,
				// Disabled buttons fade instead (styles.css), so keep their enabled colours.
				colorBgContainerDisabled: c.surface,
				colorTextDisabled: c.text,
				borderColorDisabled: c.buttonBorder,
				dangerShadow: 'none',
				defaultShadow: 'none',
				primaryShadow: 'none'
			},
			Tag: { defaultBg: c.tagBg, defaultColor: c.tagText, fontSizeSM: 11, borderRadiusSM: 5 },
			Empty: { colorTextDescription: c.text },
			Checkbox: { colorBorder: c.lineStrong },
			Switch: { colorTextQuaternary: c.lineStrong, colorTextTertiary: c.muted },
			Layout: { bodyBg: c.bg, siderBg: c.surface, headerBg: c.surface },
			Menu: {
				itemBg: 'transparent',
				subMenuItemBg: 'transparent',
				itemColor: c.muted,
				itemHoverColor: c.text,
				itemHoverBg: c.surface2,
				itemActiveBg: c.accentSoft,
				itemSelectedBg: c.accentSoft,
				itemSelectedColor: c.primaryText,
				groupTitleColor: c.muted,
				activeBarBorderWidth: 0,
				itemHeight: 28,
				// Vertical margins collapse, so 2 gives the old 2px gap between items.
				itemMarginBlock: 2,
				itemMarginInline: 0,
				itemBorderRadius: 6,
				itemPaddingInline: 9,
				iconMarginInlineEnd: 10
			},
			// The old inputs used the browser's normal line height, which sets textarea height.
			Input: { ...control, lineHeight: 1.2 },
			InputNumber: control,
			Select: {
				...control,
				controlHeight: 40,
				// Arrow and clear icons, which were text-coloured before.
				colorTextQuaternary: c.text,
				optionSelectedBg: c.accentSoft
			},
			DatePicker: { ...control, colorTextQuaternary: c.text },
			Tabs: {
				itemSelectedColor: c.primaryText,
				itemHoverColor: c.primaryText,
				inkBarColor: c.primaryText
			},
			Radio: {
				colorPrimary: c.primaryText,
				colorPrimaryHover: c.primaryText,
				colorBorder: c.lineStrong
			},
			Segmented: {
				trackBg: c.surface2,
				itemSelectedBg: mode === 'dark' ? c.line : c.surface,
				itemSelectedColor: c.text,
				...(mode === 'dark' ? { boxShadowTertiary: `0 0 0 1px ${c.lineStrong}` } : {})
			},
			// Uppercase and letter-spacing for headers live in styles.css (no tokens for them).
			Table: {
				headerBg: c.tableHeaderBg,
				headerColor: c.muted,
				headerSplitColor: 'transparent',
				fontWeightStrong: 600,
				cellPaddingBlock: 11,
				cellPaddingInline: 10,
				cellPaddingBlockMD: 11,
				cellPaddingInlineMD: 10,
				cellPaddingBlockSM: 9,
				cellPaddingInlineSM: 10,
				lineHeight: 1.25,
				rowHoverBg: `${c.text}14`
			}
		}
	};
}

function initialTheme(): ThemeMode {
	const stored = localStorage.getItem('theme');
	if (stored === 'light' || stored === 'dark') return stored;
	return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

const ThemeContext = createContext<{ theme: ThemeMode; toggleTheme: () => void }>({
	theme: 'light',
	toggleTheme: () => {}
});

export function useTheme() {
	return useContext(ThemeContext);
}

export function ThemeProvider(props: { children: ReactNode }) {
	const [theme, setTheme] = useState(initialTheme);
	useEffect(() => {
		document.documentElement.dataset.theme = theme;
	}, [theme]);
	function toggleTheme() {
		const next = theme === 'dark' ? 'light' : 'dark';
		localStorage.setItem('theme', next);
		setTheme(next);
	}
	return (
		<ThemeContext.Provider value={{ theme, toggleTheme }}>
			{/* virtual={false}: virtual lists render options without role="option". */}
			<ConfigProvider theme={themeConfig(theme)} virtual={false} tag={{ variant: 'filled' }}>
				<AntApp component={false}>{props.children}</AntApp>
			</ConfigProvider>
		</ThemeContext.Provider>
	);
}
