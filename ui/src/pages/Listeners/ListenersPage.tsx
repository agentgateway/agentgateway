import { DeleteOutlined, EditOutlined, PlusOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Card, Space, Table } from "antd";
import type { ColumnsType } from "antd/es/table";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate } from "react-router-dom";
import { deleteListener, useListeners } from "../../api";
import { StyledAlert } from "../../components/StyledAlert";
import { StyledSelect } from "../../components/StyledSelect";
import { assetUrl } from "../../utils/assetUrl";
import type { LocalListener } from "../../config";
import { useConfirm } from "../../contexts/ConfirmContext";

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

type ListenerRow = LocalListener & {
  port?: number;
};

export const ListenersPage = () => {
  const navigate = useNavigate();
  const { data: listeners, error, isLoading, mutate } = useListeners();
  const [categoryIndex, setCategoryIndex] = useState<CategoryIndex | null>(
    null,
  );
  const [selectedType, setSelectedType] = useState<string | null>(null);
  const confirm = useConfirm();

  useEffect(() => {
    fetch(assetUrl("/schema-forms/listeners/index.json"))
      .then((res) => res.json())
      .then((data) => {
        setCategoryIndex(data);
        // Select the first type by default
        if (data.types && data.types.length > 0) {
          setSelectedType(data.types[0].key);
        }
      })
      .catch((err) => {
        console.error("Failed to load listeners index:", err);
        toast.error("Failed to load listener types");
      });
  }, []);

  const handleCreate = () => {
    if (!selectedType) {
      toast.error("Please select a listener type first");
      return;
    }
    navigate(`/form?category=listeners&type=${selectedType}`);
  };

  const handleEdit = (item: ListenerRow) => {
    navigate(`/form?category=listeners&type=http&name=${item.name}`);
  };

  const handleDelete = (record: ListenerRow) => {
    confirm({
      title: "Delete Listener",
      content:
        "Are you sure you want to delete this listener? This action cannot be undone.",
      onConfirm: async () => {
        try {
          await deleteListener(record.name || "", record.port || 0);
          mutate(); // Refresh the listeners data
          toast.success("Listener deleted successfully");
        } catch (error) {
          console.error("Failed to delete listener:", error);
          toast.error("Failed to delete listener");
        }
      },
      confirmText: "Delete",
      danger: true,
    });
  };

  const columns: ColumnsType<ListenerRow> = [
    {
      title: "Name",
      dataIndex: "name",
      key: "name",
      render: (name: string | null | undefined) => name || "<unnamed>",
    },
    {
      title: "Hostname",
      dataIndex: "hostname",
      key: "hostname",
      render: (hostname: string | null | undefined) => hostname || "*",
    },
    {
      title: "Port",
      dataIndex: "port",
      key: "port",
      render: (port: number | undefined) => port || "N/A",
    },
    {
      title: "Protocol",
      dataIndex: "protocol",
      key: "protocol",
      render: (protocol: string | undefined) => protocol || "HTTP",
    },
    {
      title: "Routes",
      key: "routes",
      render: (_, record) => `${(record.routes || []).length} route(s)`,
    },
    {
      title: "Actions",
      key: "actions",
      render: (_, record) => (
        <Space size="middle">
          <span
            onClick={() => handleEdit(record)}
            style={{ cursor: "pointer", color: "#1890ff" }}
          >
            <EditOutlined /> Edit
          </span>
          <span
            onClick={() => handleDelete(record)}
            style={{ cursor: "pointer", color: "#ff4d4f" }}
          >
            <DeleteOutlined /> Delete
          </span>
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
        <h1>Listeners</h1>
        <StyledAlert
          message="Error Loading Listeners"
          description={error.message || "Failed to load listeners"}
          type="error"
          showIcon
        />
      </Container>
    );
  }

  return (
    <Container>
      <h1>Listeners</h1>

      <Card>
        <Space direction="vertical" style={{ width: "100%" }} size="large">
          <p>
            Configure port bindings and listeners (HTTP/HTTPS/TLS/TCP/HBONE
            protocols).
          </p>

          <Space>
            <StyledSelect
              placeholder="Select listener type"
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
            <span
              onClick={handleCreate}
              style={{
                backgroundColor: selectedType ? "#1890ff" : "#d9d9d9",
                color: selectedType ? "white" : "#bfbfbf",
                padding: "6px 16px",
                borderRadius: "6px",
                cursor: selectedType ? "pointer" : "not-allowed",
                display: "inline-flex",
                alignItems: "center",
                gap: "8px",
              }}
            >
              <PlusOutlined />
              Create Listener
            </span>
          </Space>

          {selectedTypeInfo && (
            <Card size="small" type="inner">
              <strong>{selectedTypeInfo.displayName}</strong>
              <p>{selectedTypeInfo.description}</p>
            </Card>
          )}
        </Space>
      </Card>

      <Card title="Existing Listeners">
        <Table
          columns={columns as any}
          dataSource={listeners as any}
          rowKey={(record: any) => record.name || record.port}
          pagination={{ pageSize: 10 }}
          loading={isLoading}
        />
      </Card>
    </Container>
  );
};
