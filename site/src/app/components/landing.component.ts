import { ChangeDetectionStrategy, Component } from '@angular/core';

interface Property {
  readonly key: string;
  readonly title: string;
  readonly body: string;
}

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

interface Step {
  readonly key: string;
  readonly title: string;
  readonly body?: string;
  readonly code: string;
  readonly note?: string;
}

interface Setting {
  readonly key: string;
  readonly name: string;
  readonly purpose: string;
}

interface Install {
  readonly key: string;
  readonly title: string;
  readonly code: string;
  readonly note?: string;
}

interface Store {
  readonly key: string;
  readonly title: string;
  readonly flag: string;
  readonly body: string;
}

@Component({
  selector: 'flr-landing',
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: './landing.component.html',
  styleUrl: './landing.component.css',
})
export class LandingComponent {
  protected readonly run = [
    'docker run -d --name lfsx \\',
    '  -p 8080:8080 \\',
    '  -v lfsx-data:/var/lib/lfsx \\',
    '  ghcr.io/ferrlabs/lfsx:latest',
  ].join('\n');

  protected readonly properties: readonly Property[] = [
    {
      key: 'fast',
      title: $localize`:@@landing.fast.title:Fast`,
      body: $localize`:@@landing.fast.body:Uploads and downloads stream end to end. A multi-gigabyte asset costs the same resident memory as a one-kilobyte icon, and the digest is computed on the bytes as they pass.`,
    },
    {
      key: 'light',
      title: $localize`:@@landing.light.title:Lightweight`,
      body: $localize`:@@landing.light.body:One statically linked binary, a distroless image, no database. Objects live on the filesystem addressed by digest, or in an S3-compatible bucket.`,
    },
    {
      key: 'secure',
      title: $localize`:@@landing.secure.title:Secure`,
      body: $localize`:@@landing.secure.body:Access mirrors the upstream repository, so revoking someone there revokes them here. Every object is verified against its declared digest before it is accepted.`,
    },
  ];

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

  protected readonly steps: readonly Step[] = [
    {
      key: 'run',
      title: $localize`:@@landing.quickstart.run.title:Run it`,
      body: $localize`:@@landing.quickstart.run.body:A container, a statically linked binary from the releases, or cargo install lfsx-server if you would rather compile.`,
      code: [
        'docker run -d --name lfsx \\',
        '  -p 8080:8080 \\',
        '  -v lfsx-data:/var/lib/lfsx \\',
        '  -e LFSX_PUBLIC_URL=https://lfs.example.com \\',
        '  ghcr.io/ferrlabs/lfsx:latest',
      ].join('\n'),
    },
    {
      key: 'point',
      title: $localize`:@@landing.quickstart.point.title:Point a repository at it`,
      body: $localize`:@@landing.quickstart.point.body:The last two path segments are the organisation and the project, and together they scope the storage. Two repositories sharing a URL share their objects.`,
      code: ['# .lfsconfig', '[lfs]', '\turl = https://lfs.example.com/my-org/my-project'].join(
        '\n',
      ),
    },
    {
      key: 'use',
      title: $localize`:@@landing.quickstart.use.title:Then use Git LFS as usual`,
      code: [
        'git lfs install',
        'git lfs track "*.psd"',
        'git add .gitattributes assets/hero.psd',
        'git commit -m "add hero artwork"',
        'git push',
      ].join('\n'),
      note: $localize`:@@landing.quickstart.use.note:Run git lfs install before cloning. Without it, files arrive as 130-byte pointer stubs, and the tools that read them, Unity, Unreal and image editors, fail in confusing ways.`,
    },
    {
      key: 'verify',
      title: $localize`:@@landing.quickstart.verify.title:Verify it works`,
      body: $localize`:@@landing.quickstart.verify.body:The doctor checks the server is up, its storage is writable, your token is accepted, and that the URL it advertises for transfers is the one you reached it on, which is the mismatch that lets negotiation succeed while every transfer fails.`,
      code: [
        'npm install -g @ferrlabs/lfsx        # or: cargo install lfsx',
        'lfsx --url https://lfs.example.com doctor --repo my-org/my-project',
      ].join('\n'),
    },
  ];

  protected readonly settings: readonly Setting[] = [
    {
      key: 'bind',
      name: 'LFSX_BIND',
      purpose: $localize`:@@landing.config.bind:listen address, 0.0.0.0:8080`,
    },
    {
      key: 'root',
      name: 'LFSX_STORAGE_ROOT',
      purpose: $localize`:@@landing.config.root:root of the object store, /var/lib/lfsx`,
    },
    {
      key: 'public',
      name: 'LFSX_PUBLIC_URL',
      purpose: $localize`:@@landing.config.public:public URL used to build transfer links`,
    },
    {
      key: 'auth',
      name: 'LFSX_AUTH',
      purpose: $localize`:@@landing.config.auth:permission source, github or gitlab or gitea, or disabled`,
    },
    {
      key: 'storage',
      name: 'LFSX_STORAGE',
      purpose: $localize`:@@landing.config.storage:s3 to keep objects in a bucket instead of on the volume`,
    },
    {
      key: 'compression',
      name: 'LFSX_COMPRESSION',
      purpose: $localize`:@@landing.config.compression:zstd:1 to zstd:19, to compress objects at rest`,
    },
    {
      key: 'encryption',
      name: 'LFSX_ENCRYPTION_KEY_FILE',
      purpose: $localize`:@@landing.config.encryption:32-byte keys as hex, to encrypt objects at rest`,
    },
    {
      key: 'quota',
      name: 'LFSX_REPO_QUOTA',
      purpose: $localize`:@@landing.config.quota:bytes a single repository may hold, unlimited`,
    },
  ];

  protected readonly installs: readonly Install[] = [
    {
      key: 'docker',
      title: 'Docker',
      code: 'docker run -d -p 8080:8080 -v lfsx-data:/var/lib/lfsx ghcr.io/ferrlabs/lfsx:latest',
    },
    {
      key: 'kubernetes',
      title: 'Kubernetes',
      code: 'helm install lfsx oci://ghcr.io/ferrlabs/charts/lfsx --set ingress.host=lfs.example.com',
      note: $localize`:@@landing.deploy.kubernetes.note:The chart encodes what is easy to get wrong: the public URL derived from the ingress host, the nginx annotations that keep large uploads from being rejected, probes on /health and /ready, and a refusal to render more than one replica unless the objects are in a bucket.`,
    },
    {
      key: 'binary',
      title: 'Binary',
      code: 'curl -fsSL .../lfsx-server-x86_64-unknown-linux-musl.tar.gz | tar xz && ./lfsx-server',
      note: $localize`:@@landing.deploy.binary.note:x86_64 or aarch64, musl or gnu. Every archive ships a .sha256 next to it.`,
    },
    {
      key: 'cargo',
      title: 'Cargo',
      code: 'cargo install lfsx-server',
    },
  ];

  protected readonly stores: readonly Store[] = [
    {
      key: 'bucket',
      title: $localize`:@@landing.storage.bucket.title:Objects in a bucket`,
      flag: 'LFSX_STORAGE=s3',
      body: $localize`:@@landing.storage.bucket.body:An S3-compatible bucket instead of the volume, which is what unties capacity from one machine. The locks move with the objects, taken with a conditional write so the store itself decides who arrived first, and that is what makes a second replica possible. The server checks at startup that the store really performs it.`,
    },
    {
      key: 'compression',
      title: $localize`:@@landing.storage.compression.title:Compression`,
      flag: 'LFSX_COMPRESSION=zstd',
      body: $localize`:@@landing.storage.compression.body:The received wisdom is that an LFS store is already compressed, and for PNG, MP3 and OGG that is true. It is badly wrong for meshes: 10.4 times on .tga and 2.9 to 6.7 times on .fbx, measured on two real Unity projects, 71% smaller overall. Objects are compressed in four-megabyte frames with an index, so ranges still work and memory stays flat.`,
    },
    {
      key: 'encryption',
      title: $localize`:@@landing.storage.encryption.title:Encryption at rest`,
      flag: 'ChaCha20-Poly1305',
      body: $localize`:@@landing.storage.encryption.body:For most self-hosted deployments the better answer is the volume. Reach for this when the storage itself is what you do not trust: a shared NAS, a bucket somebody else operates, a disk you will one day return under warranty. The key is a file path, never the key itself, and it does not protect against anyone who has the running server.`,
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
      answered: $localize`:@@landing.faq.locks.answered:Yes, for the assets that cannot be merged, with force-open reserved for admin and Maintainer. In a bucket the server checks at startup that the store really refuses a conditional write, and gives locking up rather than hand the same lock to two people.`,
    },
    {
      key: 'contribute',
      asked: $localize`:@@landing.faq.contribute.asked:Can I contribute a forge?`,
      answered: $localize`:@@landing.faq.contribute.answered:Three providers exist, so the shape is settled rather than guessed: one module under server/src/auth/ exposing three functions, plus a config variant and three arms in auth.rs. The caching, the challenge handling and the rejection accounting are shared and provider-blind.`,
    },
  ];
}
