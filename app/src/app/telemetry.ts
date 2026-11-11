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
const WORDS = `token|secret|passw(?:or)?d|pwd|key|session(?:_?id)?|auth(?!or(?!i[sz]ation))\\w{0,32}|sig(?:nature)?`;
const BENIGN = `(?![\\w.$-]{0,64}(?:(?:hot|mon|tur|don|whis|jock|lac|mal)key|desig(?:nature)?)(?![\\w.$-]))`;
const CODE_WORDS = `(?:(?:o?auth|csrf|xsrf|otp|mfa|verification|reset|invite|recovery|grant|device)[_.-]?)?(?:code|state)`;
const COLON_WORDS = `(?:${WORDS})`;
const KEY_WORDS = `${BENIGN}(?:[\\w.$-]{0,64}(?:${WORDS})|${CODE_WORDS})`;
const PARAM_KEYS = `\\b${BENIGN}(?:[\\w-]{0,64}(?:${WORDS})|${CODE_WORDS})`;
const COLON_KEYS = `\\b${BENIGN}[\\w-]{0,64}${COLON_WORDS}`;
const SENSITIVE_FIELD = new RegExp(`^${BENIGN}(?!(?:cache|i18n|primary|sort|foreign|partition|translation|route|list|row|map|object|storage|query|lookup|group|unique|react|track|idempotency)[-_]?key$|auth(?:Method|Type|Provider|Mode|Status)$)[\\w.$ -]{0,64}${COLON_WORDS}$`, 'i');

const METRIC_FIELD =
  /(?:(?:^|[_. -])(?:duration|latency|ms|millis|seconds|secs|count|time|timestamp|size|bytes|ttl|age|attempts?|retries|expires(?:_?(?:in|at))?)|(?:Duration|Latency|Ms|Millis|Seconds|Secs|Count|Time|Timestamp|Size|Bytes|Ttl|TTL|Age|Attempts?|Retries|Expires(?:In|At)?))$/;

function isSecretName(key: string): boolean {
  if (SENSITIVE_FIELD.test(key)) return true;
  const lastSegment = key.split(/[[\]:/.]+/).filter(Boolean).pop();
  return lastSegment !== undefined && SENSITIVE_FIELD.test(lastSegment);
}

function isSecretField(key: string, value: unknown, inArray = false): boolean {
  if (!isSecretName(key)) return false;
  if (value === null || value === undefined || typeof value === 'boolean') return false;
  if (typeof value === 'number' || typeof value === 'bigint') return inArray || !METRIC_FIELD.test(key);
  if (typeof value !== 'object') return true;
  if (Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype !== Object.prototype && prototype !== null;
}

function defineEntry(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, { value, enumerable: true, writable: true, configurable: true });
}
const JWT = /\beyJ[\w-]*\.[\w-]+\.[\w-]*/g;
const BASIC = /\b([Bb][Aa][Ss][Ii][Cc])\s+(?:(?:(?=[A-Za-z0-9+/]*[0-9A-Z+/])[A-Za-z0-9+/]{8,}|[A-Za-z0-9+/]{2,})={1,2}(?![\w+/=])|(?=[A-Za-z0-9+/]*[0-9+/])[A-Za-z0-9+/]{16,}(?![\w+/=])|(?=[A-Za-z0-9+/]*[0-9A-Z+/])[A-Za-z0-9+/]{8,}(?![\w+/:=]))/g;
const EMAIL = /[\w.!#$%&*+?^`{|}~-][\w.!#$%&'*+?^`{|}~-]{0,63}=*@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const WORD_EMAIL = /(?!['/=])[\w.!#$%&'*+/=?^`{|}~-]{1,64}@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const QUOTED_EMAIL = /"[^"\s]{0,64}(?:\s[^"\s]{0,64}){0,4}"@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const USERINFO = /(\/\/)(?:(?!\[[\da-f:.]+\](?::\d+)?[?#]|[\w.-]+:\d+[?#])[^\s/?#@]*(?::[^\s/@]*)?|[^\s/?#@:]+:(?!\d+(?:[/?#]|$))[^\s@?#]*)@/g;
const QUOTE = `(?:\\\\{0,4}["'])?`;
const KEY_AHEAD = `\\\\{0,4}["']?[ \\t]*[([{<]?${KEY_WORDS}[ \\t]*\\\\{0,4}["']?[ \\t]*[:=]`;
const VALUE = `(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?|(?:(?:bearer|basic|token|digest|apikey|bot)[ \\t]+(?!${KEY_AHEAD})|[a-z][\\w-]{0,20}[ \\t]+(?!${KEY_AHEAD})(?=[^\\s"'<>&;,]{8}))?(?:(?![,;&|][ \\t]*${KEY_AHEAD})[^\\s"'])+(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?)?)`;
const QUOTED = `\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?`;
const CLOSE = `[}\\]>)](?=[\\s,;"']|$)`;
const CLOSED = `\\\\{0,4}"[^"]*"|\\\\{0,4}'[^']*'`;
const REAL_KEY_AHEAD = `\\\\{0,4}["']?[ \\t]*[([{<]?${KEY_WORDS}[ \\t]*\\\\{0,4}["']?[ \\t]*(?::|=(?!=*(?:[\\s,;"']|$)))`;
const SCHEME_LEAD = `(?:(?:bearer|basic|token|digest|apikey|api-key|bot|hmac|negotiate|ntlm)[ \\t]+(?!${REAL_KEY_AHEAD})(?=[^\\s,;"']))?`;
const SEGMENT = `${SCHEME_LEAD}(?:(?:${CLOSED})(?=[\\s,;}\\]>)]|$)(?![ \\t]*[:=])|(?:${QUOTED}|(?!${CLOSE})[^\\s,;"'])(?:(?![ \\t]+${KEY_AHEAD})(?:(?!${CLOSE})[^\\r\\n,;"']|${QUOTED}))*)`;
const BREAKS = `(?:\\r?\\n[ \\t]*){0,4}`;
const TOKEN_CHARS = `[^\\s"',;:=\\\\]`;
const BEARER_KEYED = `(?=${BENIGN}(?:[^\\s"',;:=\\\\]{0,64}(?:${WORDS})|${CODE_WORDS})\\\\{0,4}["']?[ \\t]*[:=])(?=(${TOKEN_CHARS}+))\\2\\\\{0,4}["']?[ \\t]*[:=]+[ \\t]*${BREAKS}(?:${SEGMENT})?`;
const BEARER = new RegExp(`\\b(bearer)(?:\\s+bearer){0,3}\\s+(?:\\\\{0,4}["'][ \\t]*[\\w.$-]{0,64}\\\\{0,4}["']?[ \\t]*[:=][ \\t]*${BREAKS}(?:${SEGMENT})?|${QUOTED}|${BEARER_KEYED}|(?=(${TOKEN_CHARS}{8,}(?:[:=]+${TOKEN_CHARS}*)*))\\3)`, 'gi');
const SENSITIVE_COLON = new RegExp(`(${QUOTE}${COLON_KEYS}${QUOTE}[ \\t]*:[ \\t]*${BREAKS})(?!bearer(?:[ \\t]+\\*|$))(?!\\*[}\\]>)]*(?:[\\s,;"']|$))(?!${KEY_AHEAD}[ \\t]*\\r?\\n)${SEGMENT}`, 'gi');
const SENSITIVE_PARAM = new RegExp(`(${QUOTE}${PARAM_KEYS}${QUOTE}\\s*=(?!=(?:[\\s=]|$))\\s*)(?:(?:${KEY_AHEAD}=*[ \\t]*)+(?:${VALUE})?|${VALUE})`, 'gi');
const HOST_TAIL = /^((?:[a-z][\w+.-]*:)?(?:\/\/)?(?:[\w-]+(?:\.[\w-]+)+|localhost|\[[\da-f:.]+\]|[\w-]+(?=:\d))(?::\d+)?)[?#].*$/i;
const PATH_TAIL = /[?#].*$/;
const HOST_BEFORE = /[a-z][\w+.-]*:\/\/[\w.-]*$/i;
const QUERY_QUOTED = /([?#&;][^\s=&#?;"'<>]+=)(?:\\{0,4}"[^"]*"?|\\{0,4}'[^']*'?)/g;
const PARAM_VALUE = /([?#&;][^\s=&#?;"'<>]+=)(?:\\{0,4}"[^"]*"?|\\{0,4}'[^']*'?|[^\s&#;"'<>]+)/g;

function redactWord(word: string): string {
  if (!MARKERS.test(word)) return word;
  if (word.length > MAX_WORD_LENGTH) return MASK;
  const bare = word.includes('//') ? word : word.replace(word.startsWith('/') ? EMAIL : WORD_EMAIL, MASK);
  const scrubbed = bare.replace(USERINFO, '$1').replace(HOST_TAIL, '$1');
  const cut = scrubbed.includes('/') ? scrubbed.replace(PATH_TAIL, '') : scrubbed;
  return cut.replace(PARAM_VALUE, `$1${MASK}`);
}

function maskColon(match: string, key: string, offset: number, text: string): string {
  const port = /:$/.test(key) && /^\d{1,5}(?=[/?#\s"'<>),;\]}]|:(?:\s|$)|$)/.test(match.slice(key.length));
  return port && HOST_BEFORE.test(text.slice(Math.max(0, offset - 256), offset)) ? match : key + MASK;
}

function redactPass(text: string): string {
  return text
    .replace(JWT, MASK)
    .replace(QUOTED_EMAIL, MASK)
    .replace(USERINFO, '$1')
    .replace(BEARER, `$1 ${MASK}`)
    .replace(SENSITIVE_COLON, maskColon)
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

interface Copies {
  readonly plain: WeakMap<object, unknown>;
  readonly secret: WeakMap<object, unknown>;
}

function newCopies(): Copies {
  return { plain: new WeakMap(), secret: new WeakMap() };
}

function redactDeep(value: unknown, seen = newCopies(), field = ''): unknown {
  if (typeof value === 'string') return redactText(value);
  if (value === null || typeof value !== 'object') return value;
  const scope = isSecretName(field) ? seen.secret : seen.plain;
  if (scope.has(value)) return scope.get(value);
  if (Array.isArray(value)) {
    const copy: unknown[] = [];
    scope.set(value, copy);
    for (const entry of value) copy.push(isSecretField(field, entry, true) ? MASK : redactDeep(entry, seen, field));
    return copy;
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return redactText(String(value));
  const copy: Record<string, unknown> = {};
  scope.set(value, copy);
  const taken = new Set<string>();
  for (const [key, entry] of Object.entries(value)) {
    const target = redactText(key);
    if (taken.has(target)) continue;
    taken.add(target);
    defineEntry(copy, target, isSecretField(key, entry) ? MASK : redactDeep(entry, seen, key));
  }
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
