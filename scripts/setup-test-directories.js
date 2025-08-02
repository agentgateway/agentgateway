const fs = require('fs');
const path = require('path');

/**
 * Required directories for Cypress test execution
 * These directories must exist to prevent video recording and screenshot errors
 */
const REQUIRED_DIRECTORIES = [
  'cypress/videos',
  'cypress/screenshots',
  'cypress/downloads',
  'cypress/reports',
  'cypress/results'
];

/**
 * Ensure a directory exists, creating it if necessary
 * @param {string} dirPath - Path to the directory (relative to current working directory)
 */
function ensureDirectoryExists(dirPath) {
  try {
    const fullPath = path.resolve(dirPath);
    
    if (!fs.existsSync(fullPath)) {
      fs.mkdirSync(fullPath, { recursive: true });
      console.log(`📁 Created directory: ${dirPath}`);
    }
  } catch (error) {
    console.error(`❌ Failed to create directory ${dirPath}:`, error.message);
    throw error;
  }
}

/**
 * Setup all required test directories
 */
function setupTestDirectories() {
  console.log('📁 Setting up test directories...');
  
  try {
    for (const dir of REQUIRED_DIRECTORIES) {
      ensureDirectoryExists(dir);
    }
    console.log('✅ All test directories ready');
  } catch (error) {
    console.error('❌ Failed to setup test directories:', error.message);
    throw error;
  }
}

module.exports = {
  ensureDirectoryExists,
  setupTestDirectories,
  REQUIRED_DIRECTORIES
};
