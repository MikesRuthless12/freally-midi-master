/**
 * The Settings categories, in the order the rail shows them.
 *
 * Its own module so the locale gate can import it without pulling in a React
 * component — and so exporting it does not break Fast Refresh, which requires a
 * component file to export components and nothing else.
 */
export const CATEGORIES = ['general', 'appearance', 'language', 'about'] as const;

export type CategoryId = (typeof CATEGORIES)[number];
