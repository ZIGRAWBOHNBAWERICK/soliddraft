#!/usr/bin/env python3
"""Install a rustup toolchain and retry stalled downloads."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


DEFAULT_TIMEOUT = 120
DEFAULT_ATTEMPTS = 3


def run(command, timeout=30):
    return subprocess.run(command, check=True, capture_output=True, text=True, timeout=timeout)


def installed_toolchain(rustup, channel, components, targets):
    try:
        listing = run([rustup, "toolchain", "list"], timeout=15).stdout.splitlines()
        names = {line.split()[0] for line in listing if line.split()}
        if not any(name == channel or name.startswith(channel + "-") for name in names):
            return False
        installed_components = set(run(
            [rustup, "component", "list", "--toolchain", channel, "--installed"], timeout=15
        ).stdout.split())
        installed_targets = set(run(
            [rustup, "target", "list", "--toolchain", channel, "--installed"], timeout=15
        ).stdout.split())
        has_components = all(any(item == component or item.startswith(component + "-")
                                 for item in installed_components) for component in components)
        return has_components and set(targets) <= installed_targets
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
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

    if installed_toolchain(rustup, channel, components, targets):
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
        subprocess.run([rustup, "default", channel], check=True, timeout=15)
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
    rustup = shutil.which("rustup")
    if not rustup:
        print(f"::error::{step_name}: rustup is not installed", file=sys.stderr)
        return 1
    components = [value.strip() for value in component_list.split(",") if value.strip()]
    targets = [value.strip() for value in target_list.split(",") if value.strip()]
    try:
        return install(rustup, channel, components, targets, step_name)
    except ValueError as error:
        print(f"::error::{step_name}: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
