import styled from "@emotion/styled";
import { Button } from "antd";
import React from "react";
import { StyledAlert } from "./StyledAlert";

const ErrorContainer = styled.div`
  display: flex;
  justify-content: center;
  align-items: flex-start;
  min-height: 400px;
  max-height: 100vh;
  padding: var(--spacing-xl);
  overflow-y: auto;
`;

const ErrorContent = styled.div`
  width: 100%;
  margin-top: var(--spacing-xl);
`;

interface ErrorBoundaryProps {
  children: React.ReactNode;
  fallback?: React.ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
  errorInfo: React.ErrorInfo | null;
}

export class ErrorBoundary extends React.Component<
  ErrorBoundaryProps,
  ErrorBoundaryState
> {
  constructor(props: ErrorBoundaryProps) {
    super(props);
    this.state = { hasError: false, error: null, errorInfo: null };
  }

  static getDerivedStateFromError(error: Error): Partial<ErrorBoundaryState> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error("Error caught by boundary:", error, errorInfo);
    this.setState({
      error,
      errorInfo,
    });
  }

  handleReset = () => {
    this.setState({ hasError: false, error: null, errorInfo: null });
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }

      return (
        <ErrorContainer>
          <ErrorContent>
            <StyledAlert
              message="Something went wrong"
              description={
                <div>
                  <p>
                    {this.state.error?.message ||
                      "An unexpected error occurred"}
                  </p>
                  {import.meta.env.DEV && this.state.errorInfo && (
                    <details style={{ marginTop: "10px" }}>
                      <summary
                        style={{ cursor: "pointer", userSelect: "none" }}
                      >
                        Error details
                      </summary>
                      <pre
                        style={{
                          fontSize: "12px",
                          overflow: "auto",
                          maxHeight: "300px",
                          marginTop: "8px",
                          padding: "12px",
                          backgroundColor: "var(--color-bg-elevated)",
                          borderRadius: "var(--border-radius-md)",
                          border: "1px solid var(--color-border-base)",
                        }}
                      >
                        {this.state.errorInfo.componentStack}
                      </pre>
                    </details>
                  )}
                  <Button
                    type="primary"
                    onClick={this.handleReset}
                    style={{ marginTop: "16px" }}
                  >
                    Try again
                  </Button>
                </div>
              }
              type="error"
              showIcon
            />
          </ErrorContent>
        </ErrorContainer>
      );
    }

    return this.props.children;
  }
}
