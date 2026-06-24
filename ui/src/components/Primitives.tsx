import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  HelpCircle,
  Info,
  Loader2,
  X,
  XCircle,
} from "lucide-react";
import yaml from "js-yaml";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  CSSProperties,
  KeyboardEvent as ReactKeyboardEvent,
  ReactNode,
} from "react";

const drawerStack: symbol[] = [];

export function PageHeader(props: {
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="page-header">
      <div>
        <h2>{props.title}</h2>
        {props.description ? <p>{props.description}</p> : null}
      </div>
      {props.actions ? (
        <div className="page-actions">{props.actions}</div>
      ) : null}
    </div>
  );
}

export function Panel(props: { children: ReactNode; className?: string }) {
  return (
    <section className={props.className ? `panel ${props.className}` : "panel"}>
      {props.children}
    </section>
  );
}

export function useDismissiblePopover<T extends HTMLElement>(
  open: boolean,
  onClose: () => void,
) {
  const ref = useRef<T>(null);

  useEffect(() => {
    if (!open) return;
    function onPointerDown(event: PointerEvent) {
      if (!ref.current?.contains(event.target as Node)) onClose();
    }
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") onClose();
    }
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onClose]);

  return ref;
}

export function Dropdown(props: {
  value: string;
  options: Array<{
    value: string;
    label: ReactNode;
    description?: ReactNode;
    icon?: ReactNode;
    searchText?: string;
  }>;
  onChange: (value: string) => void;
  ariaLabel: string;
  placeholder?: ReactNode;
  searchable?: boolean;
  className?: string;
  allowEmpty?: boolean;
  disabled?: boolean;
  showSelectedDescription?: boolean;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const optionRefs = useRef<Array<HTMLDivElement | null>>([]);
  const typeaheadRef = useRef("");
  const typeaheadTimerRef = useRef<number | null>(null);
  const selected =
    props.options.find((option) => option.value === props.value) ??
    (props.allowEmpty ? undefined : props.options[0]);
  const filteredOptions = useMemo(() => {
    if (!props.searchable || !query.trim()) return props.options;
    const normalized = query.trim().toLowerCase();
    return props.options.filter((option) =>
      optionSearchText(option).includes(normalized),
    );
  }, [props.options, props.searchable, query]);
  const activeOption = open ? filteredOptions[activeIndex] : undefined;
  const activeOptionId = activeOption
    ? `${id}-option-${activeIndex}`
    : undefined;

  useEffect(() => {
    if (open && props.searchable) searchRef.current?.focus();
    if (!open) setQuery("");
  }, [open, props.searchable]);

  useEffect(
    () => () => {
      if (typeaheadTimerRef.current)
        window.clearTimeout(typeaheadTimerRef.current);
    },
    [],
  );

  useEffect(() => {
    const selectedIndex = filteredOptions.findIndex(
      (option) => option.value === selected?.value,
    );
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
  }, [filteredOptions, selected?.value]);

  useEffect(() => {
    if (!open) return;
    optionRefs.current[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open]);

  function selectOption(option: { value: string } | undefined) {
    if (!option) return;
    props.onChange(option.value);
    setOpen(false);
    window.requestAnimationFrame(() => triggerRef.current?.focus());
  }

  function openAtSelected() {
    const selectedIndex = filteredOptions.findIndex(
      (option) => option.value === selected?.value,
    );
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
    setOpen(true);
  }

  function moveActive(delta: number) {
    setActiveIndex((current) => {
      if (!filteredOptions.length) return 0;
      return (
        (current + delta + filteredOptions.length) % filteredOptions.length
      );
    });
  }

  function setActiveBoundary(position: "first" | "last") {
    setActiveIndex(
      position === "first" ? 0 : Math.max(filteredOptions.length - 1, 0),
    );
  }

  function clearTypeaheadLater() {
    if (typeaheadTimerRef.current)
      window.clearTimeout(typeaheadTimerRef.current);
    typeaheadTimerRef.current = window.setTimeout(() => {
      typeaheadRef.current = "";
    }, 700);
  }

  function runTypeahead(key: string) {
    if (!key.trim()) return;
    const nextQuery = `${typeaheadRef.current}${key}`.toLowerCase();
    typeaheadRef.current = nextQuery;
    clearTypeaheadLater();
    const options = props.searchable ? filteredOptions : props.options;
    if (!options.length) return;
    const current = open
      ? activeIndex
      : options.findIndex((option) => option.value === selected?.value);
    const start = current >= 0 ? current + 1 : 0;
    const ordered = [...options.slice(start), ...options.slice(0, start)];
    const match = ordered.find(
      (option) =>
        optionSearchText(option).startsWith(nextQuery) ||
        optionSearchText(option).includes(` ${nextQuery}`),
    );
    if (!match) return;
    const index = options.findIndex((option) => option.value === match.value);
    if (index >= 0) {
      setActiveIndex(index);
      setOpen(true);
    }
  }

  function onComboboxKeyDown(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (!open) openAtSelected();
      else moveActive(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) openAtSelected();
      else moveActive(-1);
      return;
    }
    if (event.key === "Home" && open) {
      event.preventDefault();
      setActiveBoundary("first");
      return;
    }
    if (event.key === "End" && open) {
      event.preventDefault();
      setActiveBoundary("last");
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      if (open) selectOption(filteredOptions[activeIndex]);
      else openAtSelected();
      return;
    }
    if (event.key === " ") {
      if (props.searchable && document.activeElement === searchRef.current)
        return;
      event.preventDefault();
      if (open) selectOption(filteredOptions[activeIndex]);
      else openAtSelected();
      return;
    }
    if (event.key === "Escape") {
      if (!open) return;
      event.preventDefault();
      setOpen(false);
      triggerRef.current?.focus();
      return;
    }
    if (
      event.key.length === 1 &&
      props.searchable &&
      document.activeElement !== searchRef.current
    ) {
      event.preventDefault();
      typeaheadRef.current = "";
      setQuery(event.key);
      setOpen(true);
      return;
    }
    if (event.key.length === 1 && !props.searchable) {
      runTypeahead(event.key);
    }
  }

  return (
    <div
      className={["custom-select", props.className].filter(Boolean).join(" ")}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        className="custom-select-trigger"
        type="button"
        role="combobox"
        ref={triggerRef}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-listbox`}
        aria-activedescendant={activeOptionId}
        aria-label={props.ariaLabel}
        disabled={props.disabled}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={onComboboxKeyDown}
      >
        {selected ? (
          <DropdownOptionContent
            option={selected}
            showDescription={props.showSelectedDescription}
          />
        ) : (
          <span className="muted">{props.placeholder ?? "No options"}</span>
        )}
        <ChevronDown size={16} />
      </button>
      {open ? (
        <div
          className="custom-select-menu"
          role="listbox"
          id={`${id}-listbox`}
          aria-label={props.ariaLabel}
        >
          {props.searchable ? (
            <input
              className="custom-select-search"
              ref={searchRef}
              role="combobox"
              aria-expanded={open}
              aria-controls={`${id}-listbox`}
              aria-activedescendant={activeOptionId}
              aria-label={`Search ${props.ariaLabel}`}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={onComboboxKeyDown}
              placeholder={`Search ${props.ariaLabel.toLowerCase()}...`}
            />
          ) : null}
          {filteredOptions.map((option, index) => (
            <div
              className={[
                "custom-select-option",
                index === activeIndex ? "active" : null,
                option.value === selected?.value ? "selected" : null,
              ]
                .filter(Boolean)
                .join(" ")}
              role="option"
              aria-selected={option.value === selected?.value}
              id={`${id}-option-${index}`}
              key={option.value}
              ref={(node) => {
                optionRefs.current[index] = node;
              }}
              tabIndex={-1}
              onMouseEnter={() => setActiveIndex(index)}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => selectOption(option)}
            >
              <DropdownOptionContent option={option} showDescription />
            </div>
          ))}
          {filteredOptions.length === 0 ? (
            <div className="custom-select-empty">No matches</div>
          ) : null}
        </div>
      ) : null}
    </div>
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
    typeof option.label === "string" || typeof option.label === "number"
      ? String(option.label)
      : "";
  const description =
    typeof option.description === "string" ||
    typeof option.description === "number"
      ? String(option.description)
      : "";
  return `${option.searchText ?? ""} ${option.value} ${label} ${description}`.toLowerCase();
}

export function SegmentedControl<T extends string>(props: {
  value: T;
  options: Array<{ value: T; label: ReactNode; description?: ReactNode }>;
  onChange: (value: T) => void;
  ariaLabel: string;
  className?: string;
}) {
  return (
    <div
      className={["segmented-control", props.className]
        .filter(Boolean)
        .join(" ")}
      role="radiogroup"
      aria-label={props.ariaLabel}
    >
      {props.options.map((option) => (
        <button
          className={option.value === props.value ? "active" : ""}
          type="button"
          role="radio"
          aria-checked={option.value === props.value}
          key={option.value}
          onClick={() => props.onChange(option.value)}
        >
          <span>{option.label}</span>
          {option.description ? <small>{option.description}</small> : null}
        </button>
      ))}
    </div>
  );
}

export function Tooltip(props: {
  content: ReactNode;
  children: ReactNode;
  side?: "top" | "right" | "bottom" | "left";
}) {
  const id = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const popoverRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<CSSProperties>({
    left: 0,
    top: 0,
    visibility: "hidden",
  });

  useLayoutEffect(() => {
    if (!open) return;

    function updatePosition() {
      const anchor = anchorRef.current;
      const popover = popoverRef.current;
      if (!anchor || !popover) return;

      const anchorRect = anchor.getBoundingClientRect();
      const popoverRect = popover.getBoundingClientRect();
      const gap = 8;
      const margin = 8;
      const viewportWidth = window.innerWidth;
      const viewportHeight = window.innerHeight;

      const candidates = orderedSides(props.side ?? "top").map((side) => {
        if (side === "top") {
          return {
            side,
            left:
              anchorRect.left + anchorRect.width / 2 - popoverRect.width / 2,
            top: anchorRect.top - popoverRect.height - gap,
          };
        }
        if (side === "bottom") {
          return {
            side,
            left:
              anchorRect.left + anchorRect.width / 2 - popoverRect.width / 2,
            top: anchorRect.bottom + gap,
          };
        }
        if (side === "right") {
          return {
            side,
            left: anchorRect.right + gap,
            top:
              anchorRect.top + anchorRect.height / 2 - popoverRect.height / 2,
          };
        }
        return {
          side,
          left: anchorRect.left - popoverRect.width - gap,
          top: anchorRect.top + anchorRect.height / 2 - popoverRect.height / 2,
        };
      });

      const fitting =
        candidates.find(
          (candidate) =>
            candidate.left >= margin &&
            candidate.top >= margin &&
            candidate.left + popoverRect.width <= viewportWidth - margin &&
            candidate.top + popoverRect.height <= viewportHeight - margin,
        ) ?? candidates[0];

      setStyle({
        left: clamp(
          fitting.left,
          margin,
          viewportWidth - popoverRect.width - margin,
        ),
        top: clamp(
          fitting.top,
          margin,
          viewportHeight - popoverRect.height - margin,
        ),
        visibility: "visible",
      });
    }

    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [open, props.side]);

  return (
    <span
      className="tooltip-wrap"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
      onFocus={() => setOpen(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <span
        className="tooltip-anchor"
        aria-describedby={open ? id : undefined}
        ref={anchorRef}
      >
        {props.children}
      </span>
      {open ? (
        <span
          className="tooltip-popover"
          role="tooltip"
          id={id}
          ref={popoverRef}
          style={style}
        >
          {formatTooltipContent(props.content)}
        </span>
      ) : null}
    </span>
  );
}

function formatTooltipContent(content: ReactNode) {
  if (typeof content !== "string") return content;
  const parts = content.split(/(`[^`]+`)/g);
  return parts.map((part, index) => {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 1) {
      return <code key={index}>{part.slice(1, -1)}</code>;
    }
    return <span key={index}>{part}</span>;
  });
}

function orderedSides(preferred: "top" | "right" | "bottom" | "left") {
  const all = ["top", "bottom", "right", "left"] as const;
  return [preferred, ...all.filter((side) => side !== preferred)];
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max);
}

export function Drawer(props: {
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  headerActions?: ReactNode;
  onClose: () => void;
  variant?: "default" | "nested";
}) {
  const drawerRef = useRef<HTMLElement>(null);
  const drawerId = useRef(Symbol("drawer"));

  useEffect(() => {
    const id = drawerId.current;
    drawerStack.push(id);
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      if (drawerStack.at(-1) !== id) return;
      event.preventDefault();
      props.onClose();
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("keydown", closeOnEscape);
      const index = drawerStack.indexOf(id);
      if (index >= 0) drawerStack.splice(index, 1);
    };
  }, [props.onClose]);

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    drawerRef.current?.focus({ preventScroll: true });
    return () => {
      const scrollX = window.scrollX;
      const scrollY = window.scrollY;
      previousFocus?.focus({ preventScroll: true });
      window.scrollTo(scrollX, scrollY);
    };
  }, []);

  function trapFocus(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key !== "Tab") return;
    const drawer = drawerRef.current;
    if (!drawer) return;
    const focusable = Array.from(
      drawer.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter(
      (element) =>
        !element.hasAttribute("disabled") && element.offsetParent !== null,
    );
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div
      className={
        props.variant === "nested"
          ? "drawer-backdrop nested"
          : "drawer-backdrop"
      }
      role="presentation"
      onMouseDown={props.onClose}
    >
      <aside
        className={props.variant === "nested" ? "drawer nested" : "drawer"}
        role="dialog"
        aria-modal="true"
        aria-labelledby="drawer-title"
        tabIndex={-1}
        ref={drawerRef}
        onKeyDown={trapFocus}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="drawer-header">
          <h3 id="drawer-title">{props.title}</h3>
          <div className="drawer-header-actions">
            {props.headerActions}
            <Tooltip content="Close">
              <button
                className="icon-button"
                type="button"
                aria-label="Close"
                onClick={props.onClose}
              >
                <X size={17} />
              </button>
            </Tooltip>
          </div>
        </div>
        <div className="drawer-body">{props.children}</div>
        {props.footer ? (
          <div className="drawer-footer">{props.footer}</div>
        ) : null}
      </aside>
    </div>
  );
}

export function ConfirmDialog(props: {
  title: string;
  children?: ReactNode;
  confirmLabel?: ReactNode;
  cancelLabel?: ReactNode;
  destructive?: boolean;
  confirmDisabled?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const previousFocus =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    dialogRef.current?.focus({ preventScroll: true });
    return () => previousFocus?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      props.onCancel();
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [props]);

  function trapFocus(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== "Tab") return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const focusable = Array.from(
      dialog.querySelectorAll<HTMLElement>(
        'button:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ),
    ).filter((element) => element.offsetParent !== null);
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  return (
    <div
      className="confirm-backdrop"
      role="presentation"
      onMouseDown={props.onCancel}
    >
      <div
        className="confirm-dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="confirm-dialog-title"
        tabIndex={-1}
        ref={dialogRef}
        onKeyDown={trapFocus}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="confirm-dialog-header">
          <h3 id="confirm-dialog-title">{props.title}</h3>
        </div>
        {props.children ? (
          <div className="confirm-dialog-body">{props.children}</div>
        ) : null}
        <div className="confirm-dialog-footer">
          <button className="button" type="button" onClick={props.onCancel}>
            {props.cancelLabel ?? "Cancel"}
          </button>
          <button
            className={props.destructive ? "button danger" : "button primary"}
            type="button"
            disabled={props.confirmDisabled}
            onClick={props.onConfirm}
          >
            {props.confirmLabel ?? "Confirm"}
          </button>
        </div>
      </div>
    </div>
  );
}

export function StatusBanner(props: {
  state: "ok" | "warn" | "bad" | "loading" | "info";
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  const Icon =
    props.state === "loading"
      ? Loader2
      : props.state === "ok"
        ? CheckCircle2
        : props.state === "warn"
          ? AlertTriangle
          : props.state === "info"
            ? Info
            : XCircle;
  return (
    <div className={`status-banner ${props.state}`}>
      <Icon
        size={18}
        className={props.state === "loading" ? "spin" : undefined}
      />
      <div>
        <strong>{props.title}</strong>
        {props.children ? <div>{props.children}</div> : null}
      </div>
      {props.action ? (
        <div className="status-banner-action">{props.action}</div>
      ) : null}
    </div>
  );
}

export function EmptyState(props: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <h3>{props.title}</h3>
      <p>{props.description}</p>
      {props.action}
    </div>
  );
}

export function Field(props: {
  label: string;
  children: ReactNode;
  hint?: string;
  className?: string;
  tooltip?: string;
}) {
  return (
    <label className={props.className ? `field ${props.className}` : "field"}>
      <span className="field-label">
        {props.label}
        {props.tooltip ? (
          <Tooltip content={props.tooltip} side="right">
            <span className="help-icon" tabIndex={0} aria-label={props.tooltip}>
              <HelpCircle size={13} aria-hidden="true" />
            </span>
          </Tooltip>
        ) : null}
      </span>
      {props.children}
      {props.hint ? <small>{props.hint}</small> : null}
    </label>
  );
}

export function FieldGroup(props: {
  label: string;
  children: ReactNode;
  hint?: string;
  className?: string;
  tooltip?: string;
}) {
  return (
    <div className={props.className ? `field ${props.className}` : "field"}>
      <span className="field-label">
        {props.label}
        {props.tooltip ? (
          <Tooltip content={props.tooltip} side="right">
            <span className="help-icon" tabIndex={0} aria-label={props.tooltip}>
              <HelpCircle size={13} aria-hidden="true" />
            </span>
          </Tooltip>
        ) : null}
      </span>
      {props.children}
      {props.hint ? <small>{props.hint}</small> : null}
    </div>
  );
}

export function JsonBlock(props: { value: unknown }) {
  return (
    <pre className="json-block">{JSON.stringify(props.value, null, 2)}</pre>
  );
}

export function YamlBlock(props: { value: unknown }) {
  const text = yaml
    .dump(props.value, { noRefs: true, lineWidth: 100 })
    .replace(/\n$/, "");
  return <YamlTextBlock value={text} />;
}

export function YamlTextBlock(props: { value: string; className?: string }) {
  const text = props.value.replace(/\n$/, "");
  const lines = text.split("\n");
  return (
    <pre
      className={
        props.className
          ? `json-block yaml-block ${props.className}`
          : "json-block yaml-block"
      }
    >
      {lines.map((line, index) => (
        <span className="yaml-line" key={`${index}-${line}`}>
          {highlightYamlLine(line)}
          {index < lines.length - 1 ? "\n" : null}
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
  return typeof value === "number"
    ? new Intl.NumberFormat().format(value)
    : "n/a";
}

export function formatDate(value: string | null | undefined) {
  if (!value) return "n/a";
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    month: "short",
    day: "numeric",
  }).format(new Date(value));
}

export function formatRelativeTime(value: string | null | undefined) {
  if (!value) return "n/a";
  const deltaMs = new Date(value).getTime() - Date.now();
  const abs = Math.abs(deltaMs);
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (abs < 60_000) return rtf.format(Math.round(deltaMs / 1_000), "second");
  if (abs < 3_600_000)
    return rtf.format(Math.round(deltaMs / 60_000), "minute");
  if (abs < 86_400_000)
    return rtf.format(Math.round(deltaMs / 3_600_000), "hour");
  return rtf.format(Math.round(deltaMs / 86_400_000), "day");
}
