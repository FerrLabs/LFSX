import { bootstrapApplication } from '@angular/platform-browser';
import { initTelemetry } from '@ferrlabs/telemetry';

import { App } from './app/app';
import { appConfig } from './app/app.config';

if (LFSX_TELEMETRY_URL.length > 0) {
  initTelemetry({ appName: 'lfsx-app', collectorUrl: LFSX_TELEMETRY_URL });
}

bootstrapApplication(App, appConfig).catch((error: unknown) => console.error(error));
