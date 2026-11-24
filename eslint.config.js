// Structure rules (ui-architect, references/structure.md): layers and sizes, checked by the machine.
export default [
  {
    files: ['ui/**/*.js'],
    languageOptions: { ecmaVersion: 2024, sourceType: 'module', globals: { window: 'readonly', document: 'readonly', navigator: 'readonly', location: 'readonly', setTimeout: 'readonly', clearTimeout: 'readonly', setInterval: 'readonly', clearInterval: 'readonly', console: 'readonly' } },
    rules: {
      'no-undef': 'error',
      'no-unused-vars': 'error',
      'max-lines': ['error', 300],
      'max-lines-per-function': ['error', 120],
      complexity: ['error', 15],
      'no-restricted-globals': ['error', 'fetch', 'EventSource', 'WebSocket'],
      'no-restricted-properties': ['error', { object: 'window', property: '__TAURI__', message: 'Only ui/api.js talks to Tauri' }],
    },
  },
  { files: ['ui/api.js'], rules: { 'no-restricted-properties': 'off' } },
  {
    files: ['ui/screens/**/*.js'],
    rules: {
      'no-restricted-imports': ['error', { patterns: [{ group: ['**/api.js', '**/api.mock.js'], message: 'Screens read data only through state.js' }] }],
      'no-restricted-syntax': ['error',
        { selector: "CallExpression[callee.property.name='createElement']", message: 'Screens build only from the kit' },
        { selector: "MemberExpression[property.name='style']", message: 'No styles in screens: kit classes and tokens' },
        { selector: "MemberExpression[property.name='innerHTML']", message: 'No raw HTML in screens' }],
    },
  },
  {
    files: ['ui/kit/**/*.js'],
    ignores: ['ui/kit/showcase.js'],
    rules: { 'no-restricted-imports': ['error', { patterns: [{ group: ['../state.js', '../api.js', '../screens/*'], message: 'The kit does not know about data' }] }] },
  },
];
