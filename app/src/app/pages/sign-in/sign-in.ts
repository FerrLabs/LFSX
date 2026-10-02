import { Component, inject, input, signal } from '@angular/core';
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

  protected submit(): void {
    const token = this.token().trim();
    if (!token) {
      return;
    }
    this.session.signIn(token);
    void this.router.navigate(['overview']);
  }
}
