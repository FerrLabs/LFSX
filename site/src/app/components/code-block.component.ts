import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  booleanAttribute,
  inject,
  input,
  linkedSignal,
} from '@angular/core';

const FEEDBACK_MS = 2000;

@Component({
  selector: 'flr-code-block',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './code-block.component.html',
  styleUrl: './code-block.component.css',
})
export class CodeBlockComponent {
  readonly code = input.required<string>();
  readonly copyable = input(false, { transform: booleanAttribute });
  readonly compact = input(false, { transform: booleanAttribute });

  protected readonly copied = linkedSignal({ source: this.code, computation: () => false });

  private timer: ReturnType<typeof setTimeout> | undefined;

  constructor() {
    inject(DestroyRef).onDestroy(() => clearTimeout(this.timer));
  }

  protected async copy(): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.code());
    } catch {
      return;
    }
    this.copied.set(true);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.copied.set(false), FEEDBACK_MS);
  }
}
