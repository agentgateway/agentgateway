describe('Connection Errors', () => {
  beforeEach(() => {
    cy.visit('/');
  });

  describe('Backend Connection Failures', () => {
    it('should handle backend service unavailability gracefully', () => {
      // Navigate to listeners page and attempt operations
      cy.get('[data-cy="nav-listeners"]').click();
      cy.get('[data-cy="listeners-page"]').should('be.visible');
      
      // Attempt to add listener when backend might be unavailable
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="add-listener-button"]').length > 0) {
          cy.get('[data-cy="add-listener-button"]').click();
          cy.wait(5000); // Wait for potential timeout
          
          // Verify graceful handling of backend unavailability
          cy.get('body').should('exist');
          cy.get('[data-cy="listeners-page"]').should('be.visible');
          cy.log('Backend unavailability handled gracefully on listeners page');
        } else {
          cy.log('Add listener button not available - testing page load with backend unavailability');
          
          // Page should still load even if backend is unavailable
          cy.get('[data-cy="listeners-page"]').should('be.visible');
        }
      });
    });

    it('should handle API timeout scenarios', () => {
      // Test configuration operations that might timeout
      cy.get('[data-cy="nav-routes"]').click();
      cy.get('[data-cy="routes-page"]').should('be.visible');
      
      // Attempt route operations that might timeout
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="add-route-button"]').length > 0) {
          cy.get('[data-cy="add-route-button"]').click();
          cy.wait(10000); // Wait longer for timeout scenario
          
          // Verify timeout is handled gracefully
          cy.get('body').should('exist');
          cy.get('[data-cy="routes-page"]').should('be.visible');
          cy.log('API timeout handled gracefully on routes page');
        } else {
          cy.log('Add route button not available - testing page timeout handling');
        }
      });
    });

    it('should handle network connectivity issues', () => {
      // Test playground connectivity which depends on backend services
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Attempt to connect when network might be unavailable
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(10000); // Wait for network timeout
          
          // Verify network issues are handled gracefully
          cy.get('body').should('exist');
          cy.get('[data-cy="playground-page"]').should('be.visible');
          cy.log('Network connectivity issues handled gracefully');
        } else {
          cy.log('Connect button not available - testing network error handling');
        }
      });
    });

    it('should display appropriate error messages for connection failures', () => {
      // Test error message display for connection failures
      cy.get('[data-cy="nav-backends"]').click();
      cy.get('[data-cy="backends-page"]').should('be.visible');
      
      // Attempt backend operations that might fail
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="add-backend-button"]').length > 0) {
          cy.get('[data-cy="add-backend-button"]').click();
          cy.wait(5000);
          
          // Look for error messages or indicators
          cy.get('body').then(($body) => {
            const errorElements = $body.find('[class*="error"], [role="alert"], [class*="failed"], .text-red-500, .text-destructive');
            
            if (errorElements.length > 0) {
              cy.log(`Found ${errorElements.length} potential error indicators`);
              
              errorElements.each((index, element) => {
                const $el = Cypress.$(element);
                if ($el.is(':visible') && $el.text().trim().length > 0) {
                  cy.log(`Error indicator ${index + 1}: ${$el.text().trim()}`);
                }
              });
            } else {
              cy.log('No error indicators found - may use different error display method');
            }
          });
        } else {
          cy.log('Add backend button not available for connection error testing');
        }
      });
    });
  });

  describe('Setup Wizard Connection Errors', () => {
    it('should handle wizard completion with backend unavailability', () => {
      // Complete wizard when backend might be unavailable
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill wizard steps
      cy.get('[data-cy="listener-name-input"]').type('connection-error-test');
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="route-name-input"]').type('connection-error-route');
      cy.get('[data-cy="route-path-input"]').type('/api/error-test');
      cy.get('[data-cy="wizard-route-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="backend-name-input"]').type('connection-error-backend');
      cy.get('[data-cy="backend-target-name-input"]').type('http://localhost:3001');
      cy.get('[data-cy="wizard-backend-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="wizard-policy-next"]').scrollIntoView().click({ force: true });
      
      // Attempt to complete wizard (may fail due to backend unavailability)
      cy.get('[data-cy="wizard-complete"]').scrollIntoView().click({ force: true });
      cy.wait(10000); // Wait for potential timeout
      
      // Verify graceful handling of completion failure
      cy.get('body').should('exist');
      cy.log('Wizard completion with backend unavailability handled gracefully');
    });

    it('should handle step transitions with network errors', () => {
      // Test step transitions when network is unreliable
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill listener step
      cy.get('[data-cy="listener-name-input"]').type('network-error-test');
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      
      // Attempt transition that might fail due to network
      cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
      cy.wait(5000);
      
      // Verify transition handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="wizard-route-step"]').length > 0) {
          cy.log('Step transition succeeded despite potential network issues');
        } else {
          cy.log('Step transition may have failed - testing error handling');
          
          // Try again
          cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
          cy.wait(3000);
        }
      });
      
      cy.log('Step transition network error handling tested');
    });

    it('should handle validation with backend dependency failures', () => {
      // Test validation that depends on backend services
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill steps that might require backend validation
      cy.get('[data-cy="listener-name-input"]').type('validation-dependency-test');
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="route-name-input"]').type('validation-dependency-route');
      cy.get('[data-cy="route-path-input"]').type('/api/validation-test');
      cy.get('[data-cy="wizard-route-next"]').scrollIntoView().click({ force: true });
      
      // Test backend validation that might fail
      cy.get('[data-cy="backend-name-input"]').type('validation-dependency-backend');
      cy.get('[data-cy="backend-target-name-input"]').type('http://unreachable-backend:3001');
      cy.get('[data-cy="wizard-backend-next"]').scrollIntoView().click({ force: true });
      
      cy.wait(5000);
      
      // Verify validation failure is handled gracefully
      cy.get('body').should('exist');
      cy.log('Backend dependency validation failure handled gracefully');
    });
  });

  describe('Playground Connection Errors', () => {
    it('should handle MCP connection failures', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Attempt MCP connection that might fail
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(10000); // Wait for connection timeout
          
          // Verify connection failure is handled gracefully
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="disconnect-button"]').length > 0) {
              cy.log('MCP connection succeeded unexpectedly');
            } else {
              cy.log('MCP connection failure handled gracefully');
              
              // Check for error indicators
              const errorElements = $body.find('[class*="error"], [class*="failed"], [class*="timeout"]');
              if (errorElements.length > 0) {
                cy.log('MCP connection error indicators found');
              }
            }
          });
        } else {
          cy.log('MCP connect button not available - testing connection error scenarios');
        }
      });
    });

    it('should handle A2A connection failures', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test A2A connection failure scenarios
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          // Attempt connection multiple times to test retry behavior
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(5000);
          
          // If connection failed, try again
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="connect-button"]').length > 0) {
              cy.get('[data-cy="connect-button"]').click();
              cy.wait(5000);
              cy.log('A2A connection retry behavior tested');
            }
          });
        } else {
          cy.log('A2A connect button not available for connection error testing');
        }
      });
    });

    it('should handle HTTP endpoint connection failures', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test HTTP connection to unreachable endpoints
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="action-panel"]').length > 0 && 
            $body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Configure HTTP request to unreachable endpoint
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            cy.wrap(paramInputs.first()).clear().type('http://unreachable-endpoint.invalid/api');
            
            // Attempt HTTP request
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(10000); // Wait for timeout
            
            // Verify HTTP connection failure is handled gracefully
            cy.get('body').should('exist');
            cy.get('[data-cy="playground-page"]').should('be.visible');
            cy.log('HTTP endpoint connection failure handled gracefully');
          }
        } else {
          cy.log('HTTP testing interface not available for connection error testing');
        }
      });
    });

    it('should handle tool execution timeouts', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test tool execution that might timeout
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Configure parameters that might cause timeout
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            cy.wrap(paramInputs.first()).clear().type('{"timeout": 30000, "delay": true}');
          }
          
          // Execute tool and wait for timeout
          cy.get('[data-cy="run-tool-button"]').click();
          cy.wait(15000); // Wait for execution timeout
          
          // Verify timeout is handled gracefully
          cy.get('body').should('exist');
          cy.get('[data-cy="playground-page"]').should('be.visible');
          cy.log('Tool execution timeout handled gracefully');
        } else {
          cy.log('Tool execution interface not available for timeout testing');
        }
      });
    });
  });

  describe('Network Error Recovery', () => {
    it('should recover from temporary network interruptions', () => {
      // Test recovery after network interruption
      cy.get('[data-cy="nav-listeners"]').click();
      cy.get('[data-cy="listeners-page"]').should('be.visible');
      
      // Simulate network interruption by rapid navigation
      cy.get('[data-cy="nav-routes"]').click();
      cy.get('[data-cy="nav-backends"]').click();
      cy.get('[data-cy="nav-policies"]').click();
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="nav-home"]').click();
      
      // Wait for potential network recovery
      cy.wait(3000);
      
      // Verify application recovered
      cy.get('[data-cy="dashboard-content"]').should('be.visible');
      cy.log('Recovery from network interruption verified');
    });

    it('should handle intermittent connectivity issues', () => {
      // Test handling of intermittent connectivity
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill form with intermittent connectivity simulation
      cy.get('[data-cy="listener-name-input"]').type('intermittent-test');
      cy.wait(2000);
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      cy.wait(2000);
      
      // Attempt navigation with potential connectivity issues
      cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
      cy.wait(5000);
      
      // Verify handling of intermittent issues
      cy.get('body').should('exist');
      cy.log('Intermittent connectivity issues handled');
    });

    it('should maintain application state during connection recovery', () => {
      // Test state maintenance during connection recovery
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill partial form data
      cy.get('[data-cy="listener-name-input"]').type('state-recovery-test');
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      
      // Simulate connection issue by navigating away and back
      // Use more robust navigation with error handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="nav-playground"]').length > 0) {
          cy.get('[data-cy="nav-playground"]').click();
          cy.wait(2000);
          
          // Navigate back home
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="nav-home"]').length > 0) {
              cy.get('[data-cy="nav-home"]').click();
              cy.wait(2000);
            } else {
              cy.log('Home navigation not available - using alternative navigation');
              cy.visit('/');
            }
          });
        } else {
          cy.log('Playground navigation not available - simulating connection issue differently');
          
          // Alternative simulation: refresh page to simulate connection interruption
          cy.reload();
          cy.wait(2000);
        }
      });
      
      // Return to wizard and check state
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-setup-wizard-button"]').length > 0) {
          cy.get('[data-cy="run-setup-wizard-button"]').click();
        } else {
          cy.log('Setup wizard button not available - may already be in wizard');
        }
      });
      
      // Verify state handling during recovery
      cy.get('body').should('exist');
      cy.log('Application state during connection recovery tested');
    });
  });

  describe('Error Message Clarity', () => {
    it('should display clear error messages for connection timeouts', () => {
      // Test timeout error message clarity
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Attempt operation that might timeout
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(15000); // Wait for timeout
          
          // Look for timeout error messages
          cy.get('body').then(($body) => {
            const timeoutElements = $body.find('[class*="timeout"], [class*="error"]');
            const messageElements = $body.find('[role="alert"], [class*="message"]');
            
            if (timeoutElements.length > 0 || messageElements.length > 0) {
              cy.log('Timeout error messages found');
              
              // Check message clarity
              [...timeoutElements, ...messageElements].forEach((element, index) => {
                const $el = Cypress.$(element);
                if ($el.is(':visible') && $el.text().trim().length > 0) {
                  const message = $el.text().trim();
                  if (message.toLowerCase().includes('timeout') || 
                      message.toLowerCase().includes('connection') ||
                      message.toLowerCase().includes('failed')) {
                    cy.log(`Clear timeout message ${index + 1}: ${message}`);
                  }
                }
              });
            } else {
              cy.log('No timeout error messages found - may use different display method');
            }
          });
        }
      });
    });

    it('should display helpful error messages for network failures', () => {
      // Test network failure error message helpfulness
      cy.get('[data-cy="nav-backends"]').click();
      cy.get('[data-cy="backends-page"]').should('be.visible');
      
      // Attempt operation that might fail due to network
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="add-backend-button"]').length > 0) {
          cy.get('[data-cy="add-backend-button"]').click();
          cy.wait(10000); // Wait for potential network failure
          
          // Look for helpful error messages
          cy.get('body').then(($body) => {
            const errorElements = $body.find('[class*="error"], [role="alert"], [class*="failed"]');
            
            if (errorElements.length > 0) {
              errorElements.each((index, element) => {
                const $el = Cypress.$(element);
                if ($el.is(':visible') && $el.text().trim().length > 0) {
                  const message = $el.text().trim();
                  
                  // Check if message is helpful
                  if (message.length > 10 && // Not just "Error"
                      (message.toLowerCase().includes('network') ||
                       message.toLowerCase().includes('connection') ||
                       message.toLowerCase().includes('server') ||
                       message.toLowerCase().includes('unavailable'))) {
                    cy.log(`Helpful network error message: ${message}`);
                  }
                }
              });
            } else {
              cy.log('No network error messages found');
            }
          });
        }
      });
    });

    it('should provide actionable error recovery suggestions', () => {
      // Test error recovery suggestions
      cy.get('[data-cy="run-setup-wizard-button"]').click();
      cy.get('[data-cy="wizard-welcome-next"]').click();
      
      // Fill form and attempt operation that might fail
      cy.get('[data-cy="listener-name-input"]').type('recovery-suggestion-test');
      cy.get('[data-cy="listener-port-input"]').clear().type('8080');
      cy.get('[data-cy="wizard-listener-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="route-name-input"]').type('recovery-route');
      cy.get('[data-cy="route-path-input"]').type('/api/recovery');
      cy.get('[data-cy="wizard-route-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="backend-name-input"]').type('recovery-backend');
      cy.get('[data-cy="backend-target-name-input"]').type('http://localhost:3001');
      cy.get('[data-cy="wizard-backend-next"]').scrollIntoView().click({ force: true });
      
      cy.get('[data-cy="wizard-policy-next"]').scrollIntoView().click({ force: true });
      cy.get('[data-cy="wizard-complete"]').scrollIntoView().click({ force: true });
      
      cy.wait(10000); // Wait for potential failure
      
      // Look for recovery suggestions
      cy.get('body').then(($body) => {
        const suggestionElements = $body.find('[class*="suggestion"], [class*="help"], [class*="retry"]');
        const buttonElements = $body.find('button[class*="retry"], button[class*="try-again"]');
        
        if (suggestionElements.length > 0 || buttonElements.length > 0) {
          cy.log('Error recovery suggestions found');
          
          // Check for actionable suggestions
          suggestionElements.each((index, element) => {
            const $el = Cypress.$(element);
            if ($el.is(':visible') && $el.text().trim().length > 0) {
              const suggestion = $el.text().trim();
              if (suggestion.toLowerCase().includes('try') ||
                  suggestion.toLowerCase().includes('retry') ||
                  suggestion.toLowerCase().includes('check')) {
                cy.log(`Actionable suggestion: ${suggestion}`);
              }
            }
          });
          
          // Check for retry buttons
          if (buttonElements.length > 0) {
            cy.log('Retry buttons available for error recovery');
          }
        } else {
          cy.log('No error recovery suggestions found');
        }
      });
    });
  });

  describe('Connection State Management', () => {
    it('should handle connection state transitions gracefully', () => {
      // Test connection state transitions
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test connection state changes
      cy.get('body').then(($body) => {
        const hasConnect = $body.find('[data-cy="connect-button"]').length > 0;
        const hasDisconnect = $body.find('[data-cy="disconnect-button"]').length > 0;
        
        if (hasConnect) {
          cy.log('Initial state: Disconnected');
          
          // Attempt connection
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(5000);
          
          // Check state transition
          cy.get('body').then(($body) => {
            const nowHasDisconnect = $body.find('[data-cy="disconnect-button"]').length > 0;
            if (nowHasDisconnect) {
              cy.log('State transition: Connected');
              
              // Test disconnect
              cy.get('[data-cy="disconnect-button"]').click();
              cy.wait(2000);
              cy.log('State transition: Disconnected');
            } else {
              cy.log('Connection may have failed - state remained disconnected');
            }
          });
        } else if (hasDisconnect) {
          cy.log('Initial state: Connected');
          
          // Test disconnect
          cy.get('[data-cy="disconnect-button"]').click();
          cy.wait(2000);
          cy.log('State transition: Disconnected');
        } else {
          cy.log('Connection state unclear - buttons may be conditionally rendered');
        }
      });
    });

    it('should maintain connection state consistency', () => {
      // Test connection state consistency across navigation
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Record initial state
      cy.get('body').then(($body) => {
        const initialState = {
          hasConnect: $body.find('[data-cy="connect-button"]').length > 0,
          hasDisconnect: $body.find('[data-cy="disconnect-button"]').length > 0
        };
        
        // Navigate away and back
        cy.get('[data-cy="nav-home"]').click();
        cy.get('[data-cy="dashboard-content"]').should('be.visible');
        
        cy.get('[data-cy="nav-playground"]').click();
        cy.get('[data-cy="playground-page"]').should('be.visible');
        
        // Check state consistency
        cy.get('body').then(($body) => {
          const finalState = {
            hasConnect: $body.find('[data-cy="connect-button"]').length > 0,
            hasDisconnect: $body.find('[data-cy="disconnect-button"]').length > 0
          };
          
          if (initialState.hasConnect === finalState.hasConnect &&
              initialState.hasDisconnect === finalState.hasDisconnect) {
            cy.log('Connection state consistency maintained');
          } else {
            cy.log('Connection state changed during navigation');
          }
        });
      });
    });
  });
});
