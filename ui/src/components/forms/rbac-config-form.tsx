"use client";

import { useState, useEffect, MouseEvent } from "react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Listener, Rule, Matcher } from "@/lib/types";
import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Trash2, Plus, ChevronsUpDown, Check } from "lucide-react";
import { fetchMcpTargets, fetchA2aTargets } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion";

interface RBACConfigFormProps {
  listener: Listener | null;
  onSave: (updatedListener: Listener) => void;
  onCancel: () => void;
}

export function RBACConfigForm({ listener, onSave, onCancel }: RBACConfigFormProps) {
  const [rules, setRules] = useState<Rule[]>([]);
  const [allTargetNames, setAllTargetNames] = useState<string[]>([]);
  const [loadingTargets, setLoadingTargets] = useState(true);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [popoverOpenStates, setPopoverOpenStates] = useState<boolean[]>([]);
  const [activeAccordionValue, setActiveAccordionValue] = useState<string>(
    rules.length > 0 ? `rule-${rules.length - 1}` : ""
  );

  useEffect(() => {
    const loadTargets = async () => {
      setLoadingTargets(true);
      setFetchError(null);
      try {
        const [mcpTargets, a2aTargets] = await Promise.all([fetchMcpTargets(), fetchA2aTargets()]);
        const mcpNames = mcpTargets.map((t) => t.name).filter(Boolean);
        const a2aNames = a2aTargets.map((t) => t.name).filter(Boolean);
        const uniqueNames = Array.from(new Set([...mcpNames, ...a2aNames]));
        setAllTargetNames(uniqueNames);
      } catch (error) {
        console.error("Failed to fetch targets:", error);
        setFetchError("Failed to load target list.");
      } finally {
        setLoadingTargets(false);
      }
    };
    loadTargets();
  }, []);

  useEffect(() => {
    setPopoverOpenStates(Array(rules.length).fill(false));
  }, [rules.length]);

  useEffect(() => {
    const activeIndex = parseInt(activeAccordionValue.split("-")[1], 10);
    if (isNaN(activeIndex) || activeIndex >= rules.length) {
      setActiveAccordionValue(rules.length > 0 ? `rule-${rules.length - 1}` : "");
    }
    setPopoverOpenStates((prev) => {
      const newStates = Array(rules.length).fill(false);
      prev
        .slice(0, Math.min(prev.length, rules.length))
        .forEach((state, i) => (newStates[i] = state));
      return newStates;
    });
  }, [rules.length, activeAccordionValue]);

  const setPopoverOpen = (index: number, open: boolean) => {
    setPopoverOpenStates((prev) => prev.map((state, i) => (i === index ? open : state)));
  };

  const handleAddRule = () => {
    const newRuleIndex = rules.length;
    const newRule: Rule = {
      key: "",
      value: "",
      resource: {
        type: "TOOL",
        target: "",
        id: "",
      },
      matcher: "EQUALS" as Matcher,
    };
    setRules([...rules, newRule]);
    setActiveAccordionValue(`rule-${newRuleIndex}`);
    setPopoverOpenStates([...popoverOpenStates, false]);
  };

  const handleRemoveRule = (index: number, event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    setRules(rules.filter((_, i) => i !== index));
    setPopoverOpenStates(popoverOpenStates.filter((_, i) => i !== index));
  };

  const handleUpdateRule = (index: number, updates: Partial<Rule>) => {
    setRules(rules.map((rule, i) => (i === index ? { ...rule, ...updates } : rule)));
  };

  const handleSave = () => {
    if (!listener) return;

    // In the new schema, RBAC configuration is handled differently
    // For now, we'll just return the listener as-is since the schema structure has changed
    const updatedListener: Listener = {
      ...listener,
      // RBAC configuration will need to be handled through the new schema structure
    };

    onSave(updatedListener);
  };

  return (
    <div className="space-y-4 py-4">
      <div className="max-h-[400px] overflow-y-auto pr-2 space-y-2">
        <Accordion
          type="single"
          collapsible
          value={activeAccordionValue}
          onValueChange={setActiveAccordionValue}
          className="w-full"
        >
          {rules.map((rule, index) => (
            <AccordionItem key={index} value={`rule-${index}`}>
              <AccordionTrigger className="hover:no-underline">
                <div className="flex justify-between items-center w-full pr-4">
                  <h4 className="text-sm font-medium text-left">Rule {index + 1}</h4>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={(e) => handleRemoveRule(index, e)}
                    className="text-destructive hover:text-destructive hover:bg-transparent p-1 h-auto w-auto"
                    data-cy={`rbac-remove-rule-button-${index}`}
                  >
                    <Trash2 className="h-4 w-4" />
                  </Button>
                </div>
              </AccordionTrigger>
              <AccordionContent>
                <div className="space-y-4 p-1 pt-2">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <Label htmlFor={`rule-key-${index}`}>Claim Key</Label>
                      <Input
                        id={`rule-key-${index}`}
                        data-cy={`rbac-rule-key-input-${index}`}
                        value={rule.key}
                        onChange={(e) => handleUpdateRule(index, { key: e.target.value })}
                        placeholder="e.g., role"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor={`rule-value-${index}`}>Claim Value</Label>
                      <Input
                        id={`rule-value-${index}`}
                        data-cy={`rbac-rule-value-input-${index}`}
                        value={rule.value}
                        onChange={(e) => handleUpdateRule(index, { value: e.target.value })}
                        placeholder="e.g., admin"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div className="space-y-2">
                      <Label>Resource Type</Label>
                      <Input
                        value="Tool"
                        readOnly
                        disabled
                        className="bg-muted text-muted-foreground"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label>Matcher</Label>
                      <Input
                        value="Equals"
                        readOnly
                        disabled
                        className="bg-muted text-muted-foreground"
                      />
                    </div>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <div className="space-y-1">
                      <Label htmlFor={`resource-target-${index}`}>Resource Target</Label>
                      <Popover
                        open={popoverOpenStates[index] ?? false}
                        onOpenChange={(open) => setPopoverOpen(index, open)}
                      >
                        <PopoverTrigger asChild>
                          <Button
                            variant="outline"
                            role="combobox"
                            aria-expanded={popoverOpenStates[index] ?? false}
                            className="w-full justify-between font-normal hover:bg-transparent"
                            disabled={loadingTargets}
                            data-cy={`rbac-resource-target-button-${index}`}
                          >
                            {rule.resource.target ||
                              (loadingTargets ? "Loading..." : "Select or type target...")}
                            <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                          </Button>
                        </PopoverTrigger>
                        <PopoverContent className="w-[--radix-popover-trigger-width] p-0">
                          <Command shouldFilter={false}>
                            <CommandInput
                              placeholder="Search target or type name..."
                              value={rule.resource.target}
                              onValueChange={(search) => {
                                handleUpdateRule(index, {
                                  resource: { ...rule.resource, target: search },
                                });
                              }}
                            />
                            <CommandList>
                              <CommandEmpty>
                                {loadingTargets ? "Loading..." : fetchError || "No target found."}
                              </CommandEmpty>
                              <CommandGroup heading="Suggestions">
                                {allTargetNames
                                  .filter((name) =>
                                    name
                                      .toLowerCase()
                                      .includes(rule.resource.target?.toLowerCase() ?? "")
                                  )
                                  .map((name) => (
                                    <CommandItem
                                      key={name}
                                      value={name}
                                      onSelect={(currentValue) => {
                                        handleUpdateRule(index, {
                                          resource: { ...rule.resource, target: currentValue },
                                        });
                                        setPopoverOpen(index, false);
                                      }}
                                    >
                                      <Check
                                        className={cn(
                                          "mr-2 h-4 w-4",
                                          rule.resource.target === name
                                            ? "opacity-100"
                                            : "opacity-0"
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
                      {fetchError && <p className="text-xs text-destructive mt-1">{fetchError}</p>}
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor={`resource-id-${index}`}>Resource ID</Label>
                      <Input
                        id={`resource-id-${index}`}
                        data-cy={`rbac-resource-id-input-${index}`}
                        value={rule.resource.id}
                        placeholder="e.g., echo-tool"
                        onChange={(e) =>
                          handleUpdateRule(index, {
                            resource: { ...rule.resource, id: e.target.value },
                          })
                        }
                      />
                    </div>
                  </div>
                </div>
              </AccordionContent>
            </AccordionItem>
          ))}
        </Accordion>
      </div>

      <Button
        variant="outline"
        className="w-full"
        onClick={handleAddRule}
        data-cy="rbac-add-rule-button"
      >
        <Plus className="h-4 w-4 mr-2" />
        Add Rule
      </Button>

      <div className="flex justify-end space-x-2 pt-4">
        <Button variant="outline" onClick={onCancel} data-cy="rbac-config-cancel-button">
          Cancel
        </Button>
        <Button onClick={handleSave} data-cy="rbac-config-save-button">
          Save Changes
        </Button>
      </div>
    </div>
  );
}
