"use client";

import { useWizard } from "@/lib/wizard-context";
import { useServer } from "@/lib/server-context";
import { SetupWizard } from "@/components/setup-wizard";

export default function SimplePage() {
  const { showWizard, showSetupWizard, handleWizardComplete, handleWizardSkip } = useWizard();
  const { setConfig } = useServer();

  const handleRunSetupWizard = () => {
    showSetupWizard();
  };

  const handleConfigChange = (newConfig: any) => {
    setConfig(newConfig);
  };

  // Convert to LocalConfig format for wizard
  const localConfig = {
    binds: [],
  };

  // If wizard is visible, render the wizard
  if (showWizard) {
    return (
      <SetupWizard
        config={localConfig}
        onConfigChange={handleConfigChange}
        onComplete={handleWizardComplete}
        onSkip={handleWizardSkip}
      />
    );
  }

  // Otherwise render the dashboard
  return (
    <div data-cy="dashboard-content">
      <h1>AgentGateway Dashboard</h1>
      <p>Welcome to AgentGateway - your intelligent API gateway solution.</p>

      {/* Dashboard Statistics Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4 my-6">
        <div data-cy="dashboard-listeners-card" className="p-4 border rounded-lg">
          <h3>Listeners</h3>
          <span data-cy="dashboard-listeners-count">0</span>
        </div>
        <div data-cy="dashboard-routes-card" className="p-4 border rounded-lg">
          <h3>Routes</h3>
          <span data-cy="dashboard-routes-count">0</span>
        </div>
        <div data-cy="dashboard-backends-card" className="p-4 border rounded-lg">
          <h3>Backends</h3>
          <span data-cy="dashboard-backends-count">0</span>
        </div>
        <div data-cy="dashboard-binds-card" className="p-4 border rounded-lg">
          <h3>Binds</h3>
          <span data-cy="dashboard-binds-count">0</span>
        </div>
      </div>

      <button data-cy="create-first-listener-button">Create First Listener</button>
      <button data-cy="run-setup-wizard-button" onClick={handleRunSetupWizard}>
        Run Setup Wizard
      </button>
    </div>
  );
}
