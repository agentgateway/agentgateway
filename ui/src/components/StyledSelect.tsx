import styled from "@emotion/styled";
import { Select } from "antd";

/**
 * Styled Select component that works well in both light and dark modes
 */
const StyledSelectBase = styled(Select)`
  &.ant-select {
    /* Select input box */
    .ant-select-selector {
      background-color: var(--color-bg-base) !important;
      border-color: var(--color-border-base) !important;
      color: var(--color-text-base) !important;
      transition: all 0.2s cubic-bezier(0.645, 0.045, 0.355, 1) !important;

      &:hover {
        border-color: var(--color-primary) !important;
        background-color: var(--color-bg-hover) !important;
      }

      &:active {
        border-color: var(--color-primary) !important;
        background-color: var(--color-bg-hover) !important;
      }
    }

    /* Selected value text */
    .ant-select-selection-item {
      color: var(--color-text-base) !important;
      transition: color 0.2s ease !important;
    }

    /* Placeholder text */
    .ant-select-selection-placeholder {
      color: var(--color-text-tertiary) !important;
    }

    /* Arrow icon */
    .ant-select-arrow {
      color: var(--color-text-secondary) !important;
      transition:
        color 0.2s ease,
        transform 0.2s ease !important;
    }

    /* Hover state for arrow */
    &:hover:not(.ant-select-disabled) {
      .ant-select-arrow {
        color: var(--color-primary) !important;
      }
    }

    /* Focused state */
    &.ant-select-focused {
      .ant-select-selector {
        border-color: var(--color-primary) !important;
        box-shadow: 0 0 0 2px rgba(22, 119, 255, 0.1) !important;
        background-color: var(--color-bg-base) !important;
      }

      .ant-select-arrow {
        color: var(--color-primary) !important;
        transform: rotate(180deg) !important;
      }
    }

    /* Open state */
    &.ant-select-open {
      .ant-select-selector {
        border-color: var(--color-primary) !important;
        box-shadow: 0 0 0 2px rgba(22, 119, 255, 0.1) !important;
      }
    }
  }

  /* Disabled state */
  &.ant-select-disabled {
    .ant-select-selector {
      background-color: var(--color-bg-elevated) !important;
      color: var(--color-text-disabled) !important;
      border-color: var(--color-border-base) !important;
    }
  }
`;

// Re-export with static properties from the original Select
export const StyledSelect = Object.assign(StyledSelectBase, {
  Option: Select.Option,
  OptGroup: Select.OptGroup,
  SECRET_COMBOBOX_MODE_DO_NOT_USE: Select.SECRET_COMBOBOX_MODE_DO_NOT_USE,
  _InternalPanelDoNotUseOrYouWillBeFired: Select._InternalPanelDoNotUseOrYouWillBeFired,
});
