import { AlertTriangle, CheckCircle2, ChevronDown, HelpCircle, Loader2, X, XCircle } from "lucide-react";
import yaml from "js-yaml";
import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent as ReactKeyboardEvent, ReactNode } from "react";

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
  return <section className={props.className ? `panel ${props.className}` : "panel"}>{props.children}</section>;
}

export function Dropdown(props: {
  value: string;
  options: Array<{ value: string; label: ReactNode; description?: ReactNode; icon?: ReactNode; searchText?: string }>;
  onChange: (value: string) => void;
  ariaLabel: string;
  placeholder?: ReactNode;
  searchable?: boolean;
  className?: string;
  allowEmpty?: boolean;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const selected = props.options.find((option) => option.value === props.value) ?? (props.allowEmpty ? undefined : props.options[0]);
  const filteredOptions = useMemo(() => {
    if (!props.searchable || !query.trim()) return props.options;
    const normalized = query.trim().toLowerCase();
    return props.options.filter((option) => optionSearchText(option).includes(normalized));
  }, [props.options, props.searchable, query]);

  useEffect(() => {
    if (open && props.searchable) searchRef.current?.focus();
    if (!open) setQuery("");
  }, [open, props.searchable]);

  useEffect(() => {
    const selectedIndex = filteredOptions.findIndex((option) => option.value === selected?.value);
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
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={`${id}-listbox`}
        aria-label={props.ariaLabel}
        onClick={() => setOpen((current) => !current)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setOpen(true);
            setActiveIndex((current) => {
              if (!filteredOptions.length) return 0;
              return event.key === "ArrowDown"
                ? (current + 1) % filteredOptions.length
                : (current - 1 + filteredOptions.length) % filteredOptions.length;
            });
          }
        }}
      >
        {selected ? <DropdownOptionContent option={selected} /> : <span className="muted">{props.placeholder ?? "No options"}</span>}
        <ChevronDown size={16} />
      </button>
      {open ? (
        <div className="custom-select-menu" role="listbox" id={`${id}-listbox`} aria-label={props.ariaLabel}>
          {props.searchable ? (
            <input
              className="custom-select-search"
              ref={searchRef}
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.preventDefault();
                  setOpen(false);
                  return;
                }
                if (event.key === "ArrowDown") {
                  event.preventDefault();
                  setActiveIndex((current) => filteredOptions.length ? (current + 1) % filteredOptions.length : 0);
                  return;
                }
                if (event.key === "ArrowUp") {
                  event.preventDefault();
                  setActiveIndex((current) => filteredOptions.length ? (current - 1 + filteredOptions.length) % filteredOptions.length : 0);
                  return;
                }
                if (event.key === "Enter") {
                  event.preventDefault();
                  selectOption(filteredOptions[activeIndex]);
                }
              }}
              placeholder={`Search ${props.ariaLabel.toLowerCase()}...`}
            />
          ) : null}
          {filteredOptions.map((option, index) => (
            <button
              className={index === activeIndex || option.value === selected?.value ? "custom-select-option active" : "custom-select-option"}
              type="button"
              role="option"
              aria-selected={option.value === selected?.value}
              id={`${id}-option-${index}`}
              key={option.value}
              ref={(node) => {
                optionRefs.current[index] = node;
              }}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => selectOption(option)}
            >
              <DropdownOptionContent option={option} showDescription />
            </button>
          ))}
          {filteredOptions.length === 0 ? <div className="custom-select-empty">No matches</div> : null}
        </div>
      ) : null}
    </div>
  );
}

function DropdownOptionContent(props: { option: { label: ReactNode; description?: ReactNode; icon?: ReactNode }; showDescription?: boolean }) {
  return (
    <span className="custom-select-value">
      {props.option.icon}
      <span className="custom-select-copy">
        <span>{props.option.label}</span>
        {props.showDescription && props.option.description ? <small>{props.option.description}</small> : null}
      </span>
    </span>
  );
}

function optionSearchText(option: { value: string; label: ReactNode; description?: ReactNode; searchText?: string }) {
  const label = typeof option.label === "string" || typeof option.label === "number" ? String(option.label) : "";
  const description = typeof option.description === "string" || typeof option.description === "number" ? String(option.description) : "";
  return `${option.searchText ?? ""} ${option.value} ${label} ${description}`.toLowerCase();
}

export function Tooltip(props: { content: ReactNode; children: ReactNode; side?: "top" | "right" | "bottom" | "left" }) {
  const id = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const popoverRef = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const [style, setStyle] = useState<CSSProperties>({ left: 0, top: 0, visibility: "hidden" });

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
            left: anchorRect.left + anchorRect.width / 2 - popoverRect.width / 2,
            top: anchorRect.top - popoverRect.height - gap,
          };
        }
        if (side === "bottom") {
          return {
            side,
            left: anchorRect.left + anchorRect.width / 2 - popoverRect.width / 2,
            top: anchorRect.bottom + gap,
          };
        }
        if (side === "right") {
          return {
            side,
            left: anchorRect.right + gap,
            top: anchorRect.top + anchorRect.height / 2 - popoverRect.height / 2,
          };
        }
        return {
          side,
          left: anchorRect.left - popoverRect.width - gap,
          top: anchorRect.top + anchorRect.height / 2 - popoverRect.height / 2,
        };
      });

      const fitting = candidates.find((candidate) =>
        candidate.left >= margin &&
        candidate.top >= margin &&
        candidate.left + popoverRect.width <= viewportWidth - margin &&
        candidate.top + popoverRect.height <= viewportHeight - margin,
      ) ?? candidates[0];

      setStyle({
        left: clamp(fitting.left, margin, viewportWidth - popoverRect.width - margin),
        top: clamp(fitting.top, margin, viewportHeight - popoverRect.height - margin),
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
      <span className="tooltip-anchor" aria-describedby={open ? id : undefined} ref={anchorRef}>{props.children}</span>
      {open ? (
        <span className="tooltip-popover" role="tooltip" id={id} ref={popoverRef} style={style}>{props.content}</span>
      ) : null}
    </span>
  );
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
  onClose: () => void;
}) {
  const drawerRef = useRef<HTMLElement>(null);

  useEffect(() => {
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      props.onClose();
    }
    document.addEventListener("keydown", closeOnEscape);
    return () => document.removeEventListener("keydown", closeOnEscape);
  }, [props.onClose]);

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    drawerRef.current?.focus();
    return () => previousFocus?.focus();
  }, []);

  function trapFocus(event: ReactKeyboardEvent<HTMLElement>) {
    if (event.key !== "Tab") return;
    const drawer = drawerRef.current;
    if (!drawer) return;
    const focusable = Array.from(drawer.querySelectorAll<HTMLElement>(
      'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
    )).filter((element) => !element.hasAttribute("disabled") && element.offsetParent !== null);
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
    <div className="drawer-backdrop" role="presentation" onMouseDown={props.onClose}>
      <aside
        className="drawer"
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
          <Tooltip content="Close">
            <button className="icon-button" type="button" aria-label="Close" onClick={props.onClose}>
              <X size={17} />
            </button>
          </Tooltip>
        </div>
        <div className="drawer-body">{props.children}</div>
        {props.footer ? <div className="drawer-footer">{props.footer}</div> : null}
      </aside>
    </div>
  );
}

export function Stat(props: { label: string; value: ReactNode; detail?: string; tone?: "ok" | "warn" | "bad" }) {
  return (
    <div className={`stat ${props.tone ?? ""}`}>
      <span>{props.label}</span>
      <strong>{props.value}</strong>
      {props.detail ? <small>{props.detail}</small> : null}
    </div>
  );
}

export function StatusBanner(props: { state: "ok" | "warn" | "bad" | "loading"; title: string; children?: ReactNode; action?: ReactNode }) {
  const Icon =
    props.state === "loading" ? Loader2 : props.state === "ok" ? CheckCircle2 : props.state === "warn" ? AlertTriangle : XCircle;
  return (
    <div className={`status-banner ${props.state}`}>
      <Icon size={18} className={props.state === "loading" ? "spin" : undefined} />
      <div>
        <strong>{props.title}</strong>
        {props.children ? <div>{props.children}</div> : null}
      </div>
      {props.action ? <div className="status-banner-action">{props.action}</div> : null}
    </div>
  );
}

export function EmptyState(props: { title: string; description: string; action?: ReactNode }) {
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
            <span className="help-icon" tabIndex={0} aria-label={props.tooltip}><HelpCircle size={13} aria-hidden="true" /></span>
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
            <span className="help-icon" tabIndex={0} aria-label={props.tooltip}><HelpCircle size={13} aria-hidden="true" /></span>
          </Tooltip>
        ) : null}
      </span>
      {props.children}
      {props.hint ? <small>{props.hint}</small> : null}
    </div>
  );
}

export function JsonBlock(props: { value: unknown }) {
  return <pre className="json-block">{JSON.stringify(props.value, null, 2)}</pre>;
}

export function YamlBlock(props: { value: unknown }) {
  const text = yaml.dump(props.value, { noRefs: true, lineWidth: 100 }).replace(/\n$/, "");
  const lines = text.split("\n");
  return (
    <pre className="json-block yaml-block">
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
  return typeof value === "number" ? new Intl.NumberFormat().format(value) : "n/a";
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
  if (abs < 3_600_000) return rtf.format(Math.round(deltaMs / 60_000), "minute");
  if (abs < 86_400_000) return rtf.format(Math.round(deltaMs / 3_600_000), "hour");
  return rtf.format(Math.round(deltaMs / 86_400_000), "day");
}
