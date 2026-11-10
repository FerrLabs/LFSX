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

const urlWithSecrets =
  /((?:[a-z][a-z0-9+.-]*:)?\/\/[^\s?#]*|\b[\w-]+(?:\.[\w-]+)+\/[^\s?#]*|(?<![\w/.])\/[^\s?#]*)[?#]\S*/gi;

export function redactUrl(text: string): string {
  return text.replace(urlWithSecrets, '$1');
}

function redactDeep(value: unknown, seen = new WeakMap<object, unknown>()): unknown {
  if (typeof value === 'string') return redactUrl(value);
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

export function sanitizeItem(item: TransportItem): TransportItem {
  return {
    ...item,
    meta: redactDeep(item.meta) as TransportItem['meta'],
    payload: redactDeep(item.payload) as TransportItem['payload'],
  };
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
