import { ErrorHandler } from '@angular/core';
import { type TransportItem, faro, getWebInstrumentations, initializeFaro } from '@grafana/faro-web-sdk';

const appName = 'lfsx-app';

export type TelemetryEnvironment = 'dev' | 'interne' | 'home' | 'prod';

export function environmentFor(hostname: string): TelemetryEnvironment {
  if (hostname.endsWith('.dev.ferrlabs')) return 'dev';
  if (hostname.endsWith('.ferrlabs')) return 'interne';
  if (hostname.endsWith('.home')) return 'home';
  return 'prod';
}

const MAX_LENGTH = 2048;
const MAX_INPUT_LENGTH = 16384;
const MAX_WORD_LENGTH = 512;
const MAX_PASSES = 4;
const MASK = '*';
const MARKERS = /[?#=@]/;
const WORDS = `token|secret|passw(?:or)?d|pwd|key|session(?:_?id)?|auth\\w{0,32}|sig(?:nature)?`;
const COLON_WORDS = `(?:${WORDS})`;
const KEY_WORDS = `(?:${WORDS}|code|state)`;
const PARAM_KEYS = `[\\w-]{0,64}${KEY_WORDS}`;
const COLON_KEYS = `[\\w-]{0,64}${COLON_WORDS}`;
const SENSITIVE_FIELD = new RegExp(`^${COLON_KEYS}$`, 'i');
const JWT = /\beyJ[\w-]*\.[\w-]+\.[\w-]*/g;
const BASIC = /\b([Bb][Aa][Ss][Ii][Cc])\s+(?:(?=[A-Za-z0-9+/]*[0-9A-Z+/])[A-Za-z0-9+/]{8,}={0,2}|[A-Za-z0-9+/]{2,}={1,2})(?![\w+/:=])/g;
const EMAIL = /[\w.!#$%&*+?^`{|}~-]{1,64}@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const USERINFO = /(\/\/)(?!(?:[\w-]+(?:\.[\w-]+)+|localhost|\[[\da-f:.]+\]):\d+[?#])[^\s/?#@[]*(?::[^\s/@[]*)?@/gi;
const QUOTE = `(?:\\\\{0,4}["'])?`;
const KEY_AHEAD = `\\\\{0,4}["']?[ \\t]*[\\w.$-]{0,64}${KEY_WORDS}[ \\t]*\\\\{0,4}["']?[ \\t]*[:=]`;
const VALUE = `(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?|(?:(?:bearer|basic|token|digest|apikey|bot)[ \\t]+(?!${KEY_AHEAD})|[a-z][\\w-]{0,20}[ \\t]+(?!${KEY_AHEAD})(?=[^\\s"'<>&;,]{8}))?(?:(?![,;&|][ \\t]*${KEY_AHEAD})[^\\s"'<>])+)`;
const QUOTED = `\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?`;
const SEGMENT = `(?:${QUOTED}|[^\\s,;"'<>{}\\]\\\\])(?:(?![ \\t]+${KEY_AHEAD})(?:[^\\r\\n,;"'<>{}\\]\\\\]|${QUOTED}))*`;
const BREAKS = `(?:\\r?\\n[ \\t]*){0,4}`;
const TOKEN_CHARS = `[^\\s"',;:=\\\\]`;
const BEARER_KEYED = `(?=[^\\s"',;:=\\\\]{0,64}${KEY_WORDS}\\\\{0,4}["']?[ \\t]*[:=])(?=(${TOKEN_CHARS}+))\\2\\\\{0,4}["']?[ \\t]*[:=]+[ \\t]*${BREAKS}(?:${SEGMENT})?`;
const BEARER = new RegExp(`\\b(bearer)(?:\\s+bearer){0,3}\\s+(?:\\\\{0,4}["'][ \\t]*[\\w.$-]{0,64}\\\\{0,4}["']?[ \\t]*[:=][ \\t]*${BREAKS}(?:${SEGMENT})?|${QUOTED}|${BEARER_KEYED}|(?=(${TOKEN_CHARS}{8,}(?:=+${TOKEN_CHARS}*)*))\\3)`, 'gi');
const SENSITIVE_COLON = new RegExp(`(${QUOTE}${COLON_KEYS}${QUOTE}[ \\t]*:[ \\t]*${BREAKS})(?!bearer(?:[ \\t]+\\*|$))(?!${KEY_AHEAD}[ \\t]*\\r?\\n)${SEGMENT}`, 'gi');
const SENSITIVE_PARAM = new RegExp(`(${QUOTE}${PARAM_KEYS}${QUOTE}\\s*=(?!=)\\s*)(?:(?:${KEY_AHEAD}=*[ \\t]*)+(?:${VALUE})?|${VALUE})`, 'gi');
const HOST_TAIL = /^((?:[a-z][\w+.-]*:)?(?:\/\/)?(?:[\w-]+(?:\.[\w-]+)+|localhost|\[[\da-f:.]+\]|[\w-]+(?=:\d))(?::\d+)?)[?#].*$/i;
const PATH_TAIL = /[?#].*$/;
const HOST_BEFORE = /(?:[a-z][\w+.-]*:|[\w.-])\/\/[\w.-]*$/i;
const QUERY_QUOTED = /([?#&;][^\s=&#?;"'<>]+=)(?:\\{0,4}"[^"]*"?|\\{0,4}'[^']*'?)/g;
const PARAM_VALUE = /([?#&;][^\s=&#?;"'<>]+=)(?:\\{0,4}"[^"]*"?|\\{0,4}'[^']*'?|[^\s&#;"'<>]+)/g;

function redactWord(word: string): string {
  if (!MARKERS.test(word)) return word;
  if (word.length > MAX_WORD_LENGTH) return MASK;
  const bare = word.includes('//') ? word : word.replace(EMAIL, MASK);
  const scrubbed = bare.replace(USERINFO, '$1').replace(HOST_TAIL, '$1');
  const cut = scrubbed.includes('/') ? scrubbed.replace(PATH_TAIL, '') : scrubbed;
  return cut.replace(PARAM_VALUE, `$1${MASK}`);
}

function maskColon(match: string, key: string, offset: number, text: string): string {
  const port = /^\d{1,5}(?:[/?#\s]|$)/.test(match.slice(key.length));
  return port && HOST_BEFORE.test(text.slice(Math.max(0, offset - 256), offset)) ? match : key + MASK;
}

function redactPass(text: string): string {
  return text
    .replace(JWT, MASK)
    .replace(USERINFO, '$1')
    .replace(BEARER, `$1 ${MASK}`)    .replace(SENSITIVE_COLON, maskColon)
    .replace(SENSITIVE_PARAM, `$1${MASK}`)
    .replace(BASIC, `$1 ${MASK}`)
    .replace(QUERY_QUOTED, `$1${MASK}`)
    .replace(/\S+/g, redactWord)
    .replace(EMAIL, MASK);
}

function dropPartialWord(text: string): string {
  const cut = text.search(/\s\S*$/);
  return cut === -1 ? MASK : text.slice(0, cut);
}

function settle(text: string): string {
  for (let pass = 0; pass < MAX_PASSES; pass++) {
    const next = redactPass(text);
    if (next === text) break;
    text = next;
  }
  return text;
}

export function redactText(value: string): string {
  const text = settle(value.length > MAX_INPUT_LENGTH ? dropPartialWord(value.slice(0, MAX_INPUT_LENGTH)) : value);
  return text.length > MAX_LENGTH ? settle(dropPartialWord(text.slice(0, MAX_LENGTH + 1))) : text;
}

function redactDeep(value: unknown, seen = new WeakMap<object, unknown>()): unknown {
  if (typeof value === 'string') return redactText(value);
  if (value === null || typeof value !== 'object') return value;
  if (seen.has(value)) return seen.get(value);
  if (Array.isArray(value)) {
    const copy: unknown[] = [];
    seen.set(value, copy);
    for (const entry of value) copy.push(redactDeep(entry, seen));
    return copy;
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return redactText(String(value));
  const copy: Record<string, unknown> = {};
  seen.set(value, copy);
  for (const [key, entry] of Object.entries(value)) copy[key] = typeof entry === 'string' && SENSITIVE_FIELD.test(key) ? MASK : redactDeep(entry, seen);
  return copy;
}

export function sanitizeItem(item: TransportItem): TransportItem | null {
  try {
    return {
      ...item,
      meta: redactDeep(item.meta) as TransportItem['meta'],
      payload: redactDeep(item.payload) as TransportItem['payload'],
    };
  } catch {
    return null;
  }
}

export function isOptedOut(): boolean {
  const { globalPrivacyControl, doNotTrack } = navigator as Navigator & { globalPrivacyControl?: boolean };
  return globalPrivacyControl === true || doNotTrack === '1';
}

export function isLocalHost(hostname: string): boolean {
  const host = hostname.toLowerCase().replace(/\.$/, '').replace(/^\[|\]$/g, '');
  return host === 'localhost' || host === '::1' || /^127(?:\.\d{1,3}){3}$/.test(host);
}

export function initTelemetry(): void {
  if (LFSX_TELEMETRY_URL.length === 0) return;
  if (typeof window === 'undefined' || isLocalHost(window.location.hostname) || isOptedOut()) return;
  try {
    initializeFaro({
      url: LFSX_TELEMETRY_URL,
      app: { name: appName, environment: environmentFor(window.location.hostname) },
      instrumentations: getWebInstrumentations({ captureConsole: false }),
      beforeSend: sanitizeItem,
      sessionTracking: { enabled: false },
    });
  } catch {
    return;
  }
}

function toError(value: unknown): Error {
  if (value instanceof Error) return value;
  if (typeof value === 'object' && value !== null && 'message' in value && typeof value.message === 'string') {
    const error = new Error(value.message);
    if ('stack' in value && typeof value.stack === 'string') error.stack = value.stack;
    return error;
  }
  try {
    return new Error(String(value));
  } catch {
    return new Error('Non-Error value');
  }
}

function reportError(value: unknown): void {
  try {
    faro.api?.pushError(toError(value));
  } catch {
    return;
  }
}

export class TelemetryErrorHandler extends ErrorHandler {
  override handleError(error: unknown): void {
    reportError(error);
    super.handleError(error);
  }
}
