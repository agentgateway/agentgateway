describe('Navigation Elements Test', () => {
  it('should find navigation elements even with 404 page', () => {
    // Visit with failOnStatusCode: false to bypass 404 error
    cy.visit('/', { failOnStatusCode: false });
    
    // Test that navigation elements are present and have correct data-cy attributes
    cy.get('[data-cy="nav-home"]').should('be.visible');
    cy.get('[data-cy="nav-listeners"]').should('be.visible');
    cy.get('[data-cy="nav-routes"]').should('be.visible');
    cy.get('[data-cy="nav-backends"]').should('be.visible');
    cy.get('[data-cy="nav-policies"]').should('be.visible');
    cy.get('[data-cy="nav-playground"]').should('be.visible');
    
    // Test navigation badges
    cy.get('[data-cy="nav-badge-listeners"]').should('contain', '0');
    cy.get('[data-cy="nav-badge-routes"]').should('contain', '0');
    cy.get('[data-cy="nav-badge-backends"]').should('contain', '0');
    
    // Test footer navigation
    cy.get('[data-cy="restart-setup-button"]').should('be.visible');
    cy.get('[data-cy="theme-toggle"]').should('be.visible');
    
    // Test that navigation elements are clickable
    cy.get('[data-cy="nav-home"]').should('not.be.disabled');
    cy.get('[data-cy="nav-listeners"]').should('not.be.disabled');
    cy.get('[data-cy="theme-toggle"]').should('not.be.disabled');
  });
  
  it('should have proper navigation structure', () => {
    cy.visit('/', { failOnStatusCode: false });
    
    // Test that all navigation elements exist in the sidebar
    const navItems = [
      'nav-home',
      'nav-listeners', 
      'nav-routes',
      'nav-backends',
      'nav-policies',
      'nav-playground'
    ];
    
    navItems.forEach(item => {
      cy.get(`[data-cy="${item}"]`)
        .should('exist')
        .should('be.visible')
        .should('have.attr', 'data-cy', item);
    });
  });
});
