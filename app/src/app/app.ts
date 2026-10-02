import { Component, computed, inject, signal } from '@angular/core';
import { toSignal } from '@angular/core/rxjs-interop';
import { NavigationEnd, Router, RouterOutlet } from '@angular/router';
import {
  RailToggleComponent,
  SidebarComponent,
  SidebarItemComponent,
  SidebarSectionComponent,
} from '@ferrlabs/ui-ng';
import { filter, map } from 'rxjs';

import { Logo } from './logo/logo';
import { Session } from './session';
import { Theme } from './theme';

const COLLAPSED = 'lfsx.dashboard.collapsed';

function wasCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED) === 'true';
  } catch {
    return false;
  }
}

@Component({
  selector: 'lfsx-root',
  imports: [
    RouterOutlet,
    Logo,
    RailToggleComponent,
    SidebarComponent,
    SidebarItemComponent,
    SidebarSectionComponent,
  ],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App {
  private readonly router = inject(Router);
  private readonly session = inject(Session);
  protected readonly theme = inject(Theme);

  protected readonly collapsed = signal(wasCollapsed());

  private readonly url = toSignal(
    this.router.events.pipe(
      filter((event): event is NavigationEnd => event instanceof NavigationEnd),
      map((event) => event.urlAfterRedirects),
    ),
    { initialValue: this.router.url },
  );

  protected readonly current = computed(() => this.url().split(/[/?#]/)[1] ?? '');

  protected collapse(collapsed: boolean): void {
    this.collapsed.set(collapsed);
    try {
      localStorage.setItem(COLLAPSED, String(collapsed));
    } catch {
      return;
    }
  }

  protected go(path: string): void {
    void this.router.navigate([path]);
  }

  protected signOut(): void {
    this.session.signOut().subscribe({
      complete: () => void this.router.navigate(['sign-in']),
      error: () => void this.router.navigate(['sign-in']),
    });
  }
}
