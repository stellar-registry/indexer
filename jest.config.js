const baseTransform = { '^.+\\.tsx?$': 'ts-jest' };

module.exports = {
  projects: [
    {
      displayName: 'unit',
      testEnvironment: 'node',
      roots: ['<rootDir>/test/unit'],
      testMatch: ['**/*.test.ts'],
      transform: baseTransform,
    },
    {
      displayName: 'integration',
      testEnvironment: 'node',
      roots: ['<rootDir>/test/integration'],
      testMatch: ['**/*.test.ts'],
      transform: baseTransform,
      globalSetup: '<rootDir>/test/integration/globalSetup.ts',
    },
  ],
};
