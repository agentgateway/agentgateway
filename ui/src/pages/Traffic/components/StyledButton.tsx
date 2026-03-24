import styled from "@emotion/styled";
import { Button } from "antd";

/**
 * Styled button variants optimized for dark theme with muted colors
 * All buttons have consistent hover states and proper contrast in both themes
 */

export const PrimaryButton = styled(Button)`
  &.ant-btn-primary {
    background: rgba(64, 150, 255, 0.15);
    border-color: rgba(64, 150, 255, 0.4);
    color: #4096ff;
    font-weight: 500;

    &:hover:not(:disabled) {
      background: rgba(64, 150, 255, 0.25);
      border-color: rgba(64, 150, 255, 0.6);
      color: #69b1ff;
    }

    &:active:not(:disabled) {
      background: rgba(64, 150, 255, 0.3);
      border-color: rgba(64, 150, 255, 0.7);
    }

    [data-theme="light"] & {
      background: rgba(22, 119, 255, 0.08);
      border-color: rgba(22, 119, 255, 0.3);
      color: #1677ff;

      &:hover:not(:disabled) {
        background: rgba(22, 119, 255, 0.15);
        border-color: rgba(22, 119, 255, 0.5);
        color: #0958d9;
      }

      &:active:not(:disabled) {
        background: rgba(22, 119, 255, 0.2);
        border-color: rgba(22, 119, 255, 0.6);
      }
    }
  }
`;

export const DefaultButton = styled(Button)`
  &.ant-btn-default {
    background: rgba(255, 255, 255, 0.05);
    border-color: rgba(255, 255, 255, 0.15);
    color: rgba(255, 255, 255, 0.75);
    font-weight: 500;

    &:hover:not(:disabled) {
      background: rgba(255, 255, 255, 0.08);
      border-color: rgba(255, 255, 255, 0.25);
      color: rgba(255, 255, 255, 0.85);
    }

    &:active:not(:disabled) {
      background: rgba(255, 255, 255, 0.12);
      border-color: rgba(255, 255, 255, 0.3);
    }

    [data-theme="light"] & {
      background: rgba(0, 0, 0, 0.02);
      border-color: rgba(0, 0, 0, 0.12);
      color: rgba(0, 0, 0, 0.75);

      &:hover:not(:disabled) {
        background: rgba(0, 0, 0, 0.04);
        border-color: rgba(0, 0, 0, 0.2);
        color: rgba(0, 0, 0, 0.85);
      }

      &:active:not(:disabled) {
        background: rgba(0, 0, 0, 0.06);
        border-color: rgba(0, 0, 0, 0.25);
      }
    }
  }
`;

export const DangerButton = styled(Button)`
  &.ant-btn-dangerous {
    background: rgba(255, 77, 79, 0.12);
    border-color: rgba(255, 77, 79, 0.35);
    color: #ff7875;
    font-weight: 500;

    &:hover:not(:disabled) {
      background: rgba(255, 77, 79, 0.2);
      border-color: rgba(255, 77, 79, 0.5);
      color: #ff9c9e;
    }

    &:active:not(:disabled) {
      background: rgba(255, 77, 79, 0.25);
      border-color: rgba(255, 77, 79, 0.6);
    }

    [data-theme="light"] & {
      background: rgba(255, 77, 79, 0.08);
      border-color: rgba(255, 77, 79, 0.3);
      color: #cf1322;

      &:hover:not(:disabled) {
        background: rgba(255, 77, 79, 0.15);
        border-color: rgba(255, 77, 79, 0.45);
        color: #a8071a;
      }

      &:active:not(:disabled) {
        background: rgba(255, 77, 79, 0.2);
        border-color: rgba(255, 77, 79, 0.55);
      }
    }
  }
`;

export const SuccessButton = styled(Button)`
  &.ant-btn {
    background: rgba(82, 196, 26, 0.12);
    border-color: rgba(82, 196, 26, 0.35);
    color: #73d13d;
    font-weight: 500;

    &:hover:not(:disabled) {
      background: rgba(82, 196, 26, 0.2);
      border-color: rgba(82, 196, 26, 0.5);
      color: #95de64;
    }

    &:active:not(:disabled) {
      background: rgba(82, 196, 26, 0.25);
      border-color: rgba(82, 196, 26, 0.6);
    }

    [data-theme="light"] & {
      background: rgba(82, 196, 26, 0.08);
      border-color: rgba(82, 196, 26, 0.3);
      color: #389e0d;

      &:hover:not(:disabled) {
        background: rgba(82, 196, 26, 0.15);
        border-color: rgba(82, 196, 26, 0.45);
        color: #237804;
      }

      &:active:not(:disabled) {
        background: rgba(82, 196, 26, 0.2);
        border-color: rgba(82, 196, 26, 0.55);
      }
    }
  }
`;
