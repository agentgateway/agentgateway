import styled from "@emotion/styled";
import { Button, Checkbox, Toolbar, ToolbarContent, ToolbarItem } from "@patternfly/react-core";
import { LogViewer, LogViewerSearch } from "@patternfly/react-log-viewer";
import { Tooltip } from "antd";
import { ChevronDown } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTheme } from "../../contexts/ThemeContext";

/**
 * Styling
 */
const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  height: 100%;
  overflow: hidden;

  .pf-v6-c-log-viewer {
    display: flex;
    flex-direction: column;
    flex-grow: 1;
    height: 100%;
    min-height: 0;

    --pf-v6-c-log-viewer__main--BackgroundColor: var(--color-bg-base);
    --pf-v6-c-log-viewer--m-dark__main--BackgroundColor: var(--color-bg-layout);
    --pf-v6-c-log-viewer--m-dark__main--BorderWidth: 1px;
    --pf-v6-c-log-viewer__main--BorderColor: var(--color-border-base);
  }

  .pf-v6-c-log-viewer__main { 
    flex-grow: 1;
  }

  .pf-v6-c-check__label {
    color: var(--color-text-base);
  }
`

const StyledFooterButton = styled(Button)`
  && {
    background-color: var(--color-bg-base);
    color: var(--color-text-base);
    border: 1px solid var(--color-border-base);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
    
    border-radius: 50%;
    
    width: 35px;
    height: 35px;
    padding: 0;
    line-height: 0;
    box-sizing: border-box;

    display: flex;
    justify-content: center;
    align-self: center; 
    align-items: center;
    bottom: 10%;
    position: fixed;
  }
`;

/**
 * Component
 */
interface SoloLogViewerProps {
  data: string[];
}

export const SoloLogViewer = ({ data }: SoloLogViewerProps) => {
  const { theme } = useTheme();
  const [isTextWrapped, setIsTextWrapped] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [currentItemCount, setCurrentItemCount] = useState(0);
  const [linesBehind, setLinesBehind] = useState(0);
  const logViewerRef = useRef<any>(null);

  useEffect(() => {
    if (!isPaused && data.length > 0) { 
      setCurrentItemCount(data.length);
      if (logViewerRef.current) { 
        logViewerRef.current.scrollToBottom();
      }
    } else if (data.length !== currentItemCount) { 
      setLinesBehind(data.length - currentItemCount);
    } else { 
      setLinesBehind(0);
    }
  }, [isPaused, data]);


  const onScroll = ({
    scrollOffsetToBottom,
    scrollUpdateWasRequested,
  }: {
    scrollOffsetToBottom: number;
    scrollDirection: string;
    scrollUpdateWasRequested: boolean;
  }) => {
    if (!scrollUpdateWasRequested) {
      if (scrollOffsetToBottom > 30) {
        setIsPaused(true);
      } else {
        setIsPaused(false);
      }
    }
  };

  const FooterButton = () => (
    <Tooltip title={`Scroll to bottom`}>
      <StyledFooterButton
        data-testid="log-viewer-footer-button"
        onClick={() => setIsPaused(false)}
        isBlock
      >
        <ChevronDown />
      </StyledFooterButton>
    </Tooltip>
  )

  return (
    <Container>
      <LogViewer
        data={data}
        data-testid="log-viewer"
        hasLineNumbers={true}
        theme={theme}
        isTextWrapped={isTextWrapped}
        scrollToRow={isPaused ? undefined : currentItemCount}
        innerRef={logViewerRef}
        onScroll={onScroll}
        footer={isPaused && <FooterButton />}
        toolbar={
          <Toolbar>
            <ToolbarContent>
                <ToolbarItem>
                  <LogViewerSearch
                    data-testid="log-viewer-search"
                    placeholder="Search logs"
                    minSearchChars={3}
                  />
                </ToolbarItem>
                <ToolbarItem alignSelf="center">
                  <Checkbox
                    data-testid="wrap-text-checkbox"
                    label="Wrap text"
                    aria-label="wrap text checkbox"
                    isChecked={isTextWrapped}
                    id="wrap-text-checkbox"
                    onChange={(_event, value) => setIsTextWrapped(value)}
                  />
                </ToolbarItem>
            </ToolbarContent>
          </Toolbar>
        }
      />
    </Container>
  );
}