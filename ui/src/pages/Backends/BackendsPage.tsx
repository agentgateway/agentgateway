import { DeleteOutlined, EditOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Card, Space, Table, Tag } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { useBackends } from "../../api";
import { StyledAlert } from "../../components/StyledAlert";
import { StyledSelect } from "../../components/StyledSelect";
import type { LocalRouteBackend } from "../../config";
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

type BackendRow = LocalRouteBackend & {
  port?: number;
  listenerName?: string | null;
  routeName?: string | null;
};

export const BackendsPage = () => {
  const navigate = useNavigate();
  const { data: backends, error, isLoading } = useBackends();
  const [categoryIndex, setCategoryIndex] = useState<CategoryIndex | null>(
    null,
  );
  const [selectedType, setSelectedType] = useState<string | null>(null);
  const confirm = useConfirm();

  useEffect(() => {
    fetch(assetUrl("/schema-forms/backends/index.json"))
      .then((res) => res.json())
      .then((data) => {
        setCategoryIndex(data);
        // Select the first type by default
        if (data.types && data.types.length > 0) {
          setSelectedType(data.types[0].key);
        }
      })
      .catch((err) => {
        console.error("Failed to load backends index:", err);
        toast.error("Failed to load backend types");
      });
  }, []);

  const handleCreate = () => {
    if (!selectedType) {
      toast.error("Please select a backend type first");
      return;
    }
    navigate(`/form?category=backends&type=${selectedType}`);
  };

  const handleEdit = (_item: BackendRow) => {
    navigate(`/form?category=backends&type=service`);
  };

  const handleDelete = (_backend: BackendRow) => {
    confirm({
      title: "Delete Backend",
      content: "Are you sure you want to delete this backend?",
      onConfirm: () => {
        // TODO: Implement delete via API
        toast.error("Delete functionality not yet implemented");
      },
      confirmText: "Delete",
      danger: true,
    });
  };

  const getBackendType = (backend: BackendRow): string => {
    if ("service" in backend) return "Service";
    if ("host" in backend) return "Host";
    if ("mcp" in backend) return "MCP";
    if ("ai" in backend) return "AI";
    if ("dynamic" in backend) return "Dynamic";
    return "Unknown";
  };

  const getBackendTarget = (backend: BackendRow): string => {
    if (
      "service" in backend &&
      backend.service &&
      typeof backend.service === "object"
    ) {
      const service = backend.service as { name: any; port: number };
      return `${service.name}:${service.port}`;
    }
    if ("host" in backend && typeof backend.host === "string") {
      return backend.host;
    }
    if ("mcp" in backend && backend.mcp) {
      return "MCP Backend";
    }
    if ("ai" in backend && backend.ai) {
      return "AI Backend";
    }
    if ("dynamic" in backend) {
      return "Dynamic";
    }
    return "N/A";
  };

  const columns: ColumnsType<BackendRow> = [
    {
      title: "Type",
      key: "type",
      render: (_, record) => <Tag color="blue">{getBackendType(record)}</Tag>,
    },
    {
      title: "Target",
      key: "target",
      render: (_, record) => getBackendTarget(record),
    },
    {
      title: "Route",
      dataIndex: "routeName",
      key: "routeName",
      render: (name: string | null | undefined) => name || "N/A",
    },
    {
      title: "Listener",
      dataIndex: "listenerName",
      key: "listenerName",
      render: (name: string | null | undefined) => name || "N/A",
    },
    {
      title: "Weight",
      dataIndex: "weight",
      key: "weight",
      render: (weight: number | undefined) => weight || 1,
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
            onClick={() => handleDelete(record)}
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
        <h1>Backends</h1>
        <StyledAlert
          message="Error Loading Backends"
          description={error.message || "Failed to load backends"}
          type="error"
          showIcon
        />
      </Container>
    );
  }

  return (
    <Container>
      <h1>Backends</h1>

      <Card>
        <Space direction="vertical" style={{ width: "100%" }} size="large">
          <p>
            Configure backend services (MCP targets, AI providers, service
            connections, hosts, dynamic).
          </p>

          <Space>
            <StyledSelect
              placeholder="Select backend type"
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
              Create Backend
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

      <Card title="Existing Backends">
        <Table
          columns={columns}
          dataSource={backends}
          rowKey={(record, index) => `${record.routeName || "route"}-${index}`}
          pagination={{ pageSize: 10 }}
          loading={isLoading}
        />
      </Card>
    </Container>
  );
};
