{
  description = "A fast, lightweight, secure Git LFS server";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-darwin"
        "x86_64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      overlays.default = final: _prev: {
        lfsx = final.callPackage ./packaging/nix/lfsx.nix { };
      };

      packages = forAllSystems (
        system:
        let
          lfsx = nixpkgs.legacyPackages.${system}.callPackage ./packaging/nix/lfsx.nix { };
        in
        {
          inherit lfsx;
          default = lfsx;
        }
      );

      apps = forAllSystems (system: {
        default = self.apps.${system}.lfsx-server;

        lfsx-server = {
          type = "app";
          program = "${self.packages.${system}.lfsx}/bin/lfsx-server";
        };

        lfsx = {
          type = "app";
          program = "${self.packages.${system}.lfsx}/bin/lfsx";
        };
      });
    };
}
