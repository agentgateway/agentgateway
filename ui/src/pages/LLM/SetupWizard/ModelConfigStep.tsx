import styled from "@emotion/styled";
import { Button, Form, Spin, Typography } from "antd";
import { CheckCircle, Cog } from "lucide-react";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { mutate } from "swr";
import { fetchConfig, updateConfig } from "../../../api/config";
import {
  Actions,
  CopyableCommand,
  FieldFormDescription,
  FieldFormItem,
  FieldFormTitle,
  StepTitle,
  StyledInput,
} from "../../../components/wizard/WizardPrimitives";
import { DEFAULT_LLM_PORT } from "../../../components/wizard/PortStep";
import { useLLMWizard } from "./LLMWizardContext";

const { Link } = Typography;

const CommandStepList = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-xs);
  margin-top: var(--spacing-xs);
`;

const CommandStepRow = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  background: var(--color-bg-elevated, #fafafa);
  border: 1px solid var(--color-border);
  border-left: 3px solid var(--ant-color-primary, #6941c6);
  border-radius: var(--border-radius-md);
  padding: var(--spacing-sm) var(--spacing-md);
`;

const CommandStepNumber = styled.div`
  flex-shrink: 0;
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: var(--ant-color-primary, #6941c6);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
  display: flex;
  align-items: center;
  justify-content: center;
  line-height: 1;
`;

const VerifyRow = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
  margin-top: var(--spacing-lg);
  margin-bottom: var(--spacing-sm);
`;

const SuccessText = styled.span`
  color: var(--color-success, #52c41a);
  display: flex;
  align-items: center;
  gap: var(--spacing-xs);
  font-size: 14px;
  font-weight: 500;
`;

const DEFAULT_MODEL_ALIAS = "my-ollama-smallthinker";
const DEFAULT_HOST = "localhost:11434";
const DEFAULT_MODEL = "smallthinker";

export function ModelConfigStep() {
  const { data, updateModelFields, setWalkthroughVerified, previousStep } = useLLMWizard();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [isVerifying, setIsVerifying] = useState(false);
  const [nameError, setNameError] = useState(true);
  const [form] = Form.useForm();
  const navigate = useNavigate();

  const modelValue = Form.useWatch("model", form) ?? data.modelFields.model ?? DEFAULT_MODEL;
  const hostValue = Form.useWatch("hostOverride", form) ?? data.modelFields.hostOverride ?? DEFAULT_HOST;

  useEffect(() => {
    form.validateFields(["name"]).then(() => setNameError(false)).catch(() => setNameError(true));
  }, [form]);

  const handleVerify = async () => {
    setIsVerifying(true);
    setWalkthroughVerified(false, null);

    try {
      const res = await fetch(`${hostValue}/api/version`);
      if (res.ok) {
        setWalkthroughVerified(true, null);
      } else {
        const msg = `Ollama responded with status ${res.status}`;
        setWalkthroughVerified(false, msg);
        toast.error(msg);
      }
    } catch {
      const msg = `Could not reach Ollama at ${hostValue}. Is it running?`;
      setWalkthroughVerified(false, msg);
      toast.error(msg);
    } finally {
      setIsVerifying(false);
    }
  };

  const handleSubmit = async () => {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }

    setIsSubmitting(true);
    try {
      const { name, model, hostOverride } = values;
      const config = await fetchConfig();

      if (!config.llm) {
        (config as any).llm = {
          port: DEFAULT_LLM_PORT,
          models: [],
          policies: {
            cors: {
              allowOrigins: ["*"],
              allowMethods: ["GET", "POST", "OPTIONS"],
              allowHeaders: ["Content-Type", "Authorization"],
              maxAge: "3600s",
            },
          },
        };
      }
      if (!Array.isArray((config.llm as any).models)) {
        (config.llm as any).models = [];
      }
      (config.llm as any).models.push({
        name,
        provider: "openAI",
        params: { model, baseUrl: hostOverride },
      });

      await updateConfig(config);
      await mutate("/config");

      toast.success("LLM configuration created");
      navigate("/llm-configuration", { state: { skipWizardRedirect: true } });
    } catch (err: any) {
      toast.error(err.message ?? "Failed to create LLM configuration");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Cog size={20} />
          Set up your model
        </div>
      </StepTitle>

      <Form
        form={form}
        layout="vertical"
        initialValues={{
          name: data.modelFields.name || DEFAULT_MODEL_ALIAS,
          model: data.modelFields.model || DEFAULT_MODEL,
          hostOverride: data.modelFields.hostOverride || DEFAULT_HOST,
        }}
        onValuesChange={(changed) => {
          updateModelFields(changed);
          if (changed.hostOverride || changed.model) {
            setWalkthroughVerified(false, null);
          }
          if (changed.name !== undefined) {
            form.validateFields(["name"]).then(() => setNameError(false)).catch(() => setNameError(true));
          }
        }}
      >
        <FieldFormItem
          name="name"
          validateTrigger="onBlur"
          rules={[
            { required: true, message: "Model Alias is required" },
            {
              validator: async (_, value) => {
                if (!value) return;
                const config = await fetchConfig();
                const taken = ((config.llm as any)?.models ?? []).some(
                  (m: any) => m.name === value
                );
                if (taken) return Promise.reject("Model alias is already in use");
              },
            },
          ]}
          label={
            <div>
              <FieldFormTitle>Model Alias</FieldFormTitle>
              <FieldFormDescription>A name that can be used to refer to this particular model.</FieldFormDescription>
            </div>
          }
        >
          <StyledInput placeholder="e.g. my-ollama-smallthinker" />
        </FieldFormItem>

        <FieldFormItem
          name="model"
          label={
            <div>
              <FieldFormTitle>Model Name</FieldFormTitle>
              <FieldFormDescription>
                Browse the{" "}
                <Link href="https://ollama.com/search" target="_blank" rel="noopener noreferrer">
                  Ollama registry
                </Link>{" "}
                - copy &amp; paste a model name below, then run the following commands:
              </FieldFormDescription>
            </div>
          }
          style={{ marginTop: "var(--spacing-md)" }}
          rules={[{ required: true, message: "Model Name is required" }]}
        >
          <StyledInput placeholder="e.g. smallthinker" />
        </FieldFormItem>

        <CommandStepList>
          <CommandStepRow>
            <CommandStepNumber>1</CommandStepNumber>
            <CopyableCommand>{`ollama pull ${modelValue || DEFAULT_MODEL}`}</CopyableCommand>
          </CommandStepRow>
          <CommandStepRow>
            <CommandStepNumber>2</CommandStepNumber>
            <CopyableCommand>ollama serve</CopyableCommand>
          </CommandStepRow>
        </CommandStepList>

        <FieldFormItem
          name="hostOverride"
          label={
            <div>
              <FieldFormTitle>Ollama Host</FieldFormTitle>
              <FieldFormDescription>Address where Ollama is listening (default: http://localhost:11434).</FieldFormDescription>
            </div>
          }
          style={{ marginTop: "var(--spacing-lg)" }}
          rules={[{ required: true, message: "Ollama host is required" }]}
        >
          <StyledInput placeholder="e.g. localhost:11434" />
        </FieldFormItem>

        <VerifyRow>
          <Button type="primary" ghost onClick={handleVerify} disabled={isVerifying}>
            {isVerifying ? <Spin size="small" /> : "Verify Ollama Connection"}
          </Button>
          {data.setupVerified && (
            <SuccessText>
              <CheckCircle size={16} /> Ollama detected
            </SuccessText>
          )}
        </VerifyRow>
      </Form>

      <Actions>
        <Button onClick={previousStep}>Back</Button>
        <Button
          type="primary"
          onClick={handleSubmit}
          loading={isSubmitting}
          disabled={!data.setupVerified || nameError}
        >
          Create
        </Button>
      </Actions>
    </div>
  );
}
