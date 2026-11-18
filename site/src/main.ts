import '@angular/localize/init';
import { bootstrapApplication } from '@angular/platform-browser';
import { initTelemetry } from '@ferrlabs/telemetry';
import { appConfig } from './app/app.config';
import { AppComponent } from './app/app.component';

initTelemetry({ appName: 'lfsx-site' });

try {
  await bootstrapApplication(AppComponent, appConfig);
} catch (err) {
  console.error(err);
}
