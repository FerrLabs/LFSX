import { ChangeDetectionStrategy, Component, computed, inject, input, LOCALE_ID } from '@angular/core';
import { DocsLayoutComponent } from '@ferrlabs/ui-ng/docs';
import { resolveLocale } from '@ferrlabs/ui-ng';
import { SiteFrameComponent } from '../chrome/site-frame.component';
import { DOCS_NAV } from './docs-nav';

@Component({
  selector: 'flr-docs-page',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SiteFrameComponent, DocsLayoutComponent],
  template: `
    <flr-site-frame [title]="metaTitle()" [description]="description()" docFooter localizedContent="false">
      <flr-docs-layout [nav]="DOCS_NAV" [lang]="locale" [slug]="slug()">
        <ng-content />
      </flr-docs-layout>
    </flr-site-frame>
  `,
})
export class DocsPageComponent {
  readonly slug = input.required<string>();
  readonly title = input('');
  readonly description = input('');

  protected readonly DOCS_NAV = DOCS_NAV;
  protected readonly locale = resolveLocale(inject(LOCALE_ID));

  protected readonly metaTitle = computed(() => {
    const title = this.title();
    return title ? `${title}: LFSX Docs` : 'LFSX Docs';
  });
}
