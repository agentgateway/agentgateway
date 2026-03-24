import { CheckCircleOutlined, RocketOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import {
    Button,
    Card,
    Checkbox,
    Form,
    Input,
    Radio,
    Steps,
    Tag,
} from "antd";
import { useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { StyledAlert } from "../../components/StyledAlert";
import { StyledSelect } from "../../components/StyledSelect";
import { useWizard } from "../../contexts";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
  max-width: 1200px;
  margin: 0 auto;
`;

const StepsContainer = styled.div`
  margin-bottom: var(--spacing-xl);
`;

const Actions = styled.div`
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: var(--spacing-xl);
  padding-top: var(--spacing-lg);
  border-top: 1px solid var(--color-border-secondary);
`;

const WelcomeContent = styled.div`
  text-align: center;
  padding: var(--spacing-xl);

  h2 {
    color: var(--color-primary);
    margin-bottom: var(--spacing-lg);
  }

  .icon {
    font-size: 64px;
    color: var(--color-primary);
    margin-bottom: var(--spacing-lg);
  }
`;

const ReviewItem = styled.div`
  padding: var(--spacing-md);
  background: var(--color-bg-elevated);
  border-radius: var(--border-radius-base);
  margin-bottom: var(--spacing-sm);
`;

interface SetupConfig {
  listener: {
    name: string;
    protocol: string;
    address: string;
    port: number;
  } | null;
  routes: {
    routeName: string;
    pathPattern: string;
    methods: string[];
  } | null;
  backends: {
    backendName: string;
    backendType: string;
    endpoint: string;
    healthCheck: boolean;
  } | null;
  policies: {
    enableAuth?: boolean;
    enableRateLimit?: boolean;
    rateLimit?: number;
    enableLogging?: boolean;
  } | null;
}

export const SetupWizardPage = () => {
  const navigate = useNavigate();
  const { stepIndex, nextStep, previousStep, resetWizard } = useWizard();
  const [form] = Form.useForm();
  const [config, setConfig] = useState<SetupConfig>({
    listener: null,
    routes: null,
    backends: null,
    policies: null,
  });

  const steps = [
    { title: "Welcome", description: "Get started" },
    { title: "Listener", description: "Configure port binding" },
    { title: "Routes", description: "Set up routes" },
    { title: "Backends", description: "Configure backends" },
    { title: "Policies", description: "Add policies" },
    { title: "Review", description: "Review and finish" },
  ];

  const handleNext = async () => {
    try {
      if (stepIndex > 0 && stepIndex < steps.length - 1) {
        const values = await form.validateFields();
        const stepKey = [null, "listener", "routes", "backends", "policies"][
          stepIndex
        ];
        if (stepKey) {
          setConfig((prev) => ({ ...prev, [stepKey]: values }));
        }
      }
      nextStep();
    } catch (error) {
      toast.error("Please fill in all required fields");
    }
  };

  const handleFinish = () => {
    toast.success("Setup completed successfully!");
    resetWizard();
    navigate("/dashboard");
  };

  const renderStepContent = () => {
    switch (stepIndex) {
      case 0:
        return (
          <WelcomeContent>
            <RocketOutlined className="icon" />
            <h2>Welcome to AgentGateway</h2>
            <p style={{ fontSize: "16px", marginBottom: "24px" }}>
              This wizard will help you set up your first configuration in just
              a few steps.
            </p>
            <StyledAlert
              message="Quick Setup"
              description="We'll configure a basic listener, route, backend, and optional policies. You can customize everything later from the main dashboard."
              type="info"
              showIcon
              style={{ textAlign: "left" }}
            />
          </WelcomeContent>
        );

      case 1:
        return (
          <div>
            <h3>Configure Listener</h3>
            <p style={{ marginBottom: "24px" }}>
              Set up a network listener to accept incoming connections.
            </p>
            <Form form={form} layout="vertical">
              <Form.Item
                name="name"
                label="Listener Name"
                rules={[{ required: true, message: "Please enter a name" }]}
                initialValue="http-listener"
              >
                <Input placeholder="e.g., http-listener" />
              </Form.Item>

              <Form.Item
                name="protocol"
                label="Protocol"
                rules={[{ required: true }]}
                initialValue="http"
              >
                <StyledSelect>
                  <StyledSelect.Option value="http">HTTP</StyledSelect.Option>
                  <StyledSelect.Option value="https">HTTPS</StyledSelect.Option>
                  <StyledSelect.Option value="tcp">TCP</StyledSelect.Option>
                </StyledSelect>
              </Form.Item>

              <Form.Item
                name="address"
                label="Bind Address"
                rules={[{ required: true }]}
                initialValue="0.0.0.0"
              >
                <Input placeholder="0.0.0.0" />
              </Form.Item>

              <Form.Item
                name="port"
                label="Port"
                rules={[
                  { required: true },
                  {
                    type: "number",
                    transform: (value) => Number(value),
                    message: "Must be a valid port number",
                  },
                ]}
                initialValue="8080"
              >
                <Input type="number" placeholder="8080" />
              </Form.Item>
            </Form>
          </div>
        );

      case 2:
        return (
          <div>
            <h3>Configure Routes</h3>
            <p style={{ marginBottom: "24px" }}>
              Define routing rules for incoming requests.
            </p>
            <Form form={form} layout="vertical">
              <Form.Item
                name="routeName"
                label="Route Name"
                rules={[{ required: true }]}
                initialValue="default-route"
              >
                <Input placeholder="e.g., api-route" />
              </Form.Item>

              <Form.Item
                name="pathPattern"
                label="Path Pattern"
                rules={[{ required: true }]}
                initialValue="/*"
              >
                <Input placeholder="e.g., /api/*" />
              </Form.Item>

              <Form.Item
                name="methods"
                label="HTTP Methods"
                initialValue={["GET", "POST"]}
              >
                <Checkbox.Group
                  options={[
                    { label: "GET", value: "GET" },
                    { label: "POST", value: "POST" },
                    { label: "PUT", value: "PUT" },
                    { label: "DELETE", value: "DELETE" },
                    { label: "PATCH", value: "PATCH" },
                  ]}
                />
              </Form.Item>
            </Form>
          </div>
        );

      case 3:
        return (
          <div>
            <h3>Configure Backend</h3>
            <p style={{ marginBottom: "24px" }}>
              Set up the upstream service that will handle requests.
            </p>
            <Form form={form} layout="vertical">
              <Form.Item
                name="backendName"
                label="Backend Name"
                rules={[{ required: true }]}
                initialValue="default-backend"
              >
                <Input placeholder="e.g., api-backend" />
              </Form.Item>

              <Form.Item
                name="backendType"
                label="Backend Type"
                rules={[{ required: true }]}
                initialValue="http"
              >
                <Radio.Group>
                  <Radio value="http">HTTP/HTTPS</Radio>
                  <Radio value="llm">LLM Provider</Radio>
                  <Radio value="mcp">MCP Server</Radio>
                </Radio.Group>
              </Form.Item>

              <Form.Item
                name="endpoint"
                label="Endpoint URL"
                rules={[{ required: true }]}
                initialValue="http://localhost:3000"
              >
                <Input placeholder="http://localhost:3000" />
              </Form.Item>

              <Form.Item
                name="healthCheck"
                label="Health Check"
                valuePropName="checked"
                initialValue={true}
              >
                <Checkbox>Enable health checks</Checkbox>
              </Form.Item>
            </Form>
          </div>
        );

      case 4:
        return (
          <div>
            <h3>Configure Policies (Optional)</h3>
            <p style={{ marginBottom: "24px" }}>
              Add optional policies for security and traffic control.
            </p>
            <Form form={form} layout="vertical">
              <Form.Item name="enableAuth" valuePropName="checked">
                <Checkbox>Enable Authentication</Checkbox>
              </Form.Item>

              <Form.Item name="enableRateLimit" valuePropName="checked">
                <Checkbox>Enable Rate Limiting</Checkbox>
              </Form.Item>

              <Form.Item
                name="rateLimit"
                label="Requests per Minute"
                dependencies={["enableRateLimit"]}
                initialValue={100}
              >
                <Input type="number" placeholder="100" />
              </Form.Item>

              <Form.Item
                name="enableLogging"
                valuePropName="checked"
                initialValue={true}
              >
                <Checkbox>Enable Request Logging</Checkbox>
              </Form.Item>
            </Form>
          </div>
        );

      case 5:
        return (
          <div>
            <h3>
              <CheckCircleOutlined
                style={{ color: "#52c41a", marginRight: "8px" }}
              />
              Review Configuration
            </h3>
            <p style={{ marginBottom: "24px" }}>
              Review your configuration before applying it.
            </p>

            <ReviewItem>
              <strong>Listener</strong>
              {config.listener && (
                <div style={{ marginTop: "8px" }}>
                  <Tag color="blue">
                    {config.listener.protocol?.toUpperCase()}
                  </Tag>
                  <span>
                    {config.listener.address}:{config.listener.port}
                  </span>
                </div>
              )}
            </ReviewItem>

            <ReviewItem>
              <strong>Route</strong>
              {config.routes && (
                <div style={{ marginTop: "8px" }}>
                  <div>{config.routes.routeName}</div>
                  <div style={{ fontSize: "12px", color: "#999" }}>
                    Path: {config.routes.pathPattern}
                  </div>
                  {config.routes.methods?.map((method: string) => (
                    <Tag
                      key={method}
                      color="green"
                      style={{ marginTop: "4px" }}
                    >
                      {method}
                    </Tag>
                  ))}
                </div>
              )}
            </ReviewItem>

            <ReviewItem>
              <strong>Backend</strong>
              {config.backends && (
                <div style={{ marginTop: "8px" }}>
                  <div>{config.backends.backendName}</div>
                  <div style={{ fontSize: "12px", color: "#999" }}>
                    {config.backends.endpoint}
                  </div>
                  <Tag color="purple" style={{ marginTop: "4px" }}>
                    {config.backends.backendType?.toUpperCase()}
                  </Tag>
                </div>
              )}
            </ReviewItem>

            <ReviewItem>
              <strong>Policies</strong>
              {config.policies && (
                <div style={{ marginTop: "8px" }}>
                  {config.policies.enableAuth && (
                    <Tag color="orange">Authentication</Tag>
                  )}
                  {config.policies.enableRateLimit && (
                    <Tag color="orange">
                      Rate Limit: {config.policies.rateLimit}/min
                    </Tag>
                  )}
                  {config.policies.enableLogging && (
                    <Tag color="orange">Logging</Tag>
                  )}
                  {!config.policies.enableAuth &&
                    !config.policies.enableRateLimit &&
                    !config.policies.enableLogging && (
                      <span style={{ color: "#999" }}>No policies enabled</span>
                    )}
                </div>
              )}
            </ReviewItem>

            <StyledAlert
              message="Ready to Apply"
              description="Click 'Finish' to apply this configuration to your AgentGateway instance."
              type="success"
              showIcon
              style={{ marginTop: "24px" }}
            />
          </div>
        );

      default:
        return null;
    }
  };

  const canGoPrevious = stepIndex > 0;
  const isLastStep = stepIndex === steps.length - 1;

  return (
    <Container>
      <h1>Setup Wizard</h1>

      <StepsContainer>
        <Steps current={stepIndex} items={steps} />
      </StepsContainer>

      <Card>
        {renderStepContent()}

        <Actions>
          <Button onClick={previousStep} disabled={!canGoPrevious}>
            Previous
          </Button>
          <Button
            type="primary"
            onClick={isLastStep ? handleFinish : handleNext}
          >
            {isLastStep ? "Finish" : "Next"}
          </Button>
        </Actions>
      </Card>
    </Container>
  );
};
