import {
  ChangeDetectionStrategy,
  Component,
  booleanAttribute,
  effect,
  inject,
  input,
} from '@angular/core';
import { Meta, Title } from '@angular/platform-browser';
import { SiteHeaderComponent } from './site-header.component';

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

  private readonly titleService = inject(Title);
  private readonly metaService = inject(Meta);

  constructor() {
    effect(() => {
      this.titleService.setTitle(this.title());
      const description = this.description();
      if (description) {
        this.metaService.updateTag({ name: 'description', content: description });
      }
    });
  }
}
