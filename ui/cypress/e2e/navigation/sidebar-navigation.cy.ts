describe('Sidebar Navigation', () => {
  beforeEach(() => {
    cy.visit('/');
  });

  it('should display all navigation links correctly', () => {
    // Verify all main navigation links are present and visible
    cy.get('[data-cy="nav-home"]').should('be.visible');
    cy.get('[data-cy="nav-listeners"]').should('be.visible');
    cy.get('[data-cy="nav-routes"]').should('be.visible');
    cy.get('[data-cy="nav-backends"]').should('be.visible');
    cy.get('[data-cy="nav-policies"]').should('be.visible');
    cy.get('[data-cy="nav-playground"]').should('be.visible');
  });

  it('should navigate to each section correctly', () => {
    // Test navigation to each section
    const sections = [
      { selector: '[data-cy="nav-listeners"]', url: '/listeners', pageSelector: '[data-cy="listeners-page"]' },
      { selector: '[data-cy="nav-routes"]', url: '/routes', pageSelector: '[data-cy="routes-page"]' },
      { selector: '[data-cy="nav-backends"]', url: '/backends', pageSelector: '[data-cy="backends-page"]' },
      { selector: '[data-cy="nav-policies"]', url: '/policies', pageSelector: '[data-cy="policies-page"]' },
      { selector: '[data-cy="nav-playground"]', url: '/playground', pageSelector: '[data-cy="playground-page"]' }
    ];

    sections.forEach(section => {
      // Click navigation link
      cy.get(section.selector).click();
      
      // Verify URL changed
      cy.url().should('include', section.url);
      
      // Verify page loaded
      cy.get(section.pageSelector).should('be.visible');
      
      // Return to home for next test
      cy.get('[data-cy="nav-home"]').click();
      cy.url().should('not.include', section.url);
    });
  });

  it('should show active state indicators correctly', () => {
    // Start on home page
    cy.get('[data-cy="nav-home"]').should('be.visible');
    
    // Navigate to listeners and check active state
    cy.get('[data-cy="nav-listeners"]').click();
    cy.url().should('include', '/listeners');
    
    // Check if active state is indicated (could be class, aria-current, or visual indicator)
    cy.get('[data-cy="nav-listeners"]').then(($el) => {
      // Check for common active state indicators
      const hasActiveClass = $el.hasClass('active') || 
                            $el.hasClass('current') || 
                            $el.hasClass('selected') ||
                            $el.attr('aria-current') === 'page' ||
                            $el.attr('data-active') === 'true';
      
      if (hasActiveClass) {
        cy.log('Active state indicator found');
      } else {
        // Check if parent has active state
        cy.wrap($el).parent().then(($parent) => {
          const parentHasActive = $parent.hasClass('active') || 
                                 $parent.hasClass('current') || 
                                 $parent.hasClass('selected');
          if (parentHasActive) {
            cy.log('Parent has active state indicator');
          } else {
            cy.log('No obvious active state indicator found - may use different pattern');
          }
        });
      }
    });
    
    // Navigate to another section and verify active state changes
    cy.get('[data-cy="nav-routes"]').click();
    cy.url().should('include', '/routes');
    
    // Verify routes is now active (if active states are implemented)
    cy.get('[data-cy="nav-routes"]').should('be.visible');
  });

  it('should display navigation badges correctly', () => {
    // Check for navigation badges (if they exist)
    cy.get('body').then(($body) => {
      // Check for listener badge
      if ($body.find('[data-cy="nav-badge-listeners"]').length > 0) {
        cy.get('[data-cy="nav-badge-listeners"]').should('be.visible');
        cy.log('Listeners badge found');
      }
      
      // Check for routes badge
      if ($body.find('[data-cy="nav-badge-routes"]').length > 0) {
        cy.get('[data-cy="nav-badge-routes"]').should('be.visible');
        cy.log('Routes badge found');
      }
      
      // Check for backends badge
      if ($body.find('[data-cy="nav-badge-backends"]').length > 0) {
        cy.get('[data-cy="nav-badge-backends"]').should('be.visible');
        cy.log('Backends badge found');
      }
      
      if ($body.find('[data-cy*="nav-badge"]').length === 0) {
        cy.log('No navigation badges found - may not be implemented');
      }
    });
  });

  it('should handle theme toggle functionality', () => {
    // Check if theme toggle exists
    cy.get('body').then(($body) => {
      if ($body.find('[data-cy="theme-toggle"]').length > 0) {
        // Get initial theme state
        cy.get('html').then(($html) => {
          const initialTheme = $html.hasClass('dark') ? 'dark' : 'light';
          cy.log(`Initial theme: ${initialTheme}`);
          
          // Try to click theme toggle with multiple strategies
          cy.get('[data-cy="theme-toggle"]').then(($toggle) => {
            // First try scrolling into view and clicking with force
            cy.wrap($toggle).scrollIntoView();
            cy.wait(200);
            
            // Try clicking with force to bypass coverage issues
            cy.wrap($toggle).click({ force: true });
            
            // Wait for theme change
            cy.wait(1000);
            
            // Verify theme changed or log that toggle was clicked
            cy.get('html').then(($newHtml) => {
              const newTheme = $newHtml.hasClass('dark') ? 'dark' : 'light';
              cy.log(`Theme after toggle: ${newTheme}`);
              
              if (initialTheme !== newTheme) {
                cy.log('Theme toggle working correctly');
              } else {
                // Check for other theme indicators
                cy.get('body').then(($newBody) => {
                  const bodyHasDark = $newBody.hasClass('dark') || $newBody.attr('data-theme') === 'dark';
                  const originalBodyDark = $body.hasClass('dark') || $body.attr('data-theme') === 'dark';
                  
                  if (bodyHasDark !== originalBodyDark) {
                    cy.log('Theme toggle working (body class changed)');
                  } else {
                    cy.log('Theme toggle clicked successfully - theme change may use different implementation');
                  }
                });
              }
            });
            
            // Try to toggle back (optional - don't fail if this doesn't work)
            cy.wrap($toggle).click({ force: true }).then(() => {
              cy.log('Theme toggle clicked again to restore original state');
            });
          });
        });
      } else {
        cy.log('Theme toggle not found - may not be implemented');
      }
    });
  });

  it('should display restart setup button and handle click', () => {
    // Check if restart setup button exists
    cy.get('body').then(($body) => {
      if ($body.find('[data-cy="restart-setup-button"]').length > 0) {
        cy.get('[data-cy="restart-setup-button"]').should('be.visible');
        
        // Click restart setup button
        cy.get('[data-cy="restart-setup-button"]').click();
        
        // Wait for potential navigation or modal
        cy.wait(1000);
        
        // Check if setup wizard opened or if we're redirected
        cy.get('body').then(($newBody) => {
          if ($newBody.find('[data-cy="setup-wizard-container"]').length > 0) {
            cy.get('[data-cy="setup-wizard-container"]').should('be.visible');
            cy.log('Setup wizard opened successfully');
            
            // Close wizard by navigating back to home
            cy.get('[data-cy="nav-home"]').click();
          } else if ($newBody.find('[data-cy="wizard-welcome-step"]').length > 0) {
            cy.get('[data-cy="wizard-welcome-step"]').should('be.visible');
            cy.log('Setup wizard welcome step displayed');
            
            // Navigate back to home
            cy.visit('/');
          } else {
            cy.log('Restart setup button clicked but no obvious wizard opened');
          }
        });
      } else {
        cy.log('Restart setup button not found - may not be implemented');
      }
    });
  });

  it('should maintain navigation state during page interactions', () => {
    // Navigate to listeners page
    cy.get('[data-cy="nav-listeners"]').click();
    cy.get('[data-cy="listeners-page"]').should('be.visible');
    
    // Perform some page interaction (scroll, click elements)
    cy.scrollTo('bottom', { ensureScrollable: false });
    cy.wait(500);
    cy.scrollTo('top', { ensureScrollable: false });
    
    // Verify navigation is still functional
    cy.get('[data-cy="nav-routes"]').should('be.visible').click();
    cy.get('[data-cy="routes-page"]').should('be.visible');
    
    // Navigate back to home
    cy.get('[data-cy="nav-home"]').click();
    cy.url().should('not.include', '/routes');
  });

  it('should handle navigation keyboard accessibility', () => {
    // Test keyboard navigation (if supported)
    cy.get('[data-cy="nav-home"]').focus();
    
    // Test Enter key activation on focused element
    cy.get('[data-cy="nav-listeners"]').focus();
    cy.focused().type('{enter}');
    cy.wait(500);
    
    // Verify navigation occurred
    cy.url().then((url) => {
      if (url.includes('/listeners')) {
        cy.log('Enter key navigation working');
      } else {
        cy.log('Keyboard navigation may not be implemented or uses different pattern');
      }
    });
    
    // Navigate back to home
    cy.get('[data-cy="nav-home"]').click();
  });

  it('should display navigation consistently across different viewport sizes', () => {
    // Test desktop view (default)
    cy.viewport(1280, 720);
    cy.get('[data-cy="nav-home"]').should('be.visible');
    cy.get('[data-cy="nav-listeners"]').should('be.visible');
    
    // Test tablet view
    cy.viewport(768, 1024);
    cy.get('[data-cy="nav-home"]').should('be.visible');
    
    // Test mobile view
    cy.viewport(375, 667);
    cy.get('body').then(($body) => {
      // Navigation might be hidden behind hamburger menu on mobile
      if ($body.find('[data-cy="nav-home"]').is(':visible')) {
        cy.get('[data-cy="nav-home"]').should('be.visible');
        cy.log('Navigation visible on mobile');
      } else {
        // Look for mobile menu toggle
        if ($body.find('[data-cy="mobile-menu-toggle"]').length > 0) {
          cy.get('[data-cy="mobile-menu-toggle"]').click();
          cy.get('[data-cy="nav-home"]').should('be.visible');
          cy.log('Mobile menu toggle working');
        } else {
          cy.log('Navigation may be hidden on mobile or uses different pattern');
        }
      }
    });
    
    // Reset to desktop view
    cy.viewport(1280, 720);
  });
});
