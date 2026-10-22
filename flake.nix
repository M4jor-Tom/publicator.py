{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      pyInterp = pkgs.python312;
      pyPkgs = ps: [ ps.playwright ];
      python = pyInterp.withPackages pyPkgs;
      # devShell adds pytest; the only shipped Python runtime is the browser worker.
      devPython = pyInterp.withPackages (ps: pyPkgs ps ++ [ ps.pytest ]);
      src = ./.;
      browsers = pkgs.playwright-driver.browsers;
      # Keep the worker and static skill independent of the source-tree store path.
      runtimeAssets = pkgs.runCommand "publicator-runtime-assets" { } ''
        mkdir -p $out/python/publicator_da $out/skill
        cp ${src}/python/publicator_da/*.py $out/python/publicator_da/
        cp ${src}/.claude/skills/publish-deviantart/SKILL.md $out/skill/SKILL.md
      '';
      # The Rust publisher starts this isolated Python Playwright protocol worker.
      rustWorker = pkgs.writeShellScriptBin "publicator-da-worker" ''
        export PYTHONPATH=${runtimeAssets}/python
        export PLAYWRIGHT_BROWSERS_PATH=${browsers}
        export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
        exec ${python}/bin/python -m publicator_da.worker "$@"
      '';
      rustSkill = "${runtimeAssets}/skill/SKILL.md";
      rustRuntime = [ pkgs.git pkgs.imagemagick pkgs.llmfit rustWorker ];
      desktopRuntime = [
        pkgs.libxkbcommon pkgs.wayland pkgs.libGL
        pkgs.libx11 pkgs.libxcursor pkgs.libxi pkgs.libxrandr pkgs.libxinerama pkgs.libxext
      ];
      wrapRustProgram = { package, binary, runtime, libraries ? [ ] }:
        pkgs.symlinkJoin {
          name = package.name;
          paths = [ package ];
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postBuild = ''
            wrapProgram $out/bin/${binary} \
              --prefix PATH : ${pkgs.lib.makeBinPath runtime} \
              --set PUBLICATOR_WORKER ${rustWorker}/bin/publicator-da-worker \
              --set PUBLICATOR_SKILL ${rustSkill} \
              ${pkgs.lib.optionalString (libraries != [ ]) "--prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath libraries}"}
          '';
        };
      rustCliUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "publicator-rust-cli";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "--no-default-features" "--bin" "publicator" ];
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ];
        doCheck = false;
      };
      rustDesktopUnwrapped = pkgs.rustPlatform.buildRustPackage {
        pname = "publicator-rust-desktop";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        cargoBuildFlags = [ "--features" "desktop" "--bin" "publicator-ui" ];
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.openssl ] ++ desktopRuntime;
        doCheck = false;
      };
      rustCli = wrapRustProgram {
        package = rustCliUnwrapped;
        binary = "publicator";
        runtime = rustRuntime;
      };
      rustDesktop = wrapRustProgram {
        package = rustDesktopUnwrapped;
        binary = "publicator-ui";
        runtime = rustRuntime;
        libraries = desktopRuntime;
      };
      rustCommand = name: subcommand: extra: {
        type = "app";
        program = "${pkgs.writeShellApplication {
          name = name;
          runtimeInputs = [ rustCli python rustWorker ] ++ extra;
          text = ''
            export PUBLICATOR_WORKER=${rustWorker}/bin/publicator-da-worker
            export PUBLICATOR_SKILL=${rustSkill}
            exec ${rustCli}/bin/publicator ${subcommand} "$@"
          '';
        }}/bin/${name}";
      };
      # A real, non-automated Firefox for one-time DeviantArt sign-in. Playwright
      # is blocked at login; the publisher copies this flake-pinned profile.
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
        publicator-desktop = rustDesktop;
        publicator-rust-cli = rustCli;
        publicator-rust-desktop = rustDesktop;
      };
      apps.${system} = {
        # `ui` is the native Iced frontend; browser publishing remains a CLI worker.
        ui = {
          type = "app";
          program = "${pkgs.writeShellApplication {
            name = "ui";
            runtimeInputs = [ rustDesktop ];
            text = ''exec ${rustDesktop}/bin/publicator-ui "$@"'';
          }}/bin/ui";
        };
        # Publish one pending entry by default, one by UUID, or all pending entries.
        da-publish = rustCommand "da-publish" "da-publish" [ pkgs.imagemagick browsers ];
        # Explicit Python-only DOM prober; not part of the publication pipeline.
        pw-daemon = {
          type = "app";
          program = "${pkgs.writeShellApplication {
            name = "pw-daemon";
            runtimeInputs = [ python browsers ];
            text = ''
              export PYTHONPATH=${runtimeAssets}/python
              export PLAYWRIGHT_BROWSERS_PATH=${browsers}
              export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
              exec ${python}/bin/python -m publicator_da.pw_daemon "$@"
            '';
          }}/bin/pw-daemon";
        };
        generate-meta = rustCommand "generate-meta" "generate-meta" [ pkgs.llmfit pkgs.imagemagick ];
        vision-models = rustCommand "vision-models" "vision-models" [ pkgs.llmfit ];
        validate = rustCommand "validate" "validate" [ ];
        echo-first = rustCommand "echo-first" "echo-first" [ ];
        prompt-audit = rustCommand "prompt-audit" "prompt-audit" [ pkgs.git ];
        check-steps = rustCommand "check-steps" "check-steps" [ ];
        login = login;
      };
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          devPython pkgs.git pkgs.imagemagick pkgs.playwright-driver.browsers
          pkgs.ollama pkgs.llmfit pkgs.rustc pkgs.cargo pkgs.rustfmt pkgs.clippy
          pkgs.pkg-config pkgs.openssl pkgs.pcre2 pkgs.jq pkgs.curl pkgs.xvfb pkgs.xvfb-run pkgs.x11vnc
          pkgs.novnc pkgs.python312Packages.websockify pkgs.xdotool pkgs.xwininfo
          pkgs.xprop pkgs.xdpyinfo pkgs.wmctrl pkgs.openbox pkgs.nodejs pkgs.playwright-test
        ];
        shellHook = ''
          export PLAYWRIGHT_BROWSERS_PATH=${pkgs.playwright-driver.browsers}
          export PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS=true
          export NOVNC_WEB=${pkgs.novnc}/share/webapps/novnc
          export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath desktopRuntime}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
        '';
      };
    };
}
