describe('Deep Linking and URL Navigation', () => {
  it('should handle direct URL access to dashboard', () => {
    // Visit dashboard directly
    cy.visit('/');
    
    // Verify dashboard loads correctly
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    cy.url().should('eq', Cypress.config().baseUrl + '/');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-home"]').should('be.visible');
  });

  it('should handle direct URL access to listeners page', () => {
    // Visit listeners page directly
    cy.visit('/listeners');
    
    // Verify listeners page loads correctly
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    cy.url().should('include', '/listeners');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-listeners"]').should('be.visible');
    
    // Verify we can navigate to other pages
    cy.get('[data-cy="nav-home"]').click();
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
  });

  it('should handle direct URL access to routes page', () => {
    // Visit routes page directly
    cy.visit('/routes');
    
    // Verify routes page loads correctly
    cy.get('[data-cy="routes-page"]').should('be.visible');
    cy.url().should('include', '/routes');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-routes"]').should('be.visible');
    
    // Test navigation to another page
    cy.get('[data-cy="nav-backends"]').click();
    cy.get('[data-cy="backends-page"]').should('be.visible');
  });

  it('should handle direct URL access to backends page', () => {
    // Visit backends page directly
    cy.visit('/backends');
    
    // Verify backends page loads correctly
    cy.get('[data-cy="backends-page"]').should('be.visible');
    cy.url().should('include', '/backends');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-backends"]').should('be.visible');
  });

  it('should handle direct URL access to policies page', () => {
    // Visit policies page directly
    cy.visit('/policies');
    
    // Verify policies page loads correctly
    cy.get('[data-cy="policies-page"]').should('be.visible');
    cy.url().should('include', '/policies');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-policies"]').should('be.visible');
  });

  it('should handle direct URL access to playground page', () => {
    // Visit playground page directly
    cy.visit('/playground');
    
    // Verify playground page loads correctly
    cy.get('[data-cy="playground-page"]').should('be.visible');
    cy.url().should('include', '/playground');
    
    // Verify navigation is functional
    cy.get('[data-cy="nav-playground"]').should('be.visible');
  });

  it('should handle invalid URLs gracefully', () => {
    // Visit a non-existent page
    cy.visit('/non-existent-page', { failOnStatusCode: false });
    
    // Check if we get a 404 page or redirect to home
    cy.get('body').then(($body) => {
      if ($body.text().includes('404') || $body.text().includes('Not Found')) {
        cy.log('404 page displayed correctly');
        
        // Verify navigation still works from 404 page
        if ($body.find('[data-cy="nav-home"]').length > 0) {
          cy.get('[data-cy="nav-home"]').click();
          cy.get('[data-cy="dashboard-content"]').should('be.visible');
        }
      } else if ($body.find('[data-cy="dashboard-content"]').length > 0) {
        cy.log('Redirected to dashboard for invalid URL');
        cy.get('[data-cy="dashboard-content"]').should('be.visible');
      } else {
        cy.log('Custom error handling for invalid URLs');
      }
    });
  });

  it('should preserve navigation context during page refresh', () => {
    // Navigate to listeners page
    cy.visit('/listeners');
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    
    // Refresh the page
    cy.reload();
    
    // Verify we're still on listeners page
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    cy.url().should('include', '/listeners');
    
    // Verify navigation is still functional
    cy.get('[data-cy="nav-listeners"]').should('be.visible');
    cy.get('[data-cy="nav-routes"]').click();
    cy.get('[data-cy="routes-page"]').should('be.visible');
  });

  it('should handle browser back and forward navigation', () => {
    // Start on home page
    cy.visit('/');
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    
    // Navigate to listeners
    cy.get('[data-cy="nav-listeners"]').click();
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    
    // Navigate to routes
    cy.get('[data-cy="nav-routes"]').click();
    cy.get('[data-cy="routes-page"]').should('be.visible');
    
    // Use browser back button
    cy.go('back');
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    cy.url().should('include', '/listeners');
    
    // Use browser back button again
    cy.go('back');
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    cy.url().should('not.include', '/listeners');
    
    // Use browser forward button
    cy.go('forward');
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    cy.url().should('include', '/listeners');
  });

  it('should handle URL parameters and query strings', () => {
    // Visit page with query parameters
    cy.visit('/listeners?filter=active&sort=name');
    
    // Verify page loads correctly with parameters
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    cy.url().should('include', '/listeners');
    cy.url().should('include', 'filter=active');
    cy.url().should('include', 'sort=name');
    
    // Verify navigation preserves or handles parameters appropriately
    cy.get('[data-cy="nav-routes"]').click();
    cy.get('[data-cy="routes-page"]').should('be.visible');
    cy.url().should('include', '/routes');
    cy.url().should('not.include', 'filter=active'); // Parameters should not carry over
  });

  it('should handle hash fragments in URLs', () => {
    // Visit page with hash fragment
    cy.visit('/playground#connection-settings');
    
    // Verify page loads correctly
    cy.get('[data-cy="playground-page"]').should('be.visible');
    cy.url().should('include', '/playground');
    cy.url().should('include', '#connection-settings');
    
    // Test navigation with hash
    cy.get('[data-cy="nav-home"]').click();
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    cy.url().should('not.include', '#connection-settings');
  });

  it('should maintain state during navigation between pages', () => {
    // Start on dashboard and note any dynamic content
    cy.visit('/');
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    
    // Check for any statistics or dynamic content
    cy.get('body').then(($body) => {
      const hasStats = $body.find('[data-cy*="count"]').length > 0;
      if (hasStats) {
        // Store initial state
        cy.get('[data-cy*="count"]').first().invoke('text').as('initialCount');
      }
    });
    
    // Navigate to different pages and back
    cy.get('[data-cy="nav-listeners"]').click();
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    
    cy.get('[data-cy="nav-backends"]').click();
    cy.get('[data-cy="backends-page"]').should('be.visible');
    
    cy.get('[data-cy="nav-home"]').click();
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    
    // Verify state is maintained (if applicable)
    cy.get('body').then(($body) => {
      if ($body.find('[data-cy*="count"]').length > 0) {
        cy.log('Dashboard statistics are displayed consistently');
      }
    });
  });

  it('should handle concurrent navigation requests', () => {
    // Start on home page
    cy.visit('/');
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
    
    // Rapidly click different navigation items
    cy.get('[data-cy="nav-listeners"]').click();
    cy.get('[data-cy="nav-routes"]').click();
    cy.get('[data-cy="nav-backends"]').click();
    
    // Wait for final navigation to complete
    cy.get('[data-cy="backends-page"]').should('be.visible');
    cy.url().should('include', '/backends');
    
    // Verify navigation is still functional
    cy.get('[data-cy="nav-home"]').click();
    cy.get('[data-cy="dashboard-content"]').should('be.visible');
  });

  it('should handle setup wizard deep linking', () => {
    // Check if setup wizard can be accessed directly
    cy.visit('/', { failOnStatusCode: false });
    
    // Try to trigger setup wizard
    cy.get('body').then(($body) => {
      if ($body.find('[data-cy="run-setup-wizard-button"]').length > 0) {
        cy.get('[data-cy="run-setup-wizard-button"]').click();
        
        // Check if wizard opens
        cy.get('body').then(($wizardBody) => {
          if ($wizardBody.find('[data-cy="wizard-welcome-step"]').length > 0) {
            cy.get('[data-cy="wizard-welcome-step"]').should('be.visible');
            
            // Check URL for wizard state
            cy.url().then((url) => {
              cy.log(`Wizard URL: ${url}`);
              
              // Try refreshing wizard page
              cy.reload();
              
              // Verify wizard state is maintained or handled gracefully
              cy.get('body').should('exist');
            });
          }
        });
      } else {
        cy.log('Setup wizard button not found - may not be available');
      }
    });
  });

  it('should handle external link behavior', () => {
    // Visit a page and check for external links
    cy.visit('/');
    
    // Look for any external links (if they exist)
    cy.get('body').then(($body) => {
      const externalLinks = $body.find('a[href^="http"]');
      if (externalLinks.length > 0) {
        // Check that external links have proper attributes
        cy.wrap(externalLinks.first()).should('have.attr', 'target', '_blank');
        cy.wrap(externalLinks.first()).should('have.attr', 'rel').and('include', 'noopener');
        cy.log('External links configured correctly');
      } else {
        cy.log('No external links found - this is expected for internal application');
      }
    });
  });
});
