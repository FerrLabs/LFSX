import { ErrorHandler } from '@angular/core';
import {
  type EventEvent,
  type MeasurementEvent,
  type TransportItem,
  TransportItemType,
  faro,
  getWebInstrumentations,
  initializeFaro,
} from '@grafana/faro-web-sdk';

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

const capabilityPaths: readonly (readonly [RegExp, string])[] = [];

export function redactUrl(url: string): string {
  const redacted = capabilityPaths.reduce((value, [pattern, replacement]) => value.replace(pattern, replacement), url);
  return redacted.replace(/[?#].*$/s, '');
}

function redactValues(values: Record<string, string> | undefined): Record<string, string> | undefined {
  if (!values) return values;
  return Object.fromEntries(
    Object.entries(values).map(([key, value]) => [key, typeof value === 'string' ? redactUrl(value) : value]),
  );
}

export function sanitizeItem(item: TransportItem): TransportItem {
  const { page } = item.meta;
  const meta = page?.url ? { ...item.meta, page: { ...page, url: redactUrl(page.url) } } : item.meta;
  if (item.type === TransportItemType.EVENT) {
    const payload = item.payload as EventEvent;
    return { ...item, meta, payload: { ...payload, attributes: redactValues(payload.attributes) } };
  }
  if (item.type === TransportItemType.MEASUREMENT) {
    const payload = item.payload as MeasurementEvent;
    return { ...item, meta, payload: { ...payload, context: redactValues(payload.context) } };
  }
  return { ...item, meta };
}

export function initTelemetry(): void {
  if (typeof window === 'undefined' || localHostnames.includes(window.location.hostname)) return;
  initializeFaro({
    url: collectorUrl,
    app: { name: appName, environment: environmentFor(window.location.hostname) },
    instrumentations: getWebInstrumentations({ captureConsole: false }),
    beforeSend: sanitizeItem,
  });
}

export class TelemetryErrorHandler extends ErrorHandler {
  override handleError(error: unknown): void {
    faro.api?.pushError(error instanceof Error ? error : new Error(String(error)));
    super.handleError(error);
  }
}
