{
  lib,
  stdenvNoCC,
  rustPlatform,
  cmake,
  perl,
  nodejs_24,
  pnpm_11,
  fetchPnpmDeps,
  buildGoModule,
  go_1_27,
  nix-update-script,
  pnpmConfigHook,
  gitRev ? null,
  pname ? null,
  agctlPname ? "agctl",
  agctlBinName ? null,
  version ? "0-unstable-${if gitRev != null then builtins.substring 0 12 gitRev else "dirty"}",
  withUI ? false,
  # vendorHash below must be repinned whenever go.mod/go.sum change.
  withAgctl ? true,
}:
let
  appManifest = lib.importTOML ./crates/agentgateway-app/Cargo.toml;
  cargoPackage = appManifest.package.name;
  binName = (lib.head appManifest.bin).name;
  name = if pname != null then pname else binName;

  agctlCmd = "controller/cmd/agctl";
  agctlBuiltBin = baseNameOf agctlCmd;
  agctlBin = if agctlBinName != null then agctlBinName else agctlBuiltBin;

  src = lib.fileset.toSource {
    root = ./.;
    # Mirrors what the Dockerfile
    fileset = lib.fileset.unions [
      ./.cargo
      ./Cargo.lock
      ./Cargo.toml
      ./catalog
      ./crates
    ];
  };
  proxy = rustPlatform.buildRustPackage {
    pname = name;
    meta = {
      description = "Open source data plane for agentic AI connectivity";
      homepage = "https://github.com/agentgateway/agentgateway";
      license = lib.licenses.asl20;
      mainProgram = binName;
      platforms = lib.platforms.unix;
    };

    inherit version src;

    env = {
      VERSION = version;
      GIT_REVISION = if gitRev != null then gitRev else "unknown";
      AGENTGATEWAY_BUILD_buildVersion = version;
      AGENTGATEWAY_BUILD_buildGitRevision = if gitRev != null then gitRev else "unknown";
    };

    cargoLock = {
      lockFile = ./Cargo.lock;
      # Avoids restating each [patch.crates-io] deps
      # NOTE nixpkgs would need to be defined using `outputHashes`
      allowBuiltinFetchGit = true;
    };

    cargoBuildFlags = [
      "--package"
      cargoPackage
    ];
    buildFeatures = lib.optional withUI "ui";

    nativeBuildInputs = [
      cmake # aws-lc-sys
      perl # aws-lc-sys
      rustPlatform.bindgenHook
    ];

    preBuild = lib.optionalString withUI ''
      mkdir -p ui
      cp -r ${uiAssets} ui/dist
    '';

    doCheck = false;

    # Lets the UI be built alone, to check the pnpmDeps hash without the Rust build.
    passthru = {
      ui = uiAssets;
    }
    // lib.optionalAttrs withAgctl { inherit agctl; };
  };

  uiName = "${name}-ui-assets";
  uiSrc = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [
      ./ui
      ./schema/admin.json
      ./schema/cel.json
      ./schema/config.json
    ];
  };
  uiAssets = stdenvNoCC.mkDerivation {
    name = uiName;
    src = uiSrc;

    pnpmRoot = "ui";
    pnpmDeps = fetchPnpmDeps {
      pname = uiName;
      # Only the manifests, so the hash changes with the lockfile and not with every edit under ui
      src = lib.fileset.toSource {
        root = ./ui;
        fileset = lib.fileset.unions [
          ./ui/package.json
          ./ui/pnpm-lock.yaml
          ./ui/pnpm-workspace.yaml
        ];
      };
      pnpm = pnpm_11;
      fetcherVersion = 4;
      # TODO: needs a good way to keep this in sync with package updates
      hash = "sha256-txBf8OOkiLXxmtd8xPiXhcYwJ/pb2XujA5gc/sXoits=";
    };

    nativeBuildInputs = [
      nodejs_24
      pnpm_11
      pnpmConfigHook
    ];

    buildPhase = ''
      runHook preBuild
      pnpm --dir ui build
      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall
      cp -r ui/dist $out
      runHook postInstall
    '';

    # Refreshes pnpmDeps.hash above after a ui/pnpm-lock.yaml bump. Nothing runs
    # this automatically outside nixpkgs; invoke it with `nix-update`.
    passthru.updateScript = nix-update-script {
      attrPath = uiName;
      extraArgs = [
        "--flake"
        "--version=skip"
        "--custom-dep"
        "pnpmDeps"
      ];
    };
  };

  agctl = (buildGoModule.override { go = go_1_27; }) (
    {
      pname = agctlPname;
      inherit version;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./api
          ./controller
          ./go.mod
          ./go.sum
        ];
      };

      vendorHash = "sha256-CyBbj2VD8Tc/po0UX59CZMFDanV3WqT5NO+EWVzx8qM=";
      subPackages = [ agctlCmd ];

      # LDFLAGS take from controller/Makefile
      ldflags = [
        "-s"
        "-w"
        "-X"
        "github.com/agentgateway/agentgateway/controller/pkg/version.Version=${version}"
      ];

      meta = {
        description = "agentgateway command line interface";
        homepage = "https://github.com/agentgateway/agentgateway";
        license = lib.licenses.asl20;
        mainProgram = agctlBin;
        platforms = lib.platforms.unix;
      };
    }
    // lib.optionalAttrs (agctlBin != agctlBuiltBin) {
      postInstall = ''
        mv "$out/bin/${agctlBuiltBin}" "$out/bin/${agctlBin}"
      '';
    }
  );
in
proxy
