import js from '@eslint/js';
import globals from 'globals';
import tseslint from 'typescript-eslint';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import prettier from 'eslint-config-prettier';

export default tseslint.config(
  {
    // Generated or vendored — not ours to lint. `validate-vision.js` is
    // gitignored PLAID tooling rather than repo code; the rest of scripts/ is
    // ours and is linted.
    ignores: [
      'dist',
      'target',
      'src/lib/ipc-types.ts',
      'src-tauri/gen',
      'scripts/validate-vision.js',
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2022,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],

      // The app is offline and AI-free by architecture. These are the two
      // rules that keep a well-meaning change from quietly breaking that.
      'no-restricted-globals': [
        'error',
        {
          name: 'fetch',
          message: 'The app makes no network calls from the UI. Go through Tauri IPC.',
        },
        { name: 'XMLHttpRequest', message: 'The app makes no network calls from the UI.' },
      ],
      'no-restricted-syntax': [
        'error',
        {
          selector: "NewExpression[callee.name='WebSocket']",
          message: 'The app makes no network connections from the UI.',
        },
      ],

      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
    },
  },
  {
    // Node context, not browser.
    files: ['*.config.{ts,js}', 'scripts/**/*.{js,mjs}'],
    languageOptions: { globals: globals.node },
  },
  {
    files: ['**/*.test.{ts,tsx}', 'src/test-setup.ts'],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
  },
  prettier,
);
