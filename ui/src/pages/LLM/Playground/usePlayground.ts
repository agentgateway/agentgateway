import { useCallback, useRef, useState } from "react";
import { useConfig } from "../../../api";
import { extractModels } from "./extractModels";
import type { Message, PlaygroundModel } from "./types";

export function usePlayground() {
  const { data: config, isLoading } = useConfig();
  const [selectedLabel, setSelectedLabel] = useState<string | null>(null);
  const [modelOverride, setModelOverride] = useState("");
  const [prompt, setPrompt] = useState("");
  const [messages, setMessages] = useState<Message[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chatEndRef = useRef<HTMLDivElement>(null);

  const models: PlaygroundModel[] = config ? extractModels(config) : [];
  const selectedModel = models.find((m) => m.label === selectedLabel) ?? null;
  /** The model name actually sent in the request body */
  const effectiveModel =
    modelOverride.trim() || selectedModel?.defaultModel || selectedModel?.label || "";

  const scrollToBottom = useCallback(() => {
    setTimeout(() => chatEndRef.current?.scrollIntoView({ behavior: "smooth" }), 50);
  }, []);

  const handleSend = useCallback(async () => {
    if (!prompt.trim() || !selectedModel) return;
    setError(null);

    const userMsg: Message = { role: "user", content: prompt.trim() };
    setMessages((prev) => [...prev, userMsg]);
    setPrompt("");
    setSending(true);
    scrollToBottom();

    try {
      const allMessages = [...messages, userMsg].map((m) => ({
        role: m.role,
        content: m.content,
      }));

      const res = await fetch(`${selectedModel.baseUrl}/v1/chat/completions`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: effectiveModel,
          messages: allMessages,
        }),
      });

      if (!res.ok) {
        const text = await res.text();
        throw new Error(text || `HTTP ${res.status}`);
      }

      const data = await res.json();
      const content = data.choices?.[0]?.message?.content ?? "(empty response)";

      setMessages((prev) => [
        ...prev,
        { role: "assistant", content },
      ]);
    } catch (e: unknown) {
      const msg =
        e && typeof e === "object" && "message" in e
          ? (e as any).message
          : "Failed to get response";
      setError(msg);
      // Remove the user message if we failed
      setMessages((prev) => prev.slice(0, -1));
      setPrompt(userMsg.content);
    } finally {
      setSending(false);
      scrollToBottom();
    }
  }, [prompt, selectedModel, effectiveModel, messages, scrollToBottom]);

  const handleClear = useCallback(() => {
    setMessages([]);
    setPrompt("");
    setError(null);
  }, []);

  const handleSelectLabel = useCallback(
    (label: string) => {
      setSelectedLabel(label);
      const m = models.find((m) => m.label === label);
      setModelOverride(m?.defaultModel ?? "");
    },
    [models]
  );

  return {
    isLoading,
    models,
    selectedLabel,
    selectedModel,
    effectiveModel,
    modelOverride,
    prompt,
    messages,
    sending,
    error,
    chatEndRef,
    handleSend,
    handleClear,
    handleSelectLabel,
    setModelOverride,
    setPrompt,
  };
}
