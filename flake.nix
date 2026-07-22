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

      # A real, non-automated Firefox for a one-time DeviantArt sign-in. DA's
      # PerimeterX wall blocks every Playwright browser at the login page, so
      # login must happen in a genuine browser (no navigator.webdriver). It
      # writes the logged-in + cleared session to the repo-local profile that
      # publish-next then copies. Reproducible: firefox is flake-pinned.
      login = {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = "login";
          runtimeInputs = [ pkgs.firefox ];
          text = ''
            profile="$PWD/.deviantart-login"
            mkdir -p "$profile"
            echo "Opening Firefox — sign in to DeviantArt, then close the window."
            echo "(If the page never loads, allow firefox through opensnitch.)"
            firefox --no-remote --profile "$profile" https://www.deviantart.com/users/login
          '';
        }}/bin/login";
      };
    in {
      apps.${system} = {
        publish-next = app "publish-next" "publish_next.py" [ pkgs.imagemagick browsers ] true;
        pw-daemon = app "pw-daemon" "pw_daemon.py" [ browsers ] true;
        validate = app "validate" "validate.py" [ ] false;
        login = login;
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
