import { DeleteOutlined } from "@ant-design/icons";
import styled from "@emotion/styled";
import { App, Button, Dropdown, Empty, InputNumber, Tag, Tooltip } from "antd";
import { Check, Edit2, EllipsisVertical, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import toast from "react-hot-toast";
import { useLocation, useNavigate } from "react-router-dom";
import { useConfig } from "../../api";
import { fetchConfig, updateConfig } from "../../api/config";
import { deleteLLM, removeBind, removeLLMModelByIndex, removeRouteBackendByIndex, removeTCPRouteBackendByIndex } from "../../api/crud";
import { useXdsMode } from "../../api/hooks";
import { PROVIDER_COLORS } from "./Playground/constants";
import type { PlaygroundModel } from "./Playground/types";

type TopLevelLLMModel = PlaygroundModel & {
    source: "topLevel";
    modelIndex: number;
};

type PortBindLLMModel = PlaygroundModel & {
    source: "portBind";
    bindPort: number;
    listenerIndex: number;
    routeIndex: number;
    isTcpRoute: boolean;
    backendIndex: number;
};

type LLMDisplayModel = TopLevelLLMModel | PortBindLLMModel;

function buildLLMDisplayModels(config: any): LLMDisplayModel[] {
    const models: LLMDisplayModel[] = [];

    if (config?.llm?.models) {
        (config.llm.models as any[]).forEach((m, modelIndex) => {
            models.push({
                label: m.name ?? "unnamed",
                defaultModel: m.params?.model ?? "",
                provider: m.provider ?? "unknown",
                baseUrl: m.params?.baseUrl,
                source: "topLevel",
                modelIndex,
            });
        });
    }

    if (config?.binds) {
        const seen = new Set<string>();
        (config.binds as any[]).forEach((bind, _bi) => {
            const bindPort = bind.port;
            (bind.listeners ?? []).forEach((listener: any, listenerIndex: number) => {
                [
                    { routes: listener.routes ?? [], isTcpRoute: false },
                    { routes: listener.tcpRoutes ?? [], isTcpRoute: true },
                ].forEach(({ routes, isTcpRoute }) => {
                    (routes as any[]).forEach((route, routeIndex) => {
                        (route.backends ?? []).forEach((backend: any, backendIndex: number) => {
                            const ai = backend.ai;
                            if (!ai) return;
                            const providers = ai.groups
                                ? ai.groups.flatMap((g: any) => g.providers ?? [])
                                : [ai];
                            for (const p of providers) {
                                const providerEntry = p.provider ?? p;
                                for (const [providerName, providerConfig] of Object.entries(providerEntry as any)) {
                                    const model = (providerConfig as any)?.model;
                                    const label = ai.name ?? model;
                                    if (!label || seen.has(label)) continue;
                                    seen.add(label);
                                    models.push({
                                        label,
                                        defaultModel: model,
                                        provider: providerName,
                                        baseUrl: `http://${ai.hostOverride}`,
                                        source: "portBind",
                                        bindPort,
                                        listenerIndex,
                                        routeIndex,
                                        isTcpRoute,
                                        backendIndex,
                                    });
                                }
                            }
                        });
                    });
                });
            });
        });
    }

    return models;
}

const PageRoot = styled.div`
  display: flex;
  flex-direction: column;
  height: calc(100vh - 64px);
  overflow-y: auto;
  padding: var(--spacing-lg);
  gap: var(--spacing-lg);
`;

const PortRow = styled.div`
  display: flex;
  align-items: center;
  gap: var(--spacing-sm);
  font-size: 14px;
  color: var(--color-text-secondary);
`;

const PortValue = styled.span`
  font-weight: 600;
  color: var(--color-text-base);
`;

const SectionTitle = styled.h2`
  font-size: 16px;
  font-weight: 600;
  color: var(--color-text-base);
  margin: 0 0 var(--spacing-md) 0;
`;

const ModelsSection = styled.div`
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
`;

const ModelGridScroll = styled.div`
  max-height: calc(100vh - 260px);
  overflow-y: auto;
  margin-top: var(--spacing-lg);
`;

const ModelGrid = styled.div`
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: var(--spacing-lg);
`;

const ModelCard = styled.div`
  border-radius: var(--border-radius-md);
  padding: var(--spacing-lg);
  background: var(--color-bg-container);
  display: flex;
  flex-direction: column;
  gap: var(--spacing-md);
  border: 1px solid rgba(0, 0, 0, 0.3);
`;

const ModelName = styled.div`
  font-weight: 600;
  font-size: 15px;
  color: var(--color-text-base);
`;

const ModelMeta = styled.div`
  font-size: 13px;
  color: var(--color-text-secondary);
`;

const MetaRow = styled.div`
  display: flex;
  justify-content: space-between;
  align-items: center;
`;

const MetaLabel = styled.span`
  font-size: 13px;
  color: var(--color-text-secondary);
`;

const CardActions = styled.div`
  display: flex;
  gap: var(--spacing-sm);
  justify-content: space-between;
  margin-top: auto;
`;

const PageHeader = styled.div`
  display: flex;
  justify-content: flex-end;
  align-items: center;
`;

const EditIconButton = styled(Button)`
  && {
    color: var(--color-text-tertiary, #999);
    &:hover {
      color: var(--color-text-base) !important;
      background: var(--color-bg-elevated, rgba(0,0,0,0.06)) !important;
    }
  }
`;

const PillTag = styled(Tag)`
  && {
    border-radius: 10px !important;
    margin: 0;
    padding: 0 8px;
  }
`;

const PageSubtitle = styled.p`
  margin: 0;
  color: var(--color-text-secondary);
  font-size: 14px;
`;

function OverflowPill({ text, color }: { text: string; color: string }) {
  const ref = useRef<HTMLElement>(null);
  const [overflows, setOverflows] = useState(false);

  useEffect(() => {
    if (ref.current) setOverflows(ref.current.scrollWidth > ref.current.clientWidth);
  }, [text]);

  const pill = (
    <PillTag color={color} ref={ref} style={{ maxWidth: 160, display: 'inline-block', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
      {text}
    </PillTag>
  );

  return overflows ? <Tooltip title={text}>{pill}</Tooltip> : pill;
}

// Main Component
export function LLMOverviewPage() {
    const { modal } = App.useApp();
    const { data: fullConfig, mutate, isLoading, error } = useConfig();
    const navigate = useNavigate();
    const location = useLocation();
    const { xdsMode } = useXdsMode();

    const [isEditingPort, setIsEditingPort] = useState(false);
    const [editPortValue, setEditPortValue] = useState<number | null>(null);
    const [isSavingPort, setIsSavingPort] = useState(false);

    const models: LLMDisplayModel[] = buildLLMDisplayModels(fullConfig);

    const port: number | null = (fullConfig as any)?.llm?.port ?? null;

    const handlePortEditStart = () => {
        setIsEditingPort(true);
        setEditPortValue(port);
    };

    const handlePortEditCancel = () => {
        setIsEditingPort(false);
        setEditPortValue(null);
    };

    const handlePortEditSave = async () => {
        if (editPortValue === null) return;
        setIsSavingPort(true);
        try {
            const config = await fetchConfig();
            if (config.llm) (config.llm as any).port = editPortValue;
            await updateConfig(config);
            await mutate();
            setIsEditingPort(false);
            setEditPortValue(null);
        } catch (err: any) {
            toast.error(err.message ?? "Failed to update port");
        } finally {
            setIsSavingPort(false);
        }
    };

    const handleDeleteModel = async (model: LLMDisplayModel) => {
        const isLastModel = models.length === 1;
        try {
            if (model.source === "topLevel") {
                const topLevelCount = models.filter(m => m.source === "topLevel").length;
                if (topLevelCount === 1) {
                    await deleteLLM();
                } else {
                    await removeLLMModelByIndex(model.modelIndex);
                }
            } else {
                const aiBackendsInBind = models.filter(m => m.source === "portBind" && m.bindPort === model.bindPort).length;
                if (aiBackendsInBind === 1) {
                    await removeBind(model.bindPort);
                } else if (model.isTcpRoute) {
                    await removeTCPRouteBackendByIndex(model.bindPort, model.listenerIndex, model.routeIndex, model.backendIndex);
                } else {
                    await removeRouteBackendByIndex(model.bindPort, model.listenerIndex, model.routeIndex, model.backendIndex);
                }
            }
            await mutate();
            if (isLastModel) {
                navigate("/llm-setup-wizard", { replace: true });
            }
        } catch (err: any) {
            toast.error(err.message ?? "Failed to delete model");
        }
    };

    useEffect(() => {
        const skipRedirect = (location.state as { skipWizardRedirect?: boolean } | null)?.skipWizardRedirect || xdsMode;
        if (!isLoading && !error && models.length === 0 && !skipRedirect) {
            navigate("/llm-setup-wizard", { replace: true });
        }
    }, [isLoading, error, models.length, navigate, location.state]);

    return (
        <PageRoot>
            <PageSubtitle>
              View your configured LLM models and their settings.
            </PageSubtitle>
            <ModelsSection>
                {models.length === 0 && (
                  <div style={{ textAlign: "center", padding: 60 }}>
                    <Empty description="No models configured. Add an LLM configuration to get started." />
                  </div>
                )}
                {models.length > 0 && (
                  <>
                    <div style={{ display: 'flex', justifyContent: 'space-between' }}> 
                      <SectionTitle>Models</SectionTitle>
                      {!xdsMode && (
                        <Button
                            type="primary"
                            onClick={() => navigate("/llm-setup-wizard")}
                        >
                            Add Model
                        </Button>
                      )}
                    </div>
                    <div>
                      {port !== null && (
                          <PortRow>
                              agentgateway exposed port:{" "}
                              <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                                  {isEditingPort? (
                                      <>
                                          <InputNumber
                                              size="small"
                                              min={1}
                                              max={65535}
                                              precision={0}
                                              value={editPortValue}
                                              onChange={(v) => setEditPortValue(v)}
                                              style={{ width: 90 }}
                                              autoFocus
                                              onPressEnter={handlePortEditSave}
                                          />
                                          <Button
                                              type="text"
                                              size="small"
                                              icon={<Check size={13} color="var(--color-success)" strokeWidth={4} />}
                                              loading={isSavingPort}
                                              onClick={handlePortEditSave}
                                          />
                                          <Button
                                              type="text"
                                              size="small"
                                              icon={<X size={13} color="var(--color-error)" strokeWidth={4} />}
                                              onClick={handlePortEditCancel}
                                              disabled={isSavingPort}
                                          />
                                      </>
                                  ) : (
                                      <>
                                          <PortValue><PillTag color="blue">{port}</PillTag></PortValue>
                                          <Tooltip title="Edit">
                                              <EditIconButton
                                                  type="text"
                                                  size="small"
                                                  icon={<Edit2 size={15} />}
                                                  onClick={handlePortEditStart}
                                              />
                                          </Tooltip>
                                      </>
                                  )}
                              </span>
                          </PortRow>
                      )}
                    </div>
                    <ModelGridScroll>
                        <ModelGrid>
                            {models.map((m, i) => (
                                <ModelCard key={i} data-testid={`llm-model-card-${i}`}>
                                    <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                                        <ModelName>{m.label}</ModelName>
                                        {!xdsMode && (
                                          <Dropdown
                                            menu={{
                                              items: [
                                                {
                                                  key: "delete",
                                                  label: "Delete",
                                                  icon: <DeleteOutlined />,
                                                  onClick: () => modal.confirm({
                                                    title: "Delete Model?",
                                                    content: <span>Are you sure you want to delete <b>{m.label}</b>?</span>,
                                                    okText: "Delete",
                                                    okButtonProps: { danger: true },
                                                    onOk: () => handleDeleteModel(m),
                                                  }),
                                                },
                                              ],
                                            }}
                                            trigger={["click"]}
                                          >
                                            <EllipsisVertical size={15} style={{ cursor: "pointer" }} />
                                          </Dropdown>
                                        )}
                                    </div>
                                    <ModelMeta style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
                                      <MetaRow>
                                        <MetaLabel>Provider</MetaLabel>
                                        <PillTag color={PROVIDER_COLORS[m.provider]}>{m.provider}</PillTag>
                                      </MetaRow>
                                      <MetaRow>
                                        <MetaLabel>Model</MetaLabel>
                                        <PillTag color="gold">{m.defaultModel}</PillTag>
                                      </MetaRow>
                                      <MetaRow>
                                        <MetaLabel>Host</MetaLabel>
                                        <OverflowPill text={m.baseUrl} color="purple" />
                                      </MetaRow>
                                    </ModelMeta>
                                    <CardActions>
                                        <Button
                                            size="small"
                                            onClick={() => navigate(`/llm-playground?modelName=${encodeURIComponent(xdsMode ? m.defaultModel : m.label)}`)}
                                            data-testid={`llm-model-card-${i}-open-playground-button`}
                                        >
                                            Open Playground
                                        </Button>
                                        {/* <Button
                                            size="small"
                                            onClick={() => navigate("/traffic-configuration/editor")}
                                            data-testid={`llm-model-card-${i}-raw-editor-button`}
                                        >
                                            Raw Editor
                                        </Button> */}
                                    </CardActions>
                                </ModelCard>
                            ))}
                        </ModelGrid>
                    </ModelGridScroll>
                  </>
                )}
            </ModelsSection>
        </PageRoot>
    );
}