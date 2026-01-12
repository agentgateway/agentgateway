"use client";

import { useMemo, useState } from "react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Listener, TLSVersion } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Check, ChevronsUpDown, X } from "lucide-react";
import { ALL_CIPHER_SUITES, TLS12_CIPHER_SUITES, TLS13_CIPHER_SUITES } from "@/lib/tls";

interface TLSConfigFormProps {
  listener: Listener | null;
  onSave: (updatedListener: Listener) => void;
  onCancel: () => void;
}

export function TLSConfigForm({ listener, onSave, onCancel }: TLSConfigFormProps) {
  // Radix/shadcn Select items cannot have empty-string values.
  // We use a sentinel value to represent "Default" (i.e., clear the selection).
  const DEFAULT_TLS_VERSION_SELECT_VALUE = "__default__";

  const [config, setConfig] = useState({
    certFile: listener?.tls?.cert || "",
    keyFile: listener?.tls?.key || "",
    rootFile: listener?.tls?.root || "",
    cipherSuites: listener?.tls?.cipherSuites || [],
    minTLSVersion: listener?.tls?.minTLSVersion || "",
    maxTLSVersion: listener?.tls?.maxTLSVersion || "",
  });

  const [cipherSuitePopoverOpen, setCipherSuitePopoverOpen] = useState(false);
  const [cipherSuiteSearch, setCipherSuiteSearch] = useState("");

  const cipherSuitesValidation = useMemo(() => {
    const canonicalSet = new Set<string>(ALL_CIPHER_SUITES);
    const seen = new Set<string>();
    const invalidInputs: string[] = [];
    const canonicalCipherSuites: string[] = [];

    for (const raw of config.cipherSuites) {
      if (!canonicalSet.has(raw)) {
        invalidInputs.push(raw);
        continue;
      }
      if (seen.has(raw)) continue;
      seen.add(raw);
      canonicalCipherSuites.push(raw);
    }

    return { invalidInputs, canonicalCipherSuites };
  }, [config.cipherSuites]);

  const handleSave = () => {
    if (!listener) return;

    if (cipherSuitesValidation.invalidInputs.length) return;
    const cipherSuites = cipherSuitesValidation.canonicalCipherSuites;

    const updatedListener: Listener = {
      ...listener,
      tls: {
        cert: config.certFile,
        key: config.keyFile,
        ...(config.rootFile ? { root: config.rootFile } : {}),
        ...(cipherSuites.length ? { cipherSuites } : {}),
        ...(config.minTLSVersion ? { minTLSVersion: config.minTLSVersion as TLSVersion } : {}),
        ...(config.maxTLSVersion ? { maxTLSVersion: config.maxTLSVersion as TLSVersion } : {}),
      },
    };

    onSave(updatedListener);
  };

  const invalidCipherSuiteBadges = cipherSuitesValidation.invalidInputs.slice(0, 6);
  const hasInvalidCipherSuites = cipherSuitesValidation.invalidInputs.length > 0;

  const canonicalSelectedSet = useMemo(() => {
    return new Set<string>(cipherSuitesValidation.canonicalCipherSuites);
  }, [cipherSuitesValidation.canonicalCipherSuites]);

  const selectedCanonicalSuites = cipherSuitesValidation.canonicalCipherSuites;

  return (
    <div className="space-y-4 py-4">
      <div className="space-y-2">
        <Label htmlFor="cert-file">Certificate File Path</Label>
        <Input
          id="cert-file"
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
          value={config.keyFile}
          onChange={(e) => setConfig({ ...config, keyFile: e.target.value })}
          placeholder="/path/to/key.pem"
        />
        <p className="text-xs text-muted-foreground">Path to the TLS private key file.</p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="root-file">Client CA Root (Optional)</Label>
        <Input
          id="root-file"
          value={config.rootFile}
          onChange={(e) => setConfig({ ...config, rootFile: e.target.value })}
          placeholder="/path/to/ca.pem"
        />
        <p className="text-xs text-muted-foreground">
          If set, the gateway will require client certificates and verify them against this CA.
        </p>
      </div>

      <div className="space-y-2">
        <Label htmlFor="cipher-suites">Cipher Suites (Optional)</Label>
        <div className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            Select one or more cipher suites. If unset, the default provider cipher list is used.
          </p>

          <div className="flex flex-wrap items-start gap-2">
            <Popover open={cipherSuitePopoverOpen} onOpenChange={setCipherSuitePopoverOpen}>
              <PopoverTrigger asChild>
                <Button
                  variant="outline"
                  role="combobox"
                  aria-expanded={cipherSuitePopoverOpen}
                  className="justify-between font-normal hover:bg-transparent"
                >
                  Select cipher suites…
                  <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                </Button>
              </PopoverTrigger>
              <PopoverContent className="w-[360px] p-0" align="start">
                <Command shouldFilter={false}>
                  <CommandInput
                    placeholder="Search cipher suites…"
                    value={cipherSuiteSearch}
                    onValueChange={setCipherSuiteSearch}
                  />
                  <CommandList>
                    <CommandEmpty>No cipher suite found.</CommandEmpty>
                    <CommandGroup heading="TLS 1.3">
                      {TLS13_CIPHER_SUITES.filter((name) =>
                        name.toLowerCase().includes(cipherSuiteSearch.toLowerCase())
                      ).map((name) => (
                        <CommandItem
                          key={name}
                          value={name}
                          onSelect={(currentValue) => {
                            const suite = currentValue.toUpperCase();
                            setConfig((prev) => {
                              const next = new Set(prev.cipherSuites.map((s) => s.toUpperCase()));
                              if (next.has(suite)) {
                                next.delete(suite);
                              } else {
                                next.add(suite);
                              }
                              return {
                                ...prev,
                                cipherSuites: Array.from(next),
                              };
                            });
                          }}
                        >
                          <Check
                            className={cn(
                              "mr-2 h-4 w-4",
                              canonicalSelectedSet.has(name) ? "opacity-100" : "opacity-0"
                            )}
                          />
                          {name}
                        </CommandItem>
                      ))}
                    </CommandGroup>
                    <CommandGroup heading="TLS 1.2">
                      {TLS12_CIPHER_SUITES.filter((name) =>
                        name.toLowerCase().includes(cipherSuiteSearch.toLowerCase())
                      ).map((name) => (
                        <CommandItem
                          key={name}
                          value={name}
                          onSelect={(currentValue) => {
                            const suite = currentValue.toUpperCase();
                            setConfig((prev) => {
                              const next = new Set(prev.cipherSuites.map((s) => s.toUpperCase()));
                              if (next.has(suite)) {
                                next.delete(suite);
                              } else {
                                next.add(suite);
                              }
                              return {
                                ...prev,
                                cipherSuites: Array.from(next),
                              };
                            });
                          }}
                        >
                          <Check
                            className={cn(
                              "mr-2 h-4 w-4",
                              canonicalSelectedSet.has(name) ? "opacity-100" : "opacity-0"
                            )}
                          />
                          {name}
                        </CommandItem>
                      ))}
                    </CommandGroup>
                  </CommandList>
                </Command>
              </PopoverContent>
            </Popover>

            {selectedCanonicalSuites.length > 0 && (
              <div className="flex flex-wrap items-center gap-2">
                {selectedCanonicalSuites.map((s) => (
                  <Badge key={s} variant="secondary" className="gap-1">
                    {s}
                    <button
                      type="button"
                      className="ml-1 inline-flex items-center rounded-sm outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={() => {
                        setConfig((prev) => ({
                          ...prev,
                          cipherSuites: prev.cipherSuites.filter((x) => x.toUpperCase() !== s),
                        }));
                      }}
                      aria-label={`Remove ${s}`}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </Badge>
                ))}
                <Button
                  type="button"
                  variant="ghost"
                  className="h-7 px-2 text-xs"
                  onClick={() => setConfig((prev) => ({ ...prev, cipherSuites: [] }))}
                >
                  Clear
                </Button>
              </div>
            )}

            {hasInvalidCipherSuites && (
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-xs text-destructive">Unsupported:</span>
                {invalidCipherSuiteBadges.map((s) => (
                  <Badge key={s} variant="destructive" className="gap-1">
                    {s}
                    <button
                      type="button"
                      className="ml-1 inline-flex items-center rounded-sm outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                      onClick={() => {
                        setConfig((prev) => ({
                          ...prev,
                          cipherSuites: prev.cipherSuites.filter((x) => x !== s),
                        }));
                      }}
                      aria-label={`Remove ${s}`}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </Badge>
                ))}
                {cipherSuitesValidation.invalidInputs.length > invalidCipherSuiteBadges.length && (
                  <span className="text-xs text-destructive">
                    +{cipherSuitesValidation.invalidInputs.length - invalidCipherSuiteBadges.length}{" "}
                    more
                  </span>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="grid grid-cols-2 gap-4">
        <div className="space-y-2">
          <Label htmlFor="min-tls-version">Min TLS Version (Optional)</Label>
          <Select
            value={config.minTLSVersion}
            onValueChange={(value) =>
              setConfig({
                ...config,
                minTLSVersion: value === DEFAULT_TLS_VERSION_SELECT_VALUE ? "" : value,
              })
            }
          >
            <SelectTrigger id="min-tls-version">
              <SelectValue placeholder="Default" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={DEFAULT_TLS_VERSION_SELECT_VALUE}>Default</SelectItem>
              <SelectItem value={TLSVersion.TLS_V1_2}>TLS 1.2</SelectItem>
              <SelectItem value={TLSVersion.TLS_V1_3}>TLS 1.3</SelectItem>
            </SelectContent>
          </Select>
        </div>

        <div className="space-y-2">
          <Label htmlFor="max-tls-version">Max TLS Version (Optional)</Label>
          <Select
            value={config.maxTLSVersion}
            onValueChange={(value) =>
              setConfig({
                ...config,
                maxTLSVersion: value === DEFAULT_TLS_VERSION_SELECT_VALUE ? "" : value,
              })
            }
          >
            <SelectTrigger id="max-tls-version">
              <SelectValue placeholder="Default" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value={DEFAULT_TLS_VERSION_SELECT_VALUE}>Default</SelectItem>
              <SelectItem value={TLSVersion.TLS_V1_2}>TLS 1.2</SelectItem>
              <SelectItem value={TLSVersion.TLS_V1_3}>TLS 1.3</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      <div className="flex justify-end space-x-2 pt-4">
        <Button variant="outline" onClick={onCancel}>
          Cancel
        </Button>
        <Button onClick={handleSave} disabled={hasInvalidCipherSuites}>
          Save Changes
        </Button>
      </div>
    </div>
  );
}
