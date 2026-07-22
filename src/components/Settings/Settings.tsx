import { useEffect, useMemo, useState } from 'react';
import {
  Check,
  Info,
  Languages,
  Monitor,
  Moon,
  Palette,
  Settings2,
  Sun,
  X,
} from 'lucide-react';

import { useTranslation } from 'react-i18next';

import { invoke, isTauri } from '../../lib/ipc';
import { LOCALES, type LocaleCode } from '../../i18n/locales';
import { CATEGORIES, type CategoryId } from './categories';
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

const CATEGORY_ICONS: Record<CategoryId, typeof Settings2> = {
  general: Settings2,
  appearance: Palette,
  language: Languages,
  about: Info,
};

/** Mirrors `Settings` in `src-tauri/src/store/settings.rs`. */
type AppSettings = {
  minimizeToTray: boolean;
  closeToTray: boolean;
  showTrayIcon: boolean;
  theme: ThemePreference;
  language: LocaleCode;
};

const DEFAULTS: AppSettings = {
  minimizeToTray: false,
  closeToTray: false,
  showTrayIcon: true,
  theme: 'system',
  language: 'en',
};

/**
 * Search terms per category, so the filter matches content and not just titles.
 *
 * Deliberately English-only, and additive to the translated label the filter
 * also checks. Someone running a Japanese UI who searches "tray" — because that
 * is the word in every tutorial they have read — should still find the setting.
 * The language pane lists every endonym so a lost user can search for their own
 * language in their own script.
 */
const CATEGORY_TERMS: Record<CategoryId, string> = {
  general: 'general tray system minimize minimise close taskbar notification area',
  appearance: 'appearance theme dark light system colour color',
  language: `language locale translation ${LOCALES.map((l) => `${l.english} ${l.native}`).join(' ')}`,
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
  const { t } = useTranslation();
  const [active, setActive] = useState<CategoryId>('general');
  const [search, setSearch] = useState('');
  // `null` until the real values are read. Seeding this with DEFAULTS was a
  // trap: `update` writes the whole object back, so one flipped checkbox
  // persisted a full mirror of the defaults over whatever was really on disk.
  // A transiently unreadable settings.json — locked by antivirus, mid-restore —
  // therefore destroyed every preference the moment the user touched anything.
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const language = useUi((s) => s.language);
  const setLanguage = useUi((s) => s.setLanguage);

  useEffect(() => {
    invoke<AppSettings>('settings_get')
      .then((s) => setSettings({ ...DEFAULTS, ...s }))
      .catch((e) => {
        // Outside Tauri there is no backend at all and nothing is wrong — the
        // panel still renders so the layout can be seen and tested. Inside it,
        // a failure here is real and has to be said out loud, because the
        // controls are about to refuse to save.
        if (isTauri()) setError(e instanceof Error ? e.message : String(e));
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
    // Never write without having read: the payload is the whole object, so
    // writing an unverified one silently replaces the fields not being edited.
    if (!settings) return;
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
      (c) =>
        t(`settings.${c}`).toLowerCase().includes(q) ||
        CATEGORY_TERMS[c].toLowerCase().includes(q),
    );
  }, [search, t]);

  // Derived, not synced: if the search filters the selected category away, the
  // first visible one is shown. Storing that in state would need an effect to
  // keep it in step, which is a second source of truth for the same fact.
  const shown = visible.includes(active) ? active : (visible[0] ?? active);

  // What the controls display before the read lands, or outside Tauri where
  // there is nothing to read. `update` refuses to write in that state, so this
  // is a placeholder for the eye only — it can never reach disk.
  const shownSettings = settings ?? DEFAULTS;
  const canPersist = settings !== null;
  const trayDisabled = !shownSettings.showTrayIcon || !canPersist;

  return (
    <div className="settings" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <div className="settings__panel">
        <div className="settings__head">
          <h2 id="settings-title">{t('settings.title')}</h2>
          <button
            type="button"
            className="btn-ghost"
            data-testid="settings-close"
            aria-label={t('common.close')}
            onClick={onClose}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>

        <div className="settings__body">
          <nav
            className="settings__nav"
            role="tablist"
            aria-orientation="vertical"
            aria-label={t('settings.categories')}
          >
            <input
              type="search"
              className="settings__search"
              value={search}
              onChange={(e) => setSearch(e.currentTarget.value)}
              placeholder={t('settings.searchPlaceholder')}
              aria-label={t('settings.searchLabel')}
            />
            {visible.length === 0 ? (
              <p className="settings__none">{t('settings.noMatch', { query: search })}</p>
            ) : (
              visible.map((id) => {
                const Icon = CATEGORY_ICONS[id];
                return (
                  <button
                    key={id}
                    type="button"
                    role="tab"
                    id={`settings-tab-${id}`}
                    data-testid={`settings-tab-${id}`}
                    aria-selected={shown === id}
                    aria-controls="settings-pane"
                    tabIndex={shown === id ? 0 : -1}
                    className="settings__tab"
                    onClick={() => setActive(id)}
                  >
                    <Icon size={14} aria-hidden="true" />
                    {t(`settings.${id}`)}
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
                <h3>{t('settings.trayHeading')}</h3>

                <Toggle
                  label={t('settings.showTrayIcon')}
                  hint={t('settings.showTrayIconHint')}
                  checked={shownSettings.showTrayIcon}
                  disabled={!canPersist}
                  onChange={(v) => update({ showTrayIcon: v })}
                />

                <Toggle
                  label={t('settings.minimizeToTray')}
                  hint={t('settings.minimizeToTrayHint')}
                  checked={shownSettings.minimizeToTray}
                  disabled={trayDisabled}
                  onChange={(v) => update({ minimizeToTray: v })}
                />

                <Toggle
                  label={t('settings.closeToTray')}
                  hint={t('settings.closeToTrayHint')}
                  checked={shownSettings.closeToTray}
                  disabled={trayDisabled}
                  onChange={(v) => update({ closeToTray: v })}
                />

                {trayDisabled && <p className="settings__note">{t('settings.trayRequired')}</p>}
              </section>
            )}

            {shown === 'appearance' && (
              <section className="settings__section">
                <h3>{t('settings.themeHeading')}</h3>
                <p className="settings__note">{t('settings.themeNote')}</p>
                <div
                  className="settings__choices"
                  role="radiogroup"
                  aria-label={t('settings.themeHeading')}
                >
                  {(
                    [
                      { value: 'system', key: 'themeSystem', Icon: Monitor },
                      { value: 'dark', key: 'themeDark', Icon: Moon },
                      { value: 'light', key: 'themeLight', Icon: Sun },
                    ] as const
                  ).map(({ value, key, Icon }) => (
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
                      {t(`settings.${key}`)}
                    </button>
                  ))}
                </div>
              </section>
            )}

            {shown === 'language' && (
              <section className="settings__section">
                <h3>{t('settings.languageHeading')}</h3>
                <p className="settings__note">{t('settings.languageNote')}</p>

                <div
                  className="settings__languages"
                  role="radiogroup"
                  aria-label={t('settings.languageLabel')}
                >
                  {LOCALES.map(({ code, native }) => (
                    <button
                      key={code}
                      type="button"
                      role="radio"
                      aria-checked={language === code}
                      className="settings__language"
                      data-testid={`language-${code}`}
                      lang={code}
                      onClick={() => {
                        // Applied immediately, and persisted alongside it. A
                        // language picker that needs a restart is one people
                        // assume is broken.
                        setLanguage(code);
                        void update({ language: code });
                      }}
                    >
                      <span className="settings__langcheck">
                        {language === code && <Check size={12} aria-hidden="true" />}
                      </span>
                      {native}
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
        (CC&nbsp;BY&nbsp;4.0). Noto Sans, Noto Sans Display and Noto Sans Mono, with per-script
        Noto families covering CJK, Arabic, Hebrew, Indic and more, under the SIL Open Font
        License&nbsp;1.1. Lucide icons under ISC. Preview kits are synthesized in-repo and
        contain no third-party samples.
      </p>
    </section>
  );
}
