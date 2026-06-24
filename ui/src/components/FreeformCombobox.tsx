import { Check, ChevronDown } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { useDismissiblePopover } from "./Primitives";

export function FreeformCombobox(props: {
  ariaLabel: string;
  value: string;
  options: string[];
  onChange: (value: string) => void;
  placeholder?: string;
  emptyText?: string;
}) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const [browseAll, setBrowseAll] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const optionRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const suppressNextFocusOpenRef = useRef(false);
  const popoverRef = useDismissiblePopover<HTMLDivElement>(open, () =>
    setOpen(false),
  );
  const filteredOptions = useMemo(() => {
    const query = browseAll ? "" : props.value.trim().toLowerCase();
    const options = query
      ? props.options.filter((option) => option.toLowerCase().includes(query))
      : props.options;
    return options.slice(0, 80);
  }, [browseAll, props.options, props.value]);

  useEffect(() => {
    const selectedIndex = browseAll
      ? filteredOptions.findIndex((option) => option === props.value)
      : -1;
    setActiveIndex(selectedIndex >= 0 ? selectedIndex : 0);
  }, [browseAll, filteredOptions, props.value]);

  useEffect(() => {
    if (!open) return;
    optionRefs.current[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open]);

  function select(value: string) {
    props.onChange(value);
    setOpen(false);
    setBrowseAll(false);
    suppressNextFocusOpenRef.current = true;
    window.requestAnimationFrame(() => inputRef.current?.focus());
  }

  function move(delta: number) {
    setActiveIndex((current) => {
      if (!filteredOptions.length) return 0;
      return (
        (current + delta + filteredOptions.length) % filteredOptions.length
      );
    });
  }

  function onKeyDown(event: ReactKeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (!open) {
        setBrowseAll(false);
        setOpen(true);
      } else move(1);
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      if (!open) {
        setBrowseAll(false);
        setOpen(true);
      } else move(-1);
      return;
    }
    if (event.key === "Enter" && open && filteredOptions[activeIndex]) {
      event.preventDefault();
      select(filteredOptions[activeIndex]);
      return;
    }
    if (event.key === "Escape" && open) {
      event.preventDefault();
      setOpen(false);
    }
  }

  return (
    <div className="freeform-combobox" ref={popoverRef}>
      <div className="freeform-combobox-input-wrap">
        <input
          ref={inputRef}
          aria-label={props.ariaLabel}
          value={props.value}
          onChange={(event) => {
            setBrowseAll(false);
            props.onChange(event.target.value);
            setOpen(true);
          }}
          onFocus={() => {
            if (suppressNextFocusOpenRef.current) {
              suppressNextFocusOpenRef.current = false;
              return;
            }
            setBrowseAll(false);
            setOpen(true);
          }}
          onKeyDown={onKeyDown}
          placeholder={props.placeholder}
          autoComplete="off"
          spellCheck={false}
        />
        <button
          className="freeform-combobox-trigger"
          type="button"
          aria-label={`Show ${props.ariaLabel} options`}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            suppressNextFocusOpenRef.current = true;
            setBrowseAll(true);
            setOpen((current) => !current);
            window.requestAnimationFrame(() => inputRef.current?.focus());
          }}
        >
          <ChevronDown size={15} />
        </button>
      </div>
      {open ? (
        <div
          className="freeform-combobox-menu"
          role="listbox"
          aria-label={props.ariaLabel}
        >
          {filteredOptions.map((option, index) => (
            <button
              className={[
                "freeform-combobox-option",
                index === activeIndex ? "active" : null,
                option === props.value ? "selected" : null,
              ]
                .filter(Boolean)
                .join(" ")}
              type="button"
              role="option"
              aria-selected={option === props.value}
              key={option}
              ref={(node) => {
                optionRefs.current[index] = node;
              }}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => select(option)}
            >
              <span>{option}</span>
              {option === props.value ? <Check size={14} /> : null}
            </button>
          ))}
          {filteredOptions.length === 0 ? (
            <div className="freeform-combobox-empty">
              {props.emptyText ?? "No matches. Custom values are allowed."}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
