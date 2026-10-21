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
      # `llm-openrouter` IS in this nixpkgs pin (unlike llm-claude-cli above), so
      # no vendoring needed. The API key is NOT baked in: it reads $OPENROUTER_KEY
      # at runtime.
      pyPkgs = ps: [ ps.jsonschema ps.playwright ps.llm llm-claude-cli ps.llm-openrouter ];
      python = pyInterp.withPackages pyPkgs;
      # devShell adds pytest so `nix develop -c pytest` runs the suite; apps don't ship it.
      devPython = pyInterp.withPackages (ps: pyPkgs ps ++ [ ps.pytest ]);
      src = ./.;
      browsers = pkgs.playwright-driver.browsers;
      # Each script resolves its code assets (the JSON schema) via __file__, so apps
      # must run them from the packaged source dir. The tags file, like all other
      # data + runtime state, resolves against CWD — invoke from the Art data dir
      # (`nix run <this>#app`).
      # extra: per-app runtime deps. pw: needs Playwright browsers (drags in the
      # heavy chromium/webkit closure — only the browser-driving apps set it).
      app = name: module: extra: pw: {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = name;
          runtimeInputs = [ python ] ++ extra;
          text = (if pw then ''
            export PLAYWRIGHT_BROWSERS_PATH=${browsers}
            export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          '' else "") + ''
            export PYTHONPATH=${src}/src
            exec python -m publicator.apps.${module} "$@"
          '';
        }}/bin/${name}";
      };

      # A real, non-automated Firefox for a one-time DeviantArt sign-in. DA's
      # PerimeterX wall blocks every Playwright browser at the login page, so
      # login must happen in a genuine browser (no navigator.webdriver). It
      # writes the logged-in + cleared session to the repo-local profile that
      # the UI then copies. Reproducible: firefox is flake-pinned.
      rustCli = pkgs.rustPlatform.buildRustPackage {
        pname = "publicator-rust-cli";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        doCheck = false;
      };
      desktopRuntime = [
        pkgs.libxkbcommon pkgs.wayland
        pkgs.libx11 pkgs.libxcursor pkgs.libxi pkgs.libxrandr pkgs.libxinerama pkgs.libxext
      ];
      rustDesktop = pkgs.rustPlatform.buildRustPackage {
        pname = "publicator-rust-desktop";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "--features" "desktop" "--bin" "publicator-ui" ];
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ] ++ desktopRuntime;
        doCheck = false;
      };
      rustWorker = pkgs.writeShellScriptBin "publicator-da-worker" ''
        export PYTHONPATH=${src}/python
        export PLAYWRIGHT_BROWSERS_PATH=${browsers}
        export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
        exec ${python}/bin/python -m publicator_da.worker "$@"
      '';
      rustCommand = name: subcommand: extra: {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = name;
          runtimeInputs = [ rustCli python rustWorker ] ++ extra;
          text = ''
            export PUBLICATOR_WORKER=${rustWorker}/bin/publicator-da-worker
            export PUBLICATOR_SKILL=${src}/.claude/skills/publish-deviantart/SKILL.md
            exec ${rustCli}/bin/publicator ${subcommand} "$@"
          '';
        }}/bin/${name}";
      };
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
      packages.${system} = {
        publicator-core = rustCli;
        publicator-rust-cli = rustCli;
        publicator-rust-desktop = rustDesktop;
      };
      apps.${system} = {
        # Rust CLI wrappers are opt-in during the migration; existing app names remain Python.
        rust-validate = rustCommand "rust-validate" "validate" [ ];
        rust-echo-first = rustCommand "rust-echo-first" "echo-first" [ ];
        rust-generate-meta = rustCommand "rust-generate-meta" "generate-meta" [ pkgs.ollama pkgs.llmfit pkgs.imagemagick ];
        rust-vision-models = rustCommand "rust-vision-models" "vision-models" [ pkgs.llmfit ];
        rust-prompt-audit = rustCommand "rust-prompt-audit" "prompt-audit" [ pkgs.git ];
        rust-da-publish = rustCommand "rust-da-publish" "da-publish" [ pkgs.imagemagick browsers ];
        rust-check-steps = rustCommand "rust-check-steps" "check-steps" [ ];
        rust-ui = {
          type = "app";
          program = "${pkgs.writeShellApplication {
            name = "rust-ui";
            runtimeInputs = [ rustDesktop python pkgs.git pkgs.imagemagick pkgs.ollama pkgs.llmfit ];
            text = ''
              export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath desktopRuntime}
              exec ${rustDesktop}/bin/publicator-ui "$@"
            '';
          }}/bin/rust-ui";
        };
        # `claude` on PATH so llm-claude-cli can shell out for AI metadata
        # (uses the logged-in Claude subscription at $HOME/.claude — no API key).
        ui = app "ui" "ui" [ pkgs.git pkgs.imagemagick browsers claude pkgs.ollama pkgs.llmfit ] true;
        # Publish the first pending publications.json entry, one by uuid, or --all (the extracted Playwright flow):
        #   nix run .#da-publish -- --data-dir <dir> [--uuid <id> | --all] [--headless]
        da-publish = app "da-publish" "da_publish" [ pkgs.imagemagick browsers ] true;
        pw-daemon = app "pw-daemon" "pw_daemon" [ browsers ] true;
        # AI title/description for image paths, no gallery/data dir needed:
        #   nix run .#generate-meta -- [--openrouter] IMAGE...
        # `claude` for the key-free path; no browsers (this one never publishes).
        generate-meta = app "generate-meta" "generate_meta" [ claude pkgs.ollama pkgs.llmfit pkgs.imagemagick ] false;
        # Host-fit verdicts for configured local vision models; no Ollama daemon needed.
        vision-models = app "vision-models" "vision_models" [ pkgs.llmfit ] false;
        validate = app "validate" "validate" [ ] false;
        # First state=unpublished entry's path/title/schedule (used by the skill).
        echo-first = app "echo-first" "echo_first" [ ] false;
        # Prompt-mapping coverage. `parsed` drops to 0 on grammar drift;
        # `nearest` rising means archive writes are failing (docs/features/prompt-mapping.md).
        prompt-audit = app "prompt-audit" "prompt_audit" [ pkgs.git ] false;
        # Drift guard: fails if the publish-deviantart skill grew a step that
        # deviantart.py's STEPS doesn't implement. No browser/data deps (cheap CI).
        check-steps = {
          type = "app";
          program = "${pkgs.writeShellApplication {
            name = "check-steps";
            runtimeInputs = [ python ];
            text = ''
              export PYTHONPATH=${src}/src
              exec python -m publicator.apps.da_publish --check-steps "$@"
            '';
          }}/bin/check-steps";
        };
        login = login;
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = [ devPython pkgs.git pkgs.imagemagick pkgs.playwright-driver.browsers claude pkgs.ollama pkgs.llmfit pkgs.rustc pkgs.cargo pkgs.rustfmt pkgs.clippy pkgs.pkg-config pkgs.openssl pkgs.pcre2 ];
        shellHook = ''
          export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
          export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          export PYTHONPATH="$PWD/src''${PYTHONPATH:+:$PYTHONPATH}"
        '';
      };
    };
}
