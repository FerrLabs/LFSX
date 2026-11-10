import { ErrorHandler } from '@angular/core';
import { type TransportItem, faro, getWebInstrumentations, initializeFaro } from '@grafana/faro-web-sdk';

const collectorUrl = 'https://rum.ferrlabs.com/collect';
const appName = 'lfsx-site';
const localHostnames = ['localhost', '127.0.0.1'];

export type TelemetryEnvironment = 'dev' | 'interne' | 'home' | 'prod';

export function environmentFor(hostname: string): TelemetryEnvironment {
  if (hostname.endsWith('.dev.ferrlabs')) return 'dev';
  if (hostname.endsWith('.ferrlabs')) return 'interne';
  if (hostname.endsWith('.home')) return 'home';
  return 'prod';
}

const MAX_LENGTH = 2048;
const MAX_WORD_LENGTH = 512;
const MARKERS = /[?#=@]/;
const AUTH_SCHEME = /\b(bearer)\s+[^\s"',;]+|\b(basic)\s+[A-Za-z0-9+/]{8,}={0,2}/gi;
const JWT = /\beyJ[\w-]*\.[\w-]+\.[\w-]*/g;
const USERINFO = /(\/\/)[^\s/?#]*@/g;
const PATH_TAIL = /[?#].*$/;
const PARAM_VALUE = /([?#&;][^\s=&#?;"'<>]+=)[^\s&#;"'<>]*/g;
const SENSITIVE_PARAM =
  /([\w-]*(?:token|secret|passw(?:or)?d|pwd|key|session(?:id)?|auth\w*|sig(?:nature)?|code|state)=)[^\s&#;"'<>]*/gi;

function redactWord(word: string): string {
  if (!MARKERS.test(word)) return word;
  if (word.length > MAX_WORD_LENGTH) return ':redacted';
  const scrubbed = word.replace(USERINFO, '$1');
  const cut = scrubbed.includes('/') ? scrubbed.replace(PATH_TAIL, '') : scrubbed;
  return cut.replace(PARAM_VALUE, '$1:redacted').replace(SENSITIVE_PARAM, '$1:redacted');
}

export function redactText(value: string): string {
  const text = value.length > MAX_LENGTH ? value.slice(0, MAX_LENGTH) : value;
  return text
    .replace(AUTH_SCHEME, (_match: string, bearer?: string, basic?: string) => `${bearer ?? basic} :redacted`)
    .replace(JWT, ':redacted')
    .replace(/\S+/g, redactWord);
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
  if (Object.getPrototypeOf(value) !== Object.prototype) return value;
  const copy: Record<string, unknown> = {};
  seen.set(value, copy);
  for (const [key, entry] of Object.entries(value)) copy[key] = redactDeep(entry, seen);
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

export function initTelemetry(): void {
  if (typeof window === 'undefined' || localHostnames.includes(window.location.hostname) || isOptedOut()) return;
  initializeFaro({
    url: collectorUrl,
    app: { name: appName, environment: environmentFor(window.location.hostname) },
    instrumentations: getWebInstrumentations({ captureConsole: false }),
    beforeSend: sanitizeItem,
    sessionTracking: { enabled: false },
  });
}

function toError(value: unknown): Error {
  if (value instanceof Error) return value;
  if (typeof value === 'object' && value !== null && 'message' in value && typeof value.message === 'string') {
    return new Error(value.message);
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
