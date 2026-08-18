import { ChangeDetectionStrategy, Component } from '@angular/core';
import { SiteShellComponent } from '@ferrlabs/ui-ng';

@Component({
  selector: 'flr-not-found',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SiteShellComponent],
  template: `
    <flr-site-shell [title]="title" [description]="description">
      <section class="not-found">
        <h1 i18n="@@notfound.title">That page is not here</h1>
        <p i18n="@@notfound.body">
          It may have moved with a release. The documentation index is the fastest way back.
        </p>
        <a href="/docs/quickstart" i18n="@@notfound.link">Go to the docs</a>
      </section>
    </flr-site-shell>
  `,
  styles: `
    .not-found {
      max-width: 36rem;
      margin: 0 auto;
      padding: 6rem 1.5rem;
      text-align: center;
    }
  `,
})
export default class NotFoundPage {
  protected readonly title = 'Not found: LFSX';
  protected readonly description = 'That page is not here.';
}
