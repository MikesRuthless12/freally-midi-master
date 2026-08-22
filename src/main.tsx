import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { TranslatedErrorBoundary } from './components/ErrorBoundary/ErrorBoundary';
import { reportCrash } from './lib/crash';
import './styles/tokens.css';
import { initTheme } from './state/theme';
import { initI18n } from './i18n';

// Before first paint, so the window never flashes the wrong theme or the wrong
// language. Both read localStorage because they must be synchronous.
initTheme();
initI18n();

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {/* ⛔ **Outside `App`, so it catches `App` itself** (TASK-093). A boundary
        rendered inside the tree it is protecting cannot survive a throw in that
        tree's own root, which in a hosted DAW is a dead rectangle the producer
        can only fix by removing and re-inserting the plugin. */}
    <TranslatedErrorBoundary onCaught={reportCrash}>
      <App />
    </TranslatedErrorBoundary>
  </React.StrictMode>,
);

// The per-script fonts (CJK, Arabic, Indic, …) — ~520 @font-face rules, and
// ~450 KB of CSS. Deliberately after the first render: blocking the window on
// them would make an English UI wait to parse Chinese font declarations it will
// never use. `unicode-range` means nothing is actually downloaded until a
// character needs it, so this costs a parse and no bytes.
void import('./assets/fonts/fonts-scripts.css');
