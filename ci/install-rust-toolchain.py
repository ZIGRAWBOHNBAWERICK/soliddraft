#!/usr/bin/env python3
"""Install a rustup toolchain and retry stalled downloads."""
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
import time


DEFAULT_TIMEOUT = 120
DEFAULT_ATTEMPTS = 3


def bootstrap_rustup():
    # These are the hosts used by Blade, boat and GitHub-hosted workflows.
    systems = {"Linux": "unknown-linux-gnu", "Darwin": "apple-darwin", "Windows": "pc-windows-msvc"}
    system = platform.system()
    suffix = systems.get(system)
    if suffix is None:
        raise ValueError(f"cannot bootstrap rustup on {system}")
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(machine, machine)
    filename = "rustup-init.exe" if system == "Windows" else "rustup-init"
    url = f"https://static.rust-lang.org/rustup/dist/{arch}-{suffix}/{filename}"
    with tempfile.TemporaryDirectory() as directory:
        installer = Path(directory) / filename
        subprocess.run(
            ["curl", "--proto", "=https", "--fail", "--silent", "--show-error", "--location",
             "--retry", "2", "--retry-max-time", "30", "--max-time", "30", "--output", installer, url],
            check=True, timeout=60,
        )
        installer.chmod(0o755)
        subprocess.run([installer, "--default-toolchain", "none", "--no-modify-path", "-y"],
                       check=True, timeout=30)


def find_rustup(step_name):
    rustup = shutil.which("rustup")
    if rustup:
        return rustup

    cargo_bin = Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo").resolve() / "bin"
    rustup = shutil.which("rustup", path=str(cargo_bin))
    if not rustup:
        print(f"{step_name}: bootstrapping rustup", flush=True)
        bootstrap_rustup()
        rustup = shutil.which("rustup", path=str(cargo_bin))
        if not rustup:
            raise FileNotFoundError(f"rustup bootstrap did not create an executable in {cargo_bin}")

    if github_path := os.environ.get("GITHUB_PATH"):
        with Path(github_path).open("a", encoding="utf-8") as path_file:
            print(cargo_bin, file=path_file)
    return rustup


def run(command, timeout=30):
    return subprocess.run(command, check=True, capture_output=True, text=True, timeout=timeout)


def installed_toolchain(rustup, channel, components, targets, step_name):
    probe_timeout = int(os.environ.get("RUSTUP_PROBE_TIMEOUT_SECONDS", "15"))
    if probe_timeout < 1:
        raise ValueError("rustup probe timeout must be a positive integer")
    try:
        listing = run([rustup, "toolchain", "list"], timeout=probe_timeout).stdout.splitlines()
        names = {line.split()[0] for line in listing if line.split()}
        if not any(name == channel or name.startswith(channel + "-") for name in names):
            return False
        installed_components = set(run(
            [rustup, "component", "list", "--toolchain", channel, "--installed"], timeout=probe_timeout
        ).stdout.split())
        installed_targets = set(run(
            [rustup, "target", "list", "--toolchain", channel, "--installed"], timeout=probe_timeout
        ).stdout.split())
        has_components = all(any(item == component or item.startswith(component + "-")
                                 for item in installed_components) for component in components)
        return has_components and set(targets) <= installed_targets
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"::warning::{step_name}: Rustup toolchain probe failed; attempting installation: {error}",
              flush=True)
        return False


def partial_download_sizes(downloads):
    sizes = {}
    for path in downloads.glob("*.partial"):
        try:
            sizes[path] = path.stat().st_size
        except FileNotFoundError:
            # Rustup renames completed downloads while we scan.
            pass
    return sizes


def install_with_idle_timeout(command, timeout):
    rustup_home = Path(os.environ.get("RUSTUP_HOME") or Path.home() / ".rustup")
    downloads = rustup_home / "downloads"
    sizes = partial_download_sizes(downloads)
    with tempfile.TemporaryDirectory() as directory:
        log = Path(directory) / "rustup.log"
        # Poll a file to stream logs without relying on selectable pipes on Windows.
        with log.open("wb") as writer, log.open("rb") as reader, subprocess.Popen(
            command, stdout=writer, stderr=subprocess.STDOUT
        ) as process:
            deadline = time.monotonic() + timeout
            try:
                while True:
                    returncode = process.poll()
                    output = reader.read(8192)
                    if output:
                        sys.stdout.buffer.write(output)
                        sys.stdout.buffer.flush()
                        deadline = time.monotonic() + timeout
                        continue
                    if returncode is not None:
                        if returncode:
                            raise subprocess.CalledProcessError(returncode, command)
                        return

                    # Older Rustup releases download without progress output in CI.
                    current_sizes = partial_download_sizes(downloads)
                    if any(size > sizes.get(path, 0) for path, size in current_sizes.items()):
                        deadline = time.monotonic() + timeout
                    sizes = current_sizes
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise subprocess.TimeoutExpired(command, timeout)
                    time.sleep(min(0.1, remaining))
            finally:
                if process.poll() is None:
                    process.kill()


def install(rustup, channel, components, targets, step_name):
    timeout = int(os.environ.get("RUSTUP_INSTALL_TIMEOUT_SECONDS", DEFAULT_TIMEOUT))
    attempts = int(os.environ.get("RUSTUP_INSTALL_ATTEMPTS", DEFAULT_ATTEMPTS))
    if timeout < 1 or attempts < 1:
        raise ValueError("rustup timeout and attempt count must be positive integers")

    if installed_toolchain(rustup, channel, components, targets, step_name):
        print(f"{step_name}: reusing installed Rust toolchain {channel}")
    else:
        command = [rustup, "toolchain", "install", channel, "--profile", "minimal",
                   "--no-self-update", "--force-non-host"]
        for component in components:
            command.extend(("--component", component))
        for target in targets:
            command.extend(("--target", target))
        for attempt in range(1, attempts + 1):
            print(f"{step_name}: rustup install attempt {attempt}/{attempts}", flush=True)
            try:
                install_with_idle_timeout(command, timeout)
                break
            except subprocess.TimeoutExpired:
                print(f"::warning::{step_name}: rustup install timed out after {timeout}s "
                      "without progress", flush=True)
            except subprocess.CalledProcessError as error:
                print(f"{step_name}: rustup install exited with status {error.returncode}", flush=True)
            if attempt == attempts:
                print(f"::error::{step_name}: Rust toolchain install failed after {attempts} attempts "
                      f"({timeout}s inactivity timeout)", file=sys.stderr, flush=True)
                return 1

    try:
        subprocess.run([rustup, "default", channel], check=True,
                       timeout=int(os.environ.get("RUSTUP_PROBE_TIMEOUT_SECONDS", "15")))
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        print(f"::error::{step_name}: could not select installed Rust toolchain {channel}: {error}",
              file=sys.stderr, flush=True)
        return 1
    return 0


def main(argv):
    if len(argv) != 5:
        print("usage: install-rust-toolchain.py CHANNEL COMPONENTS TARGETS STEP_NAME", file=sys.stderr)
        return 2
    channel, component_list, target_list, step_name = argv[1:]
    components = [value.strip() for value in component_list.split(",") if value.strip()]
    targets = [value.strip() for value in target_list.split(",") if value.strip()]
    try:
        return install(find_rustup(step_name), channel, components, targets, step_name)
    except ValueError as error:
        print(f"::error::{step_name}: {error}", file=sys.stderr)
        return 2
    except (OSError, subprocess.SubprocessError) as error:
        print(f"::error::{step_name}: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
