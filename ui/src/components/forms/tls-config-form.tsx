"use client";

import { useState } from "react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Listener } from "@/lib/types";

interface TLSConfigFormProps {
  listener: Listener | null;
  onSave: (updatedListener: Listener) => void;
  onCancel: () => void;
}

export function TLSConfigForm({ listener, onSave, onCancel }: TLSConfigFormProps) {
  const [config, setConfig] = useState({
    certFile: listener?.tls?.cert || "",
    keyFile: listener?.tls?.key || "",
  });

  const handleSave = () => {
    if (!listener) return;

    const updatedListener: Listener = {
      ...listener,
      tls: {
        cert: config.certFile,
        key: config.keyFile,
      },
    };

    onSave(updatedListener);
  };

  return (
    <div className="space-y-4 py-4">
      <div className="space-y-2">
        <Label htmlFor="cert-file">Certificate File Path</Label>
        <Input
          id="cert-file"
          data-cy="tls-cert-file-input"
          value={config.certFile}
          onChange={(e) => setConfig({ ...config, certFile: e.target.value })}
          placeholder="/path/to/cert.pem"
        />
        <p className="text-xs text-muted-foreground">Path to the TLS certificate file.</p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="key-file">Key File Path</Label>
        <Input
          id="key-file"
          data-cy="tls-key-file-input"
          value={config.keyFile}
          onChange={(e) => setConfig({ ...config, keyFile: e.target.value })}
          placeholder="/path/to/key.pem"
        />
        <p className="text-xs text-muted-foreground">Path to the TLS private key file.</p>
      </div>

      <div className="flex justify-end space-x-2 pt-4">
        <Button variant="outline" onClick={onCancel} data-cy="tls-config-cancel-button">
          Cancel
        </Button>
        <Button onClick={handleSave} data-cy="tls-config-save-button">
          Save Changes
        </Button>
      </div>
    </div>
  );
}
