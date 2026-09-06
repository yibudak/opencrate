# Contributing to OpenCrate

Contributions to code, translations, documentation and hardware compatibility are welcome.
You do not need an ASUS device to work on the control logic, interface or tooling.
Please follow our [code of conduct](CODE_OF_CONDUCT.md).

## Find a starting point

- Look for [good first issues](https://github.com/yibudak/opencrate/labels/good%20first%20issue) or [help wanted](https://github.com/yibudak/opencrate/labels/help%20wanted).
- Use an [issue template](https://github.com/yibudak/opencrate/issues/new/choose) for a reproducible bug or feature proposal. Discuss substantial hardware or architecture changes before implementing them.
- Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).

## Windows development setup

Install [Rust](https://www.rust-lang.org/tools/install), Git, Python 3.12 or later,
and [MSYS2](https://www.msys2.org/). In the **MSYS2 MINGW64** terminal, install the
native compiler and resource tools:

```sh
pacman -S --needed mingw-w64-x86_64-gcc
```

Add the MSYS2 installation's `mingw64/bin` directory to your Windows PATH, then
open a new PowerShell terminal. Fork the repository and clone your fork. From
the project directory, install Rust components and build:

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component rustfmt --component clippy
cargo build --locked -p opencrate-ui
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`rust-toolchain.toml` selects the GNU Windows toolchain. Use `gcc --version` and
`windres --version` to check your native tools. A version-controlled `Cargo.lock`
keeps application dependencies reproducible. The first build needs internet access.

`cargo run --locked -p opencrate-ui` starts the app. If you have a saved lighting
configuration with restoration enabled, launching the app can apply it. Opening
the app does not apply fan or power changes. Unsupported or missing hardware is
reported by the interface; it does not prevent work on unrelated features.

## Work without Windows or ASUS hardware

Install the native stable Rust toolchain and run the portable library tests:

```sh
cargo +stable test --locked --no-default-features --lib -p opencrate-core -p opencrate-aura -p opencrate-fan -p opencrate-power
```

These tests use synthetic data and mock controllers. They cover RGB packets,
animations, fan validation and power-policy rollback without hardware access.
The Windows GUI, COM backend, Windows APIs and installer are checked separately
on GitHub's Windows runners. Linux coverage reports describe only portable logic.

## Check scripts and documentation

```powershell
python -m pip install -r scripts/requirements-dev.txt
python -m ruff check scripts
python -m ruff format --check scripts
python -m unittest discover -s scripts/tests -v
python scripts/check-privacy.py
npm ci --prefix .github/linters --ignore-scripts
node .github/linters/node_modules/markdownlint-cli2/markdownlint-cli2-bin.mjs
```

Documentation linting uses Node.js 24. CI also runs actionlint on workflows,
PSScriptAnalyzer on installer scripts, Rustdoc with warnings denied, cargo-deny
on dependencies, and CodeQL on Rust, Python and Actions. Tool versions are
recorded in the workflow or dependency files; use the same versions locally.
To fix formatting, use `cargo fmt --all` and `python -m ruff format scripts`.

## Prepare a pull request

1. Create a focused branch from current `main` in your fork.
2. Explain the problem and intended behavior. Add a regression test for a bug
   when it can be tested reliably; avoid tests that merely duplicate implementation.
3. Keep code, comments and documentation in English. Update `en.json`, `zh-CN.json`
   and `tr.json` together when adding user-visible strings; preserve placeholders
   and serialized identifiers. See the [translation guide](README.md#contributing-translations).
4. Run the relevant checks and describe what you tested. Clearly separate mock
   tests from physical hardware observations. Screenshots are useful for UI changes.
5. Open a PR against `main` and complete its short template. Draft PRs are welcome.
   CI runs on fork PRs without repository secrets; first-time contributors may
   need a maintainer to approve the workflow run.
6. Address review comments and keep the branch current. A maintainer merges once
   required checks and review conversations are resolved. Do not force-push `main`
   or change a published release tag.

Use clear, imperative commit subjects. A GitHub `noreply` email keeps your personal
address out of commits; configure the address shown in your GitHub email settings.
Contributions are provided under the project's [MIT license](LICENSE). Preserve
third-party copyright and license notices when adapting or updating dependencies.

## Hardware changes and privacy

Preserve fan minimum duty, critical-temperature protection, rollback and external
controller ownership checks. Prefer protocol fixtures and mock services for CI.
Hardware writes belong in explicit, deliberate actions. Diagnostic examples with
`--live` or `--test-roundtrip` can change device settings and are not CI tests.

Do not commit personal preferences, machine names, serial numbers, raw captures,
firmware binaries, credentials, crash dumps or local build output. Review logs
and screenshots before sharing: custom plan names, device paths and usernames
can identify a computer. Use synthetic fixtures and document generally applicable
protocol behavior. The privacy check is a guard, not a replacement for review.

## Dependency policy

`deny.toml` rejects vulnerable or yanked dependencies, unknown sources, unapproved
licenses and wildcard version requirements. Internal path dependencies share
version requirements in `[workspace.dependencies]`. Multiple transitive Windows
crate versions are reported as warnings because the GUI stack currently needs them.

There is one specific advisory exception: [RUSTSEC-2026-0192](https://rustsec.org/advisories/RUSTSEC-2026-0192.html)
reports the unmaintained `ttf-parser` dependency used through `ab_glyph`/`epaint`.
There is no patched version. OpenCrate supplies embedded fonts; this is a tracked
maintenance limitation tracked in [issue #1](https://github.com/yibudak/opencrate/issues/1),
not a blanket exception for font vulnerabilities. Review
and remove the exception when the UI stack moves to a maintained parser. New
vulnerability advisories still fail CI. The Ubuntu font license is allowed only
for the exact bundled `epaint_default_fonts` version, and its notice is packaged.

Keep package-version updates, release tags and publication with maintainers unless
they are the purpose of your PR. See [installer and release instructions](installer/README.md).
