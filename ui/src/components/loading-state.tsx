
import { AgentgatewayLogo } from "@/components/agentgateway-logo";
import { motion } from "framer-motion";

export function LoadingState() {
  return (
    <div className="flex min-h-screen w-full items-center justify-center">
      <motion.div
        className="text-center"
        initial={{ opacity: 1, y: -20 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.5 }}
      >
        <AgentgatewayLogo className="h-16 w-auto mx-auto mb-4" />
        <p className="text-muted-foreground">Loading...</p>
      </motion.div>
    </div>
  );
}
