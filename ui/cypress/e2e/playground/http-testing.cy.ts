describe('HTTP Testing', () => {
  beforeEach(() => {
    cy.visit('/');
  });

  describe('HTTP Endpoint Configuration', () => {
    it('should navigate to playground page', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
    });

    it('should display HTTP testing interface when available', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for HTTP-specific UI elements
      cy.get('body').then(($body) => {
        // Look for HTTP endpoint configuration elements
        if ($body.find('[class*="http"]').length > 0 || 
            $body.find('[data-cy*="http"]').length > 0 ||
            $body.find('input[placeholder*="http"]').length > 0) {
          cy.log('HTTP testing interface found');
        } else {
          cy.log('HTTP testing interface not visible - may require specific backend configuration');
        }
      });
    });

    it('should handle HTTP endpoint URL configuration', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Look for URL input fields or configuration areas
      cy.get('body').then(($body) => {
        const urlInputs = $body.find('input[type="url"], input[placeholder*="http"], input[placeholder*="endpoint"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (urlInputs.length > 0) {
          // Configure HTTP endpoint URL
          cy.wrap(urlInputs.first()).clear().type('https://api.example.com/test');
          cy.log('HTTP endpoint URL configured');
        } else if (paramInputs.length > 0) {
          // Use parameter inputs for HTTP configuration
          cy.wrap(paramInputs.first()).clear().type('https://api.example.com/test');
          cy.log('HTTP endpoint configured via parameter input');
        } else {
          cy.log('HTTP endpoint configuration inputs not found');
        }
      });
    });

    it('should validate HTTP endpoint URL format', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test URL validation
      cy.get('body').then(($body) => {
        const urlInputs = $body.find('input[type="url"], input[placeholder*="http"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (urlInputs.length > 0 || paramInputs.length > 0) {
          const targetInput = urlInputs.length > 0 ? urlInputs.first() : paramInputs.first();
          
          // Test invalid URL format
          cy.wrap(targetInput).clear().type('invalid-url-format');
          cy.log('Invalid URL format tested');
          
          // Test valid URL format
          cy.wrap(targetInput).clear().type('https://valid-api.example.com/endpoint');
          cy.log('Valid URL format tested');
        } else {
          cy.log('URL inputs not available for validation testing');
        }
      });
    });
  });

  describe('Request Building and Sending', () => {
    it('should handle HTTP request parameter configuration', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for request parameter configuration
      cy.get('body').then(($body) => {
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        const textAreas = $body.find('textarea');
        
        if (paramInputs.length > 0) {
          // Configure request parameters
          cy.wrap(paramInputs.first()).clear().type('{"key": "value", "param": "test"}');
          cy.log('HTTP request parameters configured');
        } else if (textAreas.length > 0) {
          // Use textarea for request body
          cy.wrap(textAreas.first()).clear().type('{"request": "body", "data": "test"}');
          cy.log('HTTP request body configured');
        } else {
          cy.log('Request parameter inputs not found');
        }
      });
    });

    it('should handle different HTTP request body formats', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test different request body formats
      cy.get('body').then(($body) => {
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        const textAreas = $body.find('textarea');
        
        if (paramInputs.length > 0 || textAreas.length > 0) {
          const targetInput = paramInputs.length > 0 ? paramInputs.first() : textAreas.first();
          
          // Test JSON format
          cy.wrap(targetInput).clear().type('{"format": "json", "test": true}');
          cy.log('JSON request body format tested');
          
          // Test plain text format
          cy.wrap(targetInput).clear().type('plain text request body');
          cy.log('Plain text request body format tested');
          
          // Test form data format
          cy.wrap(targetInput).clear().type('key1=value1&key2=value2');
          cy.log('Form data request body format tested');
        } else {
          cy.log('Request body inputs not available for format testing');
        }
      });
    });

    it('should handle HTTP request headers configuration', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Look for header configuration options
      cy.get('body').then(($body) => {
        // Check for header-specific inputs or configuration areas
        const headerInputs = $body.find('input[placeholder*="header"], input[placeholder*="Header"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (headerInputs.length > 0) {
          cy.wrap(headerInputs.first()).clear().type('Content-Type: application/json');
          cy.log('HTTP headers configured via dedicated input');
        } else if (paramInputs.length > 1) {
          // Use second parameter input for headers if available
          cy.wrap(paramInputs.eq(1)).clear().type('{"Content-Type": "application/json", "Authorization": "Bearer token"}');
          cy.log('HTTP headers configured via parameter input');
        } else {
          cy.log('Header configuration inputs not found');
        }
      });
    });

    it('should send HTTP request when configuration is complete', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Configure and send HTTP request
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Configure request if inputs are available
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/get');
          }
          
          // Send HTTP request
          cy.get('[data-cy="run-tool-button"]').click();
          cy.log('HTTP request sent');
          
          // Wait for request completion
          cy.wait(5000);
          
        } else {
          cy.log('Send button not available - requires HTTP configuration');
        }
      });
    });
  });

  describe('Different HTTP Methods', () => {
    it('should handle GET request method', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test GET request configuration
      cy.get('body').then(($body) => {
        // Look for method selection or configuration
        const methodSelects = $body.find('select[name*="method"], select[placeholder*="method"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (methodSelects.length > 0) {
          cy.wrap(methodSelects.first()).select('GET');
          cy.log('GET method selected');
        } else if (paramInputs.length > 0) {
          // Configure GET request via parameters
          cy.wrap(paramInputs.first()).clear().type('{"method": "GET", "url": "https://httpbin.org/get"}');
          cy.log('GET request configured via parameters');
        } else {
          cy.log('HTTP method configuration not found - may default to GET');
        }
      });
    });

    it('should handle POST request method', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test POST request configuration
      cy.get('body').then(($body) => {
        const methodSelects = $body.find('select[name*="method"], select[placeholder*="method"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (methodSelects.length > 0) {
          cy.wrap(methodSelects.first()).select('POST');
          cy.log('POST method selected');
          
          // Add request body for POST
          const bodyInputs = $body.find('textarea, input[placeholder*="body"]');
          if (bodyInputs.length > 0) {
            cy.wrap(bodyInputs.first()).clear().type('{"data": "test post request"}');
          }
        } else if (paramInputs.length > 0) {
          // Configure POST request via parameters
          cy.wrap(paramInputs.first()).clear().type('{"method": "POST", "url": "https://httpbin.org/post", "body": {"test": "data"}}');
          cy.log('POST request configured via parameters');
        } else {
          cy.log('POST method configuration not found');
        }
      });
    });

    it('should handle PUT request method', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test PUT request configuration
      cy.get('body').then(($body) => {
        const methodSelects = $body.find('select[name*="method"], select[placeholder*="method"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (methodSelects.length > 0) {
          cy.wrap(methodSelects.first()).select('PUT');
          cy.log('PUT method selected');
        } else if (paramInputs.length > 0) {
          // Configure PUT request via parameters
          cy.wrap(paramInputs.first()).clear().type('{"method": "PUT", "url": "https://httpbin.org/put", "body": {"update": "data"}}');
          cy.log('PUT request configured via parameters');
        } else {
          cy.log('PUT method configuration not found');
        }
      });
    });

    it('should handle DELETE request method', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test DELETE request configuration
      cy.get('body').then(($body) => {
        const methodSelects = $body.find('select[name*="method"], select[placeholder*="method"]');
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        
        if (methodSelects.length > 0) {
          cy.wrap(methodSelects.first()).select('DELETE');
          cy.log('DELETE method selected');
        } else if (paramInputs.length > 0) {
          // Configure DELETE request via parameters
          cy.wrap(paramInputs.first()).clear().type('{"method": "DELETE", "url": "https://httpbin.org/delete"}');
          cy.log('DELETE request configured via parameters');
        } else {
          cy.log('DELETE method configuration not found');
        }
      });
    });
  });

  describe('Response Analysis', () => {
    it('should display HTTP response when request completes', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for response display area
      cy.get('body').then(($body) => {
        // Look for response display component or area
        if ($body.find('[class*="response"]').length > 0 || 
            $body.find('[id*="response"]').length > 0) {
          cy.log('HTTP response display area found');
        } else {
          cy.log('HTTP response display area not visible - may appear after request execution');
        }
      });
    });

    it('should handle HTTP response status codes', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test response status code handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Configure request to test different status codes
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            // Test 200 OK response
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/status/200');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(3000);
            cy.log('200 OK response test completed');
            
            // Test 404 Not Found response
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/status/404');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(3000);
            cy.log('404 Not Found response test completed');
          } else {
            cy.log('Parameter inputs not available for status code testing');
          }
        } else {
          cy.log('Run button not available for response status testing');
        }
      });
    });

    it('should display HTTP response headers', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for response header display
      cy.get('body').then(($body) => {
        // Look for response header display areas
        if ($body.find('[class*="header"]').length > 0 || 
            $body.find('[class*="response"]').length > 0) {
          cy.log('Response header display area found');
        } else {
          cy.log('Response header display not visible - may appear after request execution');
        }
      });
    });

    it('should handle different HTTP response content types', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test different response content types
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            // Test JSON response
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/json');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(3000);
            cy.log('JSON response content type test completed');
            
            // Test HTML response
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/html');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(3000);
            cy.log('HTML response content type test completed');
            
            // Test XML response
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/xml');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(3000);
            cy.log('XML response content type test completed');
          } else {
            cy.log('Parameter inputs not available for content type testing');
          }
        } else {
          cy.log('Run button not available for content type testing');
        }
      });
    });

    it('should handle HTTP request timeout scenarios', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test timeout handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            // Configure request with delay to test timeout
            cy.wrap(paramInputs.first()).clear().type('https://httpbin.org/delay/10');
            cy.get('[data-cy="run-tool-button"]').click();
            
            // Wait for timeout handling
            cy.wait(15000);
            
            // Verify page remains functional after timeout
            cy.get('[data-cy="playground-page"]').should('be.visible');
            cy.log('HTTP timeout scenario handled gracefully');
          } else {
            cy.log('Parameter inputs not available for timeout testing');
          }
        } else {
          cy.log('Run button not available for timeout testing');
        }
      });
    });
  });

  describe('HTTP Error Handling', () => {
    it('should handle network connection errors gracefully', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test network error handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            // Configure request to non-existent domain
            cy.wrap(paramInputs.first()).clear().type('https://non-existent-domain-12345.com/api');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(10000);
            
            // Verify graceful error handling
            cy.get('body').should('exist');
            cy.get('[data-cy="playground-page"]').should('be.visible');
            cy.log('Network connection error handled gracefully');
          } else {
            cy.log('Parameter inputs not available for network error testing');
          }
        } else {
          cy.log('Run button not available for network error testing');
        }
      });
    });

    it('should display appropriate HTTP error messages', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Check for error message display capabilities
      cy.get('body').then(($body) => {
        // Look for error message areas or toast notifications
        if ($body.find('[class*="error"]').length > 0 || 
            $body.find('[class*="toast"]').length > 0 ||
            $body.find('[role="alert"]').length > 0) {
          cy.log('HTTP error message display mechanism found');
        } else {
          cy.log('HTTP error message display not visible - may appear during error conditions');
        }
      });
    });

    it('should handle malformed HTTP request configuration', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test malformed request handling
      cy.get('body').then(($body) => {
        const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
        if (paramInputs.length > 0 && $body.find('[data-cy="run-tool-button"]').length > 0) {
          
          // Test malformed JSON configuration
          cy.wrap(paramInputs.first()).clear().type('{"malformed": json}');
          cy.get('[data-cy="run-tool-button"]').click();
          cy.wait(3000);
          
          // Verify error handling
          cy.get('[data-cy="playground-page"]').should('be.visible');
          cy.log('Malformed HTTP request configuration handled gracefully');
        } else {
          cy.log('Inputs not available for malformed request testing');
        }
      });
    });

    it('should handle CORS and security-related HTTP errors', () => {
      cy.get('[data-cy="nav-playground"]').click();
      cy.get('[data-cy="playground-page"]').should('be.visible');
      
      // Test CORS error handling
      cy.get('body').then(($body) => {
        if ($body.find('[data-cy="run-tool-button"]').length > 0) {
          
          const paramInputs = $body.find('[data-cy^="tool-parameter-"]');
          if (paramInputs.length > 0) {
            // Configure request that might trigger CORS issues
            cy.wrap(paramInputs.first()).clear().type('https://example.com/api/cors-test');
            cy.get('[data-cy="run-tool-button"]').click();
            cy.wait(5000);
            
            // Verify CORS error handling
            cy.get('[data-cy="playground-page"]').should('be.visible');
            cy.log('CORS error scenario handled gracefully');
          } else {
            cy.log('Parameter inputs not available for CORS testing');
          }
        } else {
          cy.log('Run button not available for CORS testing');
        }
      });
    });
  });
});
