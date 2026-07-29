{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  # Key-free AI metadata: `llm` drives the logged-in `claude` CLI via llm-claude-cli.
  inputs.claude-code.url = "github:sadjow/claude-code-nix";
  outputs = { self, nixpkgs, claude-code }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        # inline-snapshot (a test-only transitive dep of llm's closure) has failing
        # self-tests in this pin; skip its check so llm can build. (from scenharnist)
        overlays = [
          (final: prev: {
            pythonPackagesExtensions = prev.pythonPackagesExtensions ++ [
              (pyfinal: pyprev: {
                inline-snapshot =
                  pyprev.inline-snapshot.overridePythonAttrs (_: { doCheck = false; });
              })
            ];
          })
        ];
      };
      claude = claude-code.packages.${system}.default;
      # 3.12, not the default: llm's closure pulls sphinx, which dropped 3.11 in
      # this pin. The scripts are version-agnostic.
      pyInterp = pkgs.python312;
      # `llm-claude-cli` isn't in nixpkgs — package it from PyPI so `llm` discovers
      # it via its entry point (no stateful `llm install`). Vendored, hash-pinned.
      llm-claude-cli = pyInterp.pkgs.buildPythonPackage {
        pname = "llm-claude-cli";
        version = "0.1.3";
        pyproject = true;
        src = pkgs.fetchurl {
          url = "https://files.pythonhosted.org/packages/68/90/392ed1ee834eeb20fb2c7324a94284d860aef7c24a1568d214ee0e5f98d7/llm_claude_cli-0.1.3.tar.gz";
          hash = "sha256-crhtbO1xTWY3BCRrIq6J8bVe732pGUkAZtD2Pg+kB5E=";
        };
        build-system = [ pyInterp.pkgs.setuptools ];
        dependencies = [ pyInterp.pkgs.llm ];
        pythonImportsCheck = [ "llm_claude_cli" ];
        doCheck = false;  # upstream tests drive the real claude CLI
      };
      python = pyInterp.withPackages (ps: [ ps.jsonschema ps.playwright ps.llm llm-claude-cli ]);
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
        # `claude` on PATH so llm-claude-cli can shell out for AI metadata
        # (uses the logged-in Claude subscription at $HOME/.claude — no API key).
        publish-next = app "publish-next" "publish_next.py" [ pkgs.imagemagick browsers claude ] true;
        pw-daemon = app "pw-daemon" "pw_daemon.py" [ browsers ] true;
        validate = app "validate" "validate.py" [ ] false;
        login = login;
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = [ python pkgs.imagemagick pkgs.playwright-driver.browsers claude ];
        shellHook = ''
          export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
          export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
        '';
      };
    };
}
