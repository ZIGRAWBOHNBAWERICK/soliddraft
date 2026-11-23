#!/usr/bin/env python3
"""Install a rustup toolchain with a bounded, retrying download."""
import os
import shutil
import subprocess
import sys


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
                subprocess.run(command, check=True, timeout=timeout)
                break
            except subprocess.TimeoutExpired:
                print(f"::warning::{step_name}: rustup install timed out after {timeout}s", flush=True)
            except subprocess.CalledProcessError as error:
                print(f"{step_name}: rustup install exited with status {error.returncode}", flush=True)
            if attempt == attempts:
                print(f"::error::{step_name}: Rust toolchain install failed after {attempts} attempts "
                      f"({timeout}s per attempt)", file=sys.stderr, flush=True)
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
