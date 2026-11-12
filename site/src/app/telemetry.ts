import { ErrorHandler } from '@angular/core';
import { type TransportItem, faro, getWebInstrumentations, initializeFaro } from '@grafana/faro-web-sdk';

const collectorUrl = 'https://rum.ferrlabs.com/collect';
const appName = 'lfsx-site';

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
const WORDS = `token|secret|cookie|passw(?:or)?d|pwd|key|session(?:_?id)?|auth(?!or(?!i[sz]ation))\\w{0,32}|sig(?:nature)?`;
const BENIGN_CODE = `(?:(?:error|status|response|http|exit|return|result|reason|country|zip|postal|post|currency|lang(?:uage)?|locale|region|area|promo|scan|colou?r|event|product|item|sku|font|short|source|geo|trans|hash|dial|tax|class|airport|iata|icao|type|kind|category|state|bar|micro|char|version|room|lobby|game|level|build|seat|table|zone|course)[_.-]?codes?|(?:ready|visibility|loading|playback)[_.-]?states?)`;
const BENIGN_SHORT_CODE = `(?:(?:err|resp|uni|en|de|op|qr|iso|src)[_.-]?codes?)`;
const BENIGN_STATE = `(?:(?:ready|visibility|ui|app|view|page|form|nav(?:igation)?|router?|connection|network|player|game|load(?:ing)?|render|component|store|signaling|ice(?:[_.-]?(?:gathering|connection))?|gathering|lifecycle|button|toggle|menu|modal|sort|filter|scroll|focus|hover|play(?:back)?|media|audio|video|animation|transition|editor|selection|document|window|sync|job|task|machine|initial|current|prev(?:ious)?|next|error|status|search|query|cache|billing|shipping|address|delivery|workflow|build|deploy(?:ment)?|pipeline|cart|checkout|upload|download|online|offline|power|battery|health|stream|queue|match|lobby|round|level|room|combat|board|turn|battle|world|scene|hand|deck|card)[_.-]?states?)`;
const BENIGN = `(?:(?=auth(?!or(?!i[sz]ation)))|(?!keys?[_.-]?codes?(?![\\w.$-])|(?:[\\w.$[\\]-]{0,63}[_.$[\\]-])?(?:(?:dic|don|doohic|flu|flun|hic|hoc|ho|hoo|hot|joc|lac|malar|mic|mon|po|ric|smo|tur|turn|whis)keys?|desig(?:nature)?|${BENIGN_SHORT_CODE}|${BENIGN_STATE})(?![\\w.$-])|[\\w.$[\\]-]{0,64}${BENIGN_CODE}(?![\\w.$-])))`;
const BENIGN_LONG = BENIGN.replaceAll('{0,63}', '{0,2047}').replaceAll('{0,64}', '{0,2047}');
const CODE_WORDS = `[\\w.$[\\]-]{0,64}(?:code|state)\\]{0,4}`;
const COLON_WORDS = `(?:${WORDS})`;
const KEY_WORDS = `${BENIGN}(?:[\\w.$[\\]-]{0,64}(?:${WORDS})\\]{0,4}|${CODE_WORDS})`;
const PARAM_KEYS = `\\b(?:(?=\\w{65})${BENIGN_LONG}(?:\\w{32}){2,64}?)?${BENIGN}(?:[\\w.[\\]-]{0,64}(?:${WORDS})\\]{0,4}|${CODE_WORDS})`;
const COLON_KEYS = `\\b(?:(?=\\w{65})${BENIGN_LONG}(?:\\w{32}){2,64}?)?${BENIGN}(?:[\\w[\\]-]{0,64}${COLON_WORDS}|[\\w[\\]-]{1,64}(?:code|state))\\]{0,4}`;
const SENSITIVE_FIELD = new RegExp(`^${BENIGN}(?!(?:[\\w.$-]{0,64}[_.$-])?(?:list|row|map)[-_]?keys?$|(?:[\\w$-]{0,64}\\.)?(?:primary|query|object|foreign|lookup|group|unique)[-_]?keys?$|[\\w.$-]{0,64}(?:cache|i18n|sort|partition|translation|route|react|track|idempotency)[-_]?keys?$|[\\w.$-]{0,64}auth[-_]?(?:Method|Type|Provider|Mode|Status)$)[\\w.$-]{0,64}${COLON_WORDS}s?$|^${BENIGN}[\\w.$-]{1,64}(?:code|state)s?$`, 'i');

const METRIC_FIELD =
  /(?:(?:^|[_. -])(?:duration|latency|ms|millis|seconds|secs|count|time|timestamp|size|bytes|ttl|age|attempts?|retries|expires(?:_?(?:in|at))?)|(?:^|_)(?:DURATION|LATENCY|MS|MILLIS|SECONDS|SECS|COUNT|TIME|TIMESTAMP|SIZE|BYTES|TTL|AGE|ATTEMPTS?|RETRIES|EXPIRES(?:_?(?:IN|AT))?)|(?:Duration|Latency|Ms|Millis|Seconds|Secs|Count|Time|Timestamp|Size|Bytes|Ttl|TTL|Age|Attempts?|Retries|Expires(?:In|At)?)|(?:^|[_. -])(?:input|output|total|max|prompt|completion|cached)[_-]?[Tt]okens?|(?:^|_)(?:INPUT|OUTPUT|TOTAL|MAX|PROMPT|COMPLETION|CACHED)[_-]?TOKENS?|(?:Input|Output|Total|Max|Prompt|Completion|Cached)Tokens?)$/;

function isSecretName(key: string): boolean {
  const normalized = key.replace(/[^\w.$-]+/g, '_').replace(/^_+|_+$/g, '');
  if (SENSITIVE_FIELD.test(normalized)) return true;
  if (normalized.length <= 64) return false;
  const tail = normalized.slice(-64);
  return SENSITIVE_FIELD.test(tail.replace(/^[^_.$-]*[_.$-]/, '')) || SENSITIVE_FIELD.test(tail);
}

function isSecretValue(value: unknown, metricKey?: string): boolean {
  if (value === null || value === undefined || typeof value === 'boolean') return false;
  if (typeof value === 'number' || typeof value === 'bigint')
    return metricKey === undefined || !METRIC_FIELD.test(metricKey);
  if (typeof value !== 'object') return true;
  if (Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype !== Object.prototype && prototype !== null;
}

function isSecretField(key: string, value: unknown): boolean {
  return isSecretName(key) && isSecretValue(value, key);
}

function defineEntry(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, { value, enumerable: true, writable: true, configurable: true });
}
const JWT = /\beyJ[\w-]*\.[\w-]+\.[\w-]*/g;
const BASIC = /\b([Bb][Aa][Ss][Ii][Cc])\s+(?:(?:(?=[A-Za-z0-9+/]*[0-9A-Z+/])[A-Za-z0-9+/]{8,}|[A-Za-z0-9+/]{2,})={1,2}(?![\w+/=])|(?=[A-Za-z0-9+/]*[0-9+/])[A-Za-z0-9+/]{16,}(?![\w+/=])|(?=[A-Za-z0-9+/]*[0-9A-Z+/])[A-Za-z0-9+/]{8,}(?![\w+/:=]))(?:[,;](?![\w.$[\]-]+[ \t]*(?:=|:(?:[ \t]|$)))[^\s,;"']+)*/g;
const EMAIL = /[\w.!#$%&*+?^`{|}~-][\w.!#$%&'*+?^`{|}~-]{0,63}=*@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const WORD_EMAIL = /(?!['/=])[\w.!#$%&'*+/=?^`{|}~-]{1,64}@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const QUOTED_EMAIL = /"[^"\s]{0,64}(?:\s[^"\s]{0,64}){0,4}"@[\w-]{1,63}(?:\.[\w-]{1,63})+/g;
const USERINFO = /(\/\/)(?:(?!\[[\da-f:.]+\](?::\d+)?[?#]|[\w.-]+:\d+[?#])[^\s/?#@]*(?::[^\s/@]*)?|[^\s/?#@:[]+:(?!\d+(?:[/?#]|$))[^\s@]*|[^\s/?#@:[]*:\d+\/[^\s/?#@]*)@(?![+!*'"(),;<>=])/g;
const QUOTE = `(?:\\\\{0,4}["'])?`;
const KEY_HEAD = `(?:\\\\{0,4}["'][ \\t]*)?[([{<]?${KEY_WORDS}[ \\t]*(?:\\\\{0,4}["'][ \\t]*)?`;
const KEY_AHEAD = `${KEY_HEAD}[:=]`;
const QVAL = `(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?)(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?|(?![?#][ \\t]*${KEY_AHEAD})[^\\s"',;&|])*`;
const VALUE_JOIN = `|[ \\t]+(?![ \\t])(?![\\w.$[\\]-]{1,64}[ \\t]*(?:=|:(?:[ \\t]|$)))(?!${KEY_AHEAD})(?=[^\\r\\n,;&|])`;
const VALUE = `(?:${QVAL}|(?:(?:bearer|basic|token|digest|apikey|bot)[ \\t]+(?![ \\t])(?!${KEY_AHEAD})|[a-z][\\w-]{0,20}[ \\t]+(?![ \\t])(?![\\w.$[\\]-]{1,64}[ \\t]*(?:=|:(?:[ \\t]|$)))(?!${KEY_AHEAD})(?=[^\\s"'<>&;,]{8}))?(?:(?![,;&|?#][ \\t]*${KEY_AHEAD})[^\\t\\n\\r ,;"']|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$))))+(?:\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?|(?![,;&|?#][ \\t]*${KEY_AHEAD})[^\\t\\n\\r ,;"']|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$)))${VALUE_JOIN})*)`;
const VALUE_WORD = VALUE.replace(VALUE_JOIN, '');
const QUOTED = `\\\\{0,4}"[^"]*"?|\\\\{0,4}'[^']*'?`;
const CLOSE = `[}\\]>)](?=[\\s,;"']|$)`;
const CLOSED = `\\\\{0,4}"[^"]*"|\\\\{0,4}'[^']*'`;
const REAL_KEY_AHEAD = `${KEY_HEAD}(?::|=(?!=*(?:[\\s,;"']|$)))`;
const SCHEME_LEAD = `(?:(?:bearer|basic|token|digest|apikey|api-key|bot|hmac|negotiate|ntlm)[ \\t]+(?![ \\t])(?!${REAL_KEY_AHEAD})(?=[^\\s,;"']))?`;
const SEGMENT = `${SCHEME_LEAD}(?:(?:${CLOSED})(?=[\\s,;}\\]>)]|$)(?![ \\t]*[:=])|(?:${QUOTED}|(?!${CLOSE})[^\\t\\n\\r ,;"']|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$))))(?:[ \\t]+(?![ \\t])(?!${KEY_AHEAD})(?=[^\\r\\n,;])|(?!${CLOSE})(?![|&#?]${KEY_AHEAD})[^\\t\\n\\r ,;"']|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$)))|${QUOTED})*)`;
const BREAKS = `(?:\\r?\\n[ \\t]*(?![ \\t])){0,4}`;
const TOKEN_CHARS = `(?:(?![&#|?{(\\[<]${KEY_AHEAD})[^\\s"',;:=\\\\])`;
const BEARER_KEYED = `(?=${BENIGN}(?:[^\\s"',;:=\\\\]{0,64}(?:${WORDS})|${CODE_WORDS})\\\\{0,4}["']?[ \\t]*[:=])(?=(${TOKEN_CHARS}+))\\2\\\\{0,4}["']?[ \\t]*[:=]+[ \\t]*(?![ \\t])${BREAKS}(?:${SEGMENT})?`;
const BEARER = new RegExp(`\\b(bearer)(?:\\s+bearer){0,3}\\s+(?:\\\\{0,4}["'][ \\t]*[\\w.$-]{0,64}\\\\{0,4}["']?[ \\t]*[:=][ \\t]*(?![ \\t])${BREAKS}(?:${SEGMENT})?|${QUOTED}|${BEARER_KEYED}|(?=(?:[^\\s"',;]|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$)))){8})(?=((?:${TOKEN_CHARS}|[,;](?![ \\t\\r\\n]|$|[\\w.$[\\]-]+[ \\t]*(?:=|:(?:[ \\t]|$)))|\\\\(?!\\\\{0,3}["'])|[:=]+(?!${KEY_AHEAD}))+))\\3)`, 'gi');
const YAML_BLOCK = new RegExp(`^([ \\t]*)(${QUOTE}${COLON_KEYS}${QUOTE}[ \\t]*:[ \\t]*)[|>][-+1-9]{0,2}[ \\t]*(?:#[^\\r\\n]*)?((?:\\r?\\n(?:[ \\t]*(?=\\r?\\n|$)|\\1[ \\t]+[^\\r\\n]*))+)`, 'gim');
const SENSITIVE_COLON = new RegExp(`(${QUOTE}${COLON_KEYS}${QUOTE}[ \\t]*:[ \\t]*(?![ \\t])${BREAKS})(?!bearer(?:[ \\t]+\\*|$))(?!\\*[}\\]>)]*(?:[\\s,;"']|$))(?:(?:${KEY_AHEAD}[ \\t]*(?![ \\t])${BREAKS})+(?:${SEGMENT})?|${SEGMENT})`, 'gi');
const SENSITIVE_QUERY_PARAM = new RegExp(`([?&#;]${QUOTE}${PARAM_KEYS}${QUOTE}\\s*=(?!=(?:[\\s=]|$))=*\\s*(?!\\s))(?:(?:${KEY_AHEAD}=*[ \\t]*(?![ \\t]))+(?:${VALUE_WORD})?|${VALUE_WORD})`, 'gi');
const SENSITIVE_PARAM = new RegExp(`((?:^|[^\\w.?&#;-])${QUOTE}${PARAM_KEYS}${QUOTE}\\s*=(?!=(?:[\\s=]|$))=*\\s*(?!\\s))(?:(?:${KEY_AHEAD}=*[ \\t]*(?![ \\t]))+(?:${VALUE})?|${VALUE})`, 'gi');
const HOST_TAIL = /^((?:[a-z][\w+.-]*:)?(?:\/\/)?(?:[\w-]+(?:\.[\w-]+)+|localhost|\[[\da-f:.]+\]|[\w-]+(?=:\d))(?::\d+)?)[?#].*$/i;
const PATH_TAIL = /[?#].*$/;
const HOST_BEFORE = /[a-z][\w+.-]*:\/\/[\w.-]*$/i;
const BARE_HOST_BEFORE = /(?:^|[^\w:/])\/\/[\w.-]*$/;
const CREDENTIAL_HOST = /(?:passw(?:or)?d|pwd|secret|token|api[_-]?key|session(?:_?id)?|cookie):$/i;
const COOKIE_HEADER = /\b((?:set-)?cookie[ \t]*:[ \t]*)(?![ \t]|\*$)[^\r\n]+/gi;
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

function isHostPort(text: string, offset: number, key: string, rest: string): boolean {
  const before = text.slice(Math.max(0, offset - 256), offset);
  return HOST_BEFORE.test(before) || (BARE_HOST_BEFORE.test(before) && !CREDENTIAL_HOST.test(key) && /^\d{1,5}\//.test(rest));
}

function maskColon(match: string, key: string, offset: number, text: string): string {
  const port = /:$/.test(key) && /^\d{1,5}(?=[/?#\s"'<>),;\]}]|:(?:\s|$)|$)/.test(match.slice(key.length));
  return port && isHostPort(text, offset, key, match.slice(key.length)) ? match : key + MASK;
}

function redactPass(text: string): string {
  return text
    .replace(JWT, MASK)
    .replace(QUOTED_EMAIL, MASK)
    .replace(COOKIE_HEADER, `$1${MASK}`)
    .replace(USERINFO, '$1')
    .replace(BEARER, `$1 ${MASK}`)
    .replace(YAML_BLOCK, `$1$2${MASK}`)
    .replace(SENSITIVE_COLON, maskColon)
    .replace(SENSITIVE_QUERY_PARAM, `$1${MASK}`)
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

const SESSION_FIELD = /^session$/i;

function isPlainObject(value: object): boolean {
  const prototype = Object.getPrototypeOf(value);
  return prototype === null || prototype === Object.prototype;
}

function isPlainRecord(value: unknown): boolean {
  return typeof value === 'object' && value !== null && isPlainObject(value);
}

type Entry = readonly [string, unknown];

interface Snapshot {
  readonly elements: WeakMap<readonly unknown[], readonly unknown[]>;
  readonly entries: WeakMap<object, readonly Entry[]>;
}

interface Walk extends Snapshot {
  readonly secret: WeakSet<object>;
  readonly copies: WeakMap<object, unknown>;
}

function elementsOf(snapshot: Snapshot, array: readonly unknown[]): readonly unknown[] {
  const known = snapshot.elements.get(array);
  if (known !== undefined) return known;
  const taken = Array.from(array);
  snapshot.elements.set(array, taken);
  return taken;
}

function entriesOf(snapshot: Snapshot, container: object): readonly Entry[] {
  const known = snapshot.entries.get(container);
  if (known !== undefined) return known;
  const taken = Object.entries(container);
  snapshot.entries.set(container, taken);
  return taken;
}

function isTraversable(value: object): boolean {
  return Array.isArray(value) || isPlainObject(value);
}

function collectSecretContainers(
  meta: unknown,
  payload: unknown,
  snapshot: Snapshot,
): WeakSet<object> {
  const secret = new WeakSet<object>();
  const seenPlain = new WeakSet<object>();
  const seenMetaRoot = new WeakSet<object>();
  const visit = (value: unknown, inSecret: boolean, isMetaRoot = false): void => {
    if (value === null || typeof value !== 'object' || !isTraversable(value)) return;
    const seen = inSecret ? secret : isMetaRoot ? seenMetaRoot : seenPlain;
    if (seen.has(value)) return;
    seen.add(value);
    if (Array.isArray(value)) {
      for (const entry of elementsOf(snapshot, value)) visit(entry, inSecret);
      return;
    }
    for (const [key, entry] of entriesOf(snapshot, value)) {
      const exempt = isMetaRoot && SESSION_FIELD.test(key) && isPlainRecord(entry);
      visit(entry, inSecret || (isSecretName(key) && !exempt));
    }
  };
  visit(meta, false, true);
  visit(payload, false);
  return secret;
}

function newWalk(meta: unknown, payload: unknown): Walk {
  const snapshot: Snapshot = { elements: new WeakMap(), entries: new WeakMap() };
  return {
    ...snapshot,
    secret: collectSecretContainers(meta, payload, snapshot),
    copies: new WeakMap(),
  };
}

function redactDeep(value: unknown, walk: Walk): unknown {
  if (typeof value === 'string') return redactText(value);
  if (value === null || typeof value !== 'object') return value;
  if (!isTraversable(value)) return redactText(String(value));
  const known = walk.copies.get(value);
  if (known !== undefined) return known;
  const inSecret = walk.secret.has(value);
  if (Array.isArray(value)) {
    const copy: unknown[] = [];
    walk.copies.set(value, copy);
    for (const entry of elementsOf(walk, value))
      copy.push(inSecret && isSecretValue(entry) ? MASK : redactDeep(entry, walk));
    return copy;
  }
  const copy: Record<string, unknown> = {};
  walk.copies.set(value, copy);
  const taken = new Set<string>();
  for (const [key, entry] of entriesOf(walk, value)) {
    const base = inSecret ? MASK : redactText(key);
    let target = base;
    for (let n = 2; taken.has(target); n++) target = `${base}#${n}`;
    taken.add(target);
    const masked = inSecret ? isSecretValue(entry) : isSecretField(key, entry);
    defineEntry(copy, target, masked ? MASK : redactDeep(entry, walk));
  }
  return copy;
}

export function sanitizeItem(item: TransportItem): TransportItem | null {
  try {
    const { meta, payload } = item;
    const walk = newWalk(meta, payload);
    return {
      ...item,
      meta: redactDeep(meta, walk) as TransportItem['meta'],
      payload: redactDeep(payload, walk) as TransportItem['payload'],
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
  if (typeof window === 'undefined' || isLocalHost(window.location.hostname) || isOptedOut()) return;
  try {
    initializeFaro({
      url: collectorUrl,
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
