import { ChangeDetectionStrategy, Component } from '@angular/core';
import { CodeBlockComponent } from './code-block.component';
import { ForgeMatrixComponent } from './forge-matrix.component';
import { InstallTabsComponent } from './install-tabs.component';
import { StorageCardsComponent } from './storage-cards.component';

interface Comparison {
  readonly key: string;
  readonly metered: string;
  readonly owned: string;
}

interface Question {
  readonly key: string;
  readonly asked: string;
  readonly answered: string;
}

@Component({
  selector: 'flr-landing',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CodeBlockComponent, ForgeMatrixComponent, InstallTabsComponent, StorageCardsComponent],
  templateUrl: './landing.component.html',
  styleUrl: './landing.component.css',
})
export class LandingComponent {
  protected readonly heroCommand = 'docker run -p 8080:8080 ghcr.io/ferrlabs/lfsx:latest';

  protected readonly lfsconfig = [
    '# .lfsconfig',
    '[lfs]',
    '\turl = https://lfs.example.com/my-org/my-project',
  ].join('\n');

  protected readonly gitUsage = [
    'git lfs install',
    'git lfs track "*.psd"',
    'git add .gitattributes assets/hero.psd',
    'git commit -m "add hero artwork"',
    'git push',
  ].join('\n');

  protected readonly doctor = [
    'npm install -g @ferrlabs/lfsx',
    'lfsx --url https://lfs.example.com \\',
    '  doctor --repo my-org/my-project',
  ].join('\n');

  protected readonly ciSnippet = [
    '- run: |',
    '    printf \'protocol=https\\nhost=lfs.example.com\\nusername=git\\npassword=%s\\n\' "${{ secrets.GITHUB_TOKEN }}" \\',
    '      | git credential approve',
    '    git lfs pull',
  ].join('\n');

  protected readonly comparisons: readonly Comparison[] = [
    {
      key: 'meter',
      metered: $localize`:@@landing.compare.meter.metered:Storage and bandwidth metered, billed in packs`,
      owned: $localize`:@@landing.compare.meter.owned:A disk you already own, with no meter on it`,
    },
    {
      key: 'vendor',
      metered: $localize`:@@landing.compare.vendor.metered:Objects sit with the same vendor as the repository`,
      owned: $localize`:@@landing.compare.vendor.owned:The repository is unchanged, and only the transfer is redirected`,
    },
    {
      key: 'access',
      metered: $localize`:@@landing.compare.access.metered:Access control is the forge's, and so is the bill`,
      owned: $localize`:@@landing.compare.access.owned:Still the forge's permissions, asked live, with no second user list`,
    },
    {
      key: 'ci',
      metered: $localize`:@@landing.compare.ci.metered:A CI job pulling the same pack pays for it every time`,
      owned: $localize`:@@landing.compare.ci.owned:One committed .lfsconfig, and clients need no plugin`,
    },
  ];

  protected readonly questions: readonly Question[] = [
    {
      key: 'plugin',
      asked: $localize`:@@landing.faq.plugin.asked:Do my clients need a plugin?`,
      answered: $localize`:@@landing.faq.plugin.answered:No. Git LFS 3.0.2 and later are exercised on Linux, macOS and Windows, including the copies bundled by Git for Windows, GitHub Desktop, Sourcetree, Rider and Unity.`,
    },
    {
      key: 'proxy',
      asked: $localize`:@@landing.faq.proxy.asked:Can authentication live in the proxy?`,
      answered: $localize`:@@landing.faq.proxy.answered:No, and this rules out the approach most people reach for first. Behind an authenticating proxy, git-lfs calls the transfer URLs with no credentials at all and retries in a loop. LFSX never claims a transfer through itself is pre-authenticated, so the client authenticates each one.`,
    },
    {
      key: 'forges',
      asked: $localize`:@@landing.faq.forges.asked:Which forges are supported?`,
      answered: $localize`:@@landing.faq.forges.answered:GitHub, GitLab, and Gitea together with Forgejo, including Enterprise and self-managed instances through their API URL. GitLab grants inherited from a group count the same as ones set on the project.`,
    },
    {
      key: 'locks',
      asked: $localize`:@@landing.faq.locks.asked:Does it handle file locks?`,
      answered: $localize`:@@landing.faq.locks.answered:Yes, for the assets that cannot be merged, with force-open reserved for the levels a forge treats as administrative: admin on GitHub and Gitea, Maintainer or Owner on GitLab. In a bucket the server checks at startup that the store really refuses a conditional write, and gives locking up rather than hand the same lock to two people.`,
    },
    {
      key: 'contribute',
      asked: $localize`:@@landing.faq.contribute.asked:Can I contribute a forge?`,
      answered: $localize`:@@landing.faq.contribute.answered:Three providers exist, so the shape is settled rather than guessed: one module under server/src/auth/ exposing three functions, plus a config variant and three arms in auth.rs. The caching, the challenge handling and the rejection accounting are shared and provider-blind.`,
    },
  ];
}
