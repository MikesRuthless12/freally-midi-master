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
