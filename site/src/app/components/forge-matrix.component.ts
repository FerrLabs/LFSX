import { ChangeDetectionStrategy, Component, computed, signal } from '@angular/core';
import { SegmentedComponent } from './segmented.component';

type ForgeKey = 'github' | 'gitlab' | 'gitea';

interface Grant {
  readonly key: string;
  readonly roles: Readonly<Record<ForgeKey, string>>;
  readonly objects: string;
  readonly rejected: boolean;
}

@Component({
  selector: 'flr-forge-matrix',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [SegmentedComponent],
  templateUrl: './forge-matrix.component.html',
  styleUrl: './forge-matrix.component.css',
})
export class ForgeMatrixComponent {
  protected readonly forges: readonly { readonly key: ForgeKey; readonly label: string }[] = [
    { key: 'github', label: 'GitHub' },
    { key: 'gitlab', label: 'GitLab' },
    { key: 'gitea', label: 'Gitea, Forgejo' },
  ];

  protected readonly grants: readonly Grant[] = [
    {
      key: 'admin',
      roles: { github: 'admin', gitlab: 'Maintainer, Owner', gitea: 'admin' },
      objects: $localize`:@@landing.auth.admin:download, upload, and force a lock open`,
      rejected: false,
    },
    {
      key: 'push',
      roles: { github: 'push', gitlab: 'Developer', gitea: 'push' },
      objects: $localize`:@@landing.auth.push:download, upload, and take locks`,
      rejected: false,
    },
    {
      key: 'pull',
      roles: { github: 'pull only', gitlab: 'Reporter', gitea: 'pull only' },
      objects: $localize`:@@landing.auth.pull:download`,
      rejected: false,
    },
    {
      key: 'none',
      roles: { github: 'none', gitlab: 'Guest, or none', gitea: 'none' },
      objects: $localize`:@@landing.auth.none:rejected`,
      rejected: true,
    },
  ];

  protected readonly selected = signal<ForgeKey>('github');
  protected readonly rows = computed(() =>
    this.grants.map((grant) => ({ ...grant, role: grant.roles[this.selected()] })),
  );
  protected readonly tabsLabel = $localize`:@@landing.auth.forge:Forge`;
}
