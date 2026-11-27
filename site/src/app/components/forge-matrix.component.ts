import { ChangeDetectionStrategy, Component, computed, signal } from '@angular/core';
import { SegmentedComponent } from '@ferrlabs/ui-ng';

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
  protected readonly forges: readonly { readonly value: ForgeKey; readonly label: string }[] = [
    { value: 'github', label: 'GitHub' },
    { value: 'gitlab', label: 'GitLab' },
    { value: 'gitea', label: 'Gitea, Forgejo' },
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

  protected select(value: string): void {
    const match = this.forges.find((forge) => forge.value === value);
    if (match) this.selected.set(match.value);
  }
}
