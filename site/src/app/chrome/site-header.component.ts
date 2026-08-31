import { ChangeDetectionStrategy, Component } from '@angular/core';

interface SectionLink {
  readonly key: string;
  readonly label: string;
  readonly href: string;
}

@Component({
  selector: 'flr-site-header',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './site-header.component.html',
  styleUrl: './site-header.component.css',
})
export class SiteHeaderComponent {
  protected readonly sections: readonly SectionLink[] = [
    { key: 'why', label: $localize`:@@nav.why:Why`, href: '/#why' },
    { key: 'quickstart', label: $localize`:@@nav.quickstart:Quick start`, href: '/#quickstart' },
    { key: 'auth', label: $localize`:@@nav.auth:Auth`, href: '/#auth' },
    { key: 'config', label: $localize`:@@nav.config:Config`, href: '/#config' },
    { key: 'deploy', label: $localize`:@@nav.deploy:Deploy`, href: '/#deploy' },
    { key: 'storage', label: $localize`:@@nav.storage:Storage`, href: '/#storage' },
    { key: 'faq', label: 'FAQ', href: '/#faq' },
  ];
}
