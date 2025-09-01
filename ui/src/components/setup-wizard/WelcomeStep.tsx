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
import { ArrowRight, Network, Route, Globe, Shield } from "lucide-react";

interface WelcomeStepProps {
  onNext: () => void;
  onSkip: () => void;
}

export function WelcomeStep({ onNext, onSkip }: WelcomeStepProps) {
  return (
    <Card className="w-full max-w-3xl">
      <CardHeader>
        <div className="flex justify-center mb-6">
          <AgentgatewayLogo className="h-12" />
        </div>
        <CardTitle className="text-center">Welcome to agentgateway</CardTitle>
        <CardDescription className="text-center">
          Let&apos;s configure your gateway in just a few steps
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-6">
        <div className="space-y-3">
          <h3 className="font-medium">What is agentgateway?</h3>
          <p className="text-sm text-muted-foreground">
            <a
              href="https://agentgateway.dev"
              className="text-accent-foreground hover:text-accent/90 hover:underline"
            >
              Agentgateway
            </a>{" "}
            is an open source tool that helps you connect, secure, and observe agent-to-agent and
            agent-to-tool communication across any agent framework and environment.
          </p>
        </div>

        <div className="space-y-3">
          <h3 className="font-medium">What you&apos;ll configure:</h3>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="flex items-start space-x-3 p-3 rounded-lg bg-muted/50">
              <Network className="h-5 w-5 text-primary mt-0.5" />
              <div>
                <h4 className="font-medium text-sm">Listeners</h4>
                <p className="text-xs text-muted-foreground">
                  Network endpoints that accept incoming connections
                </p>
              </div>
            </div>
            <div className="flex items-start space-x-3 p-3 rounded-lg bg-muted/50">
              <Route className="h-5 w-5 text-green-500 mt-0.5" />
              <div>
                <h4 className="font-medium text-sm">Routes</h4>
                <p className="text-xs text-muted-foreground">
                  Rules for matching and routing requests
                </p>
              </div>
            </div>
            <div className="flex items-start space-x-3 p-3 rounded-lg bg-muted/50">
              <Globe className="h-5 w-5 text-orange-500 mt-0.5" />
              <div>
                <h4 className="font-medium text-sm">Backends</h4>
                <p className="text-xs text-muted-foreground">
                  Target servers for your requests (MCP, OpenAPI, etc.)
                </p>
              </div>
            </div>
            <div className="flex items-start space-x-3 p-3 rounded-lg bg-muted/50">
              <Shield className="h-5 w-5 text-red-500 mt-0.5" />
              <div>
                <h4 className="font-medium text-sm">Policies</h4>
                <p className="text-xs text-muted-foreground">
                  Security, traffic management, and routing policies
                </p>
              </div>
            </div>
          </div>
        </div>

        <div className="text-center p-4 bg-muted/30 rounded-lg">
          <p className="text-sm text-muted-foreground">
            This wizard will guide you through setting up your first complete gateway configuration.
            You can always modify these settings later.
          </p>
        </div>
      </CardContent>
      <CardFooter className="flex justify-between">
        <Button variant="outline" onClick={onSkip}>
          Skip Wizard
        </Button>
        <Button onClick={onNext} className="min-w-24">
          Start Setup
          <ArrowRight className="ml-2 h-4 w-4" />
        </Button>
      </CardFooter>
    </Card>
  );
}
