import { derived, writable } from 'svelte/store';
import chinese from './locales/zh-CN.json';

export type Locale = 'en' | 'zh-CN';
export type Language = 'system' | Locale;
export const detectLanguage = (tag: string): Locale => /^zh(?:[-_.]|$)/i.test(tag) ? 'zh-CN' : 'en';
export const locale = writable<Locale>(detectLanguage(typeof navigator === 'undefined' ? 'en' : navigator.language));
export const resolveLanguage = (choice: Language, system: Locale): Locale => choice === 'system' ? system : choice;
const catalog: Record<string, string> = chinese;
type Parameters = Record<string, string | number>;
const patterns = Object.entries(catalog).filter(([key]) => key.includes('{')).map(([key, translated]) => {
  const names: string[] = [];
  const escaped = key.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const source = escaped.replace(/\\\{(\w+)\\\}/g, (_, name: string) => { names.push(name); return '(.+?)'; });
  return { regex: new RegExp(`^${source}$`, 's'), names, translated };
});

function message(key: string, depth = 0): string {
  if (catalog[key]) return catalog[key];
  if (depth > 4) return key;
  for (const { regex, names, translated } of patterns) {
    const match = regex.exec(key);
    if (!match) continue;
    const values = Object.fromEntries(names.map((name, index) => [name,
      ['error', 'message', 'label'].includes(name) ? message(match[index + 1], depth + 1) : match[index + 1]
    ]));
    return translated.replace(/\{(\w+)\}/g, (token, name: string) => values[name] ?? token);
  }
  return key;
}

// Translate application-owned messages; preserve unknown SSH/system diagnostics.
// Parameter values are inserted as text by Svelte, never as HTML.
export function translate(language: Locale, key: string, params: Parameters = {}): string {
  const template = language === 'zh-CN' ? message(key) : key;
  return template.replace(/\{(\w+)\}/g, (token, name: string) => String(params[name] ?? token));
}

export const t = derived(locale, language => (key: string, params?: Parameters) => translate(language, key, params));
