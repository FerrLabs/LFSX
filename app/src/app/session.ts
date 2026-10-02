import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

export interface Viewer {
  readonly via: 'token' | 'forge';
  readonly name: string;
}

@Injectable({ providedIn: 'root' })
export class Session {
  private readonly http = inject(HttpClient);

  signIn(token: string): Observable<Viewer> {
    return this.http.post<Viewer>('/-/api/session', { token });
  }

  signOut(): Observable<void> {
    return this.http.delete<void>('/-/api/session');
  }
}
