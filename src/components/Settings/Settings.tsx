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

import { invoke } from '../../lib/ipc';
import { LOCALES } from '../../i18n/locales';
import { CATEGORIES, type CategoryId } from './categories';
import { useUi } from '../../state/ui';
import { useSession } from '../../state/session';
import { THEME_PREFERENCES, type ThemePreference } from '../../state/theme';
import './Settings.css';

/**
 * Settings — sidebar categories on the left, the selected pane on the right,
 * matching the shape used across the Freally apps.
 *
 * Every control here is real and persists. Nothing decorative: a settings
 * screen that shows a toggle which does not survive a restart is worse than
 * one that omits it. ⛔ That rule is why the tray options are gone rather than
 * disabled — the desktop shell owned the tray *and* `settings.json`, so with it
 * removed those three toggles had nothing behind them and nowhere to be saved.
 * Theme, language and motion persist to the WebView's own storage, which is the
 * only durable place a plugin has for a preference that is not part of a song.
 */

const CATEGORY_ICONS: Record<CategoryId, typeof Settings2> = {
  general: Settings2,
  appearance: Palette,
  language: Languages,
  about: Info,
};

/**
 * Search terms per category, so the filter matches content and not just titles.
 *
 * Deliberately English-only, and additive to the translated label the filter
 * also checks. Someone running a Japanese UI who searches "dataset" — because
 * that is the word in every tutorial they have read — should still find the
 * setting. The language pane lists every endonym so a lost user can search for
 * their own language in their own script.
 */
const CATEGORY_TERMS: Record<CategoryId, string> = {
  general: 'general dataset styles artists skipped problems',
  appearance: 'appearance theme dark light system colour color motion animation reduce',
  language: `language locale translation ${LOCALES.map((l) => `${l.english} ${l.native}`).join(' ')}`,
  about: 'about version licence license disclaimer credits artist names privacy',
};

/** Icons for the theme radios; the labels come from the catalog. */
const THEME_ICONS: Record<ThemePreference, typeof Sun> = {
  system: Monitor,
  dark: Moon,
  light: Sun,
};

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="settings__row">
      <input
        type="checkbox"
        checked={checked}
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
  // A model that failed to load is skipped rather than fatal (TASK-016), which
  // is right for the launch and wrong for the user: until now nothing outside
  // the console ever said so, and a missing artist looked like one that was
  // never authored.
  const problems = useSession((s) => s.problems);
  const roster = useSession((s) => s.roster);

  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const language = useUi((s) => s.language);
  const setLanguage = useUi((s) => s.setLanguage);
  const reduceMotion = useUi((s) => s.reduceMotion);
  const setReduceMotion = useUi((s) => s.setReduceMotion);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [onClose]);

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
                <h3>{t('settings.datasetHeading')}</h3>
                {problems.length === 0 ? (
                  <p className="settings__note">
                    {t('settings.datasetOk', { count: roster.length })}
                  </p>
                ) : (
                  // The list, not just the count. "3 models were skipped" is
                  // not something anyone can act on; the file and the reason
                  // are. Both are already on the wire from `roster_summary` —
                  // until now only the console ever saw them.
                  <div className="settings__problems" role="alert">
                    <p>{t('settings.datasetSkipped', { count: problems.length })}</p>
                    <ul>
                      {problems.map((problem) => (
                        <li key={problem.source}>
                          <code>{problem.source}</code> — {problem.message}
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
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
                  {THEME_PREFERENCES.map((value) => {
                    const Icon = THEME_ICONS[value];
                    return (
                      <button
                        key={value}
                        type="button"
                        role="radio"
                        aria-checked={theme === value}
                        className="settings__choice"
                        onClick={() => setTheme(value)}
                      >
                        <Icon size={16} aria-hidden="true" />
                        {t(`theme.short.${value}`)}
                      </button>
                    );
                  })}
                </div>

                <h3>{t('settings.motionHeading')}</h3>
                <Toggle
                  label={t('settings.reduceMotion')}
                  hint={t('settings.reduceMotionHint')}
                  checked={reduceMotion}
                  onChange={setReduceMotion}
                />
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
                      // Applied immediately, and persisted alongside it. A
                      // language picker that needs a restart is one people
                      // assume is broken.
                      onClick={() => setLanguage(code)}
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
      </div>
    </div>
  );
}

type AppInfo = { version: string; platform: string; arch: string };

/** Shared by the Settings → About pane and the standalone About overlay. */
export function AboutPane() {
  const { t } = useTranslation();
  const [info, setInfo] = useState<AppInfo | null>(null);

  // ⛔ Ungated. This used to be `if (!isTauri()) return`, which meant the pane
  // showed an em dash for the version and the platform in the one shell that
  // ships — the plugin's bridge has answered `app_info` all along. The mock
  // answers it too, so the dev server shows its own values rather than nothing.
  useEffect(() => {
    invoke<AppInfo>('app_info')
      .then(setInfo)
      .catch(() => setInfo(null));
  }, []);

  return (
    <section className="settings__section">
      {/* The product name is a brand, not copy — it does not translate. */}
      <h3>Freally MIDI Master</h3>
      <p className="settings__note">{t('about.tagline')}</p>

      <dl className="settings__facts">
        <dt>{t('about.version')}</dt>
        <dd>{info?.version ?? '—'}</dd>
        <dt>{t('about.platform')}</dt>
        <dd>{info ? `${info.platform} / ${info.arch}` : '—'}</dd>
        <dt>{t('about.licence')}</dt>
        <dd>{t('about.licenceValue')}</dd>
      </dl>

      <h3>{t('about.artistNamesHeading')}</h3>
      <p className="settings__note">{t('about.artistNamesBody')}</p>

      <h3>{t('about.creditsHeading')}</h3>
      <p className="settings__note">{t('about.creditsBody')}</p>
    </section>
  );
}
