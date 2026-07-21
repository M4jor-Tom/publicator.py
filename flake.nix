{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      python = pkgs.python3.withPackages (ps: [ ps.jsonschema ps.playwright ]);
      src = ./.;
      browsers = pkgs.playwright-driver.browsers;
      # Each script resolves its code assets (schema, tags/) via __file__, so apps
      # must run them from the packaged source dir. Data + runtime state resolve
      # against CWD — invoke from the Art data dir (`nix run <this>#app`).
      # extra: per-app runtime deps. pw: needs Playwright browsers (drags in the
      # heavy chromium/webkit closure — only the browser-driving apps set it).
      app = name: script: extra: pw: {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = name;
          runtimeInputs = [ python ] ++ extra;
          text = (if pw then ''
            export PLAYWRIGHT_BROWSERS_PATH=${browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          '' else "") + ''
            exec python ${src}/${script} "$@"
          '';
        }}/bin/${name}";
      };
    in {
      apps.${system} = {
        publish-next = app "publish-next" "publish_next.py" [ pkgs.imagemagick browsers ] true;
        publish-deviantart = app "publish-deviantart" "publish_deviantart.py" [ browsers ] true;
        pw-daemon = app "pw-daemon" "pw_daemon.py" [ browsers ] true;
        publish = app "publish" "publish.py" [ ] false;
        validate = app "validate" "validate.py" [ ] false;
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = [ python pkgs.imagemagick pkgs.playwright-driver.browsers ];
        shellHook = ''
          export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
          export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
        '';
      };
    };
}
