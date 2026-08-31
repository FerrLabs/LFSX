import {
  ChangeDetectionStrategy,
  Component,
  DOCUMENT,
  LOCALE_ID,
  booleanAttribute,
  effect,
  inject,
  input,
} from '@angular/core';
import { Meta, Title } from '@angular/platform-browser';
import { Router } from '@angular/router';
import { SiteHeaderComponent } from './site-header.component';

const ORIGIN = 'https://lfsx.dev';

@Component({
  selector: 'flr-site-frame',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SiteHeaderComponent],
  templateUrl: './site-frame.component.html',
  styleUrl: './site-frame.component.css',
})
export class SiteFrameComponent {
  readonly title = input.required<string>();
  readonly description = input('');
  readonly docFooter = input(false, { transform: booleanAttribute });

  // Off for the docs, whose body is English under every prefix: a page that is
  // not really translated must not advertise a French alternate, it points its
  // canonical at the English URL instead so the crawler indexes one of them.
  readonly localizedContent = input(true, { transform: booleanAttribute });

  private readonly titleService = inject(Title);
  private readonly metaService = inject(Meta);
  private readonly document = inject(DOCUMENT);
  private readonly locale = inject(LOCALE_ID);
  private readonly router = inject(Router);

  constructor() {
    effect(() => {
      this.titleService.setTitle(this.title());
      const description = this.description();
      if (description) {
        this.metaService.updateTag({ name: 'description', content: description });
      }

      const path = this.router.url.split('#')[0].split('?')[0];
      const bare = path.replace(/^\/fr(?=\/|$)/, '') || '/';
      const french = `/fr${bare === '/' ? '' : bare}`;

      if (this.localizedContent()) {
        this.setLink('canonical', null, ORIGIN + (this.locale.startsWith('fr') ? french : bare));
        this.setLink('alternate', 'en', ORIGIN + bare);
        this.setLink('alternate', 'fr', ORIGIN + french);
        this.setLink('alternate', 'x-default', ORIGIN + bare);
      } else {
        this.setLink('canonical', null, ORIGIN + bare);
      }
    });
  }

  private setLink(rel: string, hreflang: string | null, href: string): void {
    const selector = hreflang
      ? `link[rel="${rel}"][hreflang="${hreflang}"]`
      : `link[rel="${rel}"]`;
    let link = this.document.head.querySelector<HTMLLinkElement>(selector);
    if (!link) {
      link = this.document.createElement('link');
      link.rel = rel;
      if (hreflang) {
        link.hreflang = hreflang;
      }
      this.document.head.appendChild(link);
    }
    link.href = href;
  }
}
