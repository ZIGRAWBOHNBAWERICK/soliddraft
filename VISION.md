# Vision

`SolidDraft` exists so that anyone can design a mechanical part, edit it again later and hand it to manufacturing without asking a kernel vendor for permission.
It serves engineers and makers who build real parts in a browser tab or at a desktop, and the agents that drive the same commands.
It turns sketches, features, assemblies and drawings into editable parts on Catallax, and into files a shop or another CAD system can use.
It owns exactly one thing: the application around Catallax, meaning the document, the interface, the commands and file exchange.

## One kernel, one project

SolidDraft and Catallax are one project in two repositories, and SolidDraft runs only on Catallax.
There is no fallback kernel and no second engine for building solids.
Building and changing solid geometry belongs in Catallax, and a missing shape capability is fixed there rather than patched around in the application.
When SolidDraft needs Catallax work, it goes to the Catallax repository as an issue, and Catallax resolves it one way or the other.
Other geometry work, such as checks on meshes, can sit on either side; that boundary is decided case by case, and each decision is made on purpose.
The application keeps undo, rebuild, caching and file formats on its side of one kernel boundary.
SolidDraft copies no Catallax code.

## Features that work

The measure of a change is whether a feature works for the user and is finished.
The goal is a modeller that finishes the user's part, lets them edit it again and delivers it.
Numerical geometry with stated tolerances and real geometric checks is welcome when it lets a part finish that would otherwise refuse.
Hard edits finish through better algorithms and efficiency, not through machinery that waits, queues or retries on its own.
Nothing hangs: an edit that cannot finish in reasonable time fails fast with a plain reason instead of freezing the app, and releases test this, at least in the browser.
Over time, more parts are made to build within that limit.
The browser app is the priority: a new feature works in the browser before it merges, and desktop-only work waits until the browser can run it.
Measurement, timing and comparison tooling live in tests, CI and the development build, never in the runtime path of a user edit.

## Honest results

Every result is built or refused with a plain reason, and a refusal leaves the document as it was.
A result is never labelled as more certain than its check, and a failed or pending feature never exports as a finished part.
A part built within a stated tolerance is a finished part, and it exports like any other part, without a warning.
Sketches are exact: SolidDraft's sketch carries no tolerance or rounding, and a sketch that cannot be solved exactly shows as failed before anything is built from it.
Proof and status detail serves the user's part; it is not a product of its own, and SolidDraft builds no general certificate system.

## Commands a designer expects

Each command asks for the inputs a mechanical designer expects, with sensible defaults already filled in.
When Catallax offers a choice meant for the user, SolidDraft lets the user make it, starting from Catallax's default.
Names and shortcuts borrow common SolidWorks terms where they make sense; elsewhere SolidDraft makes its own or borrows from other applications.
A Coming soon command is added only once its scope is clearly defined and its work is on the roadmap of both projects where needed.
It shows only "Coming soon", without stage names or kernel details.
One frame, Edit Suite, holds parts, assemblies and drawings, and a replaced interface is deleted rather than kept behind a switch.

## The user's work survives

Every edit is one undo step, survives save and reopen, and leaves a recovery copy when something fails.
A face or edge reference follows its Catallax origin; a lost reference asks for repair and is never resolved by guessing the nearest geometry.
Documents saved by earlier versions keep opening.
Every command goes through one registry, so the window, the command line, MCP and the assistant run the same code, and a window-only command states why.
A change is proved by driving the real application as a user would, through the application loop, not by tests of internals alone.

## Scope

SolidDraft is free to use, change and share under the MIT licence, and it depends on no closed kernel, licence server or paid account.
The project pays for no AI in the public browser app; anyone can connect their own AI, and a private deployment can carry its owner's keys.
Work follows the most usable parametric CAD features first: parts and drawings, then assemblies, then field and lattice modelling.
File exchange includes the formats robot simulators read, such as URDF and USD, with joints taken from the mates.
A strength analysis (FEA) tool belongs in SolidDraft's future, after the kernel work it depends on is sorted.

A change aligns when it makes a feature work for a real part on Catallax, in the browser first, sends any Catallax work it needs to the Catallax repository as an issue, keeps each result honest, and adds no more machinery than the task needs.
A change should be resisted when it works around Catallax for solid geometry instead of raising the Catallax issue, presents an unchecked result as finished, hides a kernel choice meant for the user, adds a mode the user did not need, lets the app hang instead of failing fast, or adds machinery whose job is to wait for, retry or wrap a proof.
