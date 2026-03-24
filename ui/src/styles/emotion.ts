import { css } from "@emotion/react";

// Emotion CSS utility functions and common styles

export const flexRow = css`
  display: flex;
  flex-direction: row;
`;

export const flexColumn = css`
  display: flex;
  flex-direction: column;
`;

export const flexCenter = css`
  display: flex;
  align-items: center;
  justify-content: center;
`;

export const flexBetween = css`
  display: flex;
  align-items: center;
  justify-content: space-between;
`;

export const flexStart = css`
  display: flex;
  align-items: center;
  justify-content: flex-start;
`;

export const flexEnd = css`
  display: flex;
  align-items: center;
  justify-content: flex-end;
`;

export const flexWrap = css`
  flex-wrap: wrap;
`;

// Gap utilities
export const gap = (size: string | number) => css`
  gap: ${typeof size === "number" ? `${size}px` : size};
`;

export const gapXs = gap("var(--spacing-xs)");
export const gapSm = gap("var(--spacing-sm)");
export const gapMd = gap("var(--spacing-md)");
export const gapLg = gap("var(--spacing-lg)");
export const gapXl = gap("var(--spacing-xl)");
export const gapXxl = gap("var(--spacing-xxl)");

// Padding utilities
export const padding = (size: string | number) => css`
  padding: ${typeof size === "number" ? `${size}px` : size};
`;

export const paddingXs = padding("var(--spacing-xs)");
export const paddingSm = padding("var(--spacing-sm)");
export const paddingMd = padding("var(--spacing-md)");
export const paddingLg = padding("var(--spacing-lg)");
export const paddingXl = padding("var(--spacing-xl)");
export const paddingXxl = padding("var(--spacing-xxl)");

// Margin utilities
export const margin = (size: string | number) => css`
  margin: ${typeof size === "number" ? `${size}px` : size};
`;

export const marginXs = margin("var(--spacing-xs)");
export const marginSm = margin("var(--spacing-sm)");
export const marginMd = margin("var(--spacing-md)");
export const marginLg = margin("var(--spacing-lg)");
export const marginXl = margin("var(--spacing-xl)");
export const marginXxl = margin("var(--spacing-xxl)");

// Border radius utilities
export const roundedSm = css`
  border-radius: var(--border-radius-sm);
`;

export const roundedMd = css`
  border-radius: var(--border-radius-md);
`;

export const roundedLg = css`
  border-radius: var(--border-radius-lg);
`;

export const roundedXl = css`
  border-radius: var(--border-radius-xl);
`;

export const roundedFull = css`
  border-radius: 9999px;
`;

// Shadow utilities
export const shadowSm = css`
  box-shadow: var(--shadow-sm);
`;

export const shadowMd = css`
  box-shadow: var(--shadow-md);
`;

export const shadowLg = css`
  box-shadow: var(--shadow-lg);
`;

// Text utilities
export const textBase = css`
  color: var(--color-text-base);
  font-size: var(--font-size-base);
  line-height: var(--line-height-base);
`;

export const textSecondary = css`
  color: var(--color-text-secondary);
`;

export const textTertiary = css`
  color: var(--color-text-tertiary);
`;

export const textSm = css`
  font-size: var(--font-size-sm);
`;

export const textLg = css`
  font-size: var(--font-size-lg);
`;

export const textXl = css`
  font-size: var(--font-size-xl);
`;

export const textBold = css`
  font-weight: var(--font-weight-bold);
`;

export const textSemibold = css`
  font-weight: var(--font-weight-semibold);
`;

export const textMedium = css`
  font-weight: var(--font-weight-medium);
`;

// Transition utilities
export const transitionFast = css`
  transition: all var(--transition-fast) var(--transition-timing);
`;

export const transitionBase = css`
  transition: all var(--transition-base) var(--transition-timing);
`;

export const transitionSlow = css`
  transition: all var(--transition-slow) var(--transition-timing);
`;

// Truncate text
export const truncate = css`
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
`;

// Scrollbar styling
export const scrollbar = css`
  &::-webkit-scrollbar {
    width: 8px;
    height: 8px;
  }

  &::-webkit-scrollbar-track {
    background: var(--color-bg-container);
    border-radius: var(--border-radius-sm);
  }

  &::-webkit-scrollbar-thumb {
    background: var(--color-border-base);
    border-radius: var(--border-radius-sm);

    &:hover {
      background: var(--color-text-tertiary);
    }
  }
`;

// Full height/width utilities
export const fullWidth = css`
  width: 100%;
`;

export const fullHeight = css`
  height: 100%;
`;

export const fullSize = css`
  width: 100%;
  height: 100%;
`;

// Absolute positioning utilities
export const absoluteFill = css`
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
`;

// Clickable/Interactive utilities
export const clickable = css`
  cursor: pointer;
  user-select: none;
  ${transitionBase}

  &:hover {
    opacity: 0.8;
  }

  &:active {
    opacity: 0.6;
  }
`;

// Card-like container
export const card = css`
  ${roundedMd}
  ${shadowSm}
  ${paddingLg}
  background: var(--color-bg-container);
  border: 1px solid var(--color-border-secondary);
`;

// Hide element
export const hidden = css`
  display: none;
`;

// Responsive hide utilities
export const hideOnMobile = css`
  @media (max-width: 576px) {
    display: none;
  }
`;

export const hideOnTablet = css`
  @media (min-width: 577px) and (max-width: 992px) {
    display: none;
  }
`;

export const hideOnDesktop = css`
  @media (min-width: 993px) {
    display: none;
  }
`;
