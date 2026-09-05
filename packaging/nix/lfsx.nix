{
  lib,
  rustPlatform,
  versionCheckHook,
}:

let
  root = ../..;
in
rustPlatform.buildRustPackage {
  pname = "lfsx";
  version = (lib.importTOML (root + "/server/Cargo.toml")).package.version;
  __structuredAttrs = true;

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/cli")
      (root + "/server")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";

  doInstallCheck = true;
  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgram = "${placeholder "out"}/bin/lfsx";
  versionCheckProgramArg = "--version";

  meta = {
    description = "Fast, lightweight, secure Git LFS server";
    homepage = "https://lfsx.dev";
    license = lib.licenses.mpl20;
    mainProgram = "lfsx-server";
    platforms = lib.platforms.unix;
  };
}
