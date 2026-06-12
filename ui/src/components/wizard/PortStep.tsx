import styled from "@emotion/styled";
import { Button, Form, InputNumber } from "antd";
import { EthernetPort } from "lucide-react";
import { Actions, StepDescription, StepTitle } from "./WizardPrimitives";

export const DEFAULT_LLM_PORT = 8621;
export const DEFAULT_MCP_PORT = 8622;

const StyledInputNumber = styled(InputNumber)`
  width: 100%;
  border: 1px solid #d9d9d9 !important;

  &:hover,
  &:focus-within {
    border-color: transparent !important;
  }
`;

interface PortStepProps {
  defaultPort: number;
  description: string;
  onNext: (port: number) => void;
  onBack?: () => void;
}

export function PortStep({ defaultPort, description, onNext, onBack }: PortStepProps) {
  const [form] = Form.useForm();

  const handleNext = async () => {
    try {
      const values = await form.validateFields();
      onNext(values.port);
    } catch {
      // validation failed, antd shows inline errors
    }
  };

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <EthernetPort size={20} />
          Where do you want agentgateway to listen on?
        </div>
      </StepTitle>
      <StepDescription>{description}</StepDescription>

      <Form form={form} layout="vertical" initialValues={{ port: defaultPort }}>
        <Form.Item
          name="port"
          label="Port"
          rules={[
            { required: true, message: "Port is required" },
            { type: "number", min: 1, max: 65535, message: "Port must be between 1 and 65535" },
          ]}
        >
          <StyledInputNumber
            placeholder={`e.g. ${defaultPort}`}
            min={1}
            max={65535}
            precision={0}
          />
        </Form.Item>

        <Actions>
          {onBack ? (
            <Button onClick={onBack}>Back</Button>
          ) : (
            <span />
          )}
          <Button type="primary" onClick={handleNext}>
            Next
          </Button>
        </Actions>
      </Form>
    </div>
  );
}
