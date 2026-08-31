import { ChangeDetectionStrategy, Component } from '@angular/core';
import { SiteFrameComponent } from '../chrome/site-frame.component';

@Component({
  selector: 'flr-not-found',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SiteFrameComponent],
  template: `
    <flr-site-frame [title]="title" [description]="description" docFooter>
      <section class="not-found">
        <h1 i18n="@@notfound.title">That page is not here</h1>
        <p i18n="@@notfound.body">
          It may have moved with a release. The documentation index is the fastest way back.
        </p>
        <a href="/docs/quickstart" i18n="@@notfound.link">Go to the docs</a>
      </section>
    </flr-site-frame>
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
