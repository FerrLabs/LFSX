import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

export interface Overview {
  readonly version: string;
  readonly uptime_seconds: number;
  readonly storage: {
    readonly kind: 'local' | 's3' | 'azure' | 'gcs';
    readonly objects: number | null;
    readonly bytes: number | null;
  };
  readonly traffic: {
    readonly requests: number;
    readonly server_errors: number;
    readonly rejections: number;
    readonly uploaded_bytes: number;
    readonly downloaded_bytes: number;
    readonly transfers_in_flight: number;
  };
  readonly object_sizes: readonly SizeBucket[];
  readonly cache: { readonly hits: number; readonly misses: number; readonly bytes: number } | null;
  readonly settings: Settings;
}

export interface SizeBucket {
  readonly up_to: number | null;
  readonly objects: number;
}

export interface Settings {
  readonly auth: 'github' | 'gitlab' | 'gitea' | 'disabled';
  readonly allowed: readonly string[] | null;
  readonly restricted: readonly string[];
  readonly anonymous_read: boolean;
  readonly max_object_size: number | null;
  readonly repo_quota: number | null;
  readonly max_concurrent_transfers: number;
  readonly compression: boolean;
  readonly encryption: boolean;
  readonly presign: boolean;
  readonly locking: boolean;
  readonly gc_grace_seconds: number;
  readonly lock_max_age_seconds: number | null;
  readonly forges: readonly NamedForge[];
}

export interface NamedForge {
  readonly name: string;
  readonly auth: 'github' | 'gitlab' | 'gitea';
  readonly api_url: string;
  readonly allowed: readonly string[] | null;
}

export interface Access {
  readonly editable: boolean;
  readonly source: 'environment' | 'dashboard';
  readonly anonymous_read: boolean;
  readonly restricted: readonly string[];
  readonly allowed: readonly string[] | null;
}

export interface AccessChange {
  readonly anonymous_read: boolean;
  readonly restricted: readonly string[];
  readonly allowed: readonly string[] | null;
}

function forgeParams(forge: string | null): Record<string, string> {
  return forge === null ? {} : { forge };
}

@Injectable({ providedIn: 'root' })
export class Api {
  private readonly http = inject(HttpClient);

  overview(): Observable<Overview> {
    return this.http.get<Overview>('/-/api/overview');
  }

  access(forge: string | null): Observable<Access> {
    return this.http.get<Access>('/-/api/access', { params: forgeParams(forge) });
  }

  saveAccess(change: AccessChange, forge: string | null): Observable<Access> {
    return this.http.put<Access>('/-/api/access', change, { params: forgeParams(forge) });
  }

  resetAccess(forge: string | null): Observable<Access> {
    return this.http.delete<Access>('/-/api/access', { params: forgeParams(forge) });
  }
}
