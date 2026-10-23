{
  description = "agentgateway, an open source data plane for agentic AI connectivity";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    { self, nixpkgs, ... }:
    let
      inherit (nixpkgs.lib) genAttrs head importTOML;
      pname = (head (importTOML ./crates/agentgateway-app/Cargo.toml).bin).name;
      systems = [
        "aarch64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems = f: genAttrs systems (system: f (import nixpkgs { inherit system; }));
      gitRev = self.rev or self.dirtyRev or null;
    in
    {
      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
      overlays.default = final: _prev: {
        ${pname} = final.callPackage ./default.nix { inherit gitRev pname; };
      };
      packages = forAllSystems (
        pkgs:
        let
          agentgateway = pkgs.callPackage ./default.nix { inherit gitRev pname; };
          inherit (agentgateway) agctl;
        in
        {
          default = agentgateway;
          ${pname} = agentgateway;

          # Adds the node/pnpm build of the admin UI, embedded into the binary
          "${pname}-full" = agentgateway.override { withUI = true; };

          # The built admin UI on its own; quick way to check the pnpmDeps hash.
          "${pname}-ui-assets" = agentgateway.ui;

          # The Go CLI from controller/cmd/agctl, not the full controller.
          ${agctl.pname} = agctl;
        }
      );
    };
}
