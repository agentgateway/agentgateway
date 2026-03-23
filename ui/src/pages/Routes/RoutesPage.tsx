import { DeleteOutlined, EditOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Card, Space, Table, Tag } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { useRoutes } from "../../api";
import { ProtocolTag } from "../../components/ProtocolTag";
import { StyledAlert } from "../../components/StyledAlert";
import { StyledSelect } from "../../components/StyledSelect";
import type { LocalRoute } from "../../config";
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

type RouteRow = LocalRoute & {
  port?: number;
  listenerName?: string | null;
};

export const RoutesPage = () => {
  const navigate = useNavigate();
  const { data: routes, error, isLoading } = useRoutes();
  const [categoryIndex, setCategoryIndex] = useState<CategoryIndex | null>(
    null,
  );
  const [selectedType, setSelectedType] = useState<string | null>(null);
  const confirm = useConfirm();

  useEffect(() => {
    fetch(assetUrl("/schema-forms/routes/index.json"))
      .then((res) => res.json())
      .then((data) => {
        setCategoryIndex(data);
        // Select the first type by default
        if (data.types && data.types.length > 0) {
          setSelectedType(data.types[0].key);
        }
      })
      .catch((err) => {
        console.error("Failed to load routes index:", err);
        toast.error("Failed to load route types");
      });
  }, []);

  const handleCreate = () => {
    if (!selectedType) {
      toast.error("Please select a route type first");
      return;
    }
    navigate(`/form?category=routes&type=${selectedType}`);
  };

  const handleEdit = (item: RouteRow) => {
    navigate(`/form?category=routes&type=http&name=${item.name}`);
  };

  const handleDelete = (_name: string) => {
    confirm({
      title: "Delete Route",
      content: "Are you sure you want to delete this route?",
      onConfirm: () => {
        // TODO: Implement delete via API
        toast.error("Delete functionality not yet implemented");
      },
      confirmText: "Delete",
      danger: true,
    });
  };

  const columns: ColumnsType<RouteRow> = [
    {
      title: "Name",
      dataIndex: "name",
      key: "name",
      render: (name: string | null | undefined) => name || "<unnamed>",
    },
    {
      title: "Hostnames",
      dataIndex: "hostnames",
      key: "hostnames",
      render: (hostnames: string[] | undefined) => (
        <>
          {(hostnames || []).map((hostname, idx) => (
            <Tag key={idx}>{hostname}</Tag>
          ))}
        </>
      ),
    },
    {
      title: "Methods",
      key: "methods",
      render: (_, record) => {
        const methods = (record.matches || [])
          .map((m) => m.method)
          .filter(Boolean);
        return (
          <>
            {methods.length > 0 ? (
              methods.map((method, idx) => (
                <ProtocolTag key={idx} protocol={method as string} />
              ))
            ) : (
              <span style={{ color: "var(--color-text-secondary)" }}>Any</span>
            )}
          </>
        );
      },
    },
    {
      title: "Paths",
      key: "paths",
      render: (_, record) => {
        const paths = (record.matches || [])
          .map((m) => {
            if ("exact" in m.path) return m.path.exact;
            if ("prefix" in m.path) return m.path.prefix + "*";
            if ("regex" in m.path) return `regex: ${m.path.regex}`;
            return null;
          })
          .filter(Boolean);
        return (
          <>
            {paths.length > 0 ? (
              paths.map((path, idx) => (
                <div key={idx}>
                  <code>{path}</code>
                </div>
              ))
            ) : (
              <span style={{ color: "var(--color-text-secondary)" }}>Any</span>
            )}
          </>
        );
      },
    },
    {
      title: "Backends",
      key: "backends",
      render: (_, record) => `${(record.backends || []).length} backend(s)`,
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
            onClick={() => handleDelete(record.name || "")}
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
        <h1>Routes</h1>
        <StyledAlert
          message="Error Loading Routes"
          description={error.message || "Failed to load routes"}
          type="error"
          showIcon
        />
      </Container>
    );
  }

  return (
    <Container>
      <h1>Routes</h1>

      <Card>
        <Space direction="vertical" style={{ width: "100%" }} size="large">
          <p>Configure HTTP and TCP routes with matching conditions.</p>

          <Space>
            <StyledSelect
              placeholder="Select route type"
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
              Create Route
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

      <Card title="Existing Routes">
        <Table
          columns={columns as any}
          dataSource={routes as any}
          rowKey={(record: any) => record.name || record.routeName}
          pagination={{ pageSize: 10 }}
          loading={isLoading}
        />
      </Card>
    </Container>
  );
};
