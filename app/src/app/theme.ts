import { Injectable, effect, signal } from '@angular/core';

export type Mode = 'light' | 'dark';

const KEY = 'lfsx.dashboard.theme';

function initial(): Mode {
  try {
    const saved = localStorage.getItem(KEY);
    if (saved === 'light' || saved === 'dark') {
      return saved;
    }
  } catch {
    return 'light';
  }
  return matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

@Injectable({ providedIn: 'root' })
export class Theme {
  readonly mode = signal<Mode>(initial());

  constructor() {
    effect(() => {
      const mode = this.mode();
      document.documentElement.dataset['theme'] = mode;
      try {
        localStorage.setItem(KEY, mode);
      } catch {
        return;
      }
    });
  }

  toggle(): void {
    this.mode.update((mode) => (mode === 'dark' ? 'light' : 'dark'));
  }
}
