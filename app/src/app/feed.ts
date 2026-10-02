import { HttpErrorResponse } from '@angular/common/http';
import { Injectable, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { Subscription, catchError, of, switchMap, timer } from 'rxjs';

import { Api, Overview } from './api';

export type FeedState =
  | { readonly kind: 'loading' }
  | { readonly kind: 'ready'; readonly overview: Overview; readonly at: Date }
  | { readonly kind: 'failed'; readonly message: string };

const EVERY = 10_000;

@Injectable({ providedIn: 'root' })
export class Feed {
  private readonly api = inject(Api);
  private readonly router = inject(Router);
  private subscription: Subscription | null = null;

  readonly state = signal<FeedState>({ kind: 'loading' });

  start(): void {
    if (this.subscription) {
      return;
    }
    this.subscription = timer(0, EVERY)
      .pipe(
        switchMap(() =>
          this.api.overview().pipe(
            catchError((error: unknown) => {
              this.refused(error);
              return of(null);
            }),
          ),
        ),
      )
      .subscribe((overview) => {
        if (overview) {
          this.state.set({ kind: 'ready', overview, at: new Date() });
        }
      });
  }

  stop(): void {
    this.subscription?.unsubscribe();
    this.subscription = null;
  }

  private refused(error: unknown): void {
    if (error instanceof HttpErrorResponse && (error.status === 401 || error.status === 403)) {
      this.stop();
      void this.router.navigate(['sign-in'], {
        queryParams: { refused: error.status === 403 ? 'forbidden' : null },
      });
      return;
    }
    const message =
      error instanceof HttpErrorResponse && error.status === 0
        ? 'The server could not be reached.'
        : 'The server answered with an error.';
    if (this.state().kind !== 'ready') {
      this.state.set({ kind: 'failed', message });
    }
  }
}
