import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './styles/tokens.css';
import { initTheme } from './state/theme';
import { reconcileThemeWithSettings } from './state/ui';

// Before first paint, so the window never flashes the wrong theme. This reads
// localStorage because it must be synchronous.
initTheme();

// Then settle up with settings.json, the durable store, once the bridge is up.
// localStorage can be cleared out from under us; the file cannot.
void reconcileThemeWithSettings();

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

// The per-script fonts (CJK, Arabic, Indic, …) — ~520 @font-face rules, and
// ~450 KB of CSS. Deliberately after the first render: blocking the window on
// them would make an English UI wait to parse Chinese font declarations it will
// never use. `unicode-range` means nothing is actually downloaded until a
// character needs it, so this costs a parse and no bytes.
void import('./assets/fonts/fonts-scripts.css');
