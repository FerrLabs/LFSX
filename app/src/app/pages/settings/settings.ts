import { Component, OnDestroy, OnInit, computed, inject } from '@angular/core';
import { AccessForm } from './access-form/access-form';
import {
  ErrorStateComponent,
  LoadingStateComponent,
  TagComponent,
} from '@ferrlabs/ui-ng';

import { Settings as Configured } from '../../api';
import { Feed } from '../../feed';
import { bytes, duration } from '../../format';

interface Row {
  readonly name: string;
  readonly variable: string;
  readonly value: string;
  readonly on: boolean | null;
}

function rows(settings: Configured): readonly Row[] {
  return [
    { name: 'Forge', variable: 'LFSX_AUTH', value: settings.auth, on: null },
    {
      name: 'Largest object',
      variable: 'LFSX_MAX_OBJECT_SIZE',
      value: settings.max_object_size === null ? 'no limit' : bytes(settings.max_object_size),
      on: null,
    },
    {
      name: 'Quota per repository',
      variable: 'LFSX_REPO_QUOTA',
      value: settings.repo_quota === null ? 'no quota' : bytes(settings.repo_quota),
      on: null,
    },
    {
      name: 'Concurrent transfers',
      variable: 'LFSX_MAX_CONCURRENT_TRANSFERS',
      value:
        settings.max_concurrent_transfers === 0 ? 'no cap' : String(settings.max_concurrent_transfers),
      on: null,
    },
    { name: 'Compression', variable: 'LFSX_COMPRESSION', value: '', on: settings.compression },
    { name: 'Encryption at rest', variable: 'LFSX_ENCRYPTION_KEY_FILE', value: '', on: settings.encryption },
    { name: 'Redirected transfers', variable: 'LFSX_S3_PRESIGN', value: '', on: settings.presign },
    { name: 'Locking', variable: 'probed at boot', value: '', on: settings.locking },
    {
      name: 'Collection grace',
      variable: 'LFSX_GC_GRACE',
      value: duration(settings.gc_grace_seconds),
      on: null,
    },
    {
      name: 'Lock expiry',
      variable: 'LFSX_LOCK_MAX_AGE',
      value: settings.lock_max_age_seconds === null ? 'never' : duration(settings.lock_max_age_seconds),
      on: null,
    },
  ];
}

@Component({
  selector: 'lfsx-settings',
  imports: [AccessForm, ErrorStateComponent, LoadingStateComponent, TagComponent],
  templateUrl: './settings.html',
  styleUrl: './settings.css',
})
export class Settings implements OnInit, OnDestroy {
  protected readonly feed = inject(Feed);

  protected readonly rows = computed(() => {
    const state = this.feed.state();
    return state.kind === 'ready' ? rows(state.overview.settings) : null;
  });

  ngOnInit(): void {
    this.feed.start();
  }

  ngOnDestroy(): void {
    this.feed.stop();
  }
}
