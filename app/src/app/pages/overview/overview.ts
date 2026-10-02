import { Component, OnDestroy, OnInit, computed, inject } from '@angular/core';
import {
  BannerComponent,
  BarChartComponent,
  CardComponent,
  ErrorStateComponent,
  LoadingStateComponent,
} from '@ferrlabs/ui-ng';

import { SizeBucket } from '../../api';
import { Feed } from '../../feed';
import { bytes, count, duration } from '../../format';
import { Theme } from '../../theme';

interface Tile {
  readonly label: string;
  readonly value: string;
  readonly tone: 'plain' | 'good' | 'bad';
}

const SHORT = ['', 'K', 'M', 'G', 'T'];

function short(value: number): string {
  let scaled = value;
  let unit = 0;
  while (scaled >= 1024 && unit < SHORT.length - 1) {
    scaled /= 1024;
    unit += 1;
  }
  return `${Math.round(scaled)}${SHORT[unit]}`;
}

function bucketLabel(bucket: SizeBucket, previous: SizeBucket | undefined): string {
  if (bucket.up_to === null) {
    return `>${short(previous?.up_to ?? 0)}`;
  }
  return `≤${short(bucket.up_to)}`;
}

@Component({
  selector: 'lfsx-overview',
  imports: [BannerComponent, BarChartComponent, CardComponent, ErrorStateComponent, LoadingStateComponent],
  templateUrl: './overview.html',
  styleUrl: './overview.css',
})
export class Overview implements OnInit, OnDestroy {
  protected readonly feed = inject(Feed);
  private readonly theme = inject(Theme);
  protected readonly chartColor = computed(() => (this.theme.mode() === 'dark' ? '#5cc79f' : '#1d6b52'));
  protected readonly formatCount = (value: number) => count(value);

  protected readonly view = computed(() => {
    const state = this.feed.state();
    if (state.kind !== 'ready') {
      return null;
    }
    const { overview } = state;
    const { traffic, cache, storage, settings } = overview;
    const lookups = cache ? cache.hits + cache.misses : 0;

    const side: readonly Tile[] = [
      { label: 'Objects', value: count(storage.objects), tone: 'plain' },
      { label: 'Uploaded', value: bytes(traffic.uploaded_bytes), tone: 'plain' },
      { label: 'Downloaded', value: bytes(traffic.downloaded_bytes), tone: 'plain' },
      {
        label: 'Server errors',
        value: count(traffic.server_errors),
        tone: traffic.server_errors > 0 ? 'bad' : 'good',
      },
    ];

    const lower: readonly Tile[] = [
      { label: 'Requests', value: count(traffic.requests), tone: 'plain' },
      { label: 'Refused requests', value: count(traffic.rejections), tone: 'plain' },
      { label: 'Transfers in flight', value: count(traffic.transfers_in_flight), tone: 'plain' },
      {
        label: 'Cache hit rate',
        value: cache && lookups > 0 ? `${Math.round((cache.hits / lookups) * 100)} %` : 'no cache',
        tone: 'plain',
      },
    ];

    return {
      stored: bytes(storage.bytes),
      kind: storage.kind,
      measured: storage.objects !== null,
      sizes: overview.object_sizes.map((bucket, index, all) => ({
        label: bucketLabel(bucket, all[index - 1]),
        value: bucket.objects,
      })),
      uploads: overview.object_sizes.reduce((total, bucket) => total + bucket.objects, 0),
      side,
      lower,
      refreshed: state.at.toLocaleTimeString(),
      running: `Version ${overview.version}, up ${duration(overview.uptime_seconds)}`,
      authDisabled: settings.auth === 'disabled',
      openToTheForge: settings.auth !== 'disabled' && settings.allowed === null,
    };
  });

  ngOnInit(): void {
    this.feed.start();
  }

  ngOnDestroy(): void {
    this.feed.stop();
  }
}
