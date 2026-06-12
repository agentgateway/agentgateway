import styled from "@emotion/styled";
import React from "react";

/**
 * Per-protocol color tokens.
 * Each entry defines light-mode and dark-mode values for background,
 * border, and text so the tag looks sharp on both color schemes.
 */
const PROTOCOL_STYLES: Record<
  string,
  {
    bg: string;
    border: string;
    color: string;
    darkBg: string;
    darkBorder: string;
    darkColor: string;
  }
> = {
  HTTP: {
    bg: "rgba(22, 119, 255, 0.08)",
    border: "rgba(22, 119, 255, 0.35)",
    color: "#1677ff",
    darkBg: "rgba(22, 119, 255, 0.18)",
    darkBorder: "rgba(64, 150, 255, 0.5)",
    darkColor: "#4096ff",
  },
  HTTPS: {
    bg: "rgba(82, 196, 26, 0.08)",
    border: "rgba(82, 196, 26, 0.35)",
    color: "#389e0d",
    darkBg: "rgba(82, 196, 26, 0.18)",
    darkBorder: "rgba(115, 209, 61, 0.5)",
    darkColor: "#73d13d",
  },
  TLS: {
    bg: "rgba(250, 173, 20, 0.08)",
    border: "rgba(250, 173, 20, 0.35)",
    color: "#d48806",
    darkBg: "rgba(250, 173, 20, 0.18)",
    darkBorder: "rgba(255, 197, 61, 0.5)",
    darkColor: "#ffc53d",
  },
  TCP: {
    bg: "rgba(250, 140, 22, 0.08)",
    border: "rgba(250, 140, 22, 0.35)",
    color: "#d46b08",
    darkBg: "rgba(250, 140, 22, 0.30)",
    darkBorder: "rgba(255, 169, 64, 0.75)",
    darkColor: "#ffbe5c",
  },
  HBONE: {
    bg: "rgba(114, 46, 209, 0.08)",
    border: "rgba(114, 46, 209, 0.35)",
    color: "#531dab",
    darkBg: "rgba(114, 46, 209, 0.18)",
    darkBorder: "rgba(179, 127, 235, 0.5)",
    darkColor: "#b37feb",
  },
  GET: {
    bg: "rgba(22, 119, 255, 0.08)",
    border: "rgba(22, 119, 255, 0.35)",
    color: "#1677ff",
    darkBg: "rgba(22, 119, 255, 0.18)",
    darkBorder: "rgba(64, 150, 255, 0.5)",
    darkColor: "#4096ff",
  },
  POST: {
    bg: "rgba(82, 196, 26, 0.08)",
    border: "rgba(82, 196, 26, 0.35)",
    color: "#389e0d",
    darkBg: "rgba(82, 196, 26, 0.18)",
    darkBorder: "rgba(115, 209, 61, 0.5)",
    darkColor: "#73d13d",
  },
  PUT: {
    bg: "rgba(250, 173, 20, 0.08)",
    border: "rgba(250, 173, 20, 0.35)",
    color: "#d48806",
    darkBg: "rgba(250, 173, 20, 0.18)",
    darkBorder: "rgba(255, 197, 61, 0.5)",
    darkColor: "#ffc53d",
  },
  PATCH: {
    bg: "rgba(250, 140, 22, 0.08)",
    border: "rgba(250, 140, 22, 0.35)",
    color: "#d46b08",
    darkBg: "rgba(250, 140, 22, 0.18)",
    darkBorder: "rgba(255, 169, 64, 0.5)",
    darkColor: "#ffa940",
  },
  DELETE: {
    bg: "rgba(255, 77, 79, 0.08)",
    border: "rgba(255, 77, 79, 0.35)",
    color: "#cf1322",
    darkBg: "rgba(255, 77, 79, 0.18)",
    darkBorder: "rgba(255, 120, 117, 0.5)",
    darkColor: "#ff7875",
  },
};

const FALLBACK = {
  bg: "rgba(0, 0, 0, 0.04)",
  border: "rgba(0, 0, 0, 0.15)",
  color: "rgba(0, 0, 0, 0.65)",
  darkBg: "rgba(255, 255, 255, 0.08)",
  darkBorder: "rgba(255, 255, 255, 0.2)",
  darkColor: "rgba(255, 255, 255, 0.65)",
};

interface TagWrapProps {
  $bg: string;
  $border: string;
  $color: string;
  $darkBg: string;
  $darkBorder: string;
  $darkColor: string;
}

const TagWrap = styled.span<TagWrapProps>`
  display: inline-flex;
  align-items: center;
  padding: 0 6px;
  height: 20px;
  font-size: 11px;
  font-weight: 500;
  line-height: 20px;
  white-space: nowrap;
  border-radius: 4px;
  border: 1px solid ${(p) => p.$border};
  background: ${(p) => p.$bg};
  color: ${(p) => p.$color};
  letter-spacing: 0.3px;

  [data-theme="dark"] & {
    background: ${(p) => p.$darkBg};
    border-color: ${(p) => p.$darkBorder};
    color: ${(p) => p.$darkColor};
  }
`;

interface ProtocolTagProps {
  protocol: string;
  style?: React.CSSProperties;
  className?: string;
}

/**
 * A tag/badge for displaying protocol labels (HTTP, TCP, HBONE, etc.) and
 * HTTP method labels (GET, POST, DELETE, etc.) that looks sharp in both
 * light and dark mode.
 */
export function ProtocolTag({ protocol, style, className }: ProtocolTagProps) {
  const upper = protocol.toUpperCase();
  const tokens = PROTOCOL_STYLES[upper] ?? FALLBACK;

  return (
    <TagWrap
      $bg={tokens.bg}
      $border={tokens.border}
      $color={tokens.color}
      $darkBg={tokens.darkBg}
      $darkBorder={tokens.darkBorder}
      $darkColor={tokens.darkColor}
      style={style}
      className={className}
    >
      {protocol}
    </TagWrap>
  );
}
