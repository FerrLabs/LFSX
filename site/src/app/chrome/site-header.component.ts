import { ChangeDetectionStrategy, Component, LOCALE_ID, computed, inject } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router } from '@angular/router';
import { filter, map } from 'rxjs';

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
  private readonly router = inject(Router);

  protected readonly isFrench = inject(LOCALE_ID).startsWith('fr');
  private readonly prefix = this.isFrench ? '/fr' : '';

  protected readonly sections: readonly SectionLink[] = [
    { key: 'why', label: $localize`:@@nav.why:Why`, href: `${this.prefix}/#why` },
    {
      key: 'quickstart',
      label: $localize`:@@nav.quickstart:Quick start`,
      href: `${this.prefix}/#quickstart`,
    },
    { key: 'auth', label: $localize`:@@nav.auth:Auth`, href: `${this.prefix}/#auth` },
    { key: 'config', label: $localize`:@@nav.config:Config`, href: `${this.prefix}/#config` },
    { key: 'deploy', label: $localize`:@@nav.deploy:Deploy`, href: `${this.prefix}/#deploy` },
    { key: 'storage', label: $localize`:@@nav.storage:Storage`, href: `${this.prefix}/#storage` },
    { key: 'faq', label: 'FAQ', href: `${this.prefix}/#faq` },
  ];

  protected readonly homeHref = this.prefix || '/';

  // The path with no locale prefix and no fragment, which is the identity the
  // two language links agree on: each points at the same page under its own
  // prefix, wherever the visitor is standing.
  private readonly bare = toSignal(
    this.router.events.pipe(
      filter((event): event is NavigationEnd => event instanceof NavigationEnd),
      map((event) => event.urlAfterRedirects),
    ),
    { initialValue: this.router.url },
  );

  private readonly stripped = computed(() => {
    const path = this.bare().split('#')[0].split('?')[0];
    return path.replace(/^\/fr(?=\/|$)/, '') || '/';
  });

  protected readonly enHref = computed(() => this.stripped());
  protected readonly frHref = computed(() =>
    this.stripped() === '/' ? '/fr' : `/fr${this.stripped()}`,
  );
}
