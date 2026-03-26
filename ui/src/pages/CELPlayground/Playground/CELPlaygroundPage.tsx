import styled from "@emotion/styled";
import { Col, Row, Space } from "antd";
import { StyledAlert } from "../../../components/StyledAlert";
import { ContextEditor } from "./ContextEditor";
import { ExpressionEditor } from "./ExpressionEditor";
import { ResultPanel } from "./ResultPanel";
import { TemplatesPanel } from "./TemplatesPanel";
import { EXAMPLES, EXPRESSION_TEMPLATES, useCELPlayground } from "./useCELPlayground";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
`;

export const CELPlaygroundPage = () => {
  const {
    expression,
    inputData,
    template,
    loading,
    resultValue,
    resultError,
    hasEvaluated,
    resultExpanded,
    editorTheme,
    setExpression,
    setInputData,
    setTemplate,
    setResultExpanded,
    handleEvaluate,
    handleReset,
    loadTemplate,
    evaluateRef,
  } = useCELPlayground();

  return (
    <Container>
      <div>
        <h1>CEL Playground</h1>
      </div>

      <StyledAlert
        message="Common Expression Language (CEL)"
        description="Test CEL expressions used for policy evaluation, routing decisions, and request validation. CEL provides a simple, fast, and safe way to evaluate expressions."
        type="info"
        showIcon
        closable
      />

      <Row gutter={[16, 16]}>
        <Col xs={24}>
          <ResultPanel
            hasEvaluated={hasEvaluated}
            resultError={resultError}
            resultValue={resultValue}
            resultExpanded={resultExpanded}
            editorTheme={editorTheme}
            onToggleExpanded={() => setResultExpanded(!resultExpanded)}
          />
        </Col>

        <Col xs={24} lg={16}>
          <Space direction="vertical" style={{ width: "100%" }} size="large">
            <ExpressionEditor
              expression={expression}
              examples={EXAMPLES}
              loading={loading}
              editorTheme={editorTheme}
              evaluateRef={evaluateRef}
              onExpressionChange={setExpression}
              onEvaluate={handleEvaluate}
              onReset={handleReset}
            />

            <ContextEditor
              inputData={inputData}
              template={template}
              editorTheme={editorTheme}
              onInputDataChange={setInputData}
              onTemplateChange={setTemplate}
            />
          </Space>
        </Col>

        <Col xs={24} lg={8}>
          <TemplatesPanel
            templates={EXPRESSION_TEMPLATES}
            onLoadTemplate={loadTemplate}
          />
        </Col>
      </Row>
    </Container>
  );
};
