# nixpkgs packaging

`update-nix-package.sh` regenerates the nixpkgs derivation for a given release and proves it
builds before anyone sends it upstream.

```bash
./update-nix-package.sh          # latest GitHub release
./update-nix-package.sh 1.13.0   # a specific version
```

Docker is the only requirement. Nix runs inside the `nixos/nix` image, so this works from Git Bash
on Windows as well as from Linux or macOS. The nix store lives in a named volume (`lfsx-nix-store`),
so the first run downloads a Rust toolchain and later runs reuse it.

What a run does:

1. Resolves the version, from the argument or from the latest GitHub release.
2. Writes the derivation with placeholder hashes and builds it. Each failed build reveals exactly
   one hash, the source archive first, then the vendored crates, so two failures resolve both.
3. Builds for real, then runs `./result/bin/lfsx --version` and compares it to the expected
   version. Asking the binary rather than grepping the build log also covers the run where nix
   had the derivation cached and printed no phase output.
4. Checks the file with `nixfmt`.

It writes `package.nix` next to itself. Copy that to `pkgs/by-name/lf/lfsx/package.nix` in a
nixpkgs checkout. The build under `build/` uses the same derivation with `maintainers` emptied,
because the maintainer attribute does not exist in nixpkgs until the entry below is merged.

## Maintainer entry

`maintainers/maintainer-list.nix`, alphabetically between `brw` and `bryango`:

```nix
  bryanfrd = {
    name = "Bryan Ferrando";
    email = "bryanferrando59@gmail.com";
    github = "BryanFRD";
    githubId = 20824933;
  };
```

## Why versionCheckProgram points at lfsx

`versionCheckHook` calls the main program with `--version`, and for this package the main program
is `lfsx-server`, which ignores arguments and starts serving instead (#304). Until that is fixed
the check runs against the `lfsx` client, which answers correctly. When #304 lands, drop the
`versionCheckProgram` line from the template inside the script.
