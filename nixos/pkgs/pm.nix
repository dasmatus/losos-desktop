# pm, the package manager this OS keeps as its system manager: it builds signed
# recipes into .cpkg archives and runs their entrypoints in a sandbox, and
# pmd serves it over D-Bus.
#
# The image takes the OS from nixpkgs and keeps pm as the tool a user of the
# running system builds and runs software with. It is pinned to a commit and
# moved by hand, because pm's plugin contract has changed under this
# repository before.
{
  lib,
  rustPlatform,
  fetchgit,
  pkg-config,
}:

rustPlatform.buildRustPackage {
  pname = "pm";
  version = "0.1.0-unstable-2026-09-20";

  # fetchgit rather than fetchFromGitHub: a git clone reaches github.com from
  # hosts that cannot use its archive endpoint.
  src = fetchgit {
    url = "https://github.com/dichhead/pm";
    rev = "47a32e3f74a99f3fe21b7af2d73ab1446da8bacc";
    hash = "sha256-uckTm/0j4RBxPDmicpfkYtuMa4CHSx1NbtAUnWp+JRE=";
  };

  cargoHash = "sha256-aTpouEF4TOrN4CYwqSFG8mK/mr+u3C0AeNldyCBNH1U=";

  nativeBuildInputs = [ pkg-config ];

  # pm's own suite, less what assumes a conventional Linux host. context,
  # perms, pm_trace, sandbox, steps, symbols and worker run /bin/cat,
  # /bin/true and friends under ptrace or in pm's jail, which mirrors the
  # host's /bin and /usr; the nix build sandbox has neither, so they fail here
  # on the missing file and not on pm. The pm build in ci.yml runs pm on a
  # host that has both. The same
  # assumption holds for pm at run time on this OS, whose /bin and /usr hold
  # almost nothing; docs/nixos.md tracks that.
  cargoTestFlags = [
    "--lib"
    "--bins"
  ]
  ++ map (t: "--test=${t}") [
    "build_file"
    "cli_build"
    "command_output"
    "config_file"
    "download"
    "example_plugins"
    "graph"
    "landlock"
    "metadata"
    "packaging"
    "plugins"
    "progress"
    "wire"
  ];
  # The two tests in command_output that spawn a literal /bin/true.
  checkFlags = [
    "--skip=a_finished_command_leaves_no_line_behind"
    "--skip=a_sandbox_without_progress_still_runs_commands"
  ];

  meta = {
    description = "Build signed recipes into packages and run them in a Linux sandbox";
    homepage = "https://github.com/dichhead/pm";
    license = lib.licenses.gpl3Only;
    platforms = lib.platforms.linux;
    mainProgram = "pm";
  };
}
