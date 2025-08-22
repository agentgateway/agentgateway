"use client";
import React, { useEffect, useState } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "sonner";
import { formatDistanceToNow } from "date-fns";
import { Loader2, Trash2, Copy, Shield } from "lucide-react";

interface TokenRecord {
  id: string; // internal only (not displayed directly)
  token_prefix: string;
  // scopes removed (currently unused); backend may omit field
  created_at: string;
  expires_at: string | null;
  revoked_at: string | null;
  last_used_at: string | null;
}

interface CreateResponse {
  token: string;
  token_record: TokenRecord;
}

export default function TokensPage() {
  const [tokens, setTokens] = useState<TokenRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [newExpiry, setNewExpiry] = useState("");
  const [showToken, setShowToken] = useState<CreateResponse | null>(null);

  const load = async () => {
    setLoading(true);
    try {
      const res = await fetch("/api/tokens", { credentials: "include" });
      if (res.ok) {
        const data = await res.json();
        setTokens(data);
      } else if (res.status === 401) {
        toast.error("Authenticate to view tokens");
      } else if (res.status === 503) {
        toast.error("Token management unavailable (DB not configured)");
      } else {
        toast.error("Failed to load tokens");
      }
    } catch (e) {
      console.error(e);
      toast.error("Network error loading tokens");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const handleCreate = async () => {
    setCreating(true);
    try {
      const body = {
        expires_at: newExpiry ? new Date(newExpiry).toISOString() : null,
      };
      const res = await fetch("/api/tokens", {
        method: "POST",
        credentials: "include",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (res.ok) {
        const data: CreateResponse = await res.json();
        setTokens((prev) => [data.token_record, ...prev]);
        setShowToken(data);
        setNewExpiry("");
        toast.success("Token created");
      } else if (res.status === 401) {
        toast.error("Authenticate to create tokens");
      } else if (res.status === 403) {
        toast.error("Forbidden");
      } else {
        const err = await safeError(res);
        toast.error(err || "Create failed");
      }
    } catch (e) {
      console.error(e);
      toast.error("Create failed");
    } finally {
      setCreating(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      const res = await fetch(`/api/tokens/${id}`, { method: "DELETE", credentials: "include" });
      if (res.status === 204) {
        setTokens((prev) => prev.filter((t) => t.id !== id));
        toast.success("Token revoked");
      } else if (res.status === 401) {
        toast.error("Authenticate to revoke token");
      } else if (res.status === 403) {
        toast.error("Forbidden");
      } else {
        toast.error("Revoke failed");
      }
    } catch (e) {
      console.error(e);
      toast.error("Revoke failed");
    }
  };

  const copy = (value: string) => {
    navigator.clipboard.writeText(value);
    toast.success("Copied");
  };

  return (
    <div className="container mx-auto py-8 px-4 space-y-8">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Shield className="h-5 w-5" /> Personal Access Tokens
          </CardTitle>
          <CardDescription>
            Manage scoped access tokens. Prefixes only; full secret shown once at creation.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-6">
          <div className="flex flex-col md:flex-row gap-4 items-end">
            <div>
              <label className="block text-sm font-medium mb-1">Expires At (optional)</label>
              <Input
                type="datetime-local"
                value={newExpiry}
                onChange={(e) => setNewExpiry(e.target.value)}
              />
            </div>
            <Button onClick={handleCreate} disabled={creating}>
              {creating && <Loader2 className="h-4 w-4 mr-2 animate-spin" />}Create
            </Button>
          </div>

          {loading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" /> Loading tokens...
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Prefix</TableHead>
                    <TableHead>Created</TableHead>
                    <TableHead>Last Used</TableHead>
                    <TableHead>Expires</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="w-24">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {tokens.map((t) => (
                    <TableRow key={t.id} className={t.revoked_at ? "opacity-60" : ""}>
                      <TableCell className="font-mono text-xs">{t.token_prefix}</TableCell>
                      <TableCell className="text-xs">
                        {formatDistanceToNow(new Date(t.created_at), { addSuffix: true })}
                      </TableCell>
                      <TableCell className="text-xs">
                        {t.last_used_at
                          ? formatDistanceToNow(new Date(t.last_used_at), { addSuffix: true })
                          : "—"}
                      </TableCell>
                      <TableCell className="text-xs">
                        {t.expires_at
                          ? formatDistanceToNow(new Date(t.expires_at), { addSuffix: true })
                          : "—"}
                      </TableCell>
                      <TableCell className="text-xs">
                        {t.revoked_at
                          ? "Revoked"
                          : t.expires_at && new Date(t.expires_at) < new Date()
                            ? "Expired"
                            : "Active"}
                      </TableCell>
                      <TableCell className="flex gap-2">
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => copy(t.token_prefix)}
                          title="Copy prefix"
                        >
                          <Copy className="h-3 w-3" />
                        </Button>
                        {!t.revoked_at && (
                          <Button
                            variant="destructive"
                            size="sm"
                            onClick={() => handleDelete(t.id)}
                            title="Revoke"
                          >
                            <Trash2 className="h-3 w-3" />
                          </Button>
                        )}
                      </TableCell>
                    </TableRow>
                  ))}
                  {tokens.length === 0 && !loading && (
                    <TableRow>
                      <TableCell colSpan={7} className="text-center text-sm text-muted-foreground">
                        No tokens
                      </TableCell>
                    </TableRow>
                  )}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      <Dialog open={!!showToken} onOpenChange={(o) => !o && setShowToken(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Copy your token now</DialogTitle>
            <DialogDescription>
              This is the only time the full token will be shown. Store it securely.
            </DialogDescription>
          </DialogHeader>
          {showToken && (
            <div className="space-y-3">
              <div>
                <label className="text-xs font-medium">Full Token</label>
                <div className="mt-1 p-2 bg-muted rounded font-mono text-xs break-all">
                  {showToken.token}
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  className="mt-2"
                  onClick={() => copy(showToken.token)}
                >
                  Copy Token
                </Button>
              </div>
              <div className="text-xs text-muted-foreground">
                Prefix: {showToken.token_record.token_prefix}
              </div>
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

async function safeError(res: Response): Promise<string | null> {
  try {
    const data = await res.json();
    return data.error || null;
  } catch {
    return null;
  }
}
