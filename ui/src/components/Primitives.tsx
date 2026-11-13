import { createLink } from '@tanstack/react-router';
import {
	Alert,
	Drawer as AntDrawer,
	Tooltip as AntTooltip,
	Button,
	Empty,
	Modal,
	Segmented,
	Select
} from 'antd';
import { dump } from 'js-yaml';
import {
	AlertTriangle,
	CheckCircle2,
	Code,
	HelpCircle,
	Info,
	Loader2,
	X,
	XCircle
} from 'lucide-react';
import type {
	ReactElement,
	KeyboardEvent as ReactKeyboardEvent,
	MouseEvent as ReactMouseEvent,
	ReactNode
} from 'react';
import {
	cloneElement,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useId,
	useLayoutEffect,
	useMemo,
	useRef,
	useState
} from 'react';

import { SchemaHelpPanel } from '@/components/SchemaHelpPanel';
import { parseSchemaYamlEditorValue, SchemaYamlEditor } from '@/components/SchemaYamlEditor';
import {
	type ConfigValueError,
	configSchemaNode,
	configValueError,
	configValueErrors,
	schemaRequires
} from '@/configValidation';
import { useSchemaHelp } from '@/schemaHelp';

// Router link styled as an antd Button. It drops `type`, so use `color="primary" variant="solid"`.
export const ButtonLink = createLink(Button);

// Shared Table pagination: fixed page size with a "from–to of count" total.
export const tablePagination = {
	pageSize: 20,
	hideOnSinglePage: true,
	showTotal: (total: number, [from, to]: [number, number]) => `${from}–${to} of ${total}`
};

// Lets a drawer's form edit its value as raw YAML instead.
export type DrawerRaw = {
	// Config schema node for the value, such as ['$defs', 'LocalLLMVirtualModel'].
	// Only edits valid against it reach the form.
	schemaPath: string[];
	// The form's current value, or undefined while the form is invalid.
	value: () => unknown;
	// Raw mode opened with `value`.
	onOpen?: (value: unknown) => void;
	// Each valid raw edit. The form reloads from the last one when raw mode closes.
	onChange: (value: unknown) => void;
	// The value of a blank form. Clearing the raw text loads it, so the form can
	// start over.
	empty?: unknown;
};

// Provided by a drawer with raw mode; pins it to raw with a reason.
export const PinRawContext = createContext<((reason: string) => void) | null>(null);

// Checks a drawer's form value against its config schema before saving: false
// (with the errors shown on the form) when it is invalid.
export const DrawerCheckContext = createContext<(() => boolean) | null>(null);

// A drawer footer's primary button, which checks the form first (see
// DrawerCheckContext).
export function DrawerSaveButton(props: {
	disabled?: boolean;
	onClick: () => void;
	children: ReactNode;
}) {
	const check = useContext(DrawerCheckContext);
	return (
		<Button
			type="primary"
			disabled={props.disabled}
			onClick={() => {
				if (!check || check()) props.onClick();
			}}
		>
			{props.children}
		</Button>
	);
}

// Schema errors for a drawer's form, which Field and FieldGroup show for their
// `path`.
const FieldErrorsContext = createContext<{
	schemaPath: string[];
	// Errors for the current value once a save was attempted; null before.
	errors: () => ConfigValueError[] | null;
	register: (path: string) => () => void;
	// Fields the form needs that are empty (see Field's `missing`).
	missing: Set<symbol>;
} | null>(null);

// Forms hold unset text fields as '' (or whitespace), which the schema would
// accept as set. Only the check sees this; the saved value is unchanged.
function withoutEmptyStrings(value: unknown): unknown {
	if (Array.isArray(value)) return value.map(withoutEmptyStrings);
	if (!value || typeof value !== 'object') return value;
	return Object.fromEntries(
		Object.entries(value).flatMap(([key, item]) =>
			typeof item === 'string' && !item.trim() ? [] : [[key, withoutEmptyStrings(item)]]
		)
	);
}

function pathsOverlap(error: string, field: string) {
	return (
		error !== '' &&
		(error === field || error.startsWith(`${field}.`) || field.startsWith(`${error}.`))
	);
}

function useFieldError(path: string | undefined, missing: boolean | undefined) {
	const context = useContext(FieldErrorsContext);
	const [id] = useState(() => Symbol('field'));
	useLayoutEffect(() => (path && context ? context.register(path) : undefined), [context, path]);
	useLayoutEffect(() => {
		if (!context || !missing) return;
		context.missing.add(id);
		return () => {
			context.missing.delete(id);
		};
	}, [context, missing, id]);
	if (!context) return { required: false, error: undefined };
	const errors = context.errors();
	return {
		required: missing !== undefined || (path ? schemaRequires(context.schemaPath, path) : false),
		error:
			(path ? errors?.find(error => pathsOverlap(error.path, path))?.message : undefined) ??
			(errors && missing ? 'Required' : undefined)
	};
}

export function PageHeader(props: { title: string; description?: string; actions?: ReactNode }) {
	return (
		<div className="page-header">
			<div>
				<h2>{props.title}</h2>
				{props.description ? <p>{props.description}</p> : null}
			</div>
			{props.actions ? <div className="page-actions">{props.actions}</div> : null}
		</div>
	);
}

export function Panel(props: { children: ReactNode; className?: string }) {
	return (
		<section className={props.className ? `panel ${props.className}` : 'panel'}>
			{props.children}
		</section>
	);
}

export function useDismissiblePopover<T extends HTMLElement>(open: boolean, onClose: () => void) {
	const ref = useRef<T>(null);

	useEffect(() => {
		if (!open) return;
		function onPointerDown(event: PointerEvent) {
			if (!ref.current?.contains(event.target as Node)) onClose();
		}
		function onKeyDown(event: KeyboardEvent) {
			if (event.key === 'Escape') onClose();
		}
		document.addEventListener('pointerdown', onPointerDown);
		document.addEventListener('keydown', onKeyDown);
		return () => {
			document.removeEventListener('pointerdown', onPointerDown);
			document.removeEventListener('keydown', onKeyDown);
		};
	}, [open, onClose]);

	return ref;
}

type DropdownOption = {
	value: string;
	label: ReactNode;
	description?: ReactNode;
	icon?: ReactNode;
	searchText?: string;
};

export function Dropdown(props: {
	value: string;
	options: DropdownOption[];
	onChange: (value: string) => void;
	id?: string;
	ariaLabel: string;
	placeholder?: ReactNode;
	searchable?: boolean;
	className?: string;
	allowEmpty?: boolean;
	disabled?: boolean;
}) {
	const selected =
		props.options.find(option => option.value === props.value) ??
		(props.allowEmpty ? undefined : props.options[0]);
	const byValue = useMemo(
		() => new Map(props.options.map(option => [option.value, option])),
		[props.options]
	);
	return (
		<Select
			className={['custom-select', props.className].filter(Boolean).join(' ')}
			id={props.id}
			aria-label={props.ariaLabel}
			value={selected?.value}
			disabled={props.disabled}
			placeholder={props.placeholder ?? 'No options'}
			options={props.options.map(option => ({ value: option.value, label: option.label }))}
			showSearch={
				props.searchable
					? {
							filterOption: (input, option) =>
								optionSearchText(
									byValue.get(String(option?.value)) ?? { value: '', label: '' }
								).includes(input.trim().toLowerCase())
						}
					: false
			}
			classNames={{ popup: { root: 'custom-select-menu' } }}
			popupMatchSelectWidth={false}
			labelRender={item => {
				const option = byValue.get(String(item.value));
				return option ? <DropdownOptionContent option={option} /> : item.label;
			}}
			optionRender={item => {
				const option = byValue.get(String(item.value));
				return option ? <DropdownOptionContent option={option} showDescription /> : item.label;
			}}
			notFoundContent="No matches"
			onChange={value => props.onChange(value)}
		/>
	);
}

function DropdownOptionContent(props: {
	option: { label: ReactNode; description?: ReactNode; icon?: ReactNode };
	showDescription?: boolean;
}) {
	return (
		<span className="custom-select-value">
			{props.option.icon}
			<span className="custom-select-copy">
				<span>{props.option.label}</span>
				{props.showDescription && props.option.description ? (
					<small>{props.option.description}</small>
				) : null}
			</span>
		</span>
	);
}

function optionSearchText(option: {
	value: string;
	label: ReactNode;
	description?: ReactNode;
	searchText?: string;
}) {
	const label =
		typeof option.label === 'string' || typeof option.label === 'number'
			? String(option.label)
			: '';
	const description =
		typeof option.description === 'string' || typeof option.description === 'number'
			? String(option.description)
			: '';
	return `${option.searchText ?? ''} ${option.value} ${label} ${description}`.toLowerCase();
}

export function SegmentedControl<T extends string>(props: {
	value: T;
	options: Array<{ value: T; label: ReactNode; description?: ReactNode }>;
	onChange: (value: T) => void;
	ariaLabel: string;
	className?: string;
}) {
	return (
		<Segmented<T>
			className={props.className}
			aria-label={props.ariaLabel}
			value={props.value}
			onChange={props.onChange}
			options={props.options.map(option => ({
				value: option.value,
				label: (
					<span className="segmented-option">
						<span>{option.label}</span>
						{option.description ? <small>{option.description}</small> : null}
					</span>
				)
			}))}
		/>
	);
}

export function Tooltip(props: {
	content: ReactNode;
	children: ReactNode;
	side?: 'top' | 'right' | 'bottom' | 'left';
}) {
	// A zero focus/blur delay makes the trigger call flushSync from inside focus events that
	// React fires mid-commit (a drawer focusing itself or restoring focus); a tiny delay defers it.
	const focusDelays = { focusDelay: 0.01, blurDelay: 0.01 };
	return (
		<AntTooltip
			title={formatTooltipContent(props.content)}
			placement={props.side ?? 'top'}
			trigger={['hover', 'focus']}
			{...focusDelays}
		>
			<span className="tooltip-wrap">{props.children}</span>
		</AntTooltip>
	);
}

function formatTooltipContent(content: ReactNode) {
	if (typeof content !== 'string') return content;
	const parts = content.split(/(`[^`]+`)/g);
	return parts.map((part, index) => {
		if (part.startsWith('`') && part.endsWith('`') && part.length > 1) {
			// biome-ignore lint/suspicious/noArrayIndexKey: Existing lint violation; remove this suppression when the underlying issue is fixed.
			return <code key={index}>{part.slice(1, -1)}</code>;
		}
		// biome-ignore lint/suspicious/noArrayIndexKey: Existing lint violation; remove this suppression when the underlying issue is fixed.
		return <span key={index}>{part}</span>;
	});
}

function checkRaw(text: string, raw: DrawerRaw): { value?: unknown; error: string | null } {
	if (!text.trim() && 'empty' in raw) return { value: raw.empty, error: null };
	try {
		const value = parseSchemaYamlEditorValue(text);
		return { value, error: configValueError(raw.schemaPath, value) || null };
	} catch (err) {
		return { error: err instanceof Error ? err.message : 'Invalid YAML' };
	}
}

export function Drawer(props: {
	title: string;
	children: ReactNode;
	footer?: ReactNode | ((requestClose: () => void, dirty: boolean) => ReactNode);
	headerActions?: ReactNode;
	onClose: () => void;
	variant?: 'default' | 'nested';
	dirty?: boolean;
	saving?: boolean;
	raw?: DrawerRaw;
	// Save errors, shown in form and raw mode alike.
	error?: string | null;
}) {
	const [rawText, setRawText] = useState<string | null>(null);
	const [rawPinned, setRawPinned] = useState<string | null>(null);
	// Raw mode was asked for while the form had no value to show.
	const [rawBlocked, setRawBlocked] = useState(false);
	// Raw edits can change fields an editor's own `dirty` check does not see.
	const [rawEdited, setRawEdited] = useState(false);
	const schemaHelp = useSchemaHelp();
	const rawRef = useRef(props.raw);
	rawRef.current = props.raw;
	const pinRaw = useCallback((reason: string) => {
		setRawPinned(reason);
		setRawText(
			text => text ?? dump(rawRef.current?.value() ?? {}, { noRefs: true, lineWidth: 100 })
		);
	}, []);

	function toggleRaw() {
		// Leaving raw drops any invalid text; the form holds the last valid value.
		if (rawText !== null) {
			setRawText(null);
			return;
		}
		const value = props.raw?.value();
		setRawBlocked(value === undefined);
		if (value === undefined) return;
		props.raw?.onOpen?.(value);
		setRawText(dump(value ?? {}, { noRefs: true, lineWidth: 100 }));
	}

	// Form fields that show errors for a path, which also pick the shape errors
	// are reported against (see configValueErrors).
	const fieldPaths = useRef(new Map<string, number>());
	const missingFields = useRef(new Set<symbol>());
	const lastErrors = useRef<{ value: unknown; errors: ConfigValueError[] } | null>(null);
	const [checked, setChecked] = useState(false);
	const formErrors = useCallback(() => {
		const raw = rawRef.current;
		if (!raw) return [];
		const value = raw.value();
		// No value: the editor's own validation reports why.
		if (value === undefined || value === null) return [];
		if (lastErrors.current?.value !== value) {
			lastErrors.current = {
				value,
				errors: configValueErrors(raw.schemaPath, withoutEmptyStrings(value), [
					...fieldPaths.current.keys()
				])
			};
		}
		return lastErrors.current.errors;
	}, []);
	const schemaPathKey = props.raw?.schemaPath.join('/');
	const fieldErrors = useMemo(
		() =>
			schemaPathKey !== undefined && rawRef.current
				? {
						schemaPath: rawRef.current.schemaPath,
						errors: () => (checked ? formErrors() : null),
						register(path: string) {
							fieldPaths.current.set(path, (fieldPaths.current.get(path) ?? 0) + 1);
							return () => {
								const count = (fieldPaths.current.get(path) ?? 1) - 1;
								if (count) fieldPaths.current.set(path, count);
								else fieldPaths.current.delete(path);
							};
						},
						missing: missingFields.current
					}
				: null,
		[checked, formErrors, schemaPathKey]
	);
	function check() {
		// Raw text is validated as it is edited.
		if (rawText !== null) return true;
		lastErrors.current = null;
		setChecked(true);
		return !formErrors().length && !missingFields.current.size;
	}
	const unshownErrors =
		checked && rawText === null
			? formErrors().filter(
					error => ![...fieldPaths.current.keys()].some(path => pathsOverlap(error.path, path))
				)
			: [];

	// Derived from the text, so the value raw mode opens with is checked too.
	const rawError = rawText !== null && props.raw ? checkRaw(rawText, props.raw).error : null;

	function editRaw(text: string) {
		setRawText(text);
		setRawEdited(true);
		if (!props.raw) return;
		const checked = checkRaw(text, props.raw);
		if (checked.error) return;
		// The form may be able to show the new value; if not, it pins raw again.
		setRawPinned(null);
		props.raw.onChange(checked.value);
	}
	const [formChanged, setFormChanged] = useState(false);
	const [confirmClose, setConfirmClose] = useState(false);
	const dirty = (props.dirty ?? formChanged) || rawEdited;

	function requestClose() {
		if (dirty && !props.saving) setConfirmClose(true);
		else props.onClose();
	}

	useEffect(() => {
		if (!dirty || props.saving) return;
		function preventUnload(event: BeforeUnloadEvent) {
			event.preventDefault();
			event.returnValue = '';
		}
		window.addEventListener('beforeunload', preventUnload);
		return () => window.removeEventListener('beforeunload', preventUnload);
	}, [dirty, props.saving]);

	function trackButtonChange(event: ReactMouseEvent<HTMLElement>) {
		const target = event.target as HTMLElement;
		// Select options, toggle buttons and number steppers change values without a change event.
		if (target.closest('[role="option"], [aria-pressed], .ant-input-number-action')) {
			setFormChanged(true);
		}
	}

	function trackKeyChange(event: ReactKeyboardEvent<HTMLElement>) {
		const target = event.target as HTMLElement;
		if (
			(event.key === 'ArrowUp' || event.key === 'ArrowDown') &&
			target.closest('.ant-input-number')
		) {
			setFormChanged(true);
		}
	}

	// Drawers unmount instead of closing, so return focus to where it was.
	useEffect(() => {
		const previousFocus =
			document.activeElement instanceof HTMLElement ? document.activeElement : null;
		return () => {
			const scrollX = window.scrollX;
			const scrollY = window.scrollY;
			previousFocus?.focus({ preventScroll: true });
			window.scrollTo(scrollX, scrollY);
		};
	}, []);

	const nested = props.variant === 'nested';
	return (
		<>
			<AntDrawer
				open
				placement="right"
				size={760}
				push={false}
				mask={nested ? false : { closable: true }}
				keyboard={!confirmClose}
				closable={false}
				title={<h3>{props.title}</h3>}
				onClose={requestClose}
				classNames={{
					section: nested ? 'drawer nested' : 'drawer',
					header: 'drawer-header',
					body: 'drawer-body',
					footer: 'drawer-footer'
				}}
				extra={
					<div className="drawer-header-actions">
						{props.headerActions}
						{props.raw ? (
							<Tooltip content={rawPinned ?? 'Raw'}>
								<Button
									type={rawText === null ? 'default' : 'primary'}
									ghost={rawText !== null}
									aria-label="Raw"
									aria-pressed={rawText !== null}
									disabled={Boolean(rawPinned)}
									onClick={toggleRaw}
									icon={<Code size={17} />}
								/>
							</Tooltip>
						) : null}
						<Tooltip content="Close">
							<Button aria-label="Close" onClick={requestClose} icon={<X size={17} />} />
						</Tooltip>
					</div>
				}
				footer={
					props.footer ? (
						// Invalid raw text has not reached the form, so saving now would save stale values.
						<fieldset className="drawer-footer-actions" disabled={Boolean(rawError)}>
							<DrawerCheckContext.Provider value={props.raw ? check : null}>
								{typeof props.footer === 'function'
									? props.footer(requestClose, dirty)
									: props.footer}
							</DrawerCheckContext.Provider>
						</fieldset>
					) : null
				}
			>
				<div
					className="drawer-content"
					onChangeCapture={() => setFormChanged(true)}
					onClickCapture={trackButtonChange}
					onKeyDownCapture={trackKeyChange}
				>
					{props.raw && rawText !== null ? (
						<div className="policy-yaml-fallback">
							<SchemaHelpPanel schema={configSchemaNode(props.raw.schemaPath)} help={schemaHelp} />
							{rawError ? (
								<StatusBanner state="bad" title="Invalid configuration">
									{rawError}
								</StatusBanner>
							) : null}
							<SchemaYamlEditor
								className="policy-yaml-editor"
								invalid={Boolean(rawError)}
								path="agentgateway-drawer-raw.yaml"
								schema={configSchemaNode(props.raw.schemaPath) ?? {}}
								showLineNumbers={false}
								value={rawText}
								onChange={editRaw}
							/>
						</div>
					) : (
						<PinRawContext.Provider value={props.raw ? pinRaw : null}>
							<FieldErrorsContext.Provider value={fieldErrors}>
								{props.children}
							</FieldErrorsContext.Provider>
						</PinRawContext.Provider>
					)}
					{rawBlocked && rawText === null ? (
						<StatusBanner state="warn" title="Raw mode is unavailable">
							Part of the form is not valid yet. Fix it to edit as raw YAML.
						</StatusBanner>
					) : null}
					{unshownErrors.length ? (
						<StatusBanner state="bad" title="Invalid configuration">
							{unshownErrors.map(error => (
								<div key={error.path}>
									{error.path ? <code>{error.path}</code> : null}
									{error.path ? ': ' : null}
									{error.message}
								</div>
							))}
						</StatusBanner>
					) : null}
					{props.error ? (
						<div className="drawer-save-error">
							<StatusBanner state="bad" title="Save failed">
								{props.error}
							</StatusBanner>
						</div>
					) : null}
				</div>
			</AntDrawer>
			{confirmClose ? (
				<ConfirmDialog
					title="Discard unsaved changes?"
					destructive
					confirmLabel="Discard changes"
					onCancel={() => setConfirmClose(false)}
					onConfirm={() => {
						setConfirmClose(false);
						props.onClose();
					}}
				>
					<p>Your changes have not been saved and will be lost.</p>
				</ConfirmDialog>
			) : null}
		</>
	);
}

export function ConfirmDialog(props: {
	title: string;
	children?: ReactNode;
	confirmLabel?: ReactNode;
	cancelLabel?: ReactNode | null;
	destructive?: boolean;
	confirmDisabled?: boolean;
	onCancel: () => void;
	onConfirm: () => void;
}) {
	return (
		<Modal
			open
			width={420}
			title={props.title}
			onCancel={props.onCancel}
			classNames={{ body: 'confirm-dialog-body' }}
			footer={
				<>
					{props.cancelLabel === null ? null : (
						<Button onClick={props.onCancel}>{props.cancelLabel ?? 'Cancel'}</Button>
					)}
					<Button
						type="primary"
						danger={props.destructive}
						disabled={props.confirmDisabled}
						onClick={props.onConfirm}
					>
						{props.confirmLabel ?? 'Confirm'}
					</Button>
				</>
			}
		>
			{props.children}
		</Modal>
	);
}

const bannerTypes = {
	ok: ['success', CheckCircle2],
	warn: ['warning', AlertTriangle],
	bad: ['error', XCircle],
	info: ['info', Info],
	loading: ['info', Loader2]
} as const;

export function StatusBanner(props: {
	state: 'ok' | 'warn' | 'bad' | 'loading' | 'info';
	title: string;
	children?: ReactNode;
	action?: ReactNode;
}) {
	const [type, Icon] = bannerTypes[props.state];
	return (
		<Alert
			className={`status-banner ${props.state}`}
			type={type}
			showIcon
			icon={<Icon size={18} className={props.state === 'loading' ? 'spin' : undefined} />}
			title={props.title}
			description={props.children}
			action={props.action}
		/>
	);
}

export function EmptyState(props: { title: string; description: string; action?: ReactNode }) {
	return (
		<Empty
			className="empty-state"
			image={false}
			styles={{ root: { margin: 0, lineHeight: 'normal' }, image: { display: 'none' } }}
			description={
				<>
					<h3>{props.title}</h3>
					<p>{props.description}</p>
				</>
			}
		>
			{props.action}
		</Empty>
	);
}

export function Field(props: {
	label: string;
	children:
		| ReactElement<{ id?: string; 'aria-describedby'?: string; 'aria-invalid'?: boolean }>
		| ((id: string, hintId?: string, invalid?: boolean) => ReactNode);
	hint?: string;
	className?: string;
	tooltip?: string;
	// The dotted path this field edits within its drawer's value, such as
	// `jwks.url`: it is marked when the schema requires it, and shows schema
	// errors for it after a save attempt.
	path?: string;
	// Set on a field the form needs beyond the schema (e.g. a name identifying
	// the item): whether it is empty. It is marked as required, and saving
	// fails while it is empty.
	missing?: boolean;
}) {
	const generatedId = useId();
	const hintId = useId();
	const field = useFieldError(props.path, props.missing);
	const hint = field.error ?? props.hint;
	const describedBy = hint ? hintId : undefined;
	const id =
		typeof props.children === 'function' ? generatedId : (props.children.props.id ?? generatedId);
	const className = ['field', props.className, field.error ? 'invalid' : null]
		.filter(Boolean)
		.join(' ');
	const control = (
		<div className={props.tooltip ? 'field field-control' : className}>
			<span className="field-label">
				<label htmlFor={id} className={field.required ? 'field-required' : undefined}>
					{props.label}
				</label>
			</span>
			{typeof props.children === 'function'
				? props.children(id, describedBy, Boolean(field.error))
				: cloneElement(props.children, {
						id,
						'aria-describedby':
							[props.children.props['aria-describedby'], describedBy].filter(Boolean).join(' ') ||
							undefined,
						'aria-invalid': field.error ? true : props.children.props['aria-invalid']
					})}
			{hint ? <small id={hintId}>{hint}</small> : null}
		</div>
	);
	if (!props.tooltip) return control;
	return (
		<div className={`${className} field-with-help`}>
			<Tooltip content={props.tooltip} side="right">
				<HelpButton label={props.tooltip} />
			</Tooltip>
			{control}
		</div>
	);
}

function HelpButton(props: { label: string }) {
	return (
		<Button
			className="help-icon"
			type="text"
			size="small"
			aria-label={props.label}
			icon={<HelpCircle size={13} aria-hidden="true" />}
		/>
	);
}

export function FieldGroup(props: {
	label: string;
	children: ReactNode;
	hint?: string;
	className?: string;
	tooltip?: string;
	// As for Field.
	path?: string;
	missing?: boolean;
}) {
	const field = useFieldError(props.path, props.missing);
	const hint = field.error ?? props.hint;
	return (
		<div
			className={['field', props.className, field.error ? 'invalid' : null]
				.filter(Boolean)
				.join(' ')}
		>
			<span className="field-label">
				{field.required ? <span className="field-required">{props.label}</span> : props.label}
				{props.tooltip ? (
					<Tooltip content={props.tooltip} side="right">
						<HelpButton label={props.tooltip} />
					</Tooltip>
				) : null}
			</span>
			{props.children}
			{hint ? <small>{hint}</small> : null}
		</div>
	);
}

export function JsonBlock(props: { value: unknown }) {
	return <pre className="json-block">{JSON.stringify(props.value, null, 2)}</pre>;
}

export function YamlBlock(props: { value: unknown }) {
	const text = dump(props.value, { noRefs: true, lineWidth: 100 }).replace(/\n$/, '');
	return <YamlTextBlock value={text} />;
}

export function YamlTextBlock(props: { value: string; className?: string }) {
	const text = props.value.replace(/\n$/, '');
	const lines = text.split('\n');
	return (
		<pre
			className={
				props.className ? `json-block yaml-block ${props.className}` : 'json-block yaml-block'
			}
		>
			{lines.map((line, index) => (
				<span
					className="yaml-line"
					key={`${
						// biome-ignore lint/suspicious/noArrayIndexKey: Existing lint violation; remove this suppression when the underlying issue is fixed.
						index
					}-${line}`}
				>
					{highlightYamlLine(line)}
					{index < lines.length - 1 ? '\n' : null}
				</span>
			))}
		</pre>
	);
}

function highlightYamlLine(line: string) {
	const match = line.match(/^(\s*)([^:\n]+):(.*)$/);
	if (!match) return line;
	return (
		<>
			{match[1]}
			<span className="yaml-key">{match[2]}</span>
			<span className="yaml-punctuation">:</span>
			<span className="yaml-value">{match[3]}</span>
		</>
	);
}

export function formatNumber(value: number | null | undefined) {
	return typeof value === 'number' ? new Intl.NumberFormat().format(value) : 'n/a';
}

export function formatDate(value: string | null | undefined) {
	if (!value) return 'n/a';
	return new Intl.DateTimeFormat(undefined, {
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit',
		month: 'short',
		day: 'numeric'
	}).format(new Date(value));
}

export function formatRelativeTime(value: string | null | undefined) {
	if (!value) return 'n/a';
	const deltaMs = new Date(value).getTime() - Date.now();
	const abs = Math.abs(deltaMs);
	const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
	if (abs < 60_000) return rtf.format(Math.round(deltaMs / 1_000), 'second');
	if (abs < 3_600_000) return rtf.format(Math.round(deltaMs / 60_000), 'minute');
	if (abs < 86_400_000) return rtf.format(Math.round(deltaMs / 3_600_000), 'hour');
	return rtf.format(Math.round(deltaMs / 86_400_000), 'day');
}
