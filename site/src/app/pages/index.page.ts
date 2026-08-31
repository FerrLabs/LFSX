import { ChangeDetectionStrategy, Component } from '@angular/core';
import { SiteFrameComponent } from '../chrome/site-frame.component';
import { LandingComponent } from '../components/landing.component';

@Component({
  selector: 'flr-home',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SiteFrameComponent, LandingComponent],
  template: `
    <flr-site-frame [title]="title" [description]="description">
      <flr-landing />
    </flr-site-frame>
  `,
})
export default class HomePage {
  protected readonly title = $localize`:@@meta.title:LFSX: a fast, lightweight, secure Git LFS server`;
  protected readonly description = $localize`:@@meta.description:Self-hosted Git LFS for the large binaries a repository cannot keep. Streams end to end, stores each object once, and takes its permissions from your forge. One Rust binary, no database.`;
}
