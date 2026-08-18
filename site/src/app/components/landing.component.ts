import { ChangeDetectionStrategy, Component } from '@angular/core';

interface Property {
  readonly key: string;
  readonly title: string;
  readonly body: string;
}

@Component({
  selector: 'flr-landing',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './landing.component.html',
  styleUrl: './landing.component.css',
})
export class LandingComponent {
  protected readonly run = [
    'docker run -d --name lfsx \\',
    '  -p 8080:8080 \\',
    '  -v lfsx-data:/var/lib/lfsx \\',
    '  ghcr.io/ferrlabs/lfsx:latest',
  ].join('\n');

  protected readonly properties: readonly Property[] = [
    {
      key: 'fast',
      title: $localize`:@@landing.fast.title:Fast`,
      body: $localize`:@@landing.fast.body:Uploads and downloads stream end to end. A multi-gigabyte asset costs the same resident memory as a one-kilobyte icon, and the digest is computed on the bytes as they pass.`,
    },
    {
      key: 'light',
      title: $localize`:@@landing.light.title:Lightweight`,
      body: $localize`:@@landing.light.body:One statically linked binary, a distroless image, no database. Objects live on the filesystem addressed by digest, or in an S3-compatible bucket.`,
    },
    {
      key: 'secure',
      title: $localize`:@@landing.secure.title:Secure`,
      body: $localize`:@@landing.secure.body:Access mirrors the upstream repository, so revoking someone there revokes them here. Every object is verified against its declared digest before it is accepted.`,
    },
  ];
}
