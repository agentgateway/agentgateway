import styled from "@emotion/styled";
import type { ReactElement } from "react";

interface ResourceIconProps {
  icon: ReactElement<{ size?: number; strokeWidth?: number }>;
  color: string;
  size?: "small" | "medium" | "large";
}

const sizes = {
  small: {
    wrapper: 20,
    icon: 12,
  },
  medium: {
    wrapper: 24,
    icon: 14,
  },
  large: {
    wrapper: 28,
    icon: 16,
  },
};

const IconWrapper = styled.div<{
  $color: string;
  $size: number;
}>`
  display: flex;
  align-items: center;
  justify-content: center;
  width: ${(props) => props.$size}px;
  height: ${(props) => props.$size}px;
  border-radius: 7px;
  background: linear-gradient(135deg, ${(props) => props.$color}12, ${(props) => props.$color}08);
  border: 1px solid ${(props) => props.$color}25;
  flex-shrink: 0;
  transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
  position: relative;
  overflow: hidden;

  &::before {
    content: '';
    position: absolute;
    top: -50%;
    left: -50%;
    width: 200%;
    height: 200%;
    background: radial-gradient(circle, ${(props) => props.$color}15 0%, transparent 70%);
    opacity: 0;
    transition: opacity 0.3s ease;
  }

  &:hover {
    background: linear-gradient(135deg, ${(props) => props.$color}18, ${(props) => props.$color}12);
    border-color: ${(props) => props.$color}40;
    transform: scale(1.05);
    box-shadow: 0 2px 8px ${(props) => props.$color}20;

    &::before {
      opacity: 1;
    }

    svg {
      opacity: 1;
      transform: scale(1.05);
    }
  }

  svg {
    color: ${(props) => props.$color};
    stroke-width: 2.5;
    opacity: 0.85;
    transition: all 0.25s cubic-bezier(0.4, 0, 0.2, 1);
    filter: drop-shadow(0 1px 2px ${(props) => props.$color}15);
  }
`;

export function ResourceIcon({
  icon,
  color,
  size = "medium",
}: ResourceIconProps) {
  const sizeConfig = sizes[size];

  // Clone the icon element with the new size
  const iconWithSize = {
    ...icon,
    props: {
      ...icon.props,
      size: sizeConfig.icon,
    },
  };

  return (
    <IconWrapper
      $color={color}
      $size={sizeConfig.wrapper}
    >
      {iconWithSize}
    </IconWrapper>
  );
}
