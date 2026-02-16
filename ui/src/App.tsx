import { Routes, Route } from 'react-router-dom'
import { ThemeProvider } from '@/components/theme-provider'
import { LoadingWrapper } from '@/components/loading-wrapper'
import { Toaster } from '@/components/ui/sonner'
import { ServerProvider } from '@/lib/server-context'
import { SidebarProvider } from '@/components/ui/sidebar'
import { SidebarWrapper } from '@/components/sidebar-wrapper'
import { WizardProvider } from '@/lib/wizard-context'
import { ConfigErrorWrapper } from '@/components/config-error-wrapper'
import { XdsModeNotification } from '@/components/xds-mode-notification'

// Import pages
import Home from '@/app/page'
import Backends from '@/app/backends/page'
import Cel from '@/app/cel/page'
import Listeners from '@/app/listeners/page'
import Playground from '@/app/playground/page'
import Policies from '@/app/policies/page'
import RoutesPage from '@/app/routes/page'

function App() {
  return (
    <ServerProvider>
      <ThemeProvider
        attribute="class"
        defaultTheme="system"
        enableSystem
        disableTransitionOnChange
        storageKey="agentgateway-theme"
      >
        <XdsModeNotification />
        <LoadingWrapper>
          <WizardProvider>
            <ConfigErrorWrapper>
              <SidebarProvider>
                <div className="flex min-h-screen w-full">
                  <SidebarWrapper />
                  <main className="flex-1 overflow-auto">
                    <Routes>
                      <Route path="/" element={<Home />} />
                      <Route path="/backends" element={<Backends />} />
                      <Route path="/cel" element={<Cel />} />
                      <Route path="/listeners" element={<Listeners />} />
                      <Route path="/playground" element={<Playground />} />
                      <Route path="/policies" element={<Policies />} />
                      <Route path="/routes" element={<RoutesPage />} />
                    </Routes>
                  </main>
                </div>
              </SidebarProvider>
              <Toaster richColors />
            </ConfigErrorWrapper>
          </WizardProvider>
        </LoadingWrapper>
      </ThemeProvider>
    </ServerProvider>
  )
}

export default App
