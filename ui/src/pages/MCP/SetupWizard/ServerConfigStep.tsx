import { Button, Form } from "antd";
import { Cog } from "lucide-react";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { mutate } from "swr";
import { fetchConfig, updateConfig } from "../../../api/config";
import {
  Actions,
  FieldFormDescription,
  FieldFormItem,
  FieldFormTitle,
  StepTitle,
  StyledInput,
} from "../../../components/wizard/WizardPrimitives";
import { useMCPWizard } from "./MCPWizardContext";

const DEFAULT_NAME = "server-everything";
const DEFAULT_ARGS = "-y @modelcontextprotocol/server-everything";

export function ServerConfigStep() {
  const { data, updateServerFields, previousStep } = useMCPWizard();
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [nameError, setNameError] = useState(true);
  const [form] = Form.useForm();
  const navigate = useNavigate();

  useEffect(() => {
    form.validateFields(["name"]).then(() => setNameError(false)).catch(() => setNameError(true));
  }, [form]);

  const handleSubmit = async () => {
    let values;
    try {
      values = await form.validateFields();
    } catch {
      return;
    }

    setIsSubmitting(true);
    try {
      const { name, args } = values;

      const config = await fetchConfig();
      if (!config.mcp) {
        config.mcp = {
          port: data.port!,
          targets: [],
          statefulMode: "stateful",
          policies: {
            cors: {
              allowOrigins: ["*"],
              allowMethods: ["GET", "POST", "OPTIONS", "DELETE"],
              allowHeaders: ["Content-Type", "Authorization", "Mcp-Session-Id", "Mcp-Protocol-Version"],
              exposeHeaders: ["Mcp-Session-Id"],
            },
          },
        };
      }
      config.mcp.targets = [
        ...(config.mcp.targets ?? []),
        { name, stdio: { cmd: "npx", args: [args] } },
      ];
      await updateConfig(config);
      await mutate("/config");

      toast.success("MCP configuration created");
      navigate("/mcp-configuration", { state: { skipWizardRedirect: true } });
    } catch (err: any) {
      toast.error(err.message ?? "Failed to create MCP configuration");
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <div>
      <StepTitle>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Cog size={20} />
          Configure your MCP server
        </div>
      </StepTitle>

      <Form
        form={form}
        layout="vertical"
        initialValues={{
          name: data.serverFields.name || DEFAULT_NAME,
          args: data.serverFields.args || DEFAULT_ARGS,
        }}
        onValuesChange={(changed) => {
          updateServerFields(changed);
          if (changed.name !== undefined) {
            form.validateFields(["name"]).then(() => setNameError(false)).catch(() => setNameError(true));
          }
        }}
      >
        <FieldFormItem
          name="name"
          label={
            <div>
              <FieldFormTitle>Server Alias</FieldFormTitle>
              <FieldFormDescription>A name used to identify this MCP server.</FieldFormDescription>
            </div>
          }
          validateTrigger="onBlur"
          rules={[
            { required: true, message: "Server alias is required" },
            {
              validator: async (_, value) => {
                if (!value) return;
                const config = await fetchConfig();
                const taken = config.mcp?.targets?.some((t: any) => t.name === value);
                if (taken) return Promise.reject("Server alias is already in use");
              },
            },
          ]}
        >
          <StyledInput placeholder="e.g. server-everything" />
        </FieldFormItem>

        <FieldFormItem
          name="args"
          label={
            <div>
              <FieldFormTitle>npx Arguments</FieldFormTitle>
              <FieldFormDescription>Arguments passed to npx to start the MCP server.</FieldFormDescription>
            </div>
          }
          style={{ marginTop: "var(--spacing-md)" }}
          rules={[{ required: true, message: "npx arguments are required" }]}
        >
          <StyledInput placeholder="e.g. -y @modelcontextprotocol/server-everything" />
        </FieldFormItem>
      </Form>

      <Actions>
        <Button onClick={previousStep}>Back</Button>
        <Button
          type="primary"
          onClick={handleSubmit}
          loading={isSubmitting}
          disabled={nameError}
        >
          Create
        </Button>
      </Actions>
    </div>
  );
}
