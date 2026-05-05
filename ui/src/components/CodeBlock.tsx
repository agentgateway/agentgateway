import { Button } from "antd";
import { Check, Copy } from "lucide-react";
import { useState } from "react";
export function CodeBlock({ code }: { code: string }) {
    const [copied, setCopied] = useState(false);

    const handleCopy = async () => {
        await navigator.clipboard.writeText(code);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    return (
        <div style={{ position: "relative" }}>
            <Button
                size="small"
                type="text"
                icon={copied ? <Check size={16} /> : <Copy size={16} />}
                onClick={handleCopy}
                style={{ 
                    position: "absolute", 
                    top: 4, 
                    right: 4,
                    padding: 1,
                }}
            />
            <pre style={{
                margin: 0,
                padding: "8px 36px 8px 12px",
                fontSize: 12,
                background: "rgba(0,0,0,0.04)",
                borderRadius: 4,
                overflow: "auto",
            }}>
                {code}
            </pre>
        </div>
    );
}