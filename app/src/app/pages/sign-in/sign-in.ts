import { HttpErrorResponse } from '@angular/common/http';
import { Component, computed, inject, input, signal } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import {
  BannerComponent,
  ButtonComponent,
  FieldComponent,
  InputComponent,
} from '@ferrlabs/ui-ng';

import { Logo } from '../../logo/logo';
import { Session } from '../../session';

@Component({
  selector: 'lfsx-sign-in',
  imports: [
    Logo,
    FormsModule,
    BannerComponent,
    ButtonComponent,
    FieldComponent,
    InputComponent,
  ],
  templateUrl: './sign-in.html',
  styleUrl: './sign-in.css',
})
export class SignIn {
  private readonly session = inject(Session);
  private readonly router = inject(Router);

  readonly refused = input<string | null>(null);

  protected readonly token = signal('');
  protected readonly pending = signal(false);
  protected readonly answer = signal<string | null>(null);
  protected readonly problem = computed(() => this.answer() ?? this.refused());

  protected submit(): void {
    const token = this.token().trim();
    if (!token) {
      return;
    }
    this.pending.set(true);
    this.answer.set(null);
    this.session.signIn(token).subscribe({
      next: () => {
        this.pending.set(false);
        void this.router.navigate(['overview']);
      },
      error: (error: unknown) => {
        this.pending.set(false);
        this.answer.set(
          error instanceof HttpErrorResponse && error.status === 403
            ? 'forbidden'
            : error instanceof HttpErrorResponse && error.status === 401
              ? 'rejected'
              : 'unreachable',
        );
      },
    });
  }
}
