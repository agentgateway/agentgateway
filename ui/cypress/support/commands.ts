/// <reference types="cypress" />

// ***********************************************
// This example commands.ts shows you how to
// create various custom commands and overwrite
// existing commands.
//
// For more comprehensive examples of custom
// commands please read more here:
// https://on.cypress.io/custom-commands
// ***********************************************

declare global {
  namespace Cypress {
    interface Chainable {
      /**
       * Custom command to navigate to a specific page and wait for it to load
       * @example cy.navigateToPage('/listeners')
       */
      navigateToPage(path: string): Chainable<void>
      
      /**
       * Custom command to wait for dashboard to load completely
       * @example cy.waitForDashboard()
       */
      waitForDashboard(): Chainable<void>
      
      /**
       * Custom command to check if navigation is working
       * @example cy.checkNavigation()
       */
      checkNavigation(): Chainable<void>
      
      /**
       * Custom command to setup test data
       * @example cy.setupTestData()
       */
      setupTestData(): Chainable<void>
      
      /**
       * Custom command to clean up test data
       * @example cy.cleanupTestData()
       */
      cleanupTestData(): Chainable<void>
    }
  }
}

// Navigate to a specific page and wait for it to load
Cypress.Commands.add('navigateToPage', (path: string) => {
  cy.visit(path)
  cy.url().should('include', path)
  cy.get('body').should('be.visible')
})

// Wait for dashboard to load completely
Cypress.Commands.add('waitForDashboard', () => {
  cy.get('[data-cy="dashboard-content"]', { timeout: 10000 }).should('be.visible')
  cy.get('[data-cy="dashboard-loading"]').should('not.exist')
})

// Check if navigation is working properly
Cypress.Commands.add('checkNavigation', () => {
  // Check all main navigation links are present
  cy.get('[data-cy="nav-home"]').should('be.visible')
  cy.get('[data-cy="nav-listeners"]').should('be.visible')
  cy.get('[data-cy="nav-routes"]').should('be.visible')
  cy.get('[data-cy="nav-backends"]').should('be.visible')
  cy.get('[data-cy="nav-policies"]').should('be.visible')
  cy.get('[data-cy="nav-playground"]').should('be.visible')
})

// Setup test data (placeholder for future implementation)
Cypress.Commands.add('setupTestData', () => {
  // This will be implemented when we have actual test data requirements
  cy.log('Setting up test data...')
})

// Cleanup test data (placeholder for future implementation)
Cypress.Commands.add('cleanupTestData', () => {
  // This will be implemented when we have actual cleanup requirements
  cy.log('Cleaning up test data...')
})

export {}
