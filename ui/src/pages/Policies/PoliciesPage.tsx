import { DeleteOutlined, EditOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Card, Space, Table, Tag } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { usePolicies } from "../../api";
import { StyledAlert } from "../../components/StyledAlert";
import { StyledSelect } from "../../components/StyledSelect";
import type { FilterOrPolicy } from "../../config";
import { useConfirm } from "../../contexts/ConfirmContext";
import { assetUrl } from "../../utils/assetUrl";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
`;

interface CategoryIndex {
  category: string;
  types: Array<{
    key: string;
    displayName: string;
    description: string;
    schemaFile: string;
  }>;
}

interface PolicyRow {
  name: string;
  policies: FilterOrPolicy | null | undefined;
  port?: number;
  listenerName?: string | null;
  routeName?: string | null;
}

export const PoliciesPage = () => {
  const navigate = useNavigate();
  const { data: policies, error, isLoading } = usePolicies();
  const [categoryIndex, setCategoryIndex] = useState<CategoryIndex | null>(
    null,
  );
  const [selectedType, setSelectedType] = useState<string | null>(null);
  const confirm = useConfirm();

  useEffect(() => {
    fetch(assetUrl("/schema-forms/policies/index.json"))
      .then((res) => res.json())
      .then((data) => {
        setCategoryIndex(data);
        // Select the first type by default
        if (data.types && data.types.length > 0) {
          setSelectedType(data.types[0].key);
        }
      })
      .catch((err) => {
        console.error("Failed to load policies index:", err);
        toast.error("Failed to load policy types");
      });
  }, []);

  const handleCreate = () => {
    if (!selectedType) {
      toast.error("Please select a policy type first");
      return;
    }
    navigate(`/form?category=policies&type=${selectedType}`);
  };

  const handleEdit = (_item: PolicyRow) => {
    navigate(`/form?category=policies&type=cors`);
  };

  const handleDelete = (_name: string) => {
    confirm({
      title: "Delete Policy",
      content: "Are you sure you want to delete this policy?",
      onConfirm: () => {
        // TODO: Implement delete via API
        toast.error("Delete functionality not yet implemented");
      },
      confirmText: "Delete",
      danger: true,
    });
  };

  const getPolicyTypes = (
    policies: FilterOrPolicy | null | undefined,
  ): string[] => {
    if (!policies) return [];
    const types: string[] = [];
    if (policies.requestHeaderModifier) types.push("Request Headers");
    if (policies.responseHeaderModifier) types.push("Response Headers");
    if (policies.requestRedirect) types.push("Redirect");
    if (policies.urlRewrite) types.push("URL Rewrite");
    if (policies.requestMirror) types.push("Request Mirror");
    if (policies.directResponse) types.push("Direct Response");
    if (policies.cors) types.push("CORS");
    if (policies.mcpAuthorization) types.push("MCP Authorization");
    if (policies.authorization) types.push("Authorization");
    if (policies.mcpAuthentication) types.push("MCP Authentication");
    if (policies.a2a) types.push("A2A");
    if (policies.ai) types.push("AI");
    if (policies.backendTLS) types.push("Backend TLS");
    if (policies.backendAuth) types.push("Backend Auth");
    if (policies.localRateLimit) types.push("Local Rate Limit");
    return types;
  };

  const columns: ColumnsType<PolicyRow> = [
    {
      title: "Route",
      dataIndex: "name",
      key: "name",
      render: (name: string) => name || "<unnamed>",
    },
    {
      title: "Policy Types",
      key: "policyTypes",
      render: (_, record) => {
        const types = getPolicyTypes(record.policies);
        return (
          <>
            {types.map((type, idx) => (
              <Tag key={idx} color="green">
                {type}
              </Tag>
            ))}
          </>
        );
      },
    },
    {
      title: "Listener",
      dataIndex: "listenerName",
      key: "listenerName",
      render: (name: string | null | undefined) => name || "N/A",
    },
    {
      title: "Port",
      dataIndex: "port",
      key: "port",
      render: (port: number | undefined) => port || "N/A",
    },
    {
      title: "Actions",
      key: "actions",
      render: (_, record) => (
        <Space size="middle">
          <Button
            type="link"
            icon={<EditOutlined />}
            onClick={() => handleEdit(record)}
          >
            Edit
          </Button>
          <Button
            type="link"
            danger
            icon={<DeleteOutlined />}
            onClick={() => handleDelete(record.name)}
          >
            Delete
          </Button>
        </Space>
      ),
    },
  ];

  const selectedTypeInfo = categoryIndex?.types.find(
    (t) => t.key === selectedType,
  );

  if (error) {
    return (
      <Container>
        <h1>Policies</h1>
        <StyledAlert
          message="Error Loading Policies"
          description={error.message || "Failed to load policies"}
          type="error"
          showIcon
        />
      </Container>
    );
  }

  return (
    <Container>
      <h1>Policies</h1>

      <Card>
        <Space direction="vertical" style={{ width: "100%" }} size="large">
          <p>
            Configure security policies (JWT auth, MCP auth, ext authz), traffic
            policies (rate limiting, timeout, retry), and routing policies.
          </p>

          <Space>
            <StyledSelect
              placeholder="Select policy type"
              style={{ width: 300 }}
              value={selectedType}
              onChange={(value) => {
                if (typeof value === "string") {
                  setSelectedType(value);
                }
              }}
              options={categoryIndex?.types.map((type) => ({
                label: type.displayName,
                value: type.key,
              }))}
            />
            <Button
              type="primary"
              icon={<PlusOutlined />}
              onClick={handleCreate}
              disabled={!selectedType}
            >
              Create Policy
            </Button>
          </Space>

          {selectedTypeInfo && (
            <Card size="small" type="inner">
              <strong>{selectedTypeInfo.displayName}</strong>
              <p>{selectedTypeInfo.description}</p>
            </Card>
          )}
        </Space>
      </Card>

      <Card title="Existing Policies">
        <Table
          columns={columns as any}
          dataSource={policies as any}
          rowKey={(record: any) => record.name || record.routeName}
          pagination={{ pageSize: 10 }}
          loading={isLoading}
        />
      </Card>
    </Container>
  );
};
