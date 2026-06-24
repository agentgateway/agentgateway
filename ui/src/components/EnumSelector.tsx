import type { ReactNode } from "react";
import { Dropdown } from "./Primitives";
import { enumOptionDetails } from "../policies/policyUtils";
import type { SchemaNode } from "../policies/types";

export type EnumSelectorOption<T extends string = string> = {
  value: T;
  label?: ReactNode;
  description?: ReactNode;
  icon?: ReactNode;
  searchText?: string;
};

export function EnumSelector<T extends string>(props: {
  value: T;
  options?: Array<EnumSelectorOption<T>>;
  schema?: unknown;
  labels?: Partial<Record<T, ReactNode>>;
  descriptions?: Partial<Record<T, ReactNode>>;
  icons?: Partial<Record<T, ReactNode>>;
  onChange: (value: T) => void;
  ariaLabel: string;
  placeholder?: ReactNode;
  searchable?: boolean;
  className?: string;
  allowEmpty?: boolean;
  disabled?: boolean;
  showSelectedDescription?: boolean;
}) {
  const options = enumOptions({
    options: props.options,
    schema: props.schema,
    labels: props.labels,
    descriptions: props.descriptions,
    icons: props.icons,
  });

  return (
    <Dropdown
      ariaLabel={props.ariaLabel}
      value={props.value}
      options={options}
      onChange={(value) => props.onChange(value as T)}
      placeholder={props.placeholder}
      searchable={props.searchable}
      className={props.className}
      allowEmpty={props.allowEmpty}
      disabled={props.disabled}
      showSelectedDescription={props.showSelectedDescription}
    />
  );
}

function enumOptions<T extends string>(props: {
  options?: Array<EnumSelectorOption<T>>;
  schema?: unknown;
  labels?: Partial<Record<T, ReactNode>>;
  descriptions?: Partial<Record<T, ReactNode>>;
  icons?: Partial<Record<T, ReactNode>>;
}) {
  const schema = props.schema as SchemaNode | undefined;
  const schemaDetails = new Map(
    enumOptionDetails(schema).map((option) => [option.value, option]),
  );
  const raw: Array<EnumSelectorOption<T>> =
    props.options ??
    enumOptionDetails(schema).map((option) => ({
      value: option.value as T,
      label: option.label,
      description: option.description,
    }));

  return raw.map((option) => ({
    value: option.value,
    label: props.labels?.[option.value] ?? option.label ?? option.value,
    description:
      props.descriptions?.[option.value] ??
      option.description ??
      schemaDetails.get(option.value)?.description,
    icon: props.icons?.[option.value] ?? option.icon,
    searchText: option.searchText,
  }));
}
