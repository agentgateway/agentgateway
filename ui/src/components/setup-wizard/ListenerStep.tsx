import { useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { AgentgatewayLogo } from "@/components/agentgateway-logo";
import { ArrowLeft, ArrowRight, Network } from "lucide-react";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { LocalConfig, Listener, ListenerProtocol } from "@/lib/types";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";

interface ListenerStepProps {
  onNext: () => void;
  onPrevious: () => void;
  config: LocalConfig;
  onConfigChange: (config: LocalConfig) => void;
}

export function ListenerStep({ onNext, onPrevious, config, onConfigChange }: ListenerStepProps) {
  const [listenerName, setListenerName] = useState("default");
  const [listenerHostname, setListenerHostname] = useState("localhost");
  const [listenerPort, setListenerPort] = useState("8080");
  const [selectedProtocol, setSelectedProtocol] = useState<ListenerProtocol>(ListenerProtocol.HTTP);
  const [isUpdating, setIsUpdating] = useState(false);

  const handleNext = async () => {
    if (!listenerName.trim()) {
      toast.error("Listener name is required.");
      return;
    }
    if (!listenerHostname.trim()) {
      toast.error("Listener hostname is required.");
      return;
    }
    if (!listenerPort.trim()) {
      toast.error("Listener port is required.");
      return;
    }

    const portNumber = parseInt(listenerPort, 10);
    if (isNaN(portNumber) || portNumber <= 0 || portNumber > 65535) {
      toast.error("Invalid port number. Must be between 1 and 65535.");
      return;
    }

    setIsUpdating(true);

    try {
      const newListener: Listener = {
        name: listenerName,
        hostname: listenerHostname,
        protocol: selectedProtocol,
        routes: [],
        tcpRoutes: [],
      };

      // Create or update the bind with the listener
      const existingBind = config.binds?.find((bind) => bind.port === portNumber);

      let newBinds;
      if (existingBind) {
        // Update existing bind
        newBinds = config.binds.map((bind) =>
          bind.port === portNumber ? { ...bind, listeners: [...bind.listeners, newListener] } : bind
        );
      } else {
        // Create new bind
        newBinds = [
          ...(config.binds || []),
          {
            port: portNumber,
            listeners: [newListener],
          },
        ];
      }

      const newConfig: LocalConfig = {
        ...config,
        binds: newBinds,
      };

      onConfigChange(newConfig);
      toast.success("Listener configured successfully!");
      onNext();
    } catch (err) {
      console.error("Error configuring listener:", err);
      toast.error(err instanceof Error ? err.message : "Failed to configure listener");
    } finally {
      setIsUpdating(false);
    }
  };

  return (
    <Card className="w-full max-w-3xl">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center flex items-center justify-center gap-2">
          <Network className="h-5 w-5 text-primary" />
          Configure Listener
        </CardTitle>
        <CardDescription className="text-center">
          Set up your first listener to accept incoming connections
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className="space-y-6">
          <div className="space-y-3">
            <h3 className="font-medium">What is a Listener?</h3>
            <p className="text-sm text-muted-foreground">
              A listener is a network endpoint that accepts incoming connections on a specific port.
              You can configure the protocol (HTTP, HTTPS, TLS, TCP, or HBONE), hostname, and port.
              The listener will handle incoming requests and route them to appropriate backends.
            </p>
          </div>

          <div className="space-y-4">
            <div className="space-y-3">
              <Label htmlFor="listenerName">Listener Name</Label>
              <Input
                id="listenerName"
                value={listenerName}
                onChange={(e) => setListenerName(e.target.value)}
                placeholder="e.g., default"
              />
              <p className="text-xs text-muted-foreground">A unique name for this listener.</p>
            </div>

            <div className="space-y-3">
              <Label>Protocol</Label>
              <RadioGroup
                value={selectedProtocol}
                onValueChange={(value) => setSelectedProtocol(value as ListenerProtocol)}
                className="grid grid-cols-2 gap-4"
              >
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HTTP} id="http-protocol" />
                  <Label htmlFor="http-protocol">HTTP</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HTTPS} id="https-protocol" />
                  <Label htmlFor="https-protocol">HTTPS</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.TLS} id="tls-protocol" />
                  <Label htmlFor="tls-protocol">TLS</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.TCP} id="tcp-protocol" />
                  <Label htmlFor="tcp-protocol">TCP</Label>
                </div>
                <div className="flex items-center space-x-2">
                  <RadioGroupItem value={ListenerProtocol.HBONE} id="hbone-protocol" />
                  <Label htmlFor="hbone-protocol">HBONE</Label>
                </div>
              </RadioGroup>
              <p className="text-xs text-muted-foreground">
                Choose the protocol the listener will handle.
              </p>
            </div>

            <div className="space-y-3">
              <Label htmlFor="listenerHostname">Hostname</Label>
              <Input
                id="listenerHostname"
                value={listenerHostname}
                onChange={(e) => setListenerHostname(e.target.value)}
                placeholder="e.g., localhost or *"
              />
              <p className="text-xs text-muted-foreground">
                The hostname the listener will bind to. Use * for all hostnames.
              </p>
            </div>

            <div className="space-y-3">
              <Label htmlFor="listenerPort">Port</Label>
              <Input
                id="listenerPort"
                value={listenerPort}
                onChange={(e) => setListenerPort(e.target.value)}
                placeholder="e.g., 8080"
                type="number"
                min="1"
                max="65535"
              />
              <p className="text-xs text-muted-foreground">
                The port number the listener will use (1-65535).
              </p>
            </div>
          </div>

          <div className="p-4 bg-muted/30 rounded-lg">
            <h4 className="font-medium text-sm mb-2">Preview</h4>
            <p className="text-sm text-muted-foreground">
              Your listener will be available at:{" "}
              <code className="bg-muted px-1 py-0.5 rounded text-xs">
                {selectedProtocol.toLowerCase()}://{listenerHostname}:{listenerPort}
              </code>
            </p>
          </div>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onPrevious}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          Back
        </Button>
        <Button onClick={handleNext} disabled={isUpdating}>
          {isUpdating ? "Configuring..." : "Next"}
          <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
