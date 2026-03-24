import { ArrowLeftOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { Button, Card, Space, Typography } from "antd";
import { useEffect, useState } from "react";
import toast from "react-hot-toast";
import { useNavigate, useSearchParams } from "react-router-dom";
import { fetchConfig, updateConfig } from "../../api/config";
import { SchemaForm } from "../../components/SchemaForm";
import { StyledSelect } from "../../components/StyledSelect";
import { mergeFormDataIntoConfig } from "./configMerger";
import { assetUrl } from "../../utils/assetUrl";

const Container = styled.div`
  display: flex;
  flex-direction: column;
  gap: var(--spacing-lg);
  padding: var(--spacing-xl);
  width: 100%;
`;

const Header = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-md);
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

const CATEGORY_CONFIG: Record<
  string,
  { displayName: string; pluralName: string; basePath: string }
> = {
  policies: {
    displayName: "Policy",
    pluralName: "Policies",
    basePath: "/policies",
  },
  listeners: {
    displayName: "Listener",
    pluralName: "Listeners",
    basePath: "/listeners",
  },
  routes: { displayName: "Route", pluralName: "Routes", basePath: "/routes" },
  backends: {
    displayName: "Backend",
    pluralName: "Backends",
    basePath: "/backends",
  },
};

export const FormPage = () => {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const category = (searchParams.get("category") || "policies") as
    | "policies"
    | "listeners"
    | "routes"
    | "backends";
  const typeParam = searchParams.get("type");
  const idParam = searchParams.get("id");

  const [categoryIndex, setCategoryIndex] = useState<CategoryIndex | null>(
    null,
  );
  const [selectedType, setSelectedType] = useState<string | null>(typeParam);

  const categoryConfig = CATEGORY_CONFIG[category] || CATEGORY_CONFIG.policies;

  useEffect(() => {
    fetch(assetUrl(`/schema-forms/${category}/index.json`))
      .then((res) => res.json())
      .then((data) => {
        setCategoryIndex(data);
        // If no type in URL, select first type
        if (!typeParam && data.types && data.types.length > 0) {
          setSelectedType(data.types[0].key);
        }
      })
      .catch((err) => {
        console.error(`Failed to load ${category} index:`, err);
        toast.error(`Failed to load ${category} types`);
      });
  }, [category, typeParam]);

  const selectedTypeInfo = categoryIndex?.types.find(
    (t) => t.key === selectedType,
  );

  const handleSubmit = async (data: unknown) => {
    const savePromise = async () => {
      const config = await fetchConfig();
      const updatedConfig = await mergeFormDataIntoConfig(
        config,
        category,
        data,
      );
      await updateConfig(updatedConfig);
    };

    toast
      .promise(savePromise(), {
        loading: `${idParam ? "Updating" : "Creating"} ${categoryConfig.displayName}...`,
        success: `${categoryConfig.displayName} ${idParam ? "updated" : "created"} successfully!`,
        error: (err) =>
          err?.message || `Failed to save ${categoryConfig.displayName}`,
      })
      .then(() => {
        navigate(categoryConfig.basePath);
      });
  };

  const handleBack = () => {
    navigate(categoryConfig.basePath);
  };

  const handleTypeChange = (newType: unknown) => {
    if (typeof newType === "string") {
      setSelectedType(newType);
      // Update URL with new type
      const newParams = new URLSearchParams(searchParams);
      newParams.set("type", newType);
      navigate(`?${newParams.toString()}`, { replace: true });
    }
  };

  return (
    <Container>
      <Header>
        <Button icon={<ArrowLeftOutlined />} onClick={handleBack}>
          Back to {categoryConfig.pluralName}
        </Button>
        <Typography.Title level={2} style={{ margin: 0, flex: 1 }}>
          {idParam ? "Edit" : "Create"} {categoryConfig.displayName}
        </Typography.Title>
      </Header>

      <Card>
        <Space direction="vertical" style={{ width: "100%" }} size="large">
          <div>
            <Typography.Text
              strong
              style={{ display: "block", marginBottom: 8 }}
            >
              {categoryConfig.displayName} Type
            </Typography.Text>
            <StyledSelect
              placeholder={`Select ${category} type`}
              style={{ width: "100%" }}
              value={selectedType}
              onChange={handleTypeChange}
              disabled={!!idParam}
              options={categoryIndex?.types.map((type) => ({
                label: type.displayName,
                value: type.key,
              }))}
            />
          </div>

          {selectedTypeInfo && (
            <Card size="small" type="inner">
              <Typography.Text strong>
                {selectedTypeInfo.displayName}
              </Typography.Text>
              <Typography.Paragraph
                type="secondary"
                style={{ marginBottom: 0, marginTop: 4 }}
              >
                {selectedTypeInfo.description}
              </Typography.Paragraph>
            </Card>
          )}
        </Space>
      </Card>

      {selectedType && (
        <Card title="Configuration">
          <SchemaForm
            category={category}
            schemaType={selectedType}
            onSubmit={handleSubmit}
            initialData={undefined}
          />
        </Card>
      )}
    </Container>
  );
};
