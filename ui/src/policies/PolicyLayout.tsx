import { useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";

export function PolicySection(props: {
  icon: ReactNode;
  title: string;
  description?: string;
  compact?: boolean;
  children: ReactNode;
}) {
  return (
    <section
      className={
        props.compact ? "policy-form-section compact" : "policy-form-section"
      }
    >
      <div className="policy-form-section-header">
        <span className="policy-form-section-icon">{props.icon}</span>
        <div>
          <h4>{props.title}</h4>
          {props.description ? <p>{props.description}</p> : null}
        </div>
      </div>
      <div className="policy-form-section-body">{props.children}</div>
    </section>
  );
}

export function CollapsiblePolicySection(props: {
  icon: ReactNode;
  title: string;
  description: string;
  children: ReactNode;
  defaultOpen?: boolean;
  bodyClassName?: string;
}) {
  const [open, setOpen] = useState(Boolean(props.defaultOpen));
  return (
    <section
      className={
        open
          ? "policy-form-section transform-section open"
          : "policy-form-section transform-section"
      }
    >
      <button
        className="policy-form-section-header transform-section-toggle"
        type="button"
        aria-expanded={open}
        onClick={() => setOpen((current) => !current)}
      >
        <span className="policy-form-section-icon">{props.icon}</span>
        <div>
          <h4>{props.title}</h4>
          <p>{props.description}</p>
        </div>
        <ChevronDown size={17} />
      </button>
      {open ? (
        <div
          className={
            props.bodyClassName
              ? `policy-form-section-body ${props.bodyClassName}`
              : "policy-form-section-body"
          }
        >
          {props.children}
        </div>
      ) : null}
    </section>
  );
}

export function AdvancedSettingRow(props: {
  icon: ReactNode;
  title: string;
  description: string;
  action: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={
        props.className
          ? `advanced-setting-row ${props.className}`
          : "advanced-setting-row"
      }
    >
      <span className="policy-form-section-icon compact">{props.icon}</span>
      <div className="advanced-setting-copy">
        <strong>{props.title}</strong>
        <small>{props.description}</small>
      </div>
      {props.action}
    </section>
  );
}

export function AdvancedSettingPanel(props: {
  icon: ReactNode;
  title: string;
  description: string;
  action: ReactNode;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section
      className={
        props.className
          ? `advanced-setting-panel ${props.className}`
          : "advanced-setting-panel"
      }
    >
      <div className="advanced-setting-header">
        <span className="policy-form-section-icon compact">{props.icon}</span>
        <div className="advanced-setting-copy">
          <strong>{props.title}</strong>
          <small>{props.description}</small>
        </div>
        {props.action}
      </div>
      {props.children}
    </section>
  );
}
