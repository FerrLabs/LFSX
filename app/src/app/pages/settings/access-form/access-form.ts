import { HttpErrorResponse } from '@angular/common/http';
import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import {
  BannerComponent,
  ButtonComponent,
  CardComponent,
  FieldComponent,
  LoadingStateComponent,
  ModalComponent,
  SwitchComponent,
  TextareaComponent,
} from '@ferrlabs/ui-ng';

import { Access, AccessChange, Api } from '../../../api';
import { Feed } from '../../../feed';

function lines(text: string): string[] {
  return text
    .split(/[\n,]/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

@Component({
  selector: 'lfsx-access-form',
  imports: [
    FormsModule,
    BannerComponent,
    ButtonComponent,
    CardComponent,
    FieldComponent,
    LoadingStateComponent,
    ModalComponent,
    SwitchComponent,
    TextareaComponent,
  ],
  templateUrl: './access-form.html',
  styleUrl: './access-form.css',
})
export class AccessForm implements OnInit {
  private readonly api = inject(Api);
  private readonly feed = inject(Feed);

  protected readonly current = signal<Access | null>(null);
  protected readonly limited = signal(false);
  protected readonly allowed = signal('');
  protected readonly restricted = signal('');
  protected readonly anonymous = signal(false);

  protected readonly saving = signal(false);
  protected readonly confirming = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly saved = signal(false);

  protected readonly change = computed<AccessChange>(() => ({
    anonymous_read: this.anonymous(),
    restricted: lines(this.restricted()),
    allowed: this.limited() ? lines(this.allowed()) : null,
  }));

  protected readonly dirty = computed(() => {
    const current = this.current();
    if (!current) {
      return false;
    }
    const change = this.change();
    return (
      change.anonymous_read !== current.anonymous_read ||
      change.restricted.join('\n') !== current.restricted.join('\n') ||
      (change.allowed === null) !== (current.allowed === null) ||
      (change.allowed ?? []).join('\n') !== (current.allowed ?? []).join('\n')
    );
  });

  ngOnInit(): void {
    this.api.access().subscribe({
      next: (access) => this.load(access),
      error: (error: unknown) => this.fail(error),
    });
  }

  protected save(): void {
    this.saving.set(true);
    this.problem.set(null);
    this.saved.set(false);
    this.api.saveAccess(this.change()).subscribe({
      next: (access) => {
        this.load(access);
        this.saved.set(true);
      },
      error: (error: unknown) => this.fail(error),
    });
  }

  protected discard(): void {
    const current = this.current();
    if (current) {
      this.load(current);
    }
  }

  protected reset(): void {
    this.confirming.set(false);
    this.saving.set(true);
    this.problem.set(null);
    this.api.resetAccess().subscribe({
      next: (access) => {
        this.load(access);
        this.saved.set(true);
      },
      error: (error: unknown) => this.fail(error),
    });
  }

  private load(access: Access): void {
    this.current.set(access);
    this.limited.set(access.allowed !== null);
    this.allowed.set((access.allowed ?? []).join('\n'));
    this.restricted.set(access.restricted.join('\n'));
    this.anonymous.set(access.anonymous_read);
    this.saving.set(false);
  }

  private fail(error: unknown): void {
    this.saving.set(false);
    if (error instanceof HttpErrorResponse && error.status === 422) {
      const unreadable = (error.error?.unreadable ?? []) as string[];
      this.problem.set(`Not org/repo, org/prefix-* or org/*: ${unreadable.join(', ')}`);
      return;
    }
    if (error instanceof HttpErrorResponse && (error.status === 401 || error.status === 403)) {
      this.feed.start();
      this.problem.set('Your session no longer has admin rights. Sign in again.');
      return;
    }
    this.problem.set('The server did not save the change. Try again.');
  }
}
