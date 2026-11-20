import { ChangeDetectionStrategy, Component, computed, signal } from '@angular/core';
import { CodeBlockComponent } from './code-block.component';
import { SegmentedComponent } from './segmented.component';

type InstallKey = 'docker' | 'kubernetes' | 'binary' | 'cargo';

interface Install {
  readonly key: InstallKey;
  readonly label: string;
  readonly code: string;
  readonly note: string;
}

@Component({
  selector: 'flr-install-tabs',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CodeBlockComponent, SegmentedComponent],
  templateUrl: './install-tabs.component.html',
  styleUrl: './install-tabs.component.css',
})
export class InstallTabsComponent {
  protected readonly installs: readonly Install[] = [
    {
      key: 'docker',
      label: 'Docker',
      code: [
        'docker run -d --name lfsx \\',
        '  -p 8080:8080 \\',
        '  -v lfsx-data:/var/lib/lfsx \\',
        '  -e LFSX_PUBLIC_URL=https://lfs.example.com \\',
        '  ghcr.io/ferrlabs/lfsx:latest',
      ].join('\n'),
      note: $localize`:@@landing.install.docker.note:The volume holds every object. LFSX_PUBLIC_URL is the address clients reconnect to for each transfer.`,
    },
    {
      key: 'kubernetes',
      label: 'Kubernetes',
      code: [
        'helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx \\',
        '  --set ingress.host=lfs.example.com',
      ].join('\n'),
      note: $localize`:@@landing.deploy.kubernetes.note:The chart encodes what is easy to get wrong: the public URL derived from the ingress host, the nginx annotations that keep large uploads from being rejected, probes on /health and /ready, and a refusal to render more than one replica unless the objects are in a bucket.`,
    },
    {
      key: 'binary',
      label: 'Binary',
      code: [
        'curl -fsSL .../lfsx-server-x86_64-unknown-linux-musl.tar.gz | tar xz',
        './lfsx-server',
      ].join('\n'),
      note: $localize`:@@landing.deploy.binary.note:x86_64 or aarch64, musl or gnu. Every archive ships a .sha256 next to it.`,
    },
    {
      key: 'cargo',
      label: 'Cargo',
      code: 'cargo install lfsx-server',
      note: $localize`:@@landing.install.cargo.note:If you would rather compile it yourself.`,
    },
  ];

  protected readonly tabs = this.installs.map(({ key, label }) => ({ key, label }));
  protected readonly selected = signal<InstallKey>('docker');
  protected readonly current = computed(
    () => this.installs.find((install) => install.key === this.selected()) ?? this.installs[0],
  );
  protected readonly tabsLabel = $localize`:@@landing.install.label:Install method`;
}
