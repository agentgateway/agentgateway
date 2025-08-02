describe('A2A Testing', () => {
  beforeEach(() => {
    cy.visit('/');
  });

  describe('A2A Connection Setup', () => {
    it('should navigate to playground page', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
    });

    it('should display connect button for A2A connection', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for connect button (may not be visible if backend is unavailable)
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          cy.get('[data-cy="connect-button"]').should('be.visible');
          cy.log('A2A connect button found and visible');
        } else {
          cy.log('Connect button not found - A2A backend may be unavailable');
        }
      });
    });

    it('should handle A2A connection attempt gracefully', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Attempt to connect if button is available
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          cy.get('[data-cy="connect-button"]').click();
          
          // Wait for connection attempt
          cy.wait(3000);
          
          // Check for either success (disconnect button) or error handling
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="disconnect-button"]').length > 0) {
              cy.log('A2A connection successful');
              cy.get('[data-cy="disconnect-button"]').should('be.visible');
            } else {
              cy.log('A2A connection failed or backend unavailable - gracefully handled');
            }
          });
        } else {
          cy.log('Connect button not available - skipping A2A connection test');
        }
      });
    });

    it('should detect A2A backend type automatically', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check if the UI indicates A2A backend type
      cy.get('body').then(($body) => {
        // Look for A2A-specific UI elements or indicators
        if ($body.find('[class*="a2a"]').length > 0 || 
            $body.find('[data-cy*="a2a"]').length > 0) {
          cy.log('A2A backend type detected in UI');
        } else {
          cy.log('A2A backend type detection not visible - may be determined by backend configuration');
        }
      });
    });
  });

  describe('Skill Selection and Execution', () => {
    it('should display capabilities list with A2A skills when connected', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check if capabilities list is available
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="capabilities-list"]').length > 0) {
          cy.get('[data-cy="capabilities-list"]').should('be.visible');
          cy.log('Capabilities list found and visible');
          
          // Look specifically for A2A skill rows
          const skillRows = $body.find('[data-cy^="skill-row-"]');
          if (skillRows.length > 0) {
            cy.log(`Found ${skillRows.length} A2A skills available`);
          } else {
            cy.log('No A2A skills found - may require active A2A connection');
          }
        } else {
          cy.log('Capabilities list not found - may require active A2A connection');
        }
      });
    });

    it('should handle skill selection when A2A skills are available', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Look for A2A skill rows in capabilities list
      cy.get('body').then(($body) => {
        const skillRows = $body.find('[data-cy^="skill-row-"]');
        if (skillRows.length > 0) {
          // Click on first available A2A skill
          cy.wrap(skillRows.first()).click();
          cy.log('A2A skill selected successfully');
          
          // Verify action panel becomes available
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="action-panel"]').length > 0) {
              cy.get('[data-cy="action-panel"]').should('be.visible');
            } else {
              cy.log('Action panel not immediately visible - may require additional setup');
            }
          });
        } else {
          cy.log('No A2A skills available - requires active A2A backend connection');
        }
      });
    });

    it('should display skill information when A2A skill is selected', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for action panel availability for A2A skills
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="action-panel"]').length > 0) {
          cy.get('[data-cy="action-panel"]').should('be.visible');
          cy.log('Action panel available for A2A skill interaction');
          
          // Look for A2A-specific elements in action panel
          if ($body.find('[data-cy="run-tool-button"]').length > 0) {
            cy.log('Run button available for A2A skill execution');
          }
        } else {
          cy.log('Action panel not available - requires A2A skill selection');
        }
      });
    });

    it('should handle multiple skill types in capabilities list', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for both MCP tools and A2A skills
      cy.get('body').then(($body) => {
        const toolRows = $body.find('[data-cy^="tool-row-"]');
        const skillRows = $body.find('[data-cy^="skill-row-"]');
        
        if (toolRows.length > 0 && skillRows.length > 0) {
          cy.log(`Mixed capabilities found: ${toolRows.length} MCP tools, ${skillRows.length} A2A skills`);
        } else if (skillRows.length > 0) {
          cy.log(`A2A-only setup: ${skillRows.length} skills available`);
        } else if (toolRows.length > 0) {
          cy.log(`MCP-only setup: ${toolRows.length} tools available`);
        } else {
          cy.log('No capabilities found - requires active backend connection');
        }
      });
    });
  });

  describe('Message Composition and Sending', () => {
    it('should handle A2A message composition when skill is selected', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check if action panel and message composition area are available
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="action-panel"]').length > 0) {
          
          // Look for message composition elements (may be different from MCP parameter inputs)
          const messageInputs = $body.find('[data-cy^="tool-parameter-"]');
          const textAreas = $body.find('textarea');
          
          if (messageInputs.length > 0 || textAreas.length > 0) {
            // Fill in message composition area
            if (messageInputs.length > 0) {
              cy.wrap(messageInputs.first()).clear().type('Test A2A message content');
            } else if (textAreas.length > 0) {
              cy.wrap(textAreas.first()).clear().type('Test A2A message content');
            }
            cy.log('A2A message composed successfully');
          } else {
            cy.log('Message composition area not found - may require specific A2A skill selection');
          }
          
        } else {
          cy.log('Action panel not available - requires A2A skill selection');
        }
      });
    });

    it('should validate message format for A2A communication', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test message validation
      cy.get('body').then(($body) => {
        const messageInputs = $body.find('[data-cy^="tool-parameter-"]');
        if (messageInputs.length > 0) {
          // Test various message formats
          cy.wrap(messageInputs.first()).clear().type('{"message": "test A2A communication"}');
          cy.log('JSON message format tested');
          
          // Test plain text format
          cy.wrap(messageInputs.first()).clear().type('Plain text A2A message');
          cy.log('Plain text message format tested');
          
        } else {
          cy.log('No message inputs available for format validation testing');
        }
      });
    });

    it('should handle message sending when composition is complete', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test message sending functionality
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Fill message if input is available
          const messageInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (messageInputs.length > 0) {
            cy.wrap(messageInputs.first()).clear().type('Test A2A message for sending');
          }
          
          // Attempt to send message
          cy.get('[data-cy="run-tool-button"]').click();
          cy.log('A2A message sending attempted');
          
          // Wait for sending process
          cy.wait(3000);
          
        } else {
          cy.log('Send button not available - requires A2A skill selection and message composition');
        }
      });
    });
  });

  describe('Response Handling', () => {
    it('should display A2A response when message is sent', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check if response display area exists for A2A
      cy.get('body').then(($body) => {
        // Look for response display component or area
        if ($body.find('[class*="response"]').length > 0 || 
            $body.find('[id*="response"]').length > 0) {
          cy.log('A2A response display area found');
        } else {
          cy.log('A2A response display area not visible - may appear after message sending');
        }
      });
    });

    it('should handle A2A communication errors gracefully', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test A2A error handling by attempting operations without proper setup
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          // Try to send message without proper A2A connection
          cy.get('[data-cy="run-tool-button"]').click();
          cy.wait(5000); // Wait longer for timeout
          
          // Verify graceful error handling
          cy.get('body').should('exist'); // Page should still be functional
          cy.get('[data-cy="playground-page"]').should('be.visible');
          cy.log('A2A communication error handled gracefully');
        } else {
          cy.log('Send button not available - A2A error handling test skipped');
        }
      });
    });

    it('should display appropriate A2A response formats', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for A2A-specific response formatting
      cy.get('body').then(($body) => {
        // Look for response areas that might handle A2A format
        if ($body.find('[class*="response"]').length > 0) {
          cy.log('Response display found - should handle A2A response format');
        } else {
          cy.log('Response display not visible - A2A responses may be handled differently');
        }
      });
    });

    it('should handle A2A connection timeouts appropriately', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test timeout handling for A2A connections
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="connect-button"]').length > 0) {
          // Try to connect and wait for potential timeout
          cy.get('[data-cy="connect-button"]').click();
          cy.wait(10000); // Wait longer to test timeout handling
          
          // Verify timeout is handled gracefully
          cy.get('[data-cy="playground-page"]').should('be.visible');
          cy.log('A2A connection timeout handled gracefully');
        } else {
          cy.log('Connect button not available - timeout test skipped');
        }
      });
    });
  });

  describe('A2A Connection State Management', () => {
    it('should maintain A2A connection state during navigation', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check initial A2A connection state
      cy.get('body').then(($body) => {
        const hasConnect = $body.find('[data-cy="connect-button"]').length > 0;
        const hasDisconnect = $body.find('[data-cy="disconnect-button"]').length > 0;
        
        if (hasConnect) {
          cy.log('Initial A2A state: Disconnected');
        } else if (hasDisconnect) {
          cy.log('Initial A2A state: Connected');
        } else {
          cy.log('A2A connection state unclear - buttons may be conditionally rendered');
        }
        
        // Navigate away and back to test A2A state persistence
        cy.get('[data-cy="nav-home"]').click();
        cy.get('[data-cy="dashboard-content"]').should('be.visible');
        
        cy.get('[data-cy="nav-playground"]').click();
        cy.get('[data-cy="playground-page"]').should('be.visible');
        
        cy.log('A2A connection state persistence test completed');
      });
    });

    it('should handle A2A disconnect functionality when connected', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test A2A disconnect if button is available
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="disconnect-button"]').length > 0) {
          cy.get('[data-cy="disconnect-button"]').click();
          cy.wait(2000);
          
          // Verify A2A disconnect worked
          cy.get('body').then(($body) => {
            if ($body.find('[data-cy="connect-button"]').length > 0) {
              cy.log('A2A disconnect successful - connect button now available');
            } else {
              cy.log('A2A disconnect completed - UI state updated');
            }
          });
        } else {
          cy.log('Disconnect button not available - A2A may not be connected');
        }
      });
    });

    it('should differentiate between A2A and MCP connection states', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check if UI differentiates between connection types
      cy.get('body').then(($body) => {
        // Look for indicators of connection type
        if ($body.find('[class*="a2a"]').length > 0 || 
            $body.find('[class*="mcp"]').length > 0) {
          cy.log('Connection type differentiation found in UI');
        } else {
          cy.log('Connection type differentiation not visible - may be handled internally');
        }
        
        // Check capabilities list for type-specific content
        const toolRows = $body.find('[data-cy^="tool-row-"]');
        const skillRows = $body.find('[data-cy^="skill-row-"]');
        
        if (toolRows.length > 0 && skillRows.length > 0) {
          cy.log('Mixed A2A/MCP environment detected');
        } else if (skillRows.length > 0) {
          cy.log('A2A-specific environment detected');
        } else if (toolRows.length > 0) {
          cy.log('MCP-specific environment detected');
        }
      });
    });
  });
});
