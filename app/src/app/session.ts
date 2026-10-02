import { HttpInterceptorFn } from '@angular/common/http';
import { Injectable, signal } from '@angular/core';

const KEY = 'lfsx.dashboard.token';

function stored(): string | null {
  try {
    return sessionStorage.getItem(KEY);
  } catch {
    return null;
  }
}

@Injectable({ providedIn: 'root' })
export class Session {
  readonly token = signal<string | null>(stored());

  signIn(token: string): void {
    this.token.set(token);
    try {
      sessionStorage.setItem(KEY, token);
    } catch {
      return;
    }
  }

  signOut(): void {
    this.token.set(null);
    try {
      sessionStorage.removeItem(KEY);
    } catch {
      return;
    }
  }
}

export const bearer: HttpInterceptorFn = (request, next) => {
  const token = stored();
  if (!token || !request.url.startsWith('/-/api/')) {
    return next(request);
  }
  return next(request.clone({ setHeaders: { Authorization: `Bearer ${token}` } }));
};
