import styled from "@emotion/styled";
import { Alert } from "antd";

/**
 * Styled Alert component that works well in both light and dark modes.
 *
 * Deliberately avoids overriding background-color or border-color so that
 * Ant Design's theme algorithm (darkAlgorithm / defaultAlgorithm) can supply
 * the correct semantic colors for each alert type in each color scheme.
 * Only border-radius and text color CSS variables are applied here.
 */
export const StyledAlert = styled(Alert)`
  border-radius: var(--border-radius-lg) !important;

  .ant-alert-message {
    color: var(--color-text-base);
  }

  .ant-alert-description {
    color: var(--color-text-secondary);
  }
`;
