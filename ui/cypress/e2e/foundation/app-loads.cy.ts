/// <reference types="cypress" />

describe('Application Loading', () => {
  beforeEach(() => {
    cy.visit('/', { failOnStatusCode: false })
  })

  it('should load the application successfully', () => {
    cy.get('body').should('be.visible')
    cy.get('[data-cy="dashboard-content"]').should('be.visible')
    
    // Check that the main navigation is present
    cy.checkNavigation()
    
    // Verify dashboard statistics cards are present
    cy.get('[data-cy="dashboard-listeners-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-routes-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-backends-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-binds-card"]').should('be.visible')
    
    // Check that getting started buttons are present when no config exists
    cy.get('[data-cy="create-first-listener-button"]').should('be.visible')
  })

  it('should display navigation correctly', () => {
    cy.get('[data-cy="nav-home"]').should('be.visible')
    cy.get('[data-cy="nav-listeners"]').should('be.visible')
    
    // Check navigation badges
    cy.get('[data-cy="nav-badge-listeners"]').should('be.visible')
    cy.get('[data-cy="nav-badge-routes"]').should('be.visible')
    cy.get('[data-cy="nav-badge-backends"]').should('be.visible')
    
    // Check footer navigation
    cy.get('[data-cy="restart-setup-button"]').should('be.visible')
    cy.get('[data-cy="theme-toggle"]').should('be.visible')
  })

  it('should navigate between pages', () => {
    cy.get('[data-cy="nav-listeners"]').click()
    cy.url().should('include', '/listeners')
    
    cy.get('[data-cy="nav-routes"]').click()
    cy.url().should('include', '/routes')
    
    cy.get('[data-cy="nav-backends"]').click()
    cy.url().should('include', '/backends')
    
    cy.get('[data-cy="nav-policies"]').click()
    cy.url().should('include', '/policies')
    
    cy.get('[data-cy="nav-playground"]').click()
    cy.url().should('include', '/playground')
    
    cy.get('[data-cy="nav-home"]').click()
    cy.url().should('eq', Cypress.config().baseUrl + '/')
  })

  it('should display dashboard statistics', () => {
    cy.get('[data-cy="dashboard-content"]').should('be.visible')
    
    // Verify all dashboard cards are present
    cy.get('[data-cy="dashboard-listeners-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-routes-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-backends-card"]').should('be.visible')
    cy.get('[data-cy="dashboard-binds-card"]').should('be.visible')
    
    // Check individual count displays
    cy.get('[data-cy="dashboard-binds-count"]').should('be.visible')
    cy.get('[data-cy="dashboard-listeners-count"]').should('be.visible')
    cy.get('[data-cy="dashboard-routes-count"]').should('be.visible')
    cy.get('[data-cy="dashboard-backends-count"]').should('be.visible')
  })
})
