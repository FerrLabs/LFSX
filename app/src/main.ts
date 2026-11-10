import { bootstrapApplication } from '@angular/platform-browser';

import { App } from './app/app';
import { appConfig } from './app/app.config';
import { initTelemetry } from './app/telemetry';

initTelemetry();

bootstrapApplication(App, appConfig).catch((error: unknown) => console.error(error));
