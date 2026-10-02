import { Routes } from '@angular/router';

export const routes: Routes = [
  { path: '', pathMatch: 'full', redirectTo: 'overview' },
  {
    path: 'overview',
    title: 'Overview | LFSX',
    loadComponent: () => import('./pages/overview/overview').then((page) => page.Overview),
  },
  {
    path: 'settings',
    title: 'Settings | LFSX',
    loadComponent: () => import('./pages/settings/settings').then((page) => page.Settings),
  },
  {
    path: 'sign-in',
    title: 'Sign in | LFSX',
    loadComponent: () => import('./pages/sign-in/sign-in').then((page) => page.SignIn),
  },
  { path: '**', redirectTo: 'overview' },
];
