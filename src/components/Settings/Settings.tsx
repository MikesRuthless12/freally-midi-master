import { useEffect, useMemo, useState } from 'react';
import { Info, Monitor, Moon, Palette, Settings2, Sun, X } from 'lucide-react';

import { invoke, isTauri } from '../../lib/ipc';
import { useUi } from '../../state/ui';
import type { ThemePreference } from '../../state/theme';
import './Settings.css';

/**
 * Settings — sidebar categories on the left, the selected pane on the right,
 * matching the shape used across the Freally apps.
 *
 * Every control here is real and persists. Nothing decorative: a settings
 * screen that shows a toggle which does not survive a restart is worse than
 * one that omits it.
 */

const CATEGORIES = ['general', 'appearance', 'about'] as const;
type CategoryId = (typeof CATEGORIES)[number];

const CATEGORY_LABELS: Record<CategoryId, string> = {
  general: 'General',
  appearance: 'Appearance',
  about: 'About',
};

const CATEGORY_ICONS: Record<CategoryId, typeof Settings2> = {
  general: Settings2,
  appearance: Palette,
  about: Info,
};

/** Mirrors `Settings` in `src-tauri/src/store/settings.rs`. */
type AppSettings = {
  minimizeToTray: boolean;
  closeToTray: boolean;
  showTrayIcon: boolean;
  theme: ThemePreference;
};

const DEFAULTS: AppSettings = {
  minimizeToTray: false,
  closeToTray: false,
  showTrayIcon: true,
  theme: 'system',
};

/** Search terms per category, so the filter matches content and not just titles. */
const CATEGORY_TERMS: Record<CategoryId, string> = {
  general: 'general tray system minimize minimise close taskbar notification area',
  appearance: 'appearance theme dark light system colour color',
  about: 'about version licence license disclaimer credits artist names privacy',
};

function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="settings__row" data-disabled={disabled || undefined}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.currentTarget.checked)}
      />
      <span className="settings__rowtext">
        <span className="settings__rowlabel">{label}</span>
        {hint && <span className="settings__rowhint">{hint}</span>}
      </span>
    </label>
  );
}

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [active, setActive] = useState<CategoryId>('general');
  const [search, setSearch] = useState('');
  const [settings, setSettings] = useState<AppSettings>(DEFAULTS);
  const [error, setError] = useState<string | null>(null);

  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);

  useEffect(() => {
    invoke<AppSettings>('settings_get')
      .then((s) => setSettings({ ...DEFAULTS, ...s }))
      .catch(() => {
        // No backend (a plain browser). The panel still renders so the layout
        // can be seen and tested; the toggles simply have nothing to persist to.
      });
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

  /** Persist immediately — no Apply button to forget to press. */
  const update = async (patch: Partial<AppSettings>) => {
    const next = { ...settings, ...patch };
    setSettings(next);
    setError(null);
    try {
      await invoke<AppSettings>('settings_set', { settings: next });
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const visible = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return [...CATEGORIES];
    return CATEGORIES.filter(
      (c) => CATEGORY_LABELS[c].toLowerCase().includes(q) || CATEGORY_TERMS[c].includes(q),
    );
  }, [search]);

  // Derived, not synced: if the search filters the selected category away, the
  // first visible one is shown. Storing that in state would need an effect to
  // keep it in step, which is a second source of truth for the same fact.
  const shown = visible.includes(active) ? active : (visible[0] ?? active);

  const trayDisabled = !settings.showTrayIcon;

  return (
    <div className="settings" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <div className="settings__panel">
        <div className="settings__head">
          <h2 id="settings-title">Settings</h2>
          <button type="button" className="btn-ghost" aria-label="Close" onClick={onClose}>
            <X size={14} aria-hidden="true" />
          </button>
        </div>

        <div className="settings__body">
          <nav
            className="settings__nav"
            role="tablist"
            aria-orientation="vertical"
            aria-label="Settings categories"
          >
            <input
              type="search"
              className="settings__search"
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              placeholder="Search settings…"
              aria-label="Search settings"
            />
            {visible.length === 0 ? (
              <p className="settings__none">Nothing matches “{search}”.</p>
            ) : (
              visible.map((id) => {
                const Icon = CATEGORY_ICONS[id];
                return (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    id={`settings-tab-${id}`}
                    aria-selected={shown === id}
                    aria-controls="settings-pane"
                    tabIndex={shown === id ? 0 : -1}
                    className="settings__tab"
                    onClick={() => setActive(id)}
                  >
                    <Icon size={14} aria-hidden="true" />
                    {CATEGORY_LABELS[id]}
                  </button>
                );
              })
            )}
          </nav>

          <div
            className="settings__pane"
            id="settings-pane"
            role="tabpanel"
            aria-labelledby={`settings-tab-${shown}`}
          >
            {shown === 'general' && (
              <section className="settings__section">
                <h3>System tray</h3>

                <Toggle
                  label="Show a system tray icon"
                  hint="Adds Freally MIDI Master to the notification area, with Show and Quit."
                  checked={settings.showTrayIcon}
                  onChange={(v) => update({ showTrayIcon: v })}
                />

                <Toggle
                  label="Minimize to system tray"
                  hint="Minimizing hides the window to the tray instead of the taskbar. Click the tray icon to bring it back."
                  checked={settings.minimizeToTray}
                  disabled={trayDisabled}
                  onChange={(v) => update({ minimizeToTray: v })}
                />

                <Toggle
                  label="Close to system tray"
                  hint="Closing the window keeps the app running in the tray. Quit from the tray menu to exit."
                  checked={settings.closeToTray}
                  disabled={trayDisabled}
                  onChange={(v) => update({ closeToTray: v })}
                />

                {trayDisabled && (
                  <p className="settings__note">
                    Turn the tray icon on to use the two options above — without it there would
                    be no way to get the window back.
                  </p>
                )}

                <p className="settings__note">
                  A tray icon requires a restart to appear or disappear.
                </p>
              </section>
            )}

            {shown === 'appearance' && (
              <section className="settings__section">
                <h3>Theme</h3>
                <p className="settings__note">
                  Dark is the default and the app&rsquo;s signature look. Both themes are
                  contrast-checked to WCAG&nbsp;2.1&nbsp;AA.
                </p>
                <div className="settings__choices" role="radiogroup" aria-label="Theme">
                  {(
                    [
                      { value: 'system', label: 'Match system', Icon: Monitor },
                      { value: 'dark', label: 'Dark', Icon: Moon },
                      { value: 'light', label: 'Light', Icon: Sun },
                    ] as const
                  ).map(({ value, label, Icon }) => (
                    <button
                      key={value}
                      type="button"
                      role="radio"
                      aria-checked={theme === value}
                      className="settings__choice"
                      onClick={() => {
                        setTheme(value);
                        void update({ theme: value });
                      }}
                    >
                      <Icon size={16} aria-hidden="true" />
                      {label}
                    </button>
                  ))}
                </div>
              </section>
            )}

            {shown === 'about' && <AboutPane />}
          </div>
        </div>

        {error && (
          <p className="settings__error" role="alert">
            Could not save your settings: {error}
          </p>
        )}
      </div>
    </div>
  );
}

type AppInfo = { version: string; platform: string; arch: string };

/** Shared by the Settings → About pane and the standalone About overlay. */
export function AboutPane() {
  const [info, setInfo] = useState<AppInfo | null>(null);

  useEffect(() => {
    if (!isTauri()) return;
    invoke<AppInfo>('app_info')
      .then(setInfo)
      .catch(() => setInfo(null));
  }, []);

  return (
    <section className="settings__section">
      <h3>Freally MIDI Master</h3>
      <p className="settings__note">
        Artist-accurate MIDI, generated by a rule-based engine. No AI, no accounts, no
        telemetry.
      </p>

      <dl className="settings__facts">
        <dt>Version</dt>
        <dd>{info?.version ?? '—'}</dd>
        <dt>Platform</dt>
        <dd>{info ? `${info.platform} / ${info.arch}` : '—'}</dd>
        <dt>Licence</dt>
        <dd>Proprietary, source-available. All Rights Reserved.</dd>
      </dl>

      <h3>Artist names</h3>
      <p className="settings__note">
        Artist and producer names are descriptive references to a musical style, nothing more.
        No affiliation, endorsement, or authorship is implied or claimed. Every pattern is
        generated procedurally from hand-authored style parameters — no MIDI, audio, or data is
        copied from any recording.
      </p>

      <h3>Credits</h3>
      <p className="settings__note">
        Timing and velocity statistics derived from the Magenta Groove MIDI Dataset
        (CC&nbsp;BY&nbsp;4.0). Inter, Space Grotesk and JetBrains Mono under the SIL Open Font
        License. Lucide icons under ISC. Preview kits are synthesized in-repo and contain no
        third-party samples.
      </p>
    </section>
  );
}
